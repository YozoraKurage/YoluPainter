//! 色調補正の 6 種（グラデーションマップ・トーンカーブ・カラーバランス・明るさ/コントラスト・2 値化・ポスタリゼーション）の欄。
//! 調整の層（`layer_props`）とフィルターの段（`effect_props`）が同じ欄を使う（値は `yolu_core::ColorAdjust`）。変更が決まったときだけ
//! 新しい値を返し、`discrete` は 1 回の操作で決まる変更（曲線・分岐点・プリセット・切り替え・色）かを言う（スライダーのドラッグは離すまで
//! 1 回の取り消しにまとめるので `false`）。画面には名前と値だけを出し、説明はツールチップ。

use std::sync::Arc;

use egui::{Color32, Ui};
use yolu_core::curve::{Curve, CurvePoint};
use yolu_core::{
    BalanceRange, BrightnessContrast, Channel, ChannelKind, ColorAdjust, ColorBalance, Document,
    GradientMap, LayerId, Posterize, Rect as CoreRect, Rgba8, Threshold, ToneChannel, ToneCurves,
};

use super::properties::{slider_row, toggle_row};
use super::ramp_rows;
use crate::eyedrop::EyedropState;
use crate::lang::Lang;
use crate::rampsets::RampSets;
use crate::ui::curve::{self, CurveStyle};
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, NumberFormat, Rows};

/// 決まった変更。
#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    pub value: ColorAdjust,
    /// 1 回の操作で決まる変更か（スライダーのドラッグではない）。
    pub discrete: bool,
}

/// 欄を出すときの文脈。
pub struct Params<'a> {
    /// 選びを覚える置き場の名前（層・段ごと）。
    pub key: (&'static str, u128),
    pub enabled: bool,
    /// 描くチャンネルに効かないときの理由（スライダーのツールチップに出す）。
    pub why: Option<&'a str>,
    /// メインの色（描画色。分岐点の色をメインにしたときの色。0〜1 の RGBA）。
    pub paint: [f32; 4],
    /// サブの色（背景色）。
    pub sub: [f32; 4],
    pub lang: Lang,
    /// トーンカーブの後ろに薄く敷く分布（調整の層の下の合成）。
    pub histogram: Option<&'a Histogram>,
    /// グラデーションセット（グラデーションマップの見本の一覧）。
    pub sets: &'a mut RampSets,
    /// 画面の色を取るスポイトの状態（グラデーションマップの色の分岐点のスポイトが使う）。
    pub eyedrop: &'a mut EyedropState,
    /// 状態の帯の知らせ（グラデーションセットの保存の失敗など）。
    pub message: &'a mut String,
}

/// 6 種の欄の全部。
pub fn rows(
    ui: &mut Ui,
    rows: &mut Rows,
    p: &mut Params<'_>,
    value: &ColorAdjust,
) -> Option<Change> {
    match value {
        ColorAdjust::GradientMap(v) => gradient_map_rows(ui, rows, p, v),
        ColorAdjust::ToneCurve(v) => tone_curve_rows(ui, rows, p, v),
        ColorAdjust::ColorBalance(v) => color_balance_rows(ui, rows, p, v),
        ColorAdjust::BrightnessContrast(v) => brightness_contrast_rows(ui, rows, p, v),
        ColorAdjust::Threshold(v) => threshold_rows(ui, rows, p, v),
        ColorAdjust::Posterize(v) => posterize_rows(ui, rows, p, v),
    }
}

fn int_format() -> NumberFormat<'static> {
    NumberFormat::int("")
}

/// 見えている欄の選び（グラデーションの分岐点・曲線のチャンネル・範囲）を覚える置き場。
fn remembered<T: Clone + Send + Sync + 'static>(
    ui: &Ui,
    p: &Params<'_>,
    name: &str,
    default: T,
) -> (egui::Id, T) {
    let id = ui.make_persistent_id(("color_adjust", name, p.key));
    let value = ui.data(|d| d.get_temp::<T>(id)).unwrap_or(default);
    (id, value)
}

