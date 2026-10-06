//! M2 のポップアップ（自前のメニュー）の中身: 調整レイヤーの種類・一覧の空白の右クリック・ブラシの選択肢・チャンネルの種類。
//! 開いている種類は `PopupKind::M2(Popup)`、選ばれた項目は `Action` で返る（閉じてから当てるのは `YoluApp`）。

use crate::brushes::BrushAction;
use crate::engine::{
    Channel, ChannelInfo, ChannelKind, DualBrushMode, HeightEdgeMode, NormalYDirection, TextureMode,
};
use crate::lang::Lang;
use crate::m2::{
    self, dual_mode_label, kind_label, new_channel_info, texture_mode_label, tip_label, BrushOp,
    Edit, EffectKind, UiOp,
};
use crate::state::{Action, AppState};
use crate::subtool::SubToolAction;
use crate::ui::menu::Entry;

/// 開いている M2 のポップアップの種類。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Popup {
    /// レイヤーのパネルの調整ボタン。
    NewAdjustment,
    /// レイヤーのパネルの塗りつぶしボタン（単色・グラデーション・画像 ▸・デカール ▸）。
    NewFill,
    /// レイヤーの一覧の空白の右クリック。
    LayerBlank,
    /// ブラシの一覧の行の右クリック（対象は `brushes.ui.context`）。
    BrushContext,
    /// サブツール（バケツ・グラデーションなどのプリセット）の右クリック。
    SubToolContext,
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
    /// ステンシルの画像（読む・読んだ画像・外す）。
    StencilImage,
    StencilMode,
    StencilTiling,
    /// プロパティの欄の「フィルターを足す」（画素かマスクへ、フィルターと Generator を足す）。
    AddEffect(yolu_core::FilterTarget),
    /// 選んでいる効果の欄のドロップダウン（合成・軸・向き・置き場・形・アンカーなど）。
    Fx(crate::fx::menu::FxChoice),
    /// 効果の行の右クリック（選んでいる段・アンカーの操作）。
    EffectContext,
    /// 移動・変形の補間。
    Resampling,
    /// Normal の設定の端（Clamp・Wrap）。
    NormalEdges,
    /// Normal の設定のファイルの Y の向き（OpenGL・DirectX）。
    NormalDirection,
    /// 設定の窓の選択肢。
    Pref(crate::prefs::PrefChoice),
    /// 塗りつぶしの層のチャンネルの画像（棚の画像の一覧・ファイルから取り込む・外す）。
    FillImage(crate::engine::LayerId, Channel),
    /// 棚の画像の読み方（色空間）。
    ImageSpace(yolu_core::ImageId),
    /// 塗りつぶしの層の投影の種類・外側。
    ProjectionMode(crate::engine::LayerId),
    ProjectionWrap(crate::engine::LayerId),
    /// 形のグラデーションの形・階調のプリセット・値のカーブのプリセット。階調と値のカーブのプリセットは、欄では `ramp_rows` のグラデーションセットの
    /// 一覧と値のカーブの欄へ移ったので、今は欄から開かない（項目の出し方と当て方を試験が確かめている）。
    GradientShape(crate::engine::LayerId, Channel),
    RampPresets(crate::engine::LayerId, Channel),
    CurvePresets(crate::engine::LayerId, Channel),
    /// 見た目の設定の欄のドロップダウン（種類・描画モード・選ぶ値・テクスチャのスロット）。
    Look(crate::look::panel::LookChoice),
}

