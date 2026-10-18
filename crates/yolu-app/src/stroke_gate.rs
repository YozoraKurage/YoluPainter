//! 描き始めの確かめ: 文書を変える描き始め（ブラシ・消しゴム・図形・範囲のツール・選択範囲の塗り・グラデーション・パス）が通る確かめと、その断りの文。
//! 1 つの確かめに 1 つの関数を置き、2D のキャンバスと 3D ビュー・ツールの間で、同じ状態なら同じ文で断る（断りは呼ぶ側が `refuse` で知らせる）。
//!
//! 確かめの順（2D と 3D で同じ。3D だけの前提が先）:
//! 1. 3D だけ: モデル（無ければ何も言わない）→ 今のテクスチャセットがモデルに付いているか（`AppState::region_missing_reason`）
//! 2. 読むだけのテクスチャセット（[`read_only`]）
//! 3. 3D だけ: 押した面がほかのテクスチャセットのものか（[`other_set_face`]）
//! 4. ブラシ・消しゴムだけ: 選択ペン・クイックマスクへ回す（描くレイヤーは要らない）
//! 5. 描くレイヤー（[`selected`]。選んでいない・選んだレイヤーが文書に無いときは [`no_layer`]）→ 描けないレイヤー（[`blocker`]）
//! 6. ブラシだけ: クローンの元（3D）→ 対称と指先・クローン（[`symmetry_effect`]）→ ステンシル → core（失敗は [`cannot_paint`]）
//!
//! [`paint`] は 2 と 5（図形・範囲のツール・選択範囲の塗り）、[`layer`] は 5 だけ。

use crate::engine::{BrushEffect, LayerId, LayerKind};
use crate::lang::Lang;
use crate::rulers::Place;
use crate::state::AppState;

/// 読むだけのテクスチャセットなら、その断りの文（読むだけになった理由つき）。
pub(crate) fn read_only(app: &AppState) -> Result<(), String> {
    match app.read_only_reason() {
        Some(reason) => Err(crate::lang::refusals::read_only_set(app.lang, reason)),
        None => Ok(()),
    }
}

/// 選んでいるレイヤー。選んでいない・選んだレイヤーが文書に無い（古い選び）ときは「描くレイヤーがありません」。
pub(crate) fn selected(app: &AppState) -> Result<LayerId, String> {
    app.session
        .selected_layer
        .filter(|id| app.session.doc.layer(*id).is_some())
        .ok_or_else(|| no_layer(app.lang))
}

/// 選んでいるレイヤーに描けないときの短い理由（ラスターでない・描くチャンネルが無効）。選んでいない・文書に無いときは None（[`selected`] が断る）。
pub(crate) fn blocker(app: &AppState) -> Option<String> {
    let id = app.session.selected_layer?;
    let layer = app.session.doc.layer(id)?;
    let lang = app.lang;
    if app.m2.edit_mask {
        return None;
    }
    if layer.kind() != LayerKind::Raster {
        return Some(lang.with_reason(
            lang.t("stroke_gate-cannot-paint-on-this-layer"),
            crate::m2::layer_kind_label(lang, layer.kind()),
        ));
    }
    let channel = app.m2.paint_channel;
    // マテリアルで塗るなら、無効のチャンネルは core が有効にする
    if !app.mat.enabled && layer.surface(channel).is_some() && !layer.is_channel_enabled(channel) {
        let channel = crate::m2::channel_name(lang, &app.session.doc, channel);
        return Some(lang.f(
            "stroke_gate-the-channel-is-off-on",
            &[("channel", &channel)],
        ));
    }
    None
}

/// 描くレイヤー（[`selected`] → [`blocker`]）。
pub(crate) fn layer(app: &AppState) -> Result<LayerId, String> {
    let id = selected(app)?;
    match blocker(app) {
        Some(reason) => Err(reason),
        None => Ok(id),
    }
}

/// 塗る・消すの前の確かめ（[`read_only`] → [`layer`]）。返すのは描くレイヤー。
pub(crate) fn paint(app: &AppState) -> Result<LayerId, String> {
    read_only(app)?;
    layer(app)
}

/// 「描くレイヤーがありません」。
pub(crate) fn no_layer(lang: Lang) -> String {
    lang.t("stroke_gate-no-layer-to-paint-on").into()
}

/// 押した面がほかのテクスチャセット（`name`）のものだという断り。
pub(crate) fn other_set_face(lang: Lang, name: &str) -> String {
    let name = lang.quote(name);
    lang.f(
        "stroke_gate-this-face-belongs-to-another",
        &[("name", &name)],
    )
}