/// 同じ幅の切り替えのボタンの行（選んでいる 1 つが青）。押された番号を返す。
fn choice_buttons(
    ui: &mut Ui,
    rows: &mut Rows,
    p: &Params<'_>,
    name: &str,
    labels: &[(&str, &str)],
    current: usize,
) -> Option<usize> {
    let r = rows.row(24.0, 4.0);
    let mut pressed = None;
    for (i, cell) in Rows::split(r, labels.len(), 4.0).into_iter().enumerate() {
        let (label, tooltip) = labels[i];
        if w::button(
            ui,
            cell,
            (p.key, name, i),
            label,
            i == current,
            p.enabled,
            Some(tooltip),
            None,
        )
        .clicked()
        {
            pressed = Some(i);
        }
    }
    pressed
}

// ───────── グラデーションマップ ─────────

/// グラデーションマップの欄: 逆向きの切り替えと、共通のランプの欄（`ramp_rows`。混色・グラデーションセット・分岐点・混合率曲線）。
fn gradient_map_rows(
    ui: &mut Ui,
    rows: &mut Rows,
    p: &mut Params<'_>,
    map: &GradientMap,
) -> Option<Change> {
    let lang = p.lang;
    let mut reverse = map.reverse();
    let mut result: Option<Change> = None;
    if let Some(on) = toggle_row(
        ui,
        rows,
        "ca.gm.reverse",
        lang.pick("逆向き", "Reverse"),
        reverse,
        Some(lang.pick(
            "暗い所と明るい所の色を入れ替える",
            "Swap the colours of dark and bright areas",
        )),
        p.enabled,
    ) {
        reverse = on;
        result = Some(Change {
            value: ColorAdjust::GradientMap(GradientMap::new(map.ramp().clone(), reverse)),
            discrete: true,
        });
    }
    let mut params = ramp_rows::Params {
        key: p.key,
        enabled: p.enabled,
        lang,
        main: p.paint,
        sub: p.sub,
        scalar: false,
        features: ramp_rows::Features {
            mixing: true,
            value_curve: false,
        },
        sets: &mut *p.sets,
        eyedrop: &mut *p.eyedrop,
        message: &mut *p.message,
    };
    if let Some(change) = ramp_rows::rows(ui, rows, &mut params, map.ramp()) {
        result = Some(Change {
            value: ColorAdjust::GradientMap(GradientMap::new(change.ramp, reverse)),
            discrete: change.discrete,
        });
    }
    result
}

// ───────── トーンカーブ ─────────

/// 曲線のチャンネルごとの線の色。
fn channel_color(channel: ToneChannel) -> Color32 {
    match channel {
        ToneChannel::Composite => t::ACCENT,
        ToneChannel::Red => Color32::from_rgb(235, 90, 90),
        ToneChannel::Green => Color32::from_rgb(90, 200, 110),
        ToneChannel::Blue => Color32::from_rgb(100, 140, 245),
    }
}

/// トーンカーブの点を Photoshop と同じ 0〜255 の整数の格子（入力・出力とも 1/255 刻み）へ寄せる。この格子に乗った曲線だけが PSD の調整レイヤーへ
/// 書けるので、画面で作る曲線（ドラッグ・プリセット）は必ず乗せる。隣との間隔は core の検査（0.02）を満たす最小の 6 刻みを残す。
/// 寄せた結果が検査に通らなければ元のまま返す。
pub fn on_psd_steps(curve: &Curve) -> Curve {
    const STEPS: f64 = 255.0;
    const MIN_GAP: i64 = 6;
    let points = curve.points();
    let n = points.len() as i64;
    let mut previous = 0i64;
    let snapped: Vec<CurvePoint> = points
        .iter()
        .enumerate()
        .map(|(i, point)| {
            let i = i as i64;
            let x = if i == 0 {
                0
            } else if i == n - 1 {
                255
            } else {
                // 左の点から 6 刻み以上、右にまだ残る点の分も空けておく
                let wanted = (point.x * STEPS).round() as i64;
                wanted.clamp(previous + MIN_GAP, 255 - MIN_GAP * (n - 1 - i))
            };
            previous = x;
            CurvePoint {
                x: x as f64 / STEPS,
                y: (point.y * STEPS).round() / STEPS,
            }
        })
        .collect();
    Curve::new(snapped).unwrap_or_else(|_| curve.clone())
}

