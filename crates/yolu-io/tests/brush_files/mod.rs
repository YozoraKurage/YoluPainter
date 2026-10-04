//! ブラシの試験用ファイルの組み立て。C# の試験と同じく、公開された形式の並びどおりに試験の中で一から組む（外の .abr・.gbr は持ち込まない）。
#![allow(
    dead_code,
    clippy::vec_init_then_push,
    clippy::cloned_ref_to_slice_refs,
    clippy::too_many_arguments,
    clippy::manual_is_multiple_of
)]

pub mod corpus;

/// ビッグエンディアンの書き出し器（ABR・パターン・記述子の組み立て用）。
#[derive(Default, Clone)]
pub struct W(pub Vec<u8>);

impl W {
    pub fn new() -> W {
        W(Vec::new())
    }
    pub fn i16(mut self, v: i32) -> W {
        self.0.extend_from_slice(&(v as i16).to_be_bytes());
        self
    }
    pub fn i32(mut self, v: i64) -> W {
        self.0.extend_from_slice(&(v as i32).to_be_bytes());
        self
    }
    pub fn u8(mut self, v: i32) -> W {
        self.0.push(v as u8);
        self
    }
    pub fn bytes(mut self, b: &[u8]) -> W {
        self.0.extend_from_slice(b);
        self
    }
    pub fn ascii(self, a: &str) -> W {
        self.bytes(a.as_bytes())
    }
    pub fn double(mut self, v: f64) -> W {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    /// 長さ（NUL を含む UTF-16 の単位数）と UTF-16BE。
    pub fn unicode(mut self, t: &str) -> W {
        let units: Vec<u16> = t.encode_utf16().chain(std::iter::once(0)).collect();
        self = self.i32(units.len() as i64);
        for u in units {
            self.0.extend_from_slice(&u.to_be_bytes());
        }
        self
    }
    /// キー: 4 文字なら長さ 0 と 4 文字、そうでなければ長さと文字列。
    pub fn key(self, k: &str) -> W {
        if k.len() == 4 {
            self.i32(0).ascii(k)
        } else {
            self.i32(k.len() as i64).ascii(k)
        }
    }
    pub fn pad4(mut self) -> W {
        while self.0.len() % 4 != 0 {
            self.0.push(0);
        }
        self
    }
    pub fn done(self) -> Vec<u8> {
        self.0
    }
}

// ---------------- ActionDescriptor ----------------

/// 記述子（名前・クラス ID・項目）。項目は (キー, 型の 4 文字つきの値のバイト列)。
pub fn descriptor(class_id: &str, items: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut w = W::new().unicode("").key(class_id).i32(items.len() as i64);
    for (key, value) in items {
        w = w.key(key).bytes(value);
    }
    w.done()
}
pub fn unit(unit: &str, v: f64) -> Vec<u8> {
    W::new().ascii("UntF").ascii(unit).double(v).done()
}
pub fn doub(v: f64) -> Vec<u8> {
    W::new().ascii("doub").double(v).done()
}
pub fn boolean(v: bool) -> Vec<u8> {
    W::new().ascii("bool").u8(v as i32).done()
}
pub fn long(v: i32) -> Vec<u8> {
    W::new().ascii("long").i32(v as i64).done()
}
pub fn text(t: &str) -> Vec<u8> {
    W::new().ascii("TEXT").unicode(t).done()
}
pub fn enum_value(kind: &str, value: &str) -> Vec<u8> {
    W::new().ascii("enum").key(kind).key(value).done()
}
pub fn obj(class_id: &str, items: &[(&str, Vec<u8>)]) -> Vec<u8> {
    W::new()
        .ascii("Objc")
        .bytes(&descriptor(class_id, items))
        .done()
}
pub fn list(values: &[Vec<u8>]) -> Vec<u8> {
    let mut w = W::new().ascii("VlLs").i32(values.len() as i64);
    for v in values {
        w = w.bytes(v);
    }
    w.done()
}
pub fn dynamics(control: i32, jitter: f64) -> Vec<u8> {
    obj(
        "brVr",
        &[
            ("bVTy", long(control)),
            ("fStp", long(25)),
            ("jitter", unit("#Prc", jitter)),
        ],
    )
}

// ---------------- GIMP ----------------

/// `.gbr`。pixels は上の行から（ファイルの並び）。
pub fn gbr(
    width: u32,
    height: u32,
    pixels: &[u8],
    bytes: u32,
    name: &str,
    spacing: u32,
    version: u32,
) -> Vec<u8> {
    let mut name_bytes = name.as_bytes().to_vec();
    name_bytes.push(0);
    let header = (if version == 1 { 20 } else { 28 }) + name_bytes.len() as u32;
    let mut out = Vec::new();
    for v in [header, version, width, height, bytes] {
        out.extend_from_slice(&v.to_be_bytes());
    }
    if version != 1 {
        out.extend_from_slice(&0x4749_4D50u32.to_be_bytes());
        out.extend_from_slice(&spacing.to_be_bytes());
    }
    out.extend_from_slice(&name_bytes);
    out.extend_from_slice(pixels);
    out
}
pub fn gbr_gray(width: u32, height: u32, pixels: &[u8], name: &str, spacing: u32) -> Vec<u8> {
    gbr(width, height, pixels, 1, name, spacing, 2)
}
pub fn gih(header: &str, cells: &[Vec<u8>]) -> Vec<u8> {
    let mut out = header.as_bytes().to_vec();
    for c in cells {
        out.extend_from_slice(c);
    }
    out
}

// ---------------- Photoshop ----------------

/// 上の行から並んだ 8 bit の筆先（3x2: 上 = 255,0,0 / 下 = 0,0,128）。
pub const TIP_ROWS: [u8; 6] = [255, 0, 0, 0, 0, 128];
pub fn raw_bitmap(w: W) -> W {
    w.u8(0).bytes(&TIP_ROWS)
}
pub fn rle_bitmap(w: W) -> W {
    // 行ごとの PackBits: 上の行 = 「255 を 1 個、0 を 2 個」、下の行 = 「0 を 2 個、128 を 1 個」
    let rows: [&[u8]; 2] = [&[0x00, 255, 0xFF, 0], &[0xFF, 0, 0x00, 128]];
    let mut w = w.u8(1);
    for row in rows {
        w = w.i16(row.len() as i32);
    }
    for row in rows {
        w = w.bytes(row);
    }
    w
}
pub fn section(key: &str, data: &[u8]) -> Vec<u8> {
    W::new()
        .ascii("8BIM")
        .ascii(key)
        .i32(data.len() as i64)
        .bytes(data)
        .pad4()
        .done()
}
/// `samp` の記録 1 つ（3x2 の筆先）。
pub fn samp_record(subversion: i32, id: &str, rle: bool) -> Vec<u8> {
    let record = W::new()
        .u8(id.len() as i32)
        .ascii(id)
        .bytes(&vec![0u8; if subversion == 1 { 10 } else { 264 }])
        .i32(0)
        .i32(0)
        .i32(2)
        .i32(3)
        .i16(8);
    let record = if rle {
        rle_bitmap(record)
    } else {
        raw_bitmap(record)
    };
    let data = record.done();
    W::new().i32(data.len() as i64).bytes(&data).pad4().done()
}
/// 大きさ・内容を選べる `samp` の記録（内容は筆先の行そのまま・上の行から）。
pub fn samp_sized(
    subversion: i32,
    id: &str,
    width: i32,
    height: i32,
    rows_top_down: &[u8],
) -> Vec<u8> {
    let record = W::new()
        .u8(id.len() as i32)
        .ascii(id)
        .bytes(&vec![0u8; if subversion == 1 { 10 } else { 264 }])
        .i32(0)
        .i32(0)
        .i32(height as i64)
        .i32(width as i64)
        .i16(8)
        .u8(0)
        .bytes(rows_top_down);
    let data = record.done();
    W::new().i32(data.len() as i64).bytes(&data).pad4().done()
}

pub fn abr_v6(sections: &[Vec<u8>]) -> Vec<u8> {
    let mut w = W::new().i16(6).i16(1);
    for s in sections {
        w = w.bytes(s);
    }
    w.done()
}
/// `desc` の節の中身（版 16 と記述子）。
pub fn desc_body(items: &[(&str, Vec<u8>)]) -> Vec<u8> {
    W::new().i32(16).bytes(&descriptor("null", items)).done()
}
pub fn preset(items: &[(&str, Vec<u8>)]) -> Vec<u8> {
    obj("brushPreset", items)
}
pub fn brush_list(presets: &[Vec<u8>]) -> (&'static str, Vec<u8>) {
    ("Brsh", list(presets))
}

/// 模様 1 つ（Photoshop の Pattern 構造）。channels は上の行から、チャンネルごと。
pub fn pattern(
    mode: i32,
    name: &str,
    id: &str,
    w: i32,
    h: i32,
    channels: &[Vec<u8>],
    rle: bool,
    depth: i32,
    palette: Option<&[u8]>,
) -> Vec<u8> {
    let mut body = W::new()
        .i32(1)
        .i32(mode as i64)
        .i16(h)
        .i16(w)
        .unicode(name)
        .u8(id.len() as i32)
        .ascii(id);
    if mode == 2 {
        body = body
            .bytes(palette.expect("インデックスにはパレット"))
            .i32(0);
    }
    let mut list = W::new()
        .i32(0)
        .i32(0)
        .i32(h as i64)
        .i32(w as i64)
        .i32(channels.len() as i64);
    for plane in channels {
        let row_bytes = (w * depth / 8) as usize;
        let mut data = W::new();
        if rle {
            let rows: Vec<Vec<u8>> = (0..h as usize)
                .map(|y| {
                    let mut row = vec![(row_bytes - 1) as u8];
                    row.extend_from_slice(&plane[y * row_bytes..(y + 1) * row_bytes]);
                    row
                })
                .collect();
            for row in &rows {
                data = data.i16(row.len() as i32);
            }
            for row in &rows {
                data = data.bytes(row);
            }
        } else {
            data = data.bytes(plane);
        }
        let array = W::new()
            .i32(depth as i64)
            .i32(0)
            .i32(0)
            .i32(h as i64)
            .i32(w as i64)
            .i16(depth)
            .u8(rle as i32)
            .bytes(&data.done())
            .done();
        list = list.i32(1).i32(array.len() as i64).bytes(&array);
    }
    let l = list.i32(0).i32(0).done(); // 利用者のマスク・シートのマスク（書かれていない）
    body.i32(3).i32(l.len() as i64).bytes(&l).done()
}
pub fn pattern_gray(name: &str, id: &str, w: i32, h: i32, plane: &[u8]) -> Vec<u8> {
    pattern(1, name, id, w, h, &[plane.to_vec()], false, 8, None)
}
pub fn framed(patterns: &[Vec<u8>]) -> Vec<u8> {
    let mut w = W::new();
    for p in patterns {
        w = w.i32(p.len() as i64).bytes(p).pad4();
    }
    w.done()
}

// ---------------- PNG ----------------

/// `png` クレートで試験用の PNG を書く（行は上から）。
pub fn png_file(
    width: u32,
    height: u32,
    color: png::ColorType,
    depth: png::BitDepth,
    data: &[u8],
    palette: Option<&[u8]>,
    trns: Option<&[u8]>,
) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(color);
        encoder.set_depth(depth);
        if let Some(p) = palette {
            encoder.set_palette(p.to_vec());
        }
        if let Some(t) = trns {
            encoder.set_trns(t.to_vec());
        }
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(data).unwrap();
    }
    out
}

