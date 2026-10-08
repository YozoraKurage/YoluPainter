//! 塗りつぶしレイヤーの画像と投影・デカール・グラデーションデカール（Substance の Fill の画像と Projection、Decal）の画面の状態と操作。
//!
//! - 文書が持つもの（チャンネルごとの画像の参照・レイヤーごとの投影・チャンネルごとの形のグラデーション）は core の `Document` が決め、ここは
//!   選ぶ・渡す・覚えるだけ。どの変更も `Action::Fill` を通る（欄・キー・試験と同じ道）。1 つの操作が 1 回の Undo で、スライダー・数値・ギズモの
//!   ドラッグは `coalesce` で離すまでを 1 回にまとめる。断られたら何も変えず、理由を状態の帯へ。
//! - 画像の画素は棚から、文書の効果の入力へ渡す。復号・予算・失敗の理由・文書への受け渡しは効果の画面と共通で、`fx::inputs` が持つ
//!   （画像を差す操作が先に `use_shelf_image` で頼み、レイヤーが指したあとは毎フレームの `sync_effects` が保つ。差さなかった画像は
//!   `release_shelf_image` で手放す）。ここの `inputs` は棚の画像の ID と色空間の再公開だけ。マップ（位置・法線）とモデルのルートは、
//!   入力を作る側（メッシュマップのベイク）が渡す。入力がそろわない間は、core が値を見せ、その理由を `inactive_effect_list` が言う
//!   （欄は理由を短く出す）。
//! - 置き場（投影の箱・グラデーションの形）は 3D ビューのギズモ（`gizmo`、`view3d::shape_gizmo`）で動かす。決め方は `placement`。

pub mod gizmo;
pub mod inputs;
pub mod placement;
pub mod points;

use std::path::PathBuf;

use egui::{Pos2, Rect};
use yolu_core::fill_image::{FillError, Projection, ProjectionMode, Wrap};
use yolu_core::generator::Settings;
use yolu_core::glam::Vec2;
use yolu_core::{
    Channel, ChannelKind, ImageId, InactiveReason, InactiveTarget, LayerId, LayerKind, Rgba8,
};

use crate::lang::Lang;
use crate::notice::Source;
use crate::state::{AppState, DialogRequest};
use crate::view3d::shape_gizmo::Handle;

/// 塗りつぶしの画面の状態。
#[derive(Default)]
pub struct FillFxState {
    /// 投影の置き場のハンドルを隠している（Q・欄のボタン）。
    pub handles_hidden: bool,
    /// 3D ビューで形を編集している塗りつぶしのグラデーション（レイヤーとチャンネル）。
    pub edit_gradient: Option<(LayerId, Channel)>,
    /// 3D ビューで形を編集している、フィルターの欄の形のグラデーションの Generator（レイヤーとフィルターの段）。
    pub edit_filter: Option<(LayerId, yolu_core::FilterId)>,
    /// ギズモのドラッグの途中。
    pub drag: Option<gizmo::ShapeDrag>,
    /// ポインタの下のハンドル（カーソル用。描くたびに更新）。
    pub hover: Handle,
    /// 点を置く・動かしている塗りつぶしの点のグラデーション（レイヤーとチャンネル）。
    pub edit_points: Option<(LayerId, Channel)>,
    /// 選んでいる点の番号。
    pub point_selected: Option<usize>,
    /// 点のドラッグの途中。
    pub point_drag: Option<points::PointDrag>,
}

