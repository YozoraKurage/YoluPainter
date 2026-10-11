//! M2 のポップアップ（自前のメニュー）の中身: 調整レイヤーの種類・一覧の空白の右クリック・ブラシの選択肢・チャンネルの種類。
//! 開いている種類は `PopupKind::M2(Popup)`、選ばれた項目は `Action` で返る（閉じてから当てるのは `YoluApp`）。

use crate::engine::{
    AntiAlias, Channel, ChannelInfo, ChannelKind, DualBrushMode, HeightEdgeMode, NormalYDirection,
    TextureMode,
};
use crate::lang::Lang;
use crate::m2::{
    self, anti_alias_label, channel_format, dual_mode_label, kind_name, new_channel_info,
    texture_mode_label, tip_label, BrushOp, Edit, EffectKind, UiOp,
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
    /// 「フィルターを追加」のボタン（画素かマスクへ、フィルターを足す）。
    AddFilter(yolu_core::FilterTarget),
    /// 「ジェネレーターを追加」のボタン（画素かマスクへ、ジェネレーターを足す）。
    AddGenerator(yolu_core::FilterTarget),
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
    /// 設定のウィンドウの選択肢。
    Pref(crate::prefs::PrefChoice),
    /// 書き出しのウィンドウの形。
    ExportForm,
    /// 塗りつぶしレイヤーのチャンネルの画像（棚の画像の一覧・ファイルから取り込む・外す）。
    FillImage(crate::engine::LayerId, Channel),
    /// 棚の画像の読み方（色空間）。
    ImageSpace(yolu_core::ImageId),
    /// 塗りつぶしレイヤーの投影の種類・外側。
    ProjectionMode(crate::engine::LayerId),
    ProjectionWrap(crate::engine::LayerId),
    /// 形のグラデーションの形・階調のプリセット・値のカーブのプリセット。階調と値のカーブのプリセットは、欄では `ramp_rows` のグラデーションセットの
    /// 一覧と値のカーブの欄へ移ったので、今は欄から開かない（項目の出し方と当て方を試験が確かめている）。
    GradientShape(crate::engine::LayerId, Channel),
    RampPresets(crate::engine::LayerId, Channel),
    CurvePresets(crate::engine::LayerId, Channel),
    /// 見た目の設定の欄のドロップダウン（種類・描画モード・選ぶ値・テクスチャのスロット）。
    Look(crate::look::panel::LookChoice),
    /// レイヤーの一覧のマスクのサムネイルの右クリック（そのレイヤーのマスク）。
    MaskContext(crate::engine::LayerId),
    /// パスの一覧の行の右クリック（パスの ID）。
    PathContext(u128),
    /// パスの種類・リボンの画像・リボンの並べ方。
    PathKind,
    PathRibbonImage,
    PathRibbonMode,
    /// パスの筆先。
    PathTip,
    /// パスのプリセット。
    PathPresets,
    /// ツールの列の右クリック（対象は `toolset.ui.context_slot`。空いた所なら None）。
    ToolStripContext,
    /// ブラシのグループのタブの右クリック（対象は `toolset.ui.context_group`）。
    GroupContext,
    /// 「＋」のウィンドウの行の右クリック（対象は `toolset.catalog.context`）。
    CatalogContext,
    /// ポーズの欄のテイク（FBX の中のアニメ）。
    Take,
    /// テキストのフォント（同梱・インストール済み・選んだフォントのファイル・ファイルから選ぶ）。
    TextFont,
    /// ブラシの縁のアンチエイリアス（なし・弱・中・強）。
    AntiAlias,
    /// パスのブラシの縁のアンチエイリアス。
    PathAntiAlias,
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

/// Normal の設定の端の名前（Height の微分がキャンバスの外で読むもの）。
pub fn edges_name(lang: Lang, mode: HeightEdgeMode) -> &'static str {
    match mode {
        HeightEdgeMode::Clamp => lang.pick("クランプ", "Clamp"),
        HeightEdgeMode::Wrap => lang.pick("ラップ（タイル）", "Wrap (tiling)"),
    }
}

