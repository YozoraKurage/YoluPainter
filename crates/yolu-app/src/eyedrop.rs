//! スポイト: 2D のキャンバスか 3D のビューで押した所の値を、描画色かマテリアルの値に取る。スポイトのツール（I）は押した瞬間に終わる
//! （ドラッグは持たない。Unity 版と同じ）。どのツールからでも、2D は右ボタンを押す（押したまま動かすと見本が付いてくる。離して決める。Esc で取りやめ）、
//! 3D は右ボタンを動かさずに離す（動かせば視点を回す）で働く（ポリゴン塗りつぶしのツールの右ボタンは、アイランドのメニュー）。
//! スポイトで取っている間は、ポインタに印（スポイトの絵と、今の色｜ポインタの下の色の輪。`eyedrop_mark`）を出す。
//!
//! - **取るチャンネルは描くチャンネル**: 描画色は描くチャンネルへそのまま描かれるので、そのチャンネルの値を取れば、押した所と同じ値で
//!   塗れる。値は 1 画素の合成（表示と同じ式）。選んでいるレイヤーだけ（既定）か、全レイヤーの合成かはオプションバーで選ぶ（レイヤーだけは、
//!   フィルターなどの効果を通したレイヤーの出力）。選んでいるのがグループ・レイヤーが無いときは合成。
//! - **マテリアルで塗るがオン**（マスクに描くあいだを除く）なら、6 つの標準のチャンネルを全部読んで、Color は描画色、Emission は
//!   色、Roughness・Metallic・Height は R、Normal は R・G の傾きにする。組（どのチャンネルを塗るか）は変えない。透明なチャンネルは
//!   値を変えず、1 つも読めなければ何も変えない。
//! - 3D はポインタの下の面の UV から、その面を受け持つテクスチャセットの文書のテクセル（⌊u·幅⌋、u = 1 は最後の列）を読む。ほかの
//!   テクスチャセットの面ならそのセットの合成から（今のセットは切り替えない）。
//! - 透明（何も無い）と、読めなかった（効果のレイヤーの評価が予算などで断られた）は別: 読めなかったときは core の短い理由を状態の帯に
//!   出し、何も変えない（マテリアルの値は 6 つのうち 1 つでも読めなければ 1 つも変えない）。
//! - 文書は変えない（Undo の履歴に入らない）。

use egui::{pos2, vec2, Color32, Pos2, Rect, Ui};
use yolu_core::geometry::pick;
use yolu_core::glam::Vec2;

use crate::canvas::view::CanvasView;
use crate::engine::{Channel, ChannelKind, CoreError, Document, LayerId, LayerKind, Rgba8};
use crate::lang::Lang;
use crate::m2::channel_name;
use crate::matpaint::CHANNELS;
use crate::notice::Source;
use crate::panels::properties::toggle_row;
use crate::state::{AppState, StrokeSource, Tool};
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, Rows};

/// スポイトの設定（画面の状態。文書には入らない）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EyedropState {
    /// 選んでいるレイヤーでなく、全レイヤーの合成から取る。
    pub all_layers: bool,
    pub screen_request: Option<crate::screen_pick::Mode>,
    /// ランプの色の分岐点のスポイトが、画面の色を待っている（`screen_pick` は取れた色を描画色でなく `ramp_stop_pick` へ入れる）。
    pub ramp_stop_pending: bool,
    /// 画面から取れた、ランプの色の分岐点へ当てる色（ランプの欄が 1 回だけ取り出す）。
    pub ramp_stop_pick: Option<[u8; 3]>,
    /// スポイトの印に出す「ポインタの下の色」の、最後に読んだ結果（同じ画素・同じ文書・同じ設定なら読み直さない）。
    pub sample_cache: Option<(SampleKey, Option<Color32>)>,
}

/// 印の見本を読んだ画素と、読み方（文書の版・選んでいるレイヤー・チャンネルが変わったら読み直す）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SampleKey {
    set: usize,
    x: u32,
    y: u32,
    revision: u64,
    layer: Option<LayerId>,
    channel: Channel,
}

