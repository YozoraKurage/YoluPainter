//! Normal の設定（ハイト → ノーマル・強さ・端・ファイルの Y の向き）: 文書の操作として 1 回の Undo（スライダーのドラッグは 1 回）、
//! 保存と開き直し、読むだけのセット・描いている間の断り、チャンネルのパネルの節の操作と日英。
//! `headless_` で始まる試験は画面を描かず、Wine でも回る。
use crate::common;

use common::*;
use egui::{pos2, Rect};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use yolu_app::engine::{HeightEdgeMode, NormalSettings, NormalYDirection};
use yolu_app::lang::Lang;
use yolu_app::m2::{Edit, UiOp};
use yolu_app::state::{Action, AppState};
use yolu_app::{Tab, YoluApp};

struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new(name: &str) -> TempDir {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let p = std::env::temp_dir().join(format!(
            "yolu-normalset-{name}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn set(s: &mut AppState, settings: NormalSettings, coalesce: bool) {
    s.apply(Action::M2(Edit::NormalSettings { settings, coalesce }));
}

fn custom() -> NormalSettings {
    NormalSettings::new(true, 12.5, HeightEdgeMode::Wrap, NormalYDirection::DirectX).unwrap()
}

#[test]
fn headless_every_change_is_one_undo_and_a_drag_is_one_undo() {
    let mut s = AppState::new(64, 64);
    assert_eq!(s.doc.normal_settings(), NormalSettings::DEFAULT);
    let base = s.doc.undo_count();
    set(&mut s, NormalSettings::DEFAULT.with_derive(true), false);
    let next = s.doc.normal_settings().with_edges(HeightEdgeMode::Wrap);
    set(&mut s, next, false);
    let next = s
        .doc
        .normal_settings()
        .with_file_direction(NormalYDirection::DirectX);
    set(&mut s, next, false);
    assert_eq!(s.doc.undo_count(), base + 3);
    assert_eq!(s.message, "ノーマルのファイル: DirectX (Y−)");
    assert!(s.modified);
    // 同じ設定は何もしない（Undo の段を作らない）
    let settings = s.doc.normal_settings();
    set(&mut s, settings, false);
    assert_eq!(s.doc.undo_count(), base + 3);
    // 強さのドラッグ（まとめる）は離したとき 1 回
    for v in [5.0, 6.0, 7.5, 40.0] {
        let next = s.doc.normal_settings().with_strength(v).unwrap();
        set(&mut s, next, true);
    }
    assert_eq!(s.doc.normal_settings().strength(), 40.0);
    s.m2_end_drag();
    assert_eq!(s.doc.undo_count(), base + 4, "ドラッグ全体で 1 回");
    s.apply(Action::Undo);
    assert_eq!(
        s.doc.normal_settings().strength(),
        4.0,
        "ドラッグの前に戻る"
    );
    assert_eq!(
        s.doc.normal_settings().file_direction(),
        NormalYDirection::DirectX
    );
    s.apply(Action::Redo);
    assert_eq!(s.doc.normal_settings().strength(), 40.0);
    // 1 つずつ戻る
    s.apply(Action::Undo);
    s.apply(Action::Undo);
    assert_eq!(
        s.doc.normal_settings().file_direction(),
        NormalYDirection::OpenGL
    );
    assert_eq!(s.doc.normal_settings().edges(), HeightEdgeMode::Wrap);
    s.apply(Action::Undo);
    s.apply(Action::Undo);
    assert_eq!(s.doc.normal_settings(), NormalSettings::DEFAULT);
    // 英語のメッセージ
    s.apply(Action::M2Ui(UiOp::Language(Lang::En)));
    set(&mut s, NormalSettings::DEFAULT.with_derive(true), false);
    assert_eq!(s.message, "Height → Normal on.");
    set(&mut s, NormalSettings::DEFAULT, false);
    assert_eq!(s.message, "Height → Normal off.");
    s.apply(Action::M2Ui(UiOp::Language(Lang::Ja)));
    set(&mut s, NormalSettings::DEFAULT.with_derive(true), false);
    assert_eq!(s.message, "ハイト → ノーマルをオンにしました。");
}

#[test]
fn headless_the_settings_survive_save_and_open() {
    let dir = TempDir::new("save");
    let path = dir.0.join("normal.ylp");
    let mut s = AppState::new(64, 64);
    set(&mut s, custom(), false);
    s.apply(Action::SaveProjectAs(path.clone()));
    assert!(!s.modified, "{}", s.message);
    let mut again = AppState::new(64, 64);
    again.apply(Action::OpenProject(path.clone()));
    assert_eq!(again.doc.normal_settings(), custom(), "{}", again.message);
    assert_eq!(again.doc.normal_settings().strength(), 12.5);
    // 開き直した設定の変更も 1 回の Undo で戻り、元の設定に戻る
    set(&mut again, custom().with_strength(3.0).unwrap(), false);
    again.apply(Action::Undo);
    assert_eq!(again.doc.normal_settings(), custom());
    // 設定のないプロジェクトは既定
    let plain = dir.0.join("plain.ylp");
    let mut p = AppState::new(64, 64);
    p.apply(Action::SaveProjectAs(plain.clone()));
    let mut reopened = AppState::new(64, 64);
    reopened.apply(Action::OpenProject(plain));
    assert_eq!(reopened.doc.normal_settings(), NormalSettings::DEFAULT);
}

/// 2 つのマテリアル（0 に部品 A、1 に離れた部品 B）の Live Link のモデル（2 つのテクスチャセットができる）。
fn two_sets_model() -> yolu_protocol::Model {
    use yolu_protocol::{MaterialInfo, MaterialKey, MeshData, Model, Submesh, TextureProperty};
    let material = |name: &str| MaterialInfo {
        key: MaterialKey::Material {
            name: name.into(),
            asset: None,
        },
        shader: "Standard".into(),
        textures: vec![TextureProperty {
            name: "_MainTex".into(),
            width: 64,
            height: 64,
        }],
        routes: vec![],
    };
    let mesh = |name: &str, x: f32, material: u32| MeshData {
        key: name.into(),
        name: name.into(),
        skinned: false,
        positions: vec![[x, 0.0, 0.0], [x + 1.0, 0.0, 0.0], [x, 1.0, 0.0]],
        normals: vec![],
        uv0: vec![[0.1, 0.1], [0.4, 0.1], [0.1, 0.4]],
        submeshes: vec![Submesh {
            material,
            indices: vec![0, 2, 1],
        }],
    };
    Model {
        generation: 1,
        name: "二つ".into(),
        materials: vec![material("A"), material("B")],
        meshes: vec![mesh("a", 0.0, 0), mesh("b", 3.0, 1)],
    }
}

#[test]
fn headless_each_texture_set_keeps_its_own_normal_settings_through_save_and_open() {
    let dir = TempDir::new("two-sets");
    let path = dir.0.join("two.ylp");
    let mut s = AppState::new(64, 64);
    let (_, shape) = s.receive_link_model(&two_sets_model());
    shape.expect("3D に読める");
    assert_eq!(s.sets.len(), 2);
    let (first, second) = (s.sets.current_index(), 1 - s.sets.current_index());
    // 1 つ目のセットは custom()、2 つ目は別の設定（DirectX で強さが違う）。もう 1 つのセットの設定は変わらない
    let other =
        NormalSettings::new(true, 33.0, HeightEdgeMode::Clamp, NormalYDirection::OpenGL).unwrap();
    assert_ne!(other, custom());
    set(&mut s, custom(), false);
    assert_eq!(
        s.set_doc(second).normal_settings(),
        NormalSettings::DEFAULT,
        "ほかのセットは変わらない"
    );
    s.switch_set(second).unwrap();
    set(&mut s, other, false);
    assert_eq!(s.set_doc(first).normal_settings(), custom());
    s.apply(Action::SaveProjectAs(path.clone()));
    assert!(!s.modified, "{}", s.message);
    let mut again = AppState::new(64, 64);
    again.apply(Action::OpenProject(path.clone()));
    assert_eq!(again.sets.len(), 2, "{}", again.message);
    assert_eq!(again.set_doc(first).normal_settings(), custom());
    assert_eq!(again.set_doc(second).normal_settings(), other);
    // 開き直したあとも、セットごとに 1 回の Undo で戻る（もう片方は変わらない）
    set(&mut again, custom().with_strength(5.0).unwrap(), false);
    again.apply(Action::Undo);
    assert_eq!(again.doc.normal_settings(), other);
    assert_eq!(again.set_doc(first).normal_settings(), custom());
}

#[test]
fn headless_a_read_only_set_and_drawing_refuse_the_change_with_a_reason() {
    let mut s = AppState::new(64, 64);
    let index = s.sets.current_index();
    s.sets.get_mut(index).unwrap().read_only = Some("フィルターのあるレイヤーがあります".into());
    set(&mut s, custom(), false);
    assert_eq!(s.doc.normal_settings(), NormalSettings::DEFAULT);
    assert_eq!(
        s.message,
        "このテクスチャセットは読むだけです（フィルターのあるレイヤーがあります）。"
    );
    s.sets.get_mut(index).unwrap().read_only = None;
    let layer = s.selected_layer.unwrap();
    let stroke = s.begin_paint_stroke(layer, false).unwrap();
    set(&mut s, custom(), false);
    assert_eq!(s.doc.normal_settings(), NormalSettings::DEFAULT);
    assert!(s.message.contains("描いている間"), "{}", s.message);
    s.doc.cancel_stroke(stroke);
    set(&mut s, custom(), false);
    assert_eq!(s.doc.normal_settings(), custom());
}

// ───────── チャンネルのパネルの Normal の節 ─────────

fn open_channels(h: &mut Harness<'static, YoluApp>) {
    click_tab(h, Tab::Channels);
    h.run();
    // 「ノーマル」の節は初めは閉じている（見出しを押して開く）
    assert_eq!(h.state().state.ui.sections.get("normal"), Some(&false));
    let header = lowest(h, "ノーマル");
    click(h, header.center());
    assert_eq!(h.state().state.ui.sections.get("normal"), Some(&true));
}

/// 右上の組（チャンネル）の中の、同じ名前の部品のうち一番下のもの（チャンネルの行と同じ名前の見出しなど）。
fn lowest(h: &Harness<'_, YoluApp>, label: &str) -> Rect {
    let body = top_right_body(h);
    h.get_all_by_label(label)
        .map(|n| n.rect())
        .filter(|r| body.contains(r.center()))
        .max_by(|a, b| a.top().total_cmp(&b.top()))
        .unwrap_or_else(|| panic!("{label}"))
}

#[test]
fn the_channels_tab_edits_the_normal_settings_with_toggle_slider_and_dropdowns() {
    let mut h = app(1280.0, 1600.0, 128);
    open_channels(&mut h);
    let base = h.state().state.doc.undo_count();
    // 初めは Height → Normal がオフ（強さと端は使えない）
    assert!(!h.state().state.doc.normal_settings().derive_from_height());
    h.snapshot("channels_normal_off");
    let toggle = lowest(&h, "ハイト → ノーマル");
    click(&mut h, toggle.center());
    assert!(h.state().state.doc.normal_settings().derive_from_height());
    assert_eq!(h.state().state.doc.undo_count(), base + 1);
    // 強さのドラッグは 1 回の Undo
    let slider = lowest(&h, "強さ");
    drag(
        &mut h,
        &[
            pos2(slider.center().x, slider.center().y),
            pos2(slider.right() - 2.0, slider.center().y),
            pos2(slider.left() + slider.width() * 0.75, slider.center().y),
        ],
    );
    let strength = h.state().state.doc.normal_settings().strength();
    assert!((60.0..140.0).contains(&strength), "{strength}");
    assert_eq!(
        h.state().state.doc.undo_count(),
        base + 2,
        "ドラッグ全体で 1 回"
    );
    h.snapshot("channels_normal_on");
    h.state_mut().state.apply(Action::Undo);
    assert_eq!(h.state().state.doc.normal_settings().strength(), 4.0);
    // 端のドロップダウン
    h.run();
    let edges = lowest(&h, "クランプ");
    click(&mut h, edges.center());
    let wrap = popup_item(&h, "ラップ（タイル）").center();
    click(&mut h, wrap);
    assert_eq!(
        h.state().state.doc.normal_settings().edges(),
        HeightEdgeMode::Wrap
    );
    // ファイルの Y のドロップダウン
    let direction = lowest(&h, "OpenGL (Y+)");
    click(&mut h, direction.center());
    let directx = popup_item(&h, "DirectX (Y−)").center();
    click(&mut h, directx);
    assert_eq!(
        h.state().state.doc.normal_settings().file_direction(),
        NormalYDirection::DirectX
    );
    // 戻す
    h.state_mut().state.apply(Action::Undo);
    h.state_mut().state.apply(Action::Undo);
    assert_eq!(
        h.state().state.doc.normal_settings().edges(),
        HeightEdgeMode::Clamp
    );
    h.state_mut()
        .state
        .apply(Action::M2Ui(UiOp::Language(Lang::En)));
    h.run();
    h.snapshot("channels_normal_english");
    let _ = lowest(&h, "Height → Normal");
    let _ = lowest(&h, "Strength");
    let _ = lowest(&h, "Clamp");
    let _ = lowest(&h, "OpenGL (Y+)");
}

#[test]
fn strength_and_edges_wait_for_height_to_normal_and_a_read_only_set_disables_everything() {
    let mut h = app(1280.0, 1600.0, 128);
    open_channels(&mut h);
    // オフのあいだは強さのスライダーを触っても何も変わらない
    let slider = lowest(&h, "強さ");
    drag(
        &mut h,
        &[
            pos2(slider.center().x, slider.center().y),
            pos2(slider.right() - 2.0, slider.center().y),
        ],
    );
    assert_eq!(
        h.state().state.doc.normal_settings(),
        NormalSettings::DEFAULT
    );
    let edges = lowest(&h, "クランプ");
    click(&mut h, edges.center());
    assert!(h.state().state.popup.is_none(), "オフのあいだ端は開かない");
    // 読むだけのセットは、何も開かない・変わらない
    {
        let s = &mut h.state_mut().state;
        let index = s.sets.current_index();
        s.sets.get_mut(index).unwrap().read_only = Some("理由".into());
    }
    h.run();
    let toggle = lowest(&h, "ハイト → ノーマル");
    click(&mut h, toggle.center());
    assert!(!h.state().state.doc.normal_settings().derive_from_height());
    let direction = lowest(&h, "OpenGL (Y+)");
    click(&mut h, direction.center());
    assert!(h.state().state.popup.is_none());
}

#[test]
fn the_channel_list_and_the_normal_section_scroll_together_in_a_short_panel() {
    let mut h = app(1280.0, 500.0, 128);
    click_tab(&mut h, Tab::Channels);
    // 見出しが見える所に無いほど短いので、開いた状態を直に入れる
    h.state_mut().state.ui.sections.insert("normal", true);
    h.run();
    let content = h.state().state.m2.channels_content;
    assert!(content > 6.0 * 28.0 + 60.0, "一覧の下に節が続く: {content}");
    // ホイールで動かすと、下の節が見える
    let at = h
        .state()
        .tab_rects
        .get(&Tab::Channels)
        .unwrap()
        .center_bottom();
    let before = h.state().state.m2.channel_scroll;
    h.event(egui::Event::PointerMoved(pos2(at.x, at.y + 60.0)));
    h.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, -200.0),
        phase: egui::TouchPhase::Move,
        modifiers: egui::Modifiers::NONE,
    });
    h.run();
    assert!(h.state().state.m2.channel_scroll > before, "スクロールした");
    let max = h.state().state.m2.channel_scroll;
    // 上限を越えて動かない
    h.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, -2000.0),
        phase: egui::TouchPhase::Move,
        modifiers: egui::Modifiers::NONE,
    });
    h.run();
    assert!(h.state().state.m2.channel_scroll >= max);
    assert!(h.state().state.m2.channel_scroll <= h.state().state.m2.channels_content);
}
