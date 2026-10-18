//! `Variant` の表の中の 2 種類の BLOB: 影響元（`*Effector`）と素材の参照（`BrushPatternImageArray`・`TextureImage`）。
//!
//! どちらも CLIP STUDIO が公開していない形で、公開の解析（docs/BRUSH_IMPORT.md に出どころ）から分かる範囲だけを読む。
//! 数値はビッグエンディアン。読めない所に出会ったら、そこまでに読めた分を返す（黙って作り話で埋めない）。信頼できない入力なので、
//! 個数・長さは残りのバイト数に入ると確かめ、上限を置く。

use super::super::reader::Reader;
use super::super::short_text;

/// 影響元の曲線の点の数の上限。
pub(super) const MAX_CURVE_POINTS: usize = 64;
/// 読む曲線の数の上限。
const MAX_CURVES: usize = 4;

/// 影響元（効果）の BLOB。
#[derive(Debug, PartialEq)]
pub(super) struct Effector {
    /// 使う入力の旗: 0x10 筆圧・0x20 傾き・0x40 速さ・0x80 ランダム。
    pub flags: u32,
    /// 入力ごとの最小値（0〜1。筆圧・傾き・速さ・ランダムの順。ヘッダーの 4〜7 番目の整数の %。ヘッダーに無ければ 0）。
    pub minimums: [f64; 4],
    /// 入力ごとの最小値を符号つきの整数として読んだもの（−1〜1。色相・彩度・明度の影響元は −100 まで取る。`minimums` は負の数を 0〜1 に
    /// 丸めた値で、読み方を変えない）。
    pub signed_minimums: [f64; 4],
    /// 傾きの最大値（ヘッダーの 11 番目の整数の %。1 = 100%。100% を超える値もそのまま。ヘッダーが 11 個の整数に満たなければ None）。
    /// 0 は、ヘッダーの 0 埋めと見分けられない。実物の .sut では、ブラシサイズが 150（傾きの最大値を 100 に替えると 100 になる）、
    /// 不透明度は 500、ブラシ濃度は 100 のまま。
    pub tilt_max: Option<f64>,
    /// ヘッダーと読めた曲線のあとに残ったバイト数（速さ・ランダムの曲線が付くと 44 バイト伸びる。0 なら曲線は付いていない）。
    pub trailing: usize,
    /// 曲線の点（x・y とも 0〜1 を期待するが、ここでは丸めない）。並びはファイルのまま。どの曲線がどの入力のものかは
    /// [`Effector::pressure_curve`]・[`Effector::tilt_curve`]。
    pub curves: Vec<Vec<(f64, f64)>>,
    /// ヘッダー（長さ 44 か 48）の 9・10 番目の整数: 筆圧の曲線と、傾きの曲線の、バイト数（0 は曲線が無い）。ヘッダーがそのどちらでもないと
    /// `None`。実物の .sut では、曲線の大きさ（12 + 16 × 点の数）とちょうど合い、傾きだけを使う設定には 2 つ目の枠だけに曲線が付く。
    slots: Option<[usize; 2]>,
}

impl Effector {
    pub const PRESSURE: u32 = 0x10;
    pub const TILT: u32 = 0x20;
    pub const SPEED: u32 = 0x40;
    pub const RANDOM: u32 = 0x80;

    /// 曲線 1 つのバイト数（3 つの整数 + 点ごとに x・y の `f64`）。
    fn curve_bytes(points: &[(f64, f64)]) -> usize {
        12 + 16 * points.len()
    }

    /// ヘッダーの 2 つの枠（筆圧・筆圧でない 2 つ目）と曲線の並びが、バイト数まで合っているとき、枠ごとの曲線の番号。
    fn assigned(&self) -> Option<[Option<usize>; 2]> {
        let slots = self.slots.filter(|s| s.iter().any(|&n| n > 0))?;
        let mut next = 0;
        let mut out = [None, None];
        for (slot, &bytes) in slots.iter().enumerate() {
            if bytes == 0 {
                continue;
            }
            let curve = self.curves.get(next)?;
            if Self::curve_bytes(curve) != bytes {
                return None;
            }
            out[slot] = Some(next);
            next += 1;
        }
        (next == self.curves.len()).then_some(out)
    }

