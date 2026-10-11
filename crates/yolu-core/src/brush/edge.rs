//! 丸い筆先の縁のアンチエイリアスと、小さなダブの濃さ（拡張: C# に無い）。
//!
//! 覆いは画素（3D はテクセル）の中心の 1 点で測る。今の式（段「なし」）の縁は、規格化した距離 d が硬さ h から 1 までの smoothstep
//! で、その幅は半径 ×（1 − 硬さ）だけなので、硬い筆先や細い線では縁が画素の段になり、1 画素より小さいダブは当たった画素に 1 画素分の
//! 濃さを置く。ほかの段は、縁の帯（描く先のテクスチャの画素で弱 0.5・中 1・強 2）を次の形で当てる。
//!
//! - ぼかしの幅は max(半径 ×（1 − 硬さ）, 帯)。帯は今の式で覆いが半分になる半径（硬さ 1 ならもとの半径）を中心に置き、
//!   見た目の太さを変えない。帯がぼかしの幅以下で、濃さが 1 のダブ（柔らかい筆先・大きな筆先）は今の式そのもの（同じバイト）。
//! - 規格化した距離のテクセルの空間での勾配 g（真円は 1 / 半径）で、帯を d の単位（帯 × g）へ直す。潰した丸は画素ごとの g で、
//!   帯は縁に垂直な向きの幅になる。潰した丸の帯の式の距離は、覆いが半分になる楕円の外接の箱の外の距離で下から押さえる
//!   （[`box_floor`]。一次の近似が細い楕円の斜めで外の距離を小さく見積もり、帯が長い軸の先の外へ伸びるのを止める）。3D は、
//!   テクセル → 画面の写し J の J·Jᵀ（[`TexelMetric`]）で、画面の距離の勾配をテクセルの空間の勾配へ直す（その場所でのテクセル
//!   1 つ分の画面の大きさ）。
//! - 小さなダブ: 覆いが半分になる楕円の半径 m（軸ごと、テクセル）が帯の半分 ρ より小さい軸は、その軸を ρ まで広げ、その軸の濃さを
//!   m / ρ にする（[`widen_axes`]。面積に見合う濃さ。1 画素分のスタンプにしない）。弱の帯（0.5）は、m が 1 より小さいダブで 1 まで広げる
//!   （0.5 のままでは、画素の間を通る小さなダブがどの画素の中心にもかからず消える）。
//! - 画像の筆先は、小さなダブの濃さと広げだけ（画像の補間は今のまま）: 画像の半分の幅がρより小さい軸を ρ まで広げ、その軸の濃さを
//!   掛ける。

/// アンチエイリアスの段（[`super::BrushSettings::anti_alias`]）。並びは保存の番号の順。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum AntiAlias {
    /// 縁の帯なし（今の式）。
    #[default]
    None,
    Weak,
    Medium,
    Strong,
}

impl AntiAlias {
    /// 全部の段（保存の番号の順）。
    pub const ALL: [AntiAlias; 4] = [
        AntiAlias::None,
        AntiAlias::Weak,
        AntiAlias::Medium,
        AntiAlias::Strong,
    ];

    /// 縁の帯の幅（描く先のテクスチャの画素。なしは 0）。
    pub fn band(self) -> f64 {
        match self {
            AntiAlias::None => 0.0,
            AntiAlias::Weak => 0.5,
            AntiAlias::Medium => 1.0,
            AntiAlias::Strong => 2.0,
        }
    }

    /// ファイルに書く名前。
    pub fn id(self) -> &'static str {
        match self {
            AntiAlias::None => "none",
            AntiAlias::Weak => "weak",
            AntiAlias::Medium => "medium",
            AntiAlias::Strong => "strong",
        }
    }

    /// 名前から（知らない名前は None）。
    pub fn from_id(id: &str) -> Option<AntiAlias> {
        AntiAlias::ALL.into_iter().find(|a| a.id() == id)
    }

    /// 保存の番号（0〜3）。
    pub fn index(self) -> u8 {
        self as u8
    }

    /// 保存の番号から（範囲の外は None）。
    pub fn from_index(index: u8) -> Option<AntiAlias> {
        AntiAlias::ALL.get(index as usize).copied()
    }
}

/// ダブ 1 つ（3D は投影の画素 1 つ）の縁: 使う帯の幅（ダブの座標の単位。0 は今の式）と濃さ（1 は今のまま）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Edge {
    pub band: f64,
    pub density: f64,
}

impl Edge {
    pub(crate) const OFF: Edge = Edge {
        band: 0.0,
        density: 1.0,
    };

    /// 今の式のままか（帯なし・濃さ 1）。
    pub(crate) fn is_off(&self) -> bool {
        self.band == 0.0 && self.density == 1.0
    }
}

