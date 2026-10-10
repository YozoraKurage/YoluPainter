//! macOS の試作の配布物: Intel 向けと Apple Silicon 向けの両方のコードを持つ universal の `YoluPainter.app` を、ad-hoc の署名だけで zip にする。
//!
//! 流れ（`cargo xtask build --target universal-apple-darwin --release` → `cargo xtask bundle --target universal-apple-darwin`）:
//! - `build`: `aarch64-apple-darwin` と `x86_64-apple-darwin` で同じ命令（`-p yolu-app -p yolu-cli`）をビルドし、`lipo` で 1 つにする
//!   （`target/universal-apple-darwin/release/`）。最低の macOS の版は両方で同じ `MIN_MACOS` に揃える（`MACOSX_DEPLOYMENT_TARGET`）。
//! - `bundle`: `YoluPainter.app/Contents/{Info.plist,MacOS,Resources}` を作り、既存のロゴからアイコン（.icns）を作り、`codesign -s -`（ad-hoc）で署名し、
//!   使う人向けの文書と許諾の表記（Windows・Linux の配布物と同じ一覧）と一緒に `ditto` で zip にする。
//!
//! 署名は ad-hoc だけ（Apple Developer の署名・公証は無い）。ダウンロードした zip の `.app` は Gatekeeper に止められるので、
//! 開き方は README と Release の本文に書く。アプリは自分では入れ替えない（`yolu-app` の更新は、新しい版を知らせてリリースのページを開くだけ）。
//! `lipo`・`codesign`・`iconutil`・`sips`・`ditto`・`plutil` は macOS にしか無いので、`build`・`bundle` は macOS の上でだけ動く
//! （`Info.plist` の中身・同梱の一覧・コマンドの引数は、どの OS でも試験で確かめる）。
use super::{
    check_archive_contents, check_docs_listed, cli_exe_name, exe_name, remove_if_present,
    require_sources, root, run, third_party, workspace_version, Result, BUNDLED_DOCS, ROOT_FILES,
};
use semver::Version;
use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
use yolu_update::{asset_name, MACOS_ARCHIVE, MACOS_TRIPLES};

/// アプリの束（bundle）の名前。
pub(crate) const APP_DIR: &str = "YoluPainter.app";
/// アプリの束の識別子（`CFBundleIdentifier`）。ad-hoc の署名の識別子にも使う。
pub(crate) const BUNDLE_ID: &str = "net.yozolab.yolupainter";
/// アイコンのファイル名（拡張子なし。`CFBundleIconFile`）。
const ICON_NAME: &str = "YoluPainter";
/// アイコンの元になる、今あるアプリのロゴ（新しく描かない）。`.icns` にするのは `iconutil`。
pub(crate) const ICON_SOURCE: &str = "crates/yolu-app/assets/logo/yolupainter-1024.png";
/// 実行ファイルが要求する最低の macOS の版。aarch64 の Mac が動かせる最低の版（Apple Silicon は macOS 11 から）に両方のスライスを揃える。
/// 動作を確かめた版とは別（確かめたのは文書に書いた PC だけ）。`LSMinimumSystemVersion` にも入る。
pub(crate) const MIN_MACOS: &str = "11.0";
/// `.icns` の元になる `.iconset` の中身（ファイル名と、書き出す一辺の画素数）。macOS の標準の組。
pub(crate) const ICONSET: [(&str, u32); 10] = [
    ("icon_16x16.png", 16),
    ("icon_16x16@2x.png", 32),
    ("icon_32x32.png", 32),
    ("icon_32x32@2x.png", 64),
    ("icon_128x128.png", 128),
    ("icon_128x128@2x.png", 256),
    ("icon_256x256.png", 256),
    ("icon_256x256@2x.png", 512),
    ("icon_512x512.png", 512),
    ("icon_512x512@2x.png", 1024),
];
/// ロゴの一辺（`ICONSET` の最大。これより小さい元からは作らない）。
const ICON_SOURCE_SIZE: u32 = 1024;
/// 署名が束に足すファイル（`codesign` が作る）。
const SIGNATURE_FILE: &str = "Contents/_CodeSignature/CodeResources";

/// `.app` の中の、実行ファイルのパス（`APP_DIR` の中の相対）。
fn app_executable(name: &str) -> String {
    format!("Contents/MacOS/{name}")
}

/// zip の最上位のフォルダの名前（配布物の名前から拡張子を除いた物。`ditto --keepParent` が、このフォルダごと入れる）。
pub(crate) fn folder_name(version: &Version) -> String {
    let name = asset_name(version, MACOS_ARCHIVE).expect("macOS の配布物の名前");
    name.strip_suffix(".zip").unwrap_or(&name).to_owned()
}

