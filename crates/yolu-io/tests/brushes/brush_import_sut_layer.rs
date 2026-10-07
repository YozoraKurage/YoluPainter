//! CLIP STUDIO の `.sut` の素材の独自の入れ物 C2F（`.layer`）の取り込み: 原寸の筆先・質感を読むこと、読めないものを断ってプレビューか丸い
//! 筆先に戻ること、壊した C2F でファイル全体が失敗しないこと。試験の C2F は本物の素材ではなく、調べて分かった形を試験の中で組む
//! （`brush_files::c2f`）。本物の `.sut` での確かめは `#[ignore]` の試験（環境変数 `YOLU_REAL_SUT` のパス）。

use crate::brush_files;

use brush_files::c2f::{self, Layer};
use brush_files::sut::*;
use yolu_io::brushes::{
    import_bytes, BrushImportError, FileKind, ImportedBrush, ImportedSet, SutNote, Unrepresented,
};

fn read(bytes: &[u8]) -> Result<ImportedSet, BrushImportError> {
    import_bytes(FileKind::Sut, bytes, Some("File name"))
}

fn ok(bytes: &[u8]) -> ImportedSet {
    read(bytes).unwrap_or_else(|e| panic!("取り込めない: {e:?}"))
}

fn has(brush: &ImportedBrush, n: SutNote) -> bool {
    brush.unrepresented.contains(&Unrepresented::ClipStudio(n))
}

/// プレビューの PNG（C2F が読めたときは使わないはずの別の絵）。
fn decoy() -> Vec<u8> {
    png_gray(2, 2, &[0, 10, 20, 30])
}

fn material(c2f: &[u8], with_preview: bool) -> Vec<u8> {
    let preview = decoy();
    let mut files: Vec<(&str, &[u8])> = vec![
        ("catalog.zip", b"x"),
        ("data/material.layer", c2f),
        ("icedata/layerData.xml", b"<infolist/>"),
    ];
    if with_preview {
        files.push(("thumbnail/thumbnail.png", &preview));
    }
    tar(&files)
}

fn tip_file(material: Vec<u8>) -> Vec<u8> {
    SutBuilder::new()
        .material(Some("tip_a"), material)
        .brush(
            "Stamp",
            1,
            &[
                ("BrushSize", real(30.0)),
                ("BrushUsePatternImage", int(1)),
                (
                    "BrushPatternImageArray",
                    blob(refs(&[["C:\\mats\\tip_a.png", "cat/aaaa", "tip_a"]])),
                ),
            ],
        )
        .build()
}

fn texture_file(material: Vec<u8>) -> Vec<u8> {
    SutBuilder::new()
        .material(Some("grain"), material)
        .brush(
            "Textured",
            1,
            &[
                (
                    "TextureImage",
                    blob(refs(&[["g/grain.png", "c/grain", "grain"]])),
                ),
                ("TextureDensity", int(40)),
                ("TextureScale2", int(200)),
            ],
        )
        .build()
}

/// 筆先の被覆率の期待値: 画像の行は上から、筆先は下の行が先。値（黒の不透明度）がそのまま被覆率。
fn tip_alpha(width: u32, height: u32, values: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; values.len()];
    for y in 0..height {
        for x in 0..width {
            out[((height - 1 - y) * width + x) as usize] = values[(y * width + x) as usize];
        }
    }
    out
}

fn tip_of(file: &[u8]) -> (u32, u32, Vec<u8>, ImportedBrush) {
    let set = ok(file);
    let b = set.brushes.into_iter().next().unwrap();
    let tip = b.brush.tip.image.clone().expect("筆先の画像");
    (tip.width(), tip.height(), tip.alpha().to_vec(), b)
}

fn no_image_notes(b: &ImportedBrush) {
    for n in [
        SutNote::TipMissing,
        SutNote::ProprietaryImage,
        SutNote::PreviewImage,
        SutNote::TipGuessed,
    ] {
        assert!(!has(b, n.clone()), "{n:?} が付いている");
    }
}

// ---------------- 読めるもの ----------------

#[test]
fn the_full_size_image_in_the_layer_is_the_tip_and_dark_paints() {
    // 2×2 のタイル（256 より大きい）で、端の欠けたタイルも読む
    let layer = Layer::new(300, 270);
    let (w, h, alpha, b) = tip_of(&tip_file(material(&layer.c2f(), false)));
    assert_eq!((w, h), (300, 270));
    assert_eq!(
        alpha,
        tip_alpha(300, 270, &layer.values),
        "値がそのまま被覆率。行は下から"
    );
    assert_eq!(b.brush.base.radius, 15.0, "設定は取り込む");
    no_image_notes(&b);
}

