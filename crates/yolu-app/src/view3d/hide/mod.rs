//! ハイドのモード: 3D ビューごとに面を隠す。隠すのはそのビューの見え方と当たりだけ（描く・範囲のツール・スポイト・ブラシのカーソルは、
//! そのビューで隠した面を見ない）。2D のキャンバス・保存・書き出しは変わらない。`.ylp` にも入れず、取り消しにも入れない。
//!
//! - 隠し方はビューごと（`ViewState::hide`）: UV アイランド（`islands` の索引の番号）・ボーンの影響の項目と入れているプリセット
//!   （`pose::hide::HideState`）・シェルフの目（閉じれば、そのビューでは何も隠さない）。見せる形へ入れるのは `refresh_view`
//!   （UV アイランドとボーンの和を `View3dState::set_view_face_mask` へ）。目を閉じたテクスチャセット（`View3dState::hidden`）は
//!   全部のビューで共通のまま。
//! - 全部の面が隠れる隠し方と、描いている最中の変更は断る（元の隠し方へ戻して理由を知らせる）。
//! - ビューごとのポーズ（シェルフのポーズのプリセット）: `ViewPose`。プリセット・休みの形を選んだビューは、その形を `View3dState::
//!   set_view_pose_model` で持つ（同じポーズのビューは同じ形を共有する）。今のポーズ（ポーズのモードで変えるもの）はセッションで 1 つ。
//! - 画面: 左のツールの帯・オプションバー（`strip`）、3D ビューの右の引き出し（`panels::hide_drawer`）、左下のシェルフ（`shelf`）、選んだ・指した
//!   アイランドの重ねと隠した面の輪郭（`overlay`）、押し・四角の選び（`input`）。

pub mod input;
pub mod islands;
pub mod overlay;
pub mod shelf;
pub mod strip;

use std::sync::Arc;

use yolu_core::skin::Pose;

use self::islands::IslandIndex;
use super::pose::hide::{self as bone_hide, FaceMask, HideState};
use super::ViewId;
use crate::lang::Lang;
use crate::notice::Source;
use crate::state::AppState;

/// ハイドのモードの選ぶツール（左の帯）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HideTool {
    /// 押した面の UV アイランドを選ぶ（Shift で足す・Ctrl で外す）。
    #[default]
    Island,
    /// 四角で囲んだ UV アイランドを選ぶ（Shift で足す・Ctrl で外す）。
    Rectangle,
}

impl HideTool {
    pub const ALL: [HideTool; 2] = [Self::Island, Self::Rectangle];

    pub fn name(self, lang: Lang) -> &'static str {
        match self {
            Self::Island => lang.t("hide-select-uv-island"),
            Self::Rectangle => lang.t("hide-rectangle-select"),
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::Island => "tools/id-select",
            Self::Rectangle => "tools/select-rectangle",
        }
    }
}

/// ビューのポーズ（シェルフのポーズのプリセット）。
#[derive(Clone, Debug, Default, PartialEq)]
pub enum ViewPose {
    /// 今のポーズ（ポーズのモードで変えるもの。全部のビューで共通）。
    #[default]
    Current,
    /// 休みの形（ファイルにあったときのポーズ）。
    Rest(Pose),
    /// ポーズのプリセット（番号と、当てたポーズ。選んだときに作る）。
    Preset(u32, Pose),
}

impl ViewPose {
    /// 当てるポーズ（今のポーズなら None）。
    pub fn pose(&self) -> Option<&Pose> {
        match self {
            Self::Current => None,
            Self::Rest(p) | Self::Preset(_, p) => Some(p),
        }
    }
}

/// シェルフで選ぶポーズ（`ViewPose` の、ポーズを作る前の選び）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PoseChoice {
    Current,
    Rest,
    Preset(u32),
}