/// 塗りつぶしの操作（`Action::Fill`）。
#[derive(Clone, Debug, PartialEq)]
pub enum FillOp {
    /// チャンネルが読む画像を差す・外す（外しても値は残る）。
    Image {
        layer: LayerId,
        channel: Channel,
        image: Option<ImageId>,
    },
    /// チャンネルの画像を異方性のフィルターで読むか（斜めから当てた画像のにじみを減らす。既定は読む）。
    Anisotropic {
        layer: LayerId,
        channel: Channel,
        on: bool,
    },
    /// 投影を置き換える（`coalesce` ならドラッグを 1 回にまとめる）。
    Projection {
        layer: LayerId,
        projection: Box<Projection>,
        coalesce: bool,
    },
    /// 投影の種類を替える。UV から型の上の投影に替えたとき、置き場が初めのままならモデルの外形に合わせる。
    ProjectionMode {
        layer: LayerId,
        mode: ProjectionMode,
    },
    /// 置き場をモデルに合わせる（外形。デカールは今のビューから見て正面）。
    FitPlacement {
        layer: LayerId,
    },
    /// チャンネルのグラデーションを置き換える・外す。
    Gradient {
        layer: LayerId,
        channel: Channel,
        gradient: Option<Box<Settings>>,
        coalesce: bool,
    },
    /// 新しい形のグラデーション（モデルの外形の半分の箱、色の階調つき）を足して 3D ビューで編集できるようにする。
    AddGradient {
        layer: LayerId,
        channel: Channel,
    },
    /// 点のグラデーションを追加する（モデルがあればモデルの空間で 2 点。メインの色とサブの色）。追加したら点の編集に入る。
    AddPoints {
        layer: LayerId,
        channel: Channel,
    },
    /// 点のグラデーションを置き換える・外す（`coalesce` なら欄のドラッグを 1 回にまとめる）。
    Points {
        layer: LayerId,
        channel: Channel,
        points: Option<Box<yolu_core::fill_points::PointGradient>>,
        coalesce: bool,
    },
    /// 選んだ点を消す（最後の 1 つは消さない）。
    DeletePoint,
    /// 3D ビューのモデルへ画像をデカールとして置く（`at` は画面の点、`rect` は 3D ビューの表示域）。
    PlaceDecal {
        image: ImageId,
        at: Pos2,
        rect: Rect,
    },
    /// 棚の画像の読み方（色空間。文書の履歴に入らない）。リニアの画像は、色のチャンネルでは sRGB に直して読み、データのチャンネルでは
    /// どれでも画素の値のまま読む。
    ImageColorSpace {
        image: ImageId,
        space: yolu_core::ImageColorSpace,
    },
    /// 棚へ PNG を取り込む（棚の画像として入る。文書は変えない）。
    ImportImage(PathBuf),
    /// PNG を選ぶウィンドウを開く。
    ImportImageDialog,
    // ── 画面だけ ──
    /// 置き場のハンドルを隠す・出す。
    Handles(bool),
    ToggleHandles,
    /// グラデーションの形を 3D ビューで編集する・やめる。
    EditGradient(Option<(LayerId, Channel)>),
    /// フィルターの欄の形のグラデーションの Generator の形を 3D ビューで編集する・やめる。
    EditFilter(Option<(LayerId, yolu_core::FilterId)>),
    /// 点のグラデーションの点を、3D ビューと 2D のキャンバスで置く・動かす編集に入る・やめる。
    EditPoints(Option<(LayerId, Channel)>),
    /// 点を選ぶ（欄の行・ビューの印）。
    SelectPoint(Option<usize>),
}

impl FillOp {
    /// 文書を変える操作か（読むだけのセットでは断る）。
    pub fn edits_document(&self) -> bool {
        // 画像の読み方は棚の索引を替えるだけで、文書のレイヤーは変えない（読むだけのセットでも替えられる）
        matches!(
            self,
            FillOp::Image { .. }
                | FillOp::Anisotropic { .. }
                | FillOp::Projection { .. }
                | FillOp::ProjectionMode { .. }
                | FillOp::FitPlacement { .. }
                | FillOp::Gradient { .. }
                | FillOp::AddGradient { .. }
                | FillOp::PlaceDecal { .. }
                | FillOp::AddPoints { .. }
                | FillOp::Points { .. }
                | FillOp::DeletePoint
        )
    }
}

/// 形のギズモか点のグラデーションの点をドラッグしている間か（編集のモードの G/R/S の途中も）。ペンの接触は egui のポインタの押下にならないので、パネルが「押していなければまとめを
/// 終える」を毎フレーム行うとき、この間は終えない（終えると 1 フレームごとに別の Undo の段になる）。離す・Esc・フォーカスの喪失は、ドラッグの側が自分で終える。
pub fn dragging(app: &AppState) -> bool {
    gizmo::dragging(app) || points::dragging(app) || crate::objects::transforming(app)
}

/// 画像を差したチャンネルに値が無いとき core が置く既定の値（画像が使えない所に出る）。
pub fn default_fallback(kind: ChannelKind) -> Rgba8 {
    match kind {
        ChannelKind::Normal => Rgba8::new(128, 128, 255, 255),
        ChannelKind::Scalar => Rgba8::new(128, 128, 128, 255),
        ChannelKind::Color => Rgba8::new(255, 255, 255, 255),
    }
}

