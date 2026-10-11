mod macos;
mod netguard;
mod preflight;

use ed25519_dalek::{Signer, SigningKey};
use semver::Version;
use std::{
    env,
    error::Error,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
};
use yolu_update::{
    asset_name, asset_url, check_public_key, is_archive_target, is_beta_version, sha256, Asset,
    Envelope, Manifest, Transport, UpdateClient, MACOS_ARCHIVE, MAX_ASSET, MAX_METADATA,
    RELEASE_BASE, TARGETS, UPDATER_FILE, UPDATER_SCHEMA, WINDOWS_ARCHIVE, WINDOWS_INSTALLER,
};
type Result<T> = std::result::Result<T, Box<dyn Error>>;
const PRIVATE_KEY_ENV: &str = "YOLUPAINTER_UPDATE_PRIVATE_KEY";
const PUBLIC_KEY_ENV: &str = "YOLUPAINTER_UPDATE_PUBLIC_KEY";
/// exe とインストーラーのアイコン（ロゴ。build.rs も同じファイルを読む）。
const LOGO_ICON: &str = "crates/yolu-app/assets/logo/yolupainter.ico";
const USAGE: &str = "命令: preflight [--target T]... [--kind stable|prerelease] [--only 確かめ,...] [--installer] [--offline] / netguard / build --target T --release [--require-update-key] / bundle --target T（build・bundle の T は x86_64-pc-windows-msvc・x86_64-unknown-linux-gnu・universal-apple-darwin。universal-apple-darwin は macOS の上で） / installer --target T / mcpb --target T / symbols --target T / updater-json --version V --assets DIR [--sign] [--key-file PATH] / verify --version V --assets DIR --public-key HEX / beta-channel --version V --assets DIR --public-key HEX --output DIR [--existing PATH] / keygen --output PATH / pubkey --key-file PATH";
/// リポジトリの根（`crates/xtask` の 2 つ上）。`canonicalize` は使わない: Windows では `\\?\C:\…` の形になり、
/// makensis や Python に渡す道が、その形に対応しているとは限らないため。
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("xtask は crates/ の下にある")
        .to_path_buf()
}
fn run(command: &mut Command) -> Result<()> {
    if !command.status()?.success() {
        return Err("子コマンドが失敗しました".into());
    }
    Ok(())
}
fn cargo() -> Command {
    Command::new(env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
}
/// 許諾の照合ツールを呼ぶ。Windows の runner では標準出力がパイプで、Python の既定の
/// 符号化が ANSI になり日本語の出力で落ちるため、UTF-8 を明示する。
fn python(root: &Path) -> Command {
    let mut command = Command::new(env::var_os("PYTHON").unwrap_or_else(|| "python3".into()));
    command
        .current_dir(root)
        .env("PYTHONUTF8", "1")
        .env("PYTHONIOENCODING", "utf-8");
    command
}
fn value(args: &mut impl Iterator<Item = String>) -> Result<String> {
    args.next().ok_or_else(|| "引数の値がありません".into())
}
fn main() {
    if let Err(error) = execute(env::args().skip(1)) {
        eprintln!("配布処理を完了できません: {error}");
        std::process::exit(1);
    }
}
fn execute(mut args: impl Iterator<Item = String>) -> Result<()> {
    let command = value(&mut args)?;
    if command == "preflight" {
        return preflight::run(args);
    }
    if command == "netguard" {
        return netguard::run(args);
    }
    let mut target = None;
    let mut version = None;
    let mut assets = None;
    let mut key_file = None;
    let mut public_key = None;
    let mut output = None;
    let mut existing = None;
    let mut release = false;
    let mut sign = false;
    let mut require_update_key = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--target"
                if matches!(
                    command.as_str(),
                    "build" | "bundle" | "installer" | "mcpb" | "symbols"
                ) =>
            {
                target = Some(value(&mut args)?)
            }
            "--version"
                if matches!(command.as_str(), "updater-json" | "verify" | "beta-channel") =>
            {
                version = Some(Version::parse(&value(&mut args)?)?)
            }
            "--assets"
                if matches!(command.as_str(), "updater-json" | "verify" | "beta-channel") =>
            {
                assets = Some(PathBuf::from(value(&mut args)?))
            }
            "--key-file" if matches!(command.as_str(), "updater-json" | "pubkey") => {
                key_file = Some(PathBuf::from(value(&mut args)?))
            }
            "--public-key" if matches!(command.as_str(), "verify" | "beta-channel") => {
                public_key = Some(value(&mut args)?)
            }
            "--output" if matches!(command.as_str(), "keygen" | "beta-channel") => {
                output = Some(PathBuf::from(value(&mut args)?))
            }
            "--existing" if command == "beta-channel" => {
                existing = Some(PathBuf::from(value(&mut args)?))
            }
            "--release" if command == "build" => release = true,
            "--require-update-key" if command == "build" => require_update_key = true,
            "--sign" if command == "updater-json" => sign = true,
            _ => return Err(format!("未対応の引数: {arg}").into()),
        }
    }
    match command.as_str() {
        "build" | "bundle" | "installer" | "mcpb" | "symbols" => {
            let target = target.ok_or("--target が必要です")?;
            if !is_archive_target(&target) {
                return Err("未対応の配布ターゲットです".into());
            }
            match command.as_str() {
                "build" => {
                    if !release {
                        return Err("配布用ビルドには --release が必要です".into());
                    }
                    build(&target, require_update_key)
                }
                "bundle" => bundle(&target),
                "mcpb" => mcpb(&target),
                "symbols" => symbols(&target),
                _ => installer(&target),
            }
        }
        "updater-json" => {
            if key_file.is_some() && !sign {
                return Err("--key-file は --sign と併用してください".into());
            }
            updater(
                version.ok_or("--version が必要です")?,
                &assets.ok_or("--assets が必要です")?,
                sign,
                || signing_key(key_file.as_deref()),
            )
        }
        "verify" => verify(
            &version.ok_or("--version が必要です")?,
            &assets.ok_or("--assets が必要です")?,
            public_key_bytes(&public_key.ok_or("--public-key が必要です")?)?,
        ),
        "beta-channel" => beta_channel(
            &version.ok_or("--version が必要です")?,
            &assets.ok_or("--assets が必要です")?,
            public_key_bytes(&public_key.ok_or("--public-key が必要です")?)?,
            existing.as_deref(),
            &output.ok_or("--output が必要です")?,
        ),
        "keygen" => {
            println!("公開鍵: {}", keygen(&output.ok_or("--output が必要です")?)?);
            Ok(())
        }
        "pubkey" => {
            let key_file = key_file.ok_or("--key-file が必要です")?;
            let key = signing_key_from(Some(key_file.as_path()), None)?;
            println!("公開鍵: {}", hex::encode(key.verifying_key().to_bytes()));
            Ok(())
        }
        _ => Err(USAGE.into()),
    }
}
fn workspace_version() -> Result<Version> {
    let output = cargo()
        .current_dir(root())
        .args(["metadata", "--locked", "--no-deps", "--format-version", "1"])
        .output()?;
    if !output.status.success() {
        return Err("版を取得できません".into());
    }
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    let package = metadata["packages"]
        .as_array()
        .ok_or("packages がありません")?
        .iter()
        .find(|p| p["name"] == "yolu-app")
        .ok_or("アプリがありません")?;
    Ok(Version::parse(
        package["version"].as_str().ok_or("版がありません")?,
    )?)
}
fn remove_if_present(path: &Path) -> Result<()> {
    if fs::symlink_metadata(path).is_ok() {
        fs::remove_file(path)?;
    }
    Ok(())
}
/// アプリに組み込む更新用の公開鍵。GitHub の変数は未設定だと空文字で渡るので、空は未設定として扱う。
/// 入っているのに鍵として使えないものは、黙って更新なしのビルドにせず断る。
/// `require` のとき（Draft を作る配布）は、未設定も断る（更新できないアプリを配らない）。
fn update_public_key(environment: Option<String>, require: bool) -> Result<Option<String>> {
    let Some(text) = environment
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
    else {
        if require {
            return Err(format!(
                "{PUBLIC_KEY_ENV} が空です。リポジトリ変数に公開鍵を設定してください"
            )
            .into());
        }
        return Ok(None);
    };
    check_public_key(public_key_bytes(&text)?)?;
    Ok(Some(text))
}
/// 1 つの Rust のターゲットのリリースビルド（アプリとコマンドラインを同じ命令で）。更新用の公開鍵は `key` があるときだけ環境に置く。
fn release_build(target: &str, key: &Option<String>) -> Command {
    let mut command = cargo();
    command.current_dir(root()).args([
        "build",
        "--locked",
        "-p",
        "yolu-app",
        // コマンドライン（yolupainter-cli）と MCP サーバーも同じ配布物に入るので、同じ版・同じ組みで作る
        "-p",
        "yolu-cli",
        "--target",
        target,
        "--release",
    ]);
    match key {
        Some(key) => command.env(PUBLIC_KEY_ENV, key),
        // 空の値を渡さない（アプリは「組み込み済み」と見て、使えない鍵を持つ）。
        None => command.env_remove(PUBLIC_KEY_ENV),
    };
    command
}
fn build(target: &str, require_update_key: bool) -> Result<()> {
    let key = update_public_key(env::var(PUBLIC_KEY_ENV).ok(), require_update_key)?;
    if target == MACOS_ARCHIVE {
        // macOS は 2 つのターゲット（Apple Silicon と Intel）でビルドして 1 つにまとめる
        return macos::build(|triple| release_build(triple, &key));
    }
    run(&mut release_build(target, &key))
}
/// 配布物に入れる使う人向けの文書（リポジトリの根からの相対。配布物の中でも同じ場所に入るので、README からの相対のリンクがそのまま効く）。
/// インストーラーの `installer/yolupainter.nsi` の `DocFiles` も同じ一覧で、試験が突き合わせる。
const BUNDLED_DOCS: &[&str] = &[
    "docs/GUIDE.md",
    "docs/GUIDE_START.md",
    "docs/GUIDE_PAINT.md",
    "docs/GUIDE_SELECT.md",
    "docs/GUIDE_LAYERS.md",
    "docs/GUIDE_SETTINGS.md",
    "docs/GUIDE_KEYS.md",
    "docs/CLI.md",
    "docs/MCP.md",
    "docs/UNITY.md",
    "docs/INSTALL.md",
    "docs/BUILDING.md",
    "docs/PSD.md",
    "docs/BRUSH.md",
    "docs/BRUSH_IMPORT.md",
    "docs/SUBTOOLS.md",
    "docs/GRADIENT_MAP.md",
    "docs/PREVIEW.md",
    "docs/RECOVERY.md",
    "docs/SAVE_FOR_DISTRIBUTION.md",
    "docs/YLP_FORMAT.md",
    "docs/YLP_DECISIONS.md",
    "docs/en/GUIDE.md",
    "docs/en/GUIDE_START.md",
    "docs/en/GUIDE_PAINT.md",
    "docs/en/GUIDE_SELECT.md",
    "docs/en/GUIDE_LAYERS.md",
    "docs/en/GUIDE_SETTINGS.md",
    "docs/en/GUIDE_KEYS.md",
    "docs/en/CLI.md",
    "docs/en/MCP.md",
    "docs/en/UNITY.md",
    "docs/en/INSTALL.md",
    "docs/en/BUILDING.md",
    "docs/LIVELINK.md",
    "docs/en/LIVELINK.md",
    "docs/GUIDE_FILL.md",
    "docs/GUIDE_PATHS.md",
    "docs/GUIDE_3D.md",
    "docs/GUIDE_FILES.md",
    "docs/en/GUIDE_FILL.md",
    "docs/en/GUIDE_PATHS.md",
    "docs/en/GUIDE_3D.md",
    "docs/en/GUIDE_FILES.md",
];
/// docs/ にあって配布物へは入れないファイル（開発・リリースの手順）。docs/ に足したファイルは、入れるか外すかのどちらかに必ず載せる（試験が確かめる）。
const LEFT_OUT_DOCS: &[&str] = &["docs/DEVELOPMENT.md", "docs/RELEASING.md"];
/// 配布物の根に入れる、実行ファイルのほかのファイル。許諾の全文の束（`DEPENDENCIES.md`・`THIRD_PARTY_LICENSES.txt`）は
/// 対象ごとに `tools/third-party.py` が作るので、元の場所が `payload_source` で違う。
const ROOT_FILES: &[&str] = &[
    "LICENSE",
    "README.md",
    "README.en.md",
    "THIRD_PARTY.md",
    "DEPENDENCIES.md",
    "THIRD_PARTY_LICENSES.txt",
];
fn exe_name(target: &str) -> &'static str {
    if target.contains("windows") {
        "yolupainter.exe"
    } else {
        "yolupainter"
    }
}
/// コマンドラインと MCP サーバーの実行ファイル（アプリと同じ配布物・同じ版。Windows ではコンソールの実行ファイル）。
fn cli_exe_name(target: &str) -> &'static str {
    if target.contains("windows") {
        "yolupainter-cli.exe"
    } else {
        "yolupainter-cli"
    }
}
/// 配布物の根に入れる実行ファイル（アプリ・コマンドライン）。
fn executables(target: &str) -> [&'static str; 2] {
    [exe_name(target), cli_exe_name(target)]
}
/// アーカイブ（zip・tar.gz）とインストーラーの段に入れるファイルの、配布物の中の名前（`/` 区切り）。ここが唯一の一覧。
fn payload_names(target: &str) -> Vec<String> {
    executables(target)
        .into_iter()
        .chain(ROOT_FILES.iter().copied())
        .chain(BUNDLED_DOCS.iter().copied())
        .map(str::to_owned)
        .collect()
}
/// docs/ の下のファイルの、リポジトリの根からの相対の名前（`/` 区切り・並べ替え済み）。.md 以外（画像など）も数える。
fn docs_files(root: &Path) -> Result<Vec<String>> {
    fn walk(directory: &Path, prefix: &str, out: &mut Vec<String>) -> Result<()> {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| "docs/ のファイル名が UTF-8 ではありません")?;
            let path = format!("{prefix}/{name}");
            if entry.file_type()?.is_dir() {
                walk(&entry.path(), &path, out)?;
            } else {
                out.push(path);
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    walk(&root.join("docs"), "docs", &mut files)?;
    files.sort();
    Ok(files)
}
/// docs/ のファイルが全部「入れる」か「外す」のどちらかに（どちらか一方にだけ）載っていて、載せたファイルが全部あることを確かめる。
/// docs/ に文書を足して、一覧に載せ忘れたまま配布物を作ることを断る。
fn check_docs_listed(root: &Path) -> Result<()> {
    let files = docs_files(root)?;
    let listed: Vec<&str> = BUNDLED_DOCS.iter().chain(LEFT_OUT_DOCS).copied().collect();
    let unlisted: Vec<&String> = files
        .iter()
        .filter(|file| !listed.contains(&file.as_str()))
        .collect();
    let missing: Vec<&str> = listed
        .iter()
        .copied()
        .filter(|name| !files.iter().any(|file| file == name))
        .collect();
    let twice: Vec<&str> = BUNDLED_DOCS
        .iter()
        .copied()
        .filter(|name| LEFT_OUT_DOCS.contains(name))
        .collect();
    if unlisted.is_empty() && missing.is_empty() && twice.is_empty() {
        return Ok(());
    }
    let join = |names: Vec<&str>| names.join("、");
    Err(format!(
        "docs/ のファイルと配布物の一覧が合いません（どちらにも載っていない: {}／一覧にあるのに無い: {}／入れるにも外すにも載っている: {}）。\
         crates/xtask/src/main.rs の BUNDLED_DOCS か LEFT_OUT_DOCS に載せてください",
        join(unlisted.iter().map(|n| n.as_str()).collect()),
        join(missing),
        join(twice),
    )
    .into())
}
/// 配布物の中の名前に対する元のファイル。許諾の束だけは `tools/third-party.py` の出力、実行ファイルはビルドの出力で、
/// それ以外は配布物の中と同じ相対の場所のリポジトリのファイル。
fn payload_source(root: &Path, target: &str, license_dir: &Path, name: &str) -> PathBuf {
    match name {
        "DEPENDENCIES.md" => license_dir.join("THIRD_PARTY.md"),
        "THIRD_PARTY_LICENSES.txt" => license_dir.join("THIRD_PARTY_LICENSES.txt"),
        _ if executables(target).contains(&name) => {
            root.join("target").join(target).join("release").join(name)
        }
        _ => root.join(name),
    }
}
fn payload_entries(root: &Path, target: &str, license_dir: &Path) -> Vec<(String, PathBuf)> {
    payload_names(target)
        .into_iter()
        .map(|name| {
            let source = payload_source(root, target, license_dir, &name);
            (name, source)
        })
        .collect()
}
/// 元のファイルが無い・通常のファイルでないまま梱包して、名前の無い失敗にしない（入れ忘れ・ビルドの忘れをここで名前つきで断る）。
fn require_sources(entries: &[(String, PathBuf)]) -> Result<()> {
    for (name, source) in entries {
        if !fs::metadata(source).is_ok_and(|m| m.is_file()) {
            return Err(format!("配布物に入れるファイルがありません: {name}").into());
        }
    }
    Ok(())
}
/// 対象ごとの許諾の照合と全文の束の作成（`bundle`・`installer` と、事前確認 `preflight` が同じ命令を使う）。組まずに回る。
fn third_party(root: &Path, target: &str) -> Command {
    let mut command = python(root);
    command.args([
        "tools/third-party.py",
        "--package",
        "yolu-app",
        "--include-update",
        // 同じ配布物に入るコマンドライン（yolu-cli）の依存も、同じ照合・同じ全文束に入れる
        "--include-cli",
        "--target",
        target,
        "--bundle",
    ]);
    command
}
/// アーカイブ・インストーラーに入れるファイル（名前 → 元）。許諾の全文の束もここで作る。
fn payload(root: &Path, target: &str) -> Result<Vec<(String, PathBuf)>> {
    check_docs_listed(root)?;
    run(&mut third_party(root, target))?;
    let license_dir = root
        .join("target/third-party")
        .join(target)
        .join("yolu-app");
    let entries = payload_entries(root, target, &license_dir);
    require_sources(&entries)?;
    Ok(entries)
}
fn bundle(target: &str) -> Result<()> {
    if target == MACOS_ARCHIVE {
        return macos::bundle();
    }
    let root = root();
    let version = workspace_version()?;
    let name = asset_name(&version, target)?;
    let out = root.join("target/dist");
    fs::create_dir_all(&out)?;
    // 前回成功した配布物を今回の失敗と取り違えない。
    let destination = out.join(name);
    remove_if_present(&destination)?;
    let entries = payload(&root, target)?;
    write_archive(&destination, &entries, target.contains("windows"))?;
    println!("配布物: {}", destination.display());
    Ok(())
}
/// Windows の配布物の PDB を入れる付属物の名前（Release にだけ載せる）。クラッシュの記録の各フレームの番地から `Image base` を引いた
/// 相対の番地を、同じ版の関数名・行へ引くための物で、配布物（zip・インストーラー）には入れない（利用者に配る必要が無く、大きい）。
/// 更新の対象ではない: 署名つきの更新情報には載せず（アプリは取りに行かない）、`updater-json` はこの名前だけを知って読み飛ばし、
/// `verify` は中身の形だけを見る。
fn symbols_name(version: &Version) -> String {
    format!("yolupainter-{version}-{WINDOWS_ARCHIVE}-pdb.zip")
}
/// PDB の、付属物の zip の中での名前。実行ファイルが指す名前と同じ（デバッガーが探す名前）。
const PDB_FILE: &str = "yolupainter.pdb";
/// 配布物のビルド（`build`）が作った PDB を、`target/dist` の付属物にする。
fn symbols(target: &str) -> Result<()> {
    if target != WINDOWS_ARCHIVE {
        return Err("PDB は Windows（x86_64-pc-windows-msvc）だけです".into());
    }
    let root = root();
    let version = workspace_version()?;
    let pdb = root
        .join("target")
        .join(target)
        .join("release")
        .join(PDB_FILE);
    let out = root.join("target/dist");
    fs::create_dir_all(&out)?;
    let destination = out.join(symbols_name(&version));
    symbols_archive(&pdb, &destination)?;
    println!("付属物: {}", destination.display());
    Ok(())
}
/// `pdb` を、PDB 1 つだけの zip にして `destination` へ置く（ファイルが無い・空・大きすぎるときは、何も置かずに断る）。
/// 前回の付属物は、PDB を調べる前に消す（`bundle` と同じ。組み直しに失敗したあとで、前のビルドの PDB が今回の付属物として
/// `updater-json`・`verify` を通らないように）。
fn symbols_archive(pdb: &Path, destination: &Path) -> Result<()> {
    remove_if_present(destination)?;
    let size = fs::metadata(pdb)
        .map_err(|_| {
            format!(
                "PDB がありません: {}（配布用に `cargo xtask build` で組んだあとに作る。PDB を作る設定は release.yml の「PDB の設定」）",
                pdb.display()
            )
        })?
        .len();
    if size == 0 || size > MAX_ASSET {
        return Err("PDB の大きさが不正です".into());
    }
    write_archive(destination, &[(PDB_FILE.to_owned(), pdb.to_owned())], true)
}
/// Claude Desktop に入れる拡張（.mcpb）の名前。Windows の配布物と同じ版・対象で、Release の付属物にする。PDB の付属物と同じく、
/// 更新の対象ではない（署名つきの更新情報に載せず、アプリは取りに行かない。`updater-json` はこの名前だけを知って読み飛ばし、`verify` は中身の形だけを見る）。
fn mcpb_name(version: &Version) -> String {
    format!("yolupainter-{version}-{WINDOWS_ARCHIVE}.mcpb")
}
/// .mcpb の中の実行ファイル（`server.entry_point`）。
const MCPB_SERVER: &str = "server/yolupainter-cli.exe";
/// .mcpb の中のアイコン。アプリのロゴ（PNG）をそのまま入れる。差し替えるときは `MCPB_LOGO` を替える。
const MCPB_ICON: &str = "icon.png";
const MCPB_LOGO: &str = "crates/yolu-app/assets/logo/yolupainter-1024.png";
/// .mcpb の中のファイルの名前（`/` 区切り）。許諾の全文は、コマンドライン（アプリと同じ組みでの `yolu-cli` の部分木）のもの。
const MCPB_FILES: &[&str] = &[
    "manifest.json",
    MCPB_ICON,
    "LICENSE",
    "DEPENDENCIES.md",
    "THIRD_PARTY_LICENSES.txt",
    MCPB_SERVER,
];
/// 配布物の GitHub の置き場（マニフェストの作者・文書のリンク）。インストーラーの `HOMEPAGE` と同じ。
const HOMEPAGE: &str = "https://github.com/YozoraKurage/YoluPainter";
/// .mcpb の中継に渡す引数（アプリの番号は、拡張の設定 `port` から。既定はアプリの既定と同じ）。
const MCPB_ARGS: [&str; 3] = ["mcp", "--port", "${user_config.port}"];
/// アプリの既定の番号（`yolu_mcp::DEFAULT_PORT` と同じ。xtask はアプリの crate に依らないので、試験が食い違いを見つける）。
const MCPB_DEFAULT_PORT: u16 = 17347;
/// .mcpb の `manifest.json`（mcpb の manifest_version 0.3。`server.type` は binary）。中身は、標準入出力を起動中のアプリの MCP の受け口
/// （`http://127.0.0.1:<番号>/mcp`）へつなぐ中継（`yolupainter-cli mcp`）だけで、ツールの一覧は書かない（`tools_generated`。アプリが返す物が正本になり、
/// アプリの更新だけで新しくなる）。
fn mcpb_manifest(version: &Version) -> serde_json::Value {
    serde_json::json!({
        "manifest_version": "0.3",
        "name": "yolupainter",
        "display_name": "YoluPainter",
        "version": version.to_string(),
        "description": "Work on the project open in YoluPainter: layers, masks, effects, previews and exports. Connects to the app running on this PC.",
        "long_description": "Lets an AI assistant work on the project open in the YoluPainter app on this PC. Turn on \"Accept external commands\" in the app's settings first. This extension only relays to the app, so the tools come from the app and follow its updates. Reading, previewing and editing layers, masks and effects are available; deleting, saving over a file and replacing exported files need an explicit confirmation. Nothing is sent over the network.",
        "author": {"name": "Yozolab", "url": HOMEPAGE},
        "repository": {"type": "git", "url": HOMEPAGE},
        "homepage": HOMEPAGE,
        "documentation": format!("{HOMEPAGE}/blob/main/docs/en/MCP.md"),
        "icon": MCPB_ICON,
        "server": {
            "type": "binary",
            "entry_point": MCPB_SERVER,
            "mcp_config": {
                "command": format!("${{__dirname}}/{MCPB_SERVER}"),
                "args": MCPB_ARGS,
                "env": {},
            },
        },
        "user_config": {
            "port": {
                "type": "number",
                "title": "Port",
                "description": "The port set in YoluPainter's settings next to \"Accept external commands\"",
                "default": MCPB_DEFAULT_PORT,
                "min": 1024,
                "max": 65535,
                "required": false,
            },
        },
        "tools_generated": true,
        "keywords": ["texture", "painting", "3D", "Unity"],
        "license": "MIT",
        "compatibility": {"platforms": ["win32"]},
    })
}
/// .mcpb の `manifest.json` が、この xtask の組む形（版が今の版・実行ファイルが入る名前・Windows だけ・引数は `mcp` と設定の番号）であること。
fn check_mcpb_manifest(manifest: &serde_json::Value, version: &Version) -> Result<()> {
    let text = |pointer: &str| manifest.pointer(pointer).and_then(|v| v.as_str());
    let mut problems = Vec::new();
    if text("/manifest_version") != Some("0.3") {
        problems.push("manifest_version が 0.3 ではありません".to_owned());
    }
    if text("/name") != Some("yolupainter") {
        problems.push("name が yolupainter ではありません".to_owned());
    }
    if text("/version") != Some(version.to_string().as_str()) {
        problems.push(format!("version が {version} ではありません"));
    }
    for required in ["/description", "/author/name"] {
        if text(required).is_none_or(str::is_empty) {
            problems.push(format!("{required} がありません"));
        }
    }
    if text("/server/type") != Some("binary") {
        problems.push("server.type が binary ではありません".to_owned());
    }
    if text("/server/entry_point") != Some(MCPB_SERVER) {
        problems.push(format!(
            "server.entry_point が {MCPB_SERVER} ではありません"
        ));
    }
    let command = text("/server/mcp_config/command").unwrap_or_default();
    if command != format!("${{__dirname}}/{MCPB_SERVER}") {
        problems.push(format!(
            "server.mcp_config.command が実行ファイルを指していません: {command}"
        ));
    }
    if manifest.pointer("/server/mcp_config/args") != Some(&serde_json::json!(MCPB_ARGS)) {
        problems.push(format!(
            "server.mcp_config.args が {MCPB_ARGS:?} ではありません"
        ));
    }
    if manifest.pointer("/user_config/port/type") != Some(&serde_json::json!("number")) {
        problems.push("user_config.port（番号）がありません".to_owned());
    }
    if manifest.pointer("/compatibility/platforms") != Some(&serde_json::json!(["win32"])) {
        problems.push(
            "compatibility.platforms が [\"win32\"] ではありません（実行ファイルは Windows 用）"
                .to_owned(),
        );
    }
    if text("/icon") != Some(MCPB_ICON) {
        problems.push(format!("icon が {MCPB_ICON} ではありません"));
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(format!("manifest.json が不正です（{}）", problems.join("／")).into())
    }
}
/// .mcpb の中身: 入っているファイルが `MCPB_FILES` と同じで、`manifest.json` が `check_mcpb_manifest` を通ること。
fn check_mcpb_archive(bytes: &[u8], version: &Version) -> Result<()> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))?;
    let mut found = Vec::new();
    for index in 0..zip.len() {
        let file = zip.by_index(index)?;
        if file.is_file() {
            found.push(file.name().to_owned());
        }
    }
    found.sort();
    let mut expected: Vec<String> = MCPB_FILES.iter().map(|n| (*n).to_owned()).collect();
    expected.sort();
    if found != expected {
        return Err(format!(
            ".mcpb の中身が一覧と違います（入っている: {}／一覧: {}）",
            found.join("、"),
            expected.join("、")
        )
        .into());
    }
    let mut text = String::new();
    zip.by_name("manifest.json")?
        .take(1 << 20)
        .read_to_string(&mut text)?;
    let manifest: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("manifest.json を読めません: {e}"))?;
    check_mcpb_manifest(&manifest, version)
}
/// コマンドラインの許諾の照合と全文束（`target/third-party/<target>/yolu-cli/`）。.mcpb に入れる。事前確認 `preflight` も同じ命令を使う。
/// `build` は `-p yolu-app -p yolu-cli` の 1 回の組みで、2 つのクレートの機能が合わさる（単独の木に無い依存が入る）ので、
/// 数えるのは、その組みでの yolu-cli の部分木（`--built-with yolu-app`）。単独の木で数えると、詰めた exe に入る依存を載せ落とす。
fn third_party_cli(root: &Path, target: &str) -> Command {
    let mut command = python(root);
    command.args([
        "tools/third-party.py",
        "--package",
        "yolu-cli",
        "--built-with",
        "yolu-app",
        "--target",
        target,
        "--bundle",
    ]);
    command
}
/// Claude Desktop に入れる拡張（.mcpb）。zip に `manifest.json`・アイコン・許諾・実行ファイル 1 つ。
fn mcpb(target: &str) -> Result<()> {
    if target != WINDOWS_ARCHIVE {
        return Err(".mcpb は Windows（x86_64-pc-windows-msvc）だけです".into());
    }
    let root = root();
    let version = workspace_version()?;
    let out = root.join("target/dist");
    fs::create_dir_all(&out)?;
    let destination = out.join(mcpb_name(&version));
    // 前回の成功を今回の失敗と取り違えない。
    remove_if_present(&destination)?;
    run(&mut third_party_cli(&root, target))?;
    let license_dir = root
        .join("target/third-party")
        .join(target)
        .join("yolu-cli");
    let manifest = mcpb_manifest(&version);
    check_mcpb_manifest(&manifest, &version)?;
    let stage = root.join("target/mcpb").join(target);
    let _ = fs::remove_dir_all(&stage);
    fs::create_dir_all(&stage)?;
    let manifest_file = stage.join("manifest.json");
    fs::write(&manifest_file, serde_json::to_vec_pretty(&manifest)?)?;
    let sources = |name: &str| -> PathBuf {
        match name {
            "manifest.json" => manifest_file.clone(),
            MCPB_ICON => root.join(MCPB_LOGO),
            "LICENSE" => root.join("LICENSE"),
            "DEPENDENCIES.md" => license_dir.join("THIRD_PARTY.md"),
            "THIRD_PARTY_LICENSES.txt" => license_dir.join("THIRD_PARTY_LICENSES.txt"),
            _ => root
                .join("target")
                .join(target)
                .join("release")
                .join(cli_exe_name(target)),
        }
    };
    let entries: Vec<(String, PathBuf)> = MCPB_FILES
        .iter()
        .map(|name| ((*name).to_owned(), sources(name)))
        .collect();
    require_sources(&entries)?;
    write_archive(&destination, &entries, true)?;
    println!("配布物: {}", destination.display());
    Ok(())
}
/// NSIS が数字 4 つの版（各 0〜65535）しか受けないので、プレリリース識別子は落とす（文字列の版は別に渡す）。
fn numeric_version(version: &Version) -> Result<String> {
    let part = |n: u64| {
        u16::try_from(n).map_err(|_| "版の数字が 65535 を超えるのでインストーラーに入れられません")
    };
    Ok(format!(
        "{}.{}.{}.0",
        part(version.major)?,
        part(version.minor)?,
        part(version.patch)?
    ))
}
/// makensis に渡す引数（スクリプトの前の -D まで。スクリプトのパスは呼ぶ側が最後に足す）。
fn nsis_args(
    version: &Version,
    stage: &Path,
    outfile: &Path,
    icon: &Path,
) -> Result<Vec<std::ffi::OsString>> {
    let define = |name: &str, value: &std::ffi::OsStr| {
        let mut argument = std::ffi::OsString::from(format!("-D{name}="));
        argument.push(value);
        argument
    };
    Ok(vec![
        // 日本語の文字列を含むスクリプトを BOM の有無によらず UTF-8 として読む。警告もエラーにする。
        "-INPUTCHARSET".into(),
        "UTF8".into(),
        "-WX".into(),
        "-V2".into(),
        define("VERSION", version.to_string().as_ref()),
        define("VERSION_NUMERIC", numeric_version(version)?.as_ref()),
        define("STAGE", stage.as_os_str()),
        define("OUTFILE", outfile.as_os_str()),
        define("ICON", icon.as_os_str()),
    ])
}
fn makensis() -> Command {
    Command::new(env::var_os("MAKENSIS").unwrap_or_else(|| "makensis".into()))
}
/// インストーラーに渡す段取り用のフォルダを、前回のものを消して作り直す。名前の `/` はフォルダ（`docs/en/GUIDE.md`）なので、親を作ってから写す。
fn stage_payload(entries: &[(String, PathBuf)], stage: &Path) -> Result<()> {
    let _ = fs::remove_dir_all(stage);
    fs::create_dir_all(stage)?;
    for (name, source) in entries {
        let destination = stage.join(name);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(source, destination)?;
    }
    Ok(())
}
/// Windows のインストーラー（NSIS）。zip と同じファイルを段取り用のフォルダ（target/ の中）に集めて渡す。
/// 一時ファイルに作ってから最後に 1 回の rename で置くので、失敗した作りかけを配布物として見せない。
fn installer(target: &str) -> Result<()> {
    if target != WINDOWS_ARCHIVE {
        return Err("インストーラーは Windows（x86_64-pc-windows-msvc）だけです".into());
    }
    let root = root();
    let version = workspace_version()?;
    let name = asset_name(&version, WINDOWS_INSTALLER)?;
    let out = root.join("target/dist");
    fs::create_dir_all(&out)?;
    let destination = out.join(&name);
    remove_if_present(&destination)?;
    let entries = payload(&root, target)?;
    let stage = root.join("target/installer").join(target);
    stage_payload(&entries, &stage)?;
    let temporary = out.join(format!("{name}.tmp"));
    let _ = fs::remove_file(&temporary);
    let built = nsis_args(&version, &stage, &temporary, &root.join(LOGO_ICON))
        .and_then(|args| {
            run(makensis()
                .current_dir(&root)
                .args(args)
                .arg(root.join("installer/yolupainter.nsi")))
        })
        .and_then(|_| Ok(fs::rename(&temporary, &destination)?));
    if let Err(error) = built {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    println!("配布物: {}", destination.display());
    Ok(())
}
/// 旧い配布物を消し、一時ファイルに書いてから最後に 1 回の rename で置く。
/// 失敗したときは旧い配布物も一時ファイルも残さず、途中の書きかけを配布物として見せない。
fn write_archive(destination: &Path, entries: &[(String, PathBuf)], windows: bool) -> Result<()> {
    remove_if_present(destination)?;
    let temporary = destination.with_extension("tmp");
    if let Err(error) = archive(&temporary, entries, windows) {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    if let Err(error) = fs::rename(&temporary, destination) {
        let _ = fs::remove_file(&temporary);
        return Err(error.into());
    }
    Ok(())
}
fn archive(path: &Path, entries: &[(String, PathBuf)], windows: bool) -> Result<()> {
    let file = File::create(path)?;
    if windows {
        let mut zip = zip::ZipWriter::new(file);
        for (name, source) in entries {
            zip.start_file(
                name,
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated),
            )?;
            std::io::copy(&mut File::open(source)?, &mut zip)?;
        }
        zip.finish()?;
    } else {
        let mut tar = tar::Builder::new(flate2::write::GzEncoder::new(
            file,
            flate2::Compression::default(),
        ));
        for (name, source) in entries {
            let mut file = File::open(source)?;
            let mut header = tar::Header::new_gnu();
            header.set_size(file.metadata()?.len());
            header.set_mode(if name == "yolupainter" || name == "yolupainter-cli" {
                0o755
            } else {
                0o644
            });
            header.set_cksum();
            tar.append_data(&mut header, name, &mut file)?;
        }
        tar.into_inner()?.finish()?;
    }
    Ok(())
}
fn signing_key(file: Option<&Path>) -> Result<SigningKey> {
    signing_key_from(file, env::var(PRIVATE_KEY_ENV).ok())
}
/// 鍵ファイルを指定したときはそれだけを読み、環境変数の値は見ない。
fn signing_key_from(file: Option<&Path>, environment: Option<String>) -> Result<SigningKey> {
    let text = match file {
        Some(path) => fs::read_to_string(path)?,
        None => environment.ok_or("署名用秘密鍵がありません")?,
    };
    let bytes: [u8; 32] = hex::decode(text.trim())
        .map_err(|_| "秘密鍵は hex 形式が必要です")?
        .try_into()
        .map_err(|_| "秘密鍵は 32 バイト必要です")?;
    Ok(SigningKey::from_bytes(&bytes))
}
fn public_key_bytes(text: &str) -> Result<[u8; 32]> {
    if text.trim().is_empty() {
        return Err(
            "公開鍵が空です。リポジトリ変数 YOLUPAINTER_UPDATE_PUBLIC_KEY を設定してください"
                .into(),
        );
    }
    let bytes = hex::decode(text.trim()).map_err(|_| "公開鍵は hex 形式が必要です")?;
    bytes
        .try_into()
        .map_err(|_| "公開鍵は 32 バイト必要です".into())
}
/// `load_key` は署名するときだけ呼ぶ。鍵が読めない失敗でも古い更新情報（`UPDATER_FILE`）は残さない。
fn updater(
    version: Version,
    directory: &Path,
    sign: bool,
    load_key: impl FnOnce() -> Result<SigningKey>,
) -> Result<()> {
    let output = directory.join(UPDATER_FILE);
    remove_if_present(&output)?;
    let mut assets = Vec::new();
    for target in TARGETS {
        let name = asset_name(&version, target)?;
        let path = directory.join(&name);
        if fs::symlink_metadata(&path).is_err() {
            continue;
        }
        let size = fs::symlink_metadata(&path)?;
        if !size.is_file() || size.len() == 0 || size.len() > MAX_ASSET {
            return Err("配布物の種類または大きさが不正です".into());
        }
        let mut bytes = Vec::new();
        File::open(path)?
            .take(MAX_ASSET + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 != size.len() {
            return Err("配布物が読み込み中に変更されました".into());
        }
        assets.push(Asset {
            target: target.into(),
            url: asset_url(&version, &name),
            name,
            sha256: sha256(&bytes),
            size: size.len(),
        });
    }
    // 無関係なファイルを黙って除外しない。入力は配布物専用フォルダにする。
    // 例外は付属物だけ（PDB の `symbols_name` と .mcpb の `mcpb_name`）。
    let symbols = symbols_name(&version);
    let extension = mcpb_name(&version);
    for entry in fs::read_dir(directory)? {
        let name = entry?
            .file_name()
            .into_string()
            .map_err(|_| "配布物名が UTF-8 ではありません")?;
        // 例外は付属物の 2 つ（PDB と Claude Desktop の拡張 .mcpb）。どちらも更新の対象ではないので、更新情報には載せない
        if name == symbols || name == extension {
            let meta = fs::symlink_metadata(directory.join(&name))?;
            if !meta.is_file() || meta.len() == 0 || meta.len() > MAX_ASSET {
                return Err(format!("付属物 {name} の種類または大きさが不正です").into());
            }
            continue;
        }
        if name != format!("{UPDATER_FILE}.tmp") && !assets.iter().any(|a| a.name == name) {
            return Err(format!(
                "予期しない配布物: {name}（この版の配布物だけを置いたフォルダが必要です）"
            )
            .into());
        }
    }
    if assets.is_empty() {
        return Err("配布物がありません".into());
    }
    // インストールした Windows のアプリは、更新にインストーラーを使う。zip だけの版を出すと、
    // その版の更新の確認が「対象の配布物がありません」で止まるので、必ず並べて出す。
    let has = |target: &str| assets.iter().any(|a| a.target == target);
    if has(WINDOWS_ARCHIVE) && !has(WINDOWS_INSTALLER) {
        return Err("Windows のインストーラーがありません（zip と並べて出します）".into());
    }
    let payload = serde_json::to_string(&Manifest {
        schema: UPDATER_SCHEMA,
        version: version.to_string(),
        assets,
    })?;
    let signature = if sign {
        Some(hex::encode(load_key()?.sign(payload.as_bytes()).to_bytes()))
    } else {
        None
    };
    let temporary = directory.join(format!("{UPDATER_FILE}.tmp"));
    fs::write(
        &temporary,
        serde_json::to_vec_pretty(&Envelope { payload, signature })?,
    )?;
    fs::rename(temporary, output)?;
    Ok(())
}
/// 更新情報の URL の代わり。実際の取得はしない。
const VERIFY_URL: &str = "https://verify.invalid/updater-verify.json";
/// フォルダの中のファイルを、更新クレートの取得口として返す。
struct DirectoryTransport {
    directory: PathBuf,
    version: Version,
}
impl Transport for DirectoryTransport {
    fn get(&self, url: &str, max_bytes: usize) -> std::result::Result<Vec<u8>, yolu_update::Error> {
        let fail = |message: &str| yolu_update::Error(message.into());
        let prefix = format!("{RELEASE_BASE}/v{}/", self.version);
        let name = if url == VERIFY_URL {
            UPDATER_FILE
        } else {
            url.strip_prefix(&prefix)
                .ok_or_else(|| fail("想定外の URL です"))?
        };
        if name.is_empty() || name.contains(['/', '\\']) {
            return Err(fail("想定外のファイル名です"));
        }
        let path = self.directory.join(name);
        let metadata = fs::symlink_metadata(&path).map_err(|_| fail("ファイルがありません"))?;
        if !metadata.is_file() {
            return Err(fail("通常のファイルではありません"));
        }
        let mut bytes = Vec::new();
        File::open(path)
            .and_then(|file| file.take(max_bytes as u64 + 1).read_to_end(&mut bytes))
            .map_err(|_| fail("ファイルを読めません"))?;
        Ok(bytes)
    }
}
/// アーカイブ（zip・tar.gz）の中のファイルの名前（フォルダの項目は数えない）。zip なのは Windows と macOS、Linux は tar.gz。
fn archive_names(target: &str, bytes: &[u8]) -> Result<Vec<String>> {
    let mut names = Vec::new();
    if target == WINDOWS_ARCHIVE || target == MACOS_ARCHIVE {
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))?;
        for index in 0..zip.len() {
            let file = zip.by_index(index)?;
            if file.is_file() {
                names.push(file.name().to_owned());
            }
        }
    } else {
        let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(bytes));
        for entry in tar.entries()? {
            let entry = entry?;
            if entry.header().entry_type().is_file() {
                names.push(entry.path()?.to_string_lossy().into_owned());
            }
        }
    }
    Ok(names)
}
/// アーカイブの中身が `payload_names`（macOS は最上位のフォルダつきの `macos::archive_names`）と同じか照らす。
/// 欠けも、一覧に無いファイルも、同じ名前の重複も断る（名前を並べて知らせる）。
fn check_archive_contents(target: &str, version: &Version, bytes: &[u8]) -> Result<()> {
    let mut found = archive_names(target, bytes)?;
    found.sort();
    let expected = if target == MACOS_ARCHIVE {
        macos::archive_names(version)
    } else {
        payload_names(target)
    };
    let missing: Vec<_> = expected.iter().filter(|n| !found.contains(n)).collect();
    let mut extra: Vec<_> = found.iter().filter(|n| !expected.contains(n)).collect();
    extra.extend(
        found
            .windows(2)
            .filter(|pair| pair[0] == pair[1])
            .map(|pair| &pair[0]),
    );
    if missing.is_empty() && extra.is_empty() {
        return Ok(());
    }
    let list = |names: Vec<&String>| {
        names
            .iter()
            .map(|n| n.as_str())
            .collect::<Vec<_>>()
            .join("、")
    };
    Err(format!(
        "配布物の中身が一覧と違います（足りない: {}／余計または重複: {}）",
        list(missing),
        list(extra)
    )
    .into())
}
/// PDB の付属物の zip が、通常のファイルで、PDB だけを 1 つ持つこと。
fn check_symbols_archive(path: &Path) -> Result<()> {
    let name = path
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    if !fs::symlink_metadata(path)?.is_file() {
        return Err(format!("{name}: 通常のファイルではありません").into());
    }
    let mut bytes = Vec::new();
    File::open(path)?
        .take(MAX_ASSET + 1)
        .read_to_end(&mut bytes)?;
    let names = archive_names(WINDOWS_ARCHIVE, &bytes).map_err(|e| format!("{name}: {e}"))?;
    if names != [PDB_FILE] {
        return Err(format!("{name}: 中身が {PDB_FILE} だけではありません").into());
    }
    Ok(())
}
/// .mcpb のファイルが、通常のファイルで、大きさの上限内で、中身が `check_mcpb_archive` を通ること。
fn check_mcpb_file(path: &Path, version: &Version) -> Result<()> {
    let name = path
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    if !fs::symlink_metadata(path)?.is_file() {
        return Err(format!("{name}: 通常のファイルではありません").into());
    }
    let mut bytes = Vec::new();
    File::open(path)?
        .take(MAX_ASSET + 1)
        .read_to_end(&mut bytes)?;
    if bytes.is_empty() || bytes.len() as u64 > MAX_ASSET {
        return Err(format!("{name}: 大きさが不正です").into());
    }
    check_mcpb_archive(&bytes, version).map_err(|e| format!("{name}: {e}").into())
}
/// 公開鍵だけで、アプリと同じ検証（署名・版・大きさ・SHA-256）を通すか確かめる。
/// 秘密の鍵を取り違えた署名や、配布物と更新情報の食い違いをここで落とす。
fn verify(version: &Version, directory: &Path, public_key: [u8; 32]) -> Result<()> {
    let client = UpdateClient::with_public_key(
        DirectoryTransport {
            directory: directory.into(),
            version: version.clone(),
        },
        public_key,
    )?;
    // どの版より古い扱いにして、更新情報の版そのものを取り出す。
    let oldest = Version::parse("0.0.0-0")?;
    let mut present = Vec::new();
    let mut absent = Vec::new();
    for target in TARGETS {
        let name = asset_name(version, target)?;
        if fs::symlink_metadata(directory.join(&name)).is_ok() {
            present.push(target);
        } else {
            absent.push((target, name));
        }
    }
    if present.is_empty() {
        return Err("確認できる配布物がありません".into());
    }
    let mut archives = 0;
    for target in &present {
        let update = client
            .check(VERIFY_URL, &oldest, target, true)?
            .filter(|update| update.version() == version)
            .ok_or("更新情報の版が --version と一致しません")?;
        let name = update.asset().name.clone();
        let download = client.download(update.approve_download())?;
        // インストーラーの中は見られない（その段は同じ一覧から作り、試験が NSIS の一覧と突き合わせる）。
        if is_archive_target(target) {
            check_archive_contents(target, version, download.bytes())
                .map_err(|error| format!("{name}: {error}"))?;
            archives += 1;
        }
    }
    // PDB の付属物があれば、PDB 1 つだけの zip であること（更新情報には載らないので、署名では守られない。形だけ確かめる）。
    let symbols = directory.join(symbols_name(version));
    if fs::symlink_metadata(&symbols).is_ok() {
        check_symbols_archive(&symbols)?;
    }
    // .mcpb があれば、manifest が今の版・実行ファイルが入る形であること（PDB と同じく更新情報には載らないので、形だけ確かめる）。
    let extension = directory.join(mcpb_name(version));
    if fs::symlink_metadata(&extension).is_ok() {
        check_mcpb_file(&extension, version)?;
    }
    // 上で署名と本文が通っているので、ここで確かめるのは「ファイルが無いのに載っている」ことだけ。
    for (target, name) in &absent {
        if let Ok(Some(_)) = client.check(VERIFY_URL, &oldest, target, true) {
            return Err(format!("更新情報にある配布物がありません: {name}").into());
        }
    }
    println!(
        "署名と {} 件の配布物を確認しました（アーカイブ {archives} 件は中身も一覧と一致）",
        present.len()
    );
    Ok(())
}
/// 置き場にすでにある更新情報の版（署名は見ない。読めなければ None）。置き換えてよいかの目安にだけ使う。
fn held_version(existing: &Path) -> Option<Version> {
    let mut bytes = Vec::new();
    File::open(existing)
        .ok()?
        .take(MAX_METADATA as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    let envelope: Envelope = serde_json::from_slice(&bytes).ok()?;
    let payload: serde_json::Value = serde_json::from_str(&envelope.payload).ok()?;
    Version::parse(payload.get("version")?.as_str()?).ok()
}
/// 公開した試験版の署名つきの更新情報（`directory` の `UPDATER_FILE`）を、試験版の置き場（固定のタグの Release）へ上書きで置く物として
/// `output` へ写す。写すのは原本のバイト列そのままで、再署名しない（秘密鍵に触れない）。
///
/// - 版は試験版の形（`is_beta_version`）。正式版や形の違う版は置き場に載せない。
/// - 置く前に、公開鍵だけで署名・版・配布物の大きさと SHA-256・梱包の中身を `verify` と同じに確かめる
///   （Draft のあとに Release の資産が差し替えられても、公開した今の物で確かめ直す）。
/// - `existing`（今の置き場の更新情報）の版が今の版以上なら、置き換えず何も書かない（古い試験版を後から公開しても、置き場を戻さない）。
///   `existing` が読めなければ、壊れた置き場を直すつもりで置く。この比べは戻し防止の目安で、署名の確かめではない
///   （アプリが署名を確かめる）。
fn beta_channel(
    version: &Version,
    directory: &Path,
    public_key: [u8; 32],
    existing: Option<&Path>,
    output: &Path,
) -> Result<()> {
    let destination = output.join(UPDATER_FILE);
    remove_if_present(&destination)?;
    if !is_beta_version(version) {
        return Err(format!(
            "試験版の置き場に載せられない版です: {version}（プレリリース識別子は alpha.N・beta.N・rc.N の 1 つ）"
        )
        .into());
    }
    verify(version, directory, public_key)?;
    let source = directory.join(UPDATER_FILE);
    let mut bytes = Vec::new();
    File::open(&source)?
        .take(MAX_METADATA as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_METADATA {
        return Err("更新情報が大きすぎます".into());
    }
    if let Some(held) = existing.and_then(held_version) {
        if held.cmp_precedence(version).is_ge() {
            println!("置き場の版 {held} は今の版 {version} 以上なので、置き換えません");
            return Ok(());
        }
    }
    fs::create_dir_all(output)?;
    let temporary = output.join(format!("{UPDATER_FILE}.tmp"));
    fs::write(&temporary, &bytes)?;
    fs::rename(temporary, destination)?;
    println!("試験版の置き場に {version} を置きます");
    Ok(())
}
fn keygen(output: &Path) -> Result<String> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut seed = [0u8; 32];
    getrandom::fill(&mut seed).map_err(|_| "乱数を取得できません")?;
    let key = SigningKey::from_bytes(&seed);
    let mut file = options.open(output)?;
    writeln!(file, "{}", hex::encode(seed))?;
    file.sync_all()?;
    Ok(hex::encode(key.verifying_key().to_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use yolu_update::LINUX_ARCHIVE;
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    struct Scratch(PathBuf);
    impl Scratch {
        fn new() -> Self {
            let p = root().join("target/xtask-tests").join(format!(
                "{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&p).unwrap();
            Self(p)
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    /// 配布物の名前ごとに、名前そのものを中身にした試験用の元ファイル（`dir/src` の下。`docs/en/…` のフォルダも作る）。
    fn fake_payload(dir: &Path, target: &str) -> Vec<(String, PathBuf)> {
        fake_files(dir, payload_names(target))
    }
    fn fake_files(dir: &Path, names: Vec<String>) -> Vec<(String, PathBuf)> {
        names
            .into_iter()
            .map(|name| {
                let source = dir.join("src").join(&name);
                fs::create_dir_all(source.parent().unwrap()).unwrap();
                fs::write(&source, &name).unwrap();
                (name, source)
            })
            .collect()
    }
    #[test]
    fn zip_preserves_files_and_contents() {
        let d = Scratch::new();
        let entries = fake_payload(&d.0, WINDOWS_ARCHIVE);
        let p = d.0.join("test.zip");
        archive(&p, &entries, true).unwrap();
        let mut zip = zip::ZipArchive::new(File::open(p).unwrap()).unwrap();
        assert_eq!(zip.len(), entries.len());
        for (name, _) in entries {
            let mut s = String::new();
            zip.by_name(&name).unwrap().read_to_string(&mut s).unwrap();
            assert_eq!(s, name);
        }
        // 文書はフォルダつきの名前で入る（README からの相対のリンクがそのまま効く）。
        assert!(zip.by_name("docs/GUIDE.md").is_ok() && zip.by_name("docs/en/GUIDE.md").is_ok());
        assert!(zip.by_name("README.en.md").is_ok());
    }
    #[test]
    fn tar_preserves_executable_mode() {
        let d = Scratch::new();
        let exe = d.0.join("exe");
        fs::write(&exe, b"fixture").unwrap();
        let p = d.0.join("test.tar.gz");
        archive(&p, &[("yolupainter".into(), exe)], false).unwrap();
        let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(File::open(p).unwrap()));
        let mut entries = tar.entries().unwrap();
        let mut entry = entries.next().unwrap().unwrap();
        assert_eq!(entry.header().mode().unwrap(), 0o755);
        assert_eq!(entry.path().unwrap(), Path::new("yolupainter"));
        let mut content = Vec::new();
        entry.read_to_end(&mut content).unwrap();
        assert_eq!(content, b"fixture");
    }
    fn disposable_key() -> SigningKey {
        SigningKey::from_bytes(&[42; 32])
    }
    fn no_key() -> Result<SigningKey> {
        Err("鍵なし".into())
    }
    fn exec(list: &[&str]) -> Result<()> {
        execute(
            list.iter()
                .map(|item| item.to_string())
                .collect::<Vec<_>>()
                .into_iter(),
        )
    }
    fn asset_path(dir: &Path, version: &Version, target: usize) -> PathBuf {
        dir.join(asset_name(version, TARGETS[target]).unwrap())
    }
    fn assert_no_metadata(dir: &Path) {
        assert!(!dir.join(UPDATER_FILE).exists());
        assert!(!dir.join(format!("{UPDATER_FILE}.tmp")).exists());
    }
    /// 一覧どおりの中身の（`drop` を除き、`add` を足した）本物のアーカイブのバイト列。macOS の zip は、最上位のフォルダつきの名前で作る。
    fn fake_archive(version: &Version, target: &str, drop: &[&str], add: &[&str]) -> Vec<u8> {
        let d = Scratch::new();
        let names = if target == MACOS_ARCHIVE {
            macos::archive_names(version)
        } else {
            payload_names(target)
        };
        let mut entries = fake_files(&d.0, names);
        let prefix = if target == MACOS_ARCHIVE {
            format!("{}/", macos::folder_name(version))
        } else {
            String::new()
        };
        entries.retain(|(name, _)| {
            !drop
                .iter()
                .any(|dropped| format!("{prefix}{dropped}") == *name)
        });
        for name in add {
            let source = d.0.join("extra");
            fs::write(&source, name).unwrap();
            // ditto の `__MACOSX/…` は、束のフォルダの外（zip の最上位）に入る
            let entry = if name.starts_with("__MACOSX/") {
                (*name).to_owned()
            } else {
                format!("{prefix}{name}")
            };
            entries.push((entry, source));
        }
        let path = d.0.join("fake-archive");
        archive(&path, &entries, target != LINUX_ARCHIVE).unwrap();
        fs::read(path).unwrap()
    }
    /// 使い捨ての鍵で署名した配布物の置き場。公開鍵の hex も返す。
    fn signed_dist(version: &Version, targets: &[usize]) -> (Scratch, String) {
        signed_dist_with(version, targets, |_, bytes| bytes)
    }
    /// `alter` で、対象ごとのアーカイブのバイト列を（署名の前に）作り替えられる。
    fn signed_dist_with(
        version: &Version,
        targets: &[usize],
        alter: impl Fn(usize, Vec<u8>) -> Vec<u8>,
    ) -> (Scratch, String) {
        let d = Scratch::new();
        for &t in targets {
            fs::write(
                asset_path(&d.0, version, t),
                alter(t, fake_archive(version, TARGETS[t], &[], &[])),
            )
            .unwrap();
            if t == 0 {
                // zip はインストーラーと並べて出す。
                fs::write(asset_path(&d.0, version, 2), "installer").unwrap();
            }
        }
        updater(version.clone(), &d.0, true, || Ok(disposable_key())).unwrap();
        (d, hex::encode(disposable_key().verifying_key().to_bytes()))
    }
    #[test]
    fn generated_manifest_is_accepted_by_update_client() {
        struct Fake {
            metadata: Vec<u8>,
        }
        impl yolu_update::Transport for Fake {
            fn get(&self, url: &str, _: usize) -> std::result::Result<Vec<u8>, yolu_update::Error> {
                Ok(if url.ends_with(UPDATER_FILE) {
                    self.metadata.clone()
                } else {
                    b"archive".to_vec()
                })
            }
        }
        let d = Scratch::new();
        let v = Version::new(1, 2, 3);
        fs::write(d.0.join(asset_name(&v, TARGETS[0]).unwrap()), b"archive").unwrap();
        fs::write(
            d.0.join(asset_name(&v, WINDOWS_INSTALLER).unwrap()),
            b"setup",
        )
        .unwrap();
        let keys = Scratch::new();
        let path = keys.0.join("disposable.hex");
        fs::write(&path, hex::encode([42; 32])).unwrap();
        updater(v, &d.0, true, || signing_key_from(Some(&path), None)).unwrap();
        let key = disposable_key();
        let c = yolu_update::UpdateClient::with_public_key(
            Fake {
                metadata: fs::read(d.0.join(UPDATER_FILE)).unwrap(),
            },
            key.verifying_key().to_bytes(),
        )
        .unwrap();
        let update = c
            .check(
                &format!("https://example.invalid/{UPDATER_FILE}"),
                &Version::new(1, 0, 0),
                TARGETS[0],
                false,
            )
            .unwrap()
            .unwrap();
        assert_eq!(
            c.download(update.approve_download()).unwrap().bytes(),
            b"archive"
        );
    }
    #[test]
    fn stale_metadata_removed_on_invalid_assets() {
        let d = Scratch::new();
        fs::write(d.0.join(UPDATER_FILE), "stale").unwrap();
        fs::write(d.0.join("unexpected.zip"), "x").unwrap();
        assert!(updater(Version::new(1, 0, 0), &d.0, false, no_key).is_err());
        assert!(!d.0.join(UPDATER_FILE).exists());
    }
    #[test]
    fn empty_assets_rejected() {
        let d = Scratch::new();
        assert!(updater(Version::new(1, 0, 0), &d.0, false, no_key).is_err());
    }
    #[test]
    fn asset_of_wrong_type_or_size_rejected() {
        let v = Version::new(1, 0, 0);
        type Make = fn(&Path, &Path);
        let kinds: [(&str, Make); 4] = [
            ("directory", |_, asset| fs::create_dir(asset).unwrap()),
            ("empty", |_, asset| fs::write(asset, b"").unwrap()),
            ("oversized", |_, asset| {
                // 疎なファイルなので実際のディスクは使わない。
                File::create(asset).unwrap().set_len(MAX_ASSET + 1).unwrap();
            }),
            ("symlink", |outside, asset| {
                #[cfg(unix)]
                {
                    fs::write(outside.join("real"), b"archive").unwrap();
                    std::os::unix::fs::symlink(outside.join("real"), asset).unwrap();
                }
                #[cfg(not(unix))]
                {
                    let _ = outside;
                    fs::create_dir(asset).unwrap();
                }
            }),
        ];
        for (kind, make) in kinds {
            let d = Scratch::new();
            let outside = Scratch::new();
            fs::write(d.0.join(UPDATER_FILE), "stale").unwrap();
            make(&outside.0, &asset_path(&d.0, &v, 0));
            assert!(updater(v.clone(), &d.0, false, no_key).is_err(), "{kind}");
            assert_no_metadata(&d.0);
        }
    }
    #[test]
    fn signing_failure_leaves_no_metadata() {
        let v = Version::new(1, 0, 0);
        let d = Scratch::new();
        fs::write(asset_path(&d.0, &v, 0), b"archive").unwrap();
        fs::write(d.0.join(UPDATER_FILE), "stale").unwrap();
        assert!(updater(v, &d.0, true, no_key).is_err());
        assert_no_metadata(&d.0);
    }
    #[test]
    fn invalid_private_keys_rejected() {
        let d = Scratch::new();
        let file = |text: &str| {
            let path = d.0.join("key.hex");
            fs::write(&path, text).unwrap();
            path
        };
        for bad in ["zz", "", &"ab".repeat(31), &"ab".repeat(33)] {
            assert!(signing_key_from(Some(&file(bad)), None).is_err(), "{bad:?}");
            assert!(signing_key_from(None, Some(bad.into())).is_err(), "{bad:?}");
        }
        assert!(signing_key_from(None, None).is_err());
        assert!(signing_key_from(Some(&d.0.join("missing.hex")), None).is_err());
        // 改行つきでも読め、ファイルを指定したときは環境変数の値を見ない。
        let good = format!("{}\n", hex::encode([42; 32]));
        assert!(signing_key_from(Some(&file(&good)), Some("zz".into())).is_ok());
        assert!(signing_key_from(None, Some(good)).is_ok());
    }
    #[test]
    fn command_line_rejections() {
        let d = Scratch::new();
        let v = Version::new(1, 0, 0);
        fs::write(asset_path(&d.0, &v, 0), b"archive").unwrap();
        let dir = d.0.to_str().unwrap();
        for list in [
            vec![
                "updater-json",
                "--version",
                "1.0.0",
                "--assets",
                dir,
                "--key-file",
                "k",
            ],
            vec![
                "updater-json",
                "--version",
                "1.0.0",
                "--assets",
                dir,
                "--unknown",
            ],
            vec!["updater-json", "--assets", dir],
            vec!["updater-json", "--version", "1.0.0"],
            vec![
                "updater-json",
                "--version",
                "1.0.0",
                "--assets",
                dir,
                "--sign",
                "--target",
                "t",
            ],
            vec!["build", "--target", TARGETS[0]],
            vec!["build", "--target", "aarch64-apple-darwin", "--release"],
            vec!["bundle"],
            vec!["bundle", "--target", "aarch64-apple-darwin"],
            // PDB の付属物は Windows だけ（組まずに断る）
            vec!["symbols"],
            vec!["symbols", "--target", LINUX_ARCHIVE],
            vec!["symbols", "--target", "aarch64-apple-darwin"],
            vec!["verify", "--version", "1.0.0", "--assets", dir],
            vec!["verify", "--version", "1.0.0", "--public-key", "00"],
            vec!["pubkey"],
            vec!["keygen"],
            // 試験版の置き場: 必要な引数が揃わない・ほかの命令に --existing
            vec![
                "beta-channel",
                "--version",
                "1.0.0-rc.1",
                "--assets",
                dir,
                "--public-key",
                "00",
            ],
            vec![
                "beta-channel",
                "--version",
                "1.0.0-rc.1",
                "--assets",
                dir,
                "--output",
                dir,
            ],
            vec![
                "beta-channel",
                "--assets",
                dir,
                "--public-key",
                "00",
                "--output",
                dir,
            ],
            vec![
                "updater-json",
                "--version",
                "1.0.0",
                "--assets",
                dir,
                "--existing",
                "x",
            ],
            vec!["unknown"],
            vec![],
        ] {
            assert!(exec(&list).is_err(), "{list:?}");
            assert_no_metadata(&d.0);
        }
    }
    /// 付属物の PDB の zip の組み立て: PDB だけを `yolupainter.pdb` の名前で持ち、無い・空・大きすぎる PDB は、何も置かずに断る。
    #[test]
    fn the_symbols_archive_holds_only_the_pdb_and_refuses_a_missing_or_empty_one() {
        let d = Scratch::new();
        let v = Version::new(0, 3, 1);
        assert_eq!(
            symbols_name(&v),
            "yolupainter-0.3.1-x86_64-pc-windows-msvc-pdb.zip"
        );
        // 更新の対象の名前とは別（更新情報の鍵にも、更新の対象の配布物の名前にもならない）
        assert!(TARGETS
            .iter()
            .all(|t| asset_name(&v, t).unwrap() != symbols_name(&v)));
        let pdb = d.0.join("anything-built.pdb");
        fs::write(&pdb, b"pdb-bytes").unwrap();
        let destination = d.0.join(symbols_name(&v));
        symbols_archive(&pdb, &destination).unwrap();
        check_symbols_archive(&destination).unwrap();
        let bytes = fs::read(&destination).unwrap();
        assert_eq!(archive_names(WINDOWS_ARCHIVE, &bytes).unwrap(), [PDB_FILE]);
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let mut content = Vec::new();
        zip.by_name(PDB_FILE)
            .unwrap()
            .read_to_end(&mut content)
            .unwrap();
        assert_eq!(content, b"pdb-bytes");
        // 無い・空の PDB は断り、前回の付属物も一時ファイルも残さない（前のビルドの PDB を今回の物と取り違えない）
        fs::write(d.0.join("empty.pdb"), b"").unwrap();
        for bad in ["missing.pdb", "empty.pdb"] {
            assert!(
                destination.exists(),
                "{bad}: 前回の付属物がある状態から始める"
            );
            let error = symbols_archive(&d.0.join(bad), &destination)
                .unwrap_err()
                .to_string();
            assert!(error.contains("PDB"), "{bad}: {error}");
            assert!(
                !destination.exists() && !destination.with_extension("tmp").exists(),
                "{bad}: 前回の付属物が残った"
            );
            // 次の成功は、また前の物を置き換える
            symbols_archive(&pdb, &destination).unwrap();
            check_symbols_archive(&destination).unwrap();
        }
        // 前の付属物がある所へ成功の組み直しをしても、置き換わって 1 つだけ
        symbols_archive(&pdb, &destination).unwrap();
        check_symbols_archive(&destination).unwrap();
        assert!(!destination.with_extension("tmp").exists());
    }
    /// 付属物の PDB は更新の対象ではない: 更新情報に載せず、ほかの見知らぬファイルは今までどおり断る。形のおかしい付属物は断る。
    #[test]
    fn updater_leaves_the_symbols_archive_out_of_the_manifest_and_still_refuses_strangers() {
        let v = Version::new(1, 2, 3);
        let d = Scratch::new();
        fs::write(asset_path(&d.0, &v, 0), b"archive").unwrap();
        fs::write(asset_path(&d.0, &v, 2), b"setup").unwrap();
        let symbols = d.0.join(symbols_name(&v));
        fs::write(&symbols, b"pdb zip").unwrap();
        updater(v.clone(), &d.0, true, || Ok(disposable_key())).unwrap();
        let envelope: Envelope =
            serde_json::from_slice(&fs::read(d.0.join(UPDATER_FILE)).unwrap()).unwrap();
        let manifest: Manifest = serde_json::from_str(&envelope.payload).unwrap();
        let mut targets: Vec<_> = manifest.assets.iter().map(|a| a.target.as_str()).collect();
        targets.sort();
        assert_eq!(targets, [WINDOWS_ARCHIVE, WINDOWS_INSTALLER]);
        assert!(manifest.assets.iter().all(|a| !a.name.contains("pdb")));
        // 別の版の PDB・名前の違うファイルは、今までどおり「予期しない配布物」
        for stranger in [
            "yolupainter-9.9.9-x86_64-pc-windows-msvc-pdb.zip",
            "yolupainter.pdb",
            "notes.txt",
        ] {
            fs::write(d.0.join(stranger), b"x").unwrap();
            assert!(
                updater(v.clone(), &d.0, true, || Ok(disposable_key())).is_err(),
                "{stranger}"
            );
            assert_no_metadata(&d.0);
            fs::remove_file(d.0.join(stranger)).unwrap();
        }
        // 付属物が空・フォルダ・大きすぎるときは断る
        fs::write(&symbols, b"").unwrap();
        assert!(updater(v.clone(), &d.0, true, || Ok(disposable_key())).is_err());
        fs::remove_file(&symbols).unwrap();
        fs::create_dir(&symbols).unwrap();
        assert!(updater(v.clone(), &d.0, true, || Ok(disposable_key())).is_err());
        fs::remove_dir(&symbols).unwrap();
        File::create(&symbols)
            .unwrap()
            .set_len(MAX_ASSET + 1)
            .unwrap();
        assert!(updater(v, &d.0, true, || Ok(disposable_key())).is_err());
        assert_no_metadata(&d.0);
    }
    /// `verify` は、付属物があれば PDB 1 つだけの zip であることを見る（無くても通る。更新の確かめは今までどおり）。
    #[test]
    fn verify_checks_the_shape_of_the_symbols_archive_when_there_is_one() {
        let v = Version::new(1, 2, 3);
        let verify_with = |dir: &Path, public: &str| {
            exec(&[
                "verify",
                "--version",
                "1.2.3",
                "--assets",
                dir.to_str().unwrap(),
                "--public-key",
                public,
            ])
        };
        // 付属物があっても、署名つきの更新情報を作り直さずに通る
        let (d, public) = signed_dist(&v, &[0, 1]);
        verify_with(&d.0, &public).unwrap();
        let pdb = d.0.join("pdb-source");
        fs::write(&pdb, b"pdb").unwrap();
        symbols_archive(&pdb, &d.0.join(symbols_name(&v))).unwrap();
        fs::remove_file(&pdb).unwrap();
        verify_with(&d.0, &public).unwrap();
        // zip でない・別のファイルが入っている・PDB の名前でない付属物は断る
        fs::write(d.0.join(symbols_name(&v)), b"not a zip").unwrap();
        assert!(verify_with(&d.0, &public).is_err());
        let wrong = fake_archive(&Version::new(1, 0, 0), WINDOWS_ARCHIVE, &[], &[]);
        fs::write(d.0.join(symbols_name(&v)), wrong).unwrap();
        assert!(verify_with(&d.0, &public).is_err());
        let mut zip = zip::ZipWriter::new(File::create(d.0.join(symbols_name(&v))).unwrap());
        zip.start_file("other.pdb", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"pdb").unwrap();
        zip.finish().unwrap();
        assert!(verify_with(&d.0, &public).is_err());
    }
    /// .mcpb の試験用の組み立て: 本物の manifest と、名前を中身にしたファイルを、`drop` を除き `add` を足して zip にする。
    fn fake_mcpb(
        version: &Version,
        drop: &[&str],
        add: &[&str],
        manifest: Option<serde_json::Value>,
    ) -> Vec<u8> {
        let d = Scratch::new();
        let mut entries = Vec::new();
        for name in MCPB_FILES {
            if drop.contains(name) {
                continue;
            }
            let source = d.0.join(name.replace('/', "_"));
            if *name == "manifest.json" {
                let manifest = manifest.clone().unwrap_or_else(|| mcpb_manifest(version));
                fs::write(&source, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
            } else {
                fs::write(&source, name).unwrap();
            }
            entries.push(((*name).to_owned(), source));
        }
        for name in add {
            let source = d.0.join("extra");
            fs::write(&source, name).unwrap();
            entries.push(((*name).to_owned(), source));
        }
        let path = d.0.join("fake.mcpb");
        archive(&path, &entries, true).unwrap();
        fs::read(path).unwrap()
    }
    /// Claude Desktop の拡張の manifest: binary のサーバーで、実行ファイルは拡張の中の 1 つ、引数は mcp と設定の番号（既定はアプリの既定）、Windows だけ。
    #[test]
    fn the_mcpb_manifest_names_the_binary_server_and_only_windows() {
        let v = Version::parse("0.4.0-rc.1").unwrap();
        let manifest = mcpb_manifest(&v);
        check_mcpb_manifest(&manifest, &v).unwrap();
        assert_eq!(manifest["manifest_version"], "0.3");
        assert_eq!(manifest["version"], "0.4.0-rc.1");
        assert_eq!(manifest["server"]["type"], "binary");
        assert_eq!(
            manifest["server"]["entry_point"],
            "server/yolupainter-cli.exe"
        );
        assert_eq!(
            manifest["server"]["mcp_config"]["command"],
            "${__dirname}/server/yolupainter-cli.exe"
        );
        assert_eq!(
            manifest["server"]["mcp_config"]["args"],
            serde_json::json!(["mcp", "--port", "${user_config.port}"])
        );
        assert_eq!(manifest["user_config"]["port"]["default"], 17347);
        assert_eq!(
            manifest["compatibility"]["platforms"],
            serde_json::json!(["win32"])
        );
        assert_eq!(
            MCPB_SERVER,
            format!("server/{}", cli_exe_name(WINDOWS_ARCHIVE))
        );
        assert_eq!(
            mcpb_name(&v),
            "yolupainter-0.4.0-rc.1-x86_64-pc-windows-msvc.mcpb"
        );
        // 更新の対象の名前とは別（更新情報の鍵にも、更新の対象の配布物の名前にもならない）
        assert!(TARGETS
            .iter()
            .all(|t| asset_name(&v, t).unwrap() != mcpb_name(&v)));
        // 版が違う・形が違う manifest は断る
        assert!(check_mcpb_manifest(&manifest, &Version::new(0, 4, 1)).is_err());
        for (pointer, value) in [
            ("/server/type", serde_json::json!("node")),
            ("/server/entry_point", serde_json::json!("server/other.exe")),
            (
                "/server/mcp_config/command",
                serde_json::json!("yolupainter-cli.exe"),
            ),
            ("/server/mcp_config/args", serde_json::json!(["serve"])),
            ("/server/mcp_config/args", serde_json::json!(["mcp"])),
            ("/user_config/port/type", serde_json::json!("string")),
            (
                "/compatibility/platforms",
                serde_json::json!(["win32", "linux"]),
            ),
            ("/manifest_version", serde_json::json!("0.2")),
            ("/name", serde_json::json!("other")),
            ("/icon", serde_json::json!("logo.png")),
            ("/description", serde_json::json!("")),
        ] {
            let mut broken = manifest.clone();
            *broken.pointer_mut(pointer).unwrap() = value;
            assert!(check_mcpb_manifest(&broken, &v).is_err(), "{pointer}");
        }
    }
    /// .mcpb の中身: 一覧のファイルだけ（欠けも余りも断る）で、manifest が今の版。
    #[test]
    fn the_mcpb_archive_holds_exactly_the_listed_files_and_a_matching_manifest() {
        let v = Version::new(1, 2, 3);
        check_mcpb_archive(&fake_mcpb(&v, &[], &[], None), &v).unwrap();
        // 一覧は、manifest・アイコン・許諾・実行ファイル（フォルダつきの名前）
        assert!(MCPB_FILES.contains(&"manifest.json") && MCPB_FILES.contains(&MCPB_SERVER));
        for missing in MCPB_FILES {
            let error = check_mcpb_archive(&fake_mcpb(&v, &[missing], &[], None), &v).err();
            assert!(error.is_some(), "{missing} が無くても通った");
        }
        assert!(check_mcpb_archive(&fake_mcpb(&v, &[], &["notes.txt"], None), &v).is_err());
        assert!(
            check_mcpb_archive(&fake_mcpb(&v, &[], &[], None), &Version::new(9, 9, 9)).is_err()
        );
        let mut wrong = mcpb_manifest(&v);
        wrong["server"]["entry_point"] = serde_json::json!("server/other.exe");
        assert!(check_mcpb_archive(&fake_mcpb(&v, &[], &[], Some(wrong)), &v).is_err());
        assert!(check_mcpb_archive(b"not a zip", &v).is_err());
    }
    /// .mcpb は更新の対象ではない: 更新情報に載せず、別の版の .mcpb やほかの見知らぬファイルは今までどおり断る。形のおかしい物は断る。
    #[test]
    fn updater_leaves_the_mcpb_out_of_the_manifest_and_still_refuses_strangers() {
        let v = Version::new(1, 2, 3);
        let d = Scratch::new();
        fs::write(asset_path(&d.0, &v, 0), b"archive").unwrap();
        fs::write(asset_path(&d.0, &v, 2), b"setup").unwrap();
        let extension = d.0.join(mcpb_name(&v));
        fs::write(&extension, b"mcpb zip").unwrap();
        updater(v.clone(), &d.0, true, || Ok(disposable_key())).unwrap();
        let envelope: Envelope =
            serde_json::from_slice(&fs::read(d.0.join(UPDATER_FILE)).unwrap()).unwrap();
        let manifest: Manifest = serde_json::from_str(&envelope.payload).unwrap();
        assert!(manifest.assets.iter().all(|a| !a.name.contains("mcpb")));
        assert_eq!(manifest.assets.len(), 2);
        for stranger in [
            "yolupainter-9.9.9-x86_64-pc-windows-msvc.mcpb",
            "yolupainter.mcpb",
            "yolupainter-1.2.3-x86_64-unknown-linux-gnu.mcpb",
        ] {
            fs::write(d.0.join(stranger), b"x").unwrap();
            assert!(
                updater(v.clone(), &d.0, true, || Ok(disposable_key())).is_err(),
                "{stranger}"
            );
            assert_no_metadata(&d.0);
            fs::remove_file(d.0.join(stranger)).unwrap();
        }
        fs::write(&extension, b"").unwrap();
        assert!(updater(v.clone(), &d.0, true, || Ok(disposable_key())).is_err());
        fs::remove_file(&extension).unwrap();
        fs::create_dir(&extension).unwrap();
        assert!(updater(v.clone(), &d.0, true, || Ok(disposable_key())).is_err());
        assert_no_metadata(&d.0);
    }
    /// `verify` は、.mcpb があれば中身の形（一覧のファイル・今の版の manifest）を見る（無くても通る）。
    #[test]
    fn verify_checks_the_shape_of_the_mcpb_when_there_is_one() {
        let v = Version::new(1, 2, 3);
        let verify_with = |dir: &Path, public: &str| {
            exec(&[
                "verify",
                "--version",
                "1.2.3",
                "--assets",
                dir.to_str().unwrap(),
                "--public-key",
                public,
            ])
        };
        let (d, public) = signed_dist(&v, &[0, 1]);
        verify_with(&d.0, &public).unwrap();
        let extension = d.0.join(mcpb_name(&v));
        fs::write(&extension, fake_mcpb(&v, &[], &[], None)).unwrap();
        verify_with(&d.0, &public).unwrap();
        fs::write(&extension, fake_mcpb(&v, &[MCPB_SERVER], &[], None)).unwrap();
        assert!(
            verify_with(&d.0, &public).is_err(),
            "実行ファイルが入っていない .mcpb"
        );
        fs::write(
            &extension,
            fake_mcpb(&Version::new(9, 9, 9), &[], &[], None),
        )
        .unwrap();
        assert!(verify_with(&d.0, &public).is_err(), "版の違う manifest");
        fs::write(&extension, b"not a zip").unwrap();
        assert!(verify_with(&d.0, &public).is_err());
    }
    /// 実行ファイルの一覧: アプリとコマンドラインの 2 つ。どちらも tar では実行できる印、zip の名前は .exe。
    #[test]
    fn both_executables_are_listed_and_the_cli_keeps_its_executable_mode_in_a_tar() {
        assert_eq!(
            executables(WINDOWS_ARCHIVE),
            ["yolupainter.exe", "yolupainter-cli.exe"]
        );
        assert_eq!(
            executables(LINUX_ARCHIVE),
            ["yolupainter", "yolupainter-cli"]
        );
        let d = Scratch::new();
        let exe = d.0.join("exe");
        fs::write(&exe, b"fixture").unwrap();
        let p = d.0.join("test.tar.gz");
        archive(
            &p,
            &[
                ("yolupainter-cli".into(), exe.clone()),
                ("docs/CLI.md".into(), exe),
            ],
            false,
        )
        .unwrap();
        let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(File::open(p).unwrap()));
        let modes: Vec<(String, u32)> = tar
            .entries()
            .unwrap()
            .map(|e| {
                let e = e.unwrap();
                (
                    e.path().unwrap().to_string_lossy().into_owned(),
                    e.header().mode().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            modes,
            [
                ("yolupainter-cli".to_owned(), 0o755),
                ("docs/CLI.md".to_owned(), 0o644)
            ]
        );
    }
    /// ビルドはアプリとコマンドラインを同じ命令で組み、許諾の照合はコマンドラインの依存を含める。
    #[test]
    fn the_build_and_the_license_check_cover_the_cli_too() {
        let build = fs::read_to_string(root().join("crates/xtask/src/main.rs")).unwrap();
        assert!(
            build.contains("\"-p\",\n        \"yolu-cli\","),
            "build が yolu-cli を組む"
        );
        let command = third_party(&root(), WINDOWS_ARCHIVE);
        let args: Vec<String> = command
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(args.contains(&"--include-cli".to_owned()), "{args:?}");
        let cli = third_party_cli(&root(), WINDOWS_ARCHIVE);
        let args: Vec<String> = cli
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(
            args.windows(2).any(|w| w == ["--package", "yolu-cli"])
                && args.contains(&"--bundle".to_owned()),
            "{args:?}"
        );
        // .mcpb の exe は `-p yolu-app -p yolu-cli` の組みの物なので、その組みの木で数える
        assert!(
            args.windows(2).any(|w| w == ["--built-with", "yolu-app"]),
            "{args:?}"
        );
    }
    /// `mcpb` は Windows だけ。
    #[test]
    fn mcpb_is_for_windows_only() {
        let error = exec(&["mcpb", "--target", LINUX_ARCHIVE])
            .unwrap_err()
            .to_string();
        assert!(error.contains("Windows"), "{error}");
        assert!(exec(&["mcpb"]).is_err());
        assert!(exec(&["mcpb", "--target", "aarch64-apple-darwin"]).is_err());
    }
    #[test]
    fn archive_replaces_atomically_and_cleans_up_on_failure() {
        let d = Scratch::new();
        let destination = d.0.join("dist.zip");
        let temporary = d.0.join("dist.tmp");
        let source = d.0.join("LICENSE");
        fs::write(&source, "license").unwrap();
        let entries = vec![("LICENSE".to_owned(), source)];
        // 前回の失敗で残った一時ファイルがあっても、成功すれば置き換わって残らない。
        fs::write(&temporary, "partial").unwrap();
        write_archive(&destination, &entries, true).unwrap();
        assert!(destination.is_file() && !temporary.exists());
        // 失敗したら、旧い配布物も一時ファイルも残さない。
        let broken = vec![
            entries[0].clone(),
            ("missing".to_owned(), d.0.join("missing")),
        ];
        assert!(write_archive(&destination, &broken, true).is_err());
        assert!(!destination.exists() && !temporary.exists());
        let tar = d.0.join("dist.tar.gz");
        fs::write(&tar, "old").unwrap();
        assert!(write_archive(&tar, &broken, false).is_err());
        assert!(!tar.exists() && !d.0.join("dist.tar.tmp").exists());
    }
    #[test]
    fn license_tool_runs_with_utf8_output() {
        let command = python(&root());
        let envs: Vec<_> = command
            .get_envs()
            .map(|(k, v)| (k.to_str().unwrap(), v.and_then(|v| v.to_str())))
            .collect();
        assert!(envs.contains(&("PYTHONUTF8", Some("1"))));
        assert!(envs.contains(&("PYTHONIOENCODING", Some("utf-8"))));
    }
    #[test]
    fn windows_zip_needs_its_installer_and_both_are_listed() {
        let v = Version::new(1, 0, 0);
        let d = Scratch::new();
        fs::write(asset_path(&d.0, &v, 0), b"zip").unwrap();
        fs::write(d.0.join(UPDATER_FILE), "stale").unwrap();
        // zip だけの版は、更新情報を作らず、前の更新情報も残さない。
        let error = updater(v.clone(), &d.0, false, no_key).unwrap_err();
        assert!(error.to_string().contains("インストーラー"), "{error}");
        assert_no_metadata(&d.0);
        // 並べれば、どちらも載る。
        fs::write(asset_path(&d.0, &v, 2), b"setup").unwrap();
        updater(v.clone(), &d.0, false, no_key).unwrap();
        let envelope: Envelope =
            serde_json::from_slice(&fs::read(d.0.join(UPDATER_FILE)).unwrap()).unwrap();
        let manifest: Manifest = serde_json::from_str(&envelope.payload).unwrap();
        let targets: Vec<_> = manifest.assets.iter().map(|a| a.target.as_str()).collect();
        assert_eq!(targets, [WINDOWS_ARCHIVE, WINDOWS_INSTALLER]);
        assert_eq!(manifest.schema, UPDATER_SCHEMA);
        // Linux だけの配布は、インストーラーを要らない。
        let d = Scratch::new();
        fs::write(asset_path(&d.0, &v, 1), b"tar").unwrap();
        updater(v, &d.0, false, no_key).unwrap();
    }
    #[test]
    fn installer_alone_is_listed_but_stray_files_are_still_refused() {
        let v = Version::new(1, 0, 0);
        let d = Scratch::new();
        fs::write(asset_path(&d.0, &v, 2), b"setup").unwrap();
        fs::write(d.0.join("yolupainter-1.0.0-setup.exe"), b"x").unwrap();
        assert!(updater(v, &d.0, false, no_key).is_err());
    }
    #[test]
    fn installer_and_build_accept_only_archive_targets() {
        for list in [
            vec!["build", "--target", WINDOWS_INSTALLER, "--release"],
            vec!["bundle", "--target", WINDOWS_INSTALLER],
            vec!["installer", "--target", WINDOWS_INSTALLER],
            vec!["installer", "--target", "aarch64-apple-darwin"],
            vec!["installer"],
            vec!["installer", "--target", TARGETS[1]],
            vec!["updater-json", "--target", WINDOWS_ARCHIVE],
            vec!["bundle", "--target", TARGETS[0], "--require-update-key"],
        ] {
            assert!(exec(&list).is_err(), "{list:?}");
        }
    }
    #[test]
    fn update_public_key_is_normalised_and_validated() {
        let good = hex::encode(disposable_key().verifying_key().to_bytes());
        // 未設定・空・空白だけは「無し」。GitHub の未設定の変数は空文字で渡る。
        for unset in [None, Some(String::new()), Some("  \n".into())] {
            assert_eq!(update_public_key(unset.clone(), false).unwrap(), None);
            assert!(update_public_key(unset, true).is_err());
        }
        assert_eq!(
            update_public_key(Some(format!(" {good}\n")), true).unwrap(),
            Some(good)
        );
        // 入っているのに使えない鍵は、更新なしのビルドにせず断る（必須でなくても）。
        let weak = format!("01{}", "00".repeat(31));
        for bad in ["zz", &"ab".repeat(31), &"ab".repeat(33), weak.as_str()] {
            assert!(update_public_key(Some(bad.into()), false).is_err(), "{bad}");
        }
    }
    #[test]
    fn nsis_receives_numeric_and_full_versions_and_every_path() {
        let v = Version::parse("1.2.3-rc.1+build5").unwrap();
        assert_eq!(numeric_version(&v).unwrap(), "1.2.3.0");
        assert!(numeric_version(&Version::new(0, 65536, 0)).is_err());
        assert_eq!(
            numeric_version(&Version::new(0, 65535, 1)).unwrap(),
            "0.65535.1.0"
        );
        let icon = root().join(LOGO_ICON);
        assert!(icon.is_file(), "ロゴの .ico が無い");
        let args: Vec<String> = nsis_args(&v, Path::new("/s"), Path::new("/o.exe"), &icon)
            .unwrap()
            .into_iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        for expected in [
            "-DVERSION=1.2.3-rc.1+build5",
            "-DVERSION_NUMERIC=1.2.3.0",
            "-DSTAGE=/s",
            "-DOUTFILE=/o.exe",
            &format!("-DICON={}", icon.display()),
            "-WX",
        ] {
            assert!(args.iter().any(|a| a == expected), "{expected}: {args:?}");
        }
        // 日本語の文字列を BOM の有無によらず読む。
        assert_eq!(&args[..2], ["-INPUTCHARSET", "UTF8"]);
    }
    #[test]
    fn nsis_script_compiles_with_the_arguments_xtask_passes() {
        // makensis が無い環境（開発機・Windows の CI）では確かめない。CI の Linux は入れて走らせる。
        if makensis().arg("-VERSION").output().is_err() {
            eprintln!("makensis が無いので、インストーラーのスクリプトの試験を飛ばす");
            return;
        }
        let d = Scratch::new();
        let stage = d.0.join("stage");
        // xtask の一覧のとおりの段（スクリプトが一覧に無いファイルを入れようとすると、makensis が断る）。
        stage_payload(&fake_payload(&d.0, WINDOWS_ARCHIVE), &stage).unwrap();
        fs::copy(root().join("LICENSE"), stage.join("LICENSE")).unwrap();
        let output = d.0.join("yolupainter-test.exe.tmp");
        let version = Version::parse("1.2.3-rc.1").unwrap();
        let args = nsis_args(&version, &stage, &output, &root().join(LOGO_ICON)).unwrap();
        let status = makensis()
            .current_dir(root())
            .args(args)
            .arg(root().join("installer/yolupainter.nsi"))
            .status()
            .unwrap();
        assert!(status.success());
        let bytes = fs::read(&output).unwrap();
        assert_eq!(&bytes[..2], b"MZ");
        // 製品名と版のバージョン情報（UTF-16）が入っている。
        let find = |needle: &[u8]| bytes.windows(needle.len()).any(|w| w == needle);
        let product: Vec<u8> = "YoluPainter"
            .encode_utf16()
            .flat_map(|u| u.to_le_bytes())
            .collect();
        let version_text: Vec<u8> = "1.2.3-rc.1"
            .encode_utf16()
            .flat_map(|u| u.to_le_bytes())
            .collect();
        assert!(find(&product) && find(&version_text));
    }
    #[test]
    fn keygen_does_not_overwrite_and_limits_permissions() {
        let d = Scratch::new();
        let path = d.0.join("disposable.hex");
        let public = keygen(&path).unwrap();
        let original = fs::read(&path).unwrap();
        assert_eq!(original.len(), 65);
        // pubkey は同じ秘密鍵から同じ公開鍵を導く。
        let derived = signing_key_from(Some(&path), None).unwrap();
        assert_eq!(hex::encode(derived.verifying_key().to_bytes()), public);
        assert!(exec(&["pubkey", "--key-file", path.to_str().unwrap()]).is_ok());
        assert!(keygen(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
    #[test]
    fn public_key_argument_rejections() {
        for bad in ["", "  ", "zz", &"ab".repeat(31), &"ab".repeat(33)] {
            assert!(public_key_bytes(bad).is_err(), "{bad:?}");
        }
        assert!(public_key_bytes(&format!("{}\n", "ab".repeat(32))).is_ok());
    }
    #[test]
    fn verify_accepts_signed_assets_with_the_public_key() {
        let v = Version::new(1, 2, 3);
        let (d, public) = signed_dist(&v, &[0, 1]);
        let dir = d.0.to_str().unwrap();
        exec(&[
            "verify",
            "--version",
            "1.2.3",
            "--assets",
            dir,
            "--public-key",
            &public,
        ])
        .unwrap();
        // 片方の対象だけの配布でも通る。
        let (d, public) = signed_dist(&v, &[1]);
        let dir = d.0.to_str().unwrap();
        exec(&[
            "verify",
            "--version",
            "1.2.3",
            "--assets",
            dir,
            "--public-key",
            &public,
        ])
        .unwrap();
    }
    #[test]
    fn verify_rejects_wrong_key_version_tampering_and_missing_files() {
        let v = Version::new(1, 2, 3);
        let (d, public) = signed_dist(&v, &[0, 1]);
        let dir = d.0.to_str().unwrap();
        let verify_with = |version: &str, key: &str| {
            exec(&[
                "verify",
                "--version",
                version,
                "--assets",
                dir,
                "--public-key",
                key,
            ])
        };
        verify_with("1.2.3", &public).unwrap();
        // secret に別の鍵を入れた場合
        let other = hex::encode(SigningKey::from_bytes(&[43; 32]).verifying_key().to_bytes());
        assert!(verify_with("1.2.3", &other).is_err());
        assert!(verify_with("1.2.4", &public).is_err());
        // 同じ大きさで中身だけ変わった配布物
        let tampered = asset_path(&d.0, &v, 1);
        fs::write(&tampered, "archive-X").unwrap();
        assert!(verify_with("1.2.3", &public).is_err());
        // 更新情報に載っているのに無い配布物
        fs::remove_file(&tampered).unwrap();
        assert!(verify_with("1.2.3", &public).is_err());
        // 署名のない更新情報と、更新情報そのものの欠落
        let (d, public) = signed_dist(&v, &[0]);
        let dir = d.0.to_str().unwrap();
        updater(v.clone(), &d.0, false, no_key).unwrap();
        assert!(exec(&[
            "verify",
            "--version",
            "1.2.3",
            "--assets",
            dir,
            "--public-key",
            &public
        ])
        .is_err());
        fs::remove_file(d.0.join(UPDATER_FILE)).unwrap();
        assert!(exec(&[
            "verify",
            "--version",
            "1.2.3",
            "--assets",
            dir,
            "--public-key",
            &public
        ])
        .is_err());
        // 弱い公開鍵
        let weak = format!("01{}", "00".repeat(31));
        assert!(exec(&[
            "verify",
            "--version",
            "1.2.3",
            "--assets",
            dir,
            "--public-key",
            &weak
        ])
        .is_err());
    }
    #[test]
    fn verify_follows_no_symlinks_and_needs_assets() {
        let v = Version::new(1, 2, 3);
        let (d, public) = signed_dist(&v, &[0]);
        let dir = d.0.to_str().unwrap();
        let archive = asset_path(&d.0, &v, 0);
        #[cfg(unix)]
        {
            let outside = Scratch::new();
            fs::write(outside.0.join("real"), "archive-0").unwrap();
            fs::remove_file(&archive).unwrap();
            std::os::unix::fs::symlink(outside.0.join("real"), &archive).unwrap();
            assert!(exec(&[
                "verify",
                "--version",
                "1.2.3",
                "--assets",
                dir,
                "--public-key",
                &public
            ])
            .is_err());
        }
        fs::remove_file(&archive).ok();
        assert!(exec(&[
            "verify",
            "--version",
            "1.2.3",
            "--assets",
            dir,
            "--public-key",
            &public
        ])
        .is_err());
    }
    /// 配布物の文書の一覧と docs/ の中身が合う（`payload` も実行時に同じ確かめをする）。
    #[test]
    fn every_file_in_docs_is_bundled_or_deliberately_left_out() {
        check_docs_listed(&root()).unwrap();
        // 一覧そのものの整合: 重複なし・ASCII の名前（インストーラーの記録が ANSI のため）・docs/ の下。
        let mut names: Vec<_> = BUNDLED_DOCS.iter().chain(LEFT_OUT_DOCS).collect();
        let count = names.len();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), count, "同じ文書が二重に載っている");
        for name in names {
            assert!(name.is_ascii() && name.starts_with("docs/"), "{name}");
        }
        // 入れる文書は全部 .md。インストーラーの更新は、前の版にだけあった文書を .md の名前で消す（RemoveOldDocs）ので、
        // .md でない物（画像など）を入れる必要が出たら、installer/yolupainter.nsi の掃除の絞り込みも直す。
        for name in BUNDLED_DOCS {
            assert!(
                name.ends_with(".md"),
                "{name}: .md 以外は installer/yolupainter.nsi の RemoveOldDocs も直してから入れる"
            );
        }
        // 開発・リリースの手順は配布物に入れない。英語の文書（docs/en/ の .md）は全部入る。
        assert!(
            LEFT_OUT_DOCS.contains(&"docs/DEVELOPMENT.md")
                && LEFT_OUT_DOCS.contains(&"docs/RELEASING.md")
        );
        let english: Vec<String> = docs_files(&root())
            .unwrap()
            .into_iter()
            .filter(|file| file.starts_with("docs/en/") && file.ends_with(".md"))
            .collect();
        assert!(!english.is_empty(), "docs/en/ に文書が無い");
        for file in english {
            assert!(
                BUNDLED_DOCS.contains(&file.as_str()),
                "{file}: 英語の文書は配布物に入れる"
            );
        }
    }
    #[test]
    fn unlisted_missing_and_doubly_listed_docs_are_refused() {
        // 実際の一覧どおりの docs/ の写し。
        let write_docs = |root: &Path| {
            for name in BUNDLED_DOCS.iter().chain(LEFT_OUT_DOCS) {
                let path = root.join(name);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(path, name).unwrap();
            }
        };
        let d = Scratch::new();
        write_docs(&d.0);
        check_docs_listed(&d.0).unwrap();
        // 載せ忘れ（.md でも画像でも）は名前つきで断る。
        for stray in ["docs/NEW.md", "docs/en/NEW.md", "docs/images/screen.png"] {
            let path = d.0.join(stray);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, "x").unwrap();
            let error = check_docs_listed(&d.0).unwrap_err().to_string();
            assert!(error.contains(stray), "{stray}: {error}");
            fs::remove_file(path).unwrap();
        }
        fs::remove_dir(d.0.join("docs/images")).unwrap();
        check_docs_listed(&d.0).unwrap();
        // 一覧にあるのに無い文書（消した・名前を変えた）も断る。
        fs::remove_file(d.0.join("docs/PSD.md")).unwrap();
        let error = check_docs_listed(&d.0).unwrap_err().to_string();
        assert!(error.contains("docs/PSD.md"), "{error}");
    }
    #[test]
    fn payload_lists_root_files_and_docs_for_each_target() {
        for (target, exe, cli) in [
            (WINDOWS_ARCHIVE, "yolupainter.exe", "yolupainter-cli.exe"),
            (LINUX_ARCHIVE, "yolupainter", "yolupainter-cli"),
        ] {
            let names = payload_names(target);
            let mut sorted = names.clone();
            sorted.sort();
            sorted.dedup();
            assert_eq!(sorted.len(), names.len(), "{target}: 名前が重複している");
            assert!(names.iter().all(|n| !n.contains('\\')), "{target}");
            for expected in [
                exe,
                cli,
                "LICENSE",
                "README.md",
                "README.en.md",
                "THIRD_PARTY.md",
                "DEPENDENCIES.md",
                "THIRD_PARTY_LICENSES.txt",
                "docs/GUIDE.md",
                "docs/CLI.md",
                "docs/MCP.md",
                "docs/en/CLI.md",
                "docs/en/MCP.md",
                "docs/BRUSH_IMPORT.md",
                "docs/GRADIENT_MAP.md",
                "docs/en/GUIDE.md",
                "docs/en/BUILDING.md",
            ] {
                assert!(names.iter().any(|n| n == expected), "{target}: {expected}");
            }
            for left_out in ["docs/DEVELOPMENT.md", "docs/RELEASING.md"] {
                assert!(!names.iter().any(|n| n == left_out), "{target}: {left_out}");
            }
        }
    }
    #[test]
    fn payload_sources_are_the_repository_files_the_build_and_the_license_bundle() {
        let root = root();
        let license_dir = Path::new("/licenses");
        for target in [WINDOWS_ARCHIVE, LINUX_ARCHIVE] {
            let entries = payload_entries(&root, target, license_dir);
            let source = |name: &str| {
                entries
                    .iter()
                    .find(|(n, _)| n == name)
                    .unwrap_or_else(|| panic!("{name}"))
                    .1
                    .clone()
            };
            // アプリとコマンドラインの実行ファイルは、どちらもビルドの出力（`build` が同じ組みで作る）
            for built in executables(target) {
                assert_eq!(
                    source(built),
                    root.join("target").join(target).join("release").join(built)
                );
            }
            assert_eq!(
                source("DEPENDENCIES.md"),
                license_dir.join("THIRD_PARTY.md")
            );
            assert_eq!(
                source("THIRD_PARTY_LICENSES.txt"),
                license_dir.join("THIRD_PARTY_LICENSES.txt")
            );
            // 文書と README は、配布物の中と同じ相対の場所のリポジトリのファイル（建てなくても在る）。
            for (name, path) in &entries {
                if executables(target).contains(&name.as_str())
                    || name.starts_with("DEPENDENCIES")
                    || name.starts_with("THIRD_PARTY_L")
                {
                    continue;
                }
                assert_eq!(path, &root.join(name));
                assert!(path.is_file(), "{name} が無い");
            }
        }
    }
    #[test]
    fn a_missing_source_is_refused_by_name() {
        let d = Scratch::new();
        let mut entries = fake_payload(&d.0, WINDOWS_ARCHIVE);
        require_sources(&entries).unwrap();
        fs::remove_file(&entries[9].1).unwrap();
        let error = require_sources(&entries).unwrap_err().to_string();
        assert!(error.contains(&entries[9].0), "{error}");
        // フォルダは通常のファイルではない。
        let folder = d.0.join("folder");
        fs::create_dir_all(&folder).unwrap();
        entries[9].1 = folder;
        assert!(require_sources(&entries).is_err());
    }
    /// Markdown の中の、配布物の外へ出る・外にある物へのリンクの先（`](先)` と `[名]: 先`）。コードのかたまりと `コード` の中は読まない。
    fn markdown_link_targets(text: &str) -> Vec<String> {
        let mut targets = Vec::new();
        let mut in_fence = false;
        for line in text.lines() {
            if line.trim_start().starts_with("```") {
                in_fence = !in_fence;
                continue;
            }
            if in_fence {
                continue;
            }
            let mut plain = String::new();
            let mut in_code = false;
            for c in line.chars() {
                if c == '`' {
                    in_code = !in_code;
                } else if !in_code {
                    plain.push(c);
                }
            }
            let mut rest = plain.as_str();
            while let Some(at) = rest.find("](") {
                let after = &rest[at + 2..];
                let Some(end) = after.find(')') else { break };
                targets.extend(after[..end].split_whitespace().next().map(str::to_owned));
                rest = &after[end + 1..];
            }
            let trimmed = plain.trim_start();
            if trimmed.starts_with('[') {
                if let Some(at) = trimmed.find("]:") {
                    targets.extend(
                        trimmed[at + 2..]
                            .split_whitespace()
                            .next()
                            .map(str::to_owned),
                    );
                }
            }
        }
        targets
    }
    /// `from`（配布物の中の名前）から見た相対の `target` が指す、配布物の中の名前（`..` をたどる。根より上へ出れば None）。
    fn resolve_in_bundle(from: &str, target: &str) -> Option<String> {
        let mut parts: Vec<&str> = from.split('/').collect();
        parts.pop();
        for part in target.split('/') {
            match part {
                "" | "." => {}
                ".." => {
                    parts.pop()?;
                }
                part => parts.push(part),
            }
        }
        Some(parts.join("/"))
    }
    #[test]
    fn markdown_link_scanner_and_resolver() {
        let text = "[a](x.md) と [`b`](../y.md#h \"題\") と `[c](skip.md)` と ![i](img/p.png)\n\
                    ```\n[d](skip2.md)\n```\n[ref]: docs/z.md\n[e](https://example.invalid/q) <https://x>";
        assert_eq!(
            markdown_link_targets(text),
            [
                "x.md",
                "../y.md#h",
                "img/p.png",
                "docs/z.md",
                "https://example.invalid/q"
            ]
        );
        assert_eq!(
            resolve_in_bundle("docs/en/GUIDE.md", "../../LICENSE").unwrap(),
            "LICENSE"
        );
        assert_eq!(
            resolve_in_bundle("docs/GUIDE.md", "en/GUIDE.md").unwrap(),
            "docs/en/GUIDE.md"
        );
        assert_eq!(
            resolve_in_bundle("README.md", "docs/./GUIDE.md").unwrap(),
            "docs/GUIDE.md"
        );
        // 配布物の根より上へ出るリンクは、どこにも解決できない。
        assert!(resolve_in_bundle("README.md", "../x.md").is_none());
        assert!(resolve_in_bundle("docs/GUIDE.md", "../../x.md").is_none());
    }
    /// 配布物に入れる文書の相対のリンクが、配布物の中で切れない（開発の文書・crates/ の README などへは GitHub の URL で張る）。
    #[test]
    fn links_in_bundled_documents_resolve_inside_the_bundle() {
        let names = payload_names(WINDOWS_ARCHIVE);
        let mut broken = Vec::new();
        let mut checked = 0;
        for name in &names {
            // 許諾の束は `tools/third-party.py` が作る（リポジトリには無い）。
            if !name.ends_with(".md") || name == "DEPENDENCIES.md" {
                continue;
            }
            let text = fs::read_to_string(root().join(name)).unwrap();
            for target in markdown_link_targets(&text) {
                if target.starts_with('#')
                    || target.contains("://")
                    || target.starts_with("mailto:")
                {
                    continue;
                }
                let path = target.split(['#', '?']).next().unwrap();
                let resolved = resolve_in_bundle(name, path);
                checked += 1;
                if !resolved.is_some_and(|r| names.contains(&r)) {
                    broken.push(format!("{name} -> {target}"));
                }
            }
        }
        assert!(checked > 40, "リンクの走査が空に近い: {checked}");
        assert!(
            broken.is_empty(),
            "配布物の中で切れるリンク:\n{}",
            broken.join("\n")
        );
    }
    #[test]
    fn stage_keeps_folders_and_drops_the_previous_stage() {
        let d = Scratch::new();
        let stage = d.0.join("stage");
        fs::create_dir_all(stage.join("docs")).unwrap();
        fs::write(stage.join("docs/OLD.md"), "old").unwrap();
        let entries = fake_payload(&d.0, WINDOWS_ARCHIVE);
        stage_payload(&entries, &stage).unwrap();
        assert!(!stage.join("docs/OLD.md").exists(), "前回の段が残っている");
        for name in payload_names(WINDOWS_ARCHIVE) {
            assert_eq!(fs::read_to_string(stage.join(&name)).unwrap(), name);
        }
        assert!(stage.join("docs/en/GUIDE.md").is_file());
        // 元が無ければ断る（中途半端な段を渡さない）。
        let broken = vec![("docs/x/y.md".to_owned(), d.0.join("missing"))];
        assert!(stage_payload(&broken, &stage).is_err());
    }
    #[test]
    fn archive_contents_must_match_the_payload_list() {
        let v = Version::new(1, 2, 3);
        for target in [WINDOWS_ARCHIVE, LINUX_ARCHIVE, MACOS_ARCHIVE] {
            check_archive_contents(target, &v, &fake_archive(&v, target, &[], &[])).unwrap();
            // 文書が 1 つ欠けても、README が欠けても断る（名前つきで）。
            for missing in ["docs/en/GUIDE.md", "docs/PSD.md", "README.en.md"] {
                let error =
                    check_archive_contents(target, &v, &fake_archive(&v, target, &[missing], &[]))
                        .unwrap_err()
                        .to_string();
                assert!(
                    error.contains("足りない") && error.contains(missing),
                    "{target}: {error}"
                );
            }
            // 入れない文書・一覧に無いファイルが紛れても断る。
            for extra in [
                "docs/DEVELOPMENT.md",
                "docs/RELEASING.md",
                "docs/NEW.md",
                "stray.txt",
            ] {
                let error =
                    check_archive_contents(target, &v, &fake_archive(&v, target, &[], &[extra]))
                        .unwrap_err()
                        .to_string();
                assert!(
                    error.contains("余計") && error.contains(extra),
                    "{target}: {error}"
                );
            }
            // アーカイブでないものは中身を読めずに断る。
            assert!(check_archive_contents(target, &v, b"archive").is_err());
        }
        // 同じ名前が 2 回入っている tar.gz（zip は作る時点で断られる）。
        let d = Scratch::new();
        let mut entries = fake_payload(&d.0, LINUX_ARCHIVE);
        entries.push(entries[3].clone());
        let path = d.0.join("dup.tar.gz");
        archive(&path, &entries, false).unwrap();
        let error = check_archive_contents(LINUX_ARCHIVE, &v, &fs::read(path).unwrap())
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("重複") && error.contains(&entries[3].0),
            "{error}"
        );
    }
    /// macOS の zip は最上位のフォルダの名前が版を含む: 別の版のフォルダ名・フォルダの無い平らな zip・`__MACOSX` の項目は断る。
    #[test]
    fn the_macos_archive_must_sit_in_the_folder_of_its_own_version() {
        let v = Version::new(1, 2, 3);
        let other = Version::new(1, 2, 4);
        let bytes = fake_archive(&v, MACOS_ARCHIVE, &[], &[]);
        check_archive_contents(MACOS_ARCHIVE, &v, &bytes).unwrap();
        let error = check_archive_contents(MACOS_ARCHIVE, &other, &bytes)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("足りない") && error.contains("1.2.4"),
            "{error}"
        );
        // フォルダの無い（Windows と同じ平らな）zip
        let flat = fake_archive(&v, WINDOWS_ARCHIVE, &[], &[]);
        assert!(check_archive_contents(MACOS_ARCHIVE, &v, &flat).is_err());
        // ditto が足し得る、リソースフォークの AppleDouble の項目（実際の形: 最上位の `__MACOSX/<束のフォルダ>/._…`）
        let folder = macos::folder_name(&v);
        let stray_name = format!("__MACOSX/{folder}/._yolupainter");
        let stray = fake_archive(&v, MACOS_ARCHIVE, &[], &[stray_name.as_str()]);
        let error = check_archive_contents(MACOS_ARCHIVE, &v, &stray)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("余計") && error.contains(&stray_name),
            "{error}"
        );
    }
    #[test]
    fn verify_refuses_archives_whose_contents_differ_from_the_list() {
        let v = Version::new(1, 2, 3);
        let run = |dir: &Path, public: &str| {
            exec(&[
                "verify",
                "--version",
                "1.2.3",
                "--assets",
                dir.to_str().unwrap(),
                "--public-key",
                public,
            ])
        };
        let (d, public) = signed_dist(&v, &[0, 1]);
        run(&d.0, &public).unwrap();
        // 署名も大きさも SHA-256 も正しいが、文書が欠けた配布物 / 余計なファイルのある配布物は断る。
        for (target, name) in [(0, "docs/GUIDE.md"), (1, "docs/en/UNITY.md")] {
            let d = Scratch::new();
            fs::write(
                asset_path(&d.0, &v, target),
                fake_archive(&v, TARGETS[target], &[name], &[]),
            )
            .unwrap();
            if target == 0 {
                fs::write(asset_path(&d.0, &v, 2), "installer").unwrap();
            }
            updater(v.clone(), &d.0, true, || Ok(disposable_key())).unwrap();
            let public = hex::encode(disposable_key().verifying_key().to_bytes());
            let error = run(&d.0, &public).unwrap_err().to_string();
            assert!(
                error.contains(name) && error.contains("足りない"),
                "{error}"
            );
            assert!(
                error.contains(&asset_name(&v, TARGETS[target]).unwrap()),
                "{error}"
            );
        }
        for target in [0, 1] {
            let d = Scratch::new();
            fs::write(
                asset_path(&d.0, &v, target),
                fake_archive(&v, TARGETS[target], &[], &["docs/DEVELOPMENT.md"]),
            )
            .unwrap();
            if target == 0 {
                fs::write(asset_path(&d.0, &v, 2), "installer").unwrap();
            }
            updater(v.clone(), &d.0, true, || Ok(disposable_key())).unwrap();
            let public = hex::encode(disposable_key().verifying_key().to_bytes());
            let error = run(&d.0, &public).unwrap_err().to_string();
            assert!(
                error.contains("docs/DEVELOPMENT.md") && error.contains("余計"),
                "{error}"
            );
        }
    }
    /// インストーラーのスクリプト（`File`・`Delete`・`RMDir` と `DocFiles`）が、xtask の配布物の一覧と同じファイルを入れて消す。
    #[test]
    fn installer_script_installs_and_removes_exactly_the_payload() {
        use std::collections::BTreeSet;
        let script = fs::read_to_string(root().join("installer/yolupainter.nsi")).unwrap();
        let script = script.trim_start_matches('\u{feff}');
        let (mut installed, mut deleted, mut docs, mut folders) = (
            BTreeSet::new(),
            BTreeSet::new(),
            BTreeSet::new(),
            BTreeSet::new(),
        );
        let name = |rest: &str| {
            rest.trim_end_matches('"')
                .replace("${EXE}", "yolupainter.exe")
                .replace("${CLI_EXE}", "yolupainter-cli.exe")
                .replace("${UNINSTALLER}", "uninstall.exe")
        };
        for line in script.lines().map(str::trim) {
            if let Some(rest) = line.strip_prefix("File \"${STAGE}\\") {
                installed.insert(name(rest));
            } else if let Some(rest) = line.strip_prefix("Delete \"$INSTDIR\\") {
                deleted.insert(name(rest));
            } else if let Some(rest) = line.strip_prefix("RMDir \"$INSTDIR\\") {
                folders.insert(name(rest));
            } else if let Some(rest) = line.strip_prefix("!insertmacro ${ACTION} ") {
                let quoted: Vec<_> = rest.split('"').skip(1).step_by(2).collect();
                assert_eq!(quoted.len(), 2, "{line}");
                docs.insert(format!("{}/{}", quoted[0].replace('\\', "/"), quoted[1]));
            }
        }
        // マクロの本体（`${DIR}\${NAME}`・`$R1`）は一覧ではない。
        installed.retain(|n| !n.contains('$'));
        deleted.retain(|n| !n.contains('$'));
        let names = payload_names(WINDOWS_ARCHIVE);
        let root_files: BTreeSet<String> =
            names.iter().filter(|n| !n.contains('/')).cloned().collect();
        let doc_files: BTreeSet<String> =
            names.iter().filter(|n| n.contains('/')).cloned().collect();
        assert_eq!(installed, root_files, "File の一覧が配布物の一覧と違う");
        assert_eq!(docs, doc_files, "DocFiles の一覧が配布物の一覧と違う");
        let mut expected_deleted = root_files.clone();
        expected_deleted.insert("uninstall.exe".into());
        assert_eq!(
            deleted, expected_deleted,
            "アンインストールで消すファイルが入れるファイルと違う"
        );
        // 文書のフォルダは、空になったら消す（`RMDir` は空のときだけ消す。`/r` は使わない）。
        let expected_folders: BTreeSet<String> = doc_files
            .iter()
            .flat_map(|n| {
                let parts: Vec<_> = n.split('/').collect();
                (1..parts.len()).map(move |i| parts[..i].join("\\"))
            })
            .collect();
        assert_eq!(folders, expected_folders);
        assert!(
            !script.contains("RMDir /r \"$INSTDIR"),
            "入れ先を丸ごとは消さない"
        );
    }

    // ───────── 試験版の置き場（beta-channel） ─────────

    /// 試験版 `version`（zip・インストーラー）を、使い捨ての鍵で署名した置き場と、その公開鍵。
    fn beta_dist(version: &str) -> (Scratch, String) {
        signed_dist(&Version::parse(version).unwrap(), &[0])
    }
    fn channel_args<'a>(
        version: &'a str,
        assets: &'a Path,
        key: &'a str,
        output: &'a Path,
        existing: Option<&'a Path>,
    ) -> Vec<&'a str> {
        let mut list = vec![
            "beta-channel",
            "--version",
            version,
            "--assets",
            assets.to_str().unwrap(),
            "--public-key",
            key,
            "--output",
            output.to_str().unwrap(),
        ];
        if let Some(path) = existing {
            list.extend(["--existing", path.to_str().unwrap()]);
        }
        list
    }
    #[test]
    fn beta_channel_copies_the_signed_manifest_unchanged_and_the_app_accepts_it_at_the_beta_url() {
        let (dist, key) = beta_dist("1.2.0-rc.1");
        let out = Scratch::new();
        exec(&channel_args("1.2.0-rc.1", &dist.0, &key, &out.0, None)).unwrap();
        // 原本のバイト列のまま（再署名しない）。一時ファイルは残らない
        let original = fs::read(dist.0.join(UPDATER_FILE)).unwrap();
        assert_eq!(fs::read(out.0.join(UPDATER_FILE)).unwrap(), original);
        assert_eq!(fs::read_dir(&out.0).unwrap().count(), 1);
        // 写した物を試験版の置き場の URL で返す偽の口に置くと、同じ鍵で検証を通り、試験版が見つかる
        struct Place(Vec<u8>);
        impl Transport for Place {
            fn get(&self, url: &str, _: usize) -> std::result::Result<Vec<u8>, yolu_update::Error> {
                if url == yolu_update::BETA_UPDATER_URL {
                    Ok(self.0.clone())
                } else {
                    Err(yolu_update::Error("通信できません".into()))
                }
            }
        }
        let client = UpdateClient::with_public_key(
            Place(fs::read(out.0.join(UPDATER_FILE)).unwrap()),
            public_key_bytes(&key).unwrap(),
        )
        .unwrap();
        let found = client
            .check_channels(
                yolu_update::UPDATER_URL,
                Some(yolu_update::BETA_UPDATER_URL),
                &Version::new(1, 1, 0),
                TARGETS[0],
            )
            .unwrap()
            .unwrap();
        assert_eq!(found.version().to_string(), "1.2.0-rc.1");
        // 別の鍵を信じるアプリは受けない
        let other = UpdateClient::with_public_key(
            Place(fs::read(out.0.join(UPDATER_FILE)).unwrap()),
            SigningKey::from_bytes(&[43; 32]).verifying_key().to_bytes(),
        )
        .unwrap();
        assert!(other
            .check(
                yolu_update::BETA_UPDATER_URL,
                &Version::new(1, 1, 0),
                TARGETS[0],
                true
            )
            .is_err());
    }
    #[test]
    fn beta_channel_refuses_versions_that_are_not_beta_and_leaves_no_file() {
        for version in ["1.2.0", "1.2.0-preview.1", "1.2.0-rc1", "1.2.0-rc"] {
            let (dist, key) = beta_dist(version);
            let out = Scratch::new();
            // 前の実行の残りも消す（古い置き場の物を、今回の成果として渡さない）
            fs::write(out.0.join(UPDATER_FILE), "stale").unwrap();
            assert!(
                exec(&channel_args(version, &dist.0, &key, &out.0, None)).is_err(),
                "{version}"
            );
            assert_no_metadata(&out.0);
        }
    }
    #[test]
    fn beta_channel_checks_the_signature_and_the_assets_before_copying() {
        let (dist, key) = beta_dist("1.2.0-rc.1");
        let version = Version::parse("1.2.0-rc.1").unwrap();
        // 別の鍵
        let out = Scratch::new();
        let other = hex::encode(SigningKey::from_bytes(&[43; 32]).verifying_key().to_bytes());
        assert!(exec(&channel_args("1.2.0-rc.1", &dist.0, &other, &out.0, None)).is_err());
        assert_no_metadata(&out.0);
        // 版が違う（更新情報は 1.2.0-rc.1 のもの）
        assert!(exec(&channel_args("1.2.0-rc.2", &dist.0, &key, &out.0, None)).is_err());
        assert_no_metadata(&out.0);
        // 公開後に配布物が差し替えられた（大きさ・SHA-256 が合わない）
        fs::write(
            asset_path(&dist.0, &version, 0),
            b"replaced after the draft",
        )
        .unwrap();
        assert!(exec(&channel_args("1.2.0-rc.1", &dist.0, &key, &out.0, None)).is_err());
        assert_no_metadata(&out.0);
        // 署名の無い更新情報
        let (unsigned, key) = beta_dist("1.2.0-rc.1");
        fs::write(
            unsigned.0.join(UPDATER_FILE),
            serde_json::to_vec(&Envelope {
                payload: serde_json::to_string(&Manifest {
                    schema: UPDATER_SCHEMA,
                    version: version.to_string(),
                    assets: vec![],
                })
                .unwrap(),
                signature: None,
            })
            .unwrap(),
        )
        .unwrap();
        assert!(exec(&channel_args("1.2.0-rc.1", &unsigned.0, &key, &out.0, None)).is_err());
        assert_no_metadata(&out.0);
    }
    #[test]
    fn beta_channel_never_moves_the_place_back_to_an_older_or_the_same_beta() {
        let written = |out: &Scratch| out.0.join(UPDATER_FILE).exists();
        let (dist, key) = beta_dist("1.2.0-rc.2");
        let place = Scratch::new();
        let held = |version: &str| {
            let (d, _) = beta_dist(version);
            let path = place.0.join(format!("held-{version}.json"));
            fs::copy(d.0.join(UPDATER_FILE), &path).unwrap();
            path
        };
        // 置き場の版が同じ・新しい（rc.10 は rc.2 より新しい。辞書順ではない）: 置き換えない（成功で、何も書かない）
        for older_or_same in ["1.2.0-rc.2", "1.2.0-rc.10", "1.3.0-beta.1"] {
            let out = Scratch::new();
            let existing = held(older_or_same);
            exec(&channel_args(
                "1.2.0-rc.2",
                &dist.0,
                &key,
                &out.0,
                Some(&existing),
            ))
            .unwrap();
            assert!(!written(&out), "{older_or_same}");
        }
        // 置き場の版が古い・置き場が空・壊れている・まだ無い: 置く
        let garbage = place.0.join("garbage.json");
        fs::write(&garbage, "not json").unwrap();
        for existing in [
            held("1.2.0-rc.1"),
            held("1.2.0-beta.7"),
            garbage,
            place.0.join("absent.json"),
        ] {
            let out = Scratch::new();
            exec(&channel_args(
                "1.2.0-rc.2",
                &dist.0,
                &key,
                &out.0,
                Some(&existing),
            ))
            .unwrap();
            assert!(written(&out), "{existing:?}");
        }
    }
}