/// 段の帯 w（テクセル）・硬さ h・ダブの楕円のテクセルの空間での半径 (a, b) から、使う帯の幅（テクセル）と濃さ。w が 0 なら [`Edge::OFF`]。
pub(crate) fn band_and_density(w: f64, h: f64, a: f64, b: f64) -> Edge {
    if w <= 0.0 {
        return Edge::OFF;
    }
    let mid = 1.0 - (1.0 - h) * 0.5;
    let (ma, mb) = (a * mid, b * mid);
    let small = if ma < mb { ma } else { mb };
    // 細い線（覆いが半分になる半径が 1 より小さい）は帯を 1 テクセルまで広げる（半径 1 で 0.5、0.5 以下で 1。間は線形）
    let widen = (1.5 - small).min(1.0);
    let band = if widen > w { widen } else { w };
    let rho = band * 0.5;
    let fa = if ma < rho { ma / rho } else { 1.0 };
    let fb = if mb < rho { mb / rho } else { 1.0 };
    Edge {
        band,
        density: fa * fb,
    }
}

/// 覆いが半分になる半径が帯の半分 ρ より細い軸を ρ まで広げる倍率（[`band_and_density`] の帯で。広げない軸は 1）。
/// 短い軸を広げると、規格化した距離の帯がその軸で 2 × (1 + h) / 2 を超えず、一次の距離の近似が遠くまで伸びない。
pub(crate) fn widen_axes(e: &Edge, h: f64, a: f64, b: f64) -> (f64, f64) {
    let mid = 1.0 - (1.0 - h) * 0.5;
    let rho = e.band * 0.5;
    let f = |t: f64| {
        let m = t * mid;
        if m < rho {
            rho / m
        } else {
            1.0
        }
    };
    (f(a), f(b))
}

/// a が b より大きいか（どちらかが NaN なら偽。否定して「以下か NaN」に使う）。
#[inline]
pub(crate) fn above(a: f64, b: f64) -> bool {
    a > b
}

/// 帯の幅 band（規格化した距離の単位）での、縁の帯の内の端と外の端（規格化した距離）。帯がぼかしの幅以下なら (h, 1)。
pub(crate) fn bounds(h: f64, band: f64) -> (f64, f64) {
    let soft = 1.0 - h;
    if !above(band, soft) {
        return (h, 1.0);
    }
    let mid = 1.0 - soft * 0.5;
    let half = band * 0.5;
    let m = if half > mid { half } else { mid };
    (m - half, m + half)
}

/// 外の端が outer 以下になる、いちばん広い帯（規格化した距離の単位。[`bounds`] の逆。outer は 1 以上）。
pub(crate) fn band_for_outer(h: f64, outer: f64) -> f64 {
    let mid = 1.0 - (1.0 - h) * 0.5;
    if outer <= 2.0 * mid {
        2.0 * (outer - mid)
    } else {
        outer
    }
}

/// 楕円（半軸 radius・minor）の規格化した距離 d の、画素あたりの勾配の大きさ。u・v は回した座標を半軸で割った値（d = √(u² + v²)）。
/// 中心（d = 0）は短い軸の向きの値。
pub(crate) fn ellipse_gradient(u: f64, v: f64, d: f64, radius: f64, minor: f64) -> f64 {
    if d > 0.0 {
        ((u / radius) * (u / radius) + (v / minor) * (v / minor)).sqrt() / d
    } else {
        1.0 / minor
    }
}

/// 潰した丸の外側の距離の下からの押さえ: 回した座標 (ru, rv)（画素）が、覆いが半分になる楕円の外接の箱（半分の幅 bu・bv）から
/// どれだけ外にあるか（箱の中は 0）を、規格化した距離 d の単位で d の下限にした値 `max(d, mid + g × 箱の外の距離)`。距離の一次の
/// 近似（d / 勾配）は細い楕円の斜めで外の距離を小さく見積もり、帯が長い軸の先の外へ伸びるので、本当の距離の下限で押さえる。
/// 半分の楕円の内側（d ≤ mid）では箱の外の距離は 0 なので、値は d のまま。
pub(crate) fn box_floor(
    d: f64,
    g: f64,
    mid: f64,
    (ru, rv): (f64, f64),
    (bu, bv): (f64, f64),
) -> f64 {
    let du = ru.abs() - bu;
    let dv = rv.abs() - bv;
    let outside = if du > dv { du } else { dv };
    let outside = if outside > 0.0 { outside } else { 0.0 };
    let floor = mid + g * outside;
    if floor > d {
        floor
    } else {
        d
    }
}