fn tone_curve_rows(
    ui: &mut Ui,
    rows: &mut Rows,
    p: &Params<'_>,
    curves: &ToneCurves,
) -> Option<Change> {
    let lang = p.lang;
    let (channel_id, channel) = remembered(ui, p, "tone.channel", ToneChannel::Composite);
    let labels = [
        ("RGB", lang.pick("すべての色", "All channels")),
        (
            "R",
            lang.pick(
                "赤（スカラーのチャンネルには効きません）",
                "Red (no effect on scalar channels)",
            ),
        ),
        (
            "G",
            lang.pick(
                "緑（スカラーのチャンネルには効きません）",
                "Green (no effect on scalar channels)",
            ),
        ),
        (
            "B",
            lang.pick(
                "青（スカラーのチャンネルには効きません）",
                "Blue (no effect on scalar channels)",
            ),
        ),
    ];
    let current = ToneChannel::ALL
        .iter()
        .position(|c| *c == channel)
        .unwrap_or(0);
    let channel = match choice_buttons(ui, rows, p, "tone.channel", &labels, current) {
        Some(i) => ToneChannel::ALL[i],
        None => channel,
    };
    ui.data_mut(|d| d.insert_temp(channel_id, channel));
    let mut result: Option<Change> = None;
    let bins = p.histogram.map(|h| h.bins(channel));
    let style = CurveStyle {
        line: Some(channel_color(channel)),
        backdrop: bins,
        diagonal: true,
    };
    let r = rows.row(curve::HEIGHT, 4.0);
    if let Some(next) = curve::curve_editor_with(
        ui,
        r,
        (p.key, "tone.curve", channel),
        curves.curve(channel),
        &style,
        lang.pick(
            "横が入力、縦が出力。何も無い所を押すと点を足し、ドラッグで動かし、右クリックか枠の外へ離すと消す。Esc でドラッグをやめる",
            "Input across, output up. Click to add a point, drag to move, right-click or drag outside to remove. Escape cancels a drag",
        ),
        p.enabled,
    ) {
        result = Some(Change {
            value: ColorAdjust::ToneCurve(curves.with_curve(channel, on_psd_steps(&next))),
            discrete: true,
        });
    }
    // よく使う形（選んでいる曲線へ）
    let presets = [
        (lang.pick("線形", "Linear"), 0),
        (lang.pick("やわらかい", "Soft"), 1),
        (lang.pick("かたい", "Hard"), 2),
        (lang.pick("S 字", "S-curve"), 3),
    ];
    let labels: Vec<(&str, &str)> = presets
        .iter()
        .map(|(name, _)| {
            (
                *name,
                lang.pick(
                    "選んでいる曲線をこの形にする",
                    "Set the selected curve to this shape",
                ),
            )
        })
        .collect();
    if let Some(i) = choice_buttons(ui, rows, p, "tone.preset", &labels, usize::MAX) {
        if let Some(shape) = curve::ops::preset(presets[i].1) {
            result = Some(Change {
                value: ColorAdjust::ToneCurve(curves.with_curve(channel, on_psd_steps(&shape))),
                discrete: true,
            });
        }
    }
    result
}

// ───────── カラーバランス ─────────

