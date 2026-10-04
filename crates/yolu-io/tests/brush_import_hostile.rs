//! 信頼できないブラシファイル: 壊し方の網羅（全バイトの書き換え・全長での切り詰め・極端な値・ゴミ）と、大きさ・数・深さの上限。
//! どの入力でもパニックせず、断るか、検証を通るブラシを返す（`Core` の断りは読み手の不具合なので出てはいけない）。

mod brush_files;

use brush_files::*;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use yolu_io::brushes::{
    bundled, import_bytes, BrushImportError, Fault, FileKind, Unrepresented, UnsupportedFile,
    MAX_VBR_BYTES,
};

static IMPORTED: AtomicUsize = AtomicUsize::new(0);
static REFUSED: AtomicUsize = AtomicUsize::new(0);

/// 画面に出る文に、ファイルから来た制御文字（改行など）が残らない。
fn no_control(text: &str, what: &str) {
    assert!(
        !text.chars().any(|c| c.is_control()),
        "{what}: 文に制御文字: {text:?}"
    );
}

/// 1 つの入力の結果が満たすべきこと。壊れた形でも、決まった型で断るか、使えるブラシを返す。
fn check(kind: FileKind, bytes: &[u8], what: &str) {
    let result = catch_unwind(AssertUnwindSafe(|| import_bytes(kind, bytes, Some("F"))));
    let result = match result {
        Ok(r) => r,
        Err(_) => panic!("パニックした: {what}"),
    };
    match result {
        Ok(set) => {
            IMPORTED.fetch_add(1, Ordering::Relaxed);
            for b in &set.brushes {
                assert!(b.brush.validate().is_ok(), "検証を通らないブラシ: {what}");
                assert!(
                    !b.name.is_empty() && !b.name.chars().any(|c| c.is_control()),
                    "{what}: {:?}",
                    b.name
                );
                for t in b.brush.tip.image.iter().chain(b.brush.tip.images.iter()) {
                    assert!(t.width() <= 2048 && t.height() <= 2048);
                }
                for n in &b.unrepresented {
                    assert!(!n.to_string().is_empty() && !n.english().is_empty());
                    no_control(&n.to_string(), what);
                    no_control(&n.english(), what);
                }
            }
            for n in &set.notes {
                assert!(!n.to_string().is_empty() && !n.english().is_empty());
                no_control(&n.to_string(), what);
                no_control(&n.english(), what);
            }
        }
        Err(BrushImportError::Core(e)) => panic!("読み手が範囲外の設定を作った（{e}）: {what}"),
        Err(e) => {
            REFUSED.fetch_add(1, Ordering::Relaxed);
            assert!(!e.to_string().is_empty() && !e.english().is_empty());
            no_control(&e.to_string(), what);
            no_control(&e.english(), what);
        }
    }
}

/// 書き換え・切り詰め・極端な値・挿入・削除を全部試す。
fn hammer(kind: FileKind, base: &[u8], label: &str) {
    // 試験が空振りでないことの確認: 壊し方の一部は読めたまま（無害な書き換え）、一部は断られる
    let (before_ok, before_err) = (
        IMPORTED.load(Ordering::Relaxed),
        REFUSED.load(Ordering::Relaxed),
    );
    hammer_all(kind, base, label);
    let (ok, err) = (
        IMPORTED.load(Ordering::Relaxed) - before_ok,
        REFUSED.load(Ordering::Relaxed) - before_err,
    );
    eprintln!(
        "{label}: {} バイトから {} 通り（取り込めた {ok}、断った {err}）",
        base.len(),
        ok + err
    );
    assert!(
        ok > 0 && err > base.len(),
        "{label}: 取り込めた {ok}、断った {err}"
    );
}

fn hammer_all(kind: FileKind, base: &[u8], label: &str) {
    check(kind, base, &format!("{label}: 原本"));
    assert!(
        import_bytes(kind, base, None).is_ok(),
        "{label}: 原本は取り込める"
    );
    // 1 バイトを別の値へ
    for i in 0..base.len() {
        for value in [
            0x00u8,
            0x01,
            0x7F,
            0x80,
            0xFF,
            base[i] ^ 0xFF,
            base[i] ^ 0x01,
            base[i].wrapping_add(1),
        ] {
            if value == base[i] {
                continue;
            }
            let mut m = base.to_vec();
            m[i] = value;
            check(kind, &m, &format!("{label}: {i} バイト目を {value:#x}"));
        }
    }
    // 全長での切り詰め
    for len in 0..base.len() {
        check(kind, &base[..len], &format!("{label}: {len} バイトで切る"));
    }
    // 4 バイト（個数・長さ）を極端な値へ
    for i in 0..base.len().saturating_sub(3) {
        for value in [
            0u32,
            1,
            0x7FFF_FFFF,
            0x8000_0000,
            0xFFFF_FFFF,
            0x0001_0000,
            0x0000_0800,
            0x0000_0801,
        ] {
            let mut m = base.to_vec();
            m[i..i + 4].copy_from_slice(&value.to_be_bytes());
            check(
                kind,
                &m,
                &format!("{label}: {i} 〜 {} バイト目を {value:#x}", i + 3),
            );
        }
    }
    // 2 バイト（i16 の個数・大きさ）を極端な値へ
    for i in 0..base.len().saturating_sub(1) {
        for value in [0u16, 0x7FFF, 0x8000, 0xFFFF, 0x0801] {
            let mut m = base.to_vec();
            m[i..i + 2].copy_from_slice(&value.to_be_bytes());
            check(
                kind,
                &m,
                &format!("{label}: {i}・{} バイト目を {value:#x}", i + 1),
            );
        }
    }
    // 1 バイトの挿入と削除
    for i in 0..base.len() {
        let mut inserted = base.to_vec();
        inserted.insert(i, 0xA5);
        check(kind, &inserted, &format!("{label}: {i} バイト目に挿入"));
        let mut removed = base.to_vec();
        removed.remove(i);
        check(kind, &removed, &format!("{label}: {i} バイト目を削除"));
    }
}

