//! テキストレイヤーの塗り（`yolu_core::text`）。外部の正解は無いので、並べの性質（揃え・行間・折り返し・字間）・断り（フォントを読めない・
//! 値の範囲・予算）と、塗った画素を固定するハッシュ（回帰の固定。`tests/text-index.txt`）を試す。ハッシュの撮り直しは
//! `YOLU_GOLDEN_UPDATE=1`。道・スレッドの数によらず同じバイトであることは crate の中の試験（`src/text/tests.rs`）。
#![allow(clippy::chunks_exact_to_as_chunks)]
use sha2::{Digest, Sha256};
use yolu_core::text::{self, TextAlign, TextFont, TextSettings};
use yolu_core::{CoreError, Rgba8};

use crate::golden_update;

const REGULAR: &[u8] = include_bytes!("../../../yolu-app/assets/fonts/BIZUDPGothic-Regular.ttf");
const BOLD: &[u8] = include_bytes!("../../../yolu-app/assets/fonts/BIZUDPGothic-Bold.ttf");

fn base(text: &str) -> TextSettings {
    let mut s = TextSettings::new(
        text,
        TextFont::Bundled("biz-udpgothic-regular".into()),
        16.0,
        200.0,
    );
    s.size = 28.0;
    s.color = Rgba8::new(20, 40, 200, 255);
    s
}

fn paint(s: &TextSettings, font: &[u8]) -> Vec<u8> {
    text::render(s, font, 0, 320, 224, 128, u64::MAX)
        .unwrap()
        .to_canvas_bytes()
}

fn cases() -> Vec<(&'static str, TextSettings, &'static [u8])> {
    let mut v: Vec<(&'static str, TextSettings, &'static [u8])> = Vec::new();
    v.push(("left", base("Hello, 世界。テクスチャ"), REGULAR));
    let mut s = base("中央\nCenter line");
    s.x = 160.0;
    s.align = TextAlign::Center;
    v.push(("center", s, REGULAR));
    let mut s = base("右揃え\nRight");
    s.x = 300.0;
    s.align = TextAlign::Right;
    v.push(("right", s, REGULAR));
    let mut s = base("折り返しの幅で行を分ける。Wrapping English words too.");
    s.wrap_width = 220.0;
    s.align = TextAlign::Center;
    v.push(("wrap", s, REGULAR));
    let mut s = base("行の高さ\n二行目\n三行目");
    s.line_height = 2.0;
    v.push(("line-height", s, REGULAR));
    let mut s = base("字の間 spacing");
    s.letter_spacing = 0.25;
    v.push(("letter-spacing", s, REGULAR));
    let mut s = base("詰める tight");
    s.letter_spacing = -0.08;
    v.push(("letter-tight", s, REGULAR));
    let mut s = base("回転 30°");
    s.x = 60.0;
    s.y = 100.0;
    s.rotation = 30.0;
    v.push(("rotation", s, REGULAR));
    let mut s = base("右へ回す");
    s.x = 150.0;
    s.y = 200.0;
    s.rotation = -90.0;
    v.push(("rotation-90", s, REGULAR));
    let mut s = base("半透明 Bold");
    s.color = Rgba8::new(250, 120, 10, 128);
    v.push(("alpha-bold", s, BOLD));
    let mut s = base("大");
    s.size = 260.0;
    s.x = 120.0;
    s.y = 250.0;
    v.push(("large-off-canvas", s, REGULAR));
    v
}

fn hash(b: &[u8]) -> String {
    format!("{:x}", Sha256::digest(b))
}

