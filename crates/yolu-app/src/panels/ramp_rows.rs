//! ランプの欄（グラデーションマップの調整・フィルターと、塗りつぶしのグラデーションが共通で使う。CLIP STUDIO のグラデーションマップの窓の並びを手本に、
//! このアプリの欄の部品で組む）。上から順に:
//!
//! 1. 混色モード（通常・知覚的・リニア）と輝度の補正（知覚的のときだけ）。`features.mixing` のとき（グラデーションマップ）。
//! 2. グラデーションセット: 組の切り替え、見本の一覧（押すと当てる。利用者の組は名前を変える・消す・今のランプを足す）。
//! 3. 分岐点の編集（ダブルクリックで、その分岐点のそばに色の選びが出る）。
//! 4. 選んでいる分岐点: 前後へ移る・消す、位置、色（メイン・サブ・指定。メイン・サブは描画色に付いていく。スポイト）または不透明度、区間の中点、
//!    区間の混合率曲線（`features.mixing`）。
//! 5. 値のカーブ（`features.value_curve`。塗りつぶしのグラデーション）。
//!
//! 変更が決まったときだけ新しいランプを返し、`discrete` は 1 回の操作で決まる変更か（スライダー・色の選びのドラッグは離すまで 1 回の取り消しに
//! まとめるので `false`）。画面には名前と値だけを出し、説明はツールチップ。分岐点の選び・メイン/サブへの追従・名前の変更中は画面の状態で、文書には入れない。

use egui::{pos2, vec2, Color32, Rect, Sense, Ui};
use yolu_core::curve::Curve;
use yolu_core::generator::{ColorStop, LuminanceCorrection, MixMode, Ramp};
use yolu_core::Rgba8;

use super::color_popup;
use super::properties::{
    choice_buttons, group_label, percent_row, slider_row, toggle_row, ChoiceButton,
};
use crate::eyedrop::EyedropState;
use crate::lang::Lang;
use crate::rampsets::{RampSets, MAX_USER};
use crate::ui::curve::{self, CurveStyle};
use crate::ui::ramp::{self, ops, Selection};
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, NumberFormat, Rows};

/// 欄に出す機能。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Features {
    /// 混色モード・輝度の補正・区間の混合率曲線（グラデーションマップ。塗りつぶしのグラデーションは Unity 版と共有の並びで持てない）。
    pub mixing: bool,
    /// ランプ全体の値のカーブ（塗りつぶしのグラデーション。グラデーションマップには無い）。
    pub value_curve: bool,
}

/// 欄を出すときの文脈。
pub struct Params<'a> {
    /// 選びを覚える置き場の名前（層・段・チャンネルごと）。
    pub key: (&'static str, u128),
    pub enabled: bool,
    pub lang: Lang,
    /// メインの色とサブの色（描画色。0〜1 の RGBA）。
    pub main: [f32; 4],
    pub sub: [f32; 4],
    /// データのチャンネル（色を輝度の灰色で見せ、分岐点は値で選ぶ）。
    pub scalar: bool,
    pub features: Features,
    pub sets: &'a mut RampSets,
    pub eyedrop: &'a mut EyedropState,
    /// グラデーションセットの保存の失敗の文（呼んだ側が `AppState::fail` で知らせる。部品は `AppState` を借りない）。
    pub failure: &'a mut Option<String>,
}

/// 決まった変更。
#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    pub ramp: Ramp,
    /// 1 回の操作で決まる変更か（スライダー・色の選びのドラッグではない）。
    pub discrete: bool,
}

/// 色の分岐点がメインかサブの色に付いていく印（画面の状態）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Source {
    Fixed,
    Main,
    Sub,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Link {
    index: usize,
    source: Source,
    /// 付いていった結果の色（ほかの操作で色が変わったら、その分岐点は付いていくのをやめる）。
    last: Rgba8,
}

/// メイン・サブの色に付いていく分岐点の印（画面の状態）。分岐点ごとに 1 つで、いくつでも同時に持てる（メイン→サブの両端など）。
/// 選んでいる分岐点に関わらず付いていく（選びを替えても外れない）。外れるのは、その分岐点の色を別の操作が替えたとき・利用者が指定へ戻したとき・
/// 分岐点の並びを別の操作が替えたとき（セットを当てる・取り消しなど）。足す・消すは番号を直して残す。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Links {
    list: Vec<Link>,
    /// 印をつけたときの色の分岐点の数（番号が指す先が変わっていないかの確かめ）。
    count: usize,
}

impl Links {
    fn source(&self, index: usize) -> Source {
        self.list
            .iter()
            .find(|l| l.index == index)
            .map_or(Source::Fixed, |l| l.source)
    }

    /// 分岐点 `index` の印を外す（ほかの分岐点の印は残す）。
    fn unfollow(&mut self, index: usize) {
        self.list.retain(|l| l.index != index);
    }

    fn clear(&mut self) {
        self.list.clear();
    }

