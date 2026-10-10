//! 外へ出る通信の見張り（段 1: コードと依存）。`cargo xtask netguard` で表を出し、同じ確かめを試験（`cargo test -p xtask`）が毎回回す。
//!
//! アプリが外へ通信するのは、利用者が選んだ更新の確認だけ（README・INSTALL の約束）。待ち受けは、設定で入れている間の 127.0.0.1 だけ。
//! この約束が、新しいコードや依存で静かに破れないよう、2 つを読む。
//!
//! - ソース: `crates/*/src/**/*.rs` と `crates/*/build.rs`（この xtask 自身は配らないので対象外）から、通信と外への出口の印
//!   （[`MARKERS`]。ソケットの型・HTTP の部品・プロセスの起動・「開く」の呼び出し・`http(s)://` の文字列など）を探し、[`ALLOWED`]（ファイル・印・理由・数）に無いものを落とす。
//!   Rust の字句（コメント・文字列・文字・ライフタイム）を読み分けるので、コメントや文字列の中の識別子には反応せず、行番号が合う。
//!   `#[cfg(test)]` を付けた項目と、`#[cfg(test)] mod x;` のファイルは、配る物に入らないので除く。
//!   ソケットの `bind`・`connect` は、宛先に 127.0.0.1（`LOCALHOST`・`LOOPBACK`・`localhost`・`::1`）が読めなければ、一覧とは別に落とす。
//! - 依存: `Cargo.lock` に HTTP の客・TLS・DNS・テレメトリなどの名前が無いこと、`cargo metadata` で、待ち受けの部品（hyper・hyper-util・socket2・mio、
//!   tokio の `net`）を直接の依存に持つのが yolu-mcp だけであること、配るアプリ（yolu-app・yolu-cli）が「ブラウザーを開く」クレートに依存しないこと、
//!   低水準の通信の部品に依存するクレートが決めた範囲から増えていないこと、Windows の API の機能（宣言と、Cargo が実際に有効にした機能）に WinHTTP 以外の通信が入っていないことを確かめる。
//!   配るアプリの依存（本番とビルドスクリプトの閉包）のソースも同じ字句の読み手で読み、ソケット・通信の部品を使うクレートを理由つきの一覧（`SOCKET_CRATES`）にする。
//!   新しいクレートがソケットを使い始めたら落ちる（約 440 クレートを読む）。
//!
//! 一覧に足すときは、README・INSTALL の約束（更新の確認のほかに通信しない・外からの操作は 127.0.0.1 だけ）を破らないか確かめ、足す行に理由を書く
//! （`docs/DEVELOPMENT.md` の「通信の見張り」）。

mod allowed;
mod deps;
mod lex;
mod markers;
mod uses;

use crate::{root, Result};
use allowed::*;
use deps::*;
use lex::*;
use markers::*;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs,
    path::{Path, PathBuf},
};

// ───────── 走査 ─────────

#[derive(Debug, Default)]
struct Scan {
    hits: Vec<Hit>,
    /// 宛先が 127.0.0.1 と読めない bind・connect（ファイル・行・呼び出し）。
    endpoints: Vec<(String, usize, String)>,
    /// 走査したファイルの数（空振りを見抜く）。
    files: usize,
}