fn tips(
    app: &AppState,
    current: Option<&yolu_core::BrushTip>,
    none: &str,
    op: fn(Option<&'static str>) -> BrushOp,
) -> Vec<Entry<Action>> {
    let mut v =
        vec![Entry::item(none, Action::M2Ui(UiOp::Brush(op(None)))).radio(current.is_none())];
    for id in yolu_core::brush::BUILTIN_TIPS {
        // 名前だけ同じで中身が違う（取り込んだ）画像は、組み込みとして印を付けない
        let on = current.is_some_and(|t| yolu_core::builtin_tip(id).is_some_and(|b| *b == *t));
        v.push(
            Entry::item(
                tip_label(app.lang, id),
                Action::M2Ui(UiOp::Brush(op(Some(id)))),
            )
            .radio(on),
        );
    }
    v
}

/// 取り込んだ模様（模様から作ったブラシの質感の画像）: 一覧の並びで、ブラシの名前と質感の画像。
pub fn patterns(
    app: &AppState,
) -> Vec<(
    crate::brushes::BrushKey,
    String,
    std::sync::Arc<yolu_core::BrushTip>,
)> {
    app.brushes
        .lib
        .entries()
        .iter()
        .filter(|e| e.import.as_ref().is_some_and(|m| m.pattern))
        .filter_map(|e| {
            let image = e.baseline.texture.as_ref()?.image.clone();
            Some((e.key, e.name.clone(), image))
        })
        .collect()
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
        Popup::Look(choice) => crate::look::panel::entries(app, choice),
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
        Popup::NewAdjustment => crate::layermenu::adjustment_entries(app),
        Popup::NewFill => crate::layermenu::fill_entries(app),
        // 選んだ層が無いときのメニューバーの「レイヤー」と同じ並び
        Popup::LayerBlank => crate::shell::layer_menu(app, None),
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
        Popup::SubToolContext => {
            let Some((tool, key)) = app.subtools.ui.context else {
                return Vec::new();
            };
            let user = key.is_user();
            let modified = app.subtool_is_modified(tool, key);
            vec![
                Entry::item(
                    lang.pick("名前を変更", "Rename"),
                    Action::SubTool(SubToolAction::StartRename(tool, key)),
                )
                .enabled(free && user),
                Entry::item(
                    lang.pick("複製", "Duplicate"),
                    Action::SubTool(SubToolAction::Duplicate(tool, key)),
                )
                .enabled(free),
                Entry::item(
                    lang.pick("この設定で登録", "Register These Settings"),
                    Action::SubTool(SubToolAction::Register(tool, key)),
                )
                .enabled(free && user && modified),
                Entry::item(
                    lang.pick("元に戻す", "Revert"),
                    Action::SubTool(SubToolAction::Revert(tool, key)),
                )
                .enabled(free && modified),
                Entry::Separator,
                Entry::item(
                    lang.pick("削除", "Delete"),
                    Action::SubTool(SubToolAction::Delete(tool, key)),
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
            app.m2.brush.tip.image.as_deref(),
            lang.pick("丸（硬さ）", "Round (hardness)"),
            BrushOp::Tip,
        ),
        Popup::Texture => {
            let current = app.m2.brush.texture.as_ref().map(|t| &*t.image);
            let mut v = tips(app, current, lang.pick("なし", "None"), BrushOp::Texture);
            // 取り込んだ模様は、組み込みの質感の後ろに並べる
            let patterns = patterns(app);
            if !patterns.is_empty() {
                v.push(Entry::Separator);
                for (key, name, image) in patterns {
                    v.push(
                        Entry::item(
                            name,
                            Action::M2Ui(UiOp::Brush(BrushOp::PatternTexture(key))),
                        )
                        .radio(current.is_some_and(|t| *t == *image)),
                    );
                }
            }
            v
        }
        Popup::DualTip => tips(
            app,
            app.m2.brush.dual.as_ref().and_then(|d| d.tip.as_deref()),
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
        Popup::FillImage(..)
        | Popup::ImageSpace(_)
        | Popup::ProjectionMode(_)
        | Popup::ProjectionWrap(_)
        | Popup::GradientShape(..)
        | Popup::RampPresets(..)
        | Popup::CurvePresets(..) => crate::panels::fill_props::entries(app, popup),
        Popup::AddEffect(target) => crate::fx::menu::add_entries(app, target),
        Popup::Fx(choice) => crate::fx::menu::choice_entries(app, choice),
        Popup::EffectContext => crate::fx::menu::context_entries(app),
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