impl Lang {
    /// 塗りつぶしの投影の値の断り（core の `FillError`）。
    pub fn fill_error(self, e: &FillError) -> String {
        match e {
            FillError::Invalid(what) => self.with_reason(
                self.pick("投影の値が使えません", "Invalid projection"),
                self.pick(
                    (*what).to_owned(),
                    match *what {
                        "投影に有限でない値" | "箱に有限でない値" => {
                            "not a finite number"
                        }
                        "繰り返しは 0.001..10000" => "tiling must be 0.001 to 10000",
                        "オフセット・回転の範囲" => "offset or rotation out of range",
                        "混ぜ幅・減衰の範囲" => "blend or falloff out of range",
                        "減衰はデカールだけ" => "falloff is for decals only",
                        "箱の位置・回転・大きさの範囲" => {
                            "box position, rotation or size out of range"
                        }
                        _ => "out of range",
                    }
                    .to_owned(),
                ),
            ),
            other => self.pick(other.to_string(), "The projection cannot be used".into()),
        }
    }
}

/// レイヤー・チャンネルの効かない理由（無ければ `None`）。画像は `FillImage`、グラデーションは `FillGradient`、デカールは `Decal`。
fn inactive_reason(
    app: &AppState,
    layer: LayerId,
    target: InactiveTarget,
) -> Option<InactiveReason> {
    app.doc
        .inactive_effect_list()
        .into_iter()
        .find(|e| e.layer == layer && e.target == target)
        .map(|e| e.reason)
}

impl AppState {
    /// 画像を使うレイヤーが、今は画像を投影できない理由（短い文。使えれば `None`）。
    pub fn fill_image_problem(&self, layer: LayerId, channel: Channel) -> Option<String> {
        inactive_reason(self, layer, InactiveTarget::FillImage(channel))
            .map(|r| self.lang.inactive_reason(&r))
    }

    /// デカールが今は出ていない理由。
    pub fn decal_problem(&self, layer: LayerId) -> Option<String> {
        inactive_reason(self, layer, InactiveTarget::Decal).map(|r| self.lang.inactive_reason(&r))
    }

    /// 3D のモデルの外形（無ければ `None`）。
    pub fn model_bounds(&self) -> Option<yolu_core::geometry::Bounds> {
        self.view3d.model.as_ref().map(|m| m.geometry.bounds())
    }

    fn fill_refusal(&mut self, e: &yolu_core::CoreError) {
        let text = self.lang.core_error(e);
        self.notify(crate::notice::Kind::of_core(e), Source::FillLayer, text);
    }

    /// 塗りつぶしレイヤーの id（塗りつぶしでなければ理由を出して `None`）。
    fn fill_layer(&mut self, layer: LayerId) -> Option<LayerId> {
        match self.doc.layer(layer).map(|l| l.kind()) {
            Some(LayerKind::Fill) => Some(layer),
            _ => {
                self.refuse(
                    Source::FillLayer,
                    self.lang
                        .pick("塗りつぶしのレイヤーではありません", "Not a fill layer"),
                );
                None
            }
        }
    }

