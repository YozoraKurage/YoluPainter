//! リリースの事前確認（`cargo xtask preflight`）。配る物を作る段（許諾の照合・梱包・インストーラー）が、本番で初めて走って止まる種類の失敗を、
//! 組まずに（数分以内に）全部並べる。最初の 1 つで止めず、全部を回してから、失敗が 1 つでもあれば終了コード 1 にする。
//!
//! 確かめ（`--only` で絞れる。名前は [`CHECKS`]）:
//! - `licenses`: 対象ごとの許諾の照合（`tools/third-party.py --target T --bundle` と同じ命令。許諾の全文の束も作る）
//! - `attributes`: SHA-256 で照合する表記ファイル（`tools/licenses-reviewed.json` の `bundled` と、クレートの原文のうちリポジトリに置いた文 `repo`）が、`.gitattributes` で `eol=lf` か `-text` か
//!   （Windows の checkout で CRLF になって照合が落ちた、0.3.0 の配布の失敗）
//! - `nsis`: インストーラーの台本の `Target` が、公式の Windows 版 NSIS が持つ stub（x86-unicode・x86-ansi）か（amd64 は無く、0.3.0 の配布で落ちた）
//! - `mcpb`: Claude Desktop の拡張（.mcpb）を作る対象（Windows）で、`manifest.json` が組めて形が合うことと、拡張に入れるコマンドラインの許諾の束
//!   （`tools/third-party.py --package yolu-cli --built-with yolu-app --bundle`）が作れること、拡張に入れるロゴの PNG があること
//! - `macos`: macOS の試作の配布物（universal の .app）を作る対象で、`Info.plist` が組めて版が合うこと、アイコンの元のロゴが 1024 x 1024 の PNG であること、
//!   同梱する文書が一覧に載っていること。macOS の上では、加えて `lipo`・`codesign` などのツールと、2 つの Rust のターゲットが入っていること
//! - `version`: yolu-app の版のタグ `v<版>` が origin にまだ無いか。`--kind stable` ならプレリリースの版でないか、`--kind prerelease` なら
//!   試験版の形（`alpha.N`・`beta.N`・`rc.N`）の版か
//! - `targets`: `tools/dist-targets.json` の対象が、この xtask が配れる対象（と、Windows はインストーラーつき）か
//! - `workflows`: ワークフローの YAML が読めるか。release.yml の入力と対象の並びが食い違わないか（`tools/check-workflows.py`）
//! - `installer`（`--installer` か `--only installer` のときだけ）: Wine で `tools/test-installer.py`（wine32 が要る）
use super::{
    check_docs_listed, check_mcpb_manifest, macos, mcpb_manifest, python, root, third_party,
    third_party_cli, workspace_version, Result, MCPB_LOGO,
};
use semver::Version;
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};
use yolu_update::{
    is_archive_target, is_beta_version, MACOS_ARCHIVE, MACOS_TRIPLES, WINDOWS_ARCHIVE,
};

/// 確かめの名前（実行する順）。
pub(crate) const CHECKS: &[&str] = &[
    "licenses",
    "attributes",
    "nsis",
    "mcpb",
    "macos",
    "version",
    "targets",
    "workflows",
    "installer",
];
/// 公式の Windows 版 NSIS が持つ、インストーラーの外側の部品（stub）。amd64 の部品は公式の Windows 版には無い。
const NSIS_STUBS: &[&str] = &["x86-unicode", "x86-ansi"];
/// 並べる失敗の行数の上限（残りは件数で言う）。
const LISTED: usize = 15;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    Passed(String),
    Failed(Vec<String>),
    Skipped(String),
}
use Outcome::{Failed, Passed, Skipped};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Stable,
    Prerelease,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct Options {
    targets: Vec<String>,
    kind: Option<Kind>,
    only: Vec<String>,
    installer: bool,
    offline: bool,
}

fn parse_options(args: impl Iterator<Item = String>) -> Result<Options> {
    let mut options = Options::default();
    let mut args = args;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--target" => options.targets.push(super::value(&mut args)?),
            "--kind" => {
                options.kind = Some(match super::value(&mut args)?.as_str() {
                    "stable" => Kind::Stable,
                    "prerelease" => Kind::Prerelease,
                    other => {
                        return Err(format!("--kind は stable か prerelease です: {other}").into())
                    }
                })
            }
            "--only" => {
                for name in super::value(&mut args)?.split(',') {
                    if !CHECKS.contains(&name) {
                        return Err(
                            format!("未対応の確かめです: {name}（{}）", CHECKS.join("・")).into(),
                        );
                    }
                    options.only.push(name.to_owned());
                }
            }
            "--installer" => options.installer = true,
            "--offline" => options.offline = true,
            _ => return Err(format!("未対応の引数: {arg}").into()),
        }
    }
    for target in &options.targets {
        // 配れる対象のほか、macOS の universal の配布物を作る 2 つの Rust のターゲットも、許諾の照合のために指せる
        if !is_archive_target(target) && !MACOS_TRIPLES.contains(&target.as_str()) {
            return Err(format!("未対応の配布ターゲットです: {target}").into());
        }
    }
    Ok(options)
}

/// 実行する確かめ（`CHECKS` の順）。`--only` が無ければ既定（インストーラーの試験を除く）。`--installer` は試験を足す。
fn selected(options: &Options) -> Vec<&'static str> {
    CHECKS
        .iter()
        .copied()
        .filter(|name| {
            let listed = if options.only.is_empty() {
                *name != "installer"
            } else {
                options.only.iter().any(|only| only == name)
            };
            listed || (*name == "installer" && options.installer)
        })
        .collect()
}

// ───────── 対象の一覧（tools/dist-targets.json。CI・配布と同じ 1 か所） ─────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DistTarget {
    pub target: String,
    /// ビルドする runner（ワークフローの `runs-on`）。
    pub os: String,
    pub installer: bool,
    /// release.yml のこの名前の入力が入のときだけ組む対象（`None` は常に組む）。
    pub input: Option<String>,
    /// 入力の既定が入の対象。PR の CI（入力が無い）でもビルドする。
    pub default_on: bool,
    /// 試作の対象。ビルドが落ちても CI の結論を失敗にせず、配布も止めない（成果物が無ければ載せない）。
    pub experimental: bool,
    /// その対象のビルドのジョブの時間の上限（分）。無ければワークフローの既定。
    pub timeout_minutes: Option<u64>,
    /// ビルドに要る Rust のターゲット（空なら `target` そのもの）。macOS の universal は 2 つ。
    pub rust_targets: Vec<String>,
}