    /// 分岐点 `index` を `source` の色（いま `last`）に付いていかせる。`count` は今の色の分岐点の数。
    fn follow(&mut self, index: usize, source: Source, last: Rgba8, count: usize) {
        if self.count != count {
            self.list.clear();
        }
        self.unfollow(index);
        self.list.push(Link {
            index,
            source,
            last,
        });
        self.count = count;
    }

    /// 色の分岐点を足した・消したとき、印の番号を直す（1 つだけ増減した場合。位置が最初に食い違う所が足した・消した分岐点）。
    /// 消した分岐点の印は外す。それ以外の数の変化は全部外す。数が同じ（動かしただけ）なら何もしない。
    fn reshape(&mut self, before: &Ramp, after: &Ramp) {
        let (old, new) = (before.colors(), after.colors());
        if old.len() == new.len() {
            return;
        }
        let first_difference = |shorter: usize| {
            (0..shorter)
                .find(|&i| old[i].position != new[i].position)
                .unwrap_or(shorter)
        };
        if new.len() == old.len() + 1 {
            let at = first_difference(old.len());
            for l in &mut self.list {
                if l.index >= at {
                    l.index += 1;
                }
            }
        } else if old.len() == new.len() + 1 {
            let at = first_difference(new.len());
            self.list.retain(|l| l.index != at);
            for l in &mut self.list {
                if l.index > at {
                    l.index -= 1;
                }
            }
        } else {
            self.list.clear();
        }
        self.count = new.len();
    }

    /// 毎フレーム: 色が付いていった結果のままでなくなった分岐点の印を外し、メイン・サブの色が替わっていれば、付いていく分岐点の色を替えた
    /// ランプを返す。
    fn follow_colors(&mut self, ramp: &Ramp, main: Rgba8, sub: Rgba8) -> Option<Ramp> {
        if self.list.is_empty() {
            return None;
        }
        if ramp.colors().len() != self.count {
            self.list.clear();
            return None;
        }
        self.list.retain(|l| {
            ramp.colors()
                .get(l.index)
                .is_some_and(|c| c.color == l.last)
        });
        let mut stops = ramp.colors().to_vec();
        let mut list = self.list.clone();
        for l in &mut list {
            let wanted = match l.source {
                Source::Main => main,
                Source::Sub => sub,
                Source::Fixed => l.last,
            };
            if wanted != l.last {
                stops[l.index].color = wanted;
                l.last = wanted;
            }
        }
        if list == self.list {
            return None;
        }
        let next = ops::with_colors(ramp, stops)?;
        self.list = list;
        Some(next)
    }
}

fn rgb8(c: [f32; 4]) -> Rgba8 {
    Rgba8::new(w::to_byte(c[0]), w::to_byte(c[1]), w::to_byte(c[2]), 255)
}

fn bytes(c: Rgba8) -> [u8; 3] {
    [c.r, c.g, c.b]
}

fn put(result: &mut Option<Change>, ramp: &Ramp, discrete: bool) {
    *result = Some(Change {
        ramp: ramp.clone(),
        discrete,
    });
}

fn remembered<T: Clone + Send + Sync + 'static>(
    ui: &Ui,
    p: &Params<'_>,
    name: &str,
    default: T,
) -> (egui::Id, T) {
    let id = ui.make_persistent_id(("ramp_rows", name, p.key));
    let value = ui.data(|d| d.get_temp::<T>(id)).unwrap_or(default);
    (id, value)
}

