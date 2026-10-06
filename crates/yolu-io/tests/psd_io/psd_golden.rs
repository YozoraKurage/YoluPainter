//! PSD の写しを、実 C# Core の `PsdBridge.Export`・`Import` と `PsdCodec` に通した正解（`tests/golden/psd/`、`tools/csharp-golden/run.sh psd` で作る）と照らす。
//! - 書き出し事例: 同じ台本の文書を core で組み、`from_core` → `write` のバイト列が C# の書き出しと全バイト一致する。
//! - その PSD を読み、core にした文書の中身（層の並び・属性・マスク・調整・塗りつぶし・画素・合成）が C# の取り込みと同じ。
//! - 取り込み事例: C# が組んだ PSD（マスクの既定 0・画布からはみ出す矩形・区切りの ID の無いグループ・入れ子）を core にした中身が C# の取り込みと同じ。
//! - 断る事例: C# が断る書き出しを Rust も断る。
//!
//! 台本は `tools/csharp-golden/PsdBridgeGolden.cs` と同じ（層の ID は作った順に 100 + n、区切りの ID は 300 + n）。
use std::fmt::Write as _;
use std::path::PathBuf;
use yolu_core::{
    AdjustmentSettings, BlendMode as CoreBlend, Channel, ChannelBlend, Document, LayerId,
    LayerKind, LayerLocks, Rgba8,
};
use yolu_io::psd::{self, Document as Psd, Limits, Refusal};

const W: u32 = 24;
const H: u32 = 16;