#[test]
fn the_layer_wins_over_the_preview_png_and_needs_no_resolution_note() {
    let layer = Layer::new(40, 30);
    let (w, h, alpha, b) = tip_of(&tip_file(material(&layer.c2f(), true)));
    assert_eq!((w, h), (40, 30), "プレビューの 2×2 ではない");
    assert_eq!(alpha, tip_alpha(40, 30, &layer.values));
    no_image_notes(&b);
}

#[test]
fn the_text_encoding_and_the_extra_parameter_words_may_differ() {
    // 本物の素材には、文字が UTF-8 のものと UTF-16LE のもの、Parameter の終わりの 0 の語が 0 個と 2 個のものがあった
    for (utf16, extra) in [(false, 0), (true, 0), (false, 2), (true, 2)] {
        let mut layer = Layer::new(70, 20);
        layer.utf16 = utf16;
        layer.extra_words = extra;
        let (w, h, alpha, b) = tip_of(&tip_file(material(&layer.c2f(), false)));
        assert_eq!((w, h), (70, 20), "utf16={utf16} extra={extra}");
        assert_eq!(alpha, tip_alpha(70, 20, &layer.values));
        no_image_notes(&b);
    }
}

#[test]
fn tiles_without_pixels_stay_empty_and_the_image_is_cropped_to_its_size() {
    let mut layer = Layer::new(300, 270);
    layer.empty_tiles = vec![1, 2];
    let mut expected = layer.values.clone();
    for y in 0..270u32 {
        for x in 0..300u32 {
            let tile = (y / 256) * 2 + x / 256;
            if tile == 1 || tile == 2 {
                expected[(y * 300 + x) as usize] = 0;
            }
        }
    }
    let (_, _, alpha, b) = tip_of(&tip_file(material(&layer.c2f(), false)));
    assert_eq!(alpha, tip_alpha(300, 270, &expected));
    no_image_notes(&b);
}

#[test]
fn the_original_image_is_preferred_and_the_render_image_is_the_fallback() {
    // 元の画像と描画用の画像が違う値のとき、元の画像
    let mut layer = Layer::new(20, 20);
    layer.render_values = Some(vec![9; 20 * 20]);
    let (_, _, alpha, _) = tip_of(&tip_file(material(&layer.c2f(), false)));
    assert_eq!(alpha, tip_alpha(20, 20, &layer.values));
    // 元のミップマップが無い素材は、描画用の画像
    let mut layer = Layer::new(20, 20);
    layer.original = false;
    layer.render_values = Some((0..400).map(|i| (i % 200) as u8).collect());
    let (_, _, alpha, _) = tip_of(&tip_file(material(&layer.c2f(), false)));
    assert_eq!(
        alpha,
        tip_alpha(20, 20, layer.render_values.as_ref().unwrap())
    );
    // 元の画像が空なら、描画用の画像
    let mut layer = Layer::new(20, 20);
    layer.original_empty = true;
    layer.render_values = Some((0..400).map(|i| (i % 150) as u8).collect());
    let (_, _, alpha, _) = tip_of(&tip_file(material(&layer.c2f(), false)));
    assert_eq!(
        alpha,
        tip_alpha(20, 20, layer.render_values.as_ref().unwrap())
    );
}

#[test]
fn the_texture_is_the_layer_image_where_white_paints() {
    let layer = Layer::new(300, 20);
    let set = ok(&texture_file(material(&layer.c2f(), false)));
    let b = &set.brushes[0];
    let t = b.brush.texture.as_ref().expect("質感");
    assert_eq!((t.image.width(), t.image.height()), (300, 20));
    // 黒の不透明度 v は、白い紙の上では輝度 255 − v。白（輝度が高い所）が塗れる
    let expected: Vec<u8> = tip_alpha(300, 20, &layer.values)
        .into_iter()
        .map(|v| 255 - v)
        .collect();
    assert_eq!(t.image.alpha(), expected);
    assert_eq!((t.depth, t.scale), (0.4, 2.0));
    for n in [
        SutNote::TextureMissing,
        SutNote::ProprietaryImage,
        SutNote::PreviewImage,
    ] {
        assert!(!has(b, n.clone()), "{n:?} が付いている");
    }
}

