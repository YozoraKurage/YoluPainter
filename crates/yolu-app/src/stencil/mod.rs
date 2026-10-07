//! ステンシル（Substance Painter のステンシル。Unity 版の `TexturePaintWindow.Stencil` と同じ振る舞い）: PNG の画像を 2D のキャンバスと
//! 3D のビューの画面に半透明で重ね、ブラシはその上から塗る。画面に貼り付いている（カメラを回しても・キャンバスを動かしても画面の同じ所に
//! ある）。置き場は表示域に対する割合（中心）・表示域の高さに対する大きさ・画面の上の角度で、Y を押したままのドラッグで変える（左 = 回す
//! （Shift で 15° 刻み）、中か Ctrl+左 = 動かす、右か Alt+左 = 大きさ）。N を押しているあいだは効かない。
//!
//! 塗る値は core の `BrushStencil`: 2D はキャンバスの画素の中心を画面へ写し、3D は面のテクセルの点をカメラで画面へ写して（core の
//! `SurfaceStencil`）、そこのステンシルを読む。灰色の画像は量（輝度 × α）、色の画像は色（α が量）。置き場はストロークの始めに決め、
//! ストロークの間は変えない。
//!
//! 保存: ステンシルの状態（画像・読み方・繰り返し・反転・重ねの不透明度・置き場）は文書ではなくアプリの状態で、.ylp には入れない
//! （Unity 版もウィンドウの状態で、.ylp・ブラシの設定・プリセットには入れない）。1 つの操作は 1 つの `StencilOp` で、文書を変えないので Undo の
//! 段にはならない（塗ったストロークは 1 回の Undo）。

mod frame;
mod input;
mod overlay;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui::{Pos2, Rect};
use yolu_core::geometry::SurfaceStencil;
use yolu_core::{
    BrushStencil, Channel, CoreError, ImageColorSpace, StencilImage, StencilMapping, StencilMode,
    StencilTiling,
};

pub use frame::{
    StencilFrame, DEFAULT_CENTER, DEFAULT_OPACITY, DEFAULT_SIZE, MAX_SIZE, MIN_SIZE, ROTATE_STEP,
    SLIDER_MAX_SIZE,
};
pub use input::{handle_event, settle, update_keys, DragKind, StencilDrag};
pub use overlay::draw as draw_overlay;

/// Y を押している・動かしているあいだのポインタ（大きさは斜めの矢印、ほかは動かす形）。画像が無ければ替えない（Unity 版は重ね表示を出す
/// 画像があるときだけポインタの形を替える。Y を押しても何も起きない・ブラシのカーソルを隠さない）。
pub fn cursor_icon(st: &StencilState) -> Option<egui::CursorIcon> {
    if !st.handling() || st.image.is_none() {
        return None;
    }
    Some(match st.drag.map(|d| d.kind) {
        Some(DragKind::Scale) => egui::CursorIcon::ResizeNwSe,
        _ => egui::CursorIcon::Move,
    })
}

use crate::notice::Source;
use crate::state::{AppState, DialogRequest};

/// 重ね表示の元の画像の長い辺（塗る値は元の画像から読む。表示だけ縮める）。
pub const OVERLAY_SIDE: usize = 1024;
/// プロパティの欄の見本の長い辺。
pub const THUMB_SIDE: usize = 64;
/// 覚えておく「読んだ画像」の数。
pub const MAX_RECENT: usize = 8;

/// 縮めた画像（straight RGBA8、行は上から）。
#[derive(Clone, Debug, PartialEq)]
pub struct Preview {
    pub size: [usize; 2],
    pub rgba: Vec<u8>,
}