// ---------------- 組み合わせの例 ----------------

pub const ID: &str = "3f1c5c3e-0000-4000-8000-0123456789ab";

/// 色・デュアル・質感・コントロールを全部使う版 6 の ABR（デュアルの筆先 1 つ、模様 1 つ、プリセット 2 つ）。
pub fn dual_file(dual_scatter: f64) -> Vec<u8> {
    let grey = pattern_gray("Paper", "pat-1", 2, 2, &[0, 255, 100, 200]);
    let everything = preset(&[
        ("Nm  ", text("Everything")),
        (
            "Brsh",
            obj(
                "computedBrush",
                &[
                    ("Dmtr", unit("#Pxl", 40.0)),
                    ("Hrdn", unit("#Prc", 50.0)),
                    ("Spcn", unit("#Prc", 20.0)),
                ],
            ),
        ),
        ("useTipDynamics", boolean(true)),
        ("szVr", dynamics(1, 0.0)),
        ("angleDynamics", dynamics(7, 0.0)),
        ("roundnessDynamics", dynamics(3, 0.0)),
        ("usePaintDynamics", boolean(true)),
        ("opVr", dynamics(3, 0.0)),
        ("prVr", dynamics(4, 0.0)),
        ("useColorDynamics", boolean(true)),
        ("clVr", dynamics(0, 40.0)),
        ("H   ", unit("#Prc", 20.0)),
        ("Strt", unit("#Prc", 30.0)),
        ("Brgh", unit("#Prc", 10.0)),
        ("purity", unit("#Prc", -50.0)),
        ("useDualBrush", boolean(true)),
        (
            "dualBrush",
            obj(
                "dualBrush",
                &[
                    ("useDualBrush", boolean(true)),
                    ("Flip", boolean(false)),
                    (
                        "Brsh",
                        obj(
                            "sampledBrush",
                            &[
                                ("Dmtr", unit("#Pxl", 24.0)),
                                ("Angl", unit("#Ang", 30.0)),
                                ("Spcn", unit("#Prc", 40.0)),
                                ("sampledData", text("dual-tip")),
                            ],
                        ),
                    ),
                    ("BlnM", enum_value("BlnM", "CBrn")),
                    ("useScatter", boolean(true)),
                    ("Cnt ", doub(3.0)),
                    ("bothAxes", boolean(true)),
                    ("scatterDynamics", dynamics(0, dual_scatter)),
                ],
            ),
        ),
        ("useTexture", boolean(true)),
        (
            "Txtr",
            obj("Ptrn", &[("Nm  ", text("Paper")), ("Idnt", text("pat-1"))]),
        ),
        ("textureDepth", unit("#Prc", 60.0)),
        ("textureScale", unit("#Prc", 200.0)),
        ("InvT", boolean(true)),
        ("textureBlendMode", enum_value("BlnM", "Mltp")),
    ]);
    let missing = preset(&[
        ("Nm  ", text("Missing pattern")),
        (
            "Brsh",
            obj("computedBrush", &[("Dmtr", unit("#Pxl", 10.0))]),
        ),
        ("useTexture", boolean(true)),
        (
            "Txtr",
            obj(
                "Ptrn",
                &[("Nm  ", text("Paper")), ("Idnt", text("other-id"))],
            ),
        ),
        ("textureBlendMode", enum_value("BlnM", "Sbtr")),
        ("TxtC", boolean(true)),
    ]);
    abr_v6(&[
        section("samp", &samp_record(1, "dual-tip", false)),
        section("desc", &desc_body(&[brush_list(&[everything, missing])])),
        section("patt", &framed(&[grey])),
    ])
}

