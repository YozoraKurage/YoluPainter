//! C# との照合の入力: 手で組んだ事例（完全な指紋を比べる）と、それを壊した入力（結果と指紋の短縮を比べる）。
//! 生成は決まっている（乱数を使わない）。変えたら `tools/csharp-golden/brushes.sh` で正解を作り直す。
use super::*;
use yolu_io::brushes::FileKind;

pub struct Case {
    /// 1: 手で組んだ事例、0: 壊した入力。
    pub tag: u8,
    pub kind: FileKind,
    pub label: String,
    pub bytes: Vec<u8>,
}

pub fn kind_code(kind: FileKind) -> u8 {
    match kind {
        FileKind::Gbr => 0,
        FileKind::Gih => 1,
        FileKind::Vbr => 2,
        FileKind::Abr => 3,
        FileKind::Pat => 4,
        FileKind::Png => 5,
        // C# の読み手に無い形式（照合の事例には使わない）
        FileKind::Sut => 6,
    }
}

fn leaves_preset() -> Vec<u8> {
    preset(&[
        ("Nm  ", text("Leaves")),
        (
            "Brsh",
            obj(
                "sampledBrush",
                &[
                    ("Dmtr", unit("#Pxl", 60.0)),
                    ("Angl", unit("#Ang", 45.0)),
                    ("Rndn", unit("#Prc", 70.0)),
                    ("Spcn", unit("#Prc", 35.0)),
                    ("sampledData", text(ID)),
                ],
            ),
        ),
        ("useTipDynamics", boolean(true)),
        ("szVr", dynamics(2, 40.0)),
        ("angleDynamics", dynamics(0, 100.0)),
        ("roundnessDynamics", dynamics(0, 20.0)),
        ("useScatter", boolean(true)),
        ("scatterDynamics", dynamics(0, 150.0)),
        ("bothAxes", boolean(true)),
        ("Cnt ", long(3)),
        ("usePaintDynamics", boolean(true)),
        ("opVr", dynamics(2, 10.0)),
        ("prVr", dynamics(1, 30.0)),
        ("useTexture", boolean(true)),
        ("Wtdg", boolean(true)),
    ])
}

fn weird_preset() -> Vec<u8> {
    preset(&[
        ("Nm  ", text("Odd")),
        ("Brsh", obj("mysteryBrush", &[])),
        ("useTipDynamics", boolean(true)),
        ("minimumDiameter", unit("#Prc", 25.0)),
        (
            "szVr",
            obj(
                "brVr",
                &[
                    ("bVTy", long(1)),
                    ("fStp", long(0)),
                    ("jitter", unit("#Prc", 0.0)),
                ],
            ),
        ),
        ("useScatter", boolean(true)),
        ("scatterDynamics", dynamics(0, 50.0)),
        ("countDynamics", dynamics(0, 10.0)),
        ("useColorDynamics", boolean(true)),
        ("clVr", dynamics(2, 0.0)),
        ("colorDynamicsPerTip", boolean(false)),
        ("useDualBrush", boolean(true)),
        (
            "dualBrush",
            obj(
                "dualBrush",
                &[
                    ("Brsh", obj("mysteryBrush", &[])),
                    ("BlnM", enum_value("BlnM", "Dvsn")),
                    ("scatterDynamics", dynamics(0, 30.0)),
                    ("bothAxes", boolean(false)),
                    ("countDynamics", dynamics(0, 10.0)),
                    ("Flip", boolean(true)),
                ],
            ),
        ),
        ("Nose", boolean(true)),
    ])
}