#[test]
fn long_overflow_chains_and_noisy_tiles_are_read() {
    // 圧縮できない画素（乱数）は、1 タイルが 60 ページ以上のオーバーフローになる
    let mut layer = Layer::new(256, 256);
    let mut x = 12345u32;
    layer.values = (0..256 * 256)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            (x >> 9) as u8
        })
        .collect();
    let (_, _, alpha, b) = tip_of(&tip_file(material(&layer.c2f(), false)));
    assert_eq!(alpha, tip_alpha(256, 256, &layer.values));
    no_image_notes(&b);
}

// ---------------- どの素材がどのブラシのものか（素材の種類で当てる） ----------------

/// 素材の `icedata/layerData.xml`（種類の印 `systemtag` だけ。本物の形に合わせた最小の XML）。
fn layer_data(tag: &str) -> Vec<u8> {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"no\" ?>\n<infolist>\n  <version>2.0.0</version>\n  <info uuid=\"0123456789-abcd-ef01-2345-6789abcdef\">\n    <datalist key=\"systemtag\">\n      <data>Resizable</data>\n      <data>{tag}</data>\n    </datalist>\n    <datalist key=\"scaling\">\n      <data>0</data>\n    </datalist>\n  </info>\n</infolist>\n"
    )
    .into_bytes()
}

/// 種類の印つきの素材（`tag` が None なら layerData.xml を入れない）。
fn kinded(layer: &Layer, tag: Option<&str>) -> Vec<u8> {
    let c2f = layer.c2f();
    let xml = layer_data(tag.unwrap_or(""));
    let mut files: Vec<(&str, &[u8])> = vec![("data/material.layer", &c2f)];
    if tag.is_some() {
        files.push(("icedata/layerData.xml", &xml));
    }
    tar(&files)
}

/// 名前のない素材（名前では当たらない）の並びと、1 つのブラシの設定。
fn kinded_file(materials: Vec<Vec<u8>>, dual: bool) -> Vec<u8> {
    let mut builder = SutBuilder::new();
    for m in materials {
        builder = builder.material(None, m);
    }
    let one = |name: &str| blob(refs(&[[name, "cat/x", name]]));
    builder
        .brush(
            "Stamp",
            1,
            &[
                ("BrushUsePatternImage", int(1)),
                ("BrushPatternImageArray", one("tip main")),
                ("TextureImage", one("texture main")),
                ("TextureDensity", int(100)),
                ("UseDualBrush", int(dual as i64)),
                ("DualUsePatternImage", int(1)),
                ("DualPatternImageArray", one("tip dual")),
                ("DualTextureImage", one("texture dual")),
            ],
        )
        .build()
}

#[test]
fn materials_are_matched_by_kind_when_the_dual_brush_accounts_for_the_rest() {
    // 本物のファイルの並び: 質感 2（メイン・デュアル）→ 筆先 2（メイン・デュアル）。参照の名前では当たらない
    let (tex1, tex2) = (Layer::new(50, 10), Layer::new(60, 10));
    let (tip1, tip2) = (Layer::new(30, 20), Layer::new(40, 20));
    let file = kinded_file(
        vec![
            kinded(&tex1, Some("PaperTexture")),
            kinded(&tex2, Some("PaperTexture")),
            kinded(&tip1, Some("BrushPattern")),
            kinded(&tip2, Some("BrushPattern")),
        ],
        true,
    );
    let set = ok(&file);
    let b = &set.brushes[0];
    let tip = b.brush.tip.image.as_ref().expect("筆先");
    assert_eq!(
        (tip.width(), tip.height()),
        (30, 20),
        "メインの筆先は筆先の素材の先頭"
    );
    assert_eq!(tip.alpha(), tip_alpha(30, 20, &tip1.values));
    let texture = b.brush.texture.as_ref().expect("質感");
    assert_eq!((texture.image.width(), texture.image.height()), (50, 10));
    assert!(
        has(b, SutNote::TipGuessed) && has(b, SutNote::TextureGuessed),
        "並びで当てたので推定と知らせる"
    );
    assert!(!has(b, SutNote::TipMissing) && !has(b, SutNote::TextureMissing));
}