    /// 塗りつぶしの操作を当てる。断られたら何も変えず、理由を状態の帯へ。
    pub fn fill_apply(&mut self, op: FillOp) {
        let lang = self.lang;
        if op.edits_document() && self.is_stroking() {
            self.refuse(
                Source::FillLayer,
                crate::lang::refusals::during_stroke(lang),
            );
            return;
        }
        let revision = self.doc.revision();
        match op {
            FillOp::Image {
                layer,
                channel,
                image,
            } => {
                let Some(layer) = self.fill_layer(layer) else {
                    return;
                };
                if let Some(image) = image {
                    if let Err(why) = self.use_shelf_image(&inputs::resource_id(image)) {
                        self.fail(Source::FillLayer, why);
                        return;
                    }
                }
                self.doc.end_coalescing();
                match self.doc.set_fill_image(layer, channel, image) {
                    Ok(()) => {
                        self.info(
                            Source::FillLayer,
                            match image.and_then(|i| self.shelf.get(&inputs::resource_id(i))) {
                                Some(r) => format!(
                                    "{}: {}",
                                    lang.pick("画像を差しました", "Image set"),
                                    r.name
                                ),
                                None => lang.pick("画像を外しました", "Image removed").into(),
                            },
                        );
                    }
                    Err(e) => {
                        // 画像は先に復号して文書へ渡してある（core はそれを見てから断る）。差さなかった画像は手放す
                        if let Some(image) = image {
                            self.release_shelf_image(image);
                        }
                        self.fill_refusal(&e);
                    }
                }
            }
            FillOp::Anisotropic { layer, channel, on } => {
                let Some(layer) = self.fill_layer(layer) else {
                    return;
                };
                self.doc.end_coalescing();
                if let Err(e) = self.doc.set_fill_anisotropic(layer, channel, on) {
                    self.fill_refusal(&e);
                }
            }
            FillOp::Projection {
                layer,
                projection,
                coalesce,
            } => {
                let Some(layer) = self.fill_layer(layer) else {
                    return;
                };
                let projection = *projection;
                if let Err(e) = projection.validate() {
                    self.fail(Source::FillLayer, lang.fill_error(&e));
                    return;
                }
                if let Err(e) = self.doc.set_fill_projection(layer, projection, coalesce) {
                    self.fill_refusal(&e);
                }
            }
            FillOp::ProjectionMode { layer, mode } => {
                let Some(layer) = self.fill_layer(layer) else {
                    return;
                };
                self.set_projection_mode(layer, mode);
            }
            FillOp::FitPlacement { layer } => {
                let Some(layer) = self.fill_layer(layer) else {
                    return;
                };
                let Some(l) = self.doc.layer(layer) else {
                    return;
                };
                let mut p = *l.projection();
                match self.fitted_placement(layer, p.mode) {
                    Some(placement) => {
                        p.placement = placement;
                        self.doc.end_coalescing();
                        if let Err(e) = self.doc.set_fill_projection(layer, p, false) {
                            self.fill_refusal(&e);
                        }
                    }
                    None => {
                        self.refuse(
                            Source::FillLayer,
                            lang.pick("モデルがありません", "No model"),
                        );
                    }
                }
            }
            FillOp::Gradient {
                layer,
                channel,
                gradient,
                coalesce,
            } => {
                let Some(layer) = self.fill_layer(layer) else {
                    return;
                };
                if !coalesce {
                    self.doc.end_coalescing();
                }
                let gradient = gradient.map(|g| *g);
                let clear = gradient.is_none();
                if let Err(e) = self
                    .doc
                    .set_fill_gradient(layer, channel, gradient, coalesce)
                {
                    self.fill_refusal(&e);
                } else if clear && self.fillfx.edit_gradient == Some((layer, channel)) {
                    self.fillfx.edit_gradient = None;
                }
            }
            FillOp::AddGradient { layer, channel } => {
                let Some(layer) = self.fill_layer(layer) else {
                    return;
                };
                let settings = placement::new_shape_gradient(self.model_bounds().as_ref());
                self.doc.end_coalescing();
                match self
                    .doc
                    .set_fill_gradient(layer, channel, Some(settings), false)
                {
                    Ok(()) => {
                        self.fillfx.edit_gradient = Some((layer, channel));
                        self.fillfx.handles_hidden = false;
                    }
                    Err(e) => self.fill_refusal(&e),
                }
            }
            FillOp::AddPoints { layer, channel } => {
                let Some(layer) = self.fill_layer(layer) else {
                    return;
                };
                let g = points::new_gradient(self, channel);
                self.doc.end_coalescing();
                match self.doc.set_fill_points(layer, channel, Some(g), false) {
                    Ok(()) => {
                        self.fillfx.edit_points = Some((layer, channel));
                        self.fillfx.point_selected = None;
                    }
                    Err(e) => self.fill_refusal(&e),
                }
            }
            FillOp::Points {
                layer,
                channel,
                points: g,
                coalesce,
            } => {
                let Some(layer) = self.fill_layer(layer) else {
                    return;
                };
                if !coalesce {
                    self.doc.end_coalescing();
                }
                let removed = g.is_none();
                match self
                    .doc
                    .set_fill_points(layer, channel, g.map(|g| *g), coalesce)
                {
                    Ok(()) => {
                        if removed {
                            if self.fillfx.edit_points == Some((layer, channel)) {
                                self.fillfx.edit_points = None;
                            }
                            self.fillfx.point_selected = None;
                        }
                    }
                    Err(e) => self.fill_refusal(&e),
                }
            }
            FillOp::DeletePoint => points::delete_selected(self),
            FillOp::PlaceDecal { image, at, rect } => self.place_decal(image, at, rect),
            FillOp::ImportImage(path) => {
                // 棚を変える操作は、別のスレッドの保存が終わるまで断る（保存の結果は足した後の棚で丸ごと差し替えるので、
                // その間に取り込むと取り込んだ画像が消え、その画像を差したレイヤーが棚に無い画像を指す）
                if self.shelf_refuse_while_saving() {
                    return;
                }
                self.import_image_file(&path)
            }
            FillOp::ImageColorSpace { image, space } => {
                if self.is_stroking() {
                    self.refuse(
                        Source::FillLayer,
                        crate::lang::refusals::during_stroke(lang),
                    );
                    return;
                }
                // 読み方は棚の索引に書くので、取り込みと同じく保存中は断る
                if self.shelf_refuse_while_saving() {
                    return;
                }
                let id = inputs::resource_id(image);
                let text = match space {
                    yolu_core::ImageColorSpace::Srgb => "srgb",
                    yolu_core::ImageColorSpace::Linear => "linear",
                    yolu_core::ImageColorSpace::Unspecified => "unspecified",
                };
                match self.shelf.set_image_color_space(lang, &id, text) {
                    Ok(true) => {
                        self.modified = true;
                        // 読み方を読み替えるのは、復号している画像だけ（使っていない画像を復号して、予算に残さない）
                        if self.fx.inputs.has_decoded_image(image) {
                            if let Err(why) = self.use_shelf_image(&id) {
                                self.fail(Source::FillLayer, why);
                            }
                        }
                    }
                    Ok(false) => {}
                    Err(why) => self.fail(Source::FillLayer, why),
                }
            }
            FillOp::ImportImageDialog => {
                if !self.shelf_refuse_while_saving() {
                    self.dialog_request = Some(DialogRequest::FillImage)
                }
            }
            FillOp::Handles(hidden) => {
                if hidden
                    && self
                        .fillfx
                        .drag
                        .as_ref()
                        .is_some_and(|d| matches!(d.target, gizmo::Target::Projection(_)))
                {
                    gizmo::release(self, false);
                }
                if !hidden
                    && (self.fillfx.edit_gradient.is_some() || self.fillfx.edit_filter.is_some())
                {
                    gizmo::release(self, false);
                    self.fillfx.edit_gradient = None;
                    self.fillfx.edit_filter = None;
                }
                self.fillfx.handles_hidden = hidden;
            }
            FillOp::ToggleHandles => {
                let hidden = !self.fillfx.handles_hidden;
                self.fill_apply(FillOp::Handles(hidden));
                return;
            }
            FillOp::EditGradient(target) => {
                if self.fillfx.edit_gradient != target {
                    gizmo::release(self, false);
                }
                self.fillfx.edit_gradient = target;
                if let Some((layer, channel)) = target {
                    self.fillfx.edit_filter = None; // ギズモは 1 つ
                    self.selected_layer = Some(layer);
                    self.set_edit_mask(false);
                    self.fillfx.handles_hidden = false;
                    // 編集のモードでは、その形を選ぶ（取っ手は選んだ物に出る）
                    crate::objects::select_when_editing(
                        self,
                        crate::objects::Object::Shape(gizmo::Target::Gradient(layer, channel)),
                    );
                }
            }
            FillOp::EditFilter(target) => {
                if self.fillfx.edit_filter != target {
                    gizmo::release(self, false);
                }
                self.fillfx.edit_filter = target;
                if let Some((layer, filter)) = target {
                    self.fillfx.edit_gradient = None;
                    self.selected_layer = Some(layer);
                    crate::objects::select_when_editing(
                        self,
                        crate::objects::Object::Shape(gizmo::Target::Filter(layer, filter)),
                    );
                }
            }
            FillOp::EditPoints(target) => {
                if self.fillfx.edit_points != target {
                    points::release(self, true);
                    self.fillfx.point_selected = None;
                }
                self.fillfx.edit_points = target;
                // 編集のモードでは、最初の点を選ぶ（モデルの空間の点だけ。点は 3D ビューの印で動かす）
                if let Some((layer, channel)) = target {
                    crate::objects::select_when_editing(
                        self,
                        crate::objects::Object::Point {
                            layer,
                            channel,
                            index: 0,
                        },
                    );
                }
            }
            FillOp::SelectPoint(index) => {
                self.fillfx.point_selected = index;
            }
        }
        if self.doc.revision() != revision {
            self.modified = true;
        }
    }

