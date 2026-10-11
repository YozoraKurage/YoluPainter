//! レイヤーを足すメニューの部品（メニューバーの「レイヤー」・レイヤーの右クリック・レイヤーの一覧の空白の右クリックが同じものを使う。
//! レイヤーのパネルの下の帯のボタンは、新規レイヤー・塗りつぶし・調整だけ）: 「新規レイヤー」「新規塗りつぶしレイヤー ▸」「新規調整レイヤー ▸」
//! 「新規パスレイヤー」。
//!
//! 塗りつぶしの種類は単色・グラデーションデカール・画像・デカール。グラデーションは形（ボックス・球・平面）の一覧をもう 1 段の
//! 入れ子に開き、選んだ形で新しい塗りつぶしレイヤーを作る（名前は塗りつぶしの欄の「形」と同じ）。画像とデカールは棚の画像の一覧（その下に
//! 「ファイルから取り込む…」）を同じく入れ子に開き、選んだ画像で新しい塗りつぶしレイヤーを作る（デカールは投影を Decal に）。選ばずに閉じれば
//! 何も作らず、Undo の段も増えない。レイヤーの作成と画像・投影の設定は 1 回の Undo。新しい保存の形は無い（既存の塗りつぶしレイヤーの画像・投影・グラデーション）。

use std::path::PathBuf;

use yolu_core::fill_image::ProjectionMode;
use yolu_core::generator::Shape;
use yolu_core::{Channel, ChannelKind, ImageId, LayerKind};

use crate::fillfx::{default_fallback, inputs, placement, FillOp};
use crate::fx::names;
use crate::m2::{AdjustmentKind, Edit};
use crate::notice::Source;
use crate::state::{Action, AppState, DialogRequest};
use crate::ui::menu::Entry;

/// 新しい塗りつぶしレイヤーを作る操作（メニューの項目）。
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    /// 棚の画像で、投影が `mode`（UV・デカールなど）の塗りつぶしレイヤーを作る。
    FillImage {
        image: ImageId,
        mode: ProjectionMode,
    },
    /// PNG を選ぶウィンドウを開く（選んだら棚へ取り込み、その画像で `FillImage` と同じレイヤーを作る。選ばずに閉じたら何もしない）。
    FillImageDialog(ProjectionMode),
    /// 選んだ PNG を棚へ取り込み、その画像で塗りつぶしレイヤーを作る（ウィンドウの結果）。
    FillImageFile { path: PathBuf, mode: ProjectionMode },
    /// グラデーションデカール（モデルの外形に合わせた、その形の置き場）の塗りつぶしレイヤーを作り、3D ビューで形を編集できるようにする。
    FillGradient(Shape),
    /// 点の無い 1 本のパスを持つパスレイヤーを、選んでいるレイヤーのすぐ上に作り、パスのツールへ替える（最初の点は、パスのツールで置く）。
    PathLayer,
}

/// 「新規レイヤー」「新規塗りつぶしレイヤー ▸」「新規調整レイヤー ▸」「新規パスレイヤー」。
pub fn creation_entries(app: &AppState) -> Vec<Entry<Action>> {
    let lang = app.lang;
    let free = !app.is_stroking();
    vec![
        Entry::item(lang.pick("新規レイヤー", "New Layer"), Action::NewLayer)
            .command_key("layer.new")
            .enabled(free),
        Entry::submenu(
            lang.pick("新規塗りつぶしレイヤー", "New Fill Layer"),
            fill_entries(app),
        ),
        Entry::submenu(
            lang.pick("新規調整レイヤー", "New Adjustment Layer"),
            adjustment_entries(app),
        ),
        Entry::item(
            lang.pick("新規パスレイヤー", "New Path Layer"),
            Action::LayerMenu(Op::PathLayer),
        )
        .enabled(free),
    ]
}

/// 調整レイヤーの種類（全部）。
pub fn adjustment_entries(app: &AppState) -> Vec<Entry<Action>> {
    let free = !app.is_stroking();
    AdjustmentKind::ALL
        .iter()
        .map(|k| Entry::item(k.name(app.lang), Action::M2(Edit::NewAdjustment(*k))).enabled(free))
        .collect()
}

