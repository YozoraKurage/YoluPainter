//! 見本の画像と書き出しの試験: 縮めない大きさでは正本の合成と一致し、縮めた画像の大きさ・平均が合い、書き出しは置き換えの確認を取る。

// 画素（4 バイト）を chunks_exact(4) で回すのは読みやすさのため（yolu-core と同じ）
#![allow(clippy::chunks_exact_to_as_chunks)]

mod common;

use common::*;
use serde_json::json;
use yolu_core::{Channel, Document, Rgba8};
use yolu_ops::preview::{decode_png, fit};
use yolu_ops::reply::*;
use yolu_ops::{ErrorCode, FileHost};

fn preview(host: &mut FileHost, args: serde_json::Value) -> PreviewInfo {
    match ok(host, json!({"command": "preview", "args": args})) {
        Reply::Preview(p) => p,
        other => panic!("{other:?}"),
    }
}

fn composite(host: &mut FileHost) -> Vec<u8> {
    host.with_document(None, |d| d.composite(d.bounds()).unwrap())
        .unwrap()
}

#[test]
fn an_unshrunk_preview_is_exactly_the_composite_of_the_document() {
    let fx = Fixture::new("preview-exact");
    fx.project("a.ylp");
    let mut host = fx.host("a.ylp");
    let expected = composite(&mut host);
    // 既定の最大の辺（512）も、文書より大きい最大の辺も、縮めない
    for args in [
        json!({}),
        json!({"max_edge": 2048}),
        json!({"max_edge": 64}),
        json!({"channel": "Color"}),
    ] {
        let p = preview(&mut host, args.clone());
        assert_eq!(
            (p.width, p.height, p.source_width, p.source_height),
            (64, 48, 64, 48),
            "{args}"
        );
        assert_eq!(p.channel, "Color");
        let (w, h, pixels) = decode_png(&p.png.0).unwrap();
        assert_eq!((w, h), (64, 48));
        assert_eq!(
            pixels, expected,
            "{args}: 縮めない見本は合成の画素そのまま（透明の画素の RGB も）"
        );
    }
    // PNG の上の行が先（文書は左下原点）
    let p = preview(&mut host, json!({}));
    let decoder = png::Decoder::new(std::io::Cursor::new(&p.png.0));
    let mut reader = decoder.read_info().unwrap();
    let mut top_down = vec![0; reader.output_buffer_size().unwrap()];
    reader.next_frame(&mut top_down).unwrap();
    let last_row = &expected[(47 * 64 * 4)..];
    assert_eq!(
        &top_down[..64 * 4],
        last_row,
        "PNG の 1 行目は文書の一番上の行"
    );
}

/// 整数倍の縮めを、見本の関数とは別に（不透明度で重み付けた平均）書いて比べる。
fn expected_shrink_4x(rgba: &[u8], w: usize, h: usize) -> Vec<u8> {
    let (ow, oh) = (w / 4, h / 4);
    let mut out = vec![0u8; ow * oh * 4];
    for oy in 0..oh {
        for ox in 0..ow {
            let (mut a, mut c) = (0u32, [0u32; 3]);
            for dy in 0..4 {
                for dx in 0..4 {
                    let i = ((oy * 4 + dy) * w + ox * 4 + dx) * 4;
                    let alpha = u32::from(rgba[i + 3]);
                    a += alpha;
                    for k in 0..3 {
                        c[k] += alpha * u32::from(rgba[i + k]);
                    }
                }
            }
            let o = (oy * ow + ox) * 4;
            for k in 0..3 {
                out[o + k] = (c[k] + a / 2).checked_div(a).unwrap_or(0) as u8;
            }
            out[o + 3] = ((a + 8) / 16) as u8;
        }
    }
    out
}

#[test]
fn a_shrunk_preview_has_the_fitted_size_and_the_box_average_of_the_composite() {
    let fx = Fixture::new("preview-shrink");
    fx.project("a.ylp");
    let mut host = fx.host("a.ylp");
    let full = composite(&mut host);
    let p = preview(&mut host, json!({"max_edge": 16}));
    assert_eq!((p.width, p.height), (16, 12));
    assert_eq!((p.width, p.height), fit(64, 48, 16));
    let (_, _, pixels) = decode_png(&p.png.0).unwrap();
    assert_eq!(pixels, expected_shrink_4x(&full, 64, 48));
    // 割り切れない縮めでも、大きさは長い辺が最大の辺（短い辺は比を保つ）
    for (edge, size) in [(10, (10, 8)), (33, (33, 25)), (1, (1, 1)), (63, (63, 47))] {
        let p = preview(&mut host, json!({"max_edge": edge}));
        assert_eq!((p.width, p.height), size, "max_edge {edge}");
        let (w, h, px) = decode_png(&p.png.0).unwrap();
        assert_eq!((w, h), size);
        assert_eq!(px.len(), (w * h * 4) as usize);
    }
}

