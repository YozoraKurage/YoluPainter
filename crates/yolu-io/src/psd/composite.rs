//! PSD DTO 上の参照合成。画素の式のみ core と共有し、文書モデルには依存しない。
use super::*;
use crate::{Error, Result};
use std::sync::atomic::{AtomicBool, Ordering};
use yolu_core::{
    blend::{blend, blend_rgb, clip_onto, fade},
    CoreError, Rgba8,
};
fn mode(m: BlendMode) -> yolu_core::BlendMode {
    yolu_core::BlendMode::from_index(m as u8).unwrap()
}
fn byte(v: f64) -> u8 {
    (v * 255.0 + 0.5).floor().clamp(0.0, 255.0) as u8
}
struct Entry<'a> {
    layer: &'a Layer,
    children: Vec<Entry<'a>>,
    clips: Vec<Entry<'a>>,
    /// 色調補正の 6 種の調整は、core の式（表を 1 回だけ作る）で当てる。ほかの調整と変換できない値は None。
    core: Option<yolu_core::AdjustmentSettings>,
}
fn entry(l: &Layer) -> Option<Entry<'_>> {
    if !l.visible || l.opacity == 0 {
        return None;
    }
    let children = match &l.kind {
        LayerKind::Group { children, .. } => {
            let p = plan(children);
            if p.is_empty() {
                return None;
            }
            p
        }
        _ => Vec::new(),
    };
    let core = match &l.kind {
        LayerKind::Adjustment(a) if is_core_only(a) => super::bridge::core_adjustment(a).ok(),
        _ => None,
    };
    Some(Entry {
        layer: l,
        children,
        clips: Vec::new(),
        core,
    })
}
/// core の式で当てる調整（C# 由来でない 6 種）か。
fn is_core_only(a: &Adjustment) -> bool {
    !matches!(
        a,
        Adjustment::Invert | Adjustment::Levels { .. } | Adjustment::HueSaturation { .. }
    )
}
fn plan(layers: &[Layer]) -> Vec<Entry<'_>> {
    let bottom: Vec<_> = layers.iter().rev().collect();
    let mut p = Vec::new();
    for (i, l) in bottom.iter().enumerate() {
        if i > 0 && l.clipping {
            continue;
        }
        let Some(mut e) = entry(l) else { continue };
        if !matches!(l.kind, LayerKind::Adjustment(_)) {
            for c in bottom.iter().skip(i + 1).take_while(|c| c.clipping) {
                if let Some(c) = entry(c) {
                    e.clips.push(c)
                }
            }
        }
        p.push(e)
    }
    p
}
fn amount(l: &Layer, x: i64, y: i64) -> f64 {
    let opacity = f64::from(l.opacity) / 255.0;
    match &l.mask {
        Some(m) if m.enabled => {
            opacity * (1.0 - f64::from(m.density) / 255.0 * (f64::from(255 - m.at(x, y)) / 255.0))
        }
        _ => opacity,
    }
}
fn pixel(l: &Layer, x: i64, y: i64) -> Rgba8 {
    if let LayerKind::SolidColor([r, g, b]) = l.kind {
        return Rgba8::new(r, g, b, 255);
    }
    let x = x - i64::from(l.left);
    let y = y - i64::from(l.top);
    if x < 0 || y < 0 || x >= i64::from(l.width) || y >= i64::from(l.height) {
        Rgba8::TRANSPARENT
    } else {
        Rgba8::from_slice(&l.pixels_rgba[(y * i64::from(l.width) + x) as usize * 4..])
    }
}
fn evaluate(p: &[Entry], mut below: Rgba8, x: i64, y: i64) -> Rgba8 {
    for e in p {
        let l = e.layer;
        let a = amount(l, x, y);
        if let LayerKind::Adjustment(adj) = &l.kind {
            below = adjust(adj, e.core.as_ref(), below, a, l.blend_mode);
            continue;
        }
        let group = matches!(l.kind, LayerKind::Group { .. });
        if group && l.blend_mode == BlendMode::PassThrough && e.clips.is_empty() {
            below = fade(below, evaluate(&e.children, below, x, y), a);
            continue;
        }
        let mut g = if group {
            evaluate(&e.children, Rgba8::TRANSPARENT, x, y)
        } else {
            pixel(l, x, y)
        };
        for c in &e.clips {
            let l = c.layer;
            if let LayerKind::Adjustment(adj) = &l.kind {
                g = adjust(adj, c.core.as_ref(), g, amount(l, x, y), l.blend_mode)
            } else {
                let px = if matches!(l.kind, LayerKind::Group { .. }) {
                    evaluate(&c.children, Rgba8::TRANSPARENT, x, y)
                } else {
                    pixel(l, x, y)
                };
                g = clip_onto(g, px, amount(l, x, y), mode(l.blend_mode))
            }
        }
        below = blend(below, g, a, mode(l.blend_mode))
    }
    below
}
/// 1 行ずつ重ねる。行の前に `before_row` を呼び、`Err` ならそこで止める（取消の確かめの間隔は画布の幅の画素。画素ごとの式は 1 つのスレッドで評価する）。
fn composite_with(d: &Document, mut before_row: impl FnMut() -> Result<()>) -> Result<Vec<u8>> {
    let p = plan(&d.layers);
    let mut out = vec![0; d.width as usize * d.height as usize * 4];
    for y in 0..d.height {
        before_row()?;
        for x in 0..d.width {
            let i = (y as usize * d.width as usize + x as usize) * 4;
            out[i..i + 4].copy_from_slice(
                &evaluate(&p, Rgba8::TRANSPARENT, i64::from(x), i64::from(y)).to_array(),
            )
        }
    }
    Ok(out)
}
/// 取消の旗を行ごとに見て重ねる（立っていたら `Cancelled`）。
pub(super) fn composite_cancellable(d: &Document, cancel: Option<&AtomicBool>) -> Result<Vec<u8>> {
    composite_with(d, || {
        if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
            Err(Error::Core(CoreError::Cancelled))
        } else {
            Ok(())
        }
    })
}
pub(super) fn composite(d: &Document) -> Vec<u8> {
    composite_cancellable(d, None).expect("取消の旗が無ければ止まらない")
}
pub(super) fn matte(p: &mut [u8]) {
    for c in p.as_chunks_mut::<4>().0 {
        let a = f64::from(c[3]) / 255.0;
        let white = 255.0 * (1.0 - a);
        for v in &mut c[..3] {
            *v = (f64::from(*v) * a + white + 0.5).floor().clamp(0.0, 255.0) as u8
        }
    }
}
fn adjust(
    a: &Adjustment,
    core: Option<&yolu_core::AdjustmentSettings>,
    c: Rgba8,
    amount: f64,
    m: BlendMode,
) -> Rgba8 {
    if amount <= 0.0 || c.a == 0 {
        return c;
    }
    if is_core_only(a) {
        // 色調補正の 6 種: core の調整の式で当て、合成モードと量は同じ式で混ぜる（変換できない値は何も変えない）
        return core.map_or(c, |s| s.composite(c, amount, mode(m)));
    }
    let rgb = match *a {
        Adjustment::Invert => [255 - c.r, 255 - c.g, 255 - c.b],
        Adjustment::Levels {
            input_black,
            input_white,
            output_black,
            output_white,
            gamma,
        } => [c.r, c.g, c.b].map(|v| {
            let ib = f64::from(input_black) / 255.0;
            let iw = f64::from(input_white) / 255.0;
            let ob = f64::from(output_black) / 255.0;
            let ow = f64::from(output_white) / 255.0;
            let t = ((f64::from(v) / 255.0 - ib) / (iw - ib))
                .clamp(0.0, 1.0)
                .powf(1.0 / (f64::from(gamma) / 100.0));
            byte(ob + t * (ow - ob))
        }),
        Adjustment::HueSaturation {
            hue,
            saturation,
            lightness,
        } => {
            let r = f64::from(c.r) / 255.0;
            let g = f64::from(c.g) / 255.0;
            let b = f64::from(c.b) / 255.0;
            let max = r.max(g.max(b));
            let min = r.min(g.min(b));
            let mut l = (max + min) / 2.0;
            let d = max - min;
            let mut h = 0.0;
            let mut s = 0.0;
            if d > 1e-12 {
                s = if l > 0.5 {
                    d / (2.0 - max - min)
                } else {
                    d / (max + min)
                };
                h = if max == r {
                    (g - b) / d + if g < b { 6.0 } else { 0.0 }
                } else if max == g {
                    (b - r) / d + 2.0
                } else {
                    (r - g) / d + 4.0
                };
                h /= 6.0
            }
            h += f64::from(hue) / 360.0;
            h -= h.floor();
            s = (s * (1.0 + f64::from(saturation) / 100.0)).clamp(0.0, 1.0);
            let light = f64::from(lightness) / 100.0;
            l = if light >= 0.0 {
                l + (1.0 - l) * light
            } else {
                l * (1.0 + light)
            };
            if s <= 0.0 {
                [byte(l); 3]
            } else {
                let q = if l < 0.5 {
                    l * (1.0 + s)
                } else {
                    l + s - l * s
                };
                let p = 2.0 * l - q;
                [h + 1.0 / 3.0, h, h - 1.0 / 3.0].map(|mut t| {
                    if t < 0.0 {
                        t += 1.0
                    }
                    if t > 1.0 {
                        t -= 1.0
                    }
                    byte(if t < 1.0 / 6.0 {
                        p + (q - p) * 6.0 * t
                    } else if t < 0.5 {
                        q
                    } else if t < 2.0 / 3.0 {
                        p + (q - p) * (2.0 / 3.0 - t) * 6.0
                    } else {
                        p
                    })
                })
            }
        }
        // 6 種は上で core の式に任せて戻っている
        _ => [c.r, c.g, c.b],
    };
    let dr = f64::from(c.r) / 255.0;
    let dg = f64::from(c.g) / 255.0;
    let db = f64::from(c.b) / 255.0;
    let (r, g, b) = blend_rgb(
        mode(m),
        dr,
        dg,
        db,
        f64::from(rgb[0]) / 255.0,
        f64::from(rgb[1]) / 255.0,
        f64::from(rgb[2]) / 255.0,
    );
    Rgba8::new(
        byte(dr + (r - dr) * amount),
        byte(dg + (g - dg) * amount),
        byte(db + (b - db) * amount),
        c.a,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32) -> Document {
        Document {
            width: w,
            height: h,
            layers: vec![Layer {
                id: 1,
                name: "塗り".into(),
                kind: LayerKind::SolidColor([10, 20, 30]),
                ..Layer::default()
            }],
            composite_rgba: None,
        }
    }

    /// 取消の確かめは 1 行ごと。止めた行より先は評価しない（大きな画布で、取消が効かない時間を作らない）。
    #[test]
    fn the_cancel_check_runs_before_every_row_and_stops_the_work() {
        let d = solid(8, 20);
        let mut rows = 0;
        let full = composite_with(&d, || {
            rows += 1;
            Ok(())
        })
        .unwrap();
        assert_eq!(rows, 20);
        assert_eq!(full, composite(&d));
        assert_eq!(&full[..4], [10, 20, 30, 255]);

        let mut rows = 0;
        let flag = AtomicBool::new(false);
        let err = composite_with(&d, || {
            rows += 1;
            if rows == 5 {
                flag.store(true, Ordering::Relaxed);
            }
            if flag.load(Ordering::Relaxed) {
                Err(Error::Core(CoreError::Cancelled))
            } else {
                Ok(())
            }
        })
        .unwrap_err();
        assert!(matches!(err, Error::Core(CoreError::Cancelled)), "{err}");
        assert_eq!(rows, 5, "旗が立った行で止まる");
    }

    #[test]
    fn a_raised_flag_stops_before_any_pixel_and_no_flag_never_stops() {
        let d = solid(64, 64);
        let flag = AtomicBool::new(true);
        assert!(matches!(
            composite_cancellable(&d, Some(&flag)),
            Err(Error::Core(CoreError::Cancelled))
        ));
        flag.store(false, Ordering::Relaxed);
        assert_eq!(
            composite_cancellable(&d, Some(&flag)).unwrap(),
            composite(&d)
        );
    }
}