#[test]
fn gimp_files_survive_every_corruption() {
    hammer(
        FileKind::Gbr,
        &gbr_gray(3, 2, &[1, 2, 3, 4, 5, 6], "Tip", 50),
        "gbr 灰",
    );
    hammer(
        FileKind::Gbr,
        &gbr(2, 1, &[1, 2, 3, 4, 5, 6, 7, 8], 4, "Col", 25, 1),
        "gbr 色 v1",
    );
    hammer(
        FileKind::Gih,
        &gih("Hose\n3 ncells:3 dim:1 sel0:random\n", &cells()),
        "gih",
    );
    hammer(
        FileKind::Gih,
        &gih("Seq\n2 dim:3 sel0:pressure\n", &cells()[..2]),
        "gih 筆圧",
    );
    hammer(
        FileKind::Vbr,
        b"GIMP-VBR\n1.0\nHardness 050\n10.000000\n25.000000\n0.500000\n1.000000\n0.000000\n",
        "vbr 1.0",
    );
    hammer(FileKind::Vbr, b"GIMP-VBR\r\n1.5\r\nStar\r\ndiamond\r\n50.000000\r\n25.000000\r\n5\r\n1.000000\r\n2.500000\r\n17.500000\r\n", "vbr 1.5");
}

#[test]
fn photoshop_files_survive_every_corruption() {
    let (v1, v2) = abr_v1v2();
    hammer(FileKind::Abr, &v1, "abr v1");
    hammer(FileKind::Abr, &v2, "abr v2");
    hammer(FileKind::Abr, &abr_v10(), "abr v10");
    hammer(FileKind::Abr, &dual_file(120.0), "abr v6 の全部入り");
    hammer(FileKind::Pat, &pat_file(), "pat");
}

/// 壊し方の試験の土台にする、小さな .sut（ページ 512 バイト）。筆先・質感・影響元・表せない設定を一通り持つ。
fn sut_base() -> Vec<u8> {
    use brush_files::sut::*;
    let tip = png_gray(2, 2, &[0, 255, 255, 0]);
    let grain = png_gray(2, 1, &[255, 40]);
    let mut builder = SutBuilder::new()
        .node("Tool", 0, 0)
        .material(Some("tip_a"), tar(&[("thumbnail/thumbnail.png", &tip)]))
        .material(Some("grain"), tar(&[("thumbnail/thumbnail.png", &grain)]))
        .brush(
            "Painter",
            1,
            &[
                ("BrushSize", real(30.0)),
                ("Opacity", int(90)),
                ("BrushFlow", int(70)),
                ("BrushInterval", int(20)),
                ("BrushThickness", int(80)),
                ("BrushUsePatternImage", int(1)),
                (
                    "BrushPatternImageArray",
                    blob(refs(&[["x/tip_a.png", "cat/a", "tip_a"]])),
                ),
                (
                    "TextureImage",
                    blob(refs(&[["x/grain.png", "cat/g", "grain"]])),
                ),
                ("TextureDensity", int(60)),
                (
                    "BrushSizeEffector",
                    blob(effector(
                        44,
                        0x10 | 0x40,
                        20,
                        &[&[(0.0, 0.0), (0.5, 0.7), (1.0, 1.0)]],
                    )),
                ),
                ("OpacityEffector", blob(effector(40, 0x10, 0, &[]))),
                ("BrushUseSpray", int(1)),
                ("BrushMixColor", int(25)),
            ],
        );
    builder.filler = 12;
    builder.page_size = 512;
    builder.build()
}

#[test]
fn sut_files_survive_every_corruption() {
    let base = sut_base();
    let set = import_bytes(FileKind::Sut, &base, Some("F")).expect("土台は取り込める");
    let b = &set.brushes[0];
    assert!(b.brush.tip.image.is_some() && b.brush.texture.is_some() && b.brush.base.pressure_size);
    assert!(base.len() < 12 * 1024, "土台は小さい: {}", base.len());
    // 取り込めた・断った の数は、壊し方の網羅の中の割合の目安（SQLite は使っていないページの書き換えでは変わらない）
    let (before_ok, before_err) = (
        IMPORTED.load(Ordering::Relaxed),
        REFUSED.load(Ordering::Relaxed),
    );
    hammer_all(FileKind::Sut, &base, "sut");
    let (ok, err) = (
        IMPORTED.load(Ordering::Relaxed) - before_ok,
        REFUSED.load(Ordering::Relaxed) - before_err,
    );
    eprintln!(
        "sut: {} バイトから {} 通り（取り込めた {ok}、断った {err}）",
        base.len(),
        ok + err
    );
    assert!(
        ok > 0 && err > base.len() / 2,
        "取り込めた {ok}、断った {err}"
    );
}