/// 右ボタン（ペンのサイドボタン）でスポイトを始めている途中。2D は押した所から動かすと `at` が付いてくる（`sample` は使わず、毎フレーム読む）。
/// 3D は押した所のまま（`at`）で、`sample` は押したときに読んだ見本（動かしたらスポイトをやめて視点を回す）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RightPress {
    pub source: StrokeSource,
    pub at: Pos2,
    pub sample: Option<Color32>,
    /// 押したボタン（そのボタンを離して決める。組み合わせの表で、スポイトを別のボタンにもできる）。
    pub button: egui::PointerButton,
}

/// 押したときにスポイトとして働くか（スポイトのツール。描くツールの右ボタンは `RightPress`）。
pub fn picks(app: &AppState) -> bool {
    app.tool == Tool::Eyedropper
}

/// 2D の右ボタン（ペンのサイドボタン）でスポイトを始める。描いている最中・ほかのスポイトの途中は始めない。
pub fn right_begin(
    app: &mut AppState,
    source: StrokeSource,
    at: Pos2,
    button: egui::PointerButton,
) -> bool {
    if app.is_stroking() || app.canvas.eyedrop.is_some() || left_drag_in_progress(app) {
        return false;
    }
    app.canvas.eyedrop = Some(RightPress {
        source,
        at,
        sample: None,
        button,
    });
    true
}

/// 左ボタンのドラッグの途中か（選択・移動と変形・図形・グラデーション・パスなど、`is_stroking` に入らないドラッグと、表示の回す・動かす・拡縮も）。
fn left_drag_in_progress(app: &AppState) -> bool {
    app.canvas.rotating.is_some()
        || app.canvas.panning
        || app.canvas.zooming.is_some()
        || app.region.drag.is_some()
        || crate::tools::input::CanvasKind::ALL
            .iter()
            .any(|kind| kind.handler().dragging(app, None))
}

/// 2D の右ボタンを押したまま動かした（印の見本が付いてくる）。
pub fn right_move(app: &mut AppState, source: StrokeSource, at: Pos2) {
    if let Some(press) = app.canvas.eyedrop.as_mut() {
        if press.source == source {
            press.at = at;
        }
    }
}

/// 2D の右ボタンを離した: 離した所の値を取る（`apply`。キャンバスの表示域の外・上に別の物がある所で離した、取りこぼした、Esc などの取りやめなら
/// 何もしない）。この入力が始めたスポイトでなければ何もしない。
pub fn right_end(
    app: &mut AppState,
    view: &CanvasView,
    source: StrokeSource,
    at: Pos2,
    apply: bool,
) {
    if app
        .canvas
        .eyedrop
        .is_none_or(|press| press.source != source)
    {
        return;
    }
    app.canvas.eyedrop = None;
    if apply {
        pick_canvas(app, view, at);
    }
}

/// 2D キャンバスの `at`（画面の点）の値を取る。取れたら true（理由はステータスバー）。キャンバスの外は何もしない。
pub fn pick_canvas(app: &mut AppState, view: &CanvasView, at: Pos2) -> bool {
    let (x, y) = view.to_canvas(at);
    let (w, h) = (app.doc.width() as f64, app.doc.height() as f64);
    if !(0.0..w).contains(&x) || !(0.0..h).contains(&y) {
        return false;
    }
    pick_texel(
        app,
        app.sets.current_index(),
        x.floor() as u32,
        y.floor() as u32,
    )
}

/// 3D ビューの `at`（画面の点。`rect` は中身の表示域）の面の値を取る。取れたら true。
pub fn pick_surface(app: &mut AppState, rect: Rect, at: Pos2) -> bool {
    let lang = app.lang;
    let Some(model) = app.view3d.model.clone() else {
        return false;
    };
    let view = app.view3d.camera.view(rect.width(), rect.height());
    let p = Vec2::new(at.x - rect.left(), at.y - rect.top());
    let Some(hit) = pick(&model.geometry, &view, p) else {
        app.refuse(
            Source::Eyedropper,
            lang.pick(
                "ポインタの下にモデルがありません",
                "Nothing of the model under the pointer",
            ),
        );
        return false;
    };
    let Some(set) = (0..app.sets.len()).find(|&i| app.set_material(i) == Some(hit.material)) else {
        app.refuse(
            Source::Eyedropper,
            lang.pick(
                "この面にはテクスチャセットがありません",
                "No texture set for this surface",
            ),
        );
        return false;
    };
    let (w, h) = {
        let doc = app.set_doc(set);
        (doc.width(), doc.height())
    };
    let Some((x, y)) = texel_of(hit.uv, w, h) else {
        app.refuse(
            Source::Eyedropper,
            lang.pick(
                "この面の UV はテクスチャの外です",
                "This surface's UV is outside the texture",
            ),
        );
        return false;
    };
    pick_texel(app, set, x, y)
}