fn clamp_preset() -> Vec<u8> {
    preset(&[
        ("Nm  ", text("Clamp")),
        (
            "Brsh",
            obj(
                "computedBrush",
                &[
                    ("Dmtr", unit("#Pxl", 99999.0)),
                    ("Angl", unit("#Ang", -720.0)),
                    ("Rndn", unit("#Prc", -5.0)),
                    ("Spcn", unit("#Prc", 99999.0)),
                    ("Hrdn", unit("#Prc", 900.0)),
                ],
            ),
        ),
        (
            "toolOptions",
            obj(
                "toolOptions",
                &[("Opct", unit("#Prc", 150.0)), ("flow", unit("#Prc", -10.0))],
            ),
        ),
        ("useScatter", boolean(true)),
        ("scatterDynamics", dynamics(0, 50.0)),
        ("Cnt ", long(1000)),
        ("bothAxes", boolean(true)),
        ("useColorDynamics", boolean(true)),
        ("purity", unit("#Prc", 900.0)),
    ])
}

/// 質感の合わせ方・拡大・反転・各種コントロールを変えたプリセット（模様 "p" を指す）。
fn textured_preset(name: &str, mode: &str, scale: f64, flip: bool, angle_control: i32) -> Vec<u8> {
    preset(&[
        ("Nm  ", text(name)),
        (
            "Brsh",
            obj(
                "sampledBrush",
                &[
                    ("sampledData", text(ID)),
                    ("flipX", boolean(flip)),
                    ("flipY", boolean(false)),
                ],
            ),
        ),
        ("useTipDynamics", boolean(true)),
        ("angleDynamics", dynamics(angle_control, 0.0)),
        ("useTexture", boolean(true)),
        (
            "Txtr",
            obj("Ptrn", &[("Nm  ", text("Paper")), ("Idnt", text("p"))]),
        ),
        ("textureScale", unit("#Prc", scale)),
        ("textureBlendMode", enum_value("BlnM", mode)),
        ("TxtC", boolean(true)),
        ("textureBrightness", unit("#Prc", 10.0)),
        ("textureContrast", unit("#Prc", -10.0)),
        ("textureDepthDynamics", dynamics(1, 20.0)),
    ])
}

fn abr_with(presets: &[Vec<u8>], samples: &[u8], patterns: Option<&[u8]>) -> Vec<u8> {
    let mut sections = vec![
        section("samp", samples),
        section("desc", &desc_body(&[brush_list(presets)])),
    ];
    if let Some(p) = patterns {
        sections.push(section("patt", p));
    }
    abr_v6(&sections)
}

fn bad_desc() -> Vec<u8> {
    let desc = W::new()
        .i32(16)
        .unicode("")
        .key("null")
        .i32(1)
        .key("Brsh")
        .ascii("ObAr")
        .done();
    abr_v6(&[
        section("samp", &samp_record(1, "x", false)),
        section("desc", &desc),
    ])
}

fn hose_header(header: &str, n: usize) -> Vec<u8> {
    gih(header, &cells()[..n])
}