    /// 投影の種類を替える（C# の `SetProjectionMode`）。デカールは画像を 1 回（繰り返さない）、UV から型の上の投影に替えたとき置き場が
    /// 初めのままならモデルの外形に合わせる。置き場のハンドルをすぐ動かせるように出す。
    fn set_projection_mode(&mut self, layer: LayerId, mode: ProjectionMode) {
        let Some(l) = self.doc.layer(layer) else {
            return;
        };
        let p = *l.projection();
        if p.mode == mode {
            return;
        }
        let mut next = Projection { mode, ..p };
        if mode == ProjectionMode::Decal && p.wrap == Wrap::Repeat && p.tiles == [1.0, 1.0] {
            next.wrap = Wrap::None;
        }
        if mode != ProjectionMode::Uv && p.placement == yolu_core::fill_image::Placement::default()
        {
            if let Some(placement) = self.fitted_placement(layer, mode) {
                next.placement = placement;
            }
        }
        // 球でない投影へ替えるときの減衰は、デカールだけが持つ（core が断る）。デカールを出るなら既定の減衰へ戻す
        if mode != ProjectionMode::Decal {
            let d = Projection::default();
            next.depth_hardness = d.depth_hardness;
            next.backface_angle = d.backface_angle;
            next.backface_hardness = d.backface_hardness;
        }
        self.doc.end_coalescing();
        match self.doc.set_fill_projection(layer, next, false) {
            Ok(()) => {
                if mode != ProjectionMode::Uv {
                    self.fillfx.handles_hidden = false;
                }
            }
            Err(e) => self.fill_refusal(&e),
        }
    }

