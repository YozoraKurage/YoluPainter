//! 定規（正本の版 35、レイヤーの続きの属性のビット 2 と `ruler_count`・`rulers[i]`）と、パスの線対称（`canvas_symmetry` の `mode` 5 と `angle`）。
//! 定規もパスの線対称も無い文書は前の版のまま。往復・版の選び方・古い読み手の範囲・範囲の外の値と重なった ID の拒否・.ylp への保存を試す。
use yolu_core::glam::{DVec2, DVec3};
use yolu_core::paths::{
    CanvasPath, CanvasPoint, LayerPathEntry, PathBrush, PathStyle, PathSymmetry,
};
use yolu_core::{
    BrushSettings, CanvasSymmetry, Channel, Document, LayerId, LayerPath, Rgba8, Ruler, RulerId,
    RulerKind, RulerPlace, RulerScope, SymmetryMode,
};
use yolu_io::{
    MaterialRef, NativeDocument, NativeValue, Project, SaveTarget, SetSpec, WriterInfo,
    ANTI_ALIAS_VERSION, BAKE_PRIORITY_VERSION, MAX_NATIVE_VERSION, RULERS_VERSION,
    UNITY_NATIVE_VERSION,
};

fn writer() -> WriterInfo {
    WriterInfo {
        app: "YoluPainter-rs".into(),
        version: "0.0.1".into(),
        unity: "none".into(),
    }
}

/// 下から: 背景（ラスター）、グループ g の中に a・b、上にもう 1 枚。
struct World {
    doc: Document,
    base: LayerId,
    group: LayerId,
    a: LayerId,
    top: LayerId,
}

fn world() -> World {
    let mut doc = Document::with_tile_size(64, 64, 32).unwrap();
    let base = doc.add_layer("背景").unwrap();
    let a = doc.add_layer("a").unwrap();
    let b = doc.add_layer("b").unwrap();
    let group = doc.group_layers(&[a, b], "グループ").unwrap();
    let top = doc.add_layer("上").unwrap();
    doc.clear_history().unwrap();
    World {
        doc,
        base,
        group,
        a,
        top,
    }
}

fn ruler_2d(d: &mut Document, kind: RulerKind, a: (f64, f64), b: (f64, f64)) -> Ruler {
    Ruler::canvas(
        d.new_ruler_id(),
        kind,
        DVec2::new(a.0, a.1),
        DVec2::new(b.0, b.1),
    )
}

fn ruler_3d(d: &mut Document, kind: RulerKind) -> Ruler {
    Ruler::model(
        d.new_ruler_id(),
        kind,
        DVec3::new(0.25, -1.5, 3.0),
        DVec3::new(1.25, -1.5, 5.0),
        DVec3::new(0.0, 7.0, 0.0),
    )
}

/// 種類・空間・印・範囲をひと通り使った定規の組（3 つのレイヤーに分けて付ける）。
fn assorted(w: &mut World) {
    let d = &mut w.doc;
    let mut line = ruler_2d(d, RulerKind::Line, (3.0, 4.0), (60.5, 40.25));
    line.scope = RulerScope::All;
    let mut parallel = ruler_2d(d, RulerKind::Parallel, (10.0, 10.0), (20.0, 11.0));
    parallel.snap = true;
    parallel.visible = false;
    parallel.scope = RulerScope::Selected;
    let concentric = ruler_2d(d, RulerKind::Concentric, (32.0, 32.0), (40.0, 32.0));
    let mut persp = ruler_2d(d, RulerKind::Perspective, (-100.0, 5.0), (100.5, 5.5));
    persp.two_points = true;
    let mut sym = ruler_2d(d, RulerKind::Symmetry, (32.0, 32.0), (32.0, 50.0));
    sym.lines = 6;
    sym.line_symmetry = true;
    sym.snap = true;
    let mut rot = ruler_2d(d, RulerKind::Symmetry, (1.0, 2.0), (4.0, 2.0));
    rot.lines = 5;
    rot.line_symmetry = false;
    let m_line = ruler_3d(d, RulerKind::Line);
    let mut m_sym = ruler_3d(d, RulerKind::Symmetry);
    m_sym.lines = 4;
    m_sym.line_symmetry = true;
    m_sym.see_through = true;
    m_sym.snap = true;
    let mut m_persp = ruler_3d(d, RulerKind::Perspective);
    m_persp.two_points = true;
    let m_conc = ruler_3d(d, RulerKind::Concentric);
    let m_par = ruler_3d(d, RulerKind::Parallel);
    d.set_rulers(w.base, vec![line, parallel, concentric, persp], false)
        .unwrap();
    d.set_rulers(w.group, vec![sym, rot], false).unwrap();
    d.set_rulers(w.top, vec![m_line, m_sym, m_persp, m_conc, m_par], false)
        .unwrap();
}