#[test]
fn painted_pixels_match_the_recorded_hashes() {
    let path = golden_update::tests_dir().join("text-index.txt");
    let table = std::fs::read_to_string(&path).unwrap_or_default();
    let mut lines = Vec::new();
    for (name, s, font) in cases() {
        let px = paint(&s, font);
        assert!(
            px.chunks_exact(4).any(|p| p[3] > 0),
            "{name}: 何も塗られていない"
        );
        lines.push(format!("{name} {}", hash(&px)));
    }
    if golden_update::updating() {
        std::fs::write(&path, lines.join("\n") + "\n").unwrap();
        return;
    }
    let expected: Vec<&str> = table.lines().collect();
    assert_eq!(
        expected, lines,
        "塗った画素が記録と違う（意図した変化なら YOLU_GOLDEN_UPDATE=1 で撮り直す）"
    );
}

#[test]
fn pixels_carry_the_text_color_and_coverage_only() {
    let s = base("色 Color");
    let px = paint(&s, REGULAR);
    let mut full = 0;
    for p in px.chunks_exact(4) {
        if p[3] == 0 {
            assert_eq!(p, [0, 0, 0, 0], "覆わない画素は RGB も 0");
        } else {
            assert_eq!(&p[..3], &[20, 40, 200], "色は文字の色のまま");
            full += usize::from(p[3] == 255);
        }
    }
    assert!(full > 100, "字の中は不透明");
    // アルファは文字の不透明度に掛かる
    let mut half = s.clone();
    half.color.a = 128;
    let h = paint(&half, REGULAR);
    for (a, b) in px.chunks_exact(4).zip(h.chunks_exact(4)) {
        assert_eq!(b[3] as u32, (a[3] as u32 * 128 + 127) / 255);
    }
}

#[test]
fn alignment_places_lines_against_the_anchor_or_the_box() {
    let mut s = base("ab\nlonger line");
    let left = text::layout(&s, REGULAR, 0).unwrap();
    assert!(left.lines.iter().all(|l| l.x == 0.0));
    s.align = TextAlign::Center;
    let center = text::layout(&s, REGULAR, 0).unwrap();
    for l in &center.lines {
        assert!((l.x + l.width * 0.5).abs() < 1e-9, "中央は基準の点が真ん中");
    }
    s.align = TextAlign::Right;
    let right = text::layout(&s, REGULAR, 0).unwrap();
    for l in &right.lines {
        assert!((l.x + l.width).abs() < 1e-9, "右揃えは基準の点で終わる");
    }
    s.wrap_width = 300.0;
    let boxed = text::layout(&s, REGULAR, 0).unwrap();
    for l in &boxed.lines {
        assert!(
            (l.x + l.width - 300.0).abs() < 1e-9,
            "幅があれば箱の右端で終わる"
        );
    }
    s.align = TextAlign::Center;
    let boxed = text::layout(&s, REGULAR, 0).unwrap();
    for l in &boxed.lines {
        assert!((l.x - (300.0 - l.width) * 0.5).abs() < 1e-9);
    }
}

#[test]
fn line_height_sets_the_distance_between_baselines() {
    let mut s = base("一\n二\n\n四");
    s.line_height = 1.75;
    let l = text::layout(&s, REGULAR, 0).unwrap();
    assert_eq!(l.lines.len(), 4, "空の段落も 1 行");
    assert!((l.lines[0].top).abs() < 1e-12, "1 行目の上端が基準の点");
    for w in l.lines.windows(2) {
        assert!((w[0].baseline - w[1].baseline - 1.75 * 28.0).abs() < 1e-9);
    }
    // 1 行目のベースラインはフォントの上の高さだけ下
    assert!((l.lines[0].baseline + l.ascent).abs() < 1e-9);
}

#[test]
fn letter_spacing_widens_every_gap_but_not_the_line_end() {
    let mut s = base("abcd");
    let plain = text::layout(&s, REGULAR, 0).unwrap().lines[0].width;
    s.letter_spacing = 0.5;
    let wide = text::layout(&s, REGULAR, 0).unwrap().lines[0].width;
    assert!(
        (wide - plain - 3.0 * 0.5 * 28.0).abs() < 1e-9,
        "3 つの間に足す"
    );
}