fn walk_rs(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let mut entries: Vec<_> = fs::read_dir(dir)?.collect::<std::io::Result<_>>()?;
    entries.sort_by_key(|e| e.path());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            walk_rs(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    Ok(())
}

/// クレートのフォルダの `src/**/*.rs` と `build.rs` の（`base` からの `/` 区切りの相対の道・中身）。
fn crate_files(base: &Path, crate_dir: &Path) -> Result<Vec<(String, String)>> {
    let mut paths = Vec::new();
    if crate_dir.join("build.rs").is_file() {
        paths.push(crate_dir.join("build.rs"));
    }
    if crate_dir.join("src").is_dir() {
        walk_rs(&crate_dir.join("src"), &mut paths)?;
    }
    paths
        .into_iter()
        .map(|path| {
            let relative = path
                .strip_prefix(base)?
                .to_string_lossy()
                .replace('\\', "/");
            Ok((
                relative,
                String::from_utf8_lossy(&fs::read(&path)?).into_owned(),
            ))
        })
        .collect()
}

/// `crates/*/src/**/*.rs` と `crates/*/build.rs`（xtask を除く）の（`/` 区切りの相対の道・中身）。
fn source_files(root: &Path) -> Result<Vec<(String, String)>> {
    let mut crates: Vec<_> = fs::read_dir(root.join("crates"))?.collect::<std::io::Result<_>>()?;
    crates.sort_by_key(|e| e.path());
    let mut files = Vec::new();
    for entry in crates {
        let dir = entry.path();
        if dir.is_dir() && entry.file_name() != "xtask" {
            files.extend(crate_files(root, &dir)?);
        }
    }
    Ok(files)
}

/// `file` に書いた `mod name;` が指すファイルの道（`name.rs` か `name/mod.rs`。`mod.rs`・`lib.rs`・`main.rs` は同じ階、ほかは `<ファイル名>/` の階）。
/// `#[path = "…"]` があれば、そのファイル（`file` と同じ階から。`..` は畳む）だけ。
fn module_files(file: &str, decl: &ModDecl) -> Vec<String> {
    let (dir, base) = file.rsplit_once('/').unwrap_or(("", file));
    if let Some(path) = &decl.path {
        return vec![normalize(&format!("{dir}/{path}"))];
    }
    let stem = base.trim_end_matches(".rs");
    let folder = if matches!(base, "mod.rs" | "lib.rs" | "main.rs") {
        dir.to_owned()
    } else {
        format!("{dir}/{stem}")
    };
    vec![
        format!("{folder}/{}.rs", decl.name),
        format!("{folder}/{}/mod.rs", decl.name),
    ]
}

/// `a/b/../c` → `a/c`、`a/./b` → `a/b`。
fn normalize(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    parts.join("/")
}

/// ファイルの集まり（相対の道・中身）を走査する。試験だけのファイル（`#[cfg(test)] mod x;` の先と、その下のモジュール）は除く。
fn scan_sources(files: &[(String, String)]) -> Scan {
    let lexed: Vec<(&str, Vec<Token>, Vec<ModDecl>)> = files
        .iter()
        .map(|(file, text)| {
            let (tokens, modules) = strip_test_items(lex(text));
            (file.as_str(), tokens, modules)
        })
        .collect();
    // 試験だけのモジュールのファイルと、その下のフォルダ
    let mut test_files = BTreeSet::new();
    let mut test_folders = Vec::new();
    for (file, _, modules) in &lexed {
        for decl in modules {
            for path in module_files(file, decl) {
                if let Some(stem) = path.strip_suffix("/mod.rs") {
                    test_folders.push(format!("{stem}/"));
                } else {
                    test_folders.push(format!("{}/", path.trim_end_matches(".rs")));
                }
                test_files.insert(path);
            }
        }
    }
    let mut scan = Scan::default();
    for (file, tokens, _) in &lexed {
        if test_files.contains(*file) || test_folders.iter().any(|d| file.starts_with(d.as_str())) {
            continue;
        }
        scan.files += 1;
        scan_tokens(file, tokens, &mut scan.hits, &mut scan.endpoints);
    }
    scan
}

// ───────── 突き合わせ ─────────

#[derive(Debug, Default)]
struct Report {
    violations: Vec<String>,
    /// （ファイル・印・見つけた数・理由）。一覧のうち使われた行。
    table: Vec<(String, String, usize, String)>,
    /// 依存のうちソケット・通信の部品を使うクレート（名前・版・印の数・印）。
    dependencies: Vec<(String, String, usize, String)>,
    /// ソースを読んだ依存のクレートの数。
    dependency_crates: usize,
}

fn guidance() -> &'static str {
    "通信を足す前に README・INSTALL の約束（更新の確認のほかに通信しない。外からの操作は設定で入れている間だけ 127.0.0.1）を破らないか確かめ、\
     足してよいときは crates/xtask/src/netguard/allowed.rs の ALLOWED に、ファイル・印・数と理由を足します。"
}