/// `.app` の中のファイル（`APP_DIR` の中の相対）。署名が足す `_CodeSignature` を含む。
fn app_files() -> Vec<String> {
    [
        "Contents/Info.plist".to_owned(),
        app_executable(exe_name(MACOS_ARCHIVE)),
        app_executable(cli_exe_name(MACOS_ARCHIVE)),
        format!("Contents/Resources/{ICON_NAME}.icns"),
        SIGNATURE_FILE.to_owned(),
    ]
    .into_iter()
    .map(|name| format!("{APP_DIR}/{name}"))
    .collect()
}

/// フォルダの中のファイルの名前（`/` 区切り）。ここが唯一の一覧: `.app` と、Windows・Linux の配布物と同じ文書と許諾の表記。
pub(crate) fn payload_names() -> Vec<String> {
    app_files()
        .into_iter()
        .chain(
            ROOT_FILES
                .iter()
                .chain(BUNDLED_DOCS)
                .map(|n| (*n).to_owned()),
        )
        .collect()
}

/// zip の中のファイルの名前（最上位のフォルダつき）。`check_archive_contents` がこれと照らす。
pub(crate) fn archive_names(version: &Version) -> Vec<String> {
    let folder = folder_name(version);
    payload_names()
        .into_iter()
        .map(|name| format!("{folder}/{name}"))
        .collect()
}

/// 表示の版（`CFBundleShortVersionString`）: `x.y.z` の整数だけ。試験版の識別子は入れない（macOS が整数の並びを期待する欄で、
/// 試験版かどうかは配布物の名前と更新情報の版が持つ）。
fn short_version(version: &Version) -> String {
    format!("{}.{}.{}", version.major, version.minor, version.patch)
}

/// ビルド番号（`CFBundleVersion`）: 正式版は `x.y.z`、試験版は `x.y.z` の後ろに識別子の番号を足した `x.y.z.N`
/// （`0.6.1-rc.2` → `0.6.1.2`。番号の無い識別子は `.0`）。試験版どうし・試験版と正式版を区別するための形で、
/// 新旧の並びは表さない（`alpha.2` と `rc.2` は同じ形になる。版の前後は更新情報の SemVer が決める）。
fn build_number(version: &Version) -> String {
    let base = short_version(version);
    if version.pre.is_empty() {
        return base;
    }
    let number = version
        .pre
        .as_str()
        .rsplit('.')
        .next()
        .and_then(|last| last.parse::<u64>().ok())
        .unwrap_or(0);
    format!("{base}.{number}")
}

/// `Info.plist` に入れる文字列の XML の読み替え（版・固定の文だけが入るので、`&`・`<`・`>` だけを替える）。
fn xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// `Info.plist` の中身。版はワークスペースの版（表示の版とビルド番号の形は `short_version`・`build_number`）。
///
/// 書かない物: ファイルの関連付け（`CFBundleDocumentTypes`）。macOS が `.ylp` を開く要求を渡すのは Apple Event で、アプリはまだ受けていないので、
/// 載せると「このアプリで開く」に出て、開いても何も開かない。
pub(crate) fn info_plist(version: &Version) -> String {
    let string = |text: &str| format!("<string>{}</string>", xml(text));
    let entries: Vec<(&str, String)> = vec![
        ("CFBundleDevelopmentRegion", string("en")),
        ("CFBundleExecutable", string(exe_name(MACOS_ARCHIVE))),
        ("CFBundleIconFile", string(ICON_NAME)),
        ("CFBundleIdentifier", string(BUNDLE_ID)),
        ("CFBundleInfoDictionaryVersion", string("6.0")),
        ("CFBundleName", string("YoluPainter")),
        ("CFBundleDisplayName", string("YoluPainter")),
        ("CFBundlePackageType", string("APPL")),
        (
            "CFBundleShortVersionString",
            string(&short_version(version)),
        ),
        ("CFBundleVersion", string(&build_number(version))),
        (
            "LSApplicationCategoryType",
            string("public.app-category.graphics-design"),
        ),
        ("LSMinimumSystemVersion", string(MIN_MACOS)),
        ("NSHighResolutionCapable", "<true/>".to_owned()),
        (
            "NSHumanReadableCopyright",
            string("Copyright (c) 2026 Yozolab"),
        ),
        // 2 つの GPU を持つ Mac で、軽い GPU を使えるようにする（描画は wgpu の Metal）
        ("NSSupportsAutomaticGraphicsSwitching", "<true/>".to_owned()),
    ];
    let mut text = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n<dict>\n",
    );
    for (key, value) in entries {
        text.push_str(&format!("\t<key>{key}</key>\n\t{value}\n"));
    }
    text.push_str("</dict>\n</plist>\n");
    text
}