/// 丸の覆い（f64。対称の写し・デュアルの対称・3D）。d は規格化した距離、band は帯（規格化した単位）、density は濃さ。
/// 帯がぼかしの幅以下で濃さ 1 なら今の式（smoothstep((1 − d) / (1 − h))）。
pub(crate) fn cover64(d: f64, h: f64, band: f64, density: f64) -> f64 {
    cover64_floored(d, d, h, band, density)
}

/// [`cover64`] の、帯の式にだけ下から押さえた距離 `d_band`（[`box_floor`]）を使う形（今の式の画素は d のまま）。
pub(crate) fn cover64_floored(d: f64, d_band: f64, h: f64, band: f64, density: f64) -> f64 {
    let soft = 1.0 - h;
    if density == 1.0 && !above(band, soft) {
        if d > 1.0 {
            return 0.0;
        }
        if d <= h {
            return 1.0;
        }
        let t = (1.0 - d) / soft;
        return t * t * (3.0 - 2.0 * t);
    }
    let width = if band > soft { band } else { soft };
    let mid = 1.0 - soft * 0.5;
    let half = band * 0.5;
    let m = if half > mid { half } else { mid };
    let outer = m + width * 0.5;
    let d = d_band;
    if !above(outer, d) {
        return 0.0;
    }
    let t = (outer - d) / width;
    let t = if t > 1.0 { 1.0 } else { t };
    t * t * (3.0 - 2.0 * t) * density
}

/// テクセル 1 つの、ダブの座標の枠での大きさ: テクセル → 枠の写し J（2 × 2）の J·Jᵀ の 3 つの値（xx・xy・yy）。2D は単位行列。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TexelMetric {
    pub xx: f64,
    pub xy: f64,
    pub yy: f64,
}

impl TexelMetric {
    pub(crate) const IDENTITY: TexelMetric = TexelMetric {
        xx: 1.0,
        xy: 0.0,
        yy: 1.0,
    };

    /// J の列（テクセルの x・y の 1 つ分が枠でどれだけか）から。
    pub(crate) fn from_columns(jx: (f64, f64), jy: (f64, f64)) -> TexelMetric {
        TexelMetric {
            xx: jx.0 * jx.0 + jy.0 * jy.0,
            xy: jx.0 * jx.1 + jy.0 * jy.1,
            yy: jx.1 * jx.1 + jy.1 * jy.1,
        }
    }

    /// 使える値か（有限で、潰れていない）。
    pub(crate) fn usable(&self) -> bool {
        let det = self.xx * self.yy - self.xy * self.xy;
        self.xx.is_finite()
            && self.xy.is_finite()
            && self.yy.is_finite()
            && self.xx > 0.0
            && self.yy > 0.0
            && det > 1e-12 * self.xx * self.yy
    }

    /// 枠の勾配 (gx, gy) の、テクセルの空間での大きさ |Jᵀ·g|。
    pub(crate) fn gradient(&self, gx: f64, gy: f64) -> f64 {
        (gx * gx * self.xx + 2.0 * gx * gy * self.xy + gy * gy * self.yy)
            .max(0.0)
            .sqrt()
    }

    /// 固有値（大きい方、小さい方）: 枠で、テクセル 1 つがいちばん長い・短い向きの長さの 2 乗。
    pub(crate) fn eigen(&self) -> (f64, f64) {
        let mean = (self.xx + self.yy) * 0.5;
        let diff = (self.xx - self.yy) * 0.5;
        let r = (diff * diff + self.xy * self.xy).sqrt();
        (mean + r, (mean - r).max(0.0))
    }