pub(crate) fn parse_dist_targets(text: &str) -> Result<Vec<DistTarget>> {
    let json: serde_json::Value = serde_json::from_str(text)?;
    let list = json["targets"]
        .as_array()
        .ok_or("dist-targets.json に targets がありません")?;
    list.iter()
        .map(|item| {
            Ok(DistTarget {
                target: item["target"]
                    .as_str()
                    .ok_or("対象に target がありません")?
                    .to_owned(),
                os: item["os"]
                    .as_str()
                    .ok_or("対象に os がありません")?
                    .to_owned(),
                installer: item["installer"]
                    .as_bool()
                    .ok_or("対象に installer（真偽値）がありません")?,
                input: match &item["input"] {
                    serde_json::Value::Null => None,
                    serde_json::Value::String(name) => Some(name.clone()),
                    _ => return Err("対象の input は文字列か null です".into()),
                },
                default_on: match &item["default"] {
                    serde_json::Value::Null => false,
                    serde_json::Value::Bool(value) => *value,
                    _ => return Err("対象の default は真偽値です".into()),
                },
                experimental: match &item["experimental"] {
                    serde_json::Value::Null => false,
                    serde_json::Value::Bool(value) => *value,
                    _ => return Err("対象の experimental は真偽値です".into()),
                },
                timeout_minutes: match &item["timeout_minutes"] {
                    serde_json::Value::Null => None,
                    value => Some(
                        value
                            .as_u64()
                            .filter(|minutes| *minutes >= 1)
                            .ok_or("対象の timeout_minutes は 1 以上の整数です")?,
                    ),
                },
                rust_targets: match &item["rust_targets"] {
                    serde_json::Value::Null => Vec::new(),
                    serde_json::Value::String(text) => {
                        text.split_whitespace().map(str::to_owned).collect()
                    }
                    _ => return Err("対象の rust_targets は空白で区切った文字列です".into()),
                },
            })
        })
        .collect()
}

fn dist_targets(root: &Path) -> Result<Vec<DistTarget>> {
    parse_dist_targets(&fs::read_to_string(root.join("tools/dist-targets.json"))?)
}

// ───────── 各確かめ ─────────

fn lines(bytes: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(bytes)
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.trim().is_empty())
        .map(str::to_owned)
        .collect()
}

/// 失敗の行を、上限までに丸める（アイコンのように数百のファイルが同じ理由で落ちても、画面を埋めない）。
fn limited(mut problems: Vec<String>) -> Vec<String> {
    if problems.len() > LISTED {
        let rest = problems.len() - LISTED;
        problems.truncate(LISTED);
        problems.push(format!("ほか {rest} 件"));
    }
    problems
}

fn check_licenses(root: &Path, target: &str, offline: bool) -> Outcome {
    let mut command = third_party(root, target);
    if offline {
        command.arg("--offline");
    }
    match command.output() {
        Err(error) => Failed(vec![format!("Python を起動できません: {error}")]),
        Ok(output) if output.status.success() => Passed(
            lines(&output.stdout)
                .into_iter()
                .next()
                .unwrap_or_else(|| "照合成功".into()),
        ),
        Ok(output) => {
            let mut problems = lines(&output.stderr);
            // 標準出力の「結果: <出力先>」は理由ではない。
            problems.extend(
                lines(&output.stdout)
                    .into_iter()
                    .filter(|line| !line.starts_with("結果:")),
            );
            if problems.is_empty() {
                problems.push(format!("終了コード {:?}", output.status.code()));
            }
            Failed(limited(problems))
        }
    }
}

/// `tools/licenses-reviewed.json` の、SHA-256 で照合するリポジトリの中のファイル: `bundled`（クレートでない同梱物）と、
/// クレートの原文のうちリポジトリに置いた文（`crates` の `repo`）。クレートの原文のそれ以外は、取得元か登録先のクレートの中の物。
fn reviewed_files(root: &Path) -> Result<Vec<String>> {
    let json: serde_json::Value = serde_json::from_str(&fs::read_to_string(
        root.join("tools/licenses-reviewed.json"),
    )?)?;
    let mut files = Vec::new();
    for items in json["bundled"]
        .as_object()
        .ok_or("licenses-reviewed.json に bundled がありません")?
        .values()
    {
        for item in items.as_array().ok_or("bundled の形が違います")? {
            let listed = item["files"].as_array().into_iter().flatten();
            let license = std::iter::once(&item["license_file"]);
            for spec in listed.chain(license) {
                if let Some(path) = spec["path"].as_str() {
                    if root.join(path).is_file() && !files.iter().any(|f| f == path) {
                        files.push(path.to_owned());
                    }
                }
            }
        }
    }
    // クレートの原文のうち、上流の記載から組み立てて、リポジトリに置いた文（`repo`。tools/license-texts/）
    for review in json["crates"]
        .as_object()
        .ok_or("licenses-reviewed.json に crates がありません")?
        .values()
    {
        for spec in review["files"].as_array().into_iter().flatten() {
            if let Some(path) = spec["repo"].as_str() {
                if root.join(path).is_file() && !files.iter().any(|f| f == path) {
                    files.push(path.to_owned());
                }
            }
        }
    }
    Ok(files)
}

/// `git check-attr -z text eol` の出力（`パス NUL 属性 NUL 値 NUL` の繰り返し）から、CRLF に変換され得るファイルの問題を拾う。
/// 変換されないのは、`text` が unset（`-text`・`binary`）のものと、`eol=lf` のもの。
fn attribute_problems(output: &[u8]) -> Vec<String> {
    let fields: Vec<&[u8]> = output.split(|b| *b == 0).collect();
    let mut found: Vec<(String, String, String)> = Vec::new();
    for triple in fields.as_chunks::<3>().0 {
        let (path, attribute, value) = (
            String::from_utf8_lossy(triple[0]).into_owned(),
            String::from_utf8_lossy(triple[1]),
            String::from_utf8_lossy(triple[2]).into_owned(),
        );
        let position = match found.iter().position(|(p, _, _)| *p == path) {
            Some(position) => position,
            None => {
                found.push((path, "unspecified".into(), "unspecified".into()));
                found.len() - 1
            }
        };
        match attribute.as_ref() {
            "text" => found[position].1 = value,
            "eol" => found[position].2 = value,
            _ => {}
        }
    }
    found
        .into_iter()
        .filter(|(_, text, eol)| text != "unset" && eol != "lf")
        .map(|(path, text, eol)| format!("{path}: text={text} eol={eol}"))
        .collect()
}