// ---------------- 共有の入力（壊し方の試験と C# との照合が使う） ----------------

pub fn cells() -> Vec<Vec<u8>> {
    vec![
        gbr_gray(2, 2, &[255, 255, 0, 0], "a", 25),
        gbr(
            3,
            1,
            &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12],
            4,
            "b",
            50,
            2,
        ),
        gbr_gray(2, 2, &[0; 4], "c", 25),
    ]
}

pub fn pat_file() -> Vec<u8> {
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
    let deep = pattern(
        1,
        "Deep",
        "d",
        1,
        2,
        &[vec![0x80, 0x01, 0x40, 0x02]],
        false,
        16,
        None,
    );
    let mut palette = vec![0u8; 768];
    palette[22] = 255;
    let indexed = pattern(
        2,
        "Indexed",
        "i",
        1,
        1,
        &[vec![7]],
        false,
        8,
        Some(&palette),
    );
    let rgb = pattern(
        3,
        "Colour",
        "r",
        2,
        1,
        &[vec![255, 0], vec![0, 0], vec![0, 255]],
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
    let mut w = W::new().ascii("8BPT").i16(1).i32(5);
    for p in [g, deep, indexed, rgb, cmyk] {
        w = w.bytes(&p);
    }
    w.done()
}

pub fn abr_v1v2() -> (Vec<u8>, Vec<u8>) {
    let computed = W::new()
        .i32(0)
        .i16(30)
        .i16(40)
        .i16(50)
        .i16(-30)
        .i16(80)
        .done();
    let sampled = raw_bitmap(
        W::new()
            .i32(0)
            .i16(25)
            .u8(1)
            .i16(0)
            .i16(0)
            .i16(2)
            .i16(3)
            .i32(0)
            .i32(0)
            .i32(2)
            .i32(3)
            .i16(8),
    )
    .done();
    let v1 = W::new()
        .i16(1)
        .i16(2)
        .i16(1)
        .i32(computed.len() as i64)
        .bytes(&computed)
        .i16(2)
        .i32(sampled.len() as i64)
        .bytes(&sampled)
        .done();
    let named = rle_bitmap(
        W::new()
            .i32(0)
            .i16(10)
            .unicode("Grass 草")
            .u8(1)
            .i16(0)
            .i16(0)
            .i16(2)
            .i16(3)
            .i32(0)
            .i32(0)
            .i32(2)
            .i32(3)
            .i16(8),
    )
    .done();
    let v2 = W::new()
        .i16(2)
        .i16(1)
        .i16(2)
        .i32(named.len() as i64)
        .bytes(&named)
        .done();
    (v1, v2)
}

pub fn abr_v10() -> Vec<u8> {
    let mut samples = samp_record(2, "a", true);
    samples.extend(samp_record(2, "b", false));
    W::new()
        .i16(10)
        .i16(2)
        .bytes(&section("samp", &samples))
        .done()
}