fn all_rulers(d: &Document) -> Vec<(LayerId, Vec<Ruler>)> {
    d.layers()
        .iter()
        .map(|l| (l.id(), l.rulers().to_vec()))
        .collect()
}

#[test]
fn documents_without_rulers_keep_their_old_version_and_ones_with_rulers_are_version_35() {
    assert_eq!(RULERS_VERSION, 35);
    assert_eq!(MAX_NATIVE_VERSION, RULERS_VERSION);
    let mut w = world();
    // 定規が無い文書: 版 21、定規の欄は無い
    let plain = NativeDocument::from_core(&w.doc).unwrap();
    assert_eq!(plain.version(), UNITY_NATIVE_VERSION);
    assert!(plain.fields().iter().all(|f| !f.path.contains("ruler")));
    // 1 つでもあれば 35
    assorted(&mut w);
    let native = NativeDocument::from_core(&w.doc).unwrap();
    assert_eq!(native.version(), RULERS_VERSION);
    // 全部消して保存し直すと版も下がる
    let all = all_rulers(&w.doc);
    for (id, list) in &all {
        if !list.is_empty() {
            w.doc.set_rulers(*id, Vec::new(), false).unwrap();
        }
    }
    assert_eq!(
        NativeDocument::from_core(&w.doc).unwrap().to_bytes(),
        plain.to_bytes(),
        "定規を全部消すと、定規が無かったときと同じバイト列"
    );
    // ほかの機能（アンチエイリアスのパス = 34）と一緒なら 35 で、パスも残る
    let mut w2 = world();
    let mut path = path_with(PathSymmetry::None, 1);
    path.brush = PathBrush(BrushSettings {
        anti_alias: yolu_core::AntiAlias::Medium,
        ..path.brush.0
    });
    w2.doc.set_canvas_path(w2.a, path).unwrap();
    assert_eq!(
        NativeDocument::from_core(&w2.doc).unwrap().version(),
        ANTI_ALIAS_VERSION
    );
    assorted(&mut w2);
    let native = NativeDocument::from_core(&w2.doc).unwrap();
    assert_eq!(native.version(), RULERS_VERSION);
    let back = native.to_core().unwrap();
    assert!(back.layer(w2.a).unwrap().path().is_some());
    assert_eq!(all_rulers(&back), all_rulers(&w2.doc));
}

#[test]
fn every_kind_space_flag_and_scope_round_trips_with_the_same_bytes() {
    let mut w = world();
    assorted(&mut w);
    let native = NativeDocument::from_core(&w.doc).unwrap();
    let back = native.to_core().unwrap();
    assert_eq!(all_rulers(&back), all_rulers(&w.doc), "値が同じ");
    assert_eq!(back.undo_count(), 0, "読み込みは履歴に残らない");
    assert_eq!(
        NativeDocument::from_core(&back).unwrap().to_bytes(),
        native.to_bytes(),
        "もう一度書くと同じバイト列"
    );
    // バイト列から読み直しても同じ
    let reread = NativeDocument::read(&native.to_bytes()).unwrap();
    assert_eq!(all_rulers(&reread.to_core().unwrap()), all_rulers(&w.doc));
}