/// 対称が効いているときに、指先・クローンで描き始めない（写しごとに読み元と動きが要るので、core も断る）。`place` は描くビュー、`drawing` は
/// 描くレイヤー、`erase` は消す側か（消す側は効果を使わず色を塗る側なので断らない）。
///
/// 2D のキャンバスで効く対称は、2D の対称定規と、今のテクスチャセットの面のモデルがあるときの 3D の対称定規（core の `Brush::validate` と同じ条件）。
/// 3D ビューで効く対称は、3D の対称定規と 2D の対称定規（UV の平面）。
pub(crate) fn symmetry_effect(
    app: &AppState,
    place: Place,
    drawing: LayerId,
    erase: bool,
) -> Result<(), String> {
    let effect = matches!(
        app.m2.brush.effect,
        BrushEffect::Smudge { .. } | BrushEffect::Clone { .. }
    );
    if erase || !effect {
        return Ok(());
    }
    let active = app.rulers_active_for(place, Some(drawing));
    let canvas = active.canvas_symmetry.is_some_and(|s| s.enabled());
    let surface = active.surface_symmetry.is_some_and(|s| s.enabled())
        && (place == Place::View3d || app.region_model().is_some());
    if canvas || surface {
        return Err(app.lang.t("stroke_gate-smudge-and-clone-do-not").into());
    }
    Ok(())
}

