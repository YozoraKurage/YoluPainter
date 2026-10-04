//! Windows の実行ファイルに、アイコンと製品名・版のメタデータ（バージョン情報）を埋める。Windows 以外の対象では何もしない。
//! 製品名と版は、コード署名（SignPath Foundation の条件）が署名する実行ファイルのすべてに求める情報で、インストーラー
//! （installer/yolupainter.nsi）も同じ値を持つ。版はワークスペースの版（`CARGO_PKG_VERSION`。プレリリースの識別子つき）。
//! ホストが Linux でも、対象が Windows なら埋める（winresource が windres を探す）。

use std::env;

/// exe のアイコン（ロゴ。インストーラーも同じファイルを使う）。
const ICON: &str = "assets/logo/yolupainter.ico";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={ICON}");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let version = env::var("CARGO_PKG_VERSION").expect("cargo が版を渡す");
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
        panic!("Windows のリソース（アイコン・製品名・版）を埋められません: {error}");
    }
}