#[test]
fn the_fields_are_laid_out_as_the_spec_says() {
    let mut w = world();
    assorted(&mut w);
    let native = NativeDocument::from_core(&w.doc).unwrap();
    // 背景（layers[0]）: 続きの属性のビット 2、定規 4 つ
    assert_eq!(
        native.field("layers[0].attributes"),
        Some(&NativeValue::Byte(128))
    );
    assert_eq!(
        native.field("layers[0].attributes_ext"),
        Some(&NativeValue::Int(4))
    );
    assert_eq!(
        native.field("layers[0].ruler_count"),
        Some(&NativeValue::Int(4))
    );
    // 1 つ目 = 直線定規・2D・表示だけ（範囲は すべて）
    assert_eq!(
        native.field("layers[0].rulers[0].kind"),
        Some(&NativeValue::Byte(0))
    );
    assert_eq!(
        native.field("layers[0].rulers[0].space"),
        Some(&NativeValue::Byte(0))
    );
    assert_eq!(
        native.field("layers[0].rulers[0].flags"),
        Some(&NativeValue::Byte(1))
    );
    assert_eq!(
        native.field("layers[0].rulers[0].scope"),
        Some(&NativeValue::Byte(0))
    );
    assert_eq!(
        native.field("layers[0].rulers[0].lines"),
        Some(&NativeValue::Byte(0))
    );
    assert_eq!(
        native.field("layers[0].rulers[0].a_x"),
        Some(&NativeValue::Float(3.0))
    );
    assert_eq!(
        native.field("layers[0].rulers[0].b_y"),
        Some(&NativeValue::Float(40.25))
    );
    assert!(
        native.field("layers[0].rulers[0].a_z").is_none(),
        "2D は z を持たない"
    );
    assert!(native.field("layers[0].rulers[0].up_x").is_none());
    // 2 つ目 = 平行線: 表示は偽・スナップの印・選んでいるときだけ → flags 2、範囲 2
    assert_eq!(
        native.field("layers[0].rulers[1].flags"),
        Some(&NativeValue::Byte(2))
    );
    assert_eq!(
        native.field("layers[0].rulers[1].scope"),
        Some(&NativeValue::Byte(2))
    );
    // 4 つ目 = パースの 2 点: 表示 + 2 点 = 5
    assert_eq!(
        native.field("layers[0].rulers[3].flags"),
        Some(&NativeValue::Byte(5))
    );
    // グループ（layers[3]）の対称 6 本線対称・スナップ: 表示 + スナップ + 線対称 = 1 + 2 + 8
    let group = format!("layers[{}]", w.doc.layer_index(w.group).unwrap());
    assert_eq!(
        native.field(&format!("{group}.rulers[0].flags")),
        Some(&NativeValue::Byte(11))
    );
    assert_eq!(
        native.field(&format!("{group}.rulers[0].lines")),
        Some(&NativeValue::Byte(6))
    );
    // 一番上の 3D の対称: 表示 + スナップ + 線対称 + 見えない面 = 1 + 2 + 8 + 16 = 27、z と up を持つ
    let top = format!("layers[{}]", w.doc.layer_index(w.top).unwrap());
    assert_eq!(
        native.field(&format!("{top}.ruler_count")),
        Some(&NativeValue::Int(5))
    );
    assert_eq!(
        native.field(&format!("{top}.rulers[1].space")),
        Some(&NativeValue::Byte(1))
    );
    assert_eq!(
        native.field(&format!("{top}.rulers[1].flags")),
        Some(&NativeValue::Byte(27))
    );
    assert_eq!(
        native.field(&format!("{top}.rulers[1].a_z")),
        Some(&NativeValue::Float(3.0))
    );
    assert_eq!(
        native.field(&format!("{top}.rulers[1].up_y")),
        Some(&NativeValue::Float(1.0))
    );
    // 向きは単位にして持つ（入れたのは (0, 7, 0)）
    assert_eq!(
        native.field(&format!("{top}.rulers[1].up_x")),
        Some(&NativeValue::Float(0.0))
    );
}

