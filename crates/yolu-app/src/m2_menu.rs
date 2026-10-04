//! M2 のポップアップ（自前のメニュー）の中身: 調整レイヤーの種類・一覧の空白の右クリック・ブラシの選択肢・チャンネルの種類。
//! 開いている種類は `PopupKind::M2(Popup)`、選ばれた項目は `Action` で返る（閉じてから当てるのは `YoluApp`）。

use crate::engine::{Channel, ChannelInfo, ChannelKind, DualBrushMode, TextureMode};
use crate::m2::{
    self, dual_mode_label, kind_label, new_channel_info, preset_label, texture_mode_label,
    tip_label, AdjustmentKind, BrushOp, Edit, EffectKind, UiOp,
};
use crate::state::{Action, AppState};
use crate::ui::menu::Entry;

/// 開いている M2 のポップアップの種類。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Popup {
    /// レイヤーのパネルの調整ボタン。
    NewAdjustment,
    /// レイヤーの一覧の空白の右クリック。
    LayerBlank,
    Preset,
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
        Popup::Preset => {
            let mut v = Vec::new();
            let mut category = "";
            for (i, p) in m2::presets().iter().enumerate() {
                let (name, cat) = preset_label(lang, p);
                if p.category != category {
                    category = p.category;
                    v.push(Entry::Heading(cat.to_owned()));
                }
                v.push(
                    Entry::item(name, Action::M2Ui(UiOp::Preset(i)))
                        .radio(app.m2.preset == Some(i)),
                );
            }
            v
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