fn color_balance_rows(
    ui: &mut Ui,
    rows: &mut Rows,
    p: &Params<'_>,
    balance: &ColorBalance,
) -> Option<Change> {
    let lang = p.lang;
    let (range_id, range) = remembered(ui, p, "balance.range", BalanceRange::Midtones);
    let labels = [
        (
            lang.pick("シャドウ", "Shadows"),
            lang.pick("暗い所", "Dark areas"),
        ),
        (
            lang.pick("中間", "Midtones"),
            lang.pick("中間の明るさ", "Middle brightness"),
        ),
        (
            lang.pick("ハイライト", "Highlights"),
            lang.pick("明るい所", "Bright areas"),
        ),
    ];
    let current = BalanceRange::ALL
        .iter()
        .position(|r| *r == range)
        .unwrap_or(1);
    let range = match choice_buttons(ui, rows, p, "balance.range", &labels, current) {
        Some(i) => BalanceRange::ALL[i],
        None => range,
    };
    ui.data_mut(|d| d.insert_temp(range_id, range));
    let mut values = balance.values(range);
    let mut changed = false;
    let axes = [
        ("ca.cb.cr", lang.pick("シアン — レッド", "Cyan — Red")),
        (
            "ca.cb.mg",
            lang.pick("マゼンタ — グリーン", "Magenta — Green"),
        ),
        ("ca.cb.yb", lang.pick("イエロー — ブルー", "Yellow — Blue")),
    ];
    for (k, (id, label)) in axes.iter().enumerate() {
        if let Some(v) = slider_row(
            ui,
            rows,
            id,
            label,
            values[k] as f32,
            (-100.0, 100.0),
            int_format(),
            p.why,
            p.enabled,
        ) {
            values[k] = f64::from(v.round().clamp(-100.0, 100.0));
            changed = true;
        }
    }
    let mut result = None;
    if changed {
        if let Ok(next) = balance.with_range(range, values) {
            result = Some(Change {
                value: ColorAdjust::ColorBalance(next),
                discrete: false,
            });
        }
    }
    if let Some(on) = toggle_row(
        ui,
        rows,
        "ca.cb.luminosity",
        lang.pick("輝度を保つ", "Preserve Luminosity"),
        balance.preserve_luminosity(),
        Some(lang.pick(
            "色を動かしても明るさを元に戻す",
            "Restore the brightness after the colours are moved",
        )),
        p.enabled,
    ) {
        result = Some(Change {
            value: ColorAdjust::ColorBalance(balance.with_preserve_luminosity(on)),
            discrete: true,
        });
    }
    result
}

// ───────── 明るさ・コントラスト・2 値化・ポスタリゼーション ─────────

fn brightness_contrast_rows(
    ui: &mut Ui,
    rows: &mut Rows,
    p: &Params<'_>,
    value: &BrightnessContrast,
) -> Option<Change> {
    let lang = p.lang;
    let (mut brightness, mut contrast) = (value.brightness(), value.contrast());
    let mut changed = false;
    if let Some(v) = slider_row(
        ui,
        rows,
        "ca.bc.brightness",
        lang.pick("明るさ", "Brightness"),
        brightness as f32,
        (-150.0, 150.0),
        int_format(),
        p.why.or(Some(lang.pick(
            "中間を持ち上げる・沈める（黒と白は動かない）",
            "Lift or sink the midtones (black and white stay put)",
        ))),
        p.enabled,
    ) {
        brightness = f64::from(v.round().clamp(-150.0, 150.0));
        changed = true;
    }
    if let Some(v) = slider_row(
        ui,
        rows,
        "ca.bc.contrast",
        lang.pick("コントラスト", "Contrast"),
        contrast as f32,
        (-50.0, 100.0),
        int_format(),
        p.why.or(Some(lang.pick(
            "中間の明るさを中心に、明暗の差を広げる・縮める",
            "Widen or narrow the difference between dark and bright around the middle",
        ))),
        p.enabled,
    ) {
        contrast = f64::from(v.round().clamp(-50.0, 100.0));
        changed = true;
    }
    if !changed {
        return None;
    }
    BrightnessContrast::new(brightness, contrast)
        .ok()
        .map(|next| Change {
            value: ColorAdjust::BrightnessContrast(next),
            discrete: false,
        })
}

fn threshold_rows(
    ui: &mut Ui,
    rows: &mut Rows,
    p: &Params<'_>,
    value: &Threshold,
) -> Option<Change> {
    let lang = p.lang;
    slider_row(
        ui,
        rows,
        "ca.threshold",
        lang.pick("しきい値", "Threshold"),
        value.level() as f32,
        (1.0, 255.0),
        int_format(),
        p.why.or(Some(lang.pick(
            "この明るさ以上は白、未満は黒",
            "At or above this brightness becomes white, below it black",
        ))),
        p.enabled,
    )
    .and_then(|v| Threshold::new(v.round().clamp(1.0, 255.0) as u32).ok())
    .map(|next| Change {
        value: ColorAdjust::Threshold(next),
        discrete: false,
    })
}