/// 手で組んだ事例（ラベル・種類・バイト列）。
pub fn hand_built() -> Vec<(&'static str, FileKind, Vec<u8>)> {
    use FileKind::*;
    let mut v: Vec<(&'static str, FileKind, Vec<u8>)> = Vec::new();
    // GBR
    v.push((
        "gbr gray",
        Gbr,
        gbr_gray(2, 2, &[255, 0, 0, 128], "Pixel 筆", 50),
    ));
    v.push(("gbr v1", Gbr, gbr(1, 1, &[200], 1, "old", 25, 1)));
    v.push(("gbr v3", Gbr, gbr(2, 1, &[5, 6], 1, "v3", 100, 3)));
    v.push((
        "gbr rgba",
        Gbr,
        gbr(2, 1, &[10, 20, 30, 77, 1, 2, 3, 255], 4, "Test", 25, 2),
    ));
    v.push(("gbr wide", Gbr, gbr_gray(4, 2, &[1; 8], "Test", 100)));
    v.push(("gbr tall", Gbr, gbr_gray(2, 4, &[2; 8], "Test", 100)));
    v.push(("gbr spacing 0", Gbr, gbr_gray(1, 1, &[0], "Test", 0)));
    v.push((
        "gbr spacing huge",
        Gbr,
        gbr_gray(1, 1, &[0], "Test", u32::MAX),
    ));
    v.push(("gbr unnamed", Gbr, gbr_gray(1, 1, &[9], "", 25)));
    v.push((
        "gbr 300 name",
        Gbr,
        gbr_gray(1, 1, &[9], &"n".repeat(300), 25),
    ));
    v.push((
        "gbr large",
        Gbr,
        gbr_gray(
            300,
            200,
            &(0..60000).map(|i| (i % 251) as u8).collect::<Vec<_>>(),
            "Large",
            33,
        ),
    ));
    let mut trailing = gbr_gray(1, 1, &[9], "T", 25);
    trailing.extend_from_slice(&[1, 2, 3]);
    v.push(("gbr trailing", Gbr, trailing));
    let good = gbr_gray(2, 2, &[0; 4], "Test", 25);
    v.push(("gbr truncated", Gbr, good[..good.len() - 1].to_vec()));
    let mut bad_magic = good.clone();
    bad_magic[20] = b'X';
    v.push(("gbr bad magic", Gbr, bad_magic));
    v.push(("gbr cinepaint", Gbr, gbr(2, 2, &[0; 72], 18, "T", 25, 2)));
    v.push(("gbr pixel size 2", Gbr, gbr(2, 2, &[0; 8], 2, "T", 25, 2)));
    v.push(("gbr too big", Gbr, gbr_gray(2049, 1, &[0; 2049], "T", 25)));
    v.push(("gbr version 9", Gbr, gbr(1, 1, &[0], 1, "T", 25, 9)));
    // GIH
    let header = "Sparks\n3 ncells:3 cellwidth:2 cellheight:2 step:100 dim:1 cols:1 rows:1 placement:constant rank0:3 sel0:random\n";
    v.push(("gih random", Gih, hose_header(header, 3)));
    v.push(("gih incremental", Gih, hose_header("Seq\n2\n", 2)));
    v.push((
        "gih pressure",
        Gih,
        hose_header(
            "Felt\n2 ncells:2 dim:3 rank0:2 sel0:pressure rank1:1 sel1:ytilt rank2:1 sel2:xtilt\n",
            2,
        ),
    ));
    v.push(("gih short", Gih, hose_header("Short\n3\n", 2)));
    let mut cut = hose_header("Cut\n2\n", 2);
    cut.pop();
    v.push(("gih cut", Gih, cut));
    v.push(("gih single", Gih, hose_header("One\n1\n", 1)));
    v.push(("gih unnamed", Gih, hose_header("\n2 sel0:random\n", 2)));
    v.push(("gih crlf", Gih, hose_header("Crlf\r\n2 ncells:2\r\n", 2)));
    v.push((
        "gih tabs",
        Gih,
        hose_header("Tabs\n2\tncells:2\tdim:1\n", 2),
    ));
    for (label, header) in [
        ("gih zero cells", "Z\n0\n"),
        ("gih 257 cells", "Z\n257\n"),
        ("gih text count", "Z\nabc\n"),
        ("gih negative", "Z\n-3\n"),
        ("gih no newline", "NoNewline"),
        ("gih empty", ""),
    ] {
        v.push((label, Gih, hose_header(header, 3)));
    }
    // VBR
    v.push((
        "vbr soft",
        Vbr,
        b"GIMP-VBR\n1.0\nHardness 050\n10.000000\n25.000000\n0.500000\n1.000000\n0.000000\n"
            .to_vec(),
    ));
    v.push(("vbr star", Vbr, b"GIMP-VBR\r\n1.5\r\nStar\r\ndiamond\r\n50.000000\r\n25.000000\r\n5\r\n1.000000\r\n2.500000\r\n17.500000\r\n".to_vec()));
    v.push((
        "vbr square",
        Vbr,
        b"GIMP-VBR\n1.5\nBox\nsquare\n50\n25\n2\n0.5\n1\n0\n".to_vec(),
    ));
    v.push((
        "vbr circle spikes",
        Vbr,
        b"GIMP-VBR\n1.5\nSpiky\ncircle\n50\n25\n7\n1\n1\n90\n".to_vec(),
    ));
    v.push((
        "vbr hexagon",
        Vbr,
        b"GIMP-VBR\n1.5\nBad\nhexagon\n1\n1\n2\n1\n1\n0\n".to_vec(),
    ));
    v.push((
        "vbr spikes 21",
        Vbr,
        b"GIMP-VBR\n1.5\nBad\ncircle\n1\n1\n21\n1\n1\n0\n".to_vec(),
    ));
    v.push(("vbr short", Vbr, b"GIMP-VBR\n1.0\nShort\n10".to_vec()));
    v.push(("vbr not", Vbr, b"NOT-VBR\n1.0\n".to_vec()));
    v.push((
        "vbr nan",
        Vbr,
        b"GIMP-VBR\n1.0\nNaN\nnan\n1\n1\n1\n0\n".to_vec(),
    ));
    v.push((
        "vbr huge",
        Vbr,
        b"GIMP-VBR\n1.0\nBig\n10\n1e999\n1\n1\n0\n".to_vec(),
    ));
    v.push((
        "vbr unnamed",
        Vbr,
        b"GIMP-VBR\n1.0\n\n10\n8\n1\n1\n0\n".to_vec(),
    ));
    v.push((
        "vbr bom",
        Vbr,
        [
            &[0xEF, 0xBB, 0xBF][..],
            b"GIMP-VBR\n1.0\nBom\n10\n8\n1\n1\n0\n",
        ]
        .concat(),
    ));
    v.push((
        "vbr spacing max",
        Vbr,
        b"GIMP-VBR\n1.0\nS\n5000\n4000\n0\n20\n180\n".to_vec(),
    ));
    v.push(("vbr version 2", Vbr, b"GIMP-VBR\n2.0\nX\n".to_vec()));
    // ABR 版 1・2・10
    let (v1, v2) = abr_v1v2();
    v.push(("abr v1", Abr, v1));
    v.push(("abr v2", Abr, v2));
    v.push(("abr v10 tips", Abr, abr_v10()));
    // ABR 版 6
    v.push((
        "abr v6 leaves",
        Abr,
        abr_with(
            &[leaves_preset()],
            &samp_record(1, ID, false),
            Some(&[0; 8]),
        ),
    ));
    v.push(("abr v6 everything", Abr, dual_file(120.0)));
    v.push(("abr v6 dual no scatter", Abr, dual_file(0.0)));
    v.push(("abr v6 bad desc", Abr, bad_desc()));
    v.push((
        "abr v6 weird",
        Abr,
        abr_with(&[weird_preset()], &samp_record(1, "t", false), None),
    ));
    v.push((
        "abr v6 clamp",
        Abr,
        abr_with(&[clamp_preset()], &samp_record(1, "t", false), None),
    ));
    let pat = pattern_gray("Paper", "p", 2, 2, &[10, 20, 30, 40]);
    let modes = [
        "Mltp",
        "Sbtr",
        "blendSubtraction",
        "Drkn",
        "Ovrl",
        "CDdg",
        "CBrn",
        "linearBurn",
        "hardMix",
        "weird",
    ];
    let presets: Vec<Vec<u8>> = modes
        .iter()
        .map(|m| textured_preset(m, m, 100.0, false, 0))
        .collect();
    v.push((
        "abr v6 texture modes",
        Abr,
        abr_with(
            &presets,
            &samp_record(1, ID, false),
            Some(&framed(&[pat.clone()])),
        ),
    ));
    v.push((
        "abr v6 texture scale",
        Abr,
        abr_with(
            &[
                textured_preset("s", "Mltp", 10000.0, false, 0),
                textured_preset("t", "Mltp", 1.0, false, 0),
            ],
            &samp_record(1, ID, false),
            Some(&framed(&[pat.clone()])),
        ),
    ));
    v.push((
        "abr v6 flips",
        Abr,
        abr_with(
            &[textured_preset("f", "Mltp", 100.0, true, 0)],
            &samp_record(1, ID, false),
            Some(&framed(&[pat.clone()])),
        ),
    ));
    for control in [1, 2, 3, 4, 5, 6, 7, 9] {
        v.push((
            "abr v6 angle control",
            Abr,
            abr_with(
                &[textured_preset("c", "Mltp", 100.0, false, control)],
                &samp_record(1, ID, false),
                Some(&framed(&[pat.clone()])),
            ),
        ));
    }
    let no_shape = preset(&[("Nm  ", text("No shape"))]);
    let missing_tip = preset(&[
        ("Nm  ", text("Lost tip")),
        (
            "Brsh",
            obj("sampledBrush", &[("sampledData", text("not-there"))]),
        ),
    ]);
    let fine = preset(&[("Nm  ", text("Fine")), ("Brsh", obj("computedBrush", &[]))]);
    v.push((
        "abr v6 skipped presets",
        Abr,
        abr_with(
            &[no_shape, missing_tip, fine],
            &samp_record(1, "orphan", false),
            None,
        ),
    ));
    v.push((
        "abr v6 unknown sections",
        Abr,
        abr_v6(&[
            section("samp", &samp_record(1, "x", false)),
            section("zzzz", &[1, 2, 3]),
            section("zzzz", &[4]),
            section("yyyy", &[]),
        ]),
    ));
    let record16 = W::new()
        .u8(1)
        .ascii("z")
        .bytes(&[0; 10])
        .i32(0)
        .i32(0)
        .i32(1)
        .i32(2)
        .i16(16)
        .u8(0)
        .bytes(&[0x80, 0x01, 0xFF, 0xFF])
        .done();
    v.push((
        "abr v6 16-bit",
        Abr,
        abr_v6(&[section(
            "samp",
            &W::new()
                .i32(record16.len() as i64)
                .bytes(&record16)
                .pad4()
                .done(),
        )]),
    ));
    // ABR の模様
    let lab = pattern(
        9,
        "Lab",
        "lab-1",
        1,
        1,
        &[vec![1], vec![2], vec![3]],
        false,
        8,
        None,
    );
    let rgb = pattern(
        3,
        "Colour",
        "rgb-1",
        2,
        1,
        &[vec![255, 0], vec![0, 0], vec![0, 255]],
        true,
        8,
        None,
    );
    let ref_preset = |name: &str, pname: &str, id: &str| {
        preset(&[
            ("Nm  ", text(name)),
            ("Brsh", obj("computedBrush", &[])),
            ("useTexture", boolean(true)),
            (
                "Txtr",
                obj("Ptrn", &[("Nm  ", text(pname)), ("Idnt", text(id))]),
            ),
        ])
    };
    let unusable = abr_v6(&[
        section(
            "desc",
            &desc_body(&[brush_list(&[
                ref_preset("A", "Lab", "lab-1"),
                ref_preset("B", "Colour", "rgb-1"),
            ])]),
        ),
        section("patt", &framed(&[lab, rgb])),
    ]);
    v.push(("abr v6 patterns unusable", Abr, unusable.clone()));
    v.push((
        "abr v6 broken patt",
        Abr,
        abr_v6(&[
            section(
                "desc",
                &desc_body(&[brush_list(&[ref_preset("A", "Lab", "lab-1")])]),
            ),
            section("patt", &W::new().i32(9999).done()),
        ]),
    ));
    // ABR を断る
    v.push(("abr version 3", Abr, W::new().i16(3).done()));
    v.push((
        "abr v1 truncated",
        Abr,
        W::new().i16(1).i16(1).i16(2).i32(100).done(),
    ));
    v.push(("abr no brushes", Abr, W::new().i16(6).i16(1).done()));
    v.push(("abr subversion 7", Abr, W::new().i16(6).i16(7).done()));
    v.push((
        "abr huge tip",
        Abr,
        abr_v6(&[section(
            "samp",
            &W::new()
                .i32(30)
                .u8(1)
                .ascii("z")
                .bytes(&[0; 10])
                .i32(0)
                .i32(0)
                .i32(5000)
                .i32(5000)
                .i16(8)
                .done(),
        )]),
    ));
    v.push((
        "abr bad depth",
        Abr,
        abr_v6(&[section(
            "samp",
            &W::new()
                .i32(30)
                .u8(1)
                .ascii("z")
                .bytes(&[0; 10])
                .i32(0)
                .i32(0)
                .i32(2)
                .i32(3)
                .i16(4)
                .done(),
        )]),
    ));
    v.push((
        "abr bad compression",
        Abr,
        abr_v6(&[section(
            "samp",
            &W::new()
                .i32(30)
                .u8(1)
                .ascii("z")
                .bytes(&[0; 10])
                .i32(0)
                .i32(0)
                .i32(2)
                .i32(3)
                .i16(8)
                .u8(2)
                .done(),
        )]),
    ));
    // PAT
    v.push(("pat all kinds", Pat, pat_file()));
    let g = pattern(
        1,
        "Grain",
        "g",
        2,
        2,
        &[vec![10, 20, 30, 40]],
        true,
        8,
        None,
    );
    let cmyk = pattern(
        4,
        "Print",
        "c",
        1,
        1,
        &[vec![0], vec![0], vec![0], vec![0]],
        false,
        8,
        None,
    );
    let one_pat = |p: &Vec<u8>| W::new().ascii("8BPT").i16(1).i32(1).bytes(p).done();
    v.push(("pat only cmyk", Pat, one_pat(&cmyk)));
    v.push(("pat truncated", Pat, one_pat(&g[..g.len() - 3].to_vec())));
    v.push((
        "pat big",
        Pat,
        one_pat(&pattern(
            1,
            "Big",
            "b",
            3000,
            1,
            &[vec![0; 3000]],
            false,
            8,
            None,
        )),
    ));
    v.push((
        "pat bad signature",
        Pat,
        W::new().ascii("8BPS").i16(1).i32(0).done(),
    ));
    v.push((
        "pat version 2",
        Pat,
        W::new().ascii("8BPT").i16(2).i32(0).done(),
    ));
    v.push((
        "pat empty",
        Pat,
        W::new().ascii("8BPT").i16(1).i32(0).done(),
    ));
    // 宣言の大きさとデータの大きさが違い、使わない透明度のチャンネルが 1 つ
    let channel = |plane: &[u8]| {
        W::new()
            .i32(8)
            .i32(0)
            .i32(0)
            .i32(1)
            .i32(2)
            .i16(8)
            .u8(0)
            .bytes(plane)
            .done()
    };
    let (c0, c1) = (channel(&[5, 6]), channel(&[255, 255]));
    let list = W::new()
        .i32(0)
        .i32(0)
        .i32(1)
        .i32(2)
        .i32(1)
        .i32(1)
        .i32(c0.len() as i64)
        .bytes(&c0)
        .i32(1)
        .i32(c1.len() as i64)
        .bytes(&c1)
        .i32(0)
        .i32(0)
        .done();
    let odd = W::new()
        .i32(1)
        .i32(1)
        .i16(4)
        .i16(4)
        .unicode("Odd")
        .u8(1)
        .ascii("o")
        .i32(3)
        .i32(list.len() as i64)
        .bytes(&list)
        .done();
    v.push(("pat odd size and extra channel", Pat, one_pat(&odd)));
    v.push(("abr patterns only", Abr, unusable));
    v
}

/// 同梱の Krita の筆先のうち GIMP の形式のもの（実在のファイル。C# の読み手にも通す）。
pub fn bundled_gimp() -> Vec<(String, FileKind, Vec<u8>)> {
    yolu_brush_sets::KRITA4
        .iter()
        .filter_map(|t| {
            let kind = match t.file.rsplit('.').next()? {
                "gbr" => FileKind::Gbr,
                "gih" => FileKind::Gih,
                _ => return None,
            };
            Some((format!("krita {}", t.file), kind, t.bytes.to_vec()))
        })
        .collect()
}

/// 壊した入力の元になるファイル。
fn fuzz_bases() -> Vec<(&'static str, FileKind, Vec<u8>)> {
    use FileKind::*;
    let pat = pattern_gray("Paper", "p", 2, 2, &[10, 20, 30, 40]);
    let (v1, v2) = abr_v1v2();
    vec![
        ("gbr gray", Gbr, gbr_gray(3, 2, &[1, 2, 3, 4, 5, 6], "Tip", 50)),
        ("gbr rgba v1", Gbr, gbr(2, 1, &[1, 2, 3, 4, 5, 6, 7, 8], 4, "Col", 25, 1)),
        ("gih", Gih, gih("Hose\n3 ncells:3 dim:1 sel0:random\n", &cells())),
        ("gih pressure", Gih, gih("Seq\n2 dim:3 sel0:pressure\n", &cells()[..2])),
        ("vbr 1.0", Vbr, b"GIMP-VBR\n1.0\nHardness 050\n10.000000\n25.000000\n0.500000\n1.000000\n0.000000\n".to_vec()),
        ("vbr 1.5", Vbr, b"GIMP-VBR\r\n1.5\r\nStar\r\ndiamond\r\n50.000000\r\n25.000000\r\n5\r\n1.000000\r\n2.500000\r\n17.500000\r\n".to_vec()),
        ("abr v1", Abr, v1),
        ("abr v2", Abr, v2),
        ("abr v10", Abr, abr_v10()),
        ("abr v6 everything", Abr, dual_file(120.0)),
        ("abr v6 leaves", Abr, abr_with(&[leaves_preset()], &samp_record(1, ID, false), Some(&[0; 8]))),
        ("abr v6 texture", Abr, abr_with(&[textured_preset("t", "Sbtr", 100.0, true, 5)], &samp_record(1, ID, false), Some(&framed(&[pat])))),
        ("pat", Pat, pat_file()),
    ]
}

/// 壊した入力: 全バイトの書き換え（0xFF と下位 1 bit の反転）・切り詰め・4 バイトの極端な値（0xFFFFFFFF と 0）。大きいファイルは
/// 位置を間引く（先頭の 64 バイトは全部）。
pub fn fuzz() -> Vec<(String, FileKind, Vec<u8>)> {
    let mut out = Vec::new();
    for (label, kind, base) in fuzz_bases() {
        let stride = (base.len() / 250).max(1);
        let positions: Vec<usize> = (0..base.len())
            .filter(|i| *i < 64 || i % stride == 0)
            .collect();
        for &i in &positions {
            for value in [0xFFu8, base[i] ^ 0x01] {
                if value != base[i] {
                    let mut m = base.clone();
                    m[i] = value;
                    out.push((format!("{label}: {i} 番目を {value:#x}"), kind, m));
                }
            }
            out.push((format!("{label}: {i} で切る"), kind, base[..i].to_vec()));
            if i + 4 <= base.len() {
                for value in [0xFFFF_FFFFu32, 0] {
                    let mut m = base.clone();
                    m[i..i + 4].copy_from_slice(&value.to_be_bytes());
                    out.push((
                        format!("{label}: {i} 番目から 4 バイトを {value:#x}"),
                        kind,
                        m,
                    ));
                }
            }
        }
    }
    out
}

/// 全部（手で組んだ事例、続いて壊した入力）。
pub fn all() -> Vec<Case> {
    let mut v: Vec<Case> = hand_built()
        .into_iter()
        .map(|(label, kind, bytes)| Case {
            tag: 1,
            kind,
            label: label.to_string(),
            bytes,
        })
        .collect();
    v.extend(bundled_gimp().into_iter().map(|(label, kind, bytes)| Case {
        tag: 1,
        kind,
        label,
        bytes,
    }));
    v.extend(fuzz().into_iter().map(|(label, kind, bytes)| Case {
        tag: 0,
        kind,
        label,
        bytes,
    }));
    v
}