#[test]
fn sut_files_with_extreme_but_well_formed_shapes_never_make_the_reader_build_an_invalid_brush() {
    use brush_files::sut::*;
    // バイトの壊し方では届かない、形としては正しい極端な .sut。筆先の参照が上限いっぱい（同じ素材を 1024 個・名前の当たらない
    // 参照 1024 個）・素材の数と参照の数がちょうど 256・1 つ多い・素材が上限。どれも core の検証を通るブラシで取り込めるか、型で断る
    let tip = png_gray(1, 1, &[0]);
    let data = tar(&[("thumbnail/thumbnail.png", &tip)]);
    let brush = |array: Vec<u8>| {
        [
            ("BrushUsePatternImage", int(1)),
            ("BrushPatternImageArray", blob(array)),
        ]
    };
    let named = |n: usize, same: bool| -> Vec<u8> {
        let names: Vec<String> = (0..n)
            .map(|i| format!("mat{:04}", if same { 0 } else { i }))
            .collect();
        let items: Vec<[&str; 3]> = names
            .iter()
            .map(|n| [n.as_str(), n.as_str(), n.as_str()])
            .collect();
        refs(&items)
    };
    let with_materials = |count: usize, named_materials: bool, array: Vec<u8>| {
        let mut builder = SutBuilder::new();
        builder.filler = 0;
        builder.material_names = named_materials;
        for i in 0..count {
            builder = builder.material(Some(&format!("mat{i:04}")), data.clone());
        }
        builder.brush("Extreme", 1, &brush(array)).build()
    };
    for (what, file) in [
        (
            "同じ素材 1024 個",
            with_materials(3, true, named(1024, true)),
        ),
        (
            "名前の当たらない 1024 個",
            with_materials(256, false, named(1024, false)),
        ),
        (
            "素材と参照がちょうど 256",
            with_materials(256, false, named(256, false)),
        ),
        (
            "参照が素材より 1 つ多い",
            with_materials(256, false, named(257, false)),
        ),
        (
            "名前の当たる 256 個",
            with_materials(256, true, named(256, false)),
        ),
    ] {
        check(FileKind::Sut, &file, what);
    }
}

#[test]
fn png_files_survive_every_corruption() {
    use png::{BitDepth, ColorType};
    hammer(
        FileKind::Png,
        &png_file(3, 2, ColorType::Rgba, BitDepth::Eight, &[0; 24], None, None),
        "png rgba",
    );
    hammer(
        FileKind::Png,
        &png_file(
            2,
            1,
            ColorType::Indexed,
            BitDepth::Eight,
            &[0, 1],
            Some(&[0, 0, 0, 255, 255, 255]),
            Some(&[255, 0]),
        ),
        "png 索引",
    );
}

/// 決まった乱数（xorshift）。
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

#[test]
fn random_garbage_and_random_corruptions_never_panic() {
    let mut rng = Rng(0x2545_F491_4F6C_DD1D);
    let (v1, v2) = abr_v1v2();
    let bases: Vec<(FileKind, Vec<u8>)> = vec![
        (FileKind::Abr, v1),
        (FileKind::Abr, v2),
        (FileKind::Abr, abr_v10()),
        (FileKind::Abr, dual_file(120.0)),
        (FileKind::Pat, pat_file()),
        (
            FileKind::Gbr,
            gbr_gray(3, 2, &[1, 2, 3, 4, 5, 6], "Tip", 50),
        ),
        (FileKind::Gih, gih("Hose\n3 ncells:3\n", &cells())),
        (FileKind::Sut, sut_base()),
    ];
    for round in 0..3000 {
        // 完全なゴミ
        let len = (rng.next() % 300) as usize;
        let garbage: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
        for kind in [
            FileKind::Abr,
            FileKind::Pat,
            FileKind::Gbr,
            FileKind::Gih,
            FileKind::Vbr,
            FileKind::Png,
            FileKind::Sut,
        ] {
            check(kind, &garbage, &format!("ゴミ {round}"));
        }
        // 正しい形を数か所壊したもの
        let (kind, base) = &bases[round % bases.len()];
        let mut m = base.clone();
        for _ in 0..1 + rng.next() % 6 {
            let i = (rng.next() as usize) % m.len();
            m[i] = rng.next() as u8;
        }
        check(*kind, &m, &format!("乱れ {round}"));
    }
}