    /// 筆圧の曲線。枠が読めて曲線と合っていれば枠で決め（筆圧の曲線が無ければ None）、読めなければ最初の曲線（点の数 2 以上）。
    pub fn pressure_curve(&self) -> Option<&[(f64, f64)]> {
        match self.assigned() {
            Some([pressure, _]) => pressure.map(|i| self.curves[i].as_slice()),
            None => self.curves.first().map(Vec::as_slice),
        }
    }

    /// 2 つ目の枠の曲線（傾きの曲線）。枠が読めて曲線と合っているときだけ Some（中の None は、枠が読めて傾きの曲線が無い）。
    pub fn tilt_curve(&self) -> Option<Option<&[(f64, f64)]>> {
        Some(self.assigned()?[1].map(|i| self.curves[i].as_slice()))
    }
}

/// 影響元の BLOB を読む。頭のヘッダー（長さ 40・44・48。先頭の 4 バイトがその長さ）が読めなければ None。
/// ヘッダーの 3 番目の整数が旗、4〜7 番目が筆圧・傾き・速さ・ランダムの最小値（%）。続く曲線は (12, 点の数, 16) の 3 つの整数と、点ごとに
/// x・y の f64。速さ・ランダムを使うと全体が 44 バイト伸びる（中身の形が分からないので、その分は読まない）。
pub(super) fn parse_effector(bytes: &[u8]) -> Option<Effector> {
    let mut r = Reader::new(bytes);
    let header = r.u32().ok()? as usize;
    if !(16..=64).contains(&header) || !header.is_multiple_of(4) || header > bytes.len() {
        return None;
    }
    let mut ints = vec![header as u32];
    for _ in 1..header / 4 {
        ints.push(r.u32().ok()?);
    }
    let flags = ints[2];
    let percent = |i: usize| {
        ints.get(i)
            .map_or(0.0, |&v| (v as f64 / 100.0).clamp(0.0, 1.0))
    };
    let minimums = [percent(3), percent(4), percent(5), percent(6)];
    let signed = |i: usize| {
        ints.get(i)
            .map_or(0.0, |&v| (v as i32 as f64 / 100.0).clamp(-1.0, 1.0))
    };
    let signed_minimums = [signed(3), signed(4), signed(5), signed(6)];
    let tilt_max = ints.get(10).map(|&v| v as f64 / 100.0);
    let slots = (header == 44 || header == 48).then(|| [ints[8] as usize, ints[9] as usize]);
    let mut curves = Vec::new();
    // ヘッダーと、最後まで読めた曲線のあとに残ったバイト数
    let mut rest = r.remaining();
    while curves.len() < MAX_CURVES && r.remaining() >= 12 {
        let (Ok(_), Ok(count), Ok(_)) = (r.u32(), r.u32(), r.u32()) else {
            break;
        };
        let count = count as usize;
        if !(2..=MAX_CURVE_POINTS).contains(&count) || count * 16 > r.remaining() {
            break;
        }
        let mut points = Vec::with_capacity(count);
        for _ in 0..count {
            let (Ok(x), Ok(y)) = (r.f64(), r.f64()) else {
                return Some(Effector {
                    flags,
                    minimums,
                    signed_minimums,
                    tilt_max,
                    trailing: rest,
                    curves,
                    slots,
                });
            };
            points.push((x, y));
        }
        if points
            .iter()
            .any(|&(x, y)| !x.is_finite() || !y.is_finite())
        {
            break;
        }
        curves.push(points);
        rest = r.remaining();
    }
    Some(Effector {
        flags,
        minimums,
        signed_minimums,
        tilt_max,
        trailing: rest,
        curves,
        slots,
    })
}