#[test]
fn wrapping_breaks_at_spaces_and_between_cjk_and_keeps_kinsoku() {
    let mut s = base("alpha beta gamma delta");
    s.wrap_width = 150.0;
    let l = text::layout(&s, REGULAR, 0).unwrap();
    assert!(l.lines.len() > 1);
    for line in &l.lines {
        let t = &s.text[line.start..line.end];
        assert!(!t.trim_end().contains(' ') || line.width <= 150.0);
        assert!(line.width <= 150.0 + 1e-9, "行の幅が箱に収まる: {t:?}");
        assert!(!t.starts_with(' '), "行頭に空白を残さない: {t:?}");
    }
    // 語の中では分けない
    let words: Vec<&str> = l
        .lines
        .iter()
        .flat_map(|line| s.text[line.start..line.end].split_whitespace())
        .collect();
    assert_eq!(words, ["alpha", "beta", "gamma", "delta"]);

    // 句点は行頭に来ない（前の字と一緒に送る）
    let mut s = base("あいうえおかきくけこ。さしすせそ");
    let one = text::layout(&s, REGULAR, 0).unwrap().lines[0]
        .carets
        .clone();
    // 「こ」の右端で箱を終える: 「。」は入らないので、「こ」ごと次の行へ
    let ko_end = one
        .iter()
        .find(|(b, _)| s.text[..*b].ends_with('こ'))
        .unwrap()
        .1;
    s.wrap_width = ko_end + 0.5;
    let l = text::layout(&s, REGULAR, 0).unwrap();
    for line in &l.lines {
        assert!(!s.text[line.start..line.end].starts_with('。'));
    }
    assert!(s.text[l.lines[1].start..].starts_with("こ。"));

    // 分けられる所の無い長い語は、字の境目で分ける
    let mut s = base("Supercalifragilistic");
    s.wrap_width = 60.0;
    let l = text::layout(&s, REGULAR, 0).unwrap();
    assert!(l.lines.len() > 2);
    let joined: String = l.lines.iter().map(|x| &s.text[x.start..x.end]).collect();
    assert_eq!(joined, s.text);
}

#[test]
fn rotation_turns_the_text_around_the_anchor() {
    let mut s = base("回す");
    s.x = 160.0;
    s.y = 112.0;
    let flat = paint(&s, REGULAR);
    s.rotation = 180.0;
    let turned = paint(&s, REGULAR);
    let sum = |b: &[u8]| b.chunks_exact(4).map(|p| p[3] as u64).sum::<u64>();
    let (a, b) = (sum(&flat), sum(&turned));
    assert!(a.abs_diff(b) * 100 < a, "回しても覆う量はほぼ同じ");
    // 回転の前は基準の点より下・右、180° 回すと上・左
    let centroid = |b: &[u8]| {
        let (mut x, mut y, mut n) = (0.0, 0.0, 0.0);
        for (i, p) in b.chunks_exact(4).enumerate() {
            let w = p[3] as f64;
            x += (i % 320) as f64 * w;
            y += (i / 320) as f64 * w;
            n += w;
        }
        (x / n, y / n)
    };
    let (fx, fy) = centroid(&flat);
    let (tx, ty) = centroid(&turned);
    assert!(fx > 160.0 && fy < 112.0);
    assert!(tx < 160.0 && ty > 112.0);
}

#[test]
fn an_unreadable_font_is_refused() {
    let s = base("フォント");
    let err = text::render(&s, b"not a font", 0, 64, 64, 64, u64::MAX).unwrap_err();
    assert_eq!(
        err,
        CoreError::InvalidArgument("フォントのファイルを読めない")
    );
    assert!(text::layout(&s, &REGULAR[..1000], 0).is_err());
    assert!(text::check_font(REGULAR, 0).is_ok());
    assert!(
        text::check_font(REGULAR, 1).is_err(),
        "束でないフォントの 1 番は無い"
    );
    assert_eq!(text::font_count(REGULAR), 1);
    assert_eq!(text::font_count(b"junk"), 0);
}