/// 1 つのビューの隠し方と、ハイドのモードの画面の状態（保存しない）。
#[derive(Clone, Debug)]
pub struct ViewHide {
    /// ボーンの影響で隠す項目と、入れているプリセット。
    pub bones: HideState,
    /// 隠している UV アイランド（`IslandIndex` の番号。隠した順）。
    pub islands: Vec<u32>,
    /// 選んでいる UV アイランド（番号。選んだ順）。
    pub selected: Vec<u32>,
    /// シェルフの目（閉じていれば、このビューでは何も隠さない）。
    pub applied: bool,
    /// 右の引き出しを開いているか（ハイドのモードの間に出す）。
    pub drawer_open: bool,
    /// 引き出しのスクロールと、前のフレームの中身の高さ。
    pub drawer_scroll: f32,
    pub drawer_content: f32,
    /// 左下のシェルフを開いているか。
    pub shelf_open: bool,
    /// ポインタの下の UV アイランド（ハイドのモードの重ね）。
    pub pointed: Option<u32>,
    /// 四角で選んでいる途中（押した点・今の点。画面の点）と、押したときの組み合わせ方。
    pub rect: Option<input::RectDrag>,
    /// 重ねの塗る三角形と輪郭（カメラと選びが同じなら使い回す）。
    pub overlay: overlay::Cache,
}

impl Default for ViewHide {
    fn default() -> Self {
        ViewHide {
            bones: HideState::default(),
            islands: Vec::new(),
            selected: Vec::new(),
            applied: true,
            drawer_open: true,
            drawer_scroll: 0.0,
            drawer_content: 0.0,
            shelf_open: false,
            pointed: None,
            rect: None,
            overlay: overlay::Cache::default(),
        }
    }
}

impl ViewHide {
    /// 何も隠していないか（目の開閉は見ない）。
    pub fn is_clear(&self) -> bool {
        self.islands.is_empty() && self.bones.is_clear()
    }

    /// 隠し方を全部外す（選び・開閉は残す）。
    fn clear(&mut self) {
        self.islands.clear();
        self.bones.entries.clear();
        self.bones.presets.clear();
        self.bones.skipped.clear();
    }
}

/// ハイドのモードの、全部のビューで共通の状態。
#[derive(Debug)]
pub struct HideShared {
    pub tool: HideTool,
    /// 隠した面の輪郭を出す（ハイドのモードの間）。
    pub outline: bool,
    /// 隠したあとも選びを残す。
    pub keep_selection: bool,
    /// 受けたままの形（休みの形）の UV アイランドの索引（形が替わるまで作り直さない）。
    index: Option<Arc<IslandIndex>>,
}

impl Default for HideShared {
    fn default() -> Self {
        HideShared {
            tool: HideTool::Island,
            outline: true,
            keep_selection: false,
            index: None,
        }
    }
}

/// ハイドのモードの操作（帯・引き出し・シェルフ・キーから）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HideAction {
    Tool(HideTool),
    /// 今のビューで見えている UV アイランドを全部選ぶ（Ctrl+A）。
    SelectAll,
    /// 選びを外す（Ctrl+D）。
    Deselect,
    /// 選んだ UV アイランドを隠す（H）。
    HideSelected,
    /// 今のビューの隠し方を全部外す（Alt+H）。
    ShowAll,
    /// 今のビューで隠している UV アイランドの 1 つを外す（`ViewHide::islands` の並びの番号）。
    ShowIsland(usize),
    /// 選んでいるボーンの影響で隠す（今のビュー）。
    HideBone,
    /// このビューの隠し方を当てる・外す（シェルフの目）。
    ToggleApplied(ViewId),
    /// このビューの引き出しを開く・閉じる。
    ToggleDrawer(ViewId),
    /// このビューのシェルフを開く・閉じる。
    ToggleShelf(ViewId),
    /// このビューの隠し方をハイドのプリセット 1 つにする（None は何も隠さない）。
    ChoosePreset(ViewId, Option<u32>),
    /// このビューのポーズを選ぶ。
    ChoosePose(ViewId, PoseChoice),
    /// 隠した面の輪郭を出す・出さない。
    SetOutline(bool),
    /// 隠したあとも選びを残す・残さない。
    SetKeepSelection(bool),
}

/// 今の受けたままの形（休みの形）の UV アイランドの索引（モデルが無ければ None。形が替わるまで作り直さない）。
pub fn island_index(app: &mut AppState) -> Option<Arc<IslandIndex>> {
    let full = app.view3d.full_model()?.clone();
    let g = full.rest_geometry().clone();
    let shared = &mut app.view3d.hiding;
    if !shared.index.as_ref().is_some_and(|i| i.is_for(&g)) {
        shared.index = Some(Arc::new(IslandIndex::new(&g)));
    }
    shared.index.clone()
}

