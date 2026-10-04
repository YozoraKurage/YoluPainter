//! M2 のポップアップ（自前のメニュー）の中身: 調整レイヤーの種類・一覧の空白の右クリック・ブラシの選択肢・チャンネルの種類。
//! 開いている種類は `PopupKind::M2(Popup)`、選ばれた項目は `Action` で返る（閉じてから当てるのは `YoluApp`）。

use crate::brushes::BrushAction;
use crate::engine::{
    Channel, ChannelInfo, ChannelKind, DualBrushMode, HeightEdgeMode, NormalYDirection, TextureMode,
};
use crate::lang::Lang;
use crate::m2::{
    self, dual_mode_label, kind_label, new_channel_info, texture_mode_label, tip_label,
    AdjustmentKind, BrushOp, Edit, EffectKind, UiOp,
};
use crate::region::RegionAction;
use crate::state::{Action, AppState};
use crate::ui::menu::Entry;

/// 開いている M2 のポップアップの種類。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Popup {
    /// レイヤーのパネルの調整ボタン。
    NewAdjustment,
    /// レイヤーの一覧の空白の右クリック。
    LayerBlank,
    /// ブラシの一覧の行の右クリック（対象は `brushes.ui.context`）。
    BrushContext,
    Effect,
    Tip,
    Texture,
    TextureMode,
    DualTip,
    DualMode,
    /// チャンネルのパネルの足すボタン（種類を選ぶ）。
    NewChannel,
    /// ユーザーチャンネルの種類。
    ChannelKind(Channel),
    ChannelContext(Channel),
    /// バケツ・ポリゴン塗りつぶしの範囲（今のツールで並びが違う）。
    Region,
    /// ステンシルの画像（読む・読んだ画像・外す）。
    StencilImage,
    StencilMode,
    StencilTiling,
    /// 移動・変形の補間。
    Resampling,
    /// Normal の設定の端（Clamp・Wrap）。
    NormalEdges,
    /// Normal の設定のファイルの Y の向き（OpenGL・DirectX）。
    NormalDirection,
    /// 設定の窓の選択肢。
    Pref(crate::prefs::PrefChoice),
}

fn tips(
    app: &AppState,
    current: Option<&str>,
    none: &str,
    op: fn(Option<&'static str>) -> BrushOp,
) -> Vec<Entry<Action>> {
    let mut v =
        vec![Entry::item(none, Action::M2Ui(UiOp::Brush(op(None)))).radio(current.is_none())];
    for id in yolu_core::brush::BUILTIN_TIPS {
        v.push(
            Entry::item(
                tip_label(app.lang, id),
                Action::M2Ui(UiOp::Brush(op(Some(id)))),
            )
            .radio(current == Some(id)),
        );
    }
    v
}

/// Normal の設定の端の名前（Height の微分が画布の外で読むもの）。
pub fn edges_name(lang: Lang, mode: HeightEdgeMode) -> &'static str {
    match mode {
        HeightEdgeMode::Clamp => lang.pick("クランプ", "Clamp"),
        HeightEdgeMode::Wrap => lang.pick("ラップ（タイル）", "Wrap (tiling)"),
    }
}

/// 新しいユーザーチャンネルの名前（種類の名前に番号。文書の中で重ならない）。
pub fn new_channel_name(app: &AppState, kind: ChannelKind) -> String {
    let base = kind_label(app.lang, kind);
    (1..)
        .map(|n| format!("{base} {n}"))
        .find(|name| {
            !app.doc
                .channels()
                .iter()
                .any(|c| m2::channel_name(app.lang, &app.doc, *c) == *name)
        })
        .expect("名前は尽きない")
}