/// 色の選びの窓の名前（`color_popup` の ID の元。試験が窓の場所を引く）。画面の部品の並びによらず、`key` だけで決まる。
pub fn popup_id(key: (&'static str, u128)) -> egui::Id {
    egui::Id::new(("ramp_rows", "popup", key))
}

/// 今選んでいる分岐点（試験が読む）。
pub fn selected(ui: &Ui, key: (&'static str, u128)) -> Selection {
    let id = ui.make_persistent_id(("ramp_rows", "selection", key));
    ui.data(|d| d.get_temp::<Selection>(id)).unwrap_or_default()
}

/// 分岐点の編集の部品の名前（試験がダブルクリックの場所を引く）。
pub fn stops_salt(key: (&'static str, u128)) -> (&'static str, u128, &'static str) {
    (key.0, key.1, "ramp.stops")
}

fn mix_name(lang: Lang, mode: MixMode) -> (&'static str, &'static str) {
    match mode {
        MixMode::Standard => (
            lang.pick("通常", "Standard"),
            lang.pick(
                "色の値をそのまま混ぜる（PSD のグラデーションと同じ）",
                "Mix the color values as they are (the same as a PSD gradient)",
            ),
        ),
        MixMode::Perceptual => (
            lang.pick("知覚的", "Perceptual"),
            lang.pick(
                "人の見た目に近い空間（Oklab）で混ぜる。鮮やかで濁りにくい",
                "Mix in a perceptual space (Oklab). Vivid and less muddy",
            ),
        ),
        MixMode::Linear => (
            lang.pick("リニア", "Linear"),
            lang.pick(
                "線形の光で混ぜる。暗い色どうしでも沈まない",
                "Mix in linear light. Dark colors do not sink",
            ),
        ),
    }
}

fn correction_name(lang: Lang, level: LuminanceCorrection) -> &'static str {
    match level {
        LuminanceCorrection::None => lang.pick("なし", "None"),
        LuminanceCorrection::Low => lang.pick("低", "Low"),
        LuminanceCorrection::Medium => lang.pick("中", "Medium"),
        LuminanceCorrection::High => lang.pick("高", "High"),
        LuminanceCorrection::Max => lang.pick("最大", "Max"),
    }
}

const MIX_IDS: [&str; 3] = [
    "ramp.mix.standard",
    "ramp.mix.perceptual",
    "ramp.mix.linear",
];
const LUMINANCE_IDS: [&str; 5] = [
    "ramp.lum.none",
    "ramp.lum.low",
    "ramp.lum.medium",
    "ramp.lum.high",
    "ramp.lum.max",
];

/// 混色モードと輝度の補正の行。
fn mixing_rows(ui: &mut Ui, rows: &mut Rows, p: &Params<'_>, ramp: &Ramp) -> Option<Ramp> {
    let lang = p.lang;
    let mut result = None;
    group_label(ui, rows, lang.pick("混色モード", "Color Mixing"));
    let items: Vec<ChoiceButton> = MixMode::ALL
        .iter()
        .enumerate()
        .map(|(i, mode)| {
            let (label, tip) = mix_name(lang, *mode);
            ChoiceButton {
                id: MIX_IDS[i],
                label,
                selected: ramp.mix_mode() == *mode,
                enabled: p.enabled,
                tooltip: Some(tip),
            }
        })
        .collect();
    if let Some(i) = choice_buttons(ui, rows, &items) {
        let mode = MixMode::ALL[i];
        if mode != ramp.mix_mode() {
            result = Some(ramp.with_mixing(mode, ramp.luminance_correction()));
        }
    }
    group_label(ui, rows, lang.pick("輝度の補正", "Luminance"));
    let perceptual = ramp.mix_mode() == MixMode::Perceptual;
    let tip = if perceptual {
        lang.pick(
            "強いほど、混ぜた色が明るくなる",
            "The stronger, the lighter the mixed colors",
        )
    } else {
        lang.pick(
            "混色モードが知覚的のときだけ使える",
            "Available only when the color mixing is Perceptual",
        )
    };
    let items: Vec<ChoiceButton> = LuminanceCorrection::ALL
        .iter()
        .enumerate()
        .map(|(i, level)| ChoiceButton {
            id: LUMINANCE_IDS[i],
            label: correction_name(lang, *level),
            selected: perceptual && ramp.luminance_correction() == *level,
            enabled: p.enabled && perceptual,
            tooltip: Some(tip),
        })
        .collect();
    if let Some(i) = choice_buttons(ui, rows, &items) {
        let level = LuminanceCorrection::ALL[i];
        if perceptual && level != ramp.luminance_correction() {
            result = Some(ramp.with_mixing(MixMode::Perceptual, level));
        }
    }
    result
}

/// 見本 1 つを描く（色だけ。不透明度は市松の上に）。
fn paint_ramp(ui: &Ui, r: Rect, ramp: &Ramp, scalar: bool) {
    const STEPS: usize = 24;
    let p = ui.painter();
    w::checker(p, r, 5.0);
    for k in 0..STEPS {
        if let Ok(c) = ramp.sample_stops(k as f64 / (STEPS - 1) as f64, scalar) {
            let x0 = r.left() + r.width() * k as f32 / STEPS as f32;
            w::fill(
                p,
                Rect::from_min_size(
                    pos2(x0, r.top()),
                    vec2(r.width() / STEPS as f32 + 1.0, r.height()),
                ),
                Color32::from_rgba_unmultiplied(c.r, c.g, c.b, c.a),
            );
        }
    }
}

/// 見本の列の数（欄の幅に収まるだけ。名前はツールチップ）。
fn columns(width: f32) -> usize {
    (((width + 4.0) / 84.0).floor() as usize).max(2)
}

const THUMB_HEIGHT: f32 = 20.0;
const LIST_ROWS: usize = 5;

/// グラデーションセット（組の切り替え・見本の一覧・足す・名前・消す）。押した見本のランプを返す。
fn sets_rows(
    ui: &mut Ui,
    rows: &mut Rows,
    p: &mut Params<'_>,
    ramp: &Ramp,
) -> Option<(Ramp, bool)> {
    let lang = p.lang;
    let enabled = p.enabled;
    let mut result = None;
    // 組
    let names: Vec<String> = (0..RampSets::group_count())
        .map(|g| RampSets::group_name(lang, g))
        .collect();
    let ids: Vec<String> = (0..names.len())
        .map(|g| format!("ramp.group.{g}"))
        .collect();
    let items: Vec<ChoiceButton> = names
        .iter()
        .zip(&ids)
        .enumerate()
        .map(|(g, (name, id))| ChoiceButton {
            id,
            label: name,
            selected: p.sets.group == g,
            enabled,
            tooltip: None,
        })
        .collect();
    if let Some(g) = choice_buttons(ui, rows, &items) {
        p.sets.show_group(g);
    }
    // 一覧
    let entries = p.sets.entries(lang, p.main, p.sub);
    let cols = columns(rows.width());
    let line_count = entries.len().div_ceil(cols);
    let shown_lines = line_count.clamp(1, LIST_ROWS);
    let gap = 4.0;
    let height = shown_lines as f32 * (THUMB_HEIGHT + gap) - gap;
    let list = rows.row(height, 4.0);
    let cell_w = (list.width() - gap * (cols as f32 - 1.0)) / cols as f32;
    let mut clicked: Option<usize> = None;
    ui.scope_builder(egui::UiBuilder::new().max_rect(list), |ui| {
        egui::ScrollArea::vertical()
            .id_salt((p.key, "ramp.sets.scroll"))
            .max_height(height)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                // 全部の行ぶんの 1 枚を取り、見本はそこからの格子で置く
                let total = line_count as f32 * (THUMB_HEIGHT + gap) - gap;
                let (content, _) =
                    ui.allocate_exact_size(vec2(list.width(), total.max(0.0)), Sense::hover());
                for (index, entry) in entries.iter().enumerate() {
                    let (line, i) = (index / cols, index % cols);
                    let cell = Rect::from_min_size(
                        pos2(
                            content.left() + i as f32 * (cell_w + gap),
                            content.top() + line as f32 * (THUMB_HEIGHT + gap),
                        ),
                        vec2(cell_w, THUMB_HEIGHT),
                    );
                    let id = ui.make_persistent_id((p.key, "ramp.set", index));
                    let response = ui.interact(
                        cell,
                        id,
                        if enabled {
                            Sense::click()
                        } else {
                            Sense::hover()
                        },
                    );
                    if !ui.is_rect_visible(cell) {
                        continue;
                    }
                    paint_ramp(ui, cell, &entry.ramp, p.scalar);
                    let selected = p.sets.selected == Some(index);
                    w::outline(
                        ui.painter(),
                        cell,
                        if selected {
                            t::ACCENT
                        } else if enabled && response.hovered() {
                            t::TEXT
                        } else {
                            t::BORDER
                        },
                        if selected { 2.0 } else { 1.0 },
                        2.0,
                    );
                    let response = response.on_hover_text(&entry.name);
                    response.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, &entry.name)
                    });
                    if enabled && response.clicked() {
                        clicked = Some(index);
                    }
                }
            });
    });
    if let Some(index) = clicked {
        p.sets.selected = Some(index);
        let preset = &entries[index].ramp;
        // セットは分岐点の並び＋混合率曲線＋値のカーブ。混色モードと輝度の補正は今のランプのものを残す（混色を持てない欄では外す）。
        // 値のカーブを出さない欄（グラデーションマップ）へは直線で当てる（見えず直せないカーブを入れない）
        let mut next = if p.features.mixing {
            preset.with_mixing(ramp.mix_mode(), ramp.luminance_correction())
        } else {
            preset.without_mixing()
        };
        if !p.features.value_curve {
            next = next.with_value_curve(Curve::identity());
        }
        result = Some((next, true));
    }
    // 足す・名前・消す
    let row = rows.row(24.0, 4.0);
    let button = |n: usize| {
        Rect::from_min_size(
            pos2(row.left() + n as f32 * 28.0, row.top()),
            vec2(24.0, 24.0),
        )
    };
    let user_selected = p.sets.showing_user().then_some(p.sets.selected).flatten();
    let can_add = enabled && p.sets.user().len() < MAX_USER;
    if w::icon_button(
        ui,
        button(0),
        (p.key, "ramp.add"),
        "add",
        lang.pick(
            "今のグラデーションを自分の組に追加",
            "Add the current gradient to your set",
        ),
        false,
        can_add,
        16.0,
    )
    .clicked()
    {
        match p.sets.add("", ramp, lang) {
            Ok(index) => {
                p.sets.show_group(RampSets::user_group());
                p.sets.selected = Some(index);
            }
            Err(e) => *p.failure = Some(e.describe(lang)),
        }
    }
    let (renaming_id, renaming) = remembered(ui, p, "renaming", false);
    let mut renaming = renaming && user_selected.is_some();
    if w::icon_button(
        ui,
        button(1),
        (p.key, "ramp.rename"),
        "edit",
        lang.pick("名前を変える", "Rename"),
        renaming,
        enabled && user_selected.is_some(),
        16.0,
    )
    .clicked()
    {
        renaming = !renaming;
    }
    if w::icon_button(
        ui,
        button(2),
        (p.key, "ramp.remove"),
        "delete",
        lang.pick("自分の組から消す", "Remove from your set"),
        false,
        enabled && user_selected.is_some(),
        16.0,
    )
    .clicked()
    {
        if let Some(index) = user_selected {
            match p.sets.remove(index) {
                Ok(()) => p.sets.selected = None,
                Err(e) => *p.failure = Some(e.describe(lang)),
            }
            renaming = false;
        }
    }
    if renaming {
        if let Some(index) = user_selected {
            let field = rows.row(t::ROW_HEIGHT, 4.0);
            let current = p.sets.user()[index].name.clone();
            let (started_id, started) = remembered(ui, p, "rename.started", false);
            let out = w::text_field(
                ui,
                field,
                (p.key, "ramp.rename.field"),
                &current,
                Some(lang.pick("名前", "Name")),
                !started,
            );
            if let Some(name) = out.committed {
                if let Err(e) = p.sets.rename(index, &name) {
                    *p.failure = Some(e.describe(lang));
                }
                renaming = false;
            } else if started && !out.focused {
                renaming = false;
            }
            ui.data_mut(|d| d.insert_temp(started_id, renaming && (started || out.focused)));
        }
    } else {
        let (started_id, _) = remembered(ui, p, "rename.started", false);
        ui.data_mut(|d| d.insert_temp(started_id, false));
    }
    ui.data_mut(|d| d.insert_temp(renaming_id, renaming));
    result
}