#[test]
fn a_font_file_remembers_its_names_weight_and_style() {
    let regular = text::describe_font(REGULAR, 0).unwrap();
    assert_eq!(regular.names.family, "BIZ UDPGothic");
    assert_eq!(regular.names.postscript, "BIZUDPGothic-Regular");
    assert_eq!(regular.names.weight, 400);
    assert!(!regular.names.italic);
    assert_eq!(regular.style, "Regular");
    assert_eq!(regular.family_ja.as_deref(), Some("BIZ UDPゴシック"));
    let bold = text::describe_font(BOLD, 0).unwrap();
    assert_eq!(
        (
            bold.names.postscript.as_str(),
            bold.names.weight,
            bold.style.as_str()
        ),
        ("BIZUDPGothic-Bold", 700, "Bold")
    );
    let TextFont::File {
        path,
        index,
        sha256,
        names,
    } = text::file_font("fonts/a.ttf", 0, BOLD)
    else {
        panic!()
    };
    assert_eq!((path.as_str(), index), ("fonts/a.ttf", 0));
    assert_eq!(sha256, text::sha256(BOLD));
    assert_eq!(names, bold.names);
    assert!(text::describe_font(b"junk", 0).is_err());
    // 読めないファイルは名前の無い値（道と SHA-256 だけで探す）
    let TextFont::File { names, .. } = text::file_font("x.ttf", 0, b"junk") else {
        panic!()
    };
    assert_eq!(names, text::FontNames::default());
}

#[test]
fn values_out_of_range_are_refused() {
    let ok = base("値");
    assert!(ok.validate().is_ok());
    type Change = Box<dyn Fn(&mut TextSettings)>;
    let bad: Vec<Change> = vec![
        Box::new(|s| s.size = 0.5),
        Box::new(|s| s.size = f64::NAN),
        Box::new(|s| s.line_height = 0.0),
        Box::new(|s| s.letter_spacing = -2.0),
        Box::new(|s| s.rotation = 400.0),
        Box::new(|s| s.wrap_width = -1.0),
        Box::new(|s| s.x = f64::INFINITY),
        Box::new(|s| s.text = "制御\u{7}".into()),
        Box::new(|s| s.text = "あ".repeat(30_000)),
        Box::new(|s| s.font = TextFont::Bundled("Bad Name".into())),
        Box::new(|s| {
            s.font = TextFont::File {
                path: String::new(),
                index: 0,
                sha256: [0; 32],
                names: Default::default(),
            }
        }),
        Box::new(|s| {
            s.font = TextFont::File {
                path: "a.ttf".into(),
                index: 0,
                sha256: [0; 32],
                names: yolu_core::text::FontNames {
                    family: "改行\n".into(),
                    ..Default::default()
                },
            }
        }),
        Box::new(|s| {
            s.font = TextFont::File {
                path: "a.ttf".into(),
                index: 0,
                sha256: [0; 32],
                names: yolu_core::text::FontNames {
                    weight: 0,
                    ..Default::default()
                },
            }
        }),
    ];
    for (i, f) in bad.iter().enumerate() {
        let mut s = ok.clone();
        f(&mut s);
        assert!(s.validate().is_err(), "{i} 番の値を断らない");
        assert!(text::render(&s, REGULAR, 0, 64, 64, 64, u64::MAX).is_err());
    }
    // 改行とタブは置ける
    let mut s = ok.clone();
    s.text = "a\tb\nc".into();
    assert!(s.validate().is_ok());
}

#[test]
fn the_budget_refuses_before_keeping_the_pixels() {
    let mut s = base("予算");
    s.size = 120.0;
    let err = text::render(&s, REGULAR, 0, 320, 224, 64, 1024).unwrap_err();
    assert_eq!(err, CoreError::SourceBudgetExceeded);
}