/// 取る元のレイヤー（選んでいるレイヤーだけ。全レイヤーを対象にしているとき・選んでいるのがグループ・レイヤーが無いとき・今のセットでないセットは合成から = None）。
fn source_layer(app: &AppState, set: usize) -> Option<LayerId> {
    let current = set == app.sets.current_index();
    if current && !app.eyedrop.all_layers {
        app.selected_layer
            .and_then(|id| app.doc.layer(id))
            .filter(|l| l.kind() != LayerKind::Group)
            .map(|l| l.id())
    } else {
        None
    }
}

/// 印に出す色（スカラーのチャンネルは灰色の濃さ。ほかは RGB。透明度は見せない）。
fn swatch(doc: &Document, channel: Channel, px: Rgba8) -> Color32 {
    match doc.channel_info(channel).map(|c| c.kind) {
        Some(ChannelKind::Scalar) => Color32::from_gray(px.r),
        _ => Color32::from_rgb(px.r, px.g, px.b),
    }
}

/// スポイトの印の「今の色」（描画色。スカラーのチャンネルを描いているときは灰色の濃さ）。
pub fn current_swatch(app: &AppState) -> Color32 {
    let [r, g, b, _] = app
        .color
        .main
        .map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
    let scalar = !app.paints_material()
        && app
            .doc
            .channel_info(app.m2.paint_channel)
            .is_some_and(|c| c.kind == ChannelKind::Scalar);
    if scalar {
        Color32::from_gray(r)
    } else {
        Color32::from_rgb(r, g, b)
    }
}

/// セット `set` の画素 (x, y) の、印に出す色（取るのと同じ元・同じチャンネル。マテリアルで塗るときは Color）。透明・読めない・範囲の外は None。
/// 同じ画素・同じ文書の版・同じ設定なら読み直さない（毎フレーム呼んでよい）。文書は変えず、知らせも出さない。
pub fn sample_texel(app: &mut AppState, set: usize, x: u32, y: u32) -> Option<Color32> {
    let layer = source_layer(app, set);
    let channel = if app.paints_material() {
        Channel::Color
    } else {
        app.m2.paint_channel
    };
    let key = SampleKey {
        set,
        x,
        y,
        revision: app.set_doc(set).revision(),
        layer,
        channel,
    };
    if let Some((cached, color)) = app.eyedrop.sample_cache {
        if cached == key {
            return color;
        }
    }
    let doc = app.set_doc(set);
    let color = match read_pixel(doc, layer, channel, x, y) {
        Ok(Some(px)) => Some(swatch(doc, channel, px)),
        _ => None,
    };
    app.eyedrop.sample_cache = Some((key, color));
    color
}

/// 2D キャンバスの `at`（画面の点）の、印に出す色。キャンバスの外は None。
pub fn sample_canvas(app: &mut AppState, view: &CanvasView, at: Pos2) -> Option<Color32> {
    let (x, y) = view.to_canvas(at);
    let (w, h) = (app.doc.width() as f64, app.doc.height() as f64);
    if !(0.0..w).contains(&x) || !(0.0..h).contains(&y) {
        return None;
    }
    let set = app.sets.current_index();
    sample_texel(app, set, x.floor() as u32, y.floor() as u32)
}