fn replace_color(ramp: &Ramp, index: usize, color: Rgba8) -> Option<Ramp> {
    let mut list = ramp.colors().to_vec();
    list.get_mut(index)?.color = Rgba8::new(color.r, color.g, color.b, 255);
    ops::with_colors(ramp, list)
}

fn lane_count(ramp: &Ramp, alpha: bool) -> usize {
    if alpha {
        ramp.opacities().len()
    } else {
        ramp.colors().len()
    }
}

/// ランプの欄の全部。変更が決まったときだけ返す。
pub fn rows(ui: &mut Ui, rows: &mut Rows, p: &mut Params<'_>, source: &Ramp) -> Option<Change> {
    let lang = p.lang;
    let enabled = p.enabled;
    let ctx = ui.ctx().clone();
    let mut ramp = source.clone();
    let mut result: Option<Change> = None;
    let (selection_id, mut selection) = remembered(ui, p, "selection", Selection::default());
    let (link_id, mut links) = remembered::<Links>(ui, p, "link", Links::default());
    let popup = popup_id(p.key);
    // 欄が何フレームも描かれなかった（別の層・段を選んでいた）なら、付いていくのをやめる（見ているだけで文書が変わらないように）
    let (seen_id, seen) = remembered::<u64>(ui, p, "seen", 0);
    let frame = ctx.cumulative_frame_nr();
    if frame > seen + 3 {
        links.clear();
    }
    ui.data_mut(|d| d.insert_temp(seen_id, frame));

    // 1. 混色
    if p.features.mixing {
        if let Some(next) = mixing_rows(ui, rows, p, &ramp) {
            ramp = next;
            put(&mut result, &ramp, true);
        }
    }
    // 2. グラデーションセット
    if let Some((next, discrete)) = sets_rows(ui, rows, p, &ramp) {
        ramp = next;
        links.clear();
        put(&mut result, &ramp, discrete);
    }
    // 3. 分岐点
    let r = rows.row(ramp::STOPS_HEIGHT, 4.0);
    // 色の選びの窓を置く目安の列（欄の左の端と幅）
    let column = Rect::from_min_size(r.min, vec2(r.width(), 0.0));
    let salt = stops_salt(p.key);
    if let Some(next) = ramp::stops_editor(
        ui,
        r,
        salt,
        &ramp,
        &mut selection,
        p.scalar,
        lang.pick(
            "上: 不透明度の分岐点。下: 色の分岐点（ダブルクリックで色を選ぶ）。何も無い所を押すと追加し、ドラッグで動かし、右クリックか行の外へ離すと消す。小さなひし形は中点。Esc でドラッグをやめる",
            "Top: opacity stops. Bottom: color stops (double-click to pick a color). Click to add, drag to move, right-click or drag outside to remove. Small diamonds move the midpoint. Escape cancels a drag",
        ),
        enabled,
    ) {
        links.reshape(&ramp, &next);
        ramp = next;
        put(&mut result, &ramp, true);
    }
    selection = ops::clamp_selection(&ramp, selection);
    if let Some(request) = ramp::take_color_request(ui, salt) {
        if !p.scalar && enabled {
            let color = ramp.colors()[request.index.min(ramp.colors().len() - 1)].color;
            color_popup::open(
                &ctx,
                popup,
                request.anchor,
                column,
                bytes(color),
                request.index,
            );
            links.unfollow(request.index);
        }
    }
    // 画面から取れた色（スポイト）を、選んでいる色の分岐点へ
    if let Some(rgb) = p.eyedrop.ramp_stop_pick.take() {
        if !selection.alpha && !p.scalar {
            if let Some(next) = replace_color(
                &ramp,
                selection.index,
                Rgba8::new(rgb[0], rgb[1], rgb[2], 255),
            ) {
                ramp = next;
                links.unfollow(selection.index);
                put(&mut result, &ramp, true);
            }
        }
    }
    // メイン・サブに付いていく分岐点（選んでいるかどうかに関わらず、全部）
    if let Some(next) = links.follow_colors(&ramp, rgb8(p.main), rgb8(p.sub)) {
        ramp = next;
        put(&mut result, &ramp, false);
    }
    // 4. 選んでいる分岐点
    let count = lane_count(&ramp, selection.alpha);
    let row = rows.row(24.0, 4.0);
    let cells = [
        Rect::from_min_size(pos2(row.left(), row.top()), vec2(28.0, 24.0)),
        Rect::from_min_size(pos2(row.left() + 32.0, row.top()), vec2(28.0, 24.0)),
        Rect::from_min_size(pos2(row.left() + 64.0, row.top()), vec2(28.0, 24.0)),
    ];
    if w::button(
        ui,
        cells[0],
        (p.key, "ramp.prev"),
        "<",
        false,
        enabled && selection.index > 0,
        Some(lang.pick("前の分岐点", "Previous stop")),
        None,
    )
    .clicked()
    {
        selection.index -= 1;
    }
    if w::button(
        ui,
        cells[1],
        (p.key, "ramp.next"),
        ">",
        false,
        enabled && selection.index + 1 < count,
        Some(lang.pick("次の分岐点", "Next stop")),
        None,
    )
    .clicked()
    {
        selection.index += 1;
    }
    if w::icon_button(
        ui,
        cells[2],
        (p.key, "ramp.stop.remove"),
        "delete",
        lang.pick("分岐点を消す", "Remove stop"),
        false,
        enabled && count > 2,
        16.0,
    )
    .clicked()
    {
        if let Some(next) = ops::remove(&ramp, selection.alpha, selection.index) {
            links.reshape(&ramp, &next);
            ramp = next;
            put(&mut result, &ramp, true);
        }
    }
    selection = ops::clamp_selection(&ramp, selection);
    let count = lane_count(&ramp, selection.alpha);
    // 位置
    let (min, max) = ops::position_range(&ramp, selection.alpha, selection.index);
    let position = ops::selected_position(&ramp, selection);
    if let Some(v) = slider_row(
        ui,
        rows,
        "ramp.position",
        lang.pick("位置", "Position"),
        (position * 100.0) as f32,
        ((min * 100.0) as f32, (max * 100.0) as f32),
        NumberFormat {
            decimals: 2,
            trim: true,
            suffix: "%",
        },
        None,
        enabled,
    ) {
        if let Some(next) = ops::move_stop(
            &ramp,
            selection.alpha,
            selection.index,
            f64::from(v) / 100.0,
        ) {
            ramp = next;
            put(&mut result, &ramp, false);
        }
    }
    if selection.alpha {
        let s = ramp.opacities()[selection.index];
        if let Some(v) = percent_row(
            ui,
            rows,
            "ramp.opacity",
            lang.pick("不透明度", "Opacity"),
            s.opacity,
            (0.0, 1.0),
            Some(lang.pick(
                "この位置でグラデーションの色をどれだけ見せるか（0% なら元の色のまま）",
                "How much of the gradient color shows here (0% keeps the original color)",
            )),
            enabled,
        ) {
            let mut list = ramp.opacities().to_vec();
            list[selection.index].opacity = v;
            if let Some(next) = ops::with_opacities(&ramp, list) {
                ramp = next;
                put(&mut result, &ramp, false);
            }
        }
        if selection.index + 1 < count {
            if let Some(v) = percent_row(
                ui,
                rows,
                "ramp.opacity.midpoint",
                lang.pick("中点", "Midpoint"),
                s.midpoint,
                (0.01, 0.99),
                None,
                enabled,
            ) {
                let mut list = ramp.opacities().to_vec();
                list[selection.index].midpoint = v;
                if let Some(next) = ops::with_opacities(&ramp, list) {
                    ramp = next;
                    put(&mut result, &ramp, false);
                }
            }
        }
    } else {
        let s = ramp.colors()[selection.index];
        if p.scalar {
            let luminance = ((0.2126 * f64::from(s.color.r)
                + 0.7152 * f64::from(s.color.g)
                + 0.0722 * f64::from(s.color.b))
                / 255.0) as f32;
            if let Some(v) = slider_row(
                ui,
                rows,
                "ramp.value",
                lang.pick("値", "Value"),
                luminance,
                (0.0, 1.0),
                NumberFormat {
                    decimals: 3,
                    trim: true,
                    suffix: "",
                },
                None,
                enabled,
            ) {
                let b = (v.clamp(0.0, 1.0) * 255.0).round() as u8;
                if let Some(next) = replace_color(&ramp, selection.index, Rgba8::new(b, b, b, 255))
                {
                    ramp = next;
                    put(&mut result, &ramp, false);
                }
            }
        } else {
            color_rows(
                ui,
                rows,
                p,
                &mut ramp,
                selection,
                &mut links,
                &mut result,
                &ctx,
                popup,
            );
        }
        if selection.index + 1 < count {
            segment_rows(ui, rows, p, &mut ramp, selection.index, &mut result);
        }
    }
    // 色の選びの窓（開いていれば）
    let current = ramp
        .colors()
        .get(selection.index)
        .map_or([0; 3], |c| bytes(c.color));
    match color_popup::show(
        &ctx,
        popup,
        current,
        if selection.alpha {
            usize::MAX
        } else {
            selection.index
        },
        !selection.alpha && !p.scalar && enabled,
        lang,
    ) {
        color_popup::Outcome::Changed(rgb) | color_popup::Outcome::Reverted(rgb) => {
            if let Some(next) = replace_color(
                &ramp,
                selection.index,
                Rgba8::new(rgb[0], rgb[1], rgb[2], 255),
            ) {
                ramp = next;
                links.unfollow(selection.index);
                put(&mut result, &ramp, false);
            }
        }
        color_popup::Outcome::Idle | color_popup::Outcome::Closed => {}
    }
    // 5. 値のカーブ
    if p.features.value_curve {
        value_curve_rows(ui, rows, p, &mut ramp, &mut result);
    }
    ui.data_mut(|d| {
        d.insert_temp(selection_id, selection);
        d.insert_temp(link_id, links);
    });
    result
}