fn golden(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden/psd")
        .join(name)
}
fn read(name: &str) -> Vec<u8> {
    std::fs::read(golden(name)).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn gradient(x: u32, y: u32) -> Rgba8 {
    Rgba8::new(
        (x * 10 + 15) as u8,
        (y * 14 + 20) as u8,
        (200 - x * 5) as u8,
        255,
    )
}
fn soft(x: u32, y: u32) -> Rgba8 {
    Rgba8::new(
        (255 - x * 9) as u8,
        (x * 7 + y * 5) as u8,
        (y * 13 + 40) as u8,
        if x < 3 { 0 } else { (60 + x * 8 + y * 3) as u8 },
    )
}
fn exact() -> [AdjustmentSettings; 3] {
    [
        AdjustmentSettings::invert(),
        AdjustmentSettings::levels(
            20.0 / 255.0,
            230.0 / 255.0,
            1.37,
            10.0 / 255.0,
            240.0 / 255.0,
        )
        .unwrap(),
        AdjustmentSettings::hue_saturation(-73.0, 0.42, -0.18).unwrap(),
    ]
}

/// 層 n 番目（作った順、1 から）の ID。上位 32 bit が PSD の層 ID（100 + n）、続く 32 bit が区切りの ID（300 + n、Guid の Data2・Data3）、
/// 残りは固定（Guid の 8〜15 バイト目）。
fn id(n: usize) -> LayerId {
    let divider = (300 + n) as u32;
    LayerId(
        (((100 + n) as u128) << 96)
            | (u128::from(divider & 0xffff) << 80)
            | (u128::from(divider >> 16) << 64)
            | 0x0102_0304_0506_0708,
    )
}

/// C# の `Doc`（台本の部品）と同じ。ID は最後に、作った順の番号で配る。
struct B {
    d: Document,
    made: Vec<LayerId>,
}
impl B {
    fn new() -> B {
        B {
            d: Document::with_tile_size(W, H, 8).unwrap(),
            made: Vec::new(),
        }
    }
    fn made(&mut self, id: LayerId) -> LayerId {
        self.made.push(id);
        id
    }
    fn raster(&mut self, name: &str, px: fn(u32, u32) -> Rgba8) -> LayerId {
        let l = self.d.add_layer(name).unwrap();
        for y in 0..H {
            for x in 0..W {
                let c = px(x, y);
                if c != Rgba8::TRANSPARENT {
                    self.d.set_pixel(l, x, y, c).unwrap();
                }
            }
        }
        self.made(l)
    }
    fn fill(&mut self, name: &str, c: Rgba8) -> LayerId {
        let l = self
            .d
            .add_fill_layer(name, &[(Channel::Color, c)], None)
            .unwrap();
        self.made(l)
    }
    fn adjust(&mut self, name: &str, s: AdjustmentSettings) -> LayerId {
        let l = self.d.add_adjustment_layer(name, s, None, None).unwrap();
        self.made(l)
    }
    fn group(&mut self, name: &str, members: &[LayerId]) -> LayerId {
        let l = self.d.group_layers(members, name).unwrap();
        self.made(l)
    }
    fn empty_group(&mut self, name: &str) -> LayerId {
        let l = self.d.add_group(name, None).unwrap();
        self.made(l)
    }
    fn opacity(&mut self, l: LayerId, byte: u8) {
        self.d
            .set_layer_opacity(l, f64::from(byte) / 255.0, false)
            .unwrap()
    }
    fn mode(&mut self, l: LayerId, m: CoreBlend) {
        self.d.set_layer_blend_mode(l, m).unwrap()
    }
    fn clip(&mut self, l: LayerId) {
        self.d.set_layer_clipping(l, true).unwrap()
    }
    fn hide(&mut self, l: LayerId) {
        self.d.set_layer_visible(l, false).unwrap()
    }
    fn hide_row(&mut self, l: LayerId, y: u32, amount: u8) {
        self.d.add_layer_mask(l).unwrap();
        for x in 0..W {
            self.d.set_mask_pixel(l, x, y, amount).unwrap();
        }
    }
    fn finish(self) -> Document {
        let ids: Vec<LayerId> = self
            .d
            .layers()
            .iter()
            .map(|l| id(self.made.iter().position(|m| *m == l.id()).unwrap() + 1))
            .collect();
        let doc_id = self.d.id();
        self.d.with_persistent_ids(doc_id, &ids).unwrap()
    }
}

fn plain() -> Document {
    let mut s = B::new();
    s.raster("bg", gradient);
    let a = s.raster("soft", soft);
    s.mode(a, CoreBlend::Multiply);
    s.opacity(a, 183);
    s.clip(a);
    let h = s.raster("hidden", gradient);
    s.hide(h);
    let o = s.raster("overlay", soft);
    s.mode(o, CoreBlend::SoftLight);
    s.opacity(o, 99);
    s.finish()
}
fn masks() -> Document {
    let mut s = B::new();
    let a = s.raster("partial", gradient);
    s.hide_row(a, 5, 255);
    s.d.set_layer_mask_enabled(a, false).unwrap();
    s.d.set_layer_mask_density(a, 40.0 / 255.0, false).unwrap();
    let b = s.raster("mostly hidden", soft);
    s.d.add_layer_mask(b).unwrap();
    for y in 0..H {
        for x in 0..W {
            if !((4..9).contains(&x) && (2..6).contains(&y)) {
                s.d.set_mask_pixel(b, x, y, 255).unwrap();
            }
        }
    }
    let c = s.raster("neutral", gradient);
    s.d.add_layer_mask(c).unwrap();
    let e = s.raster("graded", soft);
    s.d.add_layer_mask(e).unwrap();
    for y in 0..H {
        for x in 0..W {
            if (x + y) % 5 == 0 {
                s.d.set_mask_pixel(e, x, y, (x * 11 + y * 7) as u8).unwrap();
            }
        }
    }
    s.d.set_layer_mask_density(e, 160.0 / 255.0, false).unwrap();
    let f = s.raster("all hidden", gradient);
    s.d.add_layer_mask(f).unwrap();
    for y in 0..H {
        for x in 0..W {
            s.d.set_mask_pixel(f, x, y, 255).unwrap();
        }
    }
    s.finish()
}
fn fills() -> Document {
    let mut s = B::new();
    s.raster("bg", gradient);
    let paint = s.fill("paint", Rgba8::new(200, 100, 50, 255));
    s.opacity(paint, 128);
    s.mode(paint, CoreBlend::Multiply);
    s.hide_row(paint, 5, 200);
    s.d.set_layer_mask_density(paint, 160.0 / 255.0, false)
        .unwrap();
    s.raster("shape", soft);
    let clipped = s.fill("clipped fill", Rgba8::new(250, 20, 20, 255));
    s.clip(clipped);
    s.opacity(clipped, 90);
    let hidden = s.fill("hidden", Rgba8::new(1, 2, 3, 255));
    s.hide(hidden);
    let over = s.fill("overlay", Rgba8::new(10, 200, 30, 255));
    s.mode(over, CoreBlend::Overlay);
    s.finish()
}
fn adjustments() -> Document {
    let mut s = B::new();
    s.raster("bg", gradient);
    for (k, settings) in exact().into_iter().enumerate() {
        let plain = s.adjust(&format!("plain{k}"), settings.clone());
        s.opacity(plain, 160);
        s.hide_row(plain, 5, 200);
        s.raster(&format!("shape{k}"), soft);
        let clipped = s.adjust(&format!("clipped{k}"), settings);
        s.clip(clipped);
        s.mode(clipped, CoreBlend::Overlay);
    }
    let hidden = s.adjust("hidden", AdjustmentSettings::invert());
    s.hide(hidden);
    s.finish()
}
fn groups() -> Document {
    let mut s = B::new();
    let x = exact();
    s.raster("bg", gradient);
    let a = s.raster("a", soft);
    let l2 = s.adjust("levels", x[1].clone());
    let pass = s.group("pass", &[a, l2]);
    s.opacity(pass, 190);
    s.hide_row(pass, 9, 90);
    let b = s.raster("b", soft);
    let h = s.adjust("hue", x[2].clone());
    let inv = s.adjust("invert", x[0].clone());
    s.clip(inv);
    s.opacity(inv, 100);
    let iso = s.group("isolated", &[b, h, inv]);
    s.mode(iso, CoreBlend::Multiply);
    let f = s.fill("fill in group", Rgba8::new(30, 90, 160, 255));
    let r2 = s.raster("clipped in group", soft);
    s.clip(r2);
    s.group("fills", &[f, r2]);
    let inner = s.raster("inner", gradient);
    let gi = s.group("inner group", &[inner]);
    let go = s.group("outer group", &[gi]);
    s.mode(go, CoreBlend::Screen);
    s.opacity(go, 200);
    let empty = s.empty_group("empty");
    s.hide(empty);
    let hidden_leaf = s.raster("in hidden group", gradient);
    let hg = s.group("hidden group", &[hidden_leaf]);
    s.hide(hg);
    s.finish()
}
fn locks() -> Document {
    let mut s = B::new();
    let a = s.raster("transparency", gradient);
    let f = s.fill("pixels", Rgba8::new(1, 2, 3, 255));
    let j = s.adjust("position", AdjustmentSettings::invert());
    let b = s.raster("all", soft);
    let inner = s.raster("in locked folder", gradient);
    let g = s.group("locked folder", &[inner]);
    let everything = s.raster("everything", soft);
    let plain = s.raster("plain", gradient);
    let set = |s: &mut B, l, locks| s.d.set_layer_locks(l, locks).unwrap();
    set(&mut s, a, LayerLocks::TRANSPARENCY);
    set(&mut s, f, LayerLocks::PIXELS);
    set(&mut s, j, LayerLocks::POSITION);
    set(&mut s, b, LayerLocks::ALL);
    set(&mut s, g, LayerLocks::ALL | LayerLocks::PIXELS);
    set(
        &mut s,
        everything,
        LayerLocks::ALL | LayerLocks::TRANSPARENCY | LayerLocks::PIXELS | LayerLocks::POSITION,
    );
    set(
        &mut s,
        plain,
        LayerLocks::TRANSPARENCY | LayerLocks::POSITION,
    );
    s.finish()
}
fn channel_blends() -> Document {
    let mut s = B::new();
    s.raster("bg", gradient);
    let a = s.raster("colour override", soft);
    s.opacity(a, 200);
    let multiply = ChannelBlend::new(Some(CoreBlend::Multiply), Some(100.0 / 255.0));
    s.d.set_channel_blend(a, Channel::Color, multiply, false)
        .unwrap();
    s.d.set_channel_blend(a, Channel::Roughness, multiply, false)
        .unwrap();
    // グループと調整は、効くチャンネルの全部（グループは標準の 6 つ、調整は有効なチャンネル）が Color と同じ値のときだけ書ける
    let inner = s.raster("inner", soft);
    let g = s.group("group", &[inner]);
    s.opacity(g, 255);
    for channel in Channel::ALL {
        s.d.set_channel_blend(
            g,
            channel,
            ChannelBlend::new(Some(CoreBlend::Screen), Some(128.0 / 255.0)),
            false,
        )
        .unwrap();
    }
    let adj = s.adjust("adjust", AdjustmentSettings::invert());
    for channel in s.d.layer(adj).unwrap().enabled_channels() {
        s.d.set_channel_opacity(adj, channel, Some(150.0 / 255.0), false)
            .unwrap();
    }
    s.finish()
}

fn fnv(bytes: impl IntoIterator<Item = u8>) -> u64 {
    bytes.into_iter().fold(0xcbf29ce484222325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
    })
}
/// C# の `Snapshot` と同じ文字列。
fn snapshot(d: &Document, diagnostics: &str) -> String {
    let mut s = String::new();
    writeln!(s, "canvas {} {}", d.width(), d.height()).unwrap();
    writeln!(s, "diagnostics {diagnostics}").unwrap();
    for (i, l) in d.layers().iter().enumerate() {
        let parent = l
            .parent()
            .and_then(|p| d.layer_index(p))
            .map_or(-1, |i| i as i64);
        let locks = if l.locks().contains(LayerLocks::ALL) {
            LayerLocks::ALL.bits()
        } else {
            l.locks().bits()
        };
        write!(
            s,
            "{i} name={} kind={:?} parent={parent} id={} visible={} opacity={} mode={} clip={} locks={locks}",
            l.name(),
            l.kind(),
            &format!("{:032x}", l.id().0)[..16],
            u8::from(l.visible()),
            (l.opacity() * 255.0).round_ties_even(),
            l.blend_mode().name(),
            u8::from(l.clipping()),
        )
        .unwrap();
        match l.mask() {
            None => s.push_str(" mask=-"),
            Some(m) => {
                let bytes = m.surface().to_canvas_bytes();
                write!(
                    s,
                    " mask={}/{}/{}/{:016x}",
                    u8::from(m.enabled()),
                    u8::from(m.inverted()),
                    (m.density() * 255.0).round_ties_even(),
                    fnv(bytes.as_chunks::<4>().0.iter().map(|p| p[3]))
                )
                .unwrap()
            }
        }
        match l.kind() {
            LayerKind::Raster => write!(
                s,
                " pixels={:016x}",
                fnv(l.surface(Channel::Color).unwrap().to_canvas_bytes())
            )
            .unwrap(),
            LayerKind::Fill => {
                let c = l.fill_value(Channel::Color).unwrap();
                write!(s, " fill={},{},{},{}", c.r, c.g, c.b, c.a).unwrap()
            }
            LayerKind::Adjustment => {
                let a = l.adjustment().unwrap();
                let bits: Vec<String> = [
                    a.input_black(),
                    a.input_white(),
                    a.gamma(),
                    a.output_black(),
                    a.output_white(),
                    a.hue(),
                    a.saturation(),
                    a.lightness(),
                ]
                .iter()
                .map(|v| format!("{:x}", v.to_bits()))
                .collect();
                write!(s, " adjustment={:?}/{}", a.kind(), bits.join("/")).unwrap()
            }
            LayerKind::Group => {}
        }
        if l.kind() != LayerKind::Group {
            let channels: Vec<String> = Channel::ALL
                .iter()
                .filter(|c| l.is_channel_enabled(**c))
                .map(|c| c.index().to_string())
                .collect();
            write!(s, " channels={}", channels.join(",")).unwrap();
        }
        s.push('\n');
    }
    writeln!(
        s,
        "composite {:016x}",
        fnv(d.composite(d.bounds()).unwrap())
    )
    .unwrap();
    s
}
fn codes(r: &psd::ReadResult) -> String {
    let mut codes: Vec<&str> = r.diagnostics().iter().map(|d| d.code.as_str()).collect();
    codes.sort();
    codes.join(",")
}
/// バイト列を読んで core にした文書の中身（C# の `Snapshot(Import(Read(bytes)), Codes(read))`）。
fn imported_snapshot(bytes: &[u8]) -> String {
    let read = psd::read(bytes, &Limits::default()).unwrap();
    assert_eq!(
        read.mode(),
        psd::CompatibilityMode::EditableRaster,
        "{:?}",
        read.diagnostics()
    );
    snapshot(&read.to_core().unwrap(), &codes(&read))
}