#[test]
fn a_reader_that_stops_at_34_refuses_a_rulers_file() {
    let mut w = world();
    assorted(&mut w);
    let native = NativeDocument::from_core(&w.doc).unwrap();
    let mut bytes = native.to_bytes();
    assert_eq!(&bytes[8..12], &RULERS_VERSION.to_le_bytes());
    // 0.5.x の読み手の上限は版 33、0.6.0 の前の版は 34: 版の数だけを下げた正本は、続きの属性の印のビット 2 を知らないので読めない
    for older in [ANTI_ALIAS_VERSION, BAKE_PRIORITY_VERSION] {
        bytes[8..12].copy_from_slice(&older.to_le_bytes());
        let e = NativeDocument::read(&bytes).unwrap_err().to_string();
        assert!(e.contains("未知の続きのレイヤー属性ビット"), "{older}: {e}");
    }
    // もっと古い版は頭の並びも違うので、別の所で読めなくなる（どれも断る）
    for older in [yolu_io::TEXT_VERSION, yolu_io::PATHS_VERSION, 25, 21] {
        bytes[8..12].copy_from_slice(&older.to_le_bytes());
        assert!(NativeDocument::read(&bytes).is_err(), "{older}");
    }
    // 35 の上の版は断る
    for newer in [MAX_NATIVE_VERSION + 1, 36, 40] {
        let mut b = native.to_bytes();
        b[8..12].copy_from_slice(&newer.to_le_bytes());
        let e = NativeDocument::read(&b).unwrap_err().to_string();
        assert!(e.contains("未対応"), "{newer}: {e}");
    }
    // 版 21 の定規の無い正本の版の数を 35 にしても、欄が足りず読めない
    let mut plain = NativeDocument::from_core(&world().doc).unwrap().to_bytes();
    plain[8..12].copy_from_slice(&RULERS_VERSION.to_le_bytes());
    assert!(NativeDocument::read(&plain).is_err());
}