/// 作ってある索引（作らない。描くときに読む）。
pub fn cached_index(app: &AppState) -> Option<Arc<IslandIndex>> {
    let full = app.view3d.full_model()?;
    app.view3d
        .hiding
        .index
        .clone()
        .filter(|i| i.is_for(full.rest_geometry()))
}

/// このビューの隠し方から作る、見せる形から除く面（目を閉じていれば・何も隠さなければ None）と、ボーンのプリセットの飛ばした項目。
fn mask_of(app: &mut AppState, id: ViewId) -> (Option<FaceMask>, Vec<bone_hide::Skipped>) {
    let Some(total) = app.view3d.full_model().map(|m| m.triangle_count()) else {
        return (None, Vec::new());
    };
    let Some(hide) = app.view3d.view_of(id).map(|v| v.hide.clone()) else {
        return (None, Vec::new());
    };
    let mut hidden = vec![false; total];
    let mut any = false;
    let mut skipped = Vec::new();
    if let Some(s) = app.view3d.pose.session.as_ref() {
        let (entries, skip) =
            bone_hide::effective(&s.rig, &hide.bones, &app.view3d.pose.hide_presets);
        skipped = skip;
        if let Some(m) = bone_hide::mask_for(&s.rig, &entries) {
            if m.len() == total {
                for (t, h) in hidden.iter_mut().enumerate() {
                    if m.is_hidden(t) {
                        *h = true;
                        any = true;
                    }
                }
            }
        }
    }
    if !hide.islands.is_empty() {
        if let Some(index) = island_index(app).filter(|i| i.triangle_count() == total) {
            for island in &hide.islands {
                for &t in index.triangles(*island) {
                    hidden[t as usize] = true;
                    any = true;
                }
            }
        }
    }
    if !hide.applied || !any {
        return (None, skipped);
    }
    (Some(FaceMask::new(hidden)), skipped)
}

/// 全部の面が隠れるので入れられなかった。
pub struct AllHidden;

/// このビューの隠し方から面の印を作り直して、見せる形へ入れる（全部の面が隠れるなら入れない）。
pub fn refresh_view(app: &mut AppState, id: ViewId) -> Result<(), AllHidden> {
    let (mask, skipped) = mask_of(app, id);
    if let Some(m) = &mask {
        if !m.is_empty() && m.hidden_count() >= m.len() {
            return Err(AllHidden);
        }
    }
    if let Some(v) = app.view3d.view_of_mut(id) {
        v.hide.bones.skipped = skipped;
    }
    app.view3d.set_view_face_mask(id, mask.map(Arc::new));
    Ok(())
}

/// このビューの隠し方を変えて面の印を作り直す。描いている最中・全部の面が隠れる変え方は断って、元の隠し方へ戻す（理由を知らせる）。
/// 変えられたら true。
pub fn change(app: &mut AppState, id: ViewId, f: impl FnOnce(&mut ViewHide)) -> bool {
    change_from(app, id, Source::View3d, f)
}

/// 描いている最中なら、隠し方・ポーズを変えるのを断って（理由を知らせる）true を返す。隠し方を変える操作の断りはここ 1 か所。
pub fn refuse_while_stroking(app: &mut AppState, source: Source) -> bool {
    if !app.is_stroking() {
        return false;
    }
    let lang = app.lang;
    app.refuse(source, lang.view_error(&super::model::ViewError::Stroking));
    true
}

/// `change` の、知らせの出どころを決める形（ポーズの欄から変えたときはポーズ）。
pub fn change_from(
    app: &mut AppState,
    id: ViewId,
    source: Source,
    f: impl FnOnce(&mut ViewHide),
) -> bool {
    let lang = app.lang;
    if refuse_while_stroking(app, source) {
        return false;
    }
    let Some(v) = app.view3d.view_of_mut(id) else {
        return false;
    };
    let before = v.hide.clone();
    f(&mut v.hide);
    match refresh_view(app, id) {
        Ok(()) => true,
        Err(AllHidden) => {
            if let Some(v) = app.view3d.view_of_mut(id) {
                v.hide = before;
            }
            app.refuse(source, lang.t("view3d-cannot-hide-every-surface"));
            false
        }
    }
}