#[test]
fn shrinking_a_flat_color_keeps_the_color_exactly_even_when_the_ratio_is_not_whole() {
    let fx = Fixture::new("preview-flat");
    let mut doc = Document::new(37, 23).unwrap();
    doc.add_fill_layer(
        "Flat",
        &[(Channel::Color, Rgba8::new(51, 102, 153, 255))],
        None,
    )
    .unwrap();
    doc.clear_history().unwrap();
    fx.project_with(
        "a.ylp",
        vec![Set::doc(
            "11111111-1111-4111-8111-111111111111",
            "Flat",
            doc,
        )],
    );
    let mut host = fx.host("a.ylp");
    for edge in [1, 5, 7, 11, 36] {
        let p = preview(&mut host, json!({"max_edge": edge}));
        let (w, h, px) = decode_png(&p.png.0).unwrap();
        assert!(w.max(h) == edge.min(37));
        for pixel in px.chunks_exact(4) {
            assert_eq!(pixel, [51, 102, 153, 255], "max_edge {edge}");
        }
        assert_eq!((p.width, p.height), (w, h));
    }
}

#[test]
fn preview_options_are_checked_and_the_preview_does_not_touch_the_document() {
    let fx = Fixture::new("preview-args");
    fx.project("a.ylp");
    let mut host = fx.host("a.ylp");
    let steps = undo_count(&mut host);
    for (args, expected) in [
        (json!({"max_edge": 0}), ErrorCode::InvalidValue),
        (json!({"max_edge": 2049}), ErrorCode::InvalidValue),
        (json!({"channel": "Gloss"}), ErrorCode::NotFound),
        (json!({"set": "Nothing"}), ErrorCode::NotFound),
    ] {
        assert_eq!(
            err(
                &mut host,
                json!({"command": "preview", "args": args.clone()})
            )
            .code,
            expected,
            "{args}"
        );
    }
    // 使っていないチャンネルは、既定の値（Normal は平ら）
    let p = preview(&mut host, json!({"channel": "Normal", "max_edge": 8}));
    assert_eq!(p.channel, "Normal");
    let (_, _, px) = decode_png(&p.png.0).unwrap();
    assert!(
        px.chunks_exact(4).all(|c| c == [128, 128, 255, 255]),
        "{:?}",
        &px[..4]
    );
    assert_eq!(undo_count(&mut host), steps);
    assert!(!host.has_unsaved_changes(), "見本は文書を変えない");
}

#[test]
fn the_preview_follows_unsaved_edits_and_lists_effects_that_do_not_show() {
    let fx = Fixture::new("preview-edits");
    fx.project("a.ylp");
    let mut host = fx.host("a.ylp");
    let before = preview(&mut host, json!({"max_edge": 2048}));
    assert!(before.inactive_effects.is_empty());
    ok(
        &mut host,
        json!({"command": "layer.set", "args": {"layer": "Base", "opacity": 0.25}}),
    );
    ok(
        &mut host,
        json!({"command": "effect.add", "args": {"layer": "Base", "kind": "blur", "values": {"radius": 2}}}),
    );
    let after = preview(&mut host, json!({"max_edge": 2048}));
    assert_ne!(before.png, after.png);
    let (_, _, px) = decode_png(&after.png.0).unwrap();
    assert_eq!(
        px,
        composite(&mut host),
        "見本は、まだ保存していない編集も含む正本の合成"
    );
    // マップ・モデルの無い Generator は入力のまま通す: 見本は変わらず、知らせに出る
    ok(
        &mut host,
        json!({"command": "effect.add", "args": {"layer": "Base", "kind": "edge_wear"}}),
    );
    let with_generator = preview(&mut host, json!({"max_edge": 2048}));
    assert_eq!(with_generator.png, after.png);
    assert_eq!(with_generator.inactive_effects.len(), 1);
    let note = &with_generator.inactive_effects[0];
    assert!(
        note.ja.contains("Base") && note.en.contains("Base") && note.en.contains("edge wear"),
        "{note:?}"
    );
}

// ───────── 書き出し ─────────