fn evaluate(scan: &Scan, allowed: &[Allow]) -> Report {
    let mut report = Report::default();
    let mut found: BTreeMap<(&str, &str), Vec<usize>> = BTreeMap::new();
    for hit in &scan.hits {
        found
            .entry((&hit.file, hit.mark))
            .or_default()
            .push(hit.line);
    }
    let listed: HashMap<(&str, &str), &Allow> =
        allowed.iter().map(|a| ((a.file, a.mark), a)).collect();
    for ((file, mark), lines) in &found {
        let lines_text = lines
            .iter()
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join("・");
        match listed.get(&(*file, *mark)) {
            None => report.violations.push(format!(
                "{file}:{} の {mark} が、許す一覧にありません（{} 行目）。{}",
                lines[0],
                lines_text,
                guidance()
            )),
            Some(allow) => {
                if let Exactly(n) = allow.count {
                    if n != lines.len() {
                        report.violations.push(format!(
                            "{file} の {mark} が一覧では {n} 件なのに {} 件になりました（{} 行目）。\
                             増えた出口が理由に当てはまるか確かめ、当てはまるときだけ一覧の数を直します。{}",
                            lines.len(),
                            lines_text,
                            guidance()
                        ));
                    }
                }
                report.table.push((
                    file.to_string(),
                    mark.to_string(),
                    lines.len(),
                    allow.why.to_string(),
                ));
            }
        }
    }
    for allow in allowed {
        if !found.contains_key(&(allow.file, allow.mark)) {
            report.violations.push(format!(
                "許す一覧の {} の {} が、ソースの中に見つかりません。使われなくなった行は一覧から消します（理由: {}）。",
                allow.file, allow.mark, allow.why
            ));
        }
    }
    for (file, line, call) in &scan.endpoints {
        report.violations.push(format!(
            "{file}:{line} の {call} の宛先に 127.0.0.1（LOCALHOST・LOOPBACK・localhost・::1）が読めません。\
             外へ向かうソケットは作りません。宛先は呼び出しの引数に直接書きます。"
        ));
    }
    report
}

// ───────── 入り口 ─────────

/// リポジトリの全部を確かめた結果。
fn check_repository(root: &Path) -> Result<(Report, usize)> {
    let sources = source_files(root)?;
    let scan = scan_sources(&sources);
    let mut report = evaluate(&scan, ALLOWED);
    report
        .violations
        .extend(check_lock(&fs::read_to_string(root.join("Cargo.lock"))?));
    let meta = read_metadata(root)?;
    report.violations.extend(check_metadata(&meta));
    let deps = scan_dependency_sources(&meta, read_crate_dir);
    report.violations.extend(deps.violations);
    report.dependencies = deps.table;
    report.dependency_crates = deps.crates;
    Ok((report, scan.files))
}

/// `cargo xtask netguard`: 許す一覧の使われ方の表を出し、落ちるものがあれば終了コード 1。
pub(crate) fn run(mut args: impl Iterator<Item = String>) -> Result<()> {
    if let Some(arg) = args.next() {
        return Err(format!("未対応の引数: {arg}").into());
    }
    let (report, files) = check_repository(&root())?;
    println!(
        "通信の見張り: ソース {files} ファイルを読みました。許す一覧の使われ方（×の数は印の出た数。`use` の行の識別子も 1 件と数える）:"
    );
    for (file, mark, count, why) in &report.table {
        println!("  {file}  {mark} ×{count}  {why}");
    }
    println!(
        "依存のソース {} クレートを読み、ソケット・通信の部品を使う物:",
        report.dependency_crates
    );
    for (name, versions, count, marks) in &report.dependencies {
        let why = SOCKET_CRATES
            .iter()
            .find(|(n, _)| n == name)
            .map_or("（一覧に無い）", |(_, why)| *why);
        println!("  {name} {versions}  {marks} ×{count}  {why}");
    }
    if report.violations.is_empty() {
        println!("通信の見張り: 一覧の外の出口も、依存の問題もありません。");
        return Ok(());
    }
    for violation in &report.violations {
        eprintln!("- {violation}");
    }
    Err(format!(
        "通信の見張りに {} 件、引っかかりました",
        report.violations.len()
    )
    .into())
}

#[cfg(test)]
mod tests;