#[test]
fn a_tip_and_a_texture_are_found_by_kind_whatever_the_file_order() {
    // デュアルなし。質感の素材が先でも、筆先は筆先の素材
    let (tex, tip) = (Layer::new(50, 10), Layer::new(30, 20));
    for order in [[0, 1], [1, 0]] {
        let all = [
            kinded(&tex, Some("PaperTexture")),
            kinded(&tip, Some("BrushPattern")),
        ];
        let file = kinded_file(order.iter().map(|&i| all[i].clone()).collect(), false);
        let b = &ok(&file).brushes[0];
        let t = b.brush.tip.image.as_ref().expect("筆先");
        assert_eq!((t.width(), t.height()), (30, 20), "順 {order:?}");
        let x = b.brush.texture.as_ref().expect("質感");
        assert_eq!(
            (x.image.width(), x.image.height()),
            (50, 10),
            "順 {order:?}"
        );
        assert!(has(b, SutNote::TipGuessed) && has(b, SutNote::TextureGuessed));
    }
}

#[test]
fn nothing_is_guessed_when_the_counts_per_kind_do_not_fit() {
    let (tex1, tex2) = (Layer::new(50, 10), Layer::new(60, 10));
    let (tip1, tip2) = (Layer::new(30, 20), Layer::new(40, 20));
    let all = || {
        vec![
            kinded(&tex1, Some("PaperTexture")),
            kinded(&tex2, Some("PaperTexture")),
            kinded(&tip1, Some("BrushPattern")),
            kinded(&tip2, Some("BrushPattern")),
        ]
    };
    // 4 つの素材があるのにデュアルブラシを使わない設定: どれが誰のものか決められない
    let b = &ok(&kinded_file(all(), false)).brushes[0];
    assert!(b.brush.tip.image.is_none() && b.brush.texture.is_none());
    assert!(has(b, SutNote::TipMissing) && has(b, SutNote::TextureMissing));
    assert!(!has(b, SutNote::TipGuessed) && !has(b, SutNote::TextureGuessed));
    // 素材が 1 つ足りない
    let mut fewer = all();
    fewer.pop();
    let b = &ok(&kinded_file(fewer, true)).brushes[0];
    assert!(b.brush.tip.image.is_none());
    assert!(has(b, SutNote::TipMissing));
    // 種類の印の無い素材が混ざる（印の無い素材が何の素材か分からないので、数えられない）
    let mut unknown = all();
    unknown[3] = kinded(&tip2, None);
    let b = &ok(&kinded_file(unknown, true)).brushes[0];
    assert!(b.brush.tip.image.is_none() && b.brush.texture.is_none());
    assert!(!has(b, SutNote::TipGuessed) && !has(b, SutNote::TextureGuessed));
    // 種類が筆先でも質感でもない素材が混ざっても、ほかの素材の数が合えば当たる（その素材は使われない）
    let other = Layer::new(10, 10);
    let mut extra = vec![
        kinded(&tip1, Some("BrushPattern")),
        kinded(&tex1, Some("PaperTexture")),
    ];
    extra.push(kinded(&other, Some("Pattern")));
    let b = &ok(&kinded_file(extra, false)).brushes[0];
    assert_eq!(b.brush.tip.image.as_ref().map(|t| t.width()), Some(30));
    assert_eq!(b.brush.texture.as_ref().map(|t| t.image.width()), Some(50));
}

// ---------------- 断るもの ----------------

/// 読めない C2F のとき: 丸い筆先で、独自の形式で読めないことを知らせ、設定は取り込む。プレビューがあればそれを使う。
fn unreadable(c2f: &[u8]) {
    let set = ok(&tip_file(material(c2f, false)));
    let b = &set.brushes[0];
    assert!(
        b.brush.tip.image.is_none() && b.brush.tip.images.is_empty(),
        "丸い筆先"
    );
    assert_eq!(b.brush.base.radius, 15.0, "設定は取り込む");
    assert!(has(b, SutNote::TipMissing));
    assert!(
        has(b, SutNote::ProprietaryImage),
        "独自の形式で読めないことを知らせる"
    );
    // プレビューがあれば、これまでどおりそれを筆先にする（C2F の失敗でプレビューまで失わない）
    let (w, h, _, b) = tip_of(&tip_file(material(c2f, true)));
    assert_eq!((w, h), (2, 2));
    assert!(has(&b, SutNote::PreviewImage));
}

