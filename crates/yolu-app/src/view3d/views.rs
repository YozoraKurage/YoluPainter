//! 3D ビューを増やす: ビューの並び（今のビューと、置いてあるほかのビュー）、今のビューの替え方、ビューの足し引き、表示の切り替えの当て先。
//!
//! 状態はビューごとの物（`ViewState`: カメラ・正投影・表示の設定・表示域・入力の途中）と、全部のビューで共通の物（`View3dState` の
//! ほかの欄: モデル・描くセット・塗りの設定・ポーズ）に分ける。入力とキーの視点の操作は「今のビュー」（`View3dState::view`）が受ける。
//! ポインタが別のビューに入ったら、今のビューで何も始まっていないときだけ、そのビューを今のビューにする（`may_activate`）。ストロークの
//! 途中で別のビューへ出ても替えないので、ストロークは始めたビューのカメラで最後まで描く。今のビューでないビューは、描くときだけ一時的に
//! 今のビューの場所へ入れ替えて描き（`with_view`）、入力は渡さない。
//!
//! ビューごとに持たせたい物（ポーズのプリセット・面を隠すプリセット・ビューごとのモデルの形など）は `ViewState` へ足す。今は全部の
//! ビューが同じモデル（`View3dState::model`）を見る。

use yolu_core::mesh_maps::MeshMapKind;

use super::display::{Op, Shading};
use super::render::MeshMapSource;
use super::{View3dState, ViewId, ViewState};
use crate::state::AppState;

/// ビューの数の上限（最初のビューを含む。描き先は 1 つのビューごとに持つので、GPU のメモリを無制限に使わない）。
pub const MAX_VIEWS: usize = 8;

impl View3dState {
    /// 今のビューの見分け。
    pub fn active_view(&self) -> ViewId {
        self.view.id
    }

    /// ビューの数（最初のビューを含む）。
    pub fn view_count(&self) -> usize {
        1 + self.parked.len()
    }

    /// 全部のビュー（今のビューが先）。
    pub fn views(&self) -> impl Iterator<Item = &ViewState> {
        std::iter::once(&self.view).chain(self.parked.iter())
    }

    /// 全部のビュー（書き換える。今のビューが先）。
    pub fn views_mut(&mut self) -> impl Iterator<Item = &mut ViewState> {
        std::iter::once(&mut self.view).chain(self.parked.iter_mut())
    }

    /// 全部のビューの見分け（小さい順）。
    pub fn view_ids(&self) -> Vec<ViewId> {
        let mut ids: Vec<ViewId> = self.views().map(|v| v.id).collect();
        ids.sort();
        ids
    }

    /// このビューがあるか。
    pub fn has_view(&self, id: ViewId) -> bool {
        self.views().any(|v| v.id == id)
    }

    /// このビューの状態（今のビューでも、置いてあるビューでも）。
    pub fn view_of(&self, id: ViewId) -> Option<&ViewState> {
        self.views().find(|v| v.id == id)
    }

    pub fn view_of_mut(&mut self, id: ViewId) -> Option<&mut ViewState> {
        if self.view.id == id {
            return Some(&mut self.view);
        }
        self.parked.iter_mut().find(|v| v.id == id)
    }

    /// 次に足すビューの見分け（使っていない、いちばん小さい番号。上限なら None）。
    pub fn next_view_id(&self) -> Option<ViewId> {
        if self.view_count() >= MAX_VIEWS {
            return None;
        }
        (1..MAX_VIEWS as u32)
            .map(ViewId)
            .find(|id| !self.has_view(*id))
    }

    /// ビューを足す（カメラと表示の設定は今のビューの写し。設定のパネルは閉じて始める）。足したビューは今のビューにしない。
    pub fn add_view(&mut self) -> Option<ViewId> {
        let id = self.next_view_id()?;
        self.insert_view(id);
        Some(id)
    }

    /// この見分けのビューを足す（保存した並びから戻すとき。もうある・上限を超える番号なら何もしない）。
    pub fn insert_view(&mut self, id: ViewId) -> bool {
        if self.has_view(id) || id.0 as usize >= MAX_VIEWS {
            return false;
        }
        let mut display = self.view.display;
        display.settings_open = false;
        display.post_dragging = false;
        display.drawn_samples = 0;
        self.parked.push(ViewState {
            id,
            camera: self.view.camera,
            auto_orthographic: self.view.auto_orthographic,
            display,
            ..ViewState::default()
        });
        true
    }

