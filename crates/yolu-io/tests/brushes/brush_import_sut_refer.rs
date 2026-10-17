//! CLIP STUDIO の `.sut` のはみ出し防止（参照レイヤーの線をはみ出さない・色の誤差・領域拡縮）の取り込み。テストのファイルは公開の解析で
//! 分かっている形をテストの中で作る（`brush_files::sut`）。
use crate::brush_files;

use brush_files::sut::*;
use rusqlite::types::Value;
use yolu_core::{ExpandMode, ReferenceLines};
use yolu_io::brushes::{import_bytes, FileKind, ImportedBrush, SutMapped, SutNote, Unrepresented};

fn brush(columns: &[(&'static str, Value)]) -> ImportedBrush {
    let file = SutBuilder::new().brush("Pen", 1, columns).build();
    import_bytes(FileKind::Sut, &file, Some("File name"))
        .unwrap()
        .brushes
        .remove(0)
}

fn has(b: &ImportedBrush, n: SutNote) -> bool {
    b.unrepresented.contains(&Unrepresented::ClipStudio(n))
}

fn mapped(b: &ImportedBrush) -> bool {
    b.mapped.contains(&SutMapped::ReferenceLines)
}

#[test]
fn the_flag_off_carries_nothing_even_with_values() {
    let b = brush(&[
        ("BrushUseReferLayer", int(0)),
        ("FillUseExpand", int(1)),
        ("FillExpandType", int(7)),
        ("FillColorMargin", real(30.0)),
    ]);
    assert_eq!(b.brush.reference_lines, ReferenceLines::default());
    assert!(!mapped(&b));
    assert_eq!(b.unrepresented, vec![]);
}

#[test]
fn the_flag_margin_and_area_scaling_are_carried() {
    for (kind, mode) in [(1, ExpandMode::Round), (2, ExpandMode::Darkest)] {
        let b = brush(&[
            ("BrushUseReferLayer", int(1)),
            ("FillColorMargin", real(20.0)),
            ("FillUseExpand", int(1)),
            ("FillExpandLength", real(5.0)),
            ("FillExpandLengthUnit", int(0)),
            ("FillExpandType", int(kind)),
        ]);
        let l = b.brush.reference_lines;
        assert!(l.enabled && l.expand, "{kind}");
        assert_eq!((l.margin, l.amount, l.mode), (20.0, 5.0, mode), "{kind}");
        assert!(mapped(&b));
        assert_eq!(b.unrepresented, vec![], "{kind}");
    }
    // 列の無い値は既定（色の誤差 12.5・量 2・最も濃いピクセルまで、領域拡縮は切）
    let b = brush(&[("BrushUseReferLayer", int(1))]);
    assert_eq!(
        b.brush.reference_lines,
        ReferenceLines {
            enabled: true,
            ..ReferenceLines::default()
        }
    );
}

#[test]
fn unverified_modes_units_and_ranges_are_reported() {
    let scaled = |kind: i64, unit: i64| {
        brush(&[
            ("BrushUseReferLayer", int(1)),
            ("FillUseExpand", int(1)),
            ("FillExpandType", int(kind)),
            ("FillExpandLengthUnit", int(unit)),
        ])
    };
    // 0 は四角くと見て写し、確かめていないと知らせる。知らない値は既定の方法で知らせる
    let b = scaled(0, 0);
    assert_eq!(b.brush.reference_lines.mode, ExpandMode::Square);
    assert!(has(&b, SutNote::ExpandMode(0.0)));
    let b = scaled(3, 0);
    assert_eq!(b.brush.reference_lines.mode, ExpandMode::Darkest);
    assert!(has(&b, SutNote::ExpandMode(3.0)));
    // 単位が画素でない
    let b = scaled(1, 1);
    assert!(has(&b, SutNote::ExpandUnit(1.0)));
    // 領域拡縮が切なら、方法・単位は知らせない
    let b = brush(&[
        ("BrushUseReferLayer", int(1)),
        ("FillExpandType", int(9)),
        ("FillExpandLengthUnit", int(2)),
    ]);
    assert_eq!(b.unrepresented, vec![]);
    // 範囲の外は収めて知らせる
    let b = brush(&[
        ("BrushUseReferLayer", int(1)),
        ("FillColorMargin", real(150.0)),
        ("FillUseExpand", int(1)),
        ("FillExpandLength", real(-300.0)),
    ]);
    assert_eq!(b.brush.reference_lines.margin, 100.0);
    assert_eq!(b.brush.reference_lines.amount, -100.0);
    assert!(has(&b, SutNote::ReferenceRange));
    assert!(b.brush.validate().is_ok());
}
