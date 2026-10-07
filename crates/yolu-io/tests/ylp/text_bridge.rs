//! テキストレイヤーの保存・復元（正本の版 30）。テキストレイヤーのある文書だけが版 30 になり、無い文書は前の版のまま。往復・版の選び方・読み手の
//! 拒否（範囲の外の値・版 25 の並びの文字の印・意味の無い版）・.ylp への保存を試す。フォントのファイルは正本に入らない。
use yolu_core::text::{TextAlign, TextFont, TextSettings};
use yolu_core::{Channel, Document, LayerId, Rgba8};
use yolu_io::{
    NativeDocument, NativeValue, Project, SaveTarget, SetSpec, WriterInfo, MIXING_VERSION,
    TEXT_VERSION, UNITY_NATIVE_VERSION,
};

const FONT: &[u8] = include_bytes!("../../../yolu-app/assets/fonts/BIZUDPGothic-Regular.ttf");

fn writer() -> WriterInfo {
    WriterInfo {
        app: "YoluPainter-rs".into(),
        version: "0.0.1".into(),
        unity: "none".into(),
    }
}

fn settings(font: TextFont) -> TextSettings {
    let mut t = TextSettings::new("テキストレイヤー\nText", font, 4.0, 60.0);
    t.size = 18.0;
    t.color = Rgba8::new(20, 120, 220, 200);
    t.line_height = 1.4;
    t.letter_spacing = 0.05;
    t.align = TextAlign::Center;
    t.rotation = -12.5;
    t.wrap_width = 0.0;
    t
}

fn with_text(font: TextFont) -> (Document, LayerId) {
    let mut doc = Document::with_tile_size(96, 64, 32).unwrap();
    let base = doc.add_layer("下").unwrap();
    doc.set_pixel(base, 2, 2, Rgba8::new(9, 8, 7, 255)).unwrap();
    let id = doc
        .add_text_layer("文字", settings(font), FONT, None, false)
        .unwrap();
    (doc, id)
}

fn bundled() -> TextFont {
    TextFont::Bundled("biz-udpgothic".into())
}

fn file_font() -> TextFont {
    TextFont::File {
        path: "C:/Fonts/Example Sans.ttf".into(),
        index: 0,
        sha256: std::array::from_fn(|i| (i * 7) as u8),
        names: yolu_core::text::FontNames {
            family: "Example Sans".into(),
            postscript: "ExampleSans-BoldItalic".into(),
            weight: 700,
            italic: true,
        },
    }
}

fn color(doc: &Document, id: LayerId) -> Vec<u8> {
    doc.layer(id)
        .unwrap()
        .surface(Channel::Color)
        .unwrap()
        .canvas_bytes()
        .unwrap()
}

#[test]
fn the_text_value_and_pixels_round_trip_at_version_30() {
    assert_eq!(TEXT_VERSION, 30);
    for font in [bundled(), file_font()] {
        let (doc, id) = with_text(font);
        let native = NativeDocument::from_core(&doc).unwrap();
        assert_eq!(native.version(), TEXT_VERSION);
        let back = native.to_core().unwrap();
        let layer = back.layer(id).unwrap();
        assert_eq!(layer.text(), doc.layer(id).unwrap().text());
        assert_eq!(color(&back, id), color(&doc, id));
        // 読み込みは履歴に残らず、もう一度書くと同じバイト列
        assert_eq!(back.undo_count(), 0);
        assert_eq!(
            NativeDocument::from_core(&back).unwrap().to_bytes(),
            native.to_bytes()
        );
    }
}

#[test]
fn only_documents_with_text_use_version_30() {
    let (mut doc, id) = with_text(bundled());
    assert_eq!(
        NativeDocument::from_core(&doc).unwrap().version(),
        TEXT_VERSION
    );
    // 値を外すと（ラスタライズ）、画素はそのまま版 21 に戻る
    doc.rasterize(id).unwrap();
    let native = NativeDocument::from_core(&doc).unwrap();
    assert_eq!(native.version(), UNITY_NATIVE_VERSION);
    assert_eq!(color(&native.to_core().unwrap(), id), color(&doc, id));
}

#[test]
fn readers_refuse_out_of_range_values_and_versions() {
    let (doc, _) = with_text(bundled());
    let native = NativeDocument::from_core(&doc).unwrap();
    let field = |name: &str| format!("layers[1].text.{name}");
    for (name, value) in [
        ("size", NativeValue::Float(0.0)),
        ("line_height", NativeValue::Float(20.0)),
        ("rotation", NativeValue::Float(400.0)),
        ("align", NativeValue::Int(3)),
        ("algorithm", NativeValue::Int(2)),
        ("font_name", NativeValue::Text("Bad Name".into())),
        ("content", NativeValue::Text("a\u{7}b".into())),
    ] {
        let changed = native.with_value(&field(name), value);
        assert!(
            changed.is_err() || NativeDocument::read(&changed.unwrap().to_bytes()).is_err(),
            "{name}"
        );
    }
    let bytes = native.to_bytes();
    // 版の数だけを 25 にすると、続きの属性の印（属性のビット 7）が版 25 に無いので読めない（0.4.x の読み手は版の数で断る）
    let mut old = bytes.clone();
    old[8..12].copy_from_slice(&MIXING_VERSION.to_le_bytes());
    assert!(NativeDocument::read(&old).is_err());
    // 意味の決まっていない版は読まない
    for v in [27i32, 28, 29, 31] {
        let mut b = bytes.clone();
        b[8..12].copy_from_slice(&v.to_le_bytes());
        assert!(NativeDocument::read(&b).is_err(), "{v}");
    }
}

#[test]
fn a_ylp_keeps_the_text_and_the_version() {
    let (doc, id) = with_text(file_font());
    let native = NativeDocument::from_core(&doc).unwrap();
    let spec = SetSpec {
        id: "0f0f0f0f-0000-4000-8000-000000000030".into(),
        name: "Set".into(),
        material: yolu_io::MaterialRef::Unassigned,
        document: Some(native.clone().into()),
        composites: vec![],
    };
    let project = Project::create(writer(), std::slice::from_ref(&spec), &spec.id).unwrap();
    let dir = std::env::temp_dir().join(format!("yolu-io-text-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("text.ylp");
    let _ = std::fs::remove_file(&path);
    SaveTarget::create(&path).unwrap().save(&project).unwrap();
    let (again, _) = SaveTarget::open(&path).unwrap();
    let reopened = &again.sets()[0].document;
    assert_eq!(reopened.version(), TEXT_VERSION);
    assert_eq!(reopened.to_bytes().unwrap(), native.to_bytes());
    let core = reopened.to_core().unwrap();
    assert_eq!(
        core.layer(id).unwrap().text(),
        doc.layer(id).unwrap().text()
    );
    // フォントのファイルの中身は正本に入らない（道と SHA-256 だけ）
    assert!(!native
        .to_bytes()
        .windows(64)
        .any(|w| w == &FONT[4096..4160]));
    std::fs::remove_dir_all(&dir).unwrap();
}