fn check_attributes_in(root: &Path, files: &[String]) -> Outcome {
    if files.is_empty() {
        return Passed("確かめる表記ファイルがありません".into());
    }
    let mut child = match Command::new("git")
        .current_dir(root)
        .args(["check-attr", "-z", "--stdin", "text", "eol"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => return Failed(vec![format!("git を起動できません: {error}")]),
    };
    let input: Vec<u8> = files
        .iter()
        .flat_map(|f| f.bytes().chain(std::iter::once(0)))
        .collect();
    // 標準入力は別のスレッドで書く（出力が詰まって双方が待つのを避ける）。
    let mut stdin = child.stdin.take().expect("piped");
    let writer = std::thread::spawn(move || stdin.write_all(&input));
    let output = match child.wait_with_output() {
        Ok(output) => output,
        Err(error) => return Failed(vec![format!("git check-attr を実行できません: {error}")]),
    };
    let _ = writer.join();
    if !output.status.success() {
        let mut problems = lines(&output.stderr);
        problems.insert(
            0,
            "git check-attr が失敗しました（git の作業コピーで回してください）".into(),
        );
        return Failed(limited(problems));
    }
    let problems = attribute_problems(&output.stdout);
    if problems.is_empty() {
        Passed(format!("{} ファイルが LF 固定か変換なし", files.len()))
    } else {
        let mut report = vec![
            "SHA-256 で照合するのに、Windows の checkout で CRLF になり得ます。.gitattributes で eol=lf か -text にしてください".to_owned(),
        ];
        report.extend(problems);
        Failed(limited(report))
    }
}

fn check_attributes(root: &Path) -> Outcome {
    match reviewed_files(root) {
        Ok(files) => check_attributes_in(root, &files),
        Err(error) => Failed(vec![format!(
            "tools/licenses-reviewed.json を読めません: {error}"
        )]),
    }
}

/// NSIS の台本の `Target` の指定（コメントの行は数えない。命令名は大文字小文字を区別しない）。
fn nsis_targets(script: &str) -> Vec<String> {
    script
        .trim_start_matches('\u{feff}')
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with(';') && !line.starts_with('#'))
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            let name = words.next()?;
            let value = words.next()?;
            name.eq_ignore_ascii_case("Target")
                .then(|| value.trim_matches('"').to_owned())
        })
        .collect()
}

fn nsis_outcome(script: &str) -> Outcome {
    let targets = nsis_targets(script);
    if targets.is_empty() {
        return Failed(vec![format!(
            "installer/yolupainter.nsi に Target の指定がありません（{} のどちらかを書く）",
            NSIS_STUBS.join("・")
        )]);
    }
    let bad: Vec<String> = targets
        .iter()
        .filter(|t| !NSIS_STUBS.iter().any(|stub| t.eq_ignore_ascii_case(stub)))
        .map(|t| {
            format!(
                "installer/yolupainter.nsi の Target {t} は、公式の Windows 版 NSIS に stub がありません（{} のどちらか）",
                NSIS_STUBS.join("・")
            )
        })
        .collect();
    if bad.is_empty() {
        Passed(format!("Target {}", targets.join("・")))
    } else {
        Failed(bad)
    }
}

/// .mcpb を作る対象（Windows）の事前確認: manifest が組めて形が合い、ロゴの PNG があり、コマンドラインの許諾の束が作れる。
fn check_mcpb(root: &Path, offline: bool) -> Outcome {
    let version = match workspace_version() {
        Ok(version) => version,
        Err(error) => return Failed(vec![format!("版を取得できません: {error}")]),
    };
    let mut problems = Vec::new();
    if let Err(error) = check_mcpb_manifest(&mcpb_manifest(&version), &version) {
        problems.push(error.to_string());
    }
    if !root.join(MCPB_LOGO).is_file() {
        problems.push(format!("拡張に入れるロゴがありません: {MCPB_LOGO}"));
    }
    let mut command = third_party_cli(root, WINDOWS_ARCHIVE);
    if offline {
        command.arg("--offline");
    }
    match command.output() {
        Err(error) => problems.push(format!("Python を起動できません: {error}")),
        Ok(output) if output.status.success() => {}
        Ok(output) => {
            problems.push("コマンドラインの許諾の照合が失敗しました".to_owned());
            problems.extend(lines(&output.stderr));
        }
    }
    if problems.is_empty() {
        Passed(format!(
            "manifest と許諾の束（yolu-cli）が通る（版 {version}）"
        ))
    } else {
        Failed(limited(problems))
    }
}

fn check_nsis(root: &Path) -> Outcome {
    match fs::read_to_string(root.join("installer/yolupainter.nsi")) {
        Ok(script) => nsis_outcome(&script),
        Err(error) => Failed(vec![format!(
            "installer/yolupainter.nsi を読めません: {error}"
        )]),
    }
}

/// macOS の試作の配布物（universal の .app）を作る前の確かめ。ツールと Rust のターゲットは、macOS の上だけ見る（ほかの OS ではビルドしない）。
/// `tool` は PATH にツールがあるか、`installed` はその Rust のターゲットのライブラリが入っているか。
fn macos_problems(
    root: &Path,
    version: &Version,
    on_macos: bool,
    tool: impl Fn(&str) -> bool,
    installed: impl Fn(&str) -> bool,
) -> Vec<String> {
    let mut problems = Vec::new();
    let plist = macos::info_plist(version);
    for needed in [
        format!("<string>{version}</string>"),
        format!("<string>{}</string>", macos::BUNDLE_ID),
    ] {
        if !plist.contains(&needed) {
            problems.push(format!("Info.plist に {needed} がありません"));
        }
    }
    if let Err(error) = macos::check_icon_source(&root.join(macos::ICON_SOURCE)) {
        problems.push(error.to_string());
    }
    if let Err(error) = check_docs_listed(root) {
        problems.push(error.to_string());
    }
    if on_macos {
        for name in macos::HOST_TOOLS {
            if !tool(name) {
                problems.push(format!(
                    "{name} が見つかりません（Xcode の Command Line Tools が要ります）"
                ));
            }
        }
        for triple in MACOS_TRIPLES {
            if !installed(triple) {
                problems.push(format!(
                    "Rust のターゲット {triple} が入っていません（`rustup target add {}`）",
                    MACOS_TRIPLES.join(" ")
                ));
            }
        }
    }
    problems
}

/// Rust のターゲットのライブラリが、使っているツールチェーンに入っているか（`rustup` が無い環境でも見られるよう、sysroot の中を見る）。
fn rust_target_installed(triple: &str) -> bool {
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let Ok(output) = Command::new(rustc).args(["--print", "sysroot"]).output() else {
        return false;
    };
    let sysroot = String::from_utf8_lossy(&output.stdout);
    Path::new(sysroot.trim())
        .join("lib/rustlib")
        .join(triple)
        .join("lib")
        .is_dir()
}

