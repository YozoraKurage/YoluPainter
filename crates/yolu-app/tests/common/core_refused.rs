//! core の文書にできない正本（開くと読むだけのセットになる中身）の見本。
use std::path::Path;

use yolu_io::{NativeDocument, NativeValue};

/// 反転の調整の層が使わない値（gamma）を既定から変えた正本。C# の読み手は種類が使わない値を黙って既定に戻すので、core へ渡すと
/// 保存で値が変わる。そのため `core_issues` が挙げて core は断り、アプリはそのセットを読むだけで開く（元のバイト列のまま保存する）。
pub fn native_core_cannot_hold() -> NativeDocument {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../yolu-io/tests/fixtures/m2-channels.utpaint");
    let native = NativeDocument::read(&std::fs::read(path).unwrap()).unwrap();
    let invert = (0..native.layer_count())
        .find(|i| {
            native.field(&format!("layers[{i}].name"))
                == Some(&NativeValue::Text("反転（チャンネルなし）".into()))
        })
        .expect("反転の調整の層");
    let refused = native
        .with_value(
            &format!("layers[{invert}].adjustment.gamma"),
            NativeValue::Float(2.0),
        )
        .unwrap();
    assert!(!refused.core_issues().is_empty());
    refused
}