/// 描き始めが断られた・失敗したときの文（「描けません（理由）」）。
pub(crate) fn cannot_paint(lang: Lang, reason: &str) -> String {
    lang.with_reason(lang.t("stroke_gate-cannot-paint"), reason)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Channel;
    use crate::m2::Edit;
    use crate::state::Action;

    fn app() -> AppState {
        AppState::new(64, 64)
    }

    fn set_read_only(s: &mut AppState) {
        let i = s.session.sets.current_index();
        s.session.sets.get_mut(i).unwrap().read_only = Some("テスト".into());
    }

    /// 読むだけとレイヤー無しが同時なら、読むだけを先に断る。
    #[test]
    fn read_only_comes_before_the_layer() {
        let mut s = app();
        set_read_only(&mut s);
        s.session.selected_layer = None;
        let read_only_text = crate::lang::refusals::read_only_set(s.lang, "テスト");
        assert_eq!(paint(&s), Err(read_only_text.clone()));
        assert_eq!(read_only(&s), Err(read_only_text));
        // 読むだけでなければ、レイヤー無し
        let i = s.session.sets.current_index();
        s.session.sets.get_mut(i).unwrap().read_only = None;
        assert_eq!(paint(&s), Err(no_layer(s.lang)));
    }

    /// 読むだけと描けないレイヤー（グループ）が同時なら、読むだけ。レイヤー無しと描けないレイヤーなら、レイヤー無し。
    #[test]
    fn the_layer_comes_before_the_blocker() {
        let mut s = app();
        s.apply(Action::M2(Edit::NewGroup));
        assert!(blocker(&s).is_some());
        set_read_only(&mut s);
        assert_eq!(
            paint(&s),
            Err(crate::lang::refusals::read_only_set(s.lang, "テスト"))
        );
        let i = s.session.sets.current_index();
        s.session.sets.get_mut(i).unwrap().read_only = None;
        assert_eq!(paint(&s), Err(blocker(&s).unwrap()));
        assert_eq!(layer(&s), Err(blocker(&s).unwrap()));
    }

    /// 選んだレイヤーが文書に無い（古い選び）ときも「描くレイヤーがありません」。描けないレイヤーの理由は出さない。
    #[test]
    fn a_stale_selection_is_no_layer() {
        let mut s = app();
        let gone = s.session.doc.add_layer("消す").unwrap();
        s.session.doc.remove_layer(gone).unwrap();
        s.session.selected_layer = Some(gone);
        assert_eq!(selected(&s), Err(no_layer(s.lang)));
        assert_eq!(blocker(&s), None);
        assert_eq!(layer(&s), Err(no_layer(s.lang)));
        assert_eq!(paint(&s), Err(no_layer(s.lang)));
        assert_eq!(no_layer(s.lang), "描くレイヤーがありません。");
        assert_eq!(no_layer(Lang::En), "No layer to paint on.");
    }

    /// 描けないレイヤー: ラスターでない（種類の名前つき）・描くチャンネルが無効。
    #[test]
    fn blocked_painting_names_the_reason() {
        let mut s = app();
        s.apply(Action::M2(Edit::NewGroup));
        assert!(blocker(&s).unwrap().contains("グループ"));
        s.lang = Lang::En;
        assert!(blocker(&s).unwrap().contains("Group"));
        s.apply(Action::NewLayer);
        let id = s.session.selected_layer.unwrap();
        assert_eq!(blocker(&s), None);
        assert_eq!(paint(&s), Ok(id));
        s.apply(Action::M2(Edit::ChannelEnabled {
            id,
            channel: Channel::Color,
            enabled: false,
        }));
        assert!(blocker(&s).is_some());
        assert_eq!(paint(&s), Err(blocker(&s).unwrap()));
        assert!(s.begin_paint_stroke(id, false).is_err());
        // マスクを描くときは、レイヤーのチャンネルを見ない
        s.m2.edit_mask = true;
        assert_eq!(blocker(&s), None);
    }

    /// 2D の縦の対称定規で、指先・クローンは 2D でも 3D でも断る。消す側・ふつうのブラシ・特殊定規にスナップしないときは断らない。
    #[test]
    fn smudge_and_clone_refuse_symmetry_but_the_eraser_and_paint_do_not() {
        use yolu_core::glam::DVec2;
        use yolu_core::{Ruler, RulerId, RulerKind};
        let mut s = app();
        let layer = s.session.selected_layer.unwrap();
        s.rulers.on_layer = true;
        let mut ruler = Ruler::canvas(
            RulerId(1),
            RulerKind::Symmetry,
            DVec2::new(32.0, 0.0),
            DVec2::new(32.0, 1.0),
        );
        ruler.lines = 2;
        ruler.line_symmetry = true;
        s.ruler_create(ruler).unwrap();
        let refused = Err(s.lang.t("stroke_gate-smudge-and-clone-do-not").to_owned());
        for effect in [
            BrushEffect::Smudge { strength: 1.0 },
            BrushEffect::Clone {
                offset: Default::default(),
            },
        ] {
            s.m2.brush.effect = effect;
            for place in [Place::Canvas, Place::View3d] {
                assert_eq!(
                    symmetry_effect(&s, place, layer, false),
                    refused,
                    "{place:?}"
                );
                assert_eq!(
                    symmetry_effect(&s, place, layer, true),
                    Ok(()),
                    "{place:?}: 消す側"
                );
            }
        }
        s.m2.brush.effect = BrushEffect::Paint;
        for place in [Place::Canvas, Place::View3d] {
            assert_eq!(
                symmetry_effect(&s, place, layer, false),
                Ok(()),
                "{place:?}"
            );
        }
        // 特殊定規にスナップしなければ、対称は効かないので指先も断らない
        s.m2.brush.effect = BrushEffect::Smudge { strength: 1.0 };
        s.rulers.snap_special = false;
        for place in [Place::Canvas, Place::View3d] {
            assert_eq!(
                symmetry_effect(&s, place, layer, false),
                Ok(()),
                "{place:?}"
            );
        }
    }

    /// 3D の対称定規は、3D ビューでは効き、2D のキャンバスではモデルが無いと効かない（core も 3D の写しを作らないので描ける）。
    #[test]
    fn the_3d_symmetry_refuses_smudge_in_2d_only_with_a_model() {
        use yolu_core::glam::DVec3;
        use yolu_core::{Ruler, RulerId, RulerKind};
        let mut s = app();
        let layer = s.session.selected_layer.unwrap();
        s.rulers.on_layer = true;
        let mut ruler = Ruler::model(
            RulerId(1),
            RulerKind::Symmetry,
            DVec3::ZERO,
            DVec3::Z,
            DVec3::Y,
        );
        ruler.lines = 2;
        ruler.line_symmetry = true;
        s.ruler_create(ruler).unwrap();
        s.m2.brush.effect = BrushEffect::Smudge { strength: 1.0 };
        assert!(s.view3d.model.is_none());
        assert_eq!(
            symmetry_effect(&s, Place::View3d, layer, false),
            Err(s.lang.t("stroke_gate-smudge-and-clone-do-not").to_owned())
        );
        assert_eq!(symmetry_effect(&s, Place::Canvas, layer, false), Ok(()));
    }

    /// ほかのテクスチャセットの面の文は、名前を引用の形で包む。描けませんの文は理由を添える。
    #[test]
    fn the_words_quote_the_name_and_add_the_reason() {
        assert_eq!(
            other_set_face(Lang::Ja, "b"),
            "ほかのテクスチャセット「b」の面です。"
        );
        assert_eq!(
            other_set_face(Lang::En, "b"),
            "This face belongs to another texture set, \"b\"."
        );
        assert_eq!(
            cannot_paint(Lang::Ja, "理由"),
            Lang::Ja.with_reason(Lang::Ja.t("stroke_gate-cannot-paint"), "理由")
        );
        assert!(cannot_paint(Lang::Ja, "理由").starts_with("描けません"));
    }
}
