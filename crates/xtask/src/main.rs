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
const USAGE: &str = "命令: build --target T --release [--require-update-key] / bundle --target T / installer --target T / updater-json --version V --assets DIR [--sign] [--key-file PATH] / verify --version V --assets DIR --public-key HEX / keygen --output PATH / pubkey --key-file PATH";
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
            "--target" if matches!(command.as_str(), "build" | "bundle" | "installer") => {
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
        "build" | "bundle" | "installer" => {
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
/// アーカイブ・インストーラーに入れるファイル（名前 → 元）。許諾の全文の束もここで作る。
fn payload(root: &Path, target: &str) -> Result<Vec<(String, PathBuf)>> {
    run(python(root).args([
        "tools/third-party.py",
        "--package",
        "yolu-app",
        "--include-update",
        "--target",
        target,
        "--bundle",
    ]))?;
    let license_dir = root
        .join("target/third-party")
        .join(target)
        .join("yolu-app");
    let exe = if target.contains("windows") {
        "yolupainter.exe"
    } else {
        "yolupainter"
    };
    Ok(vec![
        (
            exe.to_owned(),
            root.join("target").join(target).join("release").join(exe),
        ),
        ("LICENSE".into(), root.join("LICENSE")),
        ("README.md".into(), root.join("README.md")),
        ("THIRD_PARTY.md".into(), root.join("THIRD_PARTY.md")),
        ("DEPENDENCIES.md".into(), license_dir.join("THIRD_PARTY.md")),
        (
            "THIRD_PARTY_LICENSES.txt".into(),
            license_dir.join("THIRD_PARTY_LICENSES.txt"),
        ),
    ])
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
    let _ = fs::remove_dir_all(&stage);
    fs::create_dir_all(&stage)?;
    for (name, source) in &entries {
        fs::copy(source, stage.join(name))?;
    }
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
    for entry in fs::read_dir(directory)? {
        let name = entry?
            .file_name()
            .into_string()
            .map_err(|_| "配布物名が UTF-8 ではありません")?;
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
    for target in &present {
        let update = client
            .check(VERIFY_URL, &oldest, target, true)?
            .filter(|update| update.version() == version)
            .ok_or("更新情報の版が --version と一致しません")?;
        client.download(update.approve_download())?;
    }
    // 上で署名と本文が通っているので、ここで確かめるのは「ファイルが無いのに載っている」ことだけ。
    for (target, name) in &absent {
        if let Ok(Some(_)) = client.check(VERIFY_URL, &oldest, target, true) {
            return Err(format!("更新情報にある配布物がありません: {name}").into());
        }
    }
    println!("署名と {} 件の配布物を確認しました", present.len());
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
    #[test]
    fn zip_preserves_files_and_contents() {
        let d = Scratch::new();
        let entries: Vec<_> = [
            "yolupainter.exe",
            "LICENSE",
            "README.md",
            "THIRD_PARTY.md",
            "DEPENDENCIES.md",
            "THIRD_PARTY_LICENSES.txt",
        ]
        .into_iter()
        .map(|name| {
            let p = d.0.join(name);
            fs::write(&p, name).unwrap();
            (name.into(), p)
        })
        .collect();
        let p = d.0.join("test.zip");
        archive(&p, &entries, true).unwrap();
        let mut zip = zip::ZipArchive::new(File::open(p).unwrap()).unwrap();
        assert_eq!(zip.len(), entries.len());
        for (name, _) in entries {
            let mut s = String::new();
            zip.by_name(&name).unwrap().read_to_string(&mut s).unwrap();
            assert_eq!(s, name);
        }
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
    /// 使い捨ての鍵で署名した配布物の置き場。公開鍵の hex も返す。
    fn signed_dist(version: &Version, targets: &[usize]) -> (Scratch, String) {
        let d = Scratch::new();
        for &t in targets {
            fs::write(asset_path(&d.0, version, t), format!("archive-{t}")).unwrap();
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
        fs::create_dir_all(&stage).unwrap();
        for name in [
            "yolupainter.exe",
            "README.md",
            "THIRD_PARTY.md",
            "DEPENDENCIES.md",
            "THIRD_PARTY_LICENSES.txt",
        ] {
            fs::write(stage.join(name), name).unwrap();
        }
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
}