/// 色の分岐点の色の行（メイン・サブ・指定、見本、スポイト）。
#[allow(clippy::too_many_arguments)]
fn color_rows(
    ui: &mut Ui,
    rows: &mut Rows,
    p: &mut Params<'_>,
    ramp: &mut Ramp,
    selection: Selection,
    links: &mut Links,
    result: &mut Option<Change>,
    ctx: &egui::Context,
    popup: egui::Id,
) {
    let lang = p.lang;
    let enabled = p.enabled;
    let node = ramp.colors()[selection.index];
    let source = links.source(selection.index);
    let items = [
        ChoiceButton {
            id: "ramp.source.main",
            label: lang.pick("メイン", "Main"),
            selected: source == Source::Main,
            enabled,
            tooltip: Some(lang.pick(
                "メインの色にして、メインの色を替えると付いていく",
                "Use the main color and follow it when it changes",
            )),
        },
        ChoiceButton {
            id: "ramp.source.sub",
            label: lang.pick("サブ", "Sub"),
            selected: source == Source::Sub,
            enabled,
            tooltip: Some(lang.pick(
                "サブの色にして、サブの色を替えると付いていく",
                "Use the sub color and follow it when it changes",
            )),
        },
        ChoiceButton {
            id: "ramp.source.custom",
            label: lang.pick("指定", "Custom"),
            selected: source == Source::Fixed,
            enabled,
            tooltip: Some(lang.pick(
                "付いていくのをやめて、今の色のままにする",
                "Stop following and keep the current color",
            )),
        },
    ];
    if let Some(i) = choice_buttons(ui, rows, &items) {
        let chosen = match i {
            0 => Source::Main,
            1 => Source::Sub,
            _ => Source::Fixed,
        };
        match chosen {
            Source::Fixed => links.unfollow(selection.index),
            _ => {
                let color = if chosen == Source::Main {
                    rgb8(p.main)
                } else {
                    rgb8(p.sub)
                };
                if let Some(next) = replace_color(ramp, selection.index, color) {
                    *ramp = next;
                    links.follow(selection.index, chosen, color, ramp.colors().len());
                    put(result, ramp, true);
                }
            }
        }
    }
    // 見本とスポイト
    let row = rows.row(t::ROW_HEIGHT + 2.0, 4.0);
    const LABEL: f32 = 34.0;
    w::text(
        ui.painter(),
        Rect::from_min_size(row.min, vec2(LABEL, row.height())),
        lang.pick("色", "Color"),
        t::LABEL,
        w::Align::Left,
    );
    let picker = cfg!(windows);
    let eyedrop_width = if picker { 28.0 } else { 0.0 };
    let swatch = Rect::from_min_max(
        pos2(row.left() + LABEL, row.top() + 1.0),
        pos2(
            row.right() - eyedrop_width - if picker { 4.0 } else { 0.0 },
            row.bottom() - 1.0,
        ),
    );
    let c = node.color;
    let shown = [
        f32::from(c.r) / 255.0,
        f32::from(c.g) / 255.0,
        f32::from(c.b) / 255.0,
        1.0,
    ];
    if w::color_swatch(
        ui,
        swatch,
        (p.key, "ramp.color"),
        shown,
        lang.pick(
            "分岐点の色（押すと色の選びを開く）",
            "Color of the stop (opens the color picker)",
        ),
        enabled,
    )
    .clicked()
    {
        let column = Rect::from_min_size(row.min, vec2(row.width(), 0.0));
        color_popup::open(ctx, popup, swatch, column, bytes(c), selection.index);
        links.unfollow(selection.index);
    }
    if picker
        && w::icon_button(
            ui,
            Rect::from_min_size(
                pos2(row.right() - eyedrop_width, row.top()),
                vec2(eyedrop_width, row.height()),
            ),
            (p.key, "ramp.eyedropper"),
            "tools/eyedropper",
            lang.pick("画面の色を取得", "Pick Screen Color"),
            p.eyedrop.ramp_stop_pending,
            enabled && !p.eyedrop.ramp_stop_pending,
            16.0,
        )
        .clicked()
    {
        p.eyedrop.screen_request = Some(crate::screen_pick::Mode::Visible);
        p.eyedrop.ramp_stop_pending = true;
    }
}

