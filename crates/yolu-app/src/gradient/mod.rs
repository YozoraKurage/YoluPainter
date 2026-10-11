//! グラデーションのツール（Shift+G）: 2D のキャンバスか 3D ビューの画面をドラッグして、始点から終点へ、線形か放射で塗る。始点の色は描画色、
//! 終点は透明かサブの色（補間は乗算済みアルファ。core の `GradientSettings`）。選んだレイヤーの描くチャンネル 1 つ、マテリアルで塗るときはその組の
//! 全部（終点を現在のマテリアルにして 2 つのマテリアルの間も）、マスクを描くときはマスクに（白で見せる・黒で隠す）。どれも 1 回の Undo。ロックは
//! core が断る。2D は画素の中心の点で、3D ビュー（`view3d::draft`）は見えているテクセルが写る画面の点で色を決める（式は同じ）。

pub mod canvas;
pub mod props;

use egui::Pos2;
use yolu_core::glam::DVec2;
use yolu_core::material::{ChannelPaint, GradientSettings, GradientShape};
use yolu_core::{LayerKind, Rgba8};

use crate::matpaint::single_value;
use crate::matpaint::MaterialPaint;
use crate::notice::Source;
use crate::state::{to_byte, AppState, Rgba, StrokeSource};

/// 終点の色。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum End {
    /// 透明（始点の色が消えていく）。
    #[default]
    Transparent,
    /// サブの色（背景色）。
    Sub,
}

/// 終点にするマテリアル（「現在のマテリアルを終点に」で写した値）。
#[derive(Clone, Debug, PartialEq)]
pub struct EndMaterial {
    pub paint: MaterialPaint,
    pub color: Rgba,
}

/// ドラッグの途中（キャンバスの座標。左下が原点）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GradientDrag {
    pub source: StrokeSource,
    pub start: (f64, f64),
    pub current: (f64, f64),
    pub start_screen: Pos2,
}

/// グラデーションのツールの設定と途中の状態。
#[derive(Clone, Debug)]
pub struct GradientState {
    pub shape: GradientShape,
    pub end: End,
    /// 消す（マスクでは黒で隠す）か。
    pub erase: bool,
    /// マテリアルで塗るとき、終点を `end_material` にする。
    pub between: bool,
    pub end_material: Option<EndMaterial>,
    pub drag: Option<GradientDrag>,
    /// ペンで押している間のペンの番号。
    pub pen_down: Option<u32>,
}

impl Default for GradientState {
    fn default() -> Self {
        GradientState {
            shape: GradientShape::Linear,
            end: End::Transparent,
            erase: false,
            between: false,
            end_material: None,
            drag: None,
            pen_down: None,
        }
    }
}

/// グラデーションの操作（`Action::Gradient`）。
#[derive(Clone, Debug, PartialEq)]
pub enum GradientOp {
    Shape(GradientShape),
    End(End),
    Erase(bool),
    /// 2 つのマテリアルの間にする。
    Between(bool),
    /// 現在のマテリアルを終点にする。
    CaptureEnd,
    /// キャンバスの座標の始点から終点へ塗る（ドラッグを離したとき。試験も同じ道）。
    Apply {
        start: (f64, f64),
        end: (f64, f64),
    },
}

impl GradientOp {
    /// 文書を変える操作か（読むだけのセットでは断る）。
    pub fn edits_document(&self) -> bool {
        matches!(self, GradientOp::Apply { .. })
    }
}

fn rgba8(c: Rgba) -> Rgba8 {
    Rgba8::new(to_byte(c[0]), to_byte(c[1]), to_byte(c[2]), to_byte(c[3]))
}

impl AppState {
    /// これ以内（キャンバスの画素）しか動かさなければ、ドラッグでなくクリックで、何も塗らない。
    pub const GRADIENT_CLICK: f64 = 1.5;

    /// 始点の色（描画色）と終点の色。
    fn gradient_ends(&self) -> (Rgba8, Rgba8) {
        let from = single_value(self.color.main);
        let to = match self.gradient.end {
            End::Transparent => Rgba8::TRANSPARENT,
            End::Sub => rgba8(self.color.sub),
        };
        (from, to)
    }

    /// グラデーションのツールの操作を当てる。
    pub fn gradient_apply(&mut self, op: GradientOp) {
        let lang = self.lang;
        match op {
            GradientOp::Shape(shape) => self.gradient.shape = shape,
            GradientOp::End(end) => self.gradient.end = end,
            GradientOp::Erase(erase) => self.gradient.erase = erase,
            GradientOp::Between(on) => self.gradient.between = on,
            GradientOp::CaptureEnd => {
                self.gradient.end_material = Some(EndMaterial {
                    paint: self.mat.clone(),
                    color: self.color.main,
                });
                self.gradient.between = true;
                self.info(
                    Source::Gradient,
                    lang.pick(
                        "現在のマテリアルを終点にしました",
                        "The current material is the end",
                    ),
                );
            }
            GradientOp::Apply { start, end } => self.gradient_paint(start, end),
        }
    }