/// 全部のビューの隠し方・選び・ポーズを捨てる（別のモデル・別のスキンが入った。三角形とボーンの番号が別のモデルのもの）。
pub fn reset_views(view3d: &mut super::View3dState) {
    let ids = view3d.view_ids();
    for id in ids {
        if let Some(v) = view3d.view_of_mut(id) {
            let (drawer, shelf, applied) = (v.hide.drawer_open, v.hide.shelf_open, v.hide.applied);
            v.hide = ViewHide {
                drawer_open: drawer,
                shelf_open: shelf,
                applied,
                ..ViewHide::default()
            };
            v.pose = ViewPose::Current;
        }
        view3d.set_view_face_mask(id, None);
        view3d.set_view_pose_model(id, None);
    }
}

/// 今のビューで見えている（隠していない）UV アイランドの全部。
fn shown_islands(app: &mut AppState) -> Vec<u32> {
    let Some(index) = island_index(app) else {
        return Vec::new();
    };
    let material_hidden = |t: u32| {
        app.view3d
            .full_model()
            .and_then(|m| m.geometry.triangles().get(t as usize).map(|x| x.material))
            .is_some_and(|m| app.view3d.is_material_hidden(m))
    };
    (0..index.len() as u32)
        .filter(|&i| {
            index
                .triangles(i)
                .iter()
                .any(|&t| !app.view3d.is_face_hidden(t) && !material_hidden(t))
        })
        .collect()
}

/// 操作を当てる（`Action::Hide`）。
pub fn apply(app: &mut AppState, action: HideAction) {
    let id = app.view3d.active_view();
    match action {
        HideAction::Tool(tool) => app.view3d.hiding.tool = tool,
        HideAction::SelectAll => {
            let all = shown_islands(app);
            app.view3d.view.hide.selected = all;
        }
        HideAction::Deselect => app.view3d.view.hide.selected.clear(),
        HideAction::HideSelected => {
            let selected = app.view3d.view.hide.selected.clone();
            if selected.is_empty() {
                return;
            }
            let keep = app.view3d.hiding.keep_selection;
            change(app, id, |h| {
                for i in &selected {
                    if !h.islands.contains(i) {
                        h.islands.push(*i);
                    }
                }
                if !keep {
                    h.selected.clear();
                }
                // 目を閉じていたビューで隠したら、目を開ける（隠したのが見えるように）
                h.applied = true;
            });
        }
        HideAction::ShowAll => {
            change(app, id, ViewHide::clear);
        }
        HideAction::ShowIsland(at) => {
            change(app, id, |h| {
                if at < h.islands.len() {
                    h.islands.remove(at);
                }
            });
        }
        HideAction::HideBone => {
            if let Some(bone) = app.view3d.pose.session.as_ref().and_then(|s| s.selected) {
                bone_hide::hide_bone(app, bone);
            }
        }
        HideAction::ToggleApplied(view) => {
            change(app, view, |h| h.applied = !h.applied);
        }
        HideAction::ToggleDrawer(view) => {
            if let Some(v) = app.view3d.view_of_mut(view) {
                v.hide.drawer_open = !v.hide.drawer_open;
            }
        }
        HideAction::ToggleShelf(view) => {
            if let Some(v) = app.view3d.view_of_mut(view) {
                v.hide.shelf_open = !v.hide.shelf_open;
            }
        }
        HideAction::ChoosePreset(view, preset) => {
            if preset.is_some_and(|p| app.view3d.pose.hide_presets.get(p).is_none()) {
                return;
            }
            change(app, view, |h| {
                h.clear();
                h.bones.presets.extend(preset);
                h.applied = true;
            });
        }
        HideAction::ChoosePose(view, choice) => shelf::choose_pose(app, view, choice),
        HideAction::SetOutline(on) => app.view3d.hiding.outline = on,
        HideAction::SetKeepSelection(on) => app.view3d.hiding.keep_selection = on,
    }
}

/// ハイドのモードを出る（四角の選びの途中を捨て、指したアイランドの印を外す）。
pub fn leave(app: &mut AppState) {
    for v in app.view3d.views_mut() {
        v.hide.rect = None;
        v.hide.pointed = None;
    }
}

/// 隠している物の一覧の行（引き出しの「隠している物」）。
pub struct Entry {
    pub icon: &'static str,
    pub name: String,
    pub status: String,
    pub remove: Remove,
}

/// 一覧の行を外す操作。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Remove {
    /// ボーンの項目（`HideState::entries` の番号）。
    Bone(usize),
    /// プリセット（番号）。
    Preset(u32),
    /// UV アイランド（`ViewHide::islands` の並びの番号）。
    Island(usize),
}