fn check_macos(root: &Path) -> Outcome {
    let version = match workspace_version() {
        Ok(version) => version,
        Err(error) => return Failed(vec![format!("版を取得できません: {error}")]),
    };
    let on_macos = cfg!(target_os = "macos");
    let problems = macos_problems(
        root,
        &version,
        on_macos,
        |name| macos::find_tool(name).is_some(),
        rust_target_installed,
    );
    if !problems.is_empty() {
        return Failed(limited(problems));
    }
    Passed(if on_macos {
        format!("Info.plist・アイコンの元・文書の一覧・ツール・Rust のターゲットがそろっている（版 {version}）")
    } else {
        format!("Info.plist・アイコンの元・文書の一覧がそろっている（版 {version}。ツールは macOS の上でだけ見る）")
    })
}

/// `remote` にタグ `v<版>` があるか。問い合わせられなければ `Err`（ネットが無い・origin が無い）。
fn tag_exists(root: &Path, remote: &str, version: &Version) -> std::result::Result<bool, String> {
    let reference = format!("refs/tags/v{version}");
    let output = Command::new("git")
        .current_dir(root)
        // 認証の入力待ちや、返ってこない通信で止まらない。
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_HTTP_LOW_SPEED_LIMIT", "1000")
        .env("GIT_HTTP_LOW_SPEED_TIME", "20")
        .args(["ls-remote", "--tags", remote, &reference])
        .output()
        .map_err(|error| format!("git を起動できません: {error}"))?;
    if !output.status.success() {
        let why = lines(&output.stderr).into_iter().next().unwrap_or_default();
        return Err(format!("git ls-remote {remote}: {why}"));
    }
    // 注釈つきのタグは `^{}` の行も付く。
    Ok(lines(&output.stdout).iter().any(|line| {
        line.split_whitespace()
            .nth(1)
            .is_some_and(|name| name.trim_end_matches("^{}") == reference)
    }))
}

fn version_outcome(
    version: &Version,
    kind: Option<Kind>,
    tag: std::result::Result<bool, String>,
) -> Outcome {
    let mut problems = Vec::new();
    if kind == Some(Kind::Stable) && !version.pre.is_empty() {
        problems.push(format!(
            "stable の配布にプレリリースの版は使えません（{version}）"
        ));
    }
    // 試験版は、アプリが「試験版を使う」の設定で見つけられる形の版にする（版の前後が SemVer のとおりに並ぶ形）。
    if kind == Some(Kind::Prerelease) && !is_beta_version(version) {
        problems.push(format!(
            "prerelease の配布の版には alpha.N・beta.N・rc.N のプレリリース識別子が要ります（{version}。例: 0.4.0-rc.1）"
        ));
    }
    if tag == Ok(true) {
        problems.push(format!(
            "タグ v{version} が origin に既にあります（版を上げてください）"
        ));
    }
    if !problems.is_empty() {
        return Failed(problems);
    }
    match tag {
        Err(why) => Skipped(format!(
            "origin に問い合わせられず、タグ v{version} がまだ無いことは確かめていません（{why}）"
        )),
        Ok(_) => Passed(format!("版 {version}・タグ v{version} はまだ無い")),
    }
}

fn check_version(root: &Path, kind: Option<Kind>) -> Outcome {
    match workspace_version() {
        Ok(version) => version_outcome(&version, kind, tag_exists(root, "origin", &version)),
        Err(error) => Failed(vec![format!("版を取得できません: {error}")]),
    }
}

/// `dist-targets.json` の対象が、この xtask が配れる対象であること。Windows の zip はインストーラーと並べて出す（`updater-json` の決まり）。
/// macOS の universal は macOS の runner でビルドし（`lipo`・`codesign` は macOS にしか無い）、2 つの Rust のターゲットを入れる。
/// `default`（入力の既定が入）は入力のある対象だけ。
fn targets_outcome(targets: &[DistTarget]) -> Outcome {
    let mut problems = Vec::new();
    for item in targets {
        if !is_archive_target(&item.target) {
            problems.push(format!(
                "{}: xtask が配れる対象ではありません（build・bundle が断る）",
                item.target
            ));
        }
        if item.installer != (item.target == WINDOWS_ARCHIVE) {
            problems.push(format!(
                "{}: installer は Windows（{WINDOWS_ARCHIVE}）だけ true にする（zip はインストーラーと並べて出し、インストーラーを作れるのは Windows だけ）",
                item.target
            ));
        }
        if item.default_on && item.input.is_none() {
            problems.push(format!(
                "{}: default は入力（input）のある対象だけに付ける（入力が無い対象は常にビルドする）",
                item.target
            ));
        }
        // 試作の対象は、落ちても配布が残りの対象で進む。Windows の zip・インストーラーは更新の対象で、欠けると更新が止まるので、試作にしない
        if item.experimental && item.target == WINDOWS_ARCHIVE {
            problems.push(format!(
                "{}: Windows は試作にできません（experimental が true だと、ビルドが落ちても配布が進み、更新の対象が欠ける）",
                item.target
            ));
        }
        let is_macos = item.target == MACOS_ARCHIVE;
        if is_macos && !item.experimental {
            problems.push(format!(
                "{}: macOS の配布物は試作です（experimental を true にする。署名なしの試作が、落ちたときに配布を止めないため）",
                item.target
            ));
        }
        if is_macos && !item.os.starts_with("macos-") {
            problems.push(format!(
                "{}: macOS の配布物は macOS の runner でビルドする（os が {}。lipo・codesign・iconutil・ditto は macOS にしか無い）",
                item.target, item.os
            ));
        }
        let wanted: Vec<String> = if is_macos {
            MACOS_TRIPLES.iter().map(|t| (*t).to_owned()).collect()
        } else {
            Vec::new()
        };
        if item.rust_targets != wanted {
            problems.push(format!(
                "{}: rust_targets は {:?} にする（今は {:?}。universal の .app を作る 2 つの Rust のターゲットを、ワークフローが入れる）",
                item.target, wanted.join(" "), item.rust_targets.join(" ")
            ));
        }
    }
    if !targets.iter().any(|t| t.input.is_none()) {
        problems.push("入力が要らない（常に組む）対象がありません".into());
    }
    if problems.is_empty() {
        Passed(
            targets
                .iter()
                .map(|t| {
                    let base = match (&t.input, t.default_on) {
                        (Some(input), true) => format!("{}（入力 {input}・既定は入）", t.target),
                        (Some(input), false) => format!("{}（入力 {input}）", t.target),
                        (None, _) => t.target.clone(),
                    };
                    if t.experimental {
                        format!("{base}（試作）")
                    } else {
                        base
                    }
                })
                .collect::<Vec<_>>()
                .join("・"),
        )
    } else {
        Failed(problems)
    }
}