#[test]
fn the_bundled_krita_files_survive_header_corruption_and_cuts() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for tip in yolu_brush_sets::KRITA4
        .iter()
        .filter(|t| t.bytes.len() < 150_000)
    {
        let kind = match tip.file.rsplit('.').next().unwrap() {
            "gbr" => FileKind::Gbr,
            "gih" => FileKind::Gih,
            _ => FileKind::Png,
        };
        // ヘッダーの 96 バイトは全部のバイトを書き換える
        for i in 0..tip.bytes.len().min(96) {
            for value in [0x00u8, 0x7F, 0xFF, tip.bytes[i] ^ 0x80] {
                let mut m = tip.bytes.to_vec();
                m[i] = value;
                check(
                    kind,
                    &m,
                    &format!("{} の {i} バイト目を {value:#x}", tip.file),
                );
            }
        }
        for _ in 0..40 {
            let len = (rng.next() as usize) % tip.bytes.len();
            check(
                kind,
                &tip.bytes[..len],
                &format!("{} を {len} バイトで切る", tip.file),
            );
        }
    }
}

// ---------------- 大きさ・数・深さの上限 ----------------

/// 2048×2048 の筆先の PackBits の記録（1 行 = 128 画素の繰り返し 16 回 = 32 バイト。69 KB が 4 MB へ展開される）。
fn rle_bomb_record(id: &str) -> Vec<u8> {
    const SIDE: i32 = 2048;
    let row: Vec<u8> = (0..SIDE / 128).flat_map(|_| [0x81u8, 0x00]).collect(); // -127 → 128 回の繰り返し
    let mut w = W::new()
        .u8(id.len() as i32)
        .ascii(id)
        .bytes(&[0; 10])
        .i32(0)
        .i32(0)
        .i32(SIDE as i64)
        .i32(SIDE as i64)
        .i16(8)
        .u8(1);
    for _ in 0..SIDE {
        w = w.i16(row.len() as i32);
    }
    for _ in 0..SIDE {
        w = w.bytes(&row);
    }
    let data = w.done();
    W::new().i32(data.len() as i64).bytes(&data).pad4().done()
}

#[test]
fn a_small_file_cannot_expand_into_an_unbounded_amount_of_tips() {
    // 1 つあたり 69 KB で 4 MiB へ展開される筆先を 80 個（5.5 MB のファイルが 320 MiB の画像になる形）
    let mut samples = Vec::new();
    for i in 0..80 {
        samples.extend(rle_bomb_record(&format!("t{i}")));
    }
    let file = abr_v6(&[section("samp", &samples)]);
    assert!(
        file.len() < 8 * 1024 * 1024,
        "ファイルは小さい: {}",
        file.len()
    );
    let started = Instant::now();
    let result = import_bytes(FileKind::Abr, &file, None);
    assert!(
        matches!(result, Err(BrushImportError::Fault(Fault::Budget))),
        "{:?}",
        result.map(|s| s.brushes.len())
    );
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "予算で早く断る: {:?}",
        started.elapsed()
    );
    // 予算の中なら通る（1 つ）
    let one = abr_v6(&[section("samp", &rle_bomb_record("t"))]);
    let set = import_bytes(FileKind::Abr, &one, None).unwrap();
    assert_eq!(
        set.brushes[0].brush.tip.image.as_ref().unwrap().width(),
        2048
    );
}

fn pattern_bomb(name: &str) -> Vec<u8> {
    pattern_bomb_with_id(name, "x")
}

/// 2048×2048 で全部 0 の模様（PackBits で約 70 KB）。
fn pattern_bomb_with_id(name: &str, id: &str) -> Vec<u8> {
    const SIDE: i32 = 2048;
    let row: Vec<u8> = (0..SIDE / 128).flat_map(|_| [0x81u8, 0x00]).collect();
    let mut data = W::new();
    for _ in 0..SIDE {
        data = data.i16(row.len() as i32);
    }
    for _ in 0..SIDE {
        data = data.bytes(&row);
    }
    let array = W::new()
        .i32(8)
        .i32(0)
        .i32(0)
        .i32(SIDE as i64)
        .i32(SIDE as i64)
        .i16(8)
        .u8(1)
        .bytes(&data.done())
        .done();
    let list = W::new()
        .i32(0)
        .i32(0)
        .i32(SIDE as i64)
        .i32(SIDE as i64)
        .i32(1)
        .i32(1)
        .i32(array.len() as i64)
        .bytes(&array)
        .i32(0)
        .i32(0)
        .done();
    W::new()
        .i32(1)
        .i32(1)
        .i16(SIDE)
        .i16(SIDE)
        .unicode(name)
        .u8(id.len() as i32)
        .ascii(id)
        .i32(3)
        .i32(list.len() as i64)
        .bytes(&list)
        .done()
}

#[test]
fn patterns_are_bounded_too() {
    let mut w = W::new().ascii("8BPT").i16(1).i32(80);
    for i in 0..80 {
        w = w.bytes(&pattern_bomb(&format!("p{i}")));
    }
    let file = w.done();
    assert!(file.len() < 8 * 1024 * 1024);
    let started = Instant::now();
    let result = import_bytes(FileKind::Pat, &file, None);
    assert!(
        matches!(result, Err(BrushImportError::Fault(Fault::Budget))),
        "{:?}",
        result.map(|s| s.brushes.len())
    );
    assert!(started.elapsed() < Duration::from_secs(20));
    // ABR の模様の節で予算が尽きたら、筆先は取り込み、模様が読めなかったと知らせる
    let mut framed_patterns = Vec::new();
    for i in 0..80 {
        framed_patterns.push(pattern_bomb(&format!("p{i}")));
    }
    let abr = abr_v6(&[
        section("samp", &samp_record(1, "t", false)),
        section("patt", &framed(&framed_patterns)),
    ]);
    let set = import_bytes(FileKind::Abr, &abr, None).unwrap();
    assert!(set
        .notes
        .iter()
        .any(|n| matches!(n, Unrepresented::PatternsUnreadable(Fault::Budget))));
}