/// 素材の参照 1 つ分（元の場所・カタログの場所・画像の名前などの文字列。どれがどれかは分けない）。
#[derive(Debug, PartialEq)]
pub(super) struct RefItem {
    pub names: Vec<String>,
}

/// 素材の参照の BLOB の読み取り結果。
#[derive(Debug, PartialEq)]
pub(super) struct Refs {
    pub items: Vec<RefItem>,
    /// 最後まで形のとおりに読めたか（読めなければ `items` の数は当てにしない）。
    pub complete: bool,
}

/// 項目の数の上限と、1 項目の文字列の数の上限・1 つの文字列の長さ（バイト）の上限。
const MAX_REFS: usize = 1024;
const MAX_REF_STRINGS: usize = 16;
const MAX_REF_BYTES: usize = 4096;

fn utf16le(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes(*c))
        .collect();
    short_text(&String::from_utf16_lossy(&units), 256)
}

fn text(r: &mut Reader) -> Option<String> {
    let len = r.u32().ok()? as usize;
    if len > MAX_REF_BYTES || len > r.remaining() {
        return None;
    }
    Some(utf16le(r.bytes(len).ok()?))
}

/// 素材の参照の BLOB を読む。先頭の整数（8 のことが多い）と項目の数、項目ごとに「大きさ・名前の長さ・名前（UTF-16LE。長さは
/// バイト数）」と、種類（0 で終わり）・長さ・文字列の並び。
pub(super) fn parse_refs(bytes: &[u8]) -> Refs {
    let mut items = Vec::new();
    let mut r = Reader::new(bytes);
    let mut complete = false;
    'read: {
        if r.u32().is_err() {
            break 'read;
        }
        let Ok(count) = r.u32() else {
            break 'read;
        };
        if count as usize > MAX_REFS {
            break 'read;
        }
        for _ in 0..count {
            if r.u32().is_err() {
                break 'read;
            }
            let Some(first) = text(&mut r) else {
                break 'read;
            };
            let mut names = vec![first];
            loop {
                let Ok(kind) = r.u32() else {
                    // 最後の項目はここで終わることがある
                    items.push(RefItem { names });
                    complete = items.len() == count as usize;
                    break 'read;
                };
                if kind == 0 {
                    break;
                }
                let Some(name) = text(&mut r) else {
                    items.push(RefItem { names });
                    break 'read;
                };
                if names.len() < MAX_REF_STRINGS {
                    names.push(name);
                }
            }
            items.push(RefItem { names });
        }
        complete = true;
    }
    Refs { items, complete }
}

#[cfg(test)]
pub(super) mod build {
    //! 試験用の BLOB の組み立て（公開の解析の形）。

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

    /// 参照の並び。1 項目 = [元の場所, カタログの場所, 画像の名前]。
    pub fn refs(items: &[[&str; 3]]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&8u32.to_be_bytes());
        out.extend_from_slice(&(items.len() as u32).to_be_bytes());
        for [original, catalog, image] in items {
            let first = utf16le(original);
            let body_len = first.len();
            out.extend_from_slice(&(body_len as u32 + 8).to_be_bytes());
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
}

#[cfg(test)]
mod tests {
    use super::build::*;
    use super::*;

    #[test]
    fn effector_reads_flags_minimum_and_curves() {
        let pts: &[(f64, f64)] = &[(0.0, 0.0), (0.5, 0.8), (1.0, 1.0)];
        for header in [40usize, 44] {
            let blob = effector(header, 0x10, 25, &[pts, &[(0.0, 1.0), (1.0, 0.0)]]);
            let e = parse_effector(&blob).unwrap();
            assert_eq!(e.flags, Effector::PRESSURE);
            assert_eq!(e.minimums, [0.25, 0.0, 0.0, 0.0]);
            assert_eq!(e.curves.len(), 2);
            assert_eq!(e.curves[0], pts);
        }
        let blob = effector(44, 0x10 | 0x20 | 0x80, 0, &[]);
        let e = parse_effector(&blob).unwrap();
        assert_eq!(e.flags, 0xB0);
        assert!(e.curves.is_empty());
    }