    /// ビューを除く（最初のビューは除かない）。今のビューなら、先に最初のビューを今のビューにする。
    pub fn remove_view(&mut self, id: ViewId) -> bool {
        if id == ViewId::FIRST || !self.has_view(id) {
            return false;
        }
        if self.view.id == id {
            self.activate(ViewId::FIRST);
        }
        self.parked.retain(|v| v.id != id);
        true
    }

    /// このビューを今のビューにする（無いビューなら false）。入れ替えるだけで、入力の途中は見ない（見るのは `may_activate`）。
    pub fn activate(&mut self, id: ViewId) -> bool {
        if self.view.id == id {
            return true;
        }
        let Some(i) = self.parked.iter().position(|v| v.id == id) else {
            return false;
        };
        std::mem::swap(&mut self.view, &mut self.parked[i]);
        true
    }

    /// 表示の切り替えを当てる: 表示の種類と設定のパネルの開閉はビューごと（`ShadingOf` はそのビュー、ほかは今のビュー）、光・環境・
    /// トーンマッピング・仕上げは全部のビューへ（同じ値を持つ）。設定のパネルは 1 つのビューにだけ開く。
    pub fn apply_display(&mut self, op: Op) {
        match op {
            Op::Shading(_) | Op::CloseSettings => self.view.display.apply(op),
            Op::ShadingOf(id, shading) => {
                if let Some(view) = self.view_of_mut(id) {
                    view.display.apply(Op::Shading(shading));
                }
            }
            Op::ToggleSettings => {
                self.view.display.apply(op);
                if self.view.display.settings_open {
                    for view in &mut self.parked {
                        view.display.apply(Op::CloseSettings);
                    }
                }
            }
            _ => {
                self.view.display.apply(op);
                for view in &mut self.parked {
                    view.display.apply(op);
                }
            }
        }
    }

    /// どれかのビューがこの表示の種類を見ているか。
    pub fn any_view_shows(&self, pred: impl Fn(Shading) -> bool) -> bool {
        self.views().any(|v| pred(v.display.shading))
    }

    /// 今のビューで、入力（描く・回す・パン・拡縮・ドラッグの途中の形・押し）が何も始まっていないか。
    pub fn idle_input(&self) -> bool {
        let i = &self.view.input;
        i.stroke.is_none()
            && !i.released
            && i.nav.is_none()
            && i.navigation.is_none()
            && i.zoom.is_none()
            && i.pen_press.is_none()
            && i.mouse_press.is_none()
            && i.clone_press.is_none()
            && i.eyedrop.is_none()
            && i.draft.is_none()
    }
}

/// 今のビューで何かの入力が進んでいるか（描いている・回している・ポーズのギズモ・G/R/S・選択の形・塗りの置き場のドラッグの途中）。
/// 進んでいるあいだは、今のビューを替えない・ビューを閉じない・足さない。
pub fn busy(app: &AppState) -> bool {
    !app.view3d.idle_input()
        || app.holds_panel_look()
        || crate::objects::transforming(app)
        || app.sel.view3d.drag.is_some()
        || !app.sel.view3d.polygon.is_empty()
}

/// 今のビューを替えてよいか: 何も進んでいない（`busy` でない）、ポップアップ・メニューが開いていない。ボタンの押しは見ない
/// （`may_activate` が見る）。
pub fn may_switch(app: &AppState) -> bool {
    !busy(app) && app.ui.popup.is_none()
}

/// ポインタがこのビュー（`content` は表示域）の上にあり、今のビューを替えてよければ、このビューを今のビューにする。今のビューになったら true。
/// ボタンを押したまま入ってきたポインタ（別の所で始めたドラッグ）では替えない。このフレームにこのビューの上で押したなら替える（ポインタが
/// 乗ったフレームに押した: 別ウィンドウへ移ってすぐの押し・ペンが近づいてすぐ触れた）。
pub fn may_activate(
    app: &mut AppState,
    ctx: &egui::Context,
    id: ViewId,
    content: egui::Rect,
) -> bool {
    if app.view3d.active_view() == id {
        return true;
    }
    let (over, down, pressed_here) = ctx.input(|i| {
        (
            i.pointer.hover_pos().is_some_and(|p| content.contains(p)),
            i.pointer.any_down(),
            i.pointer.any_pressed()
                && i.pointer
                    .press_origin()
                    .is_some_and(|p| content.contains(p)),
        )
    });
    if !over || (down && !pressed_here) || !may_switch(app) || !app.view3d.has_view(id) {
        return false;
    }
    // 前のビューで開いていた設定のパネルは閉じる（パネルは今のビューにだけ出す）
    app.view3d.view.display.apply(Op::CloseSettings);
    app.view3d.activate(id)
}