#[test]
fn out_of_range_values_unknown_bits_and_overlapping_ids_are_refused() {
    let mut w = world();
    assorted(&mut w);
    let native = NativeDocument::from_core(&w.doc).unwrap();
    let refused = |name: &str, value: NativeValue| {
        let changed = native.with_value(name, value);
        assert!(
            changed.is_err() || NativeDocument::read(&changed.unwrap().to_bytes()).is_err(),
            "{name} を通した"
        );
    };
    let r = |n: usize, field: &str| format!("layers[0].rulers[{n}].{field}");
    refused(&r(0, "kind"), NativeValue::Byte(5));
    refused(&r(0, "space"), NativeValue::Byte(2));
    refused(&r(0, "space"), NativeValue::Byte(1)); // 2D の定規の欄を 3D として読む（欄が足りない）
    refused(&r(0, "scope"), NativeValue::Byte(3));
    refused(&r(0, "flags"), NativeValue::Byte(32)); // 未知のビット
    refused(&r(0, "flags"), NativeValue::Byte(129));
    refused(&r(0, "flags"), NativeValue::Byte(3)); // 直線定規のスナップの印
    refused(&r(0, "flags"), NativeValue::Byte(5)); // パース以外の 2 点
    refused(&r(0, "flags"), NativeValue::Byte(9)); // 対称以外の線対称
    refused(&r(0, "flags"), NativeValue::Byte(17)); // 2D の「見えない面にも写す」
    refused(&r(0, "lines"), NativeValue::Byte(2)); // 対称以外の線の本数
    refused(&r(0, "a_x"), NativeValue::Float(2e7));
    refused(&r(0, "a_x"), NativeValue::Float(f64::NAN));
    refused(&r(0, "a_y"), NativeValue::Float(f64::INFINITY));
    // a と b が同じ点
    assert!(native
        .with_values(&[
            (r(0, "b_x"), NativeValue::Float(3.0)),
            (r(0, "b_y"), NativeValue::Float(4.0)),
        ])
        .is_err());
    // 対称: 本数の範囲と、線対称の偶数
    let group = w.doc.layer_index(w.group).unwrap();
    let g = |n: usize, field: &str| format!("layers[{group}].rulers[{n}].{field}");
    for lines in [0u8, 1, 17, 255] {
        refused(&g(0, "lines"), NativeValue::Byte(lines));
    }
    refused(&g(0, "lines"), NativeValue::Byte(5)); // 線対称は偶数だけ
                                                   // 3D: 範囲は ±1e6、up は 0 でない、対称の最初の線が軸と平行でない
    let top = w.doc.layer_index(w.top).unwrap();
    let t = |n: usize, field: &str| format!("layers[{top}].rulers[{n}].{field}");
    refused(&t(0, "a_z"), NativeValue::Float(2e6));
    refused(&t(0, "up_x"), NativeValue::Float(2e6));
    refused(&t(0, "up_y"), NativeValue::Float(0.0)); // up が (0, 0, 0)
    refused(&t(1, "up_y"), NativeValue::Float(0.0));
    // 通る側: up (1, 1, 0) は単位でなくてもよく（読んで単位にする）、最初の線 (1, 0, 2) と平行でもない
    let ok = native
        .with_value(&t(1, "up_x"), NativeValue::Float(1.0))
        .unwrap();
    assert!(ok.to_core().is_ok());
    // ID が空・重なる（同じレイヤー・別のレイヤー）
    refused(&r(0, "id"), NativeValue::Guid([0; 16]));
    let NativeValue::Guid(first) = native.field(&r(0, "id")).unwrap().clone() else {
        panic!("GUID");
    };
    refused(&r(1, "id"), NativeValue::Guid(first));
    refused(&g(1, "id"), NativeValue::Guid(first));
    // 数: 0 個・65 個の欄
    refused("layers[0].ruler_count", NativeValue::Int(0));
    refused("layers[0].ruler_count", NativeValue::Int(65));
    refused("layers[0].ruler_count", NativeValue::Int(5)); // 欄が足りない
}

#[test]
fn the_first_line_parallel_to_the_axis_is_refused_when_reading() {
    let mut w = world();
    let mut sym = ruler_3d(&mut w.doc, RulerKind::Symmetry);
    sym.place = RulerPlace::Model {
        a: DVec3::ZERO,
        b: DVec3::new(0.0, 2.0, 1.0),
        up: DVec3::Y,
    };
    w.doc.set_rulers(w.top, vec![sym], false).unwrap();
    let native = NativeDocument::from_core(&w.doc).unwrap();
    let top = w.doc.layer_index(w.top).unwrap();
    // b を軸の向きにそろえて平行にすると断る
    let key = format!("layers[{top}].rulers[0].b_z");
    assert!(native.with_value(&key, NativeValue::Float(0.0)).is_err());
}