/// 1×1 の筆先（1 つ約 40 バイト）を `count` 個持つ版 6 の ABR。
fn tip_file(count: usize) -> Vec<u8> {
    let mut records = Vec::new();
    for i in 0..count {
        records.extend(samp_sized(1, &format!("t{i}"), 1, 1, &[255]));
    }
    abr_v6(&[section("samp", &records)])
}

#[test]
fn the_number_of_tips_in_a_version_6_file_is_bounded() {
    // 1×1 の筆先は画素の予算では止まらず、使われない筆先は 1 つごとにブラシになる。個数でも断る（版 1・2 と .pat も 1 万）
    let at_limit = import_bytes(FileKind::Abr, &tip_file(10_000), None).unwrap();
    assert_eq!(at_limit.brushes.len(), 10_000);
    let over = tip_file(10_001);
    assert!(over.len() < 1024 * 1024, "ファイルは小さい: {}", over.len());
    assert!(matches!(
        import_bytes(FileKind::Abr, &over, None),
        Err(BrushImportError::Fault(Fault::AbrTipCount(10_000)))
    ));
    // 節をまたいでも合計で数える
    let half = |from: usize| {
        let mut records = Vec::new();
        for i in from..from + 5_001 {
            records.extend(samp_sized(1, &format!("t{i}"), 1, 1, &[255]));
        }
        section("samp", &records)
    };
    assert!(matches!(
        import_bytes(FileKind::Abr, &abr_v6(&[half(0), half(5_001)]), None),
        Err(BrushImportError::Fault(Fault::AbrTipCount(10_000)))
    ));
}

#[test]
fn the_number_of_patterns_in_an_abr_pattern_section_is_bounded() {
    let tiny = |i: usize| pattern_gray(&format!("g{i}"), &format!("g{i}"), 1, 1, &[10]);
    let many: Vec<Vec<u8>> = (0..10_001).map(tiny).collect();
    let abr = abr_v6(&[
        section("samp", &samp_record(1, "t", false)),
        section("patt", &framed(&many)),
    ]);
    // 模様の節が読めない扱いになり、筆先は取り込む
    let set = import_bytes(FileKind::Abr, &abr, None).unwrap();
    assert!(set.notes.iter().any(|n| matches!(
        n,
        Unrepresented::PatternsUnreadable(Fault::PatCount(10_001))
    )));
    let fine = abr_v6(&[
        section("samp", &samp_record(1, "t", false)),
        section("patt", &framed(&many[..10_000])),
    ]);
    assert!(import_bytes(FileKind::Abr, &fine, None)
        .unwrap()
        .notes
        .is_empty());
}

/// 質感を使うプリセット（模様は ID で指す）。
fn textured_preset(name: &str, pattern_id: &str, invert: bool) -> Vec<u8> {
    let mut items = vec![
        ("Nm  ", text(name)),
        ("Brsh", obj("computedBrush", &[])),
        ("useTexture", boolean(true)),
        (
            "Txtr",
            obj("Ptrn", &[("Nm  ", text("P")), ("Idnt", text(pattern_id))]),
        ),
    ];
    if invert {
        items.push(("InvT", boolean(true)));
    }
    preset(&items)
}

#[test]
fn an_inverted_texture_is_made_once_per_pattern_and_shared_by_the_presets() {
    // 模様 1 つ（約 70 KB が 4 MiB へ展開される）と、それを「反転」で使うプリセット 1,000 個（反転ごとに複製すると 4 GiB）
    let mut presets: Vec<Vec<u8>> = (0..1_000)
        .map(|i| textured_preset(&format!("Inv {i}"), "x", true))
        .collect();
    presets.push(textured_preset("Plain", "x", false));
    let file = abr_v6(&[
        section("desc", &desc_body(&[brush_list(&presets)])),
        section("patt", &framed(&[pattern_bomb("P")])),
    ]);
    assert!(file.len() < 1024 * 1024, "ファイルは小さい: {}", file.len());
    let started = Instant::now();
    let set = import_bytes(FileKind::Abr, &file, None).unwrap();
    assert!(started.elapsed() < Duration::from_secs(20));
    assert_eq!(set.brushes.len(), 1_001);
    let inverted = &set.brushes[0].brush.texture.as_ref().unwrap().image;
    for b in &set.brushes[..1_000] {
        assert!(
            Arc::ptr_eq(inverted, &b.brush.texture.as_ref().unwrap().image),
            "{}: 反転した模様は共有する",
            b.name
        );
    }
    let plain = &set.brushes[1_000].brush.texture.as_ref().unwrap().image;
    assert!(!Arc::ptr_eq(inverted, plain));
    assert_eq!(
        (plain.at(0, 0), inverted.at(0, 0)),
        (0, 255),
        "反転は 255 から引く"
    );
}