fn posterize_rows(
    ui: &mut Ui,
    rows: &mut Rows,
    p: &Params<'_>,
    value: &Posterize,
) -> Option<Change> {
    let lang = p.lang;
    slider_row(
        ui,
        rows,
        "ca.posterize",
        lang.pick("階調", "Levels"),
        value.levels() as f32,
        (2.0, 255.0),
        int_format(),
        p.why.or(Some(
            lang.pick("色ごとの段の数", "Number of steps per colour"),
        )),
        p.enabled,
    )
    .and_then(|v| Posterize::new(v.round().clamp(2.0, 255.0) as u32).ok())
    .map(|next| Change {
        value: ColorAdjust::Posterize(next),
        discrete: false,
    })
}

// ───────── トーンカーブの後ろの分布 ─────────

/// 調整の層の下の合成の明るさと R・G・B の分布（粗い標本。数は出さず、曲線の後ろに薄く敷く）。
#[derive(Clone, Debug, PartialEq)]
pub struct Histogram {
    composite: Vec<f32>,
    red: Vec<f32>,
    green: Vec<f32>,
    blue: Vec<f32>,
}

impl Histogram {
    /// 柱の数。
    pub const BINS: usize = 64;

    /// そのチャンネルの柱の高さ（0〜1）。
    pub fn bins(&self, channel: ToneChannel) -> &[f32] {
        match channel {
            ToneChannel::Composite => &self.composite,
            ToneChannel::Red => &self.red,
            ToneChannel::Green => &self.green,
            ToneChannel::Blue => &self.blue,
        }
    }

    /// 層 `layer` の下の `channel` の合成から、画布の 8 × 8 か所の小さな区画を標本にして作る（不透明度で重みを付ける）。
    /// スカラーのチャンネルは灰色として数えるので、R・G・B の分布は明るさと同じ。何も見えていなければ None。
    pub fn below(doc: &Document, layer: LayerId, channel: Channel) -> Option<Histogram> {
        const GRID: u32 = 8;
        const BLOCK: u32 = 24;
        let (width, height) = (doc.width(), doc.height());
        let (bw, bh) = (BLOCK.min(width), BLOCK.min(height));
        let mut counts = vec![[0f32; Self::BINS]; 4];
        let mut any = false;
        for gy in 0..GRID {
            for gx in 0..GRID {
                let x = (width - bw) * gx / (GRID - 1);
                let y = (height - bh) * gy / (GRID - 1);
                let pixels = doc
                    .composite_below(layer, channel, CoreRect::new(x, y, bw, bh))
                    .ok()?;
                for px in pixels.as_chunks::<4>().0 {
                    if px[3] == 0 {
                        continue;
                    }
                    any = true;
                    let weight = f32::from(px[3]) / 255.0;
                    let luma = yolu_core::luminance(Rgba8::new(px[0], px[1], px[2], px[3]));
                    for (k, v) in [luma, px[0], px[1], px[2]].into_iter().enumerate() {
                        counts[k][usize::from(v) * Self::BINS / 256] += weight;
                    }
                }
            }
        }
        if !any {
            return None;
        }
        // 平方根で縮めて、いちばん高い柱を 1 にする
        let shape = |c: &[f32; Self::BINS]| -> Vec<f32> {
            let top = c.iter().copied().fold(0.0f32, f32::max).max(1e-6);
            c.iter().map(|v| (v / top).sqrt()).collect()
        };
        Some(Histogram {
            composite: shape(&counts[0]),
            red: shape(&counts[1]),
            green: shape(&counts[2]),
            blue: shape(&counts[3]),
        })
    }
}