#[test]
fn a_damaged_container_is_refused_and_the_file_still_imports() {
    let layer = Layer::new(40, 30);
    let good = layer.c2f();
    // 対照: 壊す前は読める
    tip_of(&tip_file(material(&good, false)));

    // 先頭の印が違う
    let mut bad = good.clone();
    bad[1] = b'X';
    unreadable(&bad);

    // CRC が合わない（中身の 1 バイト）
    let mut bad = good.clone();
    let at = bad.len() / 2;
    bad[at] ^= 0x55;
    unreadable(&bad);

    // 途中で切れている
    unreadable(&good[..good.len() - 7]);
    unreadable(&good[..good.len() / 2]);
    unreadable(&good[..11]);

    // 知らない種類の塊（CRC は合っている）
    let unknown = c2f::container(&[
        c2f::chunk(b"HEAD", b""),
        c2f::chunk(b"dATA", &c2f::hidden_body()),
        c2f::chunk(b"xTRA", b"?"),
        c2f::chunk(b"dATA", &c2f::plain_body(&layer.pager_pages())),
        c2f::chunk(b"TAIL", b""),
    ]);
    unreadable(&unknown);

    // HEAD が無い・TAIL が無い・dATA が無い
    unreadable(&c2f::container(&[
        c2f::chunk(b"dATA", &c2f::plain_body(&layer.pager_pages())),
        c2f::chunk(b"TAIL", b""),
    ]));
    unreadable(&c2f::container(&[
        c2f::chunk(b"HEAD", b""),
        c2f::chunk(b"dATA", &c2f::hidden_body()),
        c2f::chunk(b"dATA", &c2f::plain_body(&layer.pager_pages())),
    ]));
    unreadable(&c2f::container(&[
        c2f::chunk(b"HEAD", b""),
        c2f::chunk(b"TAIL", b""),
    ]));

    // 塊の数が多すぎる
    let mut many = vec![c2f::chunk(b"HEAD", b"")];
    many.extend((0..100).map(|_| c2f::chunk(b"dATA", &c2f::hidden_body())));
    many.push(c2f::chunk(b"TAIL", b""));
    unreadable(&c2f::container(&many));
}

#[test]
fn a_database_whose_tables_are_in_the_unreadable_part_is_refused() {
    let layer = Layer::new(40, 30);
    // 読める側の塊だけで、読めない側の塊が無いと、ページ番号が 5 ずれて表の根が合わない
    let only_plain = c2f::container(&[
        c2f::chunk(b"HEAD", b""),
        c2f::chunk(b"dATA", &c2f::plain_body(&layer.pager_pages())),
        c2f::chunk(b"TAIL", b""),
    ]);
    unreadable(&only_plain);
    // 読めない側が 6 ページあると、番号がずれる（読めない側のページ数は塊の長さから数える）
    let mut hidden = c2f::hidden_body();
    hidden.extend(std::iter::repeat_n(0xA5u8, c2f::PAGE));
    unreadable(&c2f::file(&hidden, &c2f::plain_body(&layer.pager_pages())));
    // 読めない側が短すぎる（1 ページに満たない）
    unreadable(&c2f::file(
        &[1, 0, 9, 9],
        &c2f::plain_body(&layer.pager_pages()),
    ));
    // 読める側の塊がページの大きさの倍数でない
    let mut body = c2f::plain_body(&layer.pager_pages());
    body.truncate(body.len() - 5);
    unreadable(&c2f::file(&c2f::hidden_body(), &body));
}

#[test]
fn layers_that_are_not_exactly_one_are_refused() {
    let mut two = Layer::new(30, 30);
    two.layers = 2;
    unreadable(&two.c2f());
    let mut none = Layer::new(30, 30);
    none.layers = 0;
    unreadable(&none.c2f());
}

#[test]
fn faces_the_reader_does_not_know_are_refused() {
    // 色の面（5 面）
    let mut color = Layer::new(30, 30);
    color.planes = 5;
    unreadable(&color.c2f());
    // 1 辺が上限を超える
    let mut large = Layer::new(30, 30);
    large.width = 3000;
    large.height = 4;
    large.values = vec![1; 3000 * 4];
    unreadable(&large.c2f());
}

#[test]
fn an_empty_face_is_not_an_image() {
    let mut layer = Layer::new(30, 30);
    layer.all_empty = true;
    unreadable(&layer.c2f());
}