/// 今のビューで隠している物（ボーンの項目・入れているプリセット・UV アイランドの順）。
pub fn entries(app: &AppState) -> Vec<Entry> {
    let lang = app.lang;
    let hide = &app.view3d.view.hide;
    let mut out = Vec::new();
    if let Some(s) = app.view3d.pose.session.as_ref() {
        for (i, e) in hide.bones.entries.iter().enumerate() {
            let Some(bone) = s.rig.bones().get(e.bone) else {
                continue;
            };
            out.push(Entry {
                icon: "accessibility",
                name: bone.name.clone(),
                status: format!(
                    "{}%{}",
                    (e.threshold * 100.0).round(),
                    if e.children {
                        lang.t("panels-children")
                    } else {
                        ""
                    }
                ),
                remove: Remove::Bone(i),
            });
        }
    }
    for id in &hide.bones.presets {
        if let Some(p) = app.view3d.pose.hide_presets.get(*id) {
            out.push(Entry {
                icon: "save",
                name: p.name.clone(),
                status: String::new(),
                remove: Remove::Preset(*id),
            });
        }
    }
    for (i, island) in hide.islands.iter().enumerate() {
        out.push(Entry {
            icon: "uv_wireframe",
            name: lang.f("hide-uv-island-n", &[("n", &(island + 1))]),
            status: String::new(),
            remove: Remove::Island(i),
        });
    }
    out
}

