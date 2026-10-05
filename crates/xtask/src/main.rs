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
    asset_name, asset_url, check_public_key, is_archive_target, sha256, Asset, Envelope, Manifest,
    Transport, UpdateClient, MAX_ASSET, RELEASE_BASE, TARGETS, UPDATER_FILE, UPDATER_SCHEMA,
    WINDOWS_ARCHIVE, WINDOWS_INSTALLER,
};
type Result<T> = std::result::Result<T, Box<dyn Error>>;
const PRIVATE_KEY_ENV: &str = "YOLUPAINTER_UPDATE_PRIVATE_KEY";
const PUBLIC_KEY_ENV: &str = "YOLUPAINTER_UPDATE_PUBLIC_KEY";
/// exe とインストーラーのアイコン（ロゴ。build.rs も同じファイルを読む）。
const LOGO_ICON: &str = "crates/yolu-app/assets/logo/yolupainter.ico";
const USAGE: &str = "命令: preflight [--target T]... [--kind stable|prerelease] [--only 確かめ,...] [--installer] [--offline] / build --target T --release [--require-update-key] / bundle --target T / installer --target T / symbols --target T / updater-json --version V --assets DIR [--sign] [--key-file PATH] / verify --version V --assets DIR --public-key HEX / keygen --output PATH / pubkey --key-file PATH";
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
    let mut target = None;
    let mut version = None;
    let mut assets = None;
    let mut key_file = None;
    let mut public_key = None;
    let mut output = None;
    let mut release = false;
    let mut sign = false;
    let mut require_update_key = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--target"
                if matches!(
                    command.as_str(),
                    "build" | "bundle" | "installer" | "symbols"
                ) =>
            {
                target = Some(value(&mut args)?)
            }
            "--version" if matches!(command.as_str(), "updater-json" | "verify") => {
                version = Some(Version::parse(&value(&mut args)?)?)
            }
            "--assets" if matches!(command.as_str(), "updater-json" | "verify") => {
                assets = Some(PathBuf::from(value(&mut args)?))
            }
            "--key-file" if matches!(command.as_str(), "updater-json" | "pubkey") => {
                key_file = Some(PathBuf::from(value(&mut args)?))
            }
            "--public-key" if command == "verify" => public_key = Some(value(&mut args)?),
            "--output" if command == "keygen" => output = Some(PathBuf::from(value(&mut args)?)),
            "--release" if command == "build" => release = true,
            "--require-update-key" if command == "build" => require_update_key = true,
            "--sign" if command == "updater-json" => sign = true,
            _ => return Err(format!("未対応の引数: {arg}").into()),
        }
    }
    match command.as_str() {
        "build" | "bundle" | "installer" | "symbols" => {
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
fn build(target: &str, require_update_key: bool) -> Result<()> {
    let key = update_public_key(env::var(PUBLIC_KEY_ENV).ok(), require_update_key)?;
    let mut command = cargo();
    command.current_dir(root()).args([
        "build",
        "--locked",
        "-p",
        "yolu-app",
        "--target",
        target,
        "--release",
    ]);
    match key {
        Some(key) => command.env(PUBLIC_KEY_ENV, key),
        // 空の値を渡さない（アプリは「組み込み済み」と見て、使えない鍵を持つ）。
        None => command.env_remove(PUBLIC_KEY_ENV),
    };
    run(&mut command)
}
/// 配布物に入れる使う人向けの文書（リポジトリの根からの相対。配布物の中でも同じ場所に入るので、README からの相対のリンクがそのまま効く）。
/// インストーラーの `installer/yolupainter.nsi` の `DocFiles` も同じ一覧で、試験が突き合わせる。
const BUNDLED_DOCS: &[&str] = &[
    "docs/GUIDE.md",
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
    "docs/WINDOW.md",
    "docs/SAVE_FOR_DISTRIBUTION.md",
    "docs/en/GUIDE.md",
    "docs/en/UNITY.md",
    "docs/en/INSTALL.md",
    "docs/en/BUILDING.md",
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
/// アーカイブ（zip・tar.gz）とインストーラーの段に入れるファイルの、配布物の中の名前（`/` 区切り）。ここが唯一の一覧。
fn payload_names(target: &str) -> Vec<String> {
    std::iter::once(exe_name(target))
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
        _ if name == exe_name(target) => {
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
            header.set_mode(if name == "yolupainter" { 0o755 } else { 0o644 });
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
    // 例外は PDB の付属物 1 つ（`symbols_name`）だけ。更新の対象ではないので、更新情報には載せない。
    let symbols = symbols_name(&version);
    for entry in fs::read_dir(directory)? {
        let name = entry?
            .file_name()
            .into_string()
            .map_err(|_| "配布物名が UTF-8 ではありません")?;
        if name == symbols {
            let meta = fs::symlink_metadata(directory.join(&name))?;
            if !meta.is_file() || meta.len() == 0 || meta.len() > MAX_ASSET {
                return Err("PDB の付属物の種類または大きさが不正です".into());
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
/// アーカイブ（zip・tar.gz）の中のファイルの名前（フォルダの項目は数えない）。
fn archive_names(target: &str, bytes: &[u8]) -> Result<Vec<String>> {
    let mut names = Vec::new();
    if target == WINDOWS_ARCHIVE {
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
/// アーカイブの中身が `payload_names` と同じか照らす。欠けも、一覧に無いファイルも、同じ名前の重複も断る（名前を並べて知らせる）。
fn check_archive_contents(target: &str, bytes: &[u8]) -> Result<()> {
    let mut found = archive_names(target, bytes)?;
    found.sort();
    let expected = payload_names(target);
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
            check_archive_contents(target, download.bytes())
                .map_err(|error| format!("{name}: {error}"))?;
            archives += 1;
        }
    }
    // PDB の付属物があれば、PDB 1 つだけの zip であること（更新情報には載らないので、署名では守られない。形だけ確かめる）。
    let symbols = directory.join(symbols_name(version));
    if fs::symlink_metadata(&symbols).is_ok() {
        check_symbols_archive(&symbols)?;
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
        payload_names(target)
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
    /// 一覧どおりの中身の（`drop` を除き、`add` を足した）本物のアーカイブのバイト列。
    fn fake_archive(target: &str, drop: &[&str], add: &[&str]) -> Vec<u8> {
        let d = Scratch::new();
        let mut entries = fake_payload(&d.0, target);
        entries.retain(|(name, _)| !drop.contains(&name.as_str()));
        for name in add {
            let source = d.0.join("extra");
            fs::write(&source, name).unwrap();
            entries.push((name.to_string(), source));
        }
        let path = d.0.join("fake-archive");
        archive(&path, &entries, target == WINDOWS_ARCHIVE).unwrap();
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
                alter(t, fake_archive(TARGETS[t], &[], &[])),
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
        let wrong = fake_archive(WINDOWS_ARCHIVE, &[], &[]);
        fs::write(d.0.join(symbols_name(&v)), wrong).unwrap();
        assert!(verify_with(&d.0, &public).is_err());
        let mut zip = zip::ZipWriter::new(File::create(d.0.join(symbols_name(&v))).unwrap());
        zip.start_file("other.pdb", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"pdb").unwrap();
        zip.finish().unwrap();
        assert!(verify_with(&d.0, &public).is_err());
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
        for (target, exe) in [
            (WINDOWS_ARCHIVE, "yolupainter.exe"),
            (LINUX_ARCHIVE, "yolupainter"),
        ] {
            let names = payload_names(target);
            let mut sorted = names.clone();
            sorted.sort();
            sorted.dedup();
            assert_eq!(sorted.len(), names.len(), "{target}: 名前が重複している");
            assert!(names.iter().all(|n| !n.contains('\\')), "{target}");
            for expected in [
                exe,
                "LICENSE",
                "README.md",
                "README.en.md",
                "THIRD_PARTY.md",
                "DEPENDENCIES.md",
                "THIRD_PARTY_LICENSES.txt",
                "docs/GUIDE.md",
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
            let exe = exe_name(target);
            assert_eq!(
                source(exe),
                root.join("target").join(target).join("release").join(exe)
            );
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
                if name == exe
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
        for target in [WINDOWS_ARCHIVE, LINUX_ARCHIVE] {
            check_archive_contents(target, &fake_archive(target, &[], &[])).unwrap();
            // 文書が 1 つ欠けても、README が欠けても断る（名前つきで）。
            for missing in ["docs/en/GUIDE.md", "docs/PSD.md", "README.en.md"] {
                let error = check_archive_contents(target, &fake_archive(target, &[missing], &[]))
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
                let error = check_archive_contents(target, &fake_archive(target, &[], &[extra]))
                    .unwrap_err()
                    .to_string();
                assert!(
                    error.contains("余計") && error.contains(extra),
                    "{target}: {error}"
                );
            }
            // アーカイブでないものは中身を読めずに断る。
            assert!(check_archive_contents(target, b"archive").is_err());
        }
        // 同じ名前が 2 回入っている tar.gz（zip は作る時点で断られる）。
        let d = Scratch::new();
        let mut entries = fake_payload(&d.0, LINUX_ARCHIVE);
        entries.push(entries[3].clone());
        let path = d.0.join("dup.tar.gz");
        archive(&path, &entries, false).unwrap();
        let error = check_archive_contents(LINUX_ARCHIVE, &fs::read(path).unwrap())
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("重複") && error.contains(&entries[3].0),
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
                fake_archive(TARGETS[target], &[name], &[]),
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
                fake_archive(TARGETS[target], &[], &["docs/DEVELOPMENT.md"]),
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
}