/// 3D ビューの `at`（画面の点）の面の、印に出す色。モデルが無い・面に当たらない・テクスチャセットが無い・UV が外は None。
pub fn sample_surface(app: &mut AppState, rect: Rect, at: Pos2) -> Option<Color32> {
    let model = app.view3d.model.clone()?;
    let view = app.view3d.camera.view(rect.width(), rect.height());
    let hit = pick(
        &model.geometry,
        &view,
        Vec2::new(at.x - rect.left(), at.y - rect.top()),
    )?;
    let set = (0..app.sets.len()).find(|&i| app.set_material(i) == Some(hit.material))?;
    let (w, h) = {
        let doc = app.set_doc(set);
        (doc.width(), doc.height())
    };
    let (x, y) = texel_of(hit.uv, w, h)?;
    sample_texel(app, set, x, y)
}

/// 面の UV が指すテクセル（幅 `w`・高さ `h` の文書。⌊u·幅⌋・⌊v·高さ⌋で、u = 1・v = 1 は最後の列・行）。UV が 0〜1 の外なら None
/// （繰り返し・はみ出しの面は読まない）。
pub(crate) fn texel_of(uv: Vec2, w: u32, h: u32) -> Option<(u32, u32)> {
    if !(0.0..=1.0).contains(&uv.x) || !(0.0..=1.0).contains(&uv.y) {
        return None;
    }
    let x = ((uv.x as f64 * w as f64).floor() as u32).min(w - 1);
    let y = ((uv.y as f64 * h as f64).floor() as u32).min(h - 1);
    Some((x, y))
}

/// 1 画素を読む口（`read_pixel`。試験は、読めない文書を作れないので、断る口に差し替える）。
type Reader<'a> =
    &'a dyn Fn(&Document, Option<LayerId>, Channel, u32, u32) -> Result<Option<Rgba8>, CoreError>;

/// セット `set` の文書の画素 (x, y) の値を取る。
pub fn pick_texel(app: &mut AppState, set: usize, x: u32, y: u32) -> bool {
    pick_texel_with(app, set, x, y, &read_pixel)
}

fn pick_texel_with(app: &mut AppState, set: usize, x: u32, y: u32, reader: Reader<'_>) -> bool {
    let lang = app.lang;
    let layer = source_layer(app, set);
    let material = app.paints_material();
    let wanted: Vec<Channel> = if material {
        CHANNELS.to_vec()
    } else {
        vec![app.m2.paint_channel]
    };
    let mut read: Vec<(Channel, Rgba8)> = Vec::new();
    {
        let doc = app.set_doc(set);
        for channel in wanted {
            match reader(doc, layer, channel, x, y) {
                Ok(Some(px)) => read.push((channel, px)),
                // 透明は読み飛ばす（ほかのチャンネルは取る）
                Ok(None) => {}
                // 読めなかった: 理由を出して、何も変えない
                Err(e) => {
                    app.notify(
                        crate::notice::Kind::of_core(&e),
                        Source::Eyedropper,
                        lang.core_error(&e),
                    );
                    return false;
                }
            }
        }
    }
    if read.is_empty() {
        app.refuse(
            Source::Eyedropper,
            lang.pick(
                "そこには何もありません（透明）。",
                "Nothing to pick there (transparent).",
            ),
        );
        return false;
    }
    if material {
        let mut names = Vec::new();
        for (channel, px) in &read {
            let v = |b: u8| b as f32 / 255.0;
            match *channel {
                Channel::Color => app.color.set_main([v(px.r), v(px.g), v(px.b), 1.0]),
                Channel::Emission => app.mat.emission = [v(px.r), v(px.g), v(px.b)],
                Channel::Normal => app.mat.set_normal(v(px.r) * 2.0 - 1.0, v(px.g) * 2.0 - 1.0),
                other => app.mat.set_scalar(other, v(px.r)),
            }
            names.push(channel_name(lang, &app.doc, *channel));
        }
        let joined = names.join(lang.pick("・", ", "));
        app.info(
            Source::Eyedropper,
            format!("{} {joined}", lang.pick("取得:", "Picked")),
        );
    } else {
        let px = read[0].1;
        let v = |b: u8| b as f32 / 255.0;
        app.color.set_main([v(px.r), v(px.g), v(px.b), 1.0]);
        app.info(
            Source::Eyedropper,
            format!(
                "{} R {} G {} B {}",
                lang.pick("取得:", "Picked"),
                px.r,
                px.g,
                px.b
            ),
        );
    }
    true
}