    #[test]
    fn effector_reads_a_negative_minimum_as_signed_and_keeps_the_unsigned_one() {
        let mut blob = effector(48, 0x80, 0, &[]);
        // 7 番目の整数（ランダムの最小値）に −100、4 番目（筆圧）に 50
        blob[24..28].copy_from_slice(&(-100i32).to_be_bytes());
        blob[12..16].copy_from_slice(&50u32.to_be_bytes());
        let e = parse_effector(&blob).unwrap();
        assert_eq!(e.signed_minimums, [0.5, 0.0, 0.0, -1.0]);
        // 今の読み方（負の数は大きな正の数になり 1 に丸まる）は変えない
        assert_eq!(e.minimums, [0.5, 0.0, 0.0, 1.0]);
        assert_eq!(e.trailing, 0);
        // 範囲を超える値は丸める
        blob[24..28].copy_from_slice(&(-250i32).to_be_bytes());
        assert_eq!(parse_effector(&blob).unwrap().signed_minimums[3], -1.0);
        // 曲線の後ろに残ったバイトを数える
        let mut longer = effector(44, 0x10, 0, &[&[(0.0, 0.0), (1.0, 1.0)]]);
        longer.extend_from_slice(&[0; 44]);
        assert_eq!(parse_effector(&longer).unwrap().trailing, 44);
    }

    #[test]
    fn effector_stops_at_what_it_cannot_read_and_never_panics() {
        let pts: &[(f64, f64)] = &[(0.0, 0.0), (1.0, 1.0)];
        let blob = effector(44, 0x10, 0, &[pts]);
        assert!(parse_effector(&[]).is_none());
        assert!(parse_effector(&blob[..10]).is_none());
        // 曲線の途中で切れても、読めた所まで
        for len in 0..blob.len() {
            let _ = parse_effector(&blob[..len]);
        }
        let cut = parse_effector(&blob[..blob.len() - 5]).unwrap();
        assert!(cut.curves.is_empty());
        // 点の数が巨大・NaN の点
        let mut huge = blob.clone();
        huge[44 + 4..44 + 8].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(parse_effector(&huge).unwrap().curves.is_empty());
        let mut nan = blob.clone();
        nan[44 + 12..44 + 20].copy_from_slice(&f64::NAN.to_be_bytes());
        assert!(parse_effector(&nan).unwrap().curves.is_empty());
        // ヘッダーの長さが範囲外
        let mut bad = blob.clone();
        bad[..4].copy_from_slice(&7u32.to_be_bytes());
        assert!(parse_effector(&bad).is_none());
        bad[..4].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(parse_effector(&bad).is_none());
    }

    #[test]
    fn refs_read_each_item_with_its_strings() {
        let blob = refs(&[
            ["C:\\a\\tip1.png", "cat/uuid-1", "tip1"],
            ["tip2.png", "cat/uuid-2", "tip2"],
        ]);
        let r = parse_refs(&blob);
        assert!(r.complete);
        assert_eq!(r.items.len(), 2);
        assert_eq!(r.items[0].names, ["C:\\a\\tip1.png", "cat/uuid-1", "tip1"]);
        assert_eq!(r.items[1].names[2], "tip2");
    }

    #[test]
    fn refs_never_panic_and_report_what_was_read() {
        let blob = refs(&[["a", "b", "c"]]);
        for len in 0..blob.len() {
            let r = parse_refs(&blob[..len]);
            assert!(r.items.len() <= 1);
        }
        let mut huge = blob.clone();
        huge[4..8].copy_from_slice(&u32::MAX.to_be_bytes());
        let r = parse_refs(&huge);
        assert!(!r.complete && r.items.is_empty());
        let mut long = blob.clone();
        long[12..16].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(!parse_refs(&long).complete);
        assert!(!parse_refs(&[]).complete);
    }
}