/// このビューを一時的に今のビューの場所へ入れ替えて `f` を呼ぶ（今のビューでないビューを描くとき。`f` の中ではビューを足し引き・
/// 今のビューを替えない）。無いビューなら呼ばない。
pub fn with_view<R>(
    app: &mut AppState,
    id: ViewId,
    f: impl FnOnce(&mut AppState) -> R,
) -> Option<R> {
    let active = app.view3d.active_view();
    if active == id {
        return Some(f(app));
    }
    if !app.view3d.activate(id) {
        return None;
    }
    let out = f(app);
    app.view3d.activate(active);
    Some(out)
}

/// このフレームに、どれかのビューが見ている焼いたメッシュマップの種類と、「チャンネルだけ」で見ているユーザーチャンネルの名前
/// （ビューの番号の順。名前は今のセットの文書のもの。`render::View3dRenderer::begin_frame` へ渡す）。
pub fn frame_shows(app: &AppState) -> (Vec<MeshMapKind>, Vec<String>) {
    let mut views: Vec<&ViewState> = app.view3d.views().collect();
    views.sort_by_key(|v| v.id);
    let mut maps = Vec::new();
    let mut users = Vec::new();
    for v in views {
        match v.display.shading {
            Shading::MeshMap(kind) if !maps.contains(&kind) => maps.push(kind),
            Shading::Channel(c) if !c.is_standard() => {
                if let Some(info) = app.session.doc.channel_info(c) {
                    if !users.contains(&info.name) {
                        users.push(info.name.clone());
                    }
                }
            }
            _ => {}
        }
    }
    (maps, users)
}

/// この種類の焼いたメッシュマップを持つセットの、3D に見せる絵（今のセットが先。目を閉じたセットは除く。マテリアルに付いていない
/// セットは面が無いので除く）。
pub fn mesh_map_sources(app: &AppState, kind: MeshMapKind) -> Vec<MeshMapSource<'_>> {
    let sets = &app.session.sets;
    let current = sets.current_index();
    let mut out = Vec::new();
    for (i, set) in sets.iter().enumerate() {
        let material = if i == current {
            app.view3d.material
        } else if set.visible {
            match set.bound {
                Some(m) => m as i32,
                None => continue,
            }
        } else {
            continue;
        };
        let Some(map) = set.mesh_maps.get(kind) else {
            continue;
        };
        let source = MeshMapSource {
            key: ((set.uid as u64) << 40) ^ ((kind as i32 as u64) << 32) ^ set.mesh_maps.revision(),
            material,
            map,
        };
        if i == current {
            out.insert(0, source);
        } else {
            out.push(source);
        }
    }
    out
}

/// どれかのセットがこの種類の焼いたメッシュマップを持つか（表示のメニューで選べるか）。
pub fn mesh_map_available(app: &AppState, kind: MeshMapKind) -> bool {
    app.session
        .sets
        .iter()
        .any(|s| s.mesh_maps.get(kind).is_some())
}

