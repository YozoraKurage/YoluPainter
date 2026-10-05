//! CLIP STUDIO の `.sut`（SQLite）の試験用ファイルの組み立て。本物の .sut は持ち込まず、公開の解析で分かっている形（`Node`・
//! `Variant`・`MaterialFile`、影響元と素材の参照の BLOB、無圧縮 tar の中のプレビューの PNG）を試験の中で一から組む。
//! 列の名前は読み手の定数と別に書く（読み手の綴りの取り違えを試験で見つけるため）。
use rusqlite::types::Value;
use rusqlite::Connection;

pub fn real(v: f64) -> Value {
    Value::Real(v)
}
pub fn int(v: i64) -> Value {
    Value::Integer(v)
}
pub fn blob(v: Vec<u8>) -> Value {
    Value::Blob(v)
}

#[derive(Clone, Default)]
pub struct SutBuilder {
    nodes: Vec<(String, i64, i64)>,
    variants: Vec<(i64, Vec<(&'static str, Value)>)>,
    materials: Vec<(Option<String>, Vec<u8>)>,
    /// `Variant` に足す、読まない列の数（実物は数百列）。
    pub filler: usize,
    /// `MaterialFile` に素材の名前の列を持たせるか。
    pub material_names: bool,
    pub no_node: bool,
    pub no_variant: bool,
    pub no_material: bool,
    /// SQLite のページの大きさ（既定 1024。壊し方の試験では最小の 512 で小さなファイルにする）。
    pub page_size: u32,
}

impl SutBuilder {
    pub fn new() -> SutBuilder {
        SutBuilder {
            filler: 200,
            material_names: true,
            page_size: 1024,
            ..SutBuilder::default()
        }
    }
    pub fn node(mut self, name: &str, variant: i64, init: i64) -> SutBuilder {
        self.nodes.push((name.to_string(), variant, init));
        self
    }
    pub fn variant(mut self, id: i64, cells: &[(&'static str, Value)]) -> SutBuilder {
        self.variants.push((id, cells.to_vec()));
        self
    }
    /// 素材 1 つ（名前の列の値と `FileData`）。
    pub fn material(mut self, name: Option<&str>, file_data: Vec<u8>) -> SutBuilder {
        self.materials.push((name.map(str::to_string), file_data));
        self
    }
    /// 1 つのブラシ（`Node` 1 行と `Variant` 1 行）。
    pub fn brush(self, name: &str, id: i64, cells: &[(&'static str, Value)]) -> SutBuilder {
        self.node(name, id, id).variant(id, cells)
    }

    pub fn build(&self) -> Vec<u8> {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(&format!("PRAGMA page_size = {};", self.page_size))
            .unwrap();
        if !self.no_node {
            conn.execute_batch(
                "CREATE TABLE Node (_PW_ID INTEGER PRIMARY KEY AUTOINCREMENT, NodeName TEXT, \
                 NodeVariantId INTEGER, NodeInitVariantId INTEGER, NodeUuid BLOB);",
            )
            .unwrap();
            for (name, v, init) in &self.nodes {
                conn.execute(
                    "INSERT INTO Node (NodeName, NodeVariantId, NodeInitVariantId, NodeUuid) VALUES (?1, ?2, ?3, x'00112233')",
                    rusqlite::params![name, v, init],
                )
                .unwrap();
            }
        }
        if !self.no_variant {
            let mut columns: Vec<(&'static str, &'static str)> = Vec::new();
            for (_, cells) in &self.variants {
                for (name, value) in cells {
                    if !columns.iter().any(|(n, _)| n == name) {
                        let ty = match value {
                            Value::Integer(_) => "INTEGER",
                            Value::Real(_) => "REAL",
                            Value::Text(_) => "TEXT",
                            Value::Blob(_) => "BLOB",
                            Value::Null => "INTEGER",
                        };
                        columns.push((name, ty));
                    }
                }
            }
            let mut sql = String::from(
                "CREATE TABLE Variant (_PW_ID INTEGER PRIMARY KEY AUTOINCREMENT, VariantID INTEGER",
            );
            for (name, ty) in &columns {
                sql.push_str(&format!(", {name} {ty}"));
            }
            for i in 0..self.filler {
                sql.push_str(&format!(", Filler{i:04} INTEGER DEFAULT 0"));
            }
            sql.push_str(");");
            conn.execute_batch(&sql).unwrap();
            for (id, cells) in &self.variants {
                let names: Vec<&str> = std::iter::once("VariantID")
                    .chain(cells.iter().map(|(n, _)| *n))
                    .collect();
                let marks: Vec<String> = (1..=names.len()).map(|i| format!("?{i}")).collect();
                let values: Vec<Value> = std::iter::once(Value::Integer(*id))
                    .chain(cells.iter().map(|(_, v)| v.clone()))
                    .collect();
                conn.execute(
                    &format!(
                        "INSERT INTO Variant ({}) VALUES ({})",
                        names.join(", "),
                        marks.join(", ")
                    ),
                    rusqlite::params_from_iter(values),
                )
                .unwrap();
            }
        }
        if !self.no_material {
            let name_column = if self.material_names {
                ", MaterialName TEXT"
            } else {
                ""
            };
            conn.execute_batch(&format!(
                "CREATE TABLE MaterialFile (_PW_ID INTEGER PRIMARY KEY AUTOINCREMENT{name_column}, FileData BLOB);"
            ))
            .unwrap();
            for (name, data) in &self.materials {
                if self.material_names {
                    conn.execute(
                        "INSERT INTO MaterialFile (MaterialName, FileData) VALUES (?1, ?2)",
                        rusqlite::params![name, data],
                    )
                    .unwrap();
                } else {
                    conn.execute(
                        "INSERT INTO MaterialFile (FileData) VALUES (?1)",
                        rusqlite::params![data],
                    )
                    .unwrap();
                }
            }
        }
        let data = conn.serialize("main").unwrap();
        data.to_vec()
    }
}

// ---------------- PNG と tar ----------------

pub fn png_gray(w: u32, h: u32, pixels: &[u8]) -> Vec<u8> {
    assert_eq!(pixels.len(), (w * h) as usize);
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Grayscale);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().unwrap();
        writer.write_image_data(pixels).unwrap();
    }
    out
}

/// ustar（無圧縮 tar）。項目は (名前, 中身)。
pub fn tar(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    for (name, data) in files {
        let mut h = [0u8; 512];
        let n = name.as_bytes();
        h[..n.len().min(100)].copy_from_slice(&n[..n.len().min(100)]);
        h[100..107].copy_from_slice(b"0000644");
        h[108..115].copy_from_slice(b"0000000");
        h[116..123].copy_from_slice(b"0000000");
        h[124..135].copy_from_slice(format!("{:011o}", data.len()).as_bytes());
        h[136..147].copy_from_slice(b"00000000000");
        h[156] = b'0';
        h[257..262].copy_from_slice(b"ustar");
        h[263..265].copy_from_slice(b"00");
        for b in &mut h[148..156] {
            *b = b' ';
        }
        let sum: u32 = h.iter().map(|&b| b as u32).sum();
        h[148..155].copy_from_slice(format!("{sum:06o}\0").as_bytes());
        h[155] = b' ';
        out.extend_from_slice(&h);
        out.extend_from_slice(data);
        out.resize(out.len().div_ceil(512) * 512, 0);
    }
    out.extend_from_slice(&[0u8; 1024]);
    out
}

/// 素材の `FileData`: プレビューの PNG だけを入れた tar（`.layer` の代わりの詰め物つき）。
pub fn material_with_thumbnail(png: &[u8]) -> Vec<u8> {
    tar(&[
        ("thumbnail/thumbnail.png", png),
        ("layer.layer", b"proprietary"),
    ])
}

// ---------------- BLOB ----------------

/// 影響元の BLOB（公開の解析の形: ヘッダー 40 か 44 バイト、旗は 3 番目、筆圧の最小値（%）は 4 番目、曲線は (12, 数, 16) と f64 の対）。
pub fn effector(
    header_len: usize,
    flags: u32,
    pressure_min: u32,
    curves: &[&[(f64, f64)]],
) -> Vec<u8> {
    let mut out = Vec::new();
    let mut ints = vec![0u32; header_len / 4];
    ints[0] = header_len as u32;
    ints[1] = 1;
    ints[2] = flags;
    ints[3] = pressure_min;
    for i in ints {
        out.extend_from_slice(&i.to_be_bytes());
    }
    for curve in curves {
        for i in [12u32, curve.len() as u32, 16] {
            out.extend_from_slice(&i.to_be_bytes());
        }
        for (x, y) in *curve {
            out.extend_from_slice(&x.to_be_bytes());
            out.extend_from_slice(&y.to_be_bytes());
        }
    }
    out
}

fn utf16le(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect()
}

/// 素材の参照の BLOB。1 項目 = [元の場所, カタログの場所, 画像の名前]。
pub fn refs(items: &[[&str; 3]]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&8u32.to_be_bytes());
    out.extend_from_slice(&(items.len() as u32).to_be_bytes());
    for [original, catalog, image] in items {
        let first = utf16le(original);
        out.extend_from_slice(&(first.len() as u32 + 8).to_be_bytes());
        out.extend_from_slice(&(first.len() as u32).to_be_bytes());
        out.extend_from_slice(&first);
        for (kind, s) in [(1u32, catalog), (2u32, image)] {
            let b = utf16le(s);
            out.extend_from_slice(&kind.to_be_bytes());
            out.extend_from_slice(&(b.len() as u32).to_be_bytes());
            out.extend_from_slice(&b);
        }
        out.extend_from_slice(&0u32.to_be_bytes());
    }
    out
}