/// 大きな文書いっぱいの文字は、予算が小さければ、全部の升目を塗って持つ前に断る（塗るのは少しずつで、塗り終えたタイルから予算を見て入れる）。
#[test]
fn a_small_budget_refuses_a_page_filling_text_after_the_first_few_cells() {
    let mut s = base("大");
    s.size = 3500.0;
    s.x = 300.0;
    s.y = 3800.0;
    let tile = 128u32;
    let batch = 8;
    let budget = 3 * (tile as u64) * (tile as u64) * 4;
    let (refused, painted) = text::render_batched(&s, REGULAR, 0, 4096, 4096, tile, budget, batch);
    assert_eq!(refused.unwrap_err(), CoreError::SourceBudgetExceeded);
    // 予算が足りれば、同じ文字は升目を何回にも分けて塗る。断るときは、そのうちの先頭の少しだけ
    let (whole, all) = text::render_batched(&s, REGULAR, 0, 4096, 4096, tile, u64::MAX, batch);
    assert!(whole.unwrap().tile_count() > 3);
    assert!(
        all > 10 * batch,
        "大きな文字は何十もの升目にまたがる: {all}"
    );
    assert!(
        painted <= 2 * batch,
        "全部（{all}）を塗る前に止まる: 塗ったのは {painted}"
    );
}

/// 1 回に塗る升目の数・タイルの大きさ（升目の 256 画素で割り切れない・升目より大きいものも）によらず、同じ面になる
/// （タイルへ入れるのは、重なる升目を全部塗ってから）。
#[test]
fn painting_a_few_cells_at_a_time_gives_the_same_pixels_for_any_batch_and_tile_size() {
    let mut s = base("大きな\n文字 Wide");
    s.size = 600.0;
    s.x = 150.0;
    s.y = 820.0;
    s.rotation = -17.0;
    s.letter_spacing = 0.1;
    let (width, height) = (1100u32, 900u32);
    for tile in [64u32, 96, 128, 256, 512] {
        let (reference, cells) =
            text::render_batched(&s, REGULAR, 0, width, height, tile, u64::MAX, 10_000);
        let reference = reference.unwrap();
        assert!(cells > 8, "{cells}");
        let expected = reference.to_canvas_bytes();
        assert!(expected.chunks_exact(4).any(|p| p[3] > 0));
        for batch in [1usize, 3, 7] {
            let (surface, painted) =
                text::render_batched(&s, REGULAR, 0, width, height, tile, u64::MAX, batch);
            let surface = surface.unwrap();
            assert_eq!(painted, cells, "タイル {tile}・{batch} 升目ずつ");
            assert_eq!(
                surface.tile_count(),
                reference.tile_count(),
                "タイル {tile}・{batch} 升目ずつ"
            );
            assert!(
                surface.to_canvas_bytes() == expected,
                "タイル {tile}・{batch} 升目ずつで画素が変わった"
            );
        }
    }
}

#[test]
fn empty_text_paints_nothing() {
    let s = base("");
    let surface = text::render(&s, REGULAR, 0, 64, 64, 64, u64::MAX).unwrap();
    assert_eq!(surface.tile_count(), 0);
    let l = text::layout(&s, REGULAR, 0).unwrap();
    assert_eq!(l.lines.len(), 1);
    assert_eq!(l.lines[0].width, 0.0);
}

/// 見た目を確かめるための書き出し（`YOLU_TEXT_SHEET=<ディレクトリ>`。PAM）。ふだんは何もしない。
#[test]
fn dump_sheet() {
    let Ok(dir) = std::env::var("YOLU_TEXT_SHEET") else {
        return;
    };
    std::fs::create_dir_all(&dir).unwrap();
    for (name, s, font) in cases() {
        let px = paint(&s, font);
        let mut out =
            b"P7\nWIDTH 320\nHEIGHT 224\nDEPTH 4\nMAXVAL 255\nTUPLTYPE RGB_ALPHA\nENDHDR\n"
                .to_vec();
        // PAM は上の行から
        for row in px.chunks_exact(320 * 4).rev() {
            out.extend_from_slice(row);
        }
        std::fs::write(std::path::Path::new(&dir).join(format!("{name}.pam")), out).unwrap();
    }
}