/// 塗りつぶしの種類: 単色・グラデーションデカール ▸・画像 ▸・デカール ▸。
pub fn fill_entries(app: &AppState) -> Vec<Entry<Action>> {
    let lang = app.lang;
    let free = !app.is_stroking();
    vec![
        Entry::item(lang.pick("単色", "Solid Color"), Action::M2(Edit::NewFill)).enabled(free),
        Entry::submenu(
            lang.pick("グラデーションデカール", "Gradient Decal"),
            gradient_entries(app),
        )
        .tooltip(lang.pick(
            "3D ビューの箱・球・平面の範囲で、値を塗り分ける",
            "Paints values over a box, sphere or plane in the 3D view",
        ))
        .enabled(free),
        Entry::submenu(
            lang.pick("画像", "Image"),
            image_entries(app, ProjectionMode::Uv),
        )
        .enabled(free),
        Entry::submenu(
            lang.pick("デカール", "Decal"),
            image_entries(app, ProjectionMode::Decal),
        )
        .enabled(free),
    ]
}

/// グラデーションデカールの形の一覧（選ぶと、その形の塗りつぶしレイヤーを作る）。名前と順は塗りつぶしの欄の「形」と同じ。
fn gradient_entries(app: &AppState) -> Vec<Entry<Action>> {
    let lang = app.lang;
    let free = !app.is_stroking();
    names::SHAPES
        .iter()
        .map(|shape| {
            Entry::item(
                names::shape_name(lang, *shape),
                Action::LayerMenu(Op::FillGradient(*shape)),
            )
            .tooltip(names::shape_hint(lang, *shape))
            .enabled(free)
        })
        .collect()
}

/// 棚の画像の一覧（選ぶと、その画像と投影 `mode` の塗りつぶしレイヤーを作る）と、「ファイルから取り込む…」。
fn image_entries(app: &AppState, mode: ProjectionMode) -> Vec<Entry<Action>> {
    let lang = app.lang;
    let free = !app.is_stroking();
    let mut v = Vec::new();
    for r in app.shelf.resources().iter().filter(|r| r.kind == "image") {
        let Some(image) = inputs::image_id(&r.id) else {
            continue;
        };
        let (w, h) = (
            r.metadata["width"].as_u64().unwrap_or(0),
            r.metadata["height"].as_u64().unwrap_or(0),
        );
        v.push(
            Entry::item(
                format!("{}  ({w} × {h})", r.name),
                Action::LayerMenu(Op::FillImage { image, mode }),
            )
            .enabled(free),
        );
    }
    if !v.is_empty() {
        v.push(Entry::Separator);
    }
    let mut import = Entry::item(
        lang.pick("ファイルから取り込む…", "Import from File…"),
        Action::LayerMenu(Op::FillImageDialog(mode)),
    )
    .enabled(free && app.shelf.unavailable.is_none());
    if let Some(why) = &app.shelf.unavailable {
        import = import.tooltip(why.reason(lang));
    }
    v.push(import);
    v
}

impl AppState {
    /// 新しい塗りつぶしレイヤーを作る操作を当てる。断られたら何も変えず、理由を状態の帯へ。
    pub fn layer_menu_apply(&mut self, op: Op) {
        match op {
            Op::FillImage { image, mode } => self.new_image_fill(image, mode),
            Op::FillImageDialog(mode) => {
                if !self.shelf_refuse_while_saving() {
                    self.dialog_request = Some(DialogRequest::NewFillImage(mode));
                }
            }
            Op::FillImageFile { path, mode } => {
                // 描いている間は、棚へも取り込まない（断るなら何も変えない）
                if self.is_stroking() {
                    self.refuse(
                        Source::FillLayer,
                        crate::lang::refusals::during_stroke(self.lang),
                    );
                    return;
                }
                // 取り込みは「画像を差す」欄のファイル取り込みと同じ道（棚を変える操作は、保存の最中は断る）。成功すると棚の選択が
                // 取り込んだ（すでにあれば、その）画像になるので、それを読む。失敗したら、前の選択へ戻す
                let before = self.shelf.selected.take();
                self.fill_apply(FillOp::ImportImage(path));
                match self
                    .shelf
                    .selected
                    .clone()
                    .and_then(|id| inputs::image_id(&id))
                {
                    Some(image) => self.new_image_fill(image, mode),
                    None => self.shelf.selected = before,
                }
            }
            Op::FillGradient(shape) => self.new_gradient_fill(shape),
            Op::PathLayer => self.new_path_layer(),
        }
    }