#[test]
fn an_inverted_texture_counts_against_the_decode_budget() {
    // 4 MiB の模様 40 個（160 MiB）は予算（256 MiB）に入る。全部を反転して使うと、反転の複製も予算から引かれて足りなくなる
    let patterns: Vec<Vec<u8>> = (0..40)
        .map(|i| pattern_bomb_with_id(&format!("P{i}"), &format!("p{i}")))
        .collect();
    let file = |invert: bool| {
        let presets: Vec<Vec<u8>> = (0..40)
            .map(|i| textured_preset(&format!("B{i}"), &format!("p{i}"), invert))
            .collect();
        abr_v6(&[
            section("desc", &desc_body(&[brush_list(&presets)])),
            section("patt", &framed(&patterns)),
        ])
    };
    let started = Instant::now();
    let plain = import_bytes(FileKind::Abr, &file(false), None).unwrap();
    assert_eq!(plain.brushes.len(), 40);
    assert!(plain.notes.is_empty());
    let result = import_bytes(FileKind::Abr, &file(true), None);
    assert!(
        matches!(result, Err(BrushImportError::Fault(Fault::Budget))),
        "{:?}",
        result.map(|s| s.brushes.len())
    );
    assert!(started.elapsed() < Duration::from_secs(60));
}

/// 種類の違う 4 文字のキー（大文字だけ。`samp`・`desc`・`patt` とは重ならない）。
fn unique_key(i: usize) -> String {
    (0..4)
        .rev()
        .map(|k| (b'A' + ((i / 26usize.pow(k)) % 26) as u8) as char)
        .collect()
}

#[test]
fn file_wide_notes_do_not_multiply_with_the_number_of_brushes() {
    // 種類の違う未知の節 10 万個 × 筆先 3,000 個。節の注記をブラシごとに複製すると 3 億になり、種類の重複を線形に探すと 50 億回の比較になる
    let mut records = Vec::new();
    for i in 0..3_000 {
        records.extend(samp_sized(1, &format!("t{i}"), 1, 1, &[255]));
    }
    let mut sections = vec![section("samp", &records)];
    for i in 0..100_000 {
        sections.push(section(&unique_key(i), &[]));
    }
    let file = abr_v6(&sections);
    let started = Instant::now();
    let set = import_bytes(FileKind::Abr, &file, None).unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(set.brushes.len(), 3_000);
    assert!(
        set.brushes.iter().all(|b| b.unrepresented.is_empty()),
        "ファイル全体の注記はブラシに複製しない"
    );
    // 並べるのは種類 64 個まで。残りは節の数だけを 1 つの注記で知らせる
    assert_eq!(set.notes.len(), 65);
    assert_eq!(
        set.notes[0],
        Unrepresented::SectionSkipped(unique_key(0)),
        "ファイルの並び"
    );
    assert_eq!(
        set.notes[64],
        Unrepresented::MoreSectionsSkipped(100_000 - 64)
    );
    // 同じ種類の繰り返しは数え直さない
    let repeated: Vec<Vec<u8>> = std::iter::once(section("samp", &samp_record(1, "t", false)))
        .chain((0..1_000).map(|_| section("ZZZZ", &[1])))
        .collect();
    let set = import_bytes(FileKind::Abr, &abr_v6(&repeated), None).unwrap();
    assert_eq!(
        set.notes,
        vec![Unrepresented::SectionSkipped("ZZZZ".into())]
    );
}

#[test]
fn skipped_patterns_of_a_pat_file_are_listed_once_for_the_file() {
    // 使える模様 50 個と使えない（CMYK）50 個。使えない模様を全ブラシへ複製すると 50 × 50 の注記になる
    let mut w = W::new().ascii("8BPT").i16(1).i32(100);
    for i in 0..50 {
        w = w.bytes(&pattern_gray(
            &format!("g{i}"),
            &format!("g{i}"),
            1,
            1,
            &[10],
        ));
        w = w.bytes(&pattern(
            4,
            &format!("c{i}"),
            "c",
            1,
            1,
            &[vec![0], vec![0], vec![0], vec![0]],
            false,
            8,
            None,
        ));
    }
    let set = import_bytes(FileKind::Pat, &w.done(), None).unwrap();
    assert_eq!(set.brushes.len(), 50);
    assert_eq!(set.notes.len(), 50);
    assert!(set
        .notes
        .iter()
        .all(|n| matches!(n, Unrepresented::PatternSkipped { .. })));
    assert!(
        set.brushes.iter().all(|b| b.unrepresented.is_empty()),
        "ほかの模様の失敗を、無関係なブラシに並べない"
    );
}