    /// 枠のベクトル (vx, vy) のテクセルの長さ |J⁻¹·v|。
    pub(crate) fn texel_length(&self, vx: f64, vy: f64) -> f64 {
        let det = self.xx * self.yy - self.xy * self.xy;
        // (J·Jᵀ)⁻¹ = [[yy, −xy], [−xy, xx]] / det
        ((vx * vx * self.yy - 2.0 * vx * vy * self.xy + vy * vy * self.xx) / det)
            .max(0.0)
            .sqrt()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_round_trip_by_name_and_index() {
        for a in AntiAlias::ALL {
            assert_eq!(AntiAlias::from_id(a.id()), Some(a));
            assert_eq!(AntiAlias::from_index(a.index()), Some(a));
        }
        assert_eq!(AntiAlias::from_id("soft"), None);
        assert_eq!(AntiAlias::from_index(4), None);
        assert_eq!(AntiAlias::default(), AntiAlias::None);
        assert_eq!(
            AntiAlias::ALL.map(AntiAlias::band),
            [0.0, 0.5, 1.0, 2.0],
            "帯の幅は段の順に広がる"
        );
    }

    #[test]
    fn a_band_no_wider_than_the_soft_edge_is_the_old_formula() {
        for h in [0.0, 0.3, 0.8] {
            for i in 0..=100 {
                let d = i as f64 / 80.0;
                let old = if d > 1.0 {
                    0.0
                } else if d <= h {
                    1.0
                } else {
                    let t = (1.0 - d) / (1.0 - h);
                    t * t * (3.0 - 2.0 * t)
                };
                assert_eq!(cover64(d, h, 0.0, 1.0), old);
                assert_eq!(cover64(d, h, (1.0 - h) * 0.5, 1.0), old);
                assert_eq!(cover64(d, h, 1.0 - h, 1.0), old);
            }
        }
    }

    #[test]
    fn the_band_is_centred_on_the_half_cover_radius() {
        // 硬さ 1 の丸の帯の中心はもとの半径（覆い 0.5）、硬さ 0.8 でぼかしより広い帯は今の式の半分の半径
        let half = |h: f64, band: f64| {
            let (lo, hi) = bounds(h, band);
            let mid = (lo + hi) * 0.5;
            assert!((cover64(mid, h, band, 1.0) - 0.5).abs() < 1e-12);
            mid
        };
        assert!((half(1.0, 0.1) - 1.0).abs() < 1e-12);
        assert!((half(0.8, 0.5) - 0.9).abs() < 1e-12);
        // 帯は対称: 中心から等しく離れた所の覆いは足して 1
        for k in 1..10 {
            let x = k as f64 * 0.01;
            let s = cover64(1.0 - x, 1.0, 0.2, 1.0) + cover64(1.0 + x, 1.0, 0.2, 1.0);
            assert!((s - 1.0).abs() < 1e-12, "{s}");
        }
    }

    #[test]
    fn cover_falls_monotonically_and_is_zero_outside() {
        for (h, band, k) in [(1.0, 0.1, 1.0), (0.5, 0.9, 1.0), (1.0, 3.0, 0.3)] {
            let (_, outer) = bounds(h, band);
            let mut last = f64::INFINITY;
            for i in 0..=400 {
                let d = i as f64 / 100.0;
                let c = cover64(d, h, band, k);
                assert!(c <= last + 1e-15 && (0.0..=1.0).contains(&c));
                if d >= outer {
                    assert_eq!(c, 0.0);
                }
                last = c;
            }
        }
    }

    #[test]
    fn small_dabs_get_a_density_for_their_area() {
        // 中（帯 1）の硬い丸: 半径 0.5 以上は濃さ 1、それより小さいと半径²·4
        for (r, k) in [(2.0, 1.0), (0.5, 1.0), (0.3, 0.36), (0.1, 0.04)] {
            let e = band_and_density(1.0, 1.0, r, r);
            assert_eq!(e.band, 1.0);
            assert!((e.density - k).abs() < 1e-12, "{r} {}", e.density);
        }
        // 細い楕円は短い軸だけ薄くなる
        let e = band_and_density(1.0, 1.0, 5.0, 0.25);
        assert!((e.density - 0.5).abs() < 1e-12);
        // 弱は太い線では 0.5 のまま、細い線では 1 まで広げる（間は続く）
        assert_eq!(band_and_density(0.5, 1.0, 3.0, 3.0).band, 0.5);
        assert_eq!(band_and_density(0.5, 1.0, 1.0, 1.0).band, 0.5);
        assert_eq!(band_and_density(0.5, 1.0, 0.5, 0.5).band, 1.0);
        assert!((band_and_density(0.5, 1.0, 0.75, 0.75).band - 0.75).abs() < 1e-12);
        assert_eq!(band_and_density(0.0, 1.0, 0.1, 0.1), Edge::OFF);
    }

    #[test]
    fn the_metric_turns_frame_lengths_into_texels() {
        // テクセル 1 つが枠で x に 2、y に 0.5（軸に沿う）
        let m = TexelMetric::from_columns((2.0, 0.0), (0.0, 0.5));
        assert!(m.usable());
        assert!((m.gradient(1.0, 0.0) - 2.0).abs() < 1e-12);
        assert!((m.gradient(0.0, 1.0) - 0.5).abs() < 1e-12);
        assert!((m.texel_length(4.0, 0.0) - 2.0).abs() < 1e-12);
        assert!((m.texel_length(0.0, 1.0) - 2.0).abs() < 1e-12);
        let (big, small) = m.eigen();
        assert!(
            (big - 4.0).abs() < 1e-12 && (small - 0.25).abs() < 1e-12,
            "{big} {small}"
        );
        assert!(!TexelMetric::from_columns((1.0, 1.0), (2.0, 2.0)).usable());
    }
}