    /// 選んでいるレイヤーの上に、塗りつぶしレイヤーを 1 回の Undo で足す（`fill` がレイヤーを作ってから中身を入れる）。作れたら選んで、マスクを描く状態をやめる。
    fn add_fill_layer_with(
        &mut self,
        name: &str,
        fill: impl FnOnce(
            &mut yolu_core::Document,
            yolu_core::LayerId,
            Channel,
        ) -> Result<(), yolu_core::CoreError>,
    ) -> Option<yolu_core::LayerId> {
        let channel = self.m2.paint_channel;
        let kind = self
            .doc
            .channel_info(channel)
            .map_or(ChannelKind::Color, |c| c.kind);
        let above = self
            .selected_layer
            .filter(|id| self.doc.layer(*id).is_some());
        // 値は、画像・グラデーションが使えないときに出る既定の値（デカールを置くときと同じ）
        let value = default_fallback(kind);
        match self.doc.batch(|d| {
            let id = d.add_fill_layer(name, &[(channel, value)], above)?;
            fill(d, id, channel)?;
            Ok(id)
        }) {
            Ok(id) => {
                // 断られて巻き戻したときは変更番号だけが進むので、変更の印は作れたときだけ付ける
                self.modified = true;
                self.select_new(id);
                Some(id)
            }
            Err(e) => {
                self.notify(
                    crate::notice::Kind::of_core(&e),
                    Source::FillLayer,
                    self.lang.core_error(&e),
                );
                None
            }
        }
    }

    /// 棚の画像と投影 `mode` で、新しい塗りつぶしレイヤーを作る。画像を使えない（棚に無い・読めない・予算）ときは何も作らず理由を出す。
    fn new_image_fill(&mut self, image: ImageId, mode: ProjectionMode) {
        let lang = self.lang;
        if self.is_stroking() {
            self.refuse(
                Source::FillLayer,
                crate::lang::refusals::during_stroke(lang),
            );
            return;
        }
        let (name, size) = match self.take_shelf_image(image) {
            Ok(taken) => taken,
            Err(why) => {
                self.fail(Source::FillLayer, why);
                return;
            }
        };
        // 投影の決め方はデカールを置く道・画像の欄で投影を替える道と同じ（`placement::new_projection` と `fitted_placement_for`）。
        // モデルがあれば、UV 以外は今の 3D ビューから見て正面（デカール）かモデルの外形に合わせる
        let fitted = if mode == ProjectionMode::Uv {
            None
        } else {
            self.fitted_placement_for(mode, Some(size))
        };
        let projection = placement::new_projection(mode, fitted);
        self.doc.end_coalescing();
        let created = self.add_fill_layer_with(&name, |d, id, channel| {
            d.set_fill_image(id, channel, Some(image))?;
            d.set_fill_projection(id, projection, false)
        });
        match created {
            Some(id) => {
                self.fillfx.edit_gradient = None;
                if mode != ProjectionMode::Uv {
                    self.fillfx.handles_hidden = false;
                }
                if mode == ProjectionMode::Decal {
                    match self.decal_problem(id) {
                        None => self.info(
                            Source::FillLayer,
                            format!(
                                "{}: {name}",
                                lang.pick("デカールを置きました", "Decal placed")
                            ),
                        ),
                        // 置いたが、まだ出ない（理由つき）: 気をつけること
                        Some(why) => self.warn(
                            Source::FillLayer,
                            lang.pick(
                                format!(
                                    "デカールを置きました。{}はまだ出ません（{why}）。",
                                    lang.quote(&name)
                                ),
                                format!(
                                    "Decal placed. {} is not shown yet ({why}).",
                                    lang.quote(&name)
                                ),
                            ),
                        ),
                    }
                } else {
                    self.info(
                        Source::FillLayer,
                        format!(
                            "{}: {name}",
                            lang.pick("画像の塗りつぶしを追加しました", "Image fill added")
                        ),
                    );
                }
            }
            // 作れなかった画像は手放す（どのレイヤーも指さない画像を、予算に残さない）
            None => self.release_shelf_image(image),
        }
    }

    /// グラデーションデカールの新しい塗りつぶしレイヤー（モデルの外形に合わせた、`shape` の置き場）を作り、3D ビューでその形を編集できるようにする。
    fn new_gradient_fill(&mut self, shape: Shape) {
        let lang = self.lang;
        if self.is_stroking() {
            self.refuse(
                Source::FillLayer,
                crate::lang::refusals::during_stroke(lang),
            );
            return;
        }
        let name = self.new_layer_name(LayerKind::Fill);
        let settings = placement::new_shape_gradient_of(shape, self.model_bounds().as_ref());
        self.doc.end_coalescing();
        let created = self.add_fill_layer_with(&name, |d, id, channel| {
            d.set_fill_gradient(id, channel, Some(settings), false)
        });
        if let Some(id) = created {
            let channel = self.m2.paint_channel;
            self.fillfx.edit_gradient = Some((id, channel));
            self.fillfx.handles_hidden = false;
            self.info(
                Source::FillLayer,
                format!(
                    "{}: {name}",
                    lang.pick(
                        "グラデーションデカールを追加しました",
                        "Gradient decal added"
                    )
                ),
            );
        }
    }
}