fn check_targets(root: &Path) -> Outcome {
    match dist_targets(root) {
        Ok(targets) => targets_outcome(&targets),
        Err(error) => Failed(vec![format!(
            "tools/dist-targets.json を読めません: {error}"
        )]),
    }
}

fn check_workflows(root: &Path) -> Outcome {
    match python(root).arg("tools/check-workflows.py").output() {
        Err(error) => Failed(vec![format!("Python を起動できません: {error}")]),
        Ok(output) => match output.status.code() {
            Some(0) => Passed(lines(&output.stdout).into_iter().next().unwrap_or_default()),
            // PyYAML が無い。YAML の読み取りは飛ばす（失敗にはしない。飛ばしたと表示する）。
            Some(3) => Skipped(lines(&output.stderr).into_iter().next().unwrap_or_default()),
            _ => Failed(limited(lines(&output.stderr))),
        },
    }
}

fn check_installer(root: &Path) -> Outcome {
    match python(root).arg("tools/test-installer.py").output() {
        Err(error) => Failed(vec![format!("Python を起動できません: {error}")]),
        Ok(output) if output.status.success() => {
            Passed("Wine でインストーラーの試験が通りました".into())
        }
        Ok(output) => {
            let mut all = lines(&output.stdout);
            all.extend(lines(&output.stderr));
            let tail = all.split_off(all.len().saturating_sub(LISTED));
            Failed(tail)
        }
    }
}

// ───────── 実行 ─────────

struct Report {
    name: String,
    outcome: Outcome,
}

fn report(out: &mut impl Write, name: &str, outcome: Outcome) -> Result<Report> {
    match &outcome {
        Passed(detail) => writeln!(out, "[成功] {name}: {detail}")?,
        Skipped(why) => writeln!(out, "[飛ばした] {name}: {why}")?,
        Failed(problems) => {
            writeln!(out, "[失敗] {name}")?;
            for problem in problems {
                writeln!(out, "  - {problem}")?;
            }
        }
    }
    Ok(Report {
        name: name.to_owned(),
        outcome,
    })
}