/// 事例の名前と、同じ台本の文書を組む関数。
type Case = (&'static str, fn() -> Document);

/// 書き出して、取り込み直す事例（C# の書き出しとバイト列が一致する）。
const EXPORT_CASES: [Case; 7] = [
    ("plain", plain),
    ("masks", masks),
    ("fills", fills),
    ("adjustments", adjustments),
    ("groups", groups),
    ("locks", locks),
    ("channel_blends", channel_blends),
];
/// C# が組んだ PSD を取り込む事例。
const IMPORT_CASES: [&str; 2] = ["import_masks", "import_groups"];

/// 断る事例: 名前・文書・Rust の理由・断る層の名前・C# の断りの文に入っているはずの語。
struct Refused {
    name: &'static str,
    doc: Document,
    refusal: Refusal,
    layer: &'static str,
    csharp: &'static str,
}
fn refused_cases() -> Vec<Refused> {
    let invert_mask = || {
        let mut s = B::new();
        let a = s.raster("inv", gradient);
        s.d.add_layer_mask(a).unwrap();
        s.d.set_layer_mask_inverted(a, true).unwrap();
        s.finish()
    };
    let clipped_group = || {
        let mut s = B::new();
        s.raster("bg", gradient);
        let c = s.raster("inner", soft);
        let g = s.group("clipped group", &[c]);
        s.clip(g);
        s.finish()
    };
    let translucent = || {
        let mut s = B::new();
        s.raster("bg", gradient);
        s.fill("glass", Rgba8::new(1, 2, 3, 128));
        s.finish()
    };
    let adjusted = |settings: AdjustmentSettings| {
        let mut s = B::new();
        s.raster("bg", gradient);
        s.adjust("between", settings);
        s.finish()
    };
    let case = |name, doc, refusal, layer, csharp| Refused {
        name,
        doc,
        refusal,
        layer,
        csharp,
    };
    vec![
        case(
            "refused_inverted_mask",
            invert_mask(),
            Refusal::InvertedMask,
            "inv",
            "mask inversion",
        ),
        case(
            "refused_clipped_group",
            clipped_group(),
            Refusal::ClippedGroup,
            "clipped group",
            "is clipped",
        ),
        case(
            "refused_translucent_fill",
            translucent(),
            Refusal::FillTranslucent,
            "glass",
            "is opaque",
        ),
        case(
            "refused_levels_between",
            adjusted(AdjustmentSettings::levels(0.3, 1.0, 1.0, 0.0, 1.0).unwrap()),
            Refusal::LevelsBetweenSteps,
            "between",
            "Levels setting is between them",
        ),
        case(
            "refused_gamma_between",
            adjusted(AdjustmentSettings::levels(0.0, 1.0, 1.234, 0.0, 1.0).unwrap()),
            Refusal::LevelsBetweenSteps,
            "between",
            "Levels setting is between them",
        ),
        case(
            "refused_hue_between",
            adjusted(AdjustmentSettings::hue_saturation(10.5, 0.0, 0.0).unwrap()),
            Refusal::HueSaturationBetweenSteps,
            "between",
            "Hue/Saturation stores whole degrees",
        ),
        case(
            "refused_saturation_between",
            adjusted(AdjustmentSettings::hue_saturation(0.0, 0.333, 0.0).unwrap()),
            Refusal::HueSaturationBetweenSteps,
            "between",
            "Hue/Saturation stores whole degrees",
        ),
        case(
            "refused_levels_range",
            adjusted(AdjustmentSettings::levels(0.0, 1.0 / 255.0, 1.0, 0.0, 1.0).unwrap()),
            Refusal::LevelsRange,
            "between",
            "input black of 0-253",
        ),
    ]
}

#[test]
fn index_lists_exactly_the_cases_this_test_checks() {
    let index = String::from_utf8(read("index.txt")).unwrap();
    assert!(index.starts_with("# YoluPainter "), "出どころの行");
    let listed: std::collections::BTreeSet<&str> = index
        .lines()
        .filter_map(|l| l.strip_prefix("case "))
        .filter_map(|l| l.split(' ').next())
        .collect();
    let checked: std::collections::BTreeSet<&str> = EXPORT_CASES
        .iter()
        .map(|(name, _)| *name)
        .chain(IMPORT_CASES)
        .chain(refused_cases().iter().map(|c| c.name))
        .collect();
    assert_eq!(listed, checked, "index.txt の事例と、この試験が回す事例");
    assert_eq!(
        index.lines().filter(|l| l.starts_with("case ")).count(),
        checked.len(),
        "事例が重ならない"
    );
}

#[test]
fn rust_writes_the_same_bytes_as_csharp_and_reads_the_same_document() {
    for (name, build) in EXPORT_CASES {
        let core = build();
        let projected = Psd::from_core(&core).unwrap_or_else(|e| panic!("{name}: {e}"));
        let bytes = psd::write(&projected, &Limits::default()).unwrap();
        let expected = read(&format!("{name}.psd"));
        if bytes != expected {
            let at = bytes.iter().zip(&expected).position(|(a, b)| a != b);
            panic!(
                "{name}: C# の書き出しとバイトが違う（最初の違い {at:?}、長さ {} / {}）",
                bytes.len(),
                expected.len()
            );
        }
        let want = String::from_utf8(read(&format!("{name}.snap"))).unwrap();
        assert_eq!(
            imported_snapshot(&expected),
            want,
            "{name}: 取り込んだ文書の中身"
        );
    }
}

#[test]
fn csharp_built_psds_with_unusual_masks_and_groups_import_the_same() {
    for name in IMPORT_CASES {
        let bytes = read(&format!("{name}.psd"));
        let want = String::from_utf8(read(&format!("{name}.snap"))).unwrap();
        assert_eq!(imported_snapshot(&bytes), want, "{name}");
        // 原本の書き戻しは同じバイト列
        let r = psd::read(&bytes, &Limits::default()).unwrap();
        assert_eq!(
            psd::write_edited(&r, r.document().unwrap(), &Limits::default()).unwrap(),
            bytes,
            "{name}"
        );
    }
}

#[test]
fn what_csharp_refuses_to_export_rust_refuses_too_for_the_same_reason() {
    for case in refused_cases() {
        let name = case.name;
        // C# が断った理由（保存してある文）と、事例が作りたかった理由が合っている
        let csharp = String::from_utf8(read(&format!("{name}.refused"))).unwrap();
        assert!(csharp.contains(case.csharp), "{name}: {csharp}");
        // Rust は同じ理由で、同じ層について断る（ほかの検査が代わりに断ったのでは通らない）
        let blockers = psd::export_blockers(&case.doc);
        assert_eq!(
            blockers,
            vec![psd::Blocker {
                layer: case.layer.into(),
                refusal: case.refusal.clone()
            }],
            "{name}"
        );
        match Psd::from_core(&case.doc).expect_err(name) {
            yolu_io::Error::InvalidData(message) => {
                assert_eq!(message, blockers[0].message(), "{name}")
            }
            other => panic!("{name}: {other}"),
        }
    }
}