/// `lipo -create` の引数（出力 → 入力の順に、2 つのスライスを並べる）。
pub(crate) fn lipo_args(output: &Path, inputs: &[PathBuf]) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec!["-create".into(), "-output".into(), output.into()];
    args.extend(inputs.iter().map(OsString::from));
    args
}

/// `lipo -archs` の出力が、Apple Silicon（arm64）と Intel（x86_64）の両方で、ほかを含まないこと。
pub(crate) fn check_archs(file: &str, archs: &str) -> Result<()> {
    let mut found: Vec<&str> = archs.split_whitespace().collect();
    found.sort_unstable();
    if found == ["arm64", "x86_64"] {
        Ok(())
    } else {
        Err(format!(
            "{file} が universal ではありません（アーキテクチャ: {}。要るのは arm64 と x86_64 の両方だけ）",
            if found.is_empty() {
                "なし".to_owned()
            } else {
                found.join("・")
            }
        )
        .into())
    }
}

/// `.iconset` の 1 枚を作る `sips` の引数（一辺 `size` 画素に縮める。元と同じ大きさなら縮めずそのまま写す）。
pub(crate) fn sips_args(source: &Path, size: u32, output: &Path) -> Vec<OsString> {
    vec![
        "-z".into(),
        size.to_string().into(),
        size.to_string().into(),
        source.into(),
        "--out".into(),
        output.into(),
    ]
}

/// `ditto` の引数。束ごと（`--keepParent`）zip にし、リソースフォーク・拡張属性・検疫の印は入れない（`__MACOSX/._*` の項目や
/// `com.apple.quarantine` が zip に入ると、展開した束の署名が崩れたり、検疫の印が最初から付いたりする）。
pub(crate) fn ditto_args(folder: &Path, zip: &Path) -> Vec<OsString> {
    [
        "-c",
        "-k",
        "--keepParent",
        "--norsrc",
        "--noextattr",
        "--noqtn",
    ]
    .into_iter()
    .map(OsString::from)
    .chain([OsString::from(folder), OsString::from(zip)])
    .collect()
}

/// `codesign` の引数（ad-hoc の署名: 証明書を使わず `-` で署名する）。束の中の実行ファイルも署名する（`--deep`）。
/// `--identifier` は付けない: 束の識別子は `Info.plist` の `CFBundleIdentifier`（`net.yozolab.yolupainter`）から取られ、
/// 束の中のコマンドライン（Info.plist の無い実行ファイル）には、ファイル名に中身のハッシュを足した別の識別子（`yolupainter-cli-…`）が付く。
/// `--identifier` を付けると、`--deep` で両方に同じ識別子が付いてしまう。
pub(crate) fn codesign_args(app: &Path) -> Vec<OsString> {
    ["--force", "--deep", "--sign", "-"]
        .into_iter()
        .map(OsString::from)
        .chain([OsString::from(app)])
        .collect()
}

/// macOS にしか無いツール（`build`・`bundle`・事前確認が使う）。
pub(crate) const HOST_TOOLS: [&str; 6] =
    ["lipo", "codesign", "iconutil", "sips", "ditto", "plutil"];

fn require_macos_host() -> Result<()> {
    if cfg!(target_os = "macos") {
        Ok(())
    } else {
        Err(format!(
            "macOS の配布物は macOS の上でだけ作れます（{} が要ります）",
            HOST_TOOLS.join("・")
        )
        .into())
    }
}

/// `PATH` にあるツールの場所。無ければ None（事前確認が、macOS の runner でツールが揃っているかを見る）。
pub(crate) fn find_tool(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(name))
        .find(|path| path.is_file())
}

/// `build` が作る実行ファイル（2 つのターゲットのそれぞれと、universal）。
pub(crate) fn build_outputs(root: &Path) -> Vec<PathBuf> {
    let release = |directory: &str| root.join("target").join(directory).join("release");
    MACOS_TRIPLES
        .iter()
        .copied()
        .chain([MACOS_ARCHIVE])
        .flat_map(|directory| {
            [exe_name(MACOS_ARCHIVE), cli_exe_name(MACOS_ARCHIVE)]
                .map(|name| release(directory).join(name))
        })
        .collect()
}

