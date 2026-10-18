//! Windows の実行ファイルに、アイコンと製品名・版のメタデータ（バージョン情報）を埋める。git の短い ID は全 OS の診断用に埋める。
//! 製品名と版は、コード署名（SignPath Foundation の条件）が署名する実行ファイルのすべてに求める情報で、インストーラー
//! （installer/yolupainter.nsi）も同じ値を持つ。版はワークスペースの版（`CARGO_PKG_VERSION`。プレリリースの識別子つき）。
//! ホストが Linux でも、対象が Windows なら埋める（winresource が windres を探す）。画面の文の .ftl も埋める（`embed_locales`）。

use std::{env, path::Path};

/// exe のアイコン（ロゴ。インストーラーも同じファイルを使う）。
const ICON: &str = "assets/logo/yolupainter.ico";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={ICON}");
    // 配る物の組み（.github/workflows/dist-build.yml）は、PR の CI だと merge の commit（どの履歴にも入らない）を checkout するので、
    // 診断用の ID をワークフローが環境変数で渡す（PR の先頭の commit。木は配る物と同じ）。渡されなければ HEAD。
    println!("cargo:rerun-if-env-changed=YOLU_GIT_REV");
    let revision = env::var("YOLU_GIT_REV")
        .ok()
        .filter(|id| is_revision(id))
        .or_else(|| git(&["rev-parse", "--short", "HEAD"]));
    if let Some(revision) = revision {
        println!("cargo:rustc-env=YOLU_GIT_REV={revision}");
    }
    // 存在しない経路を rerun-if-changed に渡すと、cargo は毎回「変わった」と見て組み直す。参照が packed-refs にだけある
    // （git gc・pack-refs のあと）ときは緩いファイルが無いので、存在するものだけを渡す。
    // HEAD（ブランチの切り替え）・参照（コミット）・packed-refs（まとめた参照）・HEAD の履歴（コミットのたびに追記される）。
    let branch = git(&["rev-parse", "--symbolic-full-name", "HEAD"]);
    for name in ["HEAD", "packed-refs", "logs/HEAD"]
        .into_iter()
        .chain(branch.as_deref())
    {
        if let Some(path) = git(&["rev-parse", "--git-path", name]) {
            if Path::new(&path).exists() {
                println!("cargo:rerun-if-changed={path}");
            }
        }
    }
    embed_locales();
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let version = env::var("CARGO_PKG_VERSION").expect("cargo がバージョンを渡す");
    let mut resource = winresource::WindowsResource::new();
    resource
        .set_icon(ICON)
        .set("ProductName", "YoluPainter")
        .set("FileDescription", "YoluPainter")
        .set("InternalName", "yolupainter")
        .set("OriginalFilename", "yolupainter.exe")
        .set("CompanyName", "Yozolab")
        .set("LegalCopyright", "Copyright (c) 2026 Yozolab")
        .set("ProductVersion", &version)
        .set("FileVersion", &version);
    if let Err(error) = resource.compile() {
        panic!("Windows のリソース（アイコン・製品名・バージョン）を埋められません: {error}");
    }
}

/// 画面の文（`locales/<言語>/<モジュール>.ftl`）を `include_str!` で埋める一覧を `OUT_DIR/locales.rs` に書く（言語ごとに
/// `(モジュール, 文)` の並び。ファイルの名前の順）。フォルダが無ければ空の一覧（存在しないパスは rerun-if-changed に渡さない）。
fn embed_locales() {
    let root = Path::new(&env::var("CARGO_MANIFEST_DIR").expect("cargo が渡す")).join("locales");
    if root.is_dir() {
        println!("cargo:rerun-if-changed={}", root.display());
    }
    let mut out = String::new();
    for (name, dir) in [("JA", "ja"), ("EN", "en")] {
        let mut files: Vec<_> = std::fs::read_dir(root.join(dir))
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|e| e == "ftl"))
            .collect();
        files.sort();
        out.push_str(&format!("pub(super) const {name}: &[(&str, &str)] = &[\n"));
        for path in files {
            let module = path.file_stem().unwrap().to_string_lossy().into_owned();
            let path = path.to_string_lossy().into_owned();
            out.push_str(&format!("    ({module:?}, include_str!({path:?})),\n"));
        }
        out.push_str("];\n");
    }
    let target = Path::new(&env::var("OUT_DIR").expect("cargo が渡す")).join("locales.rs");
    std::fs::write(&target, out).expect("OUT_DIR に書ける");
}

/// 環境変数で渡された ID として受け取れるか（16 進の 7〜40 文字。`cargo:` の行へそのまま書くので、それ以外は使わない）。
fn is_revision(id: &str) -> bool {
    (7..=40).contains(&id.len()) && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn git(args: &[&str]) -> Option<String> {
    let output = std::process::Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    (!text.is_empty()).then_some(text)
}