    /// モデルの外形に合わせた置き場（モデルが無ければ `None`）。デカールは今の 3D ビューのカメラから見て正面。
    pub fn fitted_placement(
        &self,
        layer: LayerId,
        mode: ProjectionMode,
    ) -> Option<yolu_core::fill_image::Placement> {
        let image = self.doc.layer(layer).and_then(|l| {
            l.fill_images().find_map(|(_, id)| {
                self.doc
                    .effect_inputs()
                    .image(id)
                    .map(|i| (i.width, i.height))
            })
        });
        self.fitted_placement_for(mode, image)
    }

    /// `fitted_placement` の、まだレイヤーが無い（これから作る）ときの形。`image` は差す画像の大きさ（デカールの縦横比に使う）。
    pub fn fitted_placement_for(
        &self,
        mode: ProjectionMode,
        image: Option<(u32, u32)>,
    ) -> Option<yolu_core::fill_image::Placement> {
        let bounds = self.model_bounds()?;
        if mode != ProjectionMode::Decal {
            return Some(placement::fit_to_bounds(&bounds, mode));
        }
        let rotation = self.view3d.camera.rotation();
        Some(placement::decal_fit(
            &bounds,
            rotation * yolu_core::glam::Vec3::Z,
            rotation * yolu_core::glam::Vec3::Y,
            image,
        ))
    }

    /// 棚の画像を塗りつぶしレイヤーが指すために取る: 名前と大きさ（デカールを置く道と、メニューが画像・デカールのレイヤーを作る道が同じに使う）。
    /// 棚に無い・復号できない（予算など）ときは理由。取ったあとで読めなければ手放してから返す（どのレイヤーも指さない画像を予算に残さない）。
    pub(crate) fn take_shelf_image(
        &mut self,
        image: ImageId,
    ) -> Result<(String, (u32, u32)), String> {
        let lang = self.lang;
        let rid = inputs::resource_id(image);
        let Some(name) = self.shelf.get(&rid).map(|r| r.name.clone()) else {
            return Err(lang
                .pick(
                    "アセットに画像がありません",
                    "No such image in the project's assets",
                )
                .into());
        };
        self.use_shelf_image(&rid)?;
        match self
            .doc
            .effect_inputs()
            .image(image)
            .map(|i| (i.width, i.height))
        {
            Some(size) => Ok((name, size)),
            None => {
                self.release_shelf_image(image);
                Err(lang
                    .pick("画像を読めません", "Cannot read the image")
                    .into())
            }
        }
    }