/// 前回の出力を消す。ビルドが途中で失敗したときに、前回の実行ファイルが残って、`bundle` が古い物を新しい版として梱包しないように、
/// `build` は最初にこれを呼ぶ。
pub(crate) fn clear_outputs(root: &Path) -> Result<()> {
    for path in build_outputs(root) {
        remove_if_present(&path)?;
    }
    Ok(())
}

/// 2 つのスライスをビルドして `lipo` で 1 つにし、`target/universal-apple-darwin/release/` に置く。
/// `cargo_release` は 1 つの Rust のターゲットのリリースビルドの命令（更新用の公開鍵の扱いを `build` と同じにして渡される）。
pub(crate) fn build(cargo_release: impl Fn(&str) -> Command) -> Result<()> {
    require_macos_host()?;
    let root = root();
    clear_outputs(&root)?;
    for triple in MACOS_TRIPLES {
        let mut command = cargo_release(triple);
        // 両方のスライスの最低の版を揃える（cc を使う依存の C のコードも同じ版で組まれる）
        command.env("MACOSX_DEPLOYMENT_TARGET", MIN_MACOS);
        run(&mut command)?;
    }
    let out = root.join("target").join(MACOS_ARCHIVE).join("release");
    fs::create_dir_all(&out)?;
    for name in [exe_name(MACOS_ARCHIVE), cli_exe_name(MACOS_ARCHIVE)] {
        let output = out.join(name);
        let inputs: Vec<PathBuf> = MACOS_TRIPLES
            .iter()
            .map(|triple| root.join("target").join(triple).join("release").join(name))
            .collect();
        run(Command::new("lipo").args(lipo_args(&output, &inputs)))?;
        let archs = Command::new("lipo").arg("-archs").arg(&output).output()?;
        check_archs(name, &String::from_utf8_lossy(&archs.stdout))?;
        println!("universal: {}", output.display());
    }
    Ok(())
}

/// アイコン（.icns）を、今あるロゴから作る。元の PNG が足りない大きさなら、何も作らず断る。
fn make_icon(root: &Path, work: &Path, destination: &Path) -> Result<()> {
    let source = root.join(ICON_SOURCE);
    check_icon_source(&source)?;
    let iconset = work.join(format!("{ICON_NAME}.iconset"));
    let _ = fs::remove_dir_all(&iconset);
    fs::create_dir_all(&iconset)?;
    for (name, size) in ICONSET {
        // sips は 1 枚ごとに元と出力の名前を標準出力へ書くので、静かにする（失敗は終了コードで分かる）
        run(Command::new("sips")
            .args(sips_args(&source, size, &iconset.join(name)))
            .stdout(Stdio::null()))?;
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    run(Command::new("iconutil")
        .args(["-c", "icns", "-o"])
        .arg(destination)
        .arg(&iconset))
}

/// PNG の頭（署名と IHDR）から、一辺の画素数を読む。PNG でなければ None。
pub(crate) fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    const MAGIC: &[u8] = b"\x89PNG\r\n\x1a\n";
    if bytes.len() < 24 || &bytes[..8] != MAGIC || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let number = |at: usize| u32::from_be_bytes(bytes[at..at + 4].try_into().expect("4 バイト"));
    Some((number(16), number(20)))
}

/// アイコンの元のロゴが PNG で、`.iconset` の最大の大きさ（1024）を縮めずに作れること。
pub(crate) fn check_icon_source(source: &Path) -> Result<()> {
    let bytes = fs::read(source)
        .map_err(|_| format!("アイコンの元のロゴがありません: {}", source.display()))?;
    match png_size(&bytes) {
        Some((width, height)) if width == ICON_SOURCE_SIZE && height == ICON_SOURCE_SIZE => Ok(()),
        Some((width, height)) => Err(format!(
            "アイコンの元のロゴが {ICON_SOURCE_SIZE} x {ICON_SOURCE_SIZE} ではありません（{width} x {height}）"
        )
        .into()),
        None => Err(format!("アイコンの元のロゴが PNG ではありません: {}", source.display()).into()),
    }
}

/// フォルダの中に置くファイル（名前 → 元）。`.app` の中の物は `bundle` が作るので、文書と許諾の表記だけ。
fn documents(root: &Path, license_dir: &Path) -> Vec<(String, PathBuf)> {
    ROOT_FILES
        .iter()
        .chain(BUNDLED_DOCS)
        .map(|name| {
            let source = match *name {
                "DEPENDENCIES.md" => license_dir.join("THIRD_PARTY.md"),
                "THIRD_PARTY_LICENSES.txt" => license_dir.join("THIRD_PARTY_LICENSES.txt"),
                _ => root.join(name),
            };
            ((*name).to_owned(), source)
        })
        .collect()
}