#[test]
fn the_64th_ruler_is_written_and_read_and_a_group_and_other_kinds_can_hold_rulers() {
    let mut w = world();
    let list: Vec<Ruler> = (0..64)
        .map(|k| {
            ruler_2d(
                &mut w.doc,
                RulerKind::Line,
                (k as f64, 0.0),
                (k as f64, 10.0),
            )
        })
        .collect();
    w.doc.set_rulers(w.group, list, false).unwrap();
    // 塗りつぶし・調整のレイヤーにも付く
    let fill = w
        .doc
        .add_fill_layer("塗り", &[(Channel::Color, Rgba8::new(9, 9, 9, 255))], None)
        .unwrap();
    let adjust = w
        .doc
        .add_adjustment_layer("調整", yolu_core::AdjustmentSettings::invert(), None, None)
        .unwrap();
    let (f, a) = (
        ruler_2d(&mut w.doc, RulerKind::Parallel, (1.0, 1.0), (9.0, 2.0)),
        ruler_3d(&mut w.doc, RulerKind::Perspective),
    );
    w.doc.set_rulers(fill, vec![f], false).unwrap();
    w.doc.set_rulers(adjust, vec![a], false).unwrap();
    let native = NativeDocument::from_core(&w.doc).unwrap();
    assert_eq!(native.version(), RULERS_VERSION);
    let back = native.to_core().unwrap();
    assert_eq!(all_rulers(&back), all_rulers(&w.doc));
    assert_eq!(back.layer(w.group).unwrap().rulers().len(), 64);
    // 65 個目は core が付けさせない
    let extra = ruler_2d(&mut w.doc, RulerKind::Line, (0.0, 0.0), (1.0, 1.0));
    let mut over = back.layer(w.group).unwrap().rulers().to_vec();
    over.push(extra);
    assert!(w.doc.set_rulers(w.group, over, false).is_err());
}

#[test]
fn a_document_resized_past_a_corner_is_still_writable() {
    let mut w = world();
    let corner = ruler_2d(&mut w.doc, RulerKind::Line, (9e6, 9e6), (9.5e6, 8e6));
    w.doc.set_rulers(w.top, vec![corner], false).unwrap();
    w.doc
        .resize_image(128, 128, yolu_core::CanvasResampling::Nearest)
        .unwrap();
    let native = NativeDocument::from_core(&w.doc).unwrap();
    assert_eq!(native.version(), RULERS_VERSION);
    assert_eq!(all_rulers(&native.to_core().unwrap()), all_rulers(&w.doc));
}