fn exported(reply: Reply) -> Exported {
    match reply {
        Reply::Exported(e) => e,
        other => panic!("{other:?}"),
    }
}

fn file_names(dir: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

#[test]
fn export_channels_writes_the_composite_of_each_used_channel_and_asks_before_replacing() {
    let fx = Fixture::new("export-channels");
    fx.project("a.ylp");
    let mut host = fx.host("a.ylp");
    let expected = composite(&mut host);
    let e = exported(ok(
        &mut host,
        json!({"command": "export.channels", "args": {"dir": "out"}}),
    ));
    assert_eq!(
        file_names(&fx.path("out")),
        ["a_Color.png"],
        "使っているチャンネルだけ。一時ファイルは残らない"
    );
    assert_eq!(e.files.len(), 1);
    assert!(!e.files[0].replaced);
    assert_eq!((e.files[0].width, e.files[0].height), (64, 48));
    let (_, _, px) = decode_png(&read(&fx.path("out/a_Color.png"))).unwrap();
    assert_eq!(px, expected, "書き出した PNG は正本の合成と一致する");
    // 名前の指定・チャンネルの指定（使っていないチャンネルも）
    let e = exported(ok(
        &mut host,
        json!({"command": "export.channels", "args": {"dir": "out2", "name": "tex", "channels": ["Color", "roughness", "4"]}}),
    ));
    assert_eq!(e.files.len(), 3);
    assert_eq!(
        file_names(&fx.path("out2")),
        ["tex_Color.png", "tex_Normal.png", "tex_Roughness.png"]
    );
    // もうあるファイルは、確認なしでは置き換えない（何も書かない）
    let original = read(&fx.path("out/a_Color.png"));
    ok(
        &mut host,
        json!({"command": "layer.set", "args": {"layer": "Base", "opacity": 0.1}}),
    );
    let e = err(
        &mut host,
        json!({"command": "export.channels", "args": {"dir": "out"}}),
    );
    assert_eq!(e.code, ErrorCode::ConfirmRequired);
    assert!(e.data.unwrap()["files"][0]
        .as_str()
        .unwrap()
        .ends_with("a_Color.png"));
    assert_eq!(read(&fx.path("out/a_Color.png")), original);
    assert_eq!(file_names(&fx.path("out")), ["a_Color.png"]);
    let e = exported(ok(
        &mut host,
        json!({"command": "export.channels", "args": {"dir": "out", "confirm": true}}),
    ));
    assert!(e.files[0].replaced);
    assert_ne!(read(&fx.path("out/a_Color.png")), original);
    // 知らないチャンネル・読むだけでない名前の不正
    assert_eq!(
        err(
            &mut host,
            json!({"command": "export.channels", "args": {"dir": "o", "channels": ["Gloss"]}})
        )
        .code,
        ErrorCode::NotFound
    );
    assert!(!fx.path("o").exists());
}

#[test]
fn export_channels_puts_the_set_name_in_the_file_name_when_there_are_several_sets() {
    let fx = Fixture::new("export-sets");
    fx.project_with(
        "proj.ylp",
        vec![
            Set::doc(
                "11111111-1111-4111-8111-111111111111",
                "Body",
                sample_document(),
            ),
            Set::doc(
                "22222222-2222-4222-8222-222222222222",
                "Face",
                sample_document(),
            ),
        ],
    );
    let mut host = fx.host("proj.ylp");
    ok(
        &mut host,
        json!({"command": "export.channels", "args": {"dir": "out", "set": "Body"}}),
    );
    ok(
        &mut host,
        json!({"command": "export.channels", "args": {"dir": "out", "set": "Face"}}),
    );
    assert_eq!(
        file_names(&fx.path("out")),
        ["proj_Body_Color.png", "proj_Face_Color.png"]
    );
}

#[test]
fn export_textures_writes_the_template_images_that_have_something_to_show() {
    let fx = Fixture::new("export-textures");
    fx.project("a.ylp");
    let mut host = fx.host("a.ylp");
    let expected = composite(&mut host);
    let e = exported(ok(
        &mut host,
        json!({"command": "export.textures", "args": {"dir": "tex"}}),
    ));
    assert_eq!(
        file_names(&fx.path("tex")),
        ["a_Albedo.png"],
        "Color だけなら Albedo だけ"
    );
    assert!(
        e.skipped.contains(&"Normal".to_string())
            && e.skipped.contains(&"MetallicSmoothness".to_string()),
        "{:?}",
        e.skipped
    );
    let (_, _, px) = decode_png(&read(&fx.path("tex/a_Albedo.png"))).unwrap();
    assert_eq!(px, expected);
    // Roughness を使うと、MetallicSmoothness が出る
    ok(
        &mut host,
        json!({"command": "layer.add", "args": {"kind": "fill", "name": "Rough", "fill": {"Roughness": "#404040"}}}),
    );
    let e = exported(ok(
        &mut host,
        json!({"command": "export.textures", "args": {"dir": "tex", "confirm": true}}),
    ));
    assert_eq!(
        file_names(&fx.path("tex")),
        ["a_Albedo.png", "a_MetallicSmoothness.png"]
    );
    assert!(e.files.iter().any(|f| f.replaced), "Albedo は置き換えた");
    // テンプレートの指定と、知らないテンプレート
    ok(
        &mut host,
        json!({"command": "export.textures", "args": {"dir": "hd", "template": "unity-hdrp"}}),
    );
    assert!(file_names(&fx.path("hd")).contains(&"a_BaseColor.png".to_string()));
    let e = err(
        &mut host,
        json!({"command": "export.textures", "args": {"dir": "x", "template": "nope"}}),
    );
    assert_eq!(e.code, ErrorCode::NotFound);
    assert_eq!(e.data.unwrap()["templates"].as_array().unwrap().len(), 3);
    // 置き換えは確認がある
    let e = err(
        &mut host,
        json!({"command": "export.textures", "args": {"dir": "tex"}}),
    );
    assert_eq!(e.code, ErrorCode::ConfirmRequired);
}

#[test]
fn export_psd_writes_a_psd_that_reads_back_and_asks_before_replacing() {
    let fx = Fixture::new("export-psd");
    fx.project("a.ylp");
    let mut host = fx.host("a.ylp");
    let e = exported(ok(
        &mut host,
        json!({"command": "export.psd", "args": {"path": "out/a.psd"}}),
    ));
    assert_eq!(
        file_names(&fx.path("out")),
        ["a.psd"],
        "一時ファイルは残らない"
    );
    assert!(!e.files[0].replaced);
    let bytes = read(&fx.path("out/a.psd"));
    let read_back = yolu_io::psd::read(&bytes, &yolu_io::psd::Limits::default()).unwrap();
    let psd = read_back.document().expect("読み戻せる PSD");
    assert_eq!((psd.width, psd.height), (64, 48));
    assert_eq!(
        psd.layers.len(),
        3,
        "一番上の段は Group・Tint・Base（Inner は Group の中）"
    );
    // 平らに 1 枚
    ok(
        &mut host,
        json!({"command": "export.psd", "args": {"path": "out/flat.psd", "mode": "flat"}}),
    );
    let flat = yolu_io::psd::read(
        &read(&fx.path("out/flat.psd")),
        &yolu_io::psd::Limits::default(),
    )
    .unwrap();
    assert_eq!(flat.document().unwrap().layers.len(), 1);
    // 置き換えは確認がある。確認が無ければ何も変えない
    let e = err(
        &mut host,
        json!({"command": "export.psd", "args": {"path": "out/a.psd", "mode": "flat"}}),
    );
    assert_eq!(e.code, ErrorCode::ConfirmRequired);
    assert_eq!(read(&fx.path("out/a.psd")), bytes);
    let e = exported(ok(
        &mut host,
        json!({"command": "export.psd", "args": {"path": "out/a.psd", "mode": "flat", "confirm": true}}),
    ));
    assert!(e.files[0].replaced);
    assert_ne!(read(&fx.path("out/a.psd")), bytes);
    assert_eq!(file_names(&fx.path("out")), ["a.psd", "flat.psd"]);
    // 名前の形・チャンネル
    let e = err(
        &mut host,
        json!({"command": "export.psd", "args": {"path": "out/a.png"}}),
    );
    assert_eq!(e.code, ErrorCode::PathRefused);
    let e = err(
        &mut host,
        json!({"command": "export.psd", "args": {"path": "out/n.psd", "channel": "Gloss"}}),
    );
    assert_eq!(e.code, ErrorCode::NotFound);
    assert_eq!(file_names(&fx.path("out")), ["a.psd", "flat.psd"]);
    // 行き先がフォルダなら断る
    std::fs::create_dir_all(fx.path("out/dir.psd")).unwrap();
    let e = err(
        &mut host,
        json!({"command": "export.psd", "args": {"path": "out/dir.psd", "confirm": true}}),
    );
    assert_eq!(e.code, ErrorCode::PathRefused);
}

#[test]
fn export_psd_tells_what_it_baked_and_what_was_inactive() {
    let fx = Fixture::new("export-psd-notes");
    fx.project("a.ylp");
    let mut host = fx.host("a.ylp");
    ok(
        &mut host,
        json!({"command": "effect.add", "args": {"layer": "Base", "kind": "blur"}}),
    );
    ok(
        &mut host,
        json!({"command": "effect.add", "args": {"layer": "Inner", "kind": "edge_wear"}}),
    );
    let e = exported(ok(
        &mut host,
        json!({"command": "export.psd", "args": {"path": "n.psd"}}),
    ));
    assert!(
        e.notes
            .iter()
            .any(|n| n.en.contains("Base") && n.en.contains("baked")),
        "{:?}",
        e.notes
    );
    assert!(
        e.notes.iter().any(|n| n.en.contains("Inner")
            && n.en.contains("edge wear")
            && n.en.contains("passes its input through")),
        "{:?}",
        e.notes
    );
}

#[test]
fn procedural_generators_work_without_maps_and_the_others_are_reported_as_not_showing() {
    let fx = Fixture::new("procedural");
    fx.project("a.ylp");
    let mut host = fx.host("a.ylp");
    let plain = preview(&mut host, json!({"max_edge": 2048}));
    // ノイズ・グランジは、位置のマップが無ければ UV で評価する（入力のまま通さない）ので、見本に出て、知らせには出ない
    ok(
        &mut host,
        json!({"command": "layer.add", "args": {"kind": "fill", "name": "Rough", "fill": {"Roughness": "#ffffff"}}}),
    );
    let rough = layer_id(&mut host, "Rough");
    ok(
        &mut host,
        json!({"command": "effect.add", "args": {"layer": rough, "kind": "grunge", "channels": ["Roughness"]}}),
    );
    let Reply::Set(info) = ok(&mut host, json!({"command": "set.info"})) else {
        panic!()
    };
    assert!(
        info.inactive_effects.is_empty(),
        "{:?}",
        info.inactive_effects
    );
    let grunge = preview(&mut host, json!({"channel": "Roughness", "max_edge": 2048}));
    let flat = {
        let mut other = fx.host("a.ylp");
        ok(
            &mut other,
            json!({"command": "layer.add", "args": {"kind": "fill", "name": "Rough", "fill": {"Roughness": "#ffffff"}}}),
        );
        preview(
            &mut other,
            json!({"channel": "Roughness", "max_edge": 2048}),
        )
    };
    assert_ne!(grunge.png, flat.png, "グランジが見本に出ている");
    assert_eq!(
        preview(&mut host, json!({"max_edge": 2048})).png,
        plain.png,
        "Color には効かない"
    );
    // マップを読む Generator は、入力が無いので知らせに出る
    ok(
        &mut host,
        json!({"command": "effect.add", "args": {"layer": rough, "kind": "dirt", "channels": ["Roughness"]}}),
    );
    let Reply::Set(info) = ok(&mut host, json!({"command": "set.info"})) else {
        panic!()
    };
    assert_eq!(info.inactive_effects.len(), 1);
    let Reply::Kinds(kinds) = ok(&mut host, json!({"command": "effect.list_kinds"})) else {
        panic!()
    };
    let needs = |id: &str| {
        kinds
            .kinds
            .iter()
            .find(|k| k.id == id)
            .unwrap()
            .needs_baked_maps
    };
    assert!(
        needs("dirt")
            && needs("edge_wear")
            && !needs("grunge")
            && !needs("procedural_noise")
            && !needs("blur")
    );
}

#[test]
fn a_file_name_stem_cannot_carry_a_path_out_of_the_export_folder() {
    let fx = Fixture::new("export-name");
    fx.project("a.ylp");
    let mut host = fx.host("a.ylp");
    ok(
        &mut host,
        json!({"command": "export.channels", "args": {"dir": "out", "name": "../evil/x"}}),
    );
    let names = file_names(&fx.path("out"));
    assert_eq!(names.len(), 1, "{names:?}");
    assert!(
        names[0].ends_with("_Color.png") && !names[0].contains('/') && !names[0].contains('\\'),
        "{names:?}"
    );
    assert!(!fx.dir.join("evil").exists() && !fx.dir.parent().unwrap().join("evil").exists());
    // 空になる名前は断る
    let e = err(
        &mut host,
        json!({"command": "export.channels", "args": {"dir": "out2", "name": "  "}}),
    );
    assert_eq!(e.code, ErrorCode::InvalidValue);
    assert!(!fx.path("out2").exists());
}