#[test]
fn a_vbr_is_small_and_a_file_of_newlines_cannot_make_it_allocate() {
    let started = Instant::now();
    // 上限を 1 バイト超える改行だけのファイルは、行に割る前に断る
    let over = vec![b'\n'; MAX_VBR_BYTES as usize + 1];
    assert!(matches!(
        import_bytes(FileKind::Vbr, &over, None),
        Err(BrushImportError::FileTooLarge { limit }) if limit == MAX_VBR_BYTES
    ));
    // 上限ちょうどの改行だけのファイルは、VBR ではないと断る（行の一覧は先頭の数行だけ）
    assert!(matches!(
        import_bytes(FileKind::Vbr, &over[..MAX_VBR_BYTES as usize], None),
        Err(BrushImportError::Fault(Fault::NotVbr))
    ));
    assert!(matches!(
        import_bytes(FileKind::Vbr, &vec![b'\r'; MAX_VBR_BYTES as usize], None),
        Err(BrushImportError::Fault(Fault::NotVbr))
    ));
    // 正しい VBR の後ろに改行が続いても、上限の中なら取り込める
    let mut padded = b"GIMP-VBR\n1.0\nPadded\n10\n25\n0.5\n1\n0\n".to_vec();
    padded.resize(MAX_VBR_BYTES as usize, b'\n');
    let set = import_bytes(FileKind::Vbr, &padded, None).unwrap();
    assert_eq!(set.brushes[0].name, "Padded");
    padded.push(b'\n');
    assert!(matches!(
        import_bytes(FileKind::Vbr, &padded, None),
        Err(BrushImportError::FileTooLarge { .. })
    ));
    assert!(started.elapsed() < Duration::from_secs(5));
}

/// 記述子を 1 つの項目に入れた版 6 の ABR（筆先 1 つつき）。
fn abr_with_descriptor_item(value: Vec<u8>) -> Vec<u8> {
    let desc = W::new()
        .i32(16)
        .unicode("")
        .key("null")
        .i32(1)
        .key("Brsh")
        .bytes(&value)
        .done();
    abr_v6(&[
        section("samp", &samp_record(1, "t", false)),
        section("desc", &desc),
    ])
}

fn unreadable(file: &[u8]) -> Fault {
    let set = import_bytes(FileKind::Abr, file, None).unwrap();
    assert_eq!(set.brushes.len(), 1, "筆先は取り込む");
    assert!(
        set.brushes[0].unrepresented.is_empty(),
        "ファイル全体の注記はブラシに複製しない"
    );
    match &set.notes[..] {
        [Unrepresented::PresetsUnreadable(f)] => f.clone(),
        other => panic!("{other:?}"),
    }
}