/// 3D ビューのタブの名前: ビューが 1 つなら「3D ビュー」、2 つ以上なら名前と表示の種類（「3D ビュー・マテリアル」「3D ビュー 2・ラフネス」。
/// 重ねて隠れているタブ・別ウィンドウのタブを見分ける）。
pub fn tab_title(app: &AppState, id: ViewId) -> String {
    let lang = app.lang;
    let name = if id == ViewId::FIRST {
        lang.t("app-3d-view").to_owned()
    } else {
        lang.f("app-3d-view-numbered", &[("number", &id.number())])
    };
    if app.view3d.view_count() < 2 {
        return name;
    }
    let Some(view) = app.view3d.view_of(id) else {
        return name;
    };
    let shading = super::display::shading_short_label(app, view.display.shading);
    lang.f(
        "app-3d-view-titled",
        &[("name", &name), ("shading", &shading)],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Action;
    use yolu_core::{Channel, ChannelInfo, ChannelKind, ColorSpace, Rgba8};

    fn user(app: &mut AppState, name: &str) -> Channel {
        app.session
            .doc
            .add_channel(ChannelInfo {
                name: name.into(),
                kind: ChannelKind::Scalar,
                color_space: ColorSpace::Linear,
                default: Rgba8::new(0, 0, 0, 255),
            })
            .unwrap()
    }

    fn show(app: &mut AppState, id: ViewId, c: Channel) {
        app.apply(Action::View3d(Op::ShadingOf(id, Shading::Channel(c))));
    }

    #[test]
    fn views_are_added_up_to_the_limit_and_the_first_one_stays() {
        let mut v = View3dState::default();
        assert_eq!(v.view_ids(), vec![ViewId::FIRST]);
        let added: Vec<ViewId> = std::iter::from_fn(|| v.add_view()).collect();
        assert_eq!(added.len(), MAX_VIEWS - 1);
        assert_eq!(v.view_count(), MAX_VIEWS);
        // 閉じた番号は、次に足すビューが使う
        assert!(v.remove_view(ViewId(3)));
        assert_eq!(v.add_view(), Some(ViewId(3)));
        // 最初のビューは除かない。今のビューを除くと、最初のビューが今のビュー
        assert!(!v.remove_view(ViewId::FIRST));
        assert!(v.activate(ViewId(2)));
        assert!(v.remove_view(ViewId(2)));
        assert_eq!(v.active_view(), ViewId::FIRST);
        // 足したビューは今のビューのカメラと表示から始まる（設定のパネルは閉じて）
        v.view.camera.yaw = 77.0;
        v.view.display.settings_open = true;
        v.apply_display(Op::Shading(Shading::Neutral));
        assert!(v.remove_view(ViewId(5)));
        let id = v.add_view().unwrap();
        let added = v.view_of(id).unwrap();
        assert_eq!(added.camera.yaw, 77.0);
        assert_eq!(added.display.shading, Shading::Neutral);
        assert!(!added.display.settings_open);
    }

    #[test]
    fn showing_too_many_user_channels_at_once_is_refused_with_a_reason() {
        let mut app = AppState::new(64, 64);
        let channels: Vec<Channel> = (0..5).map(|i| user(&mut app, &format!("U{i}"))).collect();
        for k in 1..5 {
            app.view3d.insert_view(ViewId(k));
        }
        for (k, c) in channels.iter().take(4).enumerate() {
            show(&mut app, ViewId(k as u32), *c);
            assert_eq!(
                app.view3d
                    .view_of(ViewId(k as u32))
                    .unwrap()
                    .display
                    .shading,
                Shading::Channel(*c)
            );
        }
        // 5 つ目の違うユーザーチャンネルは断る（表示は替えない）
        show(&mut app, ViewId(4), channels[4]);
        assert_eq!(
            app.view3d.view_of(ViewId(4)).unwrap().display.shading,
            Shading::Material
        );
        assert!(app.message.contains("4"), "{}", app.message);
        // 同じチャンネルなら、もう見せているので出せる。見せている 1 つを替えれば、空いた分で出せる
        show(&mut app, ViewId(4), channels[0]);
        assert_eq!(
            app.view3d.view_of(ViewId(4)).unwrap().display.shading,
            Shading::Channel(channels[0])
        );
        show(&mut app, ViewId(3), channels[4]);
        assert_eq!(
            app.view3d.view_of(ViewId(3)).unwrap().display.shading,
            Shading::Channel(channels[4])
        );
    }

    #[test]
    fn a_user_channel_over_the_3d_budget_is_refused() {
        // 4096² のユーザーチャンネルは、3 つまでなら元の大きさ（ミップ込み）で 3D の予算（256 MiB）に入り、4 つ目は入らない（約 341 MiB）
        let mut app = AppState::new_in(4096, 4096, crate::lang::Lang::Ja);
        let channels: Vec<Channel> = ["A", "B", "C", "D"]
            .iter()
            .map(|n| user(&mut app, n))
            .collect();
        for k in 1..4 {
            app.view3d.insert_view(ViewId(k));
        }
        for (k, c) in channels.iter().take(3).enumerate() {
            show(&mut app, ViewId(k as u32), *c);
            assert_eq!(
                app.view3d
                    .view_of(ViewId(k as u32))
                    .unwrap()
                    .display
                    .shading,
                Shading::Channel(*c)
            );
        }
        show(&mut app, ViewId(3), channels[3]);
        assert_eq!(
            app.view3d.view_of(ViewId(3)).unwrap().display.shading,
            Shading::Material,
            "予算を超えるので断る"
        );
        assert!(app.message.contains("「D」"), "{}", app.message);
        assert!(app.message.contains("予算"), "{}", app.message);
    }
}