/// 新しいユーザーチャンネルの名前（種類の名前に番号。文書の中で重ならない）。
pub fn new_channel_name(app: &AppState, kind: ChannelKind) -> String {
    let base = kind_name(app.lang, kind);
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

/// ユーザーチャンネルの種類のメニュー。項目は種類ごとの形式（その種類で新しく作る情報の色空間）で、印は項目の文字が欄の文字
/// （[`channel_format`]）と同じ項目に付く。ファイルから読んだリニアのカラー（欄は RGB8）はノーマルの項目に、sRGB のノーマル（欄は sRGB8）は
/// カラーの項目に印が付く。sRGB のスカラー（欄は sL8）は同じ文字の項目が無く、印は付かない。印の付いた項目は今の形式なので、押しても
/// 何もしない（今と同じ情報への `SetChannel` は何も変えず、Undo の段も作らない）。ほかの項目は、その種類で新しく作る情報への 1 回の `SetChannel`。
fn channel_kind_entries(channel: Channel, info: &ChannelInfo, free: bool) -> Vec<Entry<Action>> {
    let shown = channel_format(info);
    [ChannelKind::Color, ChannelKind::Scalar, ChannelKind::Normal]
        .into_iter()
        .map(|kind| {
            let next = new_channel_info(info.name.clone(), kind);
            let label = channel_format(&next);
            let marked = label == shown;
            let target = if marked { info.clone() } else { next };
            Entry::item(
                label,
                Action::M2(Edit::SetChannel {
                    channel,
                    info: target,
                }),
            )
            .radio(marked)
            .enabled(free)
        })
        .collect()
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
        Popup::ExportForm => crate::export::window::form_entries(app),
        Popup::Look(choice) => crate::look::panel::entries(app, choice),
        Popup::Take => crate::panels::pose::take_entries(app),
        Popup::TextFont => crate::textlayer::props::font_entries(app),
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
        // 選んだレイヤーが無いときのメニューバーの「レイヤー」と同じ並び
        Popup::LayerBlank => crate::shell::layer_menu(app, None),
        Popup::BrushContext => crate::toolset::ui::brush_menu(app),
        Popup::ToolStripContext => crate::toolset::ui::strip_menu(app),
        Popup::GroupContext => crate::toolset::ui::group_menu(app),
        Popup::CatalogContext => crate::panels::brush_catalog::context_menu(app),
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
        Popup::AntiAlias => AntiAlias::ALL
            .iter()
            .map(|a| {
                Entry::item(
                    anti_alias_label(lang, *a),
                    Action::M2Ui(UiOp::Brush(BrushOp::AntiAlias(*a))),
                )
                .radio(app.brush.anti_alias == *a)
            })
            .collect(),
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
                Entry::item(channel_format(&info), Action::M2(Edit::AddChannel(info))).enabled(free)
            })
            .collect(),
        Popup::ChannelKind(channel) => match app.doc.channel_info(channel) {
            Some(info) => channel_kind_entries(channel, info, free),
            None => Vec::new(),
        },
        Popup::FillImage(..)
        | Popup::ImageSpace(_)
        | Popup::ProjectionMode(_)
        | Popup::ProjectionWrap(_)
        | Popup::GradientShape(..)
        | Popup::RampPresets(..)
        | Popup::CurvePresets(..) => crate::panels::fill_props::entries(app, popup),
        Popup::AddFilter(target) => crate::fx::menu::add_filter_entries(app, target),
        Popup::AddGenerator(target) => crate::fx::menu::add_generator_entries(app, target),
        Popup::MaskContext(layer) => crate::fx::menu::mask_entries(app, layer),
        Popup::Fx(choice) => crate::fx::menu::choice_entries(app, choice),
        Popup::EffectContext => crate::fx::menu::context_entries(app),
        Popup::StencilImage => crate::panels::stencil_props::image_entries(app),
        Popup::StencilMode => crate::panels::stencil_props::mode_entries(app),
        Popup::StencilTiling => crate::panels::stencil_props::tiling_entries(app),
        Popup::PathContext(id) => crate::panels::path_props::context_entries(app, id),
        Popup::PathKind => crate::panels::path_props::kind_entries(app),
        Popup::PathRibbonImage => crate::panels::path_props::ribbon_image_entries(app),
        Popup::PathRibbonMode => crate::panels::path_props::ribbon_mode_entries(app),
        Popup::PathTip => crate::panels::path_props::tip_entries(app),
        Popup::PathPresets => crate::panels::path_props::preset_entries(app),
        Popup::PathAntiAlias => crate::panels::path_props::anti_alias_entries(app),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::ColorSpace;
    use crate::ui::menu::Check;

    const KINDS: [ChannelKind; 3] = [ChannelKind::Color, ChannelKind::Scalar, ChannelKind::Normal];
    const SPACES: [ColorSpace; 2] = [ColorSpace::Srgb, ColorSpace::Linear];

    /// 項目の見た目（名前・印・押せるか）。
    fn rows(entries: &[Entry<Action>]) -> Vec<(String, bool, bool)> {
        entries
            .iter()
            .map(|e| match e {
                Entry::Item {
                    label,
                    check,
                    enabled,
                    ..
                } => (label.clone(), *check == Check::Radio, *enabled),
                other => panic!("項目だけのはず: {other:?}"),
            })
            .collect()
    }

    fn info_with(kind: ChannelKind, color_space: ColorSpace) -> ChannelInfo {
        ChannelInfo {
            color_space,
            ..new_channel_info("x".into(), kind)
        }
    }

    fn app_with(info: ChannelInfo, lang: Lang) -> (AppState, Channel) {
        let mut s = AppState::new(64, 64);
        s.lang = lang;
        s.apply(Action::M2(Edit::AddChannel(info)));
        let channel = s.m2.paint_channel;
        assert!(!channel.is_standard());
        (s, channel)
    }

    fn kind_entries(s: &AppState, channel: Channel) -> Vec<Entry<Action>> {
        entries(s, Popup::ChannelKind(channel))
    }

    /// 項目の文字から行動を取る（押せる項目だけ）。
    fn action_of(entries: Vec<Entry<Action>>, label: &str) -> Option<Action> {
        entries.into_iter().find_map(|e| match e {
            Entry::Item {
                label: l,
                enabled: true,
                action,
                ..
            } if l == label => Some(action),
            _ => None,
        })
    }

    /// 欄が出す形式（`channel_format`）と同じ文字の項目だけに印が付く。アプリが作る 3 通りは今までと同じ位置、ほかで作った文書にある
    /// リニアのカラー（欄は RGB8）はノーマルの項目、sRGB のノーマル（欄は sRGB8）はカラーの項目に付く。項目はいつも 3 つで、全部押せる。
    /// 日英で同じ（文字は形式だけ）。
    #[test]
    fn the_mark_names_the_format_the_row_shows() {
        // （今のチャンネル, 欄の文字, 印の付く項目の位置）
        let cases = [
            (info_with(ChannelKind::Color, ColorSpace::Srgb), "sRGB8", 0),
            (info_with(ChannelKind::Scalar, ColorSpace::Linear), "L8", 1),
            (
                info_with(ChannelKind::Normal, ColorSpace::Linear),
                "RGB8",
                2,
            ),
            (info_with(ChannelKind::Color, ColorSpace::Linear), "RGB8", 2),
            (info_with(ChannelKind::Normal, ColorSpace::Srgb), "sRGB8", 0),
        ];
        for lang in [Lang::Ja, Lang::En] {
            for (info, shown, at) in &cases {
                let (s, channel) = app_with(info.clone(), lang);
                assert_eq!(channel_format(info), *shown);
                let expected: Vec<_> = ["sRGB8", "L8", "RGB8"]
                    .iter()
                    .enumerate()
                    .map(|(i, label)| ((*label).to_owned(), i == *at, true))
                    .collect();
                let got = rows(&kind_entries(&s, channel));
                assert_eq!(got, expected, "{shown} ({:?}) {lang:?}", info.kind);
                assert_eq!(got[*at].0, *shown);
            }
        }
    }

    /// 種類 3 つ × 色空間 2 つの全部で、印が付くなら、その項目の文字は欄の文字と同じ（印は欄と別物に付かない）。
    /// 印の付くのは 1 つ以下で、項目は 3 つのまま。sRGB のスカラー（欄は sL8）は同じ文字の項目が無く、印が付かない。
    #[test]
    fn a_mark_never_names_anything_but_the_row_format() {
        for kind in KINDS {
            for color_space in SPACES {
                let info = info_with(kind, color_space);
                let (s, channel) = app_with(info.clone(), Lang::Ja);
                let rows = rows(&kind_entries(&s, channel));
                let shown = channel_format(&info);
                let tag = format!("{shown} ({kind:?})");
                assert_eq!(rows.len(), 3, "{tag}");
                assert!(rows.iter().all(|r| r.2), "{tag}");
                let marked: Vec<_> = rows.iter().filter(|r| r.1).collect();
                if shown == "sL8" {
                    assert!(marked.is_empty(), "{tag}");
                } else {
                    assert_eq!(marked.len(), 1, "{tag}");
                    assert_eq!(marked[0].0, shown, "{tag}");
                }
            }
        }
    }

    /// 印の付いた項目を押しても、文書も Undo の段も変わらず、知らせも出ない（Undo は、そのチャンネルを追加した 1 つ前の編集を戻す）。
    /// アプリが作る 3 通りとほかで作った 2 通りのどれでも。
    #[test]
    fn pressing_the_marked_item_changes_nothing() {
        let cases = [
            info_with(ChannelKind::Color, ColorSpace::Srgb),
            info_with(ChannelKind::Scalar, ColorSpace::Linear),
            info_with(ChannelKind::Normal, ColorSpace::Linear),
            info_with(ChannelKind::Color, ColorSpace::Linear),
            info_with(ChannelKind::Normal, ColorSpace::Srgb),
        ];
        for info in cases {
            let tag = channel_format(&info);
            let (mut s, channel) = app_with(info.clone(), Lang::Ja);
            let marked = kind_entries(&s, channel)
                .into_iter()
                .find_map(|e| match e {
                    Entry::Item {
                        check: Check::Radio,
                        action,
                        ..
                    } => Some(action),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("{tag}: 印の項目がない"));
            let revision = s.doc.revision();
            let steps = s.doc.undo_count();
            let message = s.message.clone();
            s.modified = false;
            s.apply(marked);
            assert_eq!(s.doc.channel_info(channel), Some(&info), "{tag}");
            assert_eq!(s.doc.revision(), revision, "{tag}: 文書の版");
            assert_eq!(s.doc.undo_count(), steps, "{tag}: Undo の段");
            assert_eq!(s.message, message, "{tag}: 知らせ");
            assert!(!s.modified, "{tag}: 変更の印");
            // 今の種類と色空間のまま、印の位置も変わらない
            let now = rows(&kind_entries(&s, channel));
            assert_eq!(now.iter().filter(|r| r.1).count(), 1, "{tag}");
            // 最後の段はチャンネルを追加した編集のまま
            s.apply(Action::Undo);
            assert_eq!(
                s.doc.channel_info(channel),
                None,
                "{tag}: Undo 1 回で追加する前"
            );
        }
    }

    /// 既定の値を変えたチャンネルでも、印の項目は何もしない。ほかの項目を選ぶと、その種類で新しく作る情報（既定の値も）になる。
    #[test]
    fn the_marked_item_keeps_a_changed_default() {
        let info = ChannelInfo {
            default: crate::engine::Rgba8::new(10, 20, 30, 255),
            ..info_with(ChannelKind::Color, ColorSpace::Srgb)
        };
        let (mut s, channel) = app_with(info.clone(), Lang::Ja);
        let steps = s.doc.undo_count();
        let marked = action_of(kind_entries(&s, channel), "sRGB8").unwrap();
        s.apply(marked);
        assert_eq!(s.doc.channel_info(channel), Some(&info));
        assert_eq!(s.doc.undo_count(), steps);
        let other = action_of(kind_entries(&s, channel), "L8").unwrap();
        s.apply(other);
        let after = s.doc.channel_info(channel).unwrap();
        assert_eq!(*after, new_channel_info("x".into(), ChannelKind::Scalar));
        assert_eq!(s.doc.undo_count(), steps + 1);
    }

    /// 描いている間は 3 つとも押せなくなるが、印は変わらない。
    #[test]
    fn a_stroke_disables_every_item_and_keeps_the_mark() {
        let info = info_with(ChannelKind::Color, ColorSpace::Linear);
        for free in [true, false] {
            let rows = rows(&channel_kind_entries(Channel::Color, &info, free));
            assert_eq!(rows.len(), 3, "{free}");
            assert!(rows.iter().all(|r| r.2 == free), "{free}");
            assert_eq!(
                rows.iter().filter(|r| r.1).collect::<Vec<_>>(),
                [&("RGB8".to_owned(), true, free)],
                "{free}"
            );
        }
    }

    /// ほかの項目を選ぶと 1 回の `SetChannel` で、Undo 1 回で元の種類と色空間に戻る。選んだ後のメニューは、選んだ項目に印が付く。
    #[test]
    fn choosing_another_item_is_one_undo() {
        let cases = [
            // （今のチャンネル, 選ぶ項目の文字）
            (info_with(ChannelKind::Color, ColorSpace::Srgb), "L8"),
            (info_with(ChannelKind::Scalar, ColorSpace::Linear), "RGB8"),
            (info_with(ChannelKind::Normal, ColorSpace::Linear), "sRGB8"),
            // 種類は同じで色空間だけが替わる（印の項目は別の種類の項目）
            (info_with(ChannelKind::Color, ColorSpace::Linear), "sRGB8"),
            (info_with(ChannelKind::Color, ColorSpace::Linear), "L8"),
            (info_with(ChannelKind::Normal, ColorSpace::Srgb), "RGB8"),
            (info_with(ChannelKind::Normal, ColorSpace::Srgb), "L8"),
            // 欄は sL8 で印が無い。どの項目を選んでも替わる
            (info_with(ChannelKind::Scalar, ColorSpace::Srgb), "L8"),
            (info_with(ChannelKind::Scalar, ColorSpace::Srgb), "sRGB8"),
            (info_with(ChannelKind::Scalar, ColorSpace::Srgb), "RGB8"),
        ];
        for (info, pick) in cases {
            let (mut s, channel) = app_with(info.clone(), Lang::Ja);
            let tag = format!("{} -> {pick}", channel_format(&info));
            let chosen = action_of(kind_entries(&s, channel), pick)
                .unwrap_or_else(|| panic!("{tag}: 項目がない"));
            let steps = s.doc.undo_count();
            s.apply(chosen);
            let after = s.doc.channel_info(channel).unwrap().clone();
            assert_eq!(s.doc.undo_count(), steps + 1, "{tag}: 1 回の編集");
            assert_eq!(channel_format(&after), pick, "{tag}");
            assert_eq!(after.name, info.name, "{tag}");
            // 選んだ後は、選んだ項目だけに印が付く
            let rows = rows(&kind_entries(&s, channel));
            assert_eq!(rows.len(), 3, "{tag}");
            let marked: Vec<_> = rows.iter().filter(|r| r.1).collect();
            assert_eq!(marked.len(), 1, "{tag}");
            assert_eq!(marked[0].0, pick, "{tag}");
            s.apply(Action::Undo);
            assert_eq!(s.doc.channel_info(channel), Some(&info), "{tag}: Undo 1 回");
        }
    }
}