fn copy_into(source: &Path, destination: &Path, executable: bool) -> Result<()> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(source, destination)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = if executable { 0o755 } else { 0o644 };
        fs::set_permissions(destination, fs::Permissions::from_mode(mode))?;
    }
    #[cfg(not(unix))]
    let _ = executable;
    Ok(())
}

/// `.app` と文書を組み、署名して、zip にする（`target/dist/yolupainter-<版>-macos-universal-experimental.zip`）。
pub(crate) fn bundle() -> Result<()> {
    require_macos_host()?;
    let root = root();
    let version = workspace_version()?;
    let name = asset_name(&version, MACOS_ARCHIVE)?;
    let out = root.join("target/dist");
    fs::create_dir_all(&out)?;
    let destination = out.join(&name);
    // 前回成功した配布物を今回の失敗と取り違えない
    remove_if_present(&destination)?;

    check_docs_listed(&root)?;
    run(&mut third_party(&root, MACOS_ARCHIVE))?;
    let license_dir = root
        .join("target/third-party")
        .join(MACOS_ARCHIVE)
        .join("yolu-app");
    let documents = documents(&root, &license_dir);
    require_sources(&documents)?;
    let built = root.join("target").join(MACOS_ARCHIVE).join("release");
    let executables: Vec<(String, PathBuf)> =
        [exe_name(MACOS_ARCHIVE), cli_exe_name(MACOS_ARCHIVE)]
            .into_iter()
            .map(|executable| (executable.to_owned(), built.join(executable)))
            .collect();
    require_sources(&executables)?;
    for (executable, path) in &executables {
        let archs = Command::new("lipo").arg("-archs").arg(path).output()?;
        check_archs(executable, &String::from_utf8_lossy(&archs.stdout))?;
    }

    // 段取り用のフォルダは前回の物を消して作り直す（target/ の中）
    let work = root.join("target/macos").join(MACOS_ARCHIVE);
    let _ = fs::remove_dir_all(&work);
    let folder = work.join(folder_name(&version));
    let app = folder.join(APP_DIR);
    for (executable, path) in &executables {
        copy_into(path, &app.join(app_executable(executable)), true)?;
    }
    let plist = app.join("Contents/Info.plist");
    fs::write(&plist, info_plist(&version))?;
    run(Command::new("plutil").arg("-lint").arg(&plist))?;
    make_icon(
        &root,
        &work,
        &app.join(format!("Contents/Resources/{ICON_NAME}.icns")),
    )?;
    for (document, source) in &documents {
        copy_into(source, &folder.join(document), false)?;
    }

    // 署名は束の中身が全部そろってから 1 回だけ（署名のあとに束の中を変えると、署名が崩れる）
    run(Command::new("codesign").args(codesign_args(&app)))?;
    run(Command::new("codesign")
        .args(["--verify", "--deep", "--strict", "--verbose=2"])
        .arg(&app))?;

    // 一時のファイルに作ってから最後に 1 回の rename で置く（失敗した作りかけを配布物として見せない）
    let temporary = destination.with_extension("tmp");
    let _ = fs::remove_file(&temporary);
    let zipped = run(Command::new("ditto").args(ditto_args(&folder, &temporary))).and_then(|_| {
        // 作った zip の中身が、一覧と同じ（`__MACOSX` などが入っていない）ことを、その場で確かめる
        check_archive_contents(MACOS_ARCHIVE, &version, &fs::read(&temporary)?)?;
        Ok(fs::rename(&temporary, &destination)?)
    });
    if let Err(error) = zipped {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    println!("配布物: {}", destination.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version() -> Version {
        Version::parse("0.6.0").unwrap()
    }

    /// `<key>K</key>` の次の値の行（`<string>…</string>` か `<true/>`）。
    fn value_of(plist: &str, key: &str) -> Option<String> {
        let mut lines = plist.lines().map(str::trim);
        while let Some(line) = lines.next() {
            if line == format!("<key>{key}</key>") {
                return lines.next().map(str::to_owned);
            }
        }
        None
    }

    #[test]
    fn info_plist_names_the_app_the_version_and_the_bundle() {
        let plist = info_plist(&version());
        let string = |key: &str| {
            let value = value_of(&plist, key).unwrap_or_else(|| panic!("{key} が無い"));
            value
                .strip_prefix("<string>")
                .and_then(|v| v.strip_suffix("</string>"))
                .unwrap_or_else(|| panic!("{key} が文字列ではない: {value}"))
                .to_owned()
        };
        assert_eq!(string("CFBundleIdentifier"), "net.yozolab.yolupainter");
        assert_eq!(string("CFBundleExecutable"), "yolupainter");
        assert_eq!(string("CFBundleName"), "YoluPainter");
        assert_eq!(string("CFBundlePackageType"), "APPL");
        assert_eq!(string("CFBundleShortVersionString"), "0.6.0");
        assert_eq!(string("CFBundleVersion"), "0.6.0");
        assert_eq!(string("CFBundleIconFile"), "YoluPainter");
        assert_eq!(string("LSMinimumSystemVersion"), MIN_MACOS);
        assert_eq!(
            string("NSHumanReadableCopyright"),
            "Copyright (c) 2026 Yozolab"
        );
        assert_eq!(
            value_of(&plist, "NSHighResolutionCapable").unwrap(),
            "<true/>"
        );
        // 同じ鍵を 2 度書かない
        let keys: Vec<&str> = plist
            .lines()
            .filter_map(|l| l.trim().strip_prefix("<key>")?.strip_suffix("</key>"))
            .collect();
        let mut unique = keys.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), keys.len(), "{keys:?}");
        // アプリが受けていない関連付けは載せない
        assert!(
            !plist.contains("CFBundleDocumentTypes")
                && !plist.contains("UTExportedTypeDeclarations")
        );
    }

    #[test]
    fn the_shown_version_is_three_integers_and_the_build_number_tells_betas_apart() {
        // 表示の版は x.y.z の整数だけ（試験版の識別子を入れない）。ビルド番号は、正式版は x.y.z、試験版は識別子の番号を足した x.y.z.N
        for (text, shown, number) in [
            ("0.6.0", "0.6.0", "0.6.0"),
            ("0.6.1-rc.2", "0.6.1", "0.6.1.2"),
            ("1.2.3-beta.10", "1.2.3", "1.2.3.10"),
            ("1.2.3-alpha.1", "1.2.3", "1.2.3.1"),
            // 番号の無い識別子（配布の事前確認が断る形）でも、整数の並びを崩さない
            ("1.2.3-preview", "1.2.3", "1.2.3.0"),
        ] {
            let version = Version::parse(text).unwrap();
            let plist = info_plist(&version);
            assert_eq!(
                value_of(&plist, "CFBundleShortVersionString").unwrap(),
                format!("<string>{shown}</string>"),
                "{text}"
            );
            assert_eq!(
                value_of(&plist, "CFBundleVersion").unwrap(),
                format!("<string>{number}</string>"),
                "{text}"
            );
            // どちらも、整数だけを点でつないだ形
            for value in [shown, number] {
                assert!(
                    value.split('.').all(|part| part.parse::<u64>().is_ok()),
                    "{value}"
                );
            }
        }
        // 同じ x.y.z の試験版どうしと正式版は、ビルド番号が違う
        let number = |text: &str| build_number(&Version::parse(text).unwrap());
        assert_ne!(number("0.6.1-rc.1"), number("0.6.1-rc.2"));
        assert_ne!(number("0.6.1-rc.2"), number("0.6.1"));
    }

    #[test]
    fn info_plist_is_well_formed_and_builds_for_the_workspace_version() {
        // 実際のワークスペースの版でも作れる（xtask 自身の版 = ワークスペースの版）
        let actual = Version::parse(env!("CARGO_PKG_VERSION")).unwrap();
        assert!(
            info_plist(&actual).contains(&format!("<string>{}</string>", short_version(&actual)))
        );
        // 整った XML（タグの開閉が合い、宣言と plist の根がある）
        let plist = info_plist(&version());
        assert!(plist.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist"));
        assert!(
            plist.contains("<plist version=\"1.0\">\n<dict>\n")
                && plist.ends_with("</dict>\n</plist>\n")
        );
        assert_eq!(
            plist.matches("<key>").count(),
            plist.matches("</key>").count()
        );
        assert_eq!(
            plist.matches("<string>").count(),
            plist.matches("</string>").count()
        );
    }

    #[test]
    fn the_build_clears_every_earlier_output_first_so_a_failed_build_leaves_nothing_stale() {
        let dir = root()
            .join("target/xtask-tests")
            .join(format!("macos-outputs-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let outputs = build_outputs(&dir);
        // 2 つのターゲットと universal の、アプリとコマンドライン
        assert_eq!(outputs.len(), 6);
        for triple in [
            "aarch64-apple-darwin",
            "x86_64-apple-darwin",
            "universal-apple-darwin",
        ] {
            for name in ["yolupainter", "yolupainter-cli"] {
                assert!(
                    outputs.contains(&dir.join("target").join(triple).join("release").join(name)),
                    "{triple} {name}"
                );
            }
        }
        for path in &outputs {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "前回の実行ファイル").unwrap();
        }
        // ほかのファイルは消さない
        let keep = dir.join("target/universal-apple-darwin/release/other");
        fs::write(&keep, "x").unwrap();
        clear_outputs(&dir).unwrap();
        assert!(outputs.iter().all(|path| !path.exists()));
        assert!(keep.exists());
        // 無いときも通る（初めてのビルド）
        clear_outputs(&dir).unwrap();
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn xml_escapes_the_characters_that_would_break_the_plist() {
        assert_eq!(xml("a&b<c>d"), "a&amp;b&lt;c&gt;d");
        assert_eq!(xml("0.6.0-rc.1"), "0.6.0-rc.1");
    }

    #[test]
    fn the_archive_holds_the_app_beside_the_same_documents_as_the_other_platforms() {
        let names = archive_names(&version());
        let folder = "yolupainter-0.6.0-macos-universal-experimental";
        assert_eq!(folder_name(&version()), folder);
        assert!(names.iter().all(|n| n.starts_with(&format!("{folder}/"))));
        // 名前が重ならず、Windows の区切りも無い
        let mut sorted = names.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len());
        assert!(names.iter().all(|n| !n.contains('\\')));
        for expected in [
            "YoluPainter.app/Contents/Info.plist",
            "YoluPainter.app/Contents/MacOS/yolupainter",
            "YoluPainter.app/Contents/MacOS/yolupainter-cli",
            "YoluPainter.app/Contents/Resources/YoluPainter.icns",
            "YoluPainter.app/Contents/_CodeSignature/CodeResources",
            "LICENSE",
            "README.md",
            "README.en.md",
            "THIRD_PARTY.md",
            "DEPENDENCIES.md",
            "THIRD_PARTY_LICENSES.txt",
            "docs/GUIDE.md",
            "docs/INSTALL.md",
            "docs/en/INSTALL.md",
            "docs/BUILDING.md",
        ] {
            assert!(
                names.contains(&format!("{folder}/{expected}")),
                "{expected}"
            );
        }
        // 開発の手順は、ほかの配布物と同じく入れない
        for left_out in ["docs/DEVELOPMENT.md", "docs/RELEASING.md"] {
            assert!(!names.iter().any(|n| n.ends_with(left_out)), "{left_out}");
        }
        // 文書と許諾の表記は、Windows・Linux の配布物と同じ一覧（実行ファイルを除く）
        let others: Vec<String> = super::super::payload_names(yolu_update::LINUX_ARCHIVE)
            .into_iter()
            .skip(2)
            .collect();
        let ours: Vec<String> = payload_names()
            .into_iter()
            .skip(app_files().len())
            .collect();
        assert_eq!(ours, others);
    }

    #[test]
    fn documents_come_from_the_repository_and_the_license_bundle_of_the_universal_target() {
        let root = root();
        let license_dir = root
            .join("target/third-party")
            .join(MACOS_ARCHIVE)
            .join("yolu-app");
        let found = documents(&root, &license_dir);
        // `.app` の中の物は含まない（bundle が作る）。文書と許諾の表記は、Windows・Linux の配布物と同じ名前
        assert_eq!(found.len(), ROOT_FILES.len() + BUNDLED_DOCS.len());
        for (name, source) in &found {
            match name.as_str() {
                "DEPENDENCIES.md" => assert_eq!(source, &license_dir.join("THIRD_PARTY.md")),
                "THIRD_PARTY_LICENSES.txt" => {
                    assert_eq!(source, &license_dir.join("THIRD_PARTY_LICENSES.txt"))
                }
                _ => {
                    assert_eq!(source, &root.join(name));
                    assert!(source.is_file(), "{name} が無い");
                }
            }
        }
    }

    #[test]
    fn lipo_joins_the_two_slices_into_one_output() {
        let inputs = vec![
            PathBuf::from("a/yolupainter"),
            PathBuf::from("b/yolupainter"),
        ];
        let args = lipo_args(Path::new("out/yolupainter"), &inputs);
        assert_eq!(
            args,
            [
                "-create",
                "-output",
                "out/yolupainter",
                "a/yolupainter",
                "b/yolupainter"
            ]
            .map(OsString::from)
        );
    }

    #[test]
    fn only_a_file_with_both_architectures_counts_as_universal() {
        check_archs("yolupainter", "x86_64 arm64\n").unwrap();
        check_archs("yolupainter", "arm64 x86_64").unwrap();
        for bad in ["arm64", "x86_64\n", "", "arm64 x86_64 arm64e", "arm64 i386"] {
            let error = check_archs("yolupainter", bad).unwrap_err().to_string();
            assert!(
                error.contains("universal ではありません"),
                "{bad:?}: {error}"
            );
        }
    }

    #[test]
    fn the_iconset_is_the_standard_set_made_from_the_existing_logo() {
        // 標準の 10 枚（16・32・128・256・512 の 1 倍と 2 倍）。2 倍は 1 倍の 2 倍の画素数
        let names: Vec<&str> = ICONSET.iter().map(|(name, _)| *name).collect();
        let mut unique = names.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), 10);
        for base in [16u32, 32, 128, 256, 512] {
            let size = |file: String| ICONSET.iter().find(|(n, _)| *n == file).map(|(_, s)| *s);
            assert_eq!(size(format!("icon_{base}x{base}.png")), Some(base));
            assert_eq!(size(format!("icon_{base}x{base}@2x.png")), Some(base * 2));
        }
        assert_eq!(
            ICONSET.iter().map(|(_, s)| *s).max(),
            Some(ICON_SOURCE_SIZE)
        );
        // 元は、今あるアプリのロゴ（新しく描かない）で、縮めずに最大の枚を作れる大きさ
        let source = root().join(ICON_SOURCE);
        assert_eq!(
            ICON_SOURCE,
            "crates/yolu-app/assets/logo/yolupainter-1024.png"
        );
        check_icon_source(&source).unwrap();
        let args = sips_args(&source, 64, Path::new("out/icon_32x32@2x.png"));
        assert_eq!(args[..3], ["-z", "64", "64"].map(OsString::from));
        assert_eq!(args[4], OsString::from("--out"));
    }

    #[test]
    fn a_logo_that_is_missing_small_or_not_a_png_is_refused() {
        let dir = root()
            .join("target/xtask-tests")
            .join(format!("macos-logo-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let check = |name: &str, bytes: &[u8]| {
            let path = dir.join(name);
            fs::write(&path, bytes).unwrap();
            check_icon_source(&path).map_err(|e| e.to_string())
        };
        // 256 x 256 の PNG（今あるロゴの小さい方）は、縮めた物を引き延ばすことになるので断る
        let small =
            fs::read(root().join("crates/yolu-app/assets/logo/yolupainter-256.png")).unwrap();
        assert!(check("small.png", &small)
            .unwrap_err()
            .contains("256 x 256"));
        assert!(check("text.png", b"not a png at all, just text")
            .unwrap_err()
            .contains("PNG ではありません"));
        assert!(check_icon_source(&dir.join("absent.png")).is_err());
        assert_eq!(png_size(&small), Some((256, 256)));
        assert_eq!(png_size(b"\x89PNG\r\n\x1a\n"), None);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn the_zip_keeps_the_bundle_whole_and_signs_it_ad_hoc() {
        // 束ごと zip にして、リソースフォーク・拡張属性・検疫の印は入れない
        let args = ditto_args(Path::new("work/folder"), Path::new("out.tmp"));
        let text: Vec<String> = args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            text,
            [
                "-c",
                "-k",
                "--keepParent",
                "--norsrc",
                "--noextattr",
                "--noqtn",
                "work/folder",
                "out.tmp"
            ]
        );
        // ad-hoc は証明書の名前に `-`。署名は束の中の実行ファイルにも及ぶ。識別子は付けない
        // （束は Info.plist の識別子、束の中のコマンドラインはファイル名から付き、同じ識別子にならない）
        let sign: Vec<String> = codesign_args(Path::new("work/YoluPainter.app"))
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            sign,
            ["--force", "--deep", "--sign", "-", "work/YoluPainter.app"]
        );
        assert!(!sign.iter().any(|a| a.contains("identifier")));
        assert_eq!(MACOS_TRIPLES.len(), 2);
    }

    #[test]
    fn the_universal_build_is_refused_outside_macos_with_the_tools_it_needs() {
        if cfg!(target_os = "macos") {
            return;
        }
        let error = bundle().unwrap_err().to_string();
        assert!(
            error.contains("macOS の上でだけ") && error.contains("codesign"),
            "{error}"
        );
        let error = build(|_| Command::new("true")).unwrap_err().to_string();
        assert!(error.contains("macOS の上でだけ"), "{error}");
    }
}
