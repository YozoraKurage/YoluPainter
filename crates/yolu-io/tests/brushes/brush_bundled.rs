//! 同梱の Krita 4 の既定の筆先（CC0）: 原本のまま・全部読める・全部が描ける・ID が決まっている。
//! 原本の SHA-256 は葉のクレート `yolu-brush-sets` の試験でも照合する（こちらは読み手を通る側の確認）。

use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use yolu_core::glam::DVec2;
use yolu_core::{Document, Rgba8};
use yolu_io::brushes::{bundled, Source};

fn sums() -> Vec<(String, String)> {
    yolu_brush_sets::KRITA4_SHA256SUMS
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let (hash, name) = l.split_once(' ').unwrap();
            (
                name.trim()
                    .trim_start_matches('*')
                    .trim_start_matches("brushes/")
                    .to_string(),
                hash.to_string(),
            )
        })
        .collect()
}

#[test]
fn the_bundled_krita_set_loads_completely_and_matches_its_checksums() {
    let sums = sums();
    assert_eq!(sums.len(), 76);
    for (file, hash) in &sums {
        let bytes =
            bundled::original_bytes(file).unwrap_or_else(|| panic!("{file} が同梱されていない"));
        let actual: String = Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(&actual, hash, "{file} は上流の原本のまま");
    }
    assert!(
        yolu_brush_sets::KRITA4_META.contains("CC-0"),
        "許諾の宣言が筆先と一緒にある"
    );
    let started = std::time::Instant::now();
    let set = bundled::krita4();
    eprintln!(
        "同梱の筆先 {} 個を読むのに {:?}",
        set.brushes.len(),
        started.elapsed()
    );
    assert!(
        set.failures.is_empty(),
        "読めなかったファイル: {:?}",
        set.failures
            .iter()
            .map(|(f, e)| format!("{f}: {e}"))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        set.brushes.len(),
        sums.len(),
        "筆先のファイル 1 つにブラシ 1 つ"
    );
    assert!(
        set.brushes.iter().any(|b| b.brush.tip.images.len() > 1),
        "GIH のホースは複数の筆先を保つ"
    );
    assert!(
        std::ptr::eq(bundled::krita4(), bundled::krita4()),
        "読むのは 1 回だけ"
    );
}

fn painted(brush: &yolu_core::Brush) -> usize {
    let mut doc = Document::new(96, 96).unwrap();
    let layer = doc.add_layer("L").unwrap();
    let mut b = brush.clone();
    b.base.color = Rgba8::new(0, 0, 0, 255);
    b.base.pressure_size = false;
    b.base.pressure_opacity = false;
    let mut stroke = doc.begin_brush_stroke(layer, &b).unwrap();
    stroke
        .add_point(&mut doc, 20.0, 48.0, 1.0, DVec2::ZERO)
        .unwrap();
    stroke
        .add_point(&mut doc, 76.0, 48.0, 1.0, DVec2::ZERO)
        .unwrap();
    doc.end_stroke(stroke).unwrap();
    doc.composite(doc.bounds())
        .unwrap()
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| p[3] > 0)
        .count()
}

#[test]
fn every_bundled_brush_paints_and_keeps_its_id() {
    let mut ids = BTreeSet::new();
    for b in &bundled::krita4().brushes {
        assert!((4.0..=40.0).contains(&b.brush.base.radius), "{}", b.name);
        assert!(!b.brush.base.pressure_opacity, "{}", b.name);
        assert!(b.brush.validate().is_ok(), "{}", b.name);
        assert!(painted(&b.brush) > 0, "{} は描くと跡が残る", b.name);
        assert!(b.id.starts_with("bundled:krita4/"), "{}", b.id);
        assert!(ids.insert(b.id.clone()), "ID が重ならない: {}", b.id);
        assert_eq!(
            bundled::find(&b.id).map(|f| f.name.as_str()),
            Some(b.name.as_str())
        );
        assert_eq!(b.category, "Krita");
        assert!(!b.name.is_empty() && b.name != "Layer");
        assert_eq!(b.source, Source::BundledKrita4);
    }
    assert!(bundled::find("bundled:krita4/missing.png").is_none());
    assert!(bundled::find("builtin:grain").is_none());
}

#[test]
fn bundled_hoses_and_pngs_keep_their_tips() {
    let set = bundled::krita4();
    let hose = set
        .brushes
        .iter()
        .find(|b| b.id.ends_with("/chalk_chisel_random.gih"))
        .unwrap();
    assert!(hose.brush.tip.images.len() > 1);
    let png = set
        .brushes
        .iter()
        .find(|b| b.id.ends_with("/chalk.png"))
        .unwrap();
    assert!(png.brush.tip.image.is_some());
    assert_eq!(bundled::KRITA4_CATEGORY, "Krita");
}

/// 同梱の PNG の筆先を、Rust とは別の復号器（Pillow）で求めた被覆率の指紋と比べる（Unity 版の PNG の読みは Core の外の
/// Texture2D.LoadImage を通るので C# の照合に入らない。同じ式を Pillow の RGBA 変換の上で組んだもの。tools/brush-fixtures/png_tips.py）。
#[test]
fn bundled_png_tips_match_an_independent_decoder() {
    let expected = std::fs::read_to_string(format!(
        "{}/tests/fixtures/brushes/png-tips.txt",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let mut checked = 0;
    for line in expected.lines() {
        let p: Vec<&str> = line.split(' ').collect();
        let bytes =
            bundled::original_bytes(p[0]).unwrap_or_else(|| panic!("{} が同梱されていない", p[0]));
        let tip = yolu_io::brushes::read_png_tip(bytes, "t").unwrap();
        let mut all = (tip.width() as i32).to_le_bytes().to_vec();
        all.extend((tip.height() as i32).to_le_bytes());
        all.extend_from_slice(tip.alpha());
        let digest: String = Sha256::digest(&all)
            .iter()
            .take(8)
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(
            format!("{}x{} {digest}", tip.width(), tip.height()),
            format!("{} {}", p[1], p[2]),
            "{}",
            p[0]
        );
        checked += 1;
    }
    let png_files = yolu_brush_sets::KRITA4
        .iter()
        .filter(|t| t.file.ends_with(".png"))
        .count();
    assert_eq!(checked, png_files, "同梱の PNG が全部入っている");
}