/// 色の分岐点から次の分岐点までの区間: 中点、混合率曲線（グラデーションマップ）。
fn segment_rows(
    ui: &mut Ui,
    rows: &mut Rows,
    p: &mut Params<'_>,
    ramp: &mut Ramp,
    k: usize,
    result: &mut Option<Change>,
) {
    let lang = p.lang;
    let enabled = p.enabled;
    let curved = ramp.segment_curve(k).is_some();
    if p.features.mixing {
        if let Some(on) = toggle_row(
            ui,
            rows,
            "ramp.segment.curve",
            lang.pick("混合率曲線", "Mixing Curve"),
            curved,
            Some(lang.pick(
                "この分岐点から次の分岐点までの、色の混ざり方を曲線で決める（中点の代わり）",
                "Shape the mix between this stop and the next with a curve (instead of the midpoint)",
            )),
            enabled,
        ) {
            let curve = on.then(|| Ramp::curve_from_midpoint(ramp.colors()[k].midpoint));
            if let Ok(next) = ramp.with_segment_curve(k, curve) {
                *ramp = next;
                put(result, ramp, true);
            }
        }
    }
    if let Some(curve) = ramp.segment_curve(k).cloned() {
        let r = rows.row(curve::HEIGHT, 4.0);
        if let Some(next) = curve::curve_editor_with(
            ui,
            r,
            (p.key, "ramp.segment.editor", k),
            &curve,
            &CurveStyle {
                line: None,
                backdrop: None,
                diagonal: true,
            },
            lang.pick(
                "横: 2 つの分岐点の間の位置。縦: 左の分岐点の色（下）から右の分岐点の色（上）への混ざり具合。何も無い所を押すと点を追加し、ドラッグで動かし、右クリックで消す",
                "Across: position between the two stops. Up: from the left stop color (bottom) to the right stop color (top). Click to add a point, drag to move, right-click to remove",
            ),
            enabled,
        ) {
            if let Ok(updated) = ramp.with_segment_curve(k, Some(next)) {
                *ramp = updated;
                put(result, ramp, true);
            }
        }
    } else {
        let mid = ramp.colors()[k].midpoint;
        if let Some(v) = percent_row(
            ui,
            rows,
            "ramp.midpoint",
            lang.pick("中点", "Midpoint"),
            mid,
            (0.01, 0.99),
            None,
            enabled,
        ) {
            let mut list: Vec<ColorStop> = ramp.colors().to_vec();
            list[k].midpoint = v;
            if let Some(next) = ops::with_colors(ramp, list) {
                *ramp = next;
                put(result, ramp, false);
            }
        }
    }
}