pub(crate) fn run(args: impl Iterator<Item = String>) -> Result<()> {
    let options = parse_options(args)?;
    let root = root();
    let known = dist_targets(&root)?;
    let targets: Vec<String> = if options.targets.is_empty() {
        // PR の CI がビルドする対象（入力が要らない対象と、入力の既定が入の対象）
        known
            .iter()
            .filter(|t| t.input.is_none() || t.default_on)
            .map(|t| t.target.clone())
            .collect()
    } else {
        options.targets.clone()
    };
    let chosen = selected(&options);
    let mut out = std::io::stdout();
    writeln!(out, "事前確認（対象: {}）", targets.join("・"))?;
    let mut reports = Vec::new();
    for name in chosen {
        match name {
            "licenses" => {
                for target in &targets {
                    let outcome = check_licenses(&root, target, options.offline);
                    reports.push(report(&mut out, &format!("licenses {target}"), outcome)?);
                }
            }
            "attributes" => reports.push(report(&mut out, name, check_attributes(&root))?),
            "nsis" => {
                // インストーラーを作る対象（Windows）が無いなら、台本は使わない。
                let uses_installer = targets.iter().any(|target| {
                    known
                        .iter()
                        .find(|t| &t.target == target)
                        .map_or(*target == WINDOWS_ARCHIVE, |t| t.installer)
                });
                let outcome = if uses_installer {
                    check_nsis(&root)
                } else {
                    Skipped("インストーラーを作る対象がありません".into())
                };
                reports.push(report(&mut out, name, outcome)?)
            }
            "mcpb" => {
                // .mcpb を作るのは Windows の対象だけ（installer を作る対象と同じ）
                let uses_extension = targets.iter().any(|target| *target == WINDOWS_ARCHIVE);
                let outcome = if uses_extension {
                    check_mcpb(&root, options.offline)
                } else {
                    Skipped("拡張（.mcpb）を作る対象がありません".into())
                };
                reports.push(report(&mut out, name, outcome)?)
            }
            "macos" => {
                // universal の .app を作る対象があるときだけ（許諾だけを見る macOS の Rust のターゲットの指定では見ない）
                let uses_app = targets.iter().any(|target| target == MACOS_ARCHIVE);
                let outcome = if uses_app {
                    check_macos(&root)
                } else {
                    Skipped("macOS の .app を作る対象がありません".into())
                };
                reports.push(report(&mut out, name, outcome)?)
            }
            "version" => reports.push(report(&mut out, name, check_version(&root, options.kind))?),
            "targets" => reports.push(report(&mut out, name, check_targets(&root))?),
            "workflows" => reports.push(report(&mut out, name, check_workflows(&root))?),
            "installer" => reports.push(report(&mut out, name, check_installer(&root))?),
            _ => unreachable!("selected は CHECKS の名前だけ"),
        }
        out.flush()?;
    }
    let count = |want: fn(&Outcome) -> bool| reports.iter().filter(|r| want(&r.outcome)).count();
    let (passed, skipped, failed) = (
        count(|o| matches!(o, Passed(_))),
        count(|o| matches!(o, Skipped(_))),
        count(|o| matches!(o, Failed(_))),
    );
    writeln!(
        out,
        "結果: 成功 {passed}・飛ばした {skipped}・失敗 {failed}"
    )?;
    if failed > 0 {
        let names: Vec<&str> = reports
            .iter()
            .filter(|r| matches!(r.outcome, Failed(_)))
            .map(|r| r.name.as_str())
            .collect();
        return Err(format!("事前確認に失敗しました（{}）", names.join("・")).into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT: AtomicUsize = AtomicUsize::new(0);
    struct Scratch(PathBuf);
    impl Scratch {
        fn new() -> Self {
            let path = root().join("target/xtask-tests").join(format!(
                "preflight-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn args(list: &[&str]) -> impl Iterator<Item = String> {
        list.iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
            .into_iter()
    }
    fn git(dir: &Path, list: &[&str]) {
        let status = Command::new("git")
            .current_dir(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@example.invalid"])
            .args(list)
            .stdout(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "{list:?}");
    }
    /// 作業コピーのある一時の git リポジトリ。
    fn repository(scratch: &Scratch, attributes: &str, files: &[&str]) -> Vec<String> {
        git(&scratch.0, &["init", "-q"]);
        fs::write(scratch.0.join(".gitattributes"), attributes).unwrap();
        for file in files {
            let path = scratch.0.join(file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "表記\n").unwrap();
        }
        files.iter().map(|f| f.to_string()).collect()
    }

    #[test]
    fn options_are_parsed_and_unknown_ones_refused() {
        let options = parse_options(args(&[
            "--target",
            "x86_64-pc-windows-msvc",
            "--kind",
            "stable",
            "--only",
            "version,nsis",
            "--installer",
            "--offline",
        ]))
        .unwrap();
        assert_eq!(options.targets, ["x86_64-pc-windows-msvc"]);
        assert_eq!(options.kind, Some(Kind::Stable));
        assert_eq!(options.only, ["version", "nsis"]);
        assert!(options.installer && options.offline);
        // macOS の universal の配布物と、それを作る 2 つの Rust のターゲット（許諾の照合のために）も指せる
        let mac = parse_options(args(&[
            "--target",
            "universal-apple-darwin",
            "--target",
            "aarch64-apple-darwin",
            "--target",
            "x86_64-apple-darwin",
        ]))
        .unwrap();
        assert_eq!(
            mac.targets,
            [
                "universal-apple-darwin",
                "aarch64-apple-darwin",
                "x86_64-apple-darwin"
            ]
        );
        for bad in [
            vec!["--kind", "beta"],
            vec!["--only", "nothing"],
            vec!["--target", "aarch64-pc-windows-msvc"],
            vec!["--target"],
            vec!["--unknown"],
        ] {
            assert!(parse_options(args(&bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn default_checks_leave_out_the_wine_installer_test_unless_asked() {
        let default = selected(&Options::default());
        assert_eq!(
            default,
            [
                "licenses",
                "attributes",
                "nsis",
                "mcpb",
                "macos",
                "version",
                "targets",
                "workflows"
            ]
        );
        let asked = selected(&Options {
            installer: true,
            ..Options::default()
        });
        assert_eq!(asked.last(), Some(&"installer"));
        let only = selected(&Options {
            only: vec!["version".into()],
            ..Options::default()
        });
        assert_eq!(only, ["version"]);
        let only_with_installer = selected(&Options {
            only: vec!["version".into()],
            installer: true,
            ..Options::default()
        });
        assert_eq!(only_with_installer, ["version", "installer"]);
    }

    #[test]
    fn nsis_target_must_be_a_stub_the_official_windows_nsis_has() {
        // 0.3.0 の配布で落ちた形（公式の Windows 版に amd64 の stub は無い）。
        assert!(matches!(
            nsis_outcome("Target amd64-unicode\nName \"x\"\n"),
            Failed(ref p) if p[0].contains("amd64-unicode")
        ));
        assert_eq!(
            nsis_outcome("\u{feff}; 説明\nTarget x86-unicode\n"),
            Passed("Target x86-unicode".into())
        );
        assert!(matches!(nsis_outcome("  target  x86-ansi\n"), Passed(_)));
        // 指定が無い・コメントの中だけ・片方が不正なときは断る。
        assert!(matches!(nsis_outcome("Name \"x\"\n"), Failed(_)));
        assert!(matches!(nsis_outcome("; Target x86-unicode\n"), Failed(_)));
        assert!(matches!(
            nsis_outcome("Target x86-unicode\nTarget amd64-ansi\n"),
            Failed(_)
        ));
        // 実際の台本は通る。
        assert!(
            matches!(check_nsis(&root()), Passed(_)),
            "{:?}",
            check_nsis(&root())
        );
    }

    #[test]
    fn attribute_output_is_parsed_into_problems() {
        let output = b"a.md\0text\0set\0a.md\0eol\0lf\0\
                       b.md\0text\0unset\0b.md\0eol\0unspecified\0\
                       c.md\0text\0unspecified\0c.md\0eol\0unspecified\0\
                       d.md\0text\0set\0d.md\0eol\0unspecified\0\
                       e.md\0text\0auto\0e.md\0eol\0lf\0";
        assert_eq!(
            attribute_problems(output),
            [
                "c.md: text=unspecified eol=unspecified",
                "d.md: text=set eol=unspecified"
            ]
        );
        assert!(attribute_problems(b"").is_empty());
    }

    #[test]
    fn reviewed_texts_without_lf_or_no_text_are_named_by_the_real_git() {
        let s = Scratch::new();
        let files = repository(
            &s,
            "pinned/lf.md text eol=lf\npinned/binary.md -text\npinned/bin.png binary\nloose/only-text.md text\n",
            &[
                "pinned/lf.md",
                "pinned/binary.md",
                "pinned/bin.png",
                "loose/none.md",
                "loose/only-text.md",
            ],
        );
        match check_attributes_in(&s.0, &files) {
            Failed(problems) => {
                let text = problems.join("\n");
                assert!(text.contains("loose/none.md: text=unspecified eol=unspecified"));
                assert!(text.contains("loose/only-text.md: text=set eol=unspecified"));
                assert!(!text.contains("pinned/"), "{text}");
            }
            other => panic!("{other:?}"),
        }
        // 全部が固定されていれば通る。
        fs::write(
            s.0.join(".gitattributes"),
            "pinned/** text eol=lf\nloose/none.md -text\nloose/only-text.md eol=lf\n",
        )
        .unwrap();
        assert!(matches!(check_attributes_in(&s.0, &files), Passed(_)));
        // 改行コードの変換を受ける指定（text=auto）だけで eol が無い物は断る。
        fs::write(s.0.join(".gitattributes"), "* text=auto\n").unwrap();
        assert!(matches!(check_attributes_in(&s.0, &files), Failed(_)));
    }

    #[test]
    fn many_failures_are_listed_up_to_a_limit() {
        let many: Vec<String> = (0..LISTED + 5).map(|n| format!("f{n}")).collect();
        let shown = limited(many);
        assert_eq!(shown.len(), LISTED + 1);
        assert_eq!(shown.last().unwrap(), "ほか 5 件");
    }

    /// 0.3.0 の配布で落ちた失敗の再発防止: この repo の、SHA-256 で照合する表記ファイルが LF 固定か変換なしであること。
    #[test]
    fn this_repositorys_reviewed_notices_are_pinned_against_crlf() {
        let root = root();
        if !root.join(".git").exists() {
            eprintln!("git の作業コピーではないので、属性の確かめを飛ばす");
            return;
        }
        let files = reviewed_files(&root).unwrap();
        assert!(
            files.iter().any(|f| f.ends_with("THIRD-PARTY-NOTICES.md")),
            "{files:?}"
        );
        // 上流の記載から組み立てた許諾の文（クレートの原文としてリポジトリに置いた物）も、LF 固定を見張る対象
        assert!(
            files
                .iter()
                .any(|f| f.starts_with("tools/license-texts/") && f.ends_with(".txt")),
            "{files:?}"
        );
        assert_eq!(
            check_attributes_in(&root, &files),
            Passed(format!("{} ファイルが LF 固定か変換なし", files.len()))
        );
    }

    #[test]
    fn version_tag_and_kind_are_judged() {
        let release = Version::parse("1.2.3").unwrap();
        let pre = Version::parse("1.2.3-rc.1").unwrap();
        assert!(matches!(
            version_outcome(&release, Some(Kind::Stable), Ok(false)),
            Passed(_)
        ));
        assert!(matches!(
            version_outcome(&release, None, Ok(false)),
            Passed(_)
        ));
        assert!(matches!(
            version_outcome(&pre, Some(Kind::Prerelease), Ok(false)),
            Passed(_)
        ));
        // 試験版は alpha.N・beta.N・rc.N の版だけ（正式版の形・知らない名前・数の無い識別子・つなげた書き方は断る）。
        for bad in [
            "1.2.3",
            "1.2.3-preview.1",
            "1.2.3-rc",
            "1.2.3-rc1",
            "1.2.3-rc.1.2",
        ] {
            match version_outcome(
                &Version::parse(bad).unwrap(),
                Some(Kind::Prerelease),
                Ok(false),
            ) {
                Failed(p) => assert!(p[0].contains("rc.N") && p[0].contains(bad), "{p:?}"),
                other => panic!("{bad}: {other:?}"),
            }
        }
        for good in ["1.2.3-alpha.1", "1.2.3-beta.2", "1.2.3-rc.10"] {
            assert!(matches!(
                version_outcome(
                    &Version::parse(good).unwrap(),
                    Some(Kind::Prerelease),
                    Ok(false)
                ),
                Passed(_)
            ));
        }
        // 種類を言わない確かめは、版の形を見ない（手元の既定の確かめで、0.4.0 のような正式版の作業中も通る）。
        assert!(matches!(
            version_outcome(&Version::parse("1.2.3-preview.1").unwrap(), None, Ok(false)),
            Passed(_)
        ));
        // タグが既にある。
        match version_outcome(&release, Some(Kind::Stable), Ok(true)) {
            Failed(p) => assert!(p[0].contains("v1.2.3")),
            other => panic!("{other:?}"),
        }
        // stable にプレリリースの版。
        assert!(matches!(
            version_outcome(&pre, Some(Kind::Stable), Ok(false)),
            Failed(_)
        ));
        // 両方とも並べる。
        match version_outcome(&pre, Some(Kind::Stable), Ok(true)) {
            Failed(p) => assert_eq!(p.len(), 2),
            other => panic!("{other:?}"),
        }
        // 問い合わせられないときは飛ばしたと言う（成功にも失敗にもしない）が、版そのものの失敗は並べる。
        assert!(matches!(
            version_outcome(&release, Some(Kind::Stable), Err("ネットが無い".into())),
            Skipped(ref why) if why.contains("確かめていません")
        ));
        assert!(matches!(
            version_outcome(&pre, Some(Kind::Stable), Err("ネットが無い".into())),
            Failed(_)
        ));
    }

    #[test]
    fn existing_tag_is_found_in_a_real_remote_and_unreachable_remotes_are_errors() {
        let s = Scratch::new();
        git(&s.0, &["init", "-q"]);
        git(&s.0, &["commit", "-q", "--allow-empty", "-m", "初回"]);
        git(&s.0, &["tag", "v1.2.3"]);
        git(&s.0, &["tag", "-a", "-m", "注釈つき", "v2.0.0"]);
        let remote = s.0.to_str().unwrap();
        let v = |text: &str| Version::parse(text).unwrap();
        assert_eq!(tag_exists(&s.0, remote, &v("1.2.3")), Ok(true));
        assert_eq!(tag_exists(&s.0, remote, &v("2.0.0")), Ok(true));
        assert_eq!(tag_exists(&s.0, remote, &v("1.2.4")), Ok(false));
        // v1.2 は v1.2.3 の前方一致ではない（refs/tags/v1.2 の完全一致だけ）。
        assert_eq!(tag_exists(&s.0, remote, &v("1.2.0")), Ok(false));
        let missing = s.0.join("nothing-here");
        assert!(tag_exists(&s.0, missing.to_str().unwrap(), &v("1.2.3")).is_err());
    }

    #[test]
    fn dist_targets_are_validated_against_what_xtask_can_ship() {
        let good = parse_dist_targets(
            r#"{"targets":[
                {"target":"x86_64-pc-windows-msvc","os":"windows-latest","installer":true,"input":null},
                {"target":"x86_64-unknown-linux-gnu","os":"ubuntu-22.04","installer":false,"input":"linux"},
                {"target":"universal-apple-darwin","os":"macos-14","installer":false,"input":"macos","default":true,
                 "experimental":true,"timeout_minutes":90,
                 "rust_targets":"aarch64-apple-darwin x86_64-apple-darwin"}]}"#,
        )
        .unwrap();
        assert!(
            matches!(targets_outcome(&good), Passed(ref s) if s.contains("入力 linux")
            && s.contains("universal-apple-darwin（入力 macos・既定は入）（試作）"))
        );
        assert!(good[2].experimental && !good[0].experimental && !good[1].experimental);
        assert_eq!(good[2].timeout_minutes, Some(90));
        assert_eq!(good[0].timeout_minutes, None);
        assert_eq!(good[2].rust_targets, MACOS_TRIPLES);
        assert!(good[2].default_on && !good[1].default_on && good[0].rust_targets.is_empty());
        // 配れない対象・Linux にインストーラー・Windows にインストーラー無し・常に組む対象が無い。
        let mut unknown = good.clone();
        unknown[1].target = "aarch64-apple-darwin".into();
        assert!(matches!(targets_outcome(&unknown), Failed(_)));
        let mut linux_installer = good.clone();
        linux_installer[1].installer = true;
        assert!(matches!(targets_outcome(&linux_installer), Failed(_)));
        let mut no_installer = good.clone();
        no_installer[0].installer = false;
        assert!(matches!(targets_outcome(&no_installer), Failed(_)));
        let mut all_optional = good.clone();
        all_optional[0].input = Some("windows".into());
        assert!(matches!(targets_outcome(&all_optional), Failed(_)));
        // macOS の配布物: macOS 以外の runner・Rust のターゲットの食い違い・足りない
        let mut wrong_runner = good.clone();
        wrong_runner[2].os = "ubuntu-22.04".into();
        match targets_outcome(&wrong_runner) {
            Failed(p) => assert!(p.iter().any(|l| l.contains("macOS の runner")), "{p:?}"),
            other => panic!("{other:?}"),
        }
        for rust in [
            vec![],
            vec!["aarch64-apple-darwin".to_owned()],
            vec![
                "x86_64-apple-darwin".to_owned(),
                "aarch64-apple-darwin".to_owned(),
            ],
        ] {
            let mut wrong = good.clone();
            wrong[2].rust_targets = rust;
            match targets_outcome(&wrong) {
                Failed(p) => assert!(p.iter().any(|l| l.contains("rust_targets")), "{p:?}"),
                other => panic!("{other:?}"),
            }
        }
        // Rust のターゲットは macOS の配布物だけが持つ（Windows に付けると、ワークフローが入れる物と食い違う）
        let mut stray = good.clone();
        stray[0].rust_targets = vec!["x86_64-pc-windows-msvc".into()];
        assert!(matches!(targets_outcome(&stray), Failed(_)));
        // Windows は試作にできない・macOS は試作でなければならない
        let mut windows_experimental = good.clone();
        windows_experimental[0].experimental = true;
        match targets_outcome(&windows_experimental) {
            Failed(p) => assert!(
                p.iter().any(|l| l.contains("Windows は試作にできません")),
                "{p:?}"
            ),
            other => panic!("{other:?}"),
        }
        let mut macos_required = good.clone();
        macos_required[2].experimental = false;
        match targets_outcome(&macos_required) {
            Failed(p) => assert!(
                p.iter().any(|l| l.contains("macOS の配布物は試作です")),
                "{p:?}"
            ),
            other => panic!("{other:?}"),
        }
        // default は入力のある対象だけ（入力が無い対象は常にビルドする）
        let mut default_without_input = good.clone();
        default_without_input[0].default_on = true;
        assert!(
            matches!(targets_outcome(&default_without_input), Failed(ref p) if p[0].contains("default"))
        );
        assert!(parse_dist_targets("{}").is_err());
        assert!(
            parse_dist_targets(r#"{"targets":[{"target":"x","installer":1,"input":null}]}"#)
                .is_err()
        );
        for bad in [
            r#""default":"yes""#,
            r#""rust_targets":["a"]"#,
            r#""experimental":"yes""#,
            r#""timeout_minutes":0"#,
            r#""timeout_minutes":"90""#,
        ] {
            let text = format!(
                r#"{{"targets":[{{"target":"x","os":"o","installer":false,"input":"i",{bad}}}]}}"#
            );
            assert!(parse_dist_targets(&text).is_err(), "{bad}");
        }
        // 実際の一覧は通る。
        assert!(
            matches!(check_targets(&root()), Passed(_)),
            "{:?}",
            check_targets(&root())
        );
    }

    #[test]
    fn the_macos_check_wants_the_plist_the_logo_the_listed_docs_and_on_a_mac_the_tools() {
        let root = root();
        let version = Version::parse("0.6.0").unwrap();
        let all = |_: &str| true;
        let none = |_: &str| false;
        // macOS 以外ではツール・Rust のターゲットを見ない（そこではビルドしない）
        assert!(macos_problems(&root, &version, false, none, none).is_empty());
        assert!(macos_problems(&root, &version, true, all, all).is_empty());
        // macOS の上でツールが無い・片方のターゲットが無いと、名前つきで並べる
        let problems = macos_problems(
            &root,
            &version,
            true,
            |tool| tool != "iconutil",
            |t| t != "x86_64-apple-darwin",
        );
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(problems[0].contains("iconutil") && problems[1].contains("x86_64-apple-darwin"));
        assert!(problems[1].contains("rustup target add aarch64-apple-darwin x86_64-apple-darwin"));
        // ロゴが無い・文書が一覧に載っていない repo は断る
        let s = Scratch::new();
        fs::create_dir_all(s.0.join("docs")).unwrap();
        fs::write(s.0.join("docs/NEW.md"), "x").unwrap();
        let problems = macos_problems(&s.0, &version, false, all, all);
        assert!(problems.iter().any(|p| p.contains("ロゴ")), "{problems:?}");
        assert!(
            problems.iter().any(|p| p.contains("docs/NEW.md")),
            "{problems:?}"
        );
        // この repo の実際の確かめは、macOS 以外では（ツールを見ないので）通る
        if !cfg!(target_os = "macos") {
            assert!(
                matches!(check_macos(&root), Passed(_)),
                "{:?}",
                check_macos(&root)
            );
        }
    }

    #[test]
    fn workflows_check_runs_the_real_script_or_says_it_was_skipped() {
        // PyYAML が無い環境では飛ばした（失敗にしない）。ある環境では、この repo のワークフローが通る。
        match check_workflows(&root()) {
            Passed(_) | Skipped(_) => {}
            Failed(problems) => panic!("{problems:?}"),
        }
        // 壊れた YAML を置いた repo は、PyYAML があれば失敗する。
        let s = Scratch::new();
        fs::create_dir_all(s.0.join(".github/workflows")).unwrap();
        fs::create_dir_all(s.0.join("tools")).unwrap();
        fs::copy(
            root().join("tools/dist-targets.json"),
            s.0.join("tools/dist-targets.json"),
        )
        .unwrap();
        fs::write(s.0.join(".github/workflows/ci.yml"), "on: [\njobs: x: y\n").unwrap();
        let output = python(&root())
            .args(["tools/check-workflows.py", "--root"])
            .arg(&s.0)
            .output()
            .unwrap();
        match output.status.code() {
            Some(1) => assert!(String::from_utf8_lossy(&output.stderr).contains("ci.yml")),
            Some(3) => eprintln!("PyYAML が無いので、壊れた YAML の確かめを飛ばす"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn license_check_reports_a_failing_script_by_its_lines() {
        // 存在しない対象は third-party.py が断る（引数の選択肢にない）。理由の行が並ぶ。
        match check_licenses(&root(), "i686-pc-windows-msvc", true) {
            Failed(problems) => assert!(!problems.is_empty()),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn report_prints_each_failure_on_its_own_line() {
        let mut out = Vec::new();
        let r = report(&mut out, "x", Failed(vec!["一".into(), "二".into()])).unwrap();
        report(&mut out, "y", Skipped("理由".into())).unwrap();
        report(&mut out, "z", Passed("詳細".into())).unwrap();
        assert!(matches!(r.outcome, Failed(_)));
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "[失敗] x\n  - 一\n  - 二\n[飛ばした] y: 理由\n[成功] z: 詳細\n"
        );
    }
}