impl Preview {
    /// 長い辺が side になるまで、プリマルチプライドの面積の平均で縮める（透明な画素の色が縁に滲まない）。小さければそのまま。
    fn shrink(rgba: &[u8], width: usize, height: usize, side: usize) -> Preview {
        if width.max(height) <= side {
            return Preview {
                size: [width, height],
                rgba: rgba.to_vec(),
            };
        }
        let scale = side as f64 / width.max(height) as f64;
        let dw = ((width as f64 * scale).round() as usize).max(1);
        let dh = ((height as f64 * scale).round() as usize).max(1);
        let mut out = vec![0u8; dw * dh * 4];
        for dy in 0..dh {
            let (y0, y1) = (
                dy * height / dh,
                ((dy + 1) * height / dh).max(dy * height / dh + 1),
            );
            for dx in 0..dw {
                let (x0, x1) = (
                    dx * width / dw,
                    ((dx + 1) * width / dw).max(dx * width / dw + 1),
                );
                let (mut a, mut r, mut g, mut b) = (0f64, 0f64, 0f64, 0f64);
                let mut raw = [0f64; 3];
                for y in y0..y1.min(height) {
                    for x in x0..x1.min(width) {
                        let o = (y * width + x) * 4;
                        let al = rgba[o + 3] as f64;
                        a += al;
                        r += al * rgba[o] as f64;
                        g += al * rgba[o + 1] as f64;
                        b += al * rgba[o + 2] as f64;
                        raw[0] += rgba[o] as f64;
                        raw[1] += rgba[o + 1] as f64;
                        raw[2] += rgba[o + 2] as f64;
                    }
                }
                let n = ((y1.min(height) - y0) * (x1.min(width) - x0)).max(1) as f64;
                let o = (dy * dw + dx) * 4;
                if a > 0.0 {
                    out[o] = (r / a).round() as u8;
                    out[o + 1] = (g / a).round() as u8;
                    out[o + 2] = (b / a).round() as u8;
                    out[o + 3] = (a / n).round() as u8;
                } else {
                    // 全部が透明でも、色の平均を残す
                    out[o] = (raw[0] / n).round() as u8;
                    out[o + 1] = (raw[1] / n).round() as u8;
                    out[o + 2] = (raw[2] / n).round() as u8;
                }
            }
        }
        Preview {
            size: [dw, dh],
            rgba: out,
        }
    }
}

/// ステンシルを読む・使う失敗。文はここでは作らず、表示のところで言語ごとに作る（`Lang::stencil_error`）。
#[derive(Debug)]
pub enum StencilError {
    /// core の断り（大きさ・ミップマップの予算・写しが有限でない、など）。
    Core(CoreError),
    /// ファイルを開けない・読めない。
    File(std::io::Error),
    /// PNG として読めない（デコーダーの診断は英語の文なので、画面には出さない）。
    NotPng,
    /// 1 辺が上限を超える（画素を読む前に断る）。
    TooLarge { width: u32, height: u32, side: u32 },
    /// デコーダーの確保の上限を超える。
    Limits,
}

impl From<CoreError> for StencilError {
    fn from(e: CoreError) -> Self {
        Self::Core(e)
    }
}

impl From<image::ImageError> for StencilError {
    fn from(e: image::ImageError) -> Self {
        match e {
            image::ImageError::IoError(e) => Self::File(e),
            image::ImageError::Limits(_) => Self::Limits,
            _ => Self::NotPng,
        }
    }
}

/// 読んだステンシルの画像。
pub struct LoadedStencil {
    /// 読むたびに増える番号（重ね表示と見本の絵を作り直す鍵）。
    pub id: u64,
    pub name: String,
    pub path: Option<PathBuf>,
    pub width: usize,
    pub height: usize,
    /// core の画像（ミップマップつき。ストロークが共有する）。
    pub image: Arc<StencilImage>,
    preview: Preview,
    thumb: Preview,
}

impl LoadedStencil {
    /// 上の行から始まる straight RGBA8 から作る（core の画像は下の行から）。大きさ（1〜8192）・ミップの予算は core が断る。
    fn from_top_down(
        id: u64,
        name: String,
        path: Option<PathBuf>,
        width: usize,
        height: usize,
        rgba: &[u8],
        mip_budget: u64,
    ) -> Result<LoadedStencil, CoreError> {
        if width < 1 || height < 1 || rgba.len() != width * height * 4 {
            return Err(CoreError::InvalidArgument(
                "ステンシルの画像のバイト数が幅 × 高さ × 4 でない",
            ));
        }
        let mut flipped = Vec::with_capacity(rgba.len());
        for row in rgba.chunks_exact(width * 4).rev() {
            flipped.extend_from_slice(row);
        }
        let image = StencilImage::new(width, height, flipped, ImageColorSpace::Srgb, mip_budget)?;
        let preview = Preview::shrink(rgba, width, height, OVERLAY_SIDE);
        let thumb = Preview::shrink(&preview.rgba, preview.size[0], preview.size[1], THUMB_SIDE);
        Ok(LoadedStencil {
            id,
            name,
            path,
            width,
            height,
            image: Arc::new(image),
            preview,
            thumb,
        })
    }