    /// 始点から終点へ塗る（C# の `FinishToolDrag` のグラデーション）。
    fn gradient_paint(&mut self, a: (f64, f64), b: (f64, f64)) {
        if self.is_stroking() {
            self.refuse(
                Source::Gradient,
                crate::lang::refusals::during_stroke(self.lang),
            );
            return;
        }
        if (a.0 - b.0).hypot(a.1 - b.1) < Self::GRADIENT_CLICK {
            return;
        }
        let Some((id, masked)) = self.gradient_target() else {
            return;
        };
        self.gradient_fill(
            id,
            masked,
            (DVec2::new(a.0, a.1), DVec2::new(b.0, b.1)),
            None,
            &|x, y| Some(DVec2::new(x as f64 + 0.5, y as f64 + 0.5)),
        );
    }

    /// グラデーションを塗るレイヤーとマスクを描くか（塗れなければ理由を知らせて None）。
    pub(crate) fn gradient_target(&mut self) -> Option<(yolu_core::LayerId, bool)> {
        let lang = self.lang;
        let Some(id) = self.selected_layer else {
            self.refuse(
                Source::Gradient,
                lang.pick("描くレイヤーがありません。", "No layer to paint on."),
            );
            return None;
        };
        let masked = self.m2.edit_mask;
        let kind = self.doc.layer(id).map(|l| l.kind());
        if !masked && kind != Some(LayerKind::Raster) {
            self.refuse(
                Source::Gradient,
                lang.pick(
                    "ペイントレイヤーかマスクだけにグラデーションを塗れます",
                    "Only a paint layer or a mask takes a gradient",
                ),
            );
            return None;
        }
        Some((id, masked))
    }

    /// 始点から終点（`ends`。`place` の点の空間）へ塗る。region は塗る量（無ければ選択範囲だけ）、place は画素の色を決める点（None の
    /// 画素は変えない。2D は画素の中心、3D はテクセルが写る画面の点）。
    pub(crate) fn gradient_fill(
        &mut self,
        id: yolu_core::LayerId,
        masked: bool,
        (start, end): (DVec2, DVec2),
        region: Option<&yolu_core::SelectionMask>,
        place: &(dyn Fn(u32, u32) -> Option<DVec2> + Sync),
    ) {
        let lang = self.lang;
        let (from, to) = self.gradient_ends();
        let mut settings = GradientSettings {
            shape: self.gradient.shape,
            start,
            end,
            from,
            to,
            opacity: f64::from(self.brush.opacity),
        };
        let erase = self.gradient.erase;
        let revision = self.doc.revision();
        self.doc.end_coalescing();
        let result = if masked {
            // マスクは、始点の黒（隠す量 1）から透明へ。消す（reveal）なら見せる向き
            settings.from = Rgba8::new(0, 0, 0, 255);
            settings.to = Rgba8::TRANSPARENT;
            self.doc
                .gradient_mask_at(id, &settings, region, erase, place)
        } else if self.paints_material() {
            let channels = self.paint_channels();
            let end_paints: Option<Vec<ChannelPaint>> = if self.gradient.between {
                self.gradient.end_material.as_ref().map(|m| {
                    channels
                        .iter()
                        .map(|p| ChannelPaint::new(p.channel, m.paint.value(p.channel, m.color)))
                        .collect()
                })
            } else {
                None
            };
            self.doc.gradient_material_at(
                id,
                &channels,
                end_paints.as_deref(),
                &settings,
                region,
                erase,
                place,
            )
        } else {
            // 描くチャンネル 1 つ。無効のチャンネルは有効にして、1 回の Undo で塗る
            let channel = self.m2.paint_channel;
            self.doc.gradient_material_at(
                id,
                &[ChannelPaint::new(channel, from)],
                Some(&[ChannelPaint::new(channel, to)]),
                &settings,
                region,
                erase,
                place,
            )
        };
        match result {
            Ok(true) => self.info(
                Source::Gradient,
                lang.pick("グラデーションを塗りました", "Gradient applied"),
            ),
            Ok(false) => self.refuse(
                Source::Gradient,
                lang.pick("塗る所がありません", "Nothing to paint there"),
            ),
            Err(e) => self.notify(
                crate::notice::Kind::of_core(&e),
                Source::Gradient,
                lang.core_error(&e),
            ),
        }
        if self.doc.revision() != revision {
            self.modified = true;
        }
    }

    /// ドラッグの途中の形を捨てる（Esc・ツールの切り替え・フォーカスを失ったとき。2D のキャンバスと 3D ビューの両方）。何かあったか。
    pub fn gradient_cancel_drag(&mut self) -> bool {
        self.gradient.pen_down = None;
        let surface = self
            .view3d
            .input
            .draft
            .take_if(|d| d.kind == crate::view3d::draft::DraftKind::Gradient)
            .is_some();
        self.gradient.drag.take().is_some() || surface
    }
}