#[test]
fn broken_tile_data_is_refused() {
    let layer = Layer::new(30, 30);
    let pages = layer.pager_pages();
    let flat = c2f::plain_body(&pages);
    // zlib の中身の 1 バイトを書き換える（CRC は組み直す）。Adler が合わず断られる
    let find = |bytes: &[u8]| flat.windows(2).position(|w| w == bytes);
    let at = find(&[0x78, 0x9c]).expect("zlib の始まり") + 12;
    let mut bad = flat.clone();
    bad[at] ^= 0xff;
    unreadable(&c2f::file(&c2f::hidden_body(), &bad));
}

#[test]
fn a_tree_that_loops_back_on_itself_is_refused() {
    let layer = Layer::new(30, 30);
    let pages = layer.pager_pages();
    let interior: Vec<usize> = (0..pages.len()).filter(|&i| pages[i][0] == 0x05).collect();
    assert!(!interior.is_empty(), "対照: 内部ページがある");
    // 内部ページの子（右の子と、各セルの子）をすべて自分のページ番号にする
    let mut looped = pages.clone();
    for &i in &interior {
        let me = (i + c2f::HIDDEN_PAGES + 1) as u32;
        looped[i][8..12].copy_from_slice(&me.to_be_bytes());
        let cells = u16::from_be_bytes([looped[i][3], looped[i][4]]) as usize;
        for c in 0..cells {
            let at = u16::from_be_bytes([looped[i][12 + 2 * c], looped[i][13 + 2 * c]]) as usize;
            looped[i][at..at + 4].copy_from_slice(&me.to_be_bytes());
        }
    }
    unreadable(&c2f::file(&c2f::hidden_body(), &c2f::plain_body(&looped)));
    // 子が存在しないページ（0・範囲の外）
    for target in [0u32, 99_999] {
        let mut dangling = pages.clone();
        for &i in &interior {
            dangling[i][8..12].copy_from_slice(&target.to_be_bytes());
        }
        unreadable(&c2f::file(&c2f::hidden_body(), &c2f::plain_body(&dangling)));
    }
    // 葉でも内部でもないページ（0x02 は索引の内部ページ）を根にする
    let mut other = pages.clone();
    for &i in &interior {
        other[i][0] = 0x02;
    }
    unreadable(&c2f::file(&c2f::hidden_body(), &c2f::plain_body(&other)));
}

// ---------------- 上限と、壊れ方ごとの断り ----------------
//
// 断りの理由そのもの（`Refusal`）は src/brushes/sut/c2f.rs の単体試験が確かめる（同じ組み立てを使う）。ここは、ファイルの組み立てごとに
// 取り込み全体が成功して、素材だけが読めない扱い（丸い筆先・プレビューへの切り替え）になること、上限の内側は読めること。

const MIB: usize = 1024 * 1024;

#[test]
fn a_tree_nested_beyond_the_depth_limit_is_refused_and_the_file_still_imports() {
    // 内部ページが 8 段までは読める（葉が深さ 8）。9 段で断る
    let mut layer = Layer::new(40, 30);
    layer.extra_depth = 7;
    let (w, h, _, b) = tip_of(&tip_file(material(&layer.c2f(), false)));
    assert_eq!((w, h), (40, 30));
    no_image_notes(&b);
    layer.extra_depth = 8;
    unreadable(&layer.c2f());
}

#[test]
fn rows_beyond_the_row_limit_or_the_total_limit_are_refused_and_the_file_still_imports() {
    // 小さな C2F が大きな行を名乗る。1 行 64 MiB の上限（内側は読める）
    let mut layer = Layer::new(40, 30);
    layer.claims = vec![60 * MIB];
    let (w, h, _, _) = tip_of(&tip_file(material(&layer.c2f(), false)));
    assert_eq!((w, h), (40, 30));
    layer.claims = vec![64 * MIB];
    assert!(layer.c2f().len() < MIB);
    unreadable(&layer.c2f());
    // 1 行は上限の内でも、組み立てる行の合計が 256 MiB を超える
    layer.claims = vec![56 * MIB; 5];
    unreadable(&layer.c2f());
}

#[test]
fn tables_that_cannot_be_told_apart_are_refused_and_the_file_still_imports() {
    // 同じ名前で根の違う表
    let mut layer = Layer::new(40, 30);
    layer.second_layer_table = true;
    unreadable(&layer.c2f());
    // 同じ行の写しは同じ表
    let mut layer = Layer::new(40, 30);
    layer.repeated_layer_table = true;
    let (w, h, _, _) = tip_of(&tip_file(material(&layer.c2f(), false)));
    assert_eq!((w, h), (40, 30));
    // 文字の形（UTF-8 と UTF-16LE）が表の定義の行どうしで違う
    for utf16 in [false, true] {
        let mut layer = Layer::new(40, 30);
        layer.utf16 = utf16;
        layer.mixed_text = true;
        unreadable(&layer.c2f());
    }
}