    /// 3D ビューの点の面に、棚の画像のデカールを置く: 選んだレイヤーの上に、今のチャンネルにその画像（と画像を使えないときの値）を持つ
    /// 塗りつぶしレイヤーを作り、投影をデカールにして面に向ける（1 回の Undo）。置けない（モデルの外・ほかのテクスチャセットの面）ときは
    /// 何も変えずに理由を出す。
    fn place_decal(&mut self, image: ImageId, at: Pos2, rect: Rect) {
        let lang = self.lang;
        let Some((model, material)) = self.region_model() else {
            self.refuse(Source::FillLayer, self.region_missing_reason());
            return;
        };
        let view = self.view3d.camera.view(rect.width(), rect.height());
        let gui = Vec2::new(at.x - rect.left(), at.y - rect.top());
        let Some(hit) = yolu_core::geometry::pick(&model.geometry, &view, gui) else {
            self.refuse(
                Source::FillLayer,
                lang.pick("モデルの上ではありません", "Not on the model"),
            );
            return;
        };
        if hit.material != material {
            let name = model.material_name(hit.material as usize, lang);
            self.refuse(
                Source::FillLayer,
                lang.pick(
                    format!("ほかのテクスチャセット（{name}）の面です。"),
                    format!("Surface of another texture set ({name})."),
                ),
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
        let channel = self.m2.paint_channel;
        let kind = self
            .doc
            .channel_info(channel)
            .map_or(ChannelKind::Color, |c| c.kind);
        let placed = placement::decal_at(&hit, &view, gui, size);
        let projection = placement::new_projection(ProjectionMode::Decal, Some(placed));
        let above = self
            .selected_layer
            .filter(|id| self.doc.layer(*id).is_some());
        self.doc.end_coalescing();
        let result = self.doc.batch(|d| {
            let id = d.add_fill_layer(&name, &[(channel, default_fallback(kind))], above)?;
            d.set_fill_image(id, channel, Some(image))?;
            d.set_fill_projection(id, projection, false)?;
            Ok(id)
        });
        match result {
            Ok(id) => {
                self.selected_layer = Some(id);
                self.set_edit_mask(false);
                self.fillfx.handles_hidden = false;
                self.fillfx.edit_gradient = None;
                crate::objects::select_when_editing(
                    self,
                    crate::objects::Object::Shape(gizmo::Target::Projection(id)),
                );
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
            }
            Err(e) => {
                self.release_shelf_image(image);
                self.fill_refusal(&e);
            }
        }
    }

    /// PNG のファイルを棚の画像として取り込む（文書は変えない）。大きすぎる・読めない・棚がいっぱいのときは理由を出す。棚にもう同じ中身が
    /// あれば、足さずにその画像を選ぶだけ（棚も文書も「変えた」にしない）。
    fn import_image_file(&mut self, path: &std::path::Path) {
        let lang = self.lang;
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("image")
            .to_owned();
        let (mut rgba, w, h) = match read_png(path, lang) {
            Ok(read) => read,
            Err(why) => {
                self.fail(
                    Source::FillLayer,
                    lang.with_reason(cannot_add_image(lang, &name), why),
                );
                return;
            }
        };
        // 文書と同じ向き（下の行が先）で渡す（読んだ画素の並びをその場で返す。大きな画像の写しを重ねない）
        flip_rows(&mut rgba, w as usize * 4);
        let before = self.shelf.resources().len();
        match self.shelf.add_image(lang, &name, &rgba, w, h) {
            Ok(id) => {
                let added = self.shelf.resources().len() > before;
                self.shelf.selected = Some(id);
                if added {
                    self.modified = true;
                }
                self.info(
                    Source::FillLayer,
                    format!(
                        "{}: {name}",
                        if added {
                            lang.pick(
                                "アセットに画像を取り込みました",
                                "Image added to the project's assets",
                            )
                        } else {
                            lang.pick(
                                "すでにアセットにあります",
                                "Already in the project's assets",
                            )
                        }
                    ),
                );
            }
            Err(why) => self.fail(
                Source::FillLayer,
                lang.with_reason(cannot_add_image(lang, &name), why),
            ),
        }
    }
}

/// 「「名前」をアセットに取り込めません」（理由は `Lang::with_reason` で添える）。
fn cannot_add_image(lang: Lang, name: &str) -> String {
    let name = lang.quote(name);
    lang.pick(
        format!("{name}をアセットに取り込めません"),
        format!("Cannot import {name}"),
    )
}

/// 取り込める画像の 1 辺の上限（core の画像の入力と同じ）。
const MAX_IMAGE_SIDE: u32 = 8192;

/// PNG を straight RGBA8（上の行が先）で読む。寸法はヘッダーで先に見て、画素を読む前に断る（大きすぎる画像で全体を展開しない）。
/// 読める上限（1 辺と確保）は `Limits` でも守る。
fn read_png(path: &std::path::Path, lang: Lang) -> Result<(Vec<u8>, u32, u32), String> {
    use image::ImageDecoder;
    let unreadable = || {
        lang.pick("PNG として読めません", "Not a readable PNG")
            .to_owned()
    };
    let file = std::fs::File::open(path).map_err(|e| lang.file_error(&e))?;
    let mut decoder = image::codecs::png::PngDecoder::new(std::io::BufReader::new(file))
        .map_err(|_| unreadable())?;
    let (w, h) = decoder.dimensions();
    if !(1..=MAX_IMAGE_SIDE).contains(&w) || !(1..=MAX_IMAGE_SIDE).contains(&h) {
        return Err(lang
            .pick(
                "画像の 1 辺は 1〜8192 です",
                "Image sides must be 1 to 8192",
            )
            .to_owned());
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_SIDE);
    limits.max_image_height = Some(MAX_IMAGE_SIDE);
    limits.max_alloc = Some(1 << 30);
    decoder.set_limits(limits).map_err(|_| unreadable())?;
    let img = image::DynamicImage::from_decoder(decoder)
        .map_err(|_| unreadable())?
        .into_rgba8();
    Ok((img.into_raw(), w, h))
}

/// 行の並びを上下逆にする（1 行は `stride` バイト）。
fn flip_rows(raw: &mut [u8], stride: usize) {
    let rows = raw.len() / stride;
    for y in 0..rows / 2 {
        let (head, tail) = raw.split_at_mut((rows - 1 - y) * stride);
        head[y * stride..(y + 1) * stride].swap_with_slice(&mut tail[..stride]);
    }
}

/// 棚の画像を読んでいるレイヤーの名前（全部のテクスチャセット。セットが 2 つ以上ならセットの名前を前に付ける）。棚から消す前の確かめに使う。
pub fn image_users(app: &AppState, resource_id: &str) -> Vec<String> {
    let Some(image) = inputs::image_id(resource_id) else {
        return Vec::new();
    };
    let named = app.sets.len() > 1;
    let mut users = Vec::new();
    for (i, set) in app.sets.iter().enumerate() {
        for layer in app.set_doc(i).layers() {
            if layer.image_ids().any(|id| id == image)
                || yolu_core::paths::list_images(layer.paths()).contains(&image)
            {
                users.push(if named {
                    format!("{}: {}", set.name, layer.name())
                } else {
                    layer.name().to_owned()
                });
            }
        }
    }
    users
}

/// 3D ビューの上で棚の画像を引いているとき、落とす所の印（ポインタの下の面に当たるなら輪）を描き、離したらデカールを置く。画像でない
/// 素材は何もしない（レイヤーの一覧が受け取る）。
pub fn decal_drop(ui: &egui::Ui, app: &mut AppState, content: Rect) {
    let ctx = ui.ctx();
    let Some(drag) = egui::DragAndDrop::payload::<crate::shelf::ShelfDrag>(ctx) else {
        return;
    };
    if drag.kind != crate::shelf::ItemKind::Image {
        return;
    }
    let Some(at) = ctx.pointer_latest_pos().filter(|p| content.contains(*p)) else {
        return;
    };
    let painter = ui.painter_at(content);
    let on_model = app.region_model().is_some_and(|(model, _)| {
        let view = app.view3d.camera.view(content.width(), content.height());
        yolu_core::geometry::pick(
            &model.geometry,
            &view,
            Vec2::new(at.x - content.left(), at.y - content.top()),
        )
        .is_some()
    });
    let color = if on_model {
        egui::Color32::WHITE
    } else {
        crate::ui::theme::TEXT_DISABLED
    };
    painter.circle_stroke(
        at,
        14.0,
        egui::Stroke::new(3.0, egui::Color32::from_black_alpha(140)),
    );
    painter.circle_stroke(at, 14.0, egui::Stroke::new(1.5, color));
    if ui.input(|i| i.pointer.any_released()) {
        if let Some(image) = inputs::image_id(&drag.id) {
            app.apply(crate::state::Action::Fill(FillOp::PlaceDecal {
                image,
                at,
                rect: content,
            }));
        }
        egui::DragAndDrop::clear_payload(ctx);
    }
}