/// 一覧の行を外す。
pub fn remove(app: &mut AppState, what: Remove) {
    match what {
        Remove::Bone(i) => bone_hide::remove_entry(app, i),
        Remove::Preset(id) => bone_hide::toggle_preset(app, id),
        Remove::Island(i) => apply(app, HideAction::ShowIsland(i)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mode::{EditorMode, ModeAction};
    use crate::state::Action;
    use crate::view3d::pose::PoseAction;

    fn cube() -> AppState {
        let mut app = AppState::new(64, 64);
        app.view3d.load_demo();
        app
    }

    fn shown(app: &AppState) -> usize {
        app.view3d.model.as_ref().map_or(0, |m| m.triangle_count())
    }

    fn hide(app: &mut AppState, islands: &[u32]) {
        app.view3d.view.hide.selected = islands.to_vec();
        apply(app, HideAction::HideSelected);
    }

    #[test]
    fn hide_mode_needs_a_model_and_is_refused_while_drawing() {
        let mut app = AppState::new(32, 32);
        assert!(!app.set_mode(EditorMode::Hide));
        assert_eq!(
            app.message,
            "ハイドのモードにできません（モデルがありません）。"
        );
        let mut app = cube();
        app.apply(Action::Mode(ModeAction::Set(EditorMode::Hide)));
        assert_eq!(app.mode, EditorMode::Hide);
        // 選びと四角の途中は、出るときに捨てる
        app.view3d.view.hide.pointed = Some(1);
        app.apply(Action::Mode(ModeAction::Set(EditorMode::Paint)));
        assert_eq!(app.mode, EditorMode::Paint);
        assert_eq!(app.view3d.view.hide.pointed, None);
        // モデルが無くなったら、フレームの初めにペイントへ
        app.set_mode(EditorMode::Hide);
        app.view3d = crate::view3d::View3dState::default();
        crate::mode::leave_pose_without_rig(&mut app);
        assert_eq!(app.mode, EditorMode::Paint);
        // ポーズはボーンのあるモデルが要るまま
        let mut app = cube();
        assert!(!app.set_mode(EditorMode::Pose));
        // 描いている最中は入らない
        let layer = app.session.selected_layer.unwrap();
        let settings = app.stroke_settings(false);
        let stroke = app.session.doc.begin_stroke(layer, &settings).unwrap();
        app.session.stroke = Some(stroke);
        app.canvas.stroke = Some(crate::state::StrokeSource::Mouse);
        assert!(!app.set_mode(EditorMode::Hide));
        assert_eq!(app.mode, EditorMode::Paint);
        assert_eq!(app.message, crate::lang::refusals::during_stroke(app.lang));
        let stroke = app.session.stroke.take().unwrap();
        app.session.doc.cancel_stroke(stroke);
        app.canvas.stroke = None;
        assert!(app.set_mode(EditorMode::Hide));
    }

    #[test]
    fn hiding_uv_islands_removes_their_faces_and_show_all_brings_them_back() {
        let mut app = cube();
        let index = island_index(&mut app).unwrap();
        assert_eq!(index.len(), 6, "立方体の 6 面は 6 つのアイランド");
        hide(&mut app, &[0, 2]);
        assert_eq!(shown(&app), 8);
        assert!(
            app.view3d.view.hide.selected.is_empty(),
            "隠したら選びを外す"
        );
        assert_eq!(app.view3d.view.hide.islands, vec![0, 2]);
        // 見せる形の番号 → 受けたままの形の番号（面 0 と 2 の三角形を飛ばす）
        assert_eq!(app.view3d.full_triangle(0), Some(2));
        assert_eq!(app.view3d.full_triangle(2), Some(6));
        assert!(app.view3d.is_face_hidden(0) && app.view3d.is_face_hidden(5));
        assert!(!app.view3d.is_face_hidden(2));
        let e = entries(&app);
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].name, "UV アイランド 1");
        // 1 つだけ外す
        remove(&mut app, e[1].remove);
        assert_eq!(shown(&app), 10);
        // すべて表示
        apply(&mut app, HideAction::ShowAll);
        assert_eq!(shown(&app), 12);
        assert!(Arc::ptr_eq(
            app.view3d.model.as_ref().unwrap(),
            app.view3d.full_model().unwrap()
        ));
        // 隠したあとも選びを残す設定
        apply(&mut app, HideAction::SetKeepSelection(true));
        hide(&mut app, &[3]);
        assert_eq!(app.view3d.view.hide.selected, vec![3]);
    }

    #[test]
    fn hiding_every_face_or_changing_while_drawing_is_refused() {
        let mut app = cube();
        hide(&mut app, &[0, 1, 2, 3, 4, 5]);
        assert_eq!(shown(&app), 12, "全部は隠さない");
        assert!(app.view3d.view.hide.islands.is_empty());
        assert!(app.message.contains("すべての面"), "{}", app.message);
        // 描いている最中は変えない
        let layer = app.session.selected_layer.unwrap();
        let settings = app.stroke_settings(false);
        let stroke = app.session.doc.begin_stroke(layer, &settings).unwrap();
        app.session.stroke = Some(stroke);
        hide(&mut app, &[1]);
        assert_eq!(shown(&app), 12);
        assert!(app.message.contains("描いて"), "{}", app.message);
        let stroke = app.session.stroke.take().unwrap();
        app.session.doc.cancel_stroke(stroke);
    }

    #[test]
    fn each_view_hides_its_own_faces_and_views_with_the_same_hiding_share_one_shape() {
        let mut app = cube();
        let second = ViewId(1);
        assert!(app.view3d.insert_view(second));
        // 最初のビューだけ隠す
        hide(&mut app, &[0]);
        assert_eq!(shown(&app), 10);
        let full = app.view3d.full_model().unwrap().clone();
        assert!(Arc::ptr_eq(app.view3d.view_model(second).unwrap(), &full));
        // ビュー 2 を今のビューにすると、ビュー 2 の形（何も隠さない）が出る
        assert!(app.view3d.activate(second));
        assert_eq!(shown(&app), 12);
        assert!(!app.view3d.is_face_hidden(0));
        assert_eq!(app.view3d.full_triangle(0), Some(0));
        // ビュー 2 でも同じ面を隠すと、ビュー 1 と同じ形（同じ世代）を共有する
        hide(&mut app, &[0]);
        let a = app.view3d.view_model(ViewId::FIRST).unwrap().clone();
        let b = app.view3d.view_model(second).unwrap().clone();
        assert!(Arc::ptr_eq(&a, &b), "同じ隠し方は 1 つの形");
        let (models, base) = app.view3d.frame_models();
        assert_eq!(models.len(), 1);
        assert!(Arc::ptr_eq(base.as_ref().unwrap(), &a));
        // 違う隠し方なら 2 つ。塗り広げの元は受けたままの形
        hide(&mut app, &[1]);
        let (models, base) = app.view3d.frame_models();
        assert_eq!(models.len(), 2);
        assert!(Arc::ptr_eq(base.as_ref().unwrap(), &full));
        // シェルフの目を閉じると、そのビューでは何も隠さない（隠し方は残る）
        apply(&mut app, HideAction::ToggleApplied(second));
        assert_eq!(shown(&app), 12);
        assert_eq!(app.view3d.view.hide.islands, vec![0, 1]);
        apply(&mut app, HideAction::ToggleApplied(second));
        assert_eq!(shown(&app), 8);
        // ビューを閉じると、その形も捨てる
        assert!(app.view3d.remove_view(second));
        assert_eq!(shown(&app), 10);
        assert_eq!(app.view3d.frame_models().0.len(), 1);
    }

    #[test]
    fn select_all_and_deselect_pick_the_islands_shown_in_the_view() {
        let mut app = cube();
        hide(&mut app, &[4]);
        apply(&mut app, HideAction::SelectAll);
        assert_eq!(app.view3d.view.hide.selected, vec![0, 1, 2, 3, 5]);
        apply(&mut app, HideAction::Deselect);
        assert!(app.view3d.view.hide.selected.is_empty());
        // 選ぶツール
        apply(&mut app, HideAction::Tool(HideTool::Rectangle));
        assert_eq!(app.view3d.hiding.tool, HideTool::Rectangle);
    }

    #[test]
    fn a_view_can_show_another_pose_and_views_with_the_same_pose_share_its_shape() {
        let mut app = AppState::new(64, 64);
        app.apply(Action::Pose(PoseAction::LoadFigure));
        let second = ViewId(1);
        let third = ViewId(2);
        app.view3d.insert_view(second);
        app.view3d.insert_view(third);
        // 今のポーズを曲げる
        let s = app.view3d.pose.session.as_ref().unwrap();
        let bone = s
            .rig
            .bones()
            .iter()
            .position(|b| b.name == "右前腕")
            .unwrap();
        let mut pose = s.pose().clone();
        pose.locals[bone].rotation = yolu_core::glam::Quat::from_rotation_z(1.0);
        crate::view3d::pose::set_pose(&mut app.view3d, pose).unwrap();
        let bent = app.view3d.full_model().unwrap().clone();
        // ビュー 2 と 3 は初期ポーズ: 同じ形を共有し、今のポーズの形とは位置が違う
        apply(&mut app, HideAction::ChoosePose(second, PoseChoice::Rest));
        apply(&mut app, HideAction::ChoosePose(third, PoseChoice::Rest));
        let b = app.view3d.view_model(second).unwrap().clone();
        let c = app.view3d.view_model(third).unwrap().clone();
        assert!(Arc::ptr_eq(&b, &c));
        assert!(!Arc::ptr_eq(&b, &bent));
        assert!(
            b.meshes
                .iter()
                .zip(&bent.meshes)
                .any(|(x, y)| x.positions != y.positions),
            "曲げた腕の位置が違う"
        );
        assert!(Arc::ptr_eq(app.view3d.model.as_ref().unwrap(), &bent));
        // 今のポーズを変えても、初期ポーズのビューは動かない
        crate::view3d::pose::reset(&mut app.view3d).unwrap();
        assert!(Arc::ptr_eq(app.view3d.view_model(second).unwrap(), &b));
        // 今のポーズへ戻す
        apply(
            &mut app,
            HideAction::ChoosePose(second, PoseChoice::Current),
        );
        assert!(Arc::ptr_eq(
            app.view3d.view_model(second).unwrap(),
            app.view3d.full_model().unwrap()
        ));
        // 別のスキンを読むと、ビューのポーズも隠し方も捨てる
        app.apply(Action::Pose(PoseAction::LoadFigure));
        assert_eq!(app.view3d.view_of(third).unwrap().pose, ViewPose::Current);
    }

    #[test]
    fn nothing_hidden_leaves_the_path_redraw_unchanged() {
        let mut app = cube();
        let g = app.view3d.full_model().unwrap().rest_geometry().clone();
        assert_eq!(
            app.path_hidden(&g),
            None,
            "何も隠していなければ除く面は無し"
        );
        hide(&mut app, &[1]);
        let hidden = app.path_hidden(&g).unwrap();
        assert_eq!(hidden.iter().filter(|h| **h).count(), 2);
        assert!(hidden[2] && hidden[3]);
        // 目を閉じたビューの隠し方は使わない
        apply(&mut app, HideAction::ToggleApplied(ViewId::FIRST));
        assert_eq!(app.path_hidden(&g), None);
    }
}