#[test]
fn a_ylp_keeps_the_rulers_and_the_version() {
    let mut w = world();
    assorted(&mut w);
    let native = NativeDocument::from_core(&w.doc).unwrap();
    let spec = SetSpec {
        id: "0f0f0f0f-0000-4000-8000-000000000035".into(),
        name: "Set".into(),
        material: MaterialRef::Unassigned,
        document: Some(native.clone().into()),
        composites: vec![],
    };
    let project = Project::create(writer(), std::slice::from_ref(&spec), &spec.id).unwrap();
    let dir = std::env::temp_dir().join(format!("yolu-io-rulers-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("rulers.ylp");
    let _ = std::fs::remove_file(&path);
    SaveTarget::create(&path).unwrap().save(&project).unwrap();
    let (again, _) = SaveTarget::open(&path).unwrap();
    let reopened = &again.sets()[0].document;
    assert_eq!(reopened.version(), RULERS_VERSION);
    assert_eq!(reopened.to_bytes().unwrap(), native.to_bytes());
    let core = reopened.to_core().unwrap();
    assert_eq!(all_rulers(&core), all_rulers(&w.doc));
    std::fs::remove_dir_all(&dir).unwrap();
}

// ───────── パスの線対称（mode 5） ─────────

fn path_with(symmetry: PathSymmetry, id: u128) -> CanvasPath {
    CanvasPath {
        style: PathStyle {
            symmetry,
            ..PathStyle::default()
        },
        id,
        channel: Channel::Color,
        brush: PathBrush(BrushSettings {
            radius: 2.25,
            hardness: 1.0,
            color: Rgba8::new(20, 30, 200, 255),
            ..BrushSettings::default()
        }),
        points: vec![
            CanvasPoint::new(3.5, 4.0, 1.0).unwrap(),
            CanvasPoint::new(27.25, 21.5, 1.0).unwrap(),
        ],
        material: None,
    }
}

fn symmetry_of(doc: &Document, layer: LayerId) -> PathSymmetry {
    match doc.layer(layer).unwrap().path() {
        Some(LayerPath::Canvas(p)) => p.style.symmetry,
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_lines_symmetry_path_is_mode_5_with_an_angle_and_makes_the_file_version_35() {
    let c = DVec2::new(32.0, 32.0);
    for (count, angle) in [(2u32, 45.0), (6, 12.5), (16, -360.0), (4, 360.0), (2, 0.0)] {
        let mut w = world();
        let lines = CanvasSymmetry::lines(c, count, angle).unwrap();
        w.doc
            .set_canvas_path(w.a, path_with(PathSymmetry::Canvas(lines), 7))
            .unwrap();
        let native = NativeDocument::from_core(&w.doc).unwrap();
        assert_eq!(native.version(), RULERS_VERSION, "{count} 本 {angle}°");
        let at = |f: &str| {
            native.field(&format!(
                "layers[1].paths.items[0].extra.canvas_symmetry.{f}"
            ))
        };
        assert_eq!(at("mode"), Some(&NativeValue::Int(5)));
        assert_eq!(at("count"), Some(&NativeValue::Int(count as i32)));
        assert_eq!(at("angle"), Some(&NativeValue::Float(angle)));
        let back = native.to_core().unwrap();
        assert_eq!(
            symmetry_of(&back, w.a),
            PathSymmetry::Canvas(lines),
            "角度もビットのまま戻る"
        );
        assert_eq!(
            NativeDocument::from_core(&back).unwrap().to_bytes(),
            native.to_bytes()
        );
    }
    // 縦・横・両方・放射状は今のまま（版も欄も増えない。放射状に角度は無い）
    for (mode, count, mode_value) in [
        (SymmetryMode::Vertical, 2u32, 1),
        (SymmetryMode::Horizontal, 2, 2),
        (SymmetryMode::Both, 2, 3),
        (SymmetryMode::Radial, 5, 4),
    ] {
        let mut w = world();
        let s = CanvasSymmetry::new(mode, c, count).unwrap();
        w.doc
            .set_canvas_path(w.a, path_with(PathSymmetry::Canvas(s), 7))
            .unwrap();
        let native = NativeDocument::from_core(&w.doc).unwrap();
        assert!(native.version() < RULERS_VERSION, "{mode:?}");
        let at = |f: &str| {
            native.field(&format!(
                "layers[1].paths.items[0].extra.canvas_symmetry.{f}"
            ))
        };
        assert_eq!(at("mode"), Some(&NativeValue::Int(mode_value)));
        assert!(at("angle").is_none(), "{mode:?} は角度の欄を持たない");
        assert_eq!(
            symmetry_of(&native.to_core().unwrap(), w.a),
            PathSymmetry::Canvas(s)
        );
    }
}

#[test]
fn mode_5_is_refused_when_odd_out_of_range_or_in_an_older_version() {
    let c = DVec2::new(32.0, 32.0);
    let mut w = world();
    w.doc
        .set_canvas_path(
            w.a,
            path_with(
                PathSymmetry::Canvas(CanvasSymmetry::lines(c, 4, 30.0).unwrap()),
                7,
            ),
        )
        .unwrap();
    let native = NativeDocument::from_core(&w.doc).unwrap();
    let key = |f: &str| format!("layers[1].paths.items[0].extra.canvas_symmetry.{f}");
    let refused = |name: String, value: NativeValue| {
        let changed = native.with_value(&name, value);
        assert!(
            changed.is_err() || NativeDocument::read(&changed.unwrap().to_bytes()).is_err(),
            "{name} を通した"
        );
    };
    refused(key("count"), NativeValue::Int(3)); // 線対称は偶数だけ
    refused(key("count"), NativeValue::Int(1));
    refused(key("angle"), NativeValue::Float(361.0));
    refused(key("angle"), NativeValue::Float(f64::NAN));
    refused(key("mode"), NativeValue::Int(6));
    refused(key("mode"), NativeValue::Int(0));
    // 版の数だけを 34 にした正本は、種類 5 を知らないので読めない
    let mut bytes = native.to_bytes();
    bytes[8..12].copy_from_slice(&ANTI_ALIAS_VERSION.to_le_bytes());
    assert!(NativeDocument::read(&bytes).is_err());
    // 種類 4（放射状）を 5 に書き換えると、角度の欄が足りず読めない
    let mut w = world();
    w.doc
        .set_canvas_path(
            w.a,
            path_with(
                PathSymmetry::Canvas(CanvasSymmetry::new(SymmetryMode::Radial, c, 4).unwrap()),
                7,
            ),
        )
        .unwrap();
    let radial = NativeDocument::from_core(&w.doc).unwrap();
    assert!(radial
        .with_value(&key("mode"), NativeValue::Int(5))
        .is_err());
    // core の側も、線対称の奇数・範囲の外の角度のパスを持てない
    assert!(CanvasSymmetry::lines(c, 3, 0.0).is_err());
    let mut bad = CanvasSymmetry::lines(c, 4, 0.0).unwrap();
    bad.angle = 500.0;
    assert!(w
        .doc
        .set_canvas_path(w.a, path_with(PathSymmetry::Canvas(bad), 8))
        .is_err());
}

#[test]
fn a_path_in_a_list_with_other_paths_and_a_lines_symmetry_round_trips() {
    let mut w = world();
    let lines = CanvasSymmetry::lines(DVec2::new(10.0, 20.0), 8, -77.25).unwrap();
    let entries = vec![
        LayerPathEntry {
            name: "下".into(),
            visible: true,
            path: LayerPath::Canvas(path_with(PathSymmetry::None, 1)),
        },
        LayerPathEntry {
            name: "上".into(),
            visible: false,
            path: LayerPath::Canvas(path_with(PathSymmetry::Canvas(lines), 2)),
        },
    ];
    w.doc.set_canvas_paths(w.a, entries).unwrap();
    // 定規もいっしょに
    let r = ruler_2d(&mut w.doc, RulerKind::Concentric, (5.0, 5.0), (9.0, 5.0));
    w.doc.set_rulers(w.a, vec![r.clone()], false).unwrap();
    let native = NativeDocument::from_core(&w.doc).unwrap();
    assert_eq!(native.version(), RULERS_VERSION);
    let back = native.to_core().unwrap();
    assert_eq!(back.layer(w.a).unwrap().rulers(), &[r]);
    let sym: Vec<_> = back
        .layer(w.a)
        .unwrap()
        .paths()
        .iter()
        .map(|e| e.path.style().symmetry)
        .collect();
    assert_eq!(sym, vec![PathSymmetry::None, PathSymmetry::Canvas(lines)]);
    let _ = RulerId(0);
}

#[test]
fn truncated_or_damaged_rulers_documents_are_refused_or_read_without_panicking() {
    let mut w = world();
    assorted(&mut w);
    let mut path = path_with(
        PathSymmetry::Canvas(CanvasSymmetry::lines(DVec2::new(8.0, 9.0), 4, 33.0).unwrap()),
        3,
    );
    path.points.truncate(2);
    w.doc.set_canvas_path(w.a, path).unwrap();
    let bytes = NativeDocument::from_core(&w.doc).unwrap().to_bytes();
    assert!(NativeDocument::read(&bytes).is_ok());
    // どこで切っても断る
    for cut in 0..bytes.len() {
        assert!(
            NativeDocument::read(&bytes[..cut]).is_err(),
            "{cut} バイトで切った"
        );
    }
    // どのバイトを壊しても、読めるか断るかのどちらか（読めたなら、確かめに通る定規だけ）
    for at in 0..bytes.len() {
        for flip in [0xffu8, 0x55, 0x01] {
            let mut damaged = bytes.clone();
            damaged[at] ^= flip;
            if let Ok(doc) = NativeDocument::read(&damaged) {
                if let Ok(core) = doc.to_core() {
                    for layer in core.layers() {
                        assert!(layer.rulers().iter().all(|r| r.validate().is_ok()));
                    }
                }
            }
        }
    }
}