/// ランプ全体の値のカーブ（塗りつぶしのグラデーション）。
fn value_curve_rows(
    ui: &mut Ui,
    rows: &mut Rows,
    p: &mut Params<'_>,
    ramp: &mut Ramp,
    result: &mut Option<Change>,
) {
    let lang = p.lang;
    group_label(ui, rows, lang.pick("値のカーブ", "Value Curve"));
    let names = [
        lang.pick("線形", "Linear"),
        lang.pick("やわらかい", "Soft"),
        lang.pick("かたい", "Hard"),
        lang.pick("S 字", "S-curve"),
    ];
    const IDS: [&str; 4] = [
        "ramp.vc.linear",
        "ramp.vc.soft",
        "ramp.vc.hard",
        "ramp.vc.s",
    ];
    let items: Vec<ChoiceButton> = names
        .iter()
        .enumerate()
        .map(|(i, name)| ChoiceButton {
            id: IDS[i],
            label: name,
            selected: false,
            enabled: p.enabled,
            tooltip: None,
        })
        .collect();
    if let Some(i) = choice_buttons(ui, rows, &items) {
        if let Some(next) = ops::curve_preset(ramp, i) {
            *ramp = next;
            put(result, ramp, true);
        }
    }
    let r = rows.row(ramp::CURVE_HEIGHT, 4.0);
    if let Some(next) = ramp::curve_editor(
        ui,
        r,
        (p.key, "ramp.vc.editor"),
        ramp,
        lang.pick(
            "形の値（横）からランプの位置（縦）へ。何も無い所を押すと点を追加し、ドラッグで動かし、右クリックで消す。Esc でドラッグをやめる",
            "Shape value in (across), gradient position out (up). Click to add a point, drag to move, right-click to remove. Escape cancels a drag",
        ),
        p.enabled,
    ) {
        *ramp = next;
        put(result, ramp, true);
    }
}