/// 1 チャンネルの 1 画素（透明、またはそのチャンネルが無ければ Ok(None)。読めなければ core の理由）。`layer` があればそのレイヤーの画素、
/// 無ければ合成。
fn read_pixel(
    doc: &Document,
    layer: Option<LayerId>,
    channel: Channel,
    x: u32,
    y: u32,
) -> Result<Option<Rgba8>, CoreError> {
    if doc.channel_info(channel).is_none() {
        return Ok(None);
    }
    // レイヤーだけなら、フィルターなどの効果を通したレイヤーの出力（マスク・不透明度・合成の前）。表示と同じ値になる
    let px = match layer {
        Some(id) => doc.layer_output_pixel(id, channel, x, y)?,
        None => doc.composite_pixel(channel, x, y)?,
    };
    Ok((px.a > 0).then_some(px))
}

/// オプションバーの欄（`x` から右へ）。
pub fn options(ui: &mut Ui, app: &mut AppState, r: Rect, x: f32) {
    let x = x + 4.0;
    let lang = app.lang;
    let (y, h) = (r.top() + 6.0, r.height() - 12.0);
    let width = 22.0 + w::text_width(ui.painter(), all_layers_label(lang), t::LABEL) + 8.0;
    if x + width > r.right() - 8.0 {
        return;
    }
    app.eyedrop.all_layers = w::toggle(
        ui,
        Rect::from_min_size(pos2(x, y), vec2(width, h)),
        "options.eyedropper.all-layers",
        all_layers_label(lang),
        app.eyedrop.all_layers,
        Some(lang.pick(
            "選んだレイヤーでなく、チャンネルの合成から取る",
            "Pick from the composite instead of the selected layer",
        )),
        true,
    );
    crate::screen_pick::options(ui, app, r, x + width + 8.0);
}

/// ツールプロパティ: 取る元（選んだレイヤーか全レイヤーか）と、画面から取る（Windows）。取る元はサブツールの一覧でも選べる。
pub fn props(ui: &mut Ui, app: &mut AppState, rows: &mut Rows, _ctx: &egui::Context) {
    let lang = app.lang;
    if let Some(v) = toggle_row(
        ui,
        rows,
        "props.eyedropper.all-layers",
        all_layers_label(lang),
        app.eyedrop.all_layers,
        Some(lang.pick(
            "選んだレイヤーでなく、チャンネルの合成から取る",
            "Pick from the composite instead of the selected layer",
        )),
        true,
    ) {
        app.eyedrop.all_layers = v;
    }
    crate::screen_pick::props(ui, app, rows);
}