#[test]
fn tile_blocks_that_do_not_add_up_are_refused_and_the_file_still_imports() {
    let control = Layer::new(300, 20);
    let tiles = c2f::tiles_of(300, 20, &control.values);
    let good = |i: usize| c2f::RawBlock::new(i as u32, tiles[i].as_deref());
    let with = |blocks: &[Vec<u8>]| {
        let mut layer = Layer::new(300, 20);
        layer.blocks = Some(c2f::assemble_blocks(blocks));
        layer.c2f()
    };
    // 対照: 正しい並びは読める
    let (w, h, alpha, _) = tip_of(&tip_file(material(
        &with(&[good(0).build(), good(1).build()]),
        false,
    )));
    assert_eq!((w, h), (300, 20));
    assert_eq!(alpha, tip_alpha(300, 20, &control.values));
    // 番号が重なる・タイルが足りない・番号が範囲の外
    unreadable(&with(&[good(0).build(), good(0).build()]));
    unreadable(&with(&[good(0).build()]));
    unreadable(&with(&[
        good(0).build(),
        c2f::RawBlock::new(2, tiles[1].as_deref()).build(),
    ]));
    // 大きさが 256×256 でない・長さの欄が zlib の長さ + 4 でない・zlib が 65536 バイトに展開されない
    let mut small = good(1);
    small.tile_w = 128;
    unreadable(&with(&[good(0).build(), small.build()]));
    let mut stored = good(1);
    stored.stored_delta = 1;
    unreadable(&with(&[good(0).build(), stored.build()]));
    let mut short = good(1);
    short.stream = Some(c2f::zlib(&vec![1u8; 65535]));
    unreadable(&with(&[good(0).build(), short.build()]));
    // 面の記述のタイルの大きさが 256 でない
    let mut layer = Layer::new(40, 30);
    layer.tile_param = 128;
    unreadable(&layer.c2f());
}

/// 同じ名前の素材を `count` 個並べた file。1 つ目のブラシは先頭の素材、2 つ目のブラシは最後の素材を筆先にする。
fn many_materials(c2f_bytes: &[u8], count: usize) -> Vec<u8> {
    let name = |i: usize| format!("mat_{i:03}");
    let mut builder = SutBuilder::new();
    for i in 0..count {
        let mut files: Vec<(&str, &[u8])> = vec![("data/material.layer", c2f_bytes)];
        files.push(("icedata/layerData.xml", b"<infolist/>"));
        builder = builder.material(Some(&name(i)), tar(&files));
    }
    let tip = |n: String| {
        [
            ("BrushSize", real(30.0)),
            ("BrushUsePatternImage", int(1)),
            (
                "BrushPatternImageArray",
                blob(refs(&[[&format!("C:\\mats\\{n}.png"), "cat/aaaa", &n]])),
            ),
        ]
    };
    builder
        .brush("First", 1, &tip(name(0)))
        .brush("Last", 2, &tip(name(count - 1)))
        .build()
}

#[test]
fn the_work_on_the_layers_is_limited_for_the_whole_file_not_for_each_material() {
    // 1 つずつなら上限の内の C2F（組み立てる行が 112 MiB）を 256 個並べる。256 MiB を超えた先の素材は、画像が無いものとして扱う
    // （先頭の素材は読める。取り込み全体は失敗しない）
    let mut layer = Layer::new(40, 30);
    layer.claims = vec![56 * MIB, 56 * MIB];
    let set = ok(&many_materials(&layer.c2f(), 256));
    assert_eq!(set.brushes.len(), 2);
    let (first, last) = (&set.brushes[0], &set.brushes[1]);
    let tip = first.brush.tip.image.as_ref().expect("先頭の素材は読める");
    assert_eq!((tip.width(), tip.height()), (40, 30));
    no_image_notes(first);
    assert!(last.brush.tip.image.is_none() && last.brush.tip.images.is_empty());
    assert!(has(last, SutNote::TipMissing) && has(last, SutNote::ProprietaryImage));
}

