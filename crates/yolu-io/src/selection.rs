use crate::{check, Error, NativeDocument, Result};
use yolu_core::{SelectionMask, TileCoord};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectionTile {
    pub x: i32,
    pub y: i32,
    pub amounts: Vec<u8>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    width: i32,
    height: i32,
    tile_size: i32,
    tiles: Vec<SelectionTile>,
}
impl Selection {
    pub fn width(&self) -> i32 {
        self.width
    }
    pub fn height(&self) -> i32 {
        self.height
    }
    pub fn tile_size(&self) -> i32 {
        self.tile_size
    }
    pub fn tiles(&self) -> &[SelectionTile] {
        &self.tiles
    }
    /// core の選択範囲から作る。C# の `SelectionBinary.Write` と同じに、量のあるタイルだけを (y, x) の順に並べる
    /// （同じ選択範囲はいつも同じバイト列になる）。
    pub fn from_core(mask: &SelectionMask) -> Result<Self> {
        let n = mask.tile_size() as usize * mask.tile_size() as usize;
        let mut tiles = Vec::new();
        for coord in mask.tile_coords() {
            let mut amounts = vec![0u8; n];
            mask.copy_tile(coord, &mut amounts)?;
            if amounts.iter().any(|a| *a != 0) {
                tiles.push(SelectionTile {
                    x: coord.x as i32,
                    y: coord.y as i32,
                    amounts,
                });
            }
        }
        Ok(Self {
            width: mask.width() as i32,
            height: mask.height() as i32,
            tile_size: mask.tile_size() as i32,
            tiles,
        })
    }
    /// core の選択範囲へ。何も選んでいない（タイルが 0 枚の）選択範囲は空の選択範囲になる
    /// （`Document::restore_selection` は空を選択なしにする）。
    pub fn to_core(&self) -> Result<SelectionMask> {
        Ok(SelectionMask::from_amount_tiles(
            self.width as u32,
            self.height as u32,
            self.tile_size as u32,
            self.tiles
                .iter()
                .map(|t| (TileCoord::new(t.x as u32, t.y as u32), t.amounts.clone())),
        )?)
    }
    pub fn read(b: &[u8], doc: &NativeDocument) -> Result<Self> {
        Self::read_sized(b, (doc.width(), doc.height(), doc.tile_size()))
    }
    /// `read` の、正本の大きさ（幅・高さ・タイル寸法）だけを渡す形。
    pub(crate) fn read_sized(b: &[u8], (dw, dh, dts): (i32, i32, i32)) -> Result<Self> {
        check(b.get(..4) == Some(b"YLSL"), "選択範囲の識別子が不正です")?;
        let mut at = 4;
        fn int(b: &[u8], at: &mut usize) -> Result<i32> {
            let s = b
                .get(*at..*at + 4)
                .ok_or_else(|| Error::InvalidData("選択範囲が途中で切れています".into()))?;
            *at += 4;
            Ok(i32::from_le_bytes(s.try_into().unwrap()))
        }
        check(int(b, &mut at)? == 1, "選択範囲の版が未対応です")?;
        let w = int(b, &mut at)?;
        let h = int(b, &mut at)?;
        let ts = int(b, &mut at)?;
        let n = int(b, &mut at)?;
        check(
            w == dw && h == dh && ts == dts,
            "選択範囲と正本の大きさが一致しません",
        )?;
        let cols = (w + ts - 1) / ts;
        let rows = (h + ts - 1) / ts;
        check(n >= 0 && n <= cols * rows, "選択範囲のタイル数が不正です")?;
        check(
            b.len() == 24 + n as usize * (8 + ts as usize * ts as usize),
            "選択範囲の長さが不正です",
        )?;
        let mut tiles = Vec::new();
        let mut previous = -1;
        for _ in 0..n {
            let x = int(b, &mut at)?;
            let y = int(b, &mut at)?;
            check(
                x >= 0 && x < cols && y >= 0 && y < rows && y * cols + x > previous,
                "選択範囲のタイルの位置または並びが不正です",
            )?;
            previous = y * cols + x;
            let amounts = b[at..at + (ts * ts) as usize].to_vec();
            at += amounts.len();
            check(amounts.iter().any(|a| *a != 0), "空の選択タイルです")?;
            for (i, a) in amounts.iter().enumerate() {
                check(
                    *a == 0 || (x * ts + i as i32 % ts < w && y * ts + i as i32 / ts < h),
                    "選択範囲の余白が0ではありません",
                )?;
            }
            tiles.push(SelectionTile { x, y, amounts });
        }
        Ok(Self {
            width: w,
            height: h,
            tile_size: ts,
            tiles,
        })
    }
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut b = b"YLSL".to_vec();
        for n in [
            1,
            self.width,
            self.height,
            self.tile_size,
            self.tiles.len() as i32,
        ] {
            b.extend(n.to_le_bytes());
        }
        for t in &self.tiles {
            b.extend(t.x.to_le_bytes());
            b.extend(t.y.to_le_bytes());
            b.extend(&t.amounts);
        }
        b
    }
}