    pub(crate) fn preview(&self) -> &Preview {
        &self.preview
    }

    pub(crate) fn thumb(&self) -> &Preview {
        &self.thumb
    }
}

/// PNG のファイルを straight RGBA8（行は上から）に読む。1 辺が 8192 を超える画像は、画素を読む前に断る。
pub fn read_png(path: &Path) -> Result<(usize, usize, Vec<u8>), StencilError> {
    use image::ImageDecoder;
    let file = std::fs::File::open(path).map_err(StencilError::File)?;
    let mut decoder = image::codecs::png::PngDecoder::new(std::io::BufReader::new(file))?;
    let (w, h) = decoder.dimensions();
    let side = StencilImage::MAX_SIDE as u32;
    if w > side || h > side {
        return Err(StencilError::TooLarge {
            width: w,
            height: h,
            side,
        });
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(side);
    limits.max_image_height = Some(side);
    limits.max_alloc = Some(1 << 30);
    decoder.set_limits(limits)?;
    let img = image::DynamicImage::from_decoder(decoder)?.into_rgba8();
    let (w, h) = (img.width() as usize, img.height() as usize);
    Ok((w, h, img.into_raw()))
}

/// 読んだことのある画像（パスで覚え、選んだら読み直す）。
#[derive(Clone, Debug, PartialEq)]
pub struct RecentImage {
    pub name: String,
    pub path: PathBuf,
}

/// ステンシルの操作（画面だけ。文書を変えないので Undo の段にはならない）。
#[derive(Clone, Debug, PartialEq)]
pub enum StencilOp {
    /// 画像のファイルを選ぶウィンドウを開く。
    Pick,
    /// PNG を読む。
    Load(PathBuf),
    /// 読んだことのある画像の n 番目（新しい順）へ。
    Recent(usize),
    /// 外す（置き場・読み方はそのまま）。
    Clear,
    Mode(StencilMode),
    Tiling(StencilTiling),
    Invert(bool),
    /// 重ね表示の不透明度（0〜1。表示だけ）。
    Opacity(f32),
    /// 表示域の高さに対する画像の高さ。
    Size(f32),
    /// 画面の上の角度（度、時計回りが正）。
    Angle(f32),
    /// 置き場を初めに戻す（中心・大きさ・角度）。
    ResetPlacement,
}

/// ステンシルの状態。
pub struct StencilState {
    pub image: Option<LoadedStencil>,
    pub recent: Vec<RecentImage>,
    pub mode: StencilMode,
    pub tiling: StencilTiling,
    pub invert: bool,
    pub opacity: f32,
    /// 表示域に対する中心（左上が 0, 0、右下が 1, 1）。
    pub center: [f32; 2],
    /// 表示域の高さに対する画像の高さ。
    pub size: f32,
    /// 画面の上の角度（度、時計回りが正、(-180, 180]）。
    pub angle: f32,
    /// Y を押している。
    pub key_held: bool,
    /// N を押していて、ステンシルを使わない。
    pub ignore_held: bool,
    pub drag: Option<StencilDrag>,
    /// ステンシルのミップマップの予算（試験が下げる）。
    pub mip_budget: u64,
    next_id: u64,
    overlay: Option<overlay::OverlayTexture>,
    thumb: Option<overlay::ThumbTexture>,
}

impl Default for StencilState {
    fn default() -> Self {
        StencilState {
            image: None,
            recent: Vec::new(),
            mode: StencilMode::Auto,
            tiling: StencilTiling::None,
            invert: false,
            opacity: DEFAULT_OPACITY,
            center: DEFAULT_CENTER,
            size: DEFAULT_SIZE,
            angle: 0.0,
            key_held: false,
            ignore_held: false,
            drag: None,
            mip_budget: StencilImage::DEFAULT_MIP_BUDGET_BYTES,
            next_id: 0,
            overlay: None,
            thumb: None,
        }
    }
}

fn finite(v: f32, fallback: f32) -> f32 {
    if v.is_finite() {
        v
    } else {
        fallback
    }
}

impl StencilState {
    /// 次のストロークがステンシルを通して塗るか（画像があり、N を押していない）。
    pub fn applies(&self) -> bool {
        self.image.is_some() && !self.ignore_held
    }

    /// Y を押しているか、ステンシルを動かしているあいだ（ストロークを始めない。ブラシのカーソルを隠すのは、画像があるとき: `cursor_icon`）。
    pub fn handling(&self) -> bool {
        self.key_held || self.drag.is_some()
    }

    /// 重ね表示を出すか: 画像があり、N を押していない（Y を押しているあいだは出す）。
    pub fn shown(&self) -> bool {
        self.image.is_some() && (!self.ignore_held || self.handling())
    }

    /// 自動を、この画像で決めた読み方（画像が無ければ None）。
    pub fn resolved_mode(&self) -> Option<StencilMode> {
        self.image
            .as_ref()
            .map(|i| BrushStencil::resolve(self.mode, &i.image))
    }

    /// この表示域（画面の点）での置き場。画像が無い・表示域が空なら None。
    pub fn frame_in(&self, view: Rect) -> Option<StencilFrame> {
        let image = self.image.as_ref()?;
        (view.width() > 0.0 && view.height() > 0.0).then(|| {
            StencilFrame::new(
                view,
                self.center,
                self.size,
                self.angle,
                image.width,
                image.height,
            )
        })
    }

    /// 画像を RGBA8（行は上から）から入れる（ファイルを読んだ後と試験）。置き場はそのまま。ミップが予算を超える画像は断る。
    pub fn set_image_rgba(
        &mut self,
        name: &str,
        width: usize,
        height: usize,
        rgba: &[u8],
    ) -> Result<(), CoreError> {
        self.set_loaded(name, None, width, height, rgba)
    }

    fn set_loaded(
        &mut self,
        name: &str,
        path: Option<PathBuf>,
        width: usize,
        height: usize,
        rgba: &[u8],
    ) -> Result<(), CoreError> {
        let loaded = LoadedStencil::from_top_down(
            self.next_id + 1,
            name.to_owned(),
            path,
            width,
            height,
            rgba,
            self.mip_budget,
        )?;
        self.next_id += 1;
        self.image = Some(loaded);
        self.overlay = None;
        self.thumb = None;
        Ok(())
    }

    fn remember(&mut self, name: &str, path: &Path) {
        self.recent.retain(|r| r.path != path);
        self.recent.insert(
            0,
            RecentImage {
                name: name.to_owned(),
                path: path.to_path_buf(),
            },
        );
        self.recent.truncate(MAX_RECENT);
    }

    /// 置き場を初めに戻す。
    pub fn reset_placement(&mut self) {
        self.center = DEFAULT_CENTER;
        self.size = DEFAULT_SIZE;
        self.angle = 0.0;
    }

    pub fn set_size(&mut self, size: f32) {
        self.size = finite(size, DEFAULT_SIZE).clamp(MIN_SIZE, MAX_SIZE);
    }

    pub fn set_angle(&mut self, degrees: f32) {
        self.angle = crate::canvas::view::normalize_angle(degrees);
    }

    pub fn set_center(&mut self, x: f32, y: f32) {
        self.center = [finite(x, 0.5), finite(y, 0.5)];
    }
}

/// ファイルの名前（拡張子つき）。
fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

impl AppState {
    /// ステンシルの操作を当てる。描いている最中は断る（画像や読み方を替えると、ストロークの途中で値が変わる）。
    pub fn stencil_op(&mut self, op: StencilOp) {
        let lang = self.lang;
        if self.is_stroking() {
            self.refuse(Source::Stencil, crate::lang::refusals::during_stroke(lang));
            return;
        }
        match op {
            StencilOp::Pick => self.dialog_request = Some(DialogRequest::OpenStencil),
            StencilOp::Load(path) => self.load_stencil(&path),
            StencilOp::Recent(i) => {
                if let Some(r) = self.stencil.recent.get(i).cloned() {
                    self.load_stencil(&r.path);
                }
            }
            StencilOp::Clear => {
                self.stencil.image = None;
                self.stencil.overlay = None;
                self.stencil.thumb = None;
                self.stencil.drag = None;
                self.info(
                    Source::Stencil,
                    lang.pick("ステンシルを外しました。", "The stencil was removed."),
                );
            }
            StencilOp::Mode(mode) => self.stencil.mode = mode,
            StencilOp::Tiling(tiling) => self.stencil.tiling = tiling,
            StencilOp::Invert(on) => self.stencil.invert = on,
            StencilOp::Opacity(v) => {
                self.stencil.opacity = finite(v, DEFAULT_OPACITY).clamp(0.0, 1.0)
            }
            StencilOp::Size(v) => self.stencil.set_size(v),
            StencilOp::Angle(v) => self.stencil.set_angle(v),
            StencilOp::ResetPlacement => self.stencil.reset_placement(),
        }
    }

    /// PNG をステンシルにする（読めない・大きすぎる・ミップが予算を超えるときは、今のステンシルのまま理由を出す）。
    fn load_stencil(&mut self, path: &Path) {
        let lang = self.lang;
        let name = file_name(path);
        let result = read_png(path).and_then(|(w, h, rgba)| {
            Ok(self
                .stencil
                .set_loaded(&name, Some(path.to_path_buf()), w, h, &rgba)?)
        });
        match result {
            Ok(()) => {
                self.stencil.remember(&name, path);
                self.info(
                    Source::Stencil,
                    format!("{}: {name}", lang.pick("ステンシル", "Stencil")),
                );
            }
            Err(e) => {
                self.stencil.recent.retain(|r| r.path != path);
                self.fail(
                    Source::Stencil,
                    lang.with_reason(
                        lang.pick(
                            format!("ステンシル{}を読めません", lang.quote(&name)),
                            format!("Cannot load the stencil {}", lang.quote(&name)),
                        ),
                        lang.stencil_error(&e),
                    ),
                );
            }
        }
    }

    /// ステンシルの色を受けるチャンネル（マスクは量だけ）。
    fn stencil_color_channels(&self) -> Vec<Channel> {
        if self.m2.edit_mask {
            Vec::new()
        } else {
            vec![self.m2.paint_channel]
        }
    }

    /// 次の 2D のストロークが通すステンシル（使わなければ None）: キャンバスの画素 → 画面 → 画像の写しを 1 つにして渡す。
    /// rect は今のキャンバスの表示域。
    pub fn canvas_stencil(&self, rect: Rect) -> Result<Option<Arc<BrushStencil>>, CoreError> {
        let st = &self.stencil;
        let Some(image) = st.image.as_ref().filter(|_| st.applies()) else {
            return Ok(None);
        };
        let Some(frame) = st.frame_in(rect) else {
            return Ok(None);
        };
        let view = self.view.view(rect, self.doc.width(), self.doc.height());
        let [a, b, c, d, e, g] = view.screen_affine();
        let [p, q, r, s, t, u] = frame.image_affine();
        let mapping = StencilMapping::new(
            p * a + q * d,
            p * b + q * e,
            p * c + q * g + r,
            s * a + t * d,
            s * b + t * e,
            s * c + t * g + u,
        )?;
        Ok(Some(Arc::new(BrushStencil::new(
            image.image.clone(),
            st.mode,
            st.tiling,
            st.invert,
            Some(mapping),
            &self.stencil_color_channels(),
        ))))
    }

    /// 次の 3D のストロークが通すステンシル（使わなければ None）: ブラシのステンシルと、面のテクセルの点を画面から画像へ写す置き場。
    /// 画面の点は 3D の表示域の左上が原点（カメラ・ストロークの点と同じ）。rect は 3D の表示域。
    pub fn surface_stencil(
        &self,
        rect: Rect,
    ) -> Result<Option<(Arc<BrushStencil>, SurfaceStencil)>, CoreError> {
        let st = &self.stencil;
        let Some(image) = st.image.as_ref().filter(|_| st.applies()) else {
            return Ok(None);
        };
        let local = Rect::from_min_size(Pos2::ZERO, rect.size());
        let Some(frame) = st.frame_in(local) else {
            return Ok(None);
        };
        let [xx, xy, x0, yx, yy, y0] = frame.image_affine();
        let to_image = StencilMapping::new(xx, xy, x0, yx, yy, y0)?;
        let surface = SurfaceStencil::new(to_image, frame.image_per_point())?;
        let brush = BrushStencil::new(
            image.image.clone(),
            st.mode,
            st.tiling,
            st.invert,
            None,
            &self.stencil_color_channels(),
        );
        Ok(Some((Arc::new(brush), surface)))
    }
}