#[test]
fn the_pixels_on_the_layers_are_limited_for_the_whole_file_not_for_each_material() {
    // 2048×2048（1 辺の上限いっぱい）の素材を 70 個。1 つずつは上限の内でも、展開する画素の合計（256 Mi）には 64 個ぶんしか入らない
    let mut layer = Layer::new(2048, 2048);
    layer.values = vec![7; 2048 * 2048];
    let set = ok(&many_materials(&layer.c2f(), 70));
    let (first, last) = (&set.brushes[0], &set.brushes[1]);
    let tip = first.brush.tip.image.as_ref().expect("先頭の素材は読める");
    assert_eq!((tip.width(), tip.height()), (2048, 2048));
    assert!(last.brush.tip.image.is_none());
    assert!(has(last, SutNote::TipMissing) && has(last, SutNote::ProprietaryImage));
}

// ---------------- どこを壊しても全体は取り込める ----------------

#[test]
fn any_damage_to_the_pages_leaves_the_file_importable() {
    // 塊の CRC を組み直して、読み手の中の検査（ページ・B 木・行・面の記述・タイル）に届くようにする。どんな壊れ方でも、
    // ファイル全体は取り込めて（パニックも失敗もしない）、素材だけが読めないか、別の読めた画像になる
    let layer = Layer::new(300, 20);
    let flat = c2f::plain_body(&layer.pager_pages());
    let hidden = c2f::hidden_body();
    let mut cases = 0usize;
    let stride = (flat.len() / 1000).max(1);
    for at in (0..flat.len()).step_by(stride) {
        for mutate in [0x00u8, 0xff, 0x55] {
            let mut bad = flat.clone();
            bad[at] = if mutate == 0x55 {
                bad[at] ^ 0x55
            } else {
                mutate
            };
            let set = ok(&tip_file(material(&c2f::file(&hidden, &bad), false)));
            assert_eq!(set.brushes.len(), 1, "位置 {at}");
            cases += 1;
        }
    }
    // 切り詰め（ページの途中・ページの境）
    for len in (2..flat.len()).step_by(stride * 3 + 1) {
        let set = ok(&tip_file(material(
            &c2f::file(&hidden, &flat[..len]),
            false,
        )));
        assert_eq!(set.brushes.len(), 1, "長さ {len}");
        cases += 1;
    }
    assert!(cases > 1000, "{cases} 通り");
    println!("{cases} 通り");
}

// ---------------- 本物のファイル（試験の外） ----------------

/// 環境変数 `YOLU_REAL_SUT` の `.sut`（本物の CLIP STUDIO の書き出し。リポジトリには入れない）を読み、素材の画像が原寸で読めて、
/// 「独自の形式で読めない」と言われないことを確かめる。名前や画像は表示しない。画素の意味（黒の不透明度・向き）は、素材の中の CLIP STUDIO の見本との
/// 突き合わせで確かめる（`src/brushes/sut/c2f.rs` の `real::the_pixels_are_the_black_ink_opacity_the_material_thumbnail_shows`）。
#[test]
#[ignore = "本物の .sut が要る（YOLU_REAL_SUT）"]
fn a_real_sut_reads_its_full_size_images() {
    let Ok(path) = std::env::var("YOLU_REAL_SUT") else {
        panic!("YOLU_REAL_SUT に .sut のパスを入れる");
    };
    let bytes = std::fs::read(path).expect("読める");
    let set = ok(&bytes);
    let mut images = 0;
    for b in &set.brushes {
        assert!(
            !has(b, SutNote::ProprietaryImage),
            "独自の形式で読めないと言われた"
        );
        assert!(!has(b, SutNote::PreviewImage), "プレビューにした");
        for tip in b.brush.tip.image.iter().chain(b.brush.tip.images.iter()) {
            assert!(tip.width() >= 64 && tip.height() >= 64, "原寸のはず");
            let alpha = tip.alpha();
            assert!(alpha.iter().any(|&v| v != alpha[0]), "平らでない");
            images += 1;
        }
        if let Some(t) = &b.brush.texture {
            let alpha = t.image.alpha();
            assert!(t.image.width() >= 64 && alpha.iter().any(|&v| v != alpha[0]));
            images += 1;
        }
    }
    assert!(set.notes.iter().all(|n| !matches!(
        n,
        Unrepresented::ClipStudio(SutNote::MaterialsUnreadable(_))
    )));
    assert!(images > 0, "画像が 1 つも読めない");
    println!("読めた画像: {images}（ブラシ {}）", set.brushes.len());
}