#[test]
fn descriptor_limits_hold_without_overflowing_the_stack_or_memory() {
    // 入れ子の一覧を 10000 段
    let mut deep = Vec::new();
    for _ in 0..10_000 {
        deep.extend(W::new().ascii("VlLs").i32(1).done());
    }
    deep.extend(W::new().ascii("bool").u8(1).done());
    assert_eq!(
        unreadable(&abr_with_descriptor_item(deep)),
        Fault::DescriptorDepth
    );
    // 入れ子のオブジェクトを 1000 段
    let mut objects = Vec::new();
    for _ in 0..1000 {
        objects.extend(
            W::new()
                .ascii("Objc")
                .unicode("")
                .key("null")
                .i32(1)
                .key("Brsh")
                .done(),
        );
    }
    assert_eq!(
        unreadable(&abr_with_descriptor_item(objects)),
        Fault::DescriptorDepth
    );
    // 宣言だけ大きい一覧・項目数・文字列・キー・生のデータ
    assert_eq!(
        unreadable(&abr_with_descriptor_item(
            W::new().ascii("VlLs").i32(0xFFFF_FFFF).done()
        )),
        Fault::DescriptorItems
    );
    assert_eq!(
        unreadable(&abr_with_descriptor_item(
            W::new()
                .ascii("Objc")
                .unicode("")
                .key("null")
                .i32(0xFFFF_FFFF)
                .done()
        )),
        Fault::DescriptorItems
    );
    assert!(matches!(
        unreadable(&abr_with_descriptor_item(
            W::new().ascii("TEXT").i32(0xFFFF_FFFF).done()
        )),
        Fault::BadCount { .. }
    ));
    assert!(matches!(
        unreadable(&abr_with_descriptor_item(
            W::new().ascii("alis").i32(0x7FFF_FFFF).done()
        )),
        Fault::BadCount { .. }
    ));
    assert!(matches!(
        unreadable(&abr_with_descriptor_item(
            W::new().ascii("enum").i32(0x7FFF_FFFF).done()
        )),
        Fault::BadCount { .. }
    ));
    assert_eq!(
        unreadable(&abr_with_descriptor_item(
            W::new().ascii("obj ").i32(0xFFFF_FFFF).done()
        )),
        Fault::DescriptorItems
    );
    assert!(matches!(
        unreadable(&abr_with_descriptor_item(
            W::new().ascii("obj ").i32(1).ascii("zzzz").done()
        )),
        Fault::DescriptorReference(_)
    ));
    // 値の合計が上限（100 万）を超える: 10 万個の整数の一覧を 11 個
    let mut many = W::new().ascii("VlLs").i32(11);
    let one_list = W::new().ascii("VlLs").i32(100_000).done();
    let longs: Vec<u8> = (0..100_000)
        .flat_map(|_| W::new().ascii("long").i32(1).done())
        .collect();
    for _ in 0..11 {
        many = many.bytes(&one_list[4..]).bytes(&longs);
    }
    let started = Instant::now();
    // 一覧の中の一覧なので、型の 4 文字を足す
    let mut value = W::new().ascii("VlLs").i32(11);
    for _ in 0..11 {
        value = value.ascii("VlLs").i32(100_000).bytes(&longs);
    }
    let _ = many;
    assert_eq!(
        unreadable(&abr_with_descriptor_item(value.done())),
        Fault::DescriptorItems
    );
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[test]
fn huge_declared_sizes_in_the_file_structure_are_refused_before_allocating() {
    // 節の長さ・サンプルの長さ・模様の長さが 4 GiB
    assert!(matches!(
        import_bytes(
            FileKind::Abr,
            &W::new().i16(6).i16(1).ascii("8BIMsamp").i32(-1).done(),
            None
        ),
        Err(BrushImportError::Fault(Fault::BadCount { .. }))
    ));
    let samp = abr_v6(&[W::new().ascii("8BIMsamp").i32(8).i32(-1).i32(0).done()]);
    assert!(matches!(
        import_bytes(FileKind::Abr, &samp, None),
        Err(BrushImportError::Fault(Fault::BadCount { .. }))
    ));
    let patt = abr_v6(&[
        section("samp", &samp_record(1, "t", false)),
        W::new().ascii("8BIMpatt").i32(4).i32(-1).done(),
    ]);
    let set = import_bytes(FileKind::Abr, &patt, None).unwrap();
    assert!(set
        .notes
        .iter()
        .any(|n| matches!(n, Unrepresented::PatternsUnreadable(Fault::BadCount { .. }))));
    // ABR v1 の個数が上限（1 万）を超える
    assert!(import_bytes(FileKind::Abr, &W::new().i16(1).i16(10_001).done(), None).is_err());
    // GIH のセル数 256 で、先頭のセルが 2048×2048 と宣言するだけ
    let big_cell = gbr(2048, 2048, &[], 1, "big", 25, 2);
    assert!(matches!(
        import_bytes(FileKind::Gih, &gih("Big\n256\n", &[big_cell]), None),
        Err(BrushImportError::Fault(Fault::BadCount { .. }))
    ));
    // 模様のチャンネル数・データ長
    assert!(import_bytes(
        FileKind::Pat,
        &W::new()
            .ascii("8BPT")
            .i16(1)
            .i32(1)
            .i32(1)
            .i32(1)
            .i16(1)
            .i16(1)
            .unicode("x")
            .u8(0)
            .i32(3)
            .i32(-1)
            .done(),
        None
    )
    .is_err());
}

#[test]
fn names_from_files_are_cleaned_everywhere() {
    let preset_with_bad_name = preset(&[
        ("Nm  ", text("A\nB\u{0007}C")),
        ("Brsh", obj("computedBrush", &[])),
    ]);
    let file = abr_v6(&[
        section("desc", &desc_body(&[brush_list(&[preset_with_bad_name])])),
        section("samp", &samp_record(1, "t", false)),
    ]);
    let set = import_bytes(FileKind::Abr, &file, None).unwrap();
    assert_eq!(set.brushes[0].name, "ABC");
    let long = "x".repeat(1000);
    let preset_long = preset(&[("Nm  ", text(&long)), ("Brsh", obj("computedBrush", &[]))]);
    let file = abr_v6(&[
        section("desc", &desc_body(&[brush_list(&[preset_long])])),
        section("samp", &samp_record(1, "t", false)),
    ]);
    assert_eq!(
        import_bytes(FileKind::Abr, &file, None).unwrap().brushes[0]
            .name
            .chars()
            .count(),
        128
    );
}

#[test]
fn keys_and_type_codes_from_a_file_cannot_put_control_characters_in_a_message() {
    let clean = |text: &str| assert!(!text.chars().any(|c| c.is_control()), "{text:?}");
    // 知らない節のキー（制御文字は ? になり、字数が残る）
    let file = abr_v6(&[
        section("samp", &samp_record(1, "t", false)),
        section("a\nb\u{7}", &[1]),
    ]);
    let set = import_bytes(FileKind::Abr, &file, None).unwrap();
    assert_eq!(
        set.notes,
        vec![Unrepresented::SectionSkipped("a?b?".into())]
    );
    for n in &set.notes {
        clean(&n.to_string());
        clean(&n.english());
    }
    // 設定の値の型・参照の形
    let kind = unreadable(&abr_with_descriptor_item(
        W::new().ascii("\u{1}\n\u{7}\u{1b}").done(),
    ));
    assert!(matches!(&kind, Fault::DescriptorType { kind, .. } if kind == "????"));
    let form = unreadable(&abr_with_descriptor_item(
        W::new()
            .ascii("obj ")
            .i32(1)
            .ascii("\u{7}\n\u{1}\u{2}")
            .done(),
    ));
    assert_eq!(form, Fault::DescriptorReference("????".into()));
    for fault in [kind, form] {
        clean(&fault.to_string());
        clean(&fault.english());
    }
    // 知らない拡張子（ファイル名から来る）
    match FileKind::from_extension("x\ny\u{7}") {
        Err(UnsupportedFile::Extension(e)) => assert_eq!(e, "x?y?"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn the_bundled_set_is_not_affected_by_hostile_input() {
    // 同梱は原本のバイト列から読むので、壊れた入力の試験と同じ読み手を通っても結果が変わらない
    assert!(bundled::krita4().failures.is_empty());
    assert_eq!(
        bundled::krita4().brushes.len(),
        yolu_brush_sets::KRITA4.len()
    );
}