/// 項目の並び。
pub fn entries(app: &AppState, popup: Popup) -> Vec<Entry<Action>> {
    let lang = app.lang;
    let free = !app.is_stroking();
    match popup {
        Popup::Resampling => [
            yolu_core::Resampling::Bilinear,
            yolu_core::Resampling::Nearest,
        ]
        .into_iter()
        .map(|mode| {
            Entry::item(
                crate::transform::resampling_name(lang, mode),
                Action::M2Ui(UiOp::Resampling(mode)),
            )
            .radio(app.transform.resampling == mode)
        })
        .collect(),
        Popup::Pref(choice) => crate::prefs::entries(app, choice),
        Popup::NormalEdges => {
            let current = app.doc.normal_settings();
            [HeightEdgeMode::Clamp, HeightEdgeMode::Wrap]
                .into_iter()
                .map(|mode| {
                    Entry::item(
                        edges_name(lang, mode),
                        Action::M2(Edit::NormalSettings {
                            settings: current.with_edges(mode),
                            coalesce: false,
                        }),
                    )
                    .radio(current.edges() == mode)
                    .enabled(free)
                })
                .collect()
        }
        Popup::NormalDirection => {
            let current = app.doc.normal_settings();
            [NormalYDirection::OpenGL, NormalYDirection::DirectX]
                .into_iter()
                .map(|direction| {
                    Entry::item(
                        m2::direction_name(direction),
                        Action::M2(Edit::NormalSettings {
                            settings: current.with_file_direction(direction),
                            coalesce: false,
                        }),
                    )
                    .radio(current.file_direction() == direction)
                    .enabled(free)
                })
                .collect()
        }
        Popup::Region => {
            let by_color = app.tool == crate::state::Tool::Fill;
            let mut v = Vec::new();
            if by_color {
                v.push(
                    Entry::item(
                        lang.pick("近い色", "Similar colors"),
                        Action::Region(RegionAction::ByColor(true)),
                    )
                    .radio(app.region.by_color),
                );
            }
            for kind in crate::region::KINDS {
                let action = if by_color {
                    // バケツ: 範囲の種類を選んだら、近い色はやめる
                    Action::Region(RegionAction::FillRange(kind))
                } else {
                    Action::Region(RegionAction::Kind(kind))
                };
                v.push(
                    Entry::item(crate::region::kind_name(lang, kind), action)
                        .radio((!by_color || !app.region.by_color) && app.region.kind == kind),
                );
            }
            v
        }
        Popup::NewAdjustment => AdjustmentKind::ALL
            .iter()
            .map(|k| Entry::item(k.name(lang), Action::M2(Edit::NewAdjustment(*k))).enabled(free))
            .collect(),
        Popup::LayerBlank => {
            let mut v = vec![
                Entry::item(lang.pick("新規レイヤー", "New Layer"), Action::NewLayer)
                    .shortcut("Ctrl+Shift+N")
                    .enabled(free),
                Entry::item(
                    lang.pick("新規グループ", "New Group"),
                    Action::M2(Edit::NewGroup),
                )
                .enabled(free),
                Entry::item(
                    lang.pick("新規塗りつぶしレイヤー", "New Fill Layer"),
                    Action::M2(Edit::NewFill),
                )
                .enabled(free),
                Entry::Separator,
            ];
            for k in AdjustmentKind::ALL {
                v.push(
                    Entry::item(
                        format!(
                            "{}: {}",
                            lang.pick("新規調整レイヤー", "New Adjustment Layer"),
                            k.name(lang)
                        ),
                        Action::M2(Edit::NewAdjustment(k)),
                    )
                    .enabled(free),
                );
            }
            v
        }
        Popup::BrushContext => {
            let Some(key) = app.brushes.ui.context else {
                return Vec::new();
            };
            let user = key.is_user();
            vec![
                Entry::item(
                    lang.pick("名前を変更", "Rename"),
                    Action::Brush(BrushAction::StartRename(key)),
                )
                .enabled(free && user),
                Entry::item(
                    lang.pick("複製", "Duplicate"),
                    Action::Brush(BrushAction::Duplicate(key)),
                )
                .enabled(free),
                Entry::item(
                    lang.pick("この設定で登録", "Register These Settings"),
                    Action::Brush(BrushAction::Register(key)),
                )
                .enabled(free && user && app.brush_is_modified(key)),
                Entry::item(
                    lang.pick("元に戻す", "Revert"),
                    Action::Brush(BrushAction::Revert(key)),
                )
                .enabled(free && app.brush_is_modified(key)),
                Entry::Separator,
                Entry::item(
                    lang.pick("削除", "Delete"),
                    Action::Brush(BrushAction::Delete(key)),
                )
                .enabled(free && user),
            ]
        }
        Popup::Effect => {
            let current = EffectKind::of(&app.m2.brush.effect);
            EffectKind::ALL
                .iter()
                .map(|k| {
                    Entry::item(k.name(lang), Action::M2Ui(UiOp::Brush(BrushOp::Effect(*k))))
                        .radio(current == *k)
                })
                .collect()
        }
        Popup::Tip => tips(
            app,
            app.m2.brush.tip.image.as_ref().map(|t| t.name()),
            lang.pick("丸（硬さ）", "Round (hardness)"),
            BrushOp::Tip,
        ),
        Popup::Texture => tips(
            app,
            app.m2.brush.texture.as_ref().map(|t| t.image.name()),
            lang.pick("なし", "None"),
            BrushOp::Texture,
        ),
        Popup::DualTip => tips(
            app,
            app.m2
                .brush
                .dual
                .as_ref()
                .and_then(|d| d.tip.as_ref())
                .map(|t| t.name()),
            lang.pick("丸（硬さ）", "Round (hardness)"),
            BrushOp::DualTip,
        ),
        Popup::TextureMode => {
            let current = app.m2.brush.texture.as_ref().map(|t| t.mode);
            TextureMode::ALL
                .iter()
                .map(|m| {
                    Entry::item(
                        texture_mode_label(lang, *m),
                        Action::M2Ui(UiOp::Brush(BrushOp::TextureMode(*m))),
                    )
                    .radio(current == Some(*m))
                })
                .collect()
        }
        Popup::DualMode => {
            let current = app.m2.brush.dual.as_ref().map(|d| d.mode);
            DualBrushMode::ALL
                .iter()
                .map(|m| {
                    Entry::item(
                        dual_mode_label(lang, *m),
                        Action::M2Ui(UiOp::Brush(BrushOp::DualMode(*m))),
                    )
                    .radio(current == Some(*m))
                })
                .collect()
        }
        Popup::NewChannel => [ChannelKind::Color, ChannelKind::Scalar, ChannelKind::Normal]
            .iter()
            .map(|k| {
                let info = new_channel_info(new_channel_name(app, *k), *k);
                Entry::item(kind_label(lang, *k), Action::M2(Edit::AddChannel(info))).enabled(free)
            })
            .collect(),
        Popup::ChannelKind(channel) => {
            let Some(info) = app.doc.channel_info(channel) else {
                return Vec::new();
            };
            [ChannelKind::Color, ChannelKind::Scalar, ChannelKind::Normal]
                .iter()
                .map(|k| {
                    let next = ChannelInfo {
                        kind: *k,
                        ..new_channel_info(info.name.clone(), *k)
                    };
                    Entry::item(
                        kind_label(lang, *k),
                        Action::M2(Edit::SetChannel {
                            channel,
                            info: next,
                        }),
                    )
                    .radio(info.kind == *k)
                    .enabled(free)
                })
                .collect()
        }
        Popup::StencilImage => crate::panels::stencil_props::image_entries(app),
        Popup::StencilMode => crate::panels::stencil_props::mode_entries(app),
        Popup::StencilTiling => crate::panels::stencil_props::tiling_entries(app),
        Popup::ChannelContext(channel) => {
            let user = !channel.is_standard();
            vec![
                Entry::item(
                    lang.pick("描くチャンネルにする", "Paint This Channel"),
                    Action::M2Ui(UiOp::PaintChannel(channel)),
                )
                .radio(app.m2.paint_channel == channel)
                .enabled(free),
                Entry::item(
                    lang.pick("キャンバスに表示", "Show in Canvas"),
                    Action::M2Ui(UiOp::DisplayChannel(channel)),
                )
                .radio(app.m2.display_channel == channel),
                Entry::Separator,
                Entry::item(
                    lang.pick("名前を変更", "Rename"),
                    Action::M2Ui(UiOp::RenameChannel(channel)),
                )
                .enabled(free && user),
                Entry::item(
                    lang.pick("チャンネルを削除", "Delete Channel"),
                    Action::M2(Edit::RemoveChannel(channel)),
                )
                .enabled(free && user),
            ]
        }
    }
}