fn all_layers_label(lang: Lang) -> &'static str {
    lang.pick("全レイヤーを対象", "Sample All Layers")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang::Lang;
    use crate::matpaint::MatAction;
    use crate::state::Action;

    /// 画素 (3, 3) だけが不透明な、スポイトのツールの状態。
    fn state() -> AppState {
        let mut s = AppState::new(32, 32);
        s.tool = Tool::Eyedropper;
        let id = s.selected_layer.unwrap();
        for channel in [Channel::Color, Channel::Roughness] {
            s.doc
                .set_channel_pixel(id, channel, 3, 3, Rgba8::new(90, 80, 70, 255))
                .unwrap();
        }
        s
    }

    fn refuse(
        _: &Document,
        _: Option<LayerId>,
        _: Channel,
        _: u32,
        _: u32,
    ) -> Result<Option<Rgba8>, CoreError> {
        Err(CoreError::WorkingBudgetExceeded)
    }

    #[test]
    fn a_refused_read_says_why_and_is_not_taken_for_a_transparent_texel() {
        let mut s = state();
        s.color.set_main([0.25, 0.5, 0.75, 1.0]);
        let before = s.color.clone();
        // 同じ画素でも、透明は「何もありません」、読めないのは理由（作業メモリの予算）
        assert!(!pick_texel_with(&mut s, 0, 20, 20, &read_pixel));
        assert_eq!(s.message, "そこには何もありません（透明）。");
        assert!(!pick_texel_with(&mut s, 0, 3, 3, &refuse));
        assert_eq!(
            s.message,
            Lang::Ja.core_error(&CoreError::WorkingBudgetExceeded)
        );
        assert!(!s.message.contains("透明"), "{}", s.message);
        assert_eq!(s.color, before, "何も変えない");
        s.lang = Lang::En;
        assert!(!pick_texel_with(&mut s, 0, 3, 3, &refuse));
        assert_eq!(s.message, "Working memory budget exceeded");
        assert_eq!(s.color, before);
        // 読めれば取る
        assert!(pick_texel_with(&mut s, 0, 3, 3, &read_pixel));
        assert_eq!(s.message, "Picked R 90 G 80 B 70");
    }

    #[test]
    fn in_material_mode_one_refused_channel_changes_none_of_the_values() {
        let mut s = state();
        s.apply(Action::Mat(MatAction::Enabled(true)));
        let (mat, color) = (s.mat.clone(), s.color.clone());
        // Roughness だけ断る。先に読めた Color（と、あとに読める分）も反映しない
        let only_roughness =
            |doc: &Document, layer: Option<LayerId>, channel: Channel, x: u32, y: u32| {
                if channel == Channel::Roughness {
                    Err(CoreError::WorkingBudgetExceeded)
                } else {
                    read_pixel(doc, layer, channel, x, y)
                }
            };
        assert!(!pick_texel_with(&mut s, 0, 3, 3, &only_roughness));
        assert_eq!(
            s.message,
            Lang::Ja.core_error(&CoreError::WorkingBudgetExceeded)
        );
        assert_eq!((s.mat.clone(), s.color.clone()), (mat, color));
        // 読めれば、Color と Roughness を取る
        assert!(pick_texel_with(&mut s, 0, 3, 3, &read_pixel));
        assert_eq!(s.message, "取得: カラー・ラフネス");
    }

    #[test]
    fn the_texel_under_a_uv_follows_the_floor_rule_with_the_last_column_at_one() {
        let uv = |u: f32, v: f32| Vec2::new(u, v);
        // ⌊u·幅⌋・⌊v·高さ⌋（幅と高さは別でもよい）
        assert_eq!(texel_of(uv(0.54, 0.38), 128, 128), Some((69, 48)));
        assert_eq!(texel_of(uv(0.5, 0.25), 64, 32), Some((32, 8)));
        // 端: 0 は最初、1 は最後の列・行（幅そのものにはならない）
        assert_eq!(texel_of(uv(0.0, 0.0), 64, 32), Some((0, 0)));
        assert_eq!(texel_of(uv(1.0, 1.0), 64, 32), Some((63, 31)));
        assert_eq!(texel_of(uv(1.0, 0.0), 64, 32), Some((63, 0)));
        assert_eq!(texel_of(uv(0.0, 1.0), 64, 32), Some((0, 31)));
        // テクセルの境目: ちょうどはそのテクセル、手前は 1 つ前
        assert_eq!(texel_of(uv(0.5, 0.5), 64, 64), Some((32, 32)));
        assert_eq!(texel_of(uv(0.499, 0.499), 64, 64), Some((31, 31)));
        // 1 テクセルの文書
        assert_eq!(texel_of(uv(1.0, 1.0), 1, 1), Some((0, 0)));
        // 0〜1 の外・数でない値は読まない
        for (u, v) in [
            (-0.0001, 0.5),
            (0.5, -1.0),
            (1.0001, 0.5),
            (0.5, 2.0),
            (f32::NAN, 0.5),
            (0.5, f32::INFINITY),
        ] {
            assert_eq!(texel_of(uv(u, v), 64, 64), None, "{u} {v}");
        }
    }

    #[test]
    fn a_missing_channel_is_not_a_refusal() {
        let s = state();
        let doc = &s.doc;
        let gone = Channel::from_index(42).unwrap();
        assert_eq!(read_pixel(doc, None, gone, 3, 3), Ok(None));
        assert_eq!(
            read_pixel(doc, None, Channel::Color, 3, 3),
            Ok(Some(Rgba8::new(90, 80, 70, 255)))
        );
        assert_eq!(read_pixel(doc, None, Channel::Color, 4, 4), Ok(None));
        // 範囲の外は読めない（透明ではない）
        assert!(matches!(
            read_pixel(doc, None, Channel::Color, 99, 3),
            Err(CoreError::InvalidArgument(_))
        ));
    }
}