/// 曲線が効くチャンネルのうち、分布を敷くもの。描くチャンネルが層で有効（法線でない）ならそれ、そうでなければ層の有効なチャンネルの先頭
/// （法線は除く）。トーンカーブの調整の層は Color 以外のスカラーだけに有効にもできるので、いつも Color の分布を敷くと入力と違う分布を見せてしまう。
pub fn histogram_channel(doc: &Document, layer: LayerId, paint: Channel) -> Channel {
    let Some(l) = doc.layer(layer) else {
        return Channel::Color;
    };
    let usable = |c: Channel| {
        l.is_channel_enabled(c)
            && doc
                .channel_info(c)
                .is_some_and(|info| info.kind != ChannelKind::Normal)
    };
    if usable(paint) {
        paint
    } else {
        l.enabled_channels()
            .into_iter()
            .find(|c| usable(*c))
            .unwrap_or(Channel::Color)
    }
}

/// 層の下の分布（文書の変更の通し番号とチャンネルが変わるまで覚える）。トーンカーブの調整の層だけで求める。
/// `paint` は描いているチャンネル（敷くチャンネルは [`histogram_channel`] が決める）。
pub fn cached_histogram(
    ui: &Ui,
    doc: &Document,
    layer: LayerId,
    paint: Channel,
) -> Option<Arc<Histogram>> {
    let id = ui.make_persistent_id(("color_adjust.histogram", layer.0));
    let serial = doc.change_serial();
    let channel = histogram_channel(doc, layer, paint);
    if let Some((s, c, h)) = ui.data(|d| d.get_temp::<(u64, Channel, Option<Arc<Histogram>>)>(id)) {
        if s == serial && c == channel {
            return h;
        }
    }
    let h = Histogram::below(doc, layer, channel).map(Arc::new);
    ui.data_mut(|d| d.insert_temp(id, (serial, channel, h.clone())));
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_histogram_is_a_normalised_shape_of_what_lies_below() {
        let mut doc = Document::with_tile_size(64, 48, 16).unwrap();
        let base = doc.add_layer("下地").unwrap();
        for y in 0..48 {
            for x in 0..64 {
                let v = (x * 4) as u8;
                doc.set_pixel(base, x, y, Rgba8::new(v, 255 - v, 40, 255))
                    .unwrap();
            }
        }
        let adj = doc
            .add_adjustment_layer("調整", yolu_core::AdjustmentSettings::invert(), None, None)
            .unwrap();
        let h = Histogram::below(&doc, adj, Channel::Color).unwrap();
        for channel in ToneChannel::ALL {
            let bins = h.bins(channel);
            assert_eq!(bins.len(), Histogram::BINS);
            let top = bins.iter().copied().fold(0.0f32, f32::max);
            assert!((top - 1.0).abs() < 1e-6, "{channel:?} {top}");
            assert!(bins.iter().all(|v| (0.0..=1.0).contains(v)));
        }
        // 青は 40 だけ（1 つの柱）、赤は左から右へ広がる
        let blue = h.bins(ToneChannel::Blue);
        assert_eq!(blue.iter().filter(|v| **v > 0.0).count(), 1);
        assert!(
            h.bins(ToneChannel::Red)
                .iter()
                .filter(|v| **v > 0.0)
                .count()
                > 20
        );
        // 下に何も無ければ None（一番下の層の下）
        assert!(Histogram::below(&doc, base, Channel::Color).is_none());
    }

    #[test]
    fn the_histogram_is_made_from_the_channel_the_curve_acts_on() {
        let mut doc = Document::with_tile_size(64, 48, 16).unwrap();
        let base = doc.add_layer("下地").unwrap();
        for y in 0..48 {
            for x in 0..64 {
                doc.set_pixel(base, x, y, Rgba8::new(200, 30, 30, 255))
                    .unwrap();
                doc.set_channel_pixel(base, Channel::Roughness, x, y, Rgba8::new(40, 40, 40, 255))
                    .unwrap();
            }
        }
        let curves = || yolu_core::AdjustmentSettings::tone_curve(ToneCurves::identity());
        // 粗さだけに効く層: 描くチャンネルが色でも、敷くのは層が効く粗さ
        let rough = doc
            .add_adjustment_layer("粗さ", curves(), Some(&[Channel::Roughness]), None)
            .unwrap();
        assert_eq!(
            histogram_channel(&doc, rough, Channel::Color),
            Channel::Roughness
        );
        assert_eq!(
            histogram_channel(&doc, rough, Channel::Roughness),
            Channel::Roughness
        );
        let peak = |h: &Histogram| {
            h.bins(ToneChannel::Composite)
                .iter()
                .position(|v| (*v - 1.0).abs() < 1e-6)
                .unwrap()
        };
        let h_rough = Histogram::below(&doc, rough, Channel::Roughness).unwrap();
        assert_eq!(peak(&h_rough), 40 * Histogram::BINS / 256);
        // スカラーは灰色なので R・G・B も同じ形
        assert_eq!(
            h_rough.bins(ToneChannel::Red),
            h_rough.bins(ToneChannel::Blue)
        );
        let h_colour = Histogram::below(&doc, rough, Channel::Color).unwrap();
        assert_ne!(peak(&h_colour), peak(&h_rough), "色の分布は別");
        // すべてのチャンネルに効く層: 描くチャンネルが有効ならそれ、法線は選ばない
        let all = doc
            .add_adjustment_layer("全部", curves(), None, None)
            .unwrap();
        assert_eq!(histogram_channel(&doc, all, Channel::Color), Channel::Color);
        assert_eq!(
            histogram_channel(&doc, all, Channel::Height),
            Channel::Height
        );
        assert_eq!(
            histogram_channel(&doc, all, Channel::Normal),
            Channel::Color
        );
    }

    #[test]
    fn tone_curves_made_on_the_panel_sit_on_the_psd_steps_and_write_to_psd() {
        let psd_ok = |curve: &Curve| {
            let mut doc = Document::with_tile_size(32, 32, 16).unwrap();
            doc.add_layer("下地").unwrap();
            doc.add_adjustment_layer(
                "曲線",
                yolu_core::AdjustmentSettings::tone_curve(
                    ToneCurves::identity().with_curve(ToneChannel::Composite, curve.clone()),
                ),
                None,
                None,
            )
            .unwrap();
            yolu_io::psd::export_blockers(&doc).is_empty()
        };
        for i in 0..4 {
            let shape = curve::ops::preset(i).unwrap();
            let snapped = on_psd_steps(&shape);
            for point in snapped.points() {
                for v in [point.x, point.y] {
                    assert!(((v * 255.0).round() - v * 255.0).abs() < 1e-9, "{i}: {v}");
                }
            }
            assert!(psd_ok(&snapped), "プリセット {i}");
            // 形はほとんど変わらない（1/255 の半分以内）
            for (a, b) in shape.points().iter().zip(snapped.points()) {
                assert!((a.x - b.x).abs() <= 0.5 / 255.0 + 1e-9);
                assert!((a.y - b.y).abs() <= 0.5 / 255.0 + 1e-9);
            }
        }
        // 刻みに乗っていないプリセットのままだと、PSD は刻みの間として断る
        assert!(!psd_ok(&curve::ops::preset(1).unwrap()));
        // 点を近づけて寄せても間隔の検査に通る（0.30 と 0.32 は丸めると 5 刻みで、そのままでは通らない）
        let close = Curve::new(vec![
            CurvePoint { x: 0.0, y: 0.0 },
            CurvePoint { x: 0.30, y: 0.2 },
            CurvePoint { x: 0.32, y: 0.8 },
            CurvePoint { x: 1.0, y: 1.0 },
        ])
        .unwrap();
        let snapped = on_psd_steps(&close);
        let gap = snapped.points()[2].x - snapped.points()[1].x;
        assert!(gap >= Curve::MIN_GAP && psd_ok(&snapped), "{gap}");
        // 点がいちばん詰まった形（16 点）でも、右の端の手前に収まる
        let dense: Vec<CurvePoint> = (0..16)
            .map(|i| CurvePoint {
                x: if i == 15 { 1.0 } else { f64::from(i) * 0.0667 },
                y: f64::from(i) / 15.0,
            })
            .collect();
        let dense = Curve::new(dense).unwrap();
        assert!(psd_ok(&on_psd_steps(&dense)));
        // 線形はそのまま
        assert_eq!(on_psd_steps(&Curve::identity()), Curve::identity());
    }
}
