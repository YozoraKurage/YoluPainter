//! ステンシル（C# の BrushStencil.cs。Substance Painter のステンシル: 画面に重ねた画像を通して塗る）。
//!
//! - 量（マスク）のモードは輝度 × α（反転は (1 − 輝度) × α）、色のモードは画像の色をそのチャンネルの値として塗り α が量。
//!   量はダブの天井（不透明度）に掛ける（紙の質感と同じく、重なったダブで量を越えない）。
//! - 画像の外（繰り返さない軸）は塗らない。読みは双線形、1 画素あたりの画像の画素（footprint）が 1 を超えればミップマップの間の三線形。
//! - ミップマップは塗りつぶしの画像と同じ作り方（各段は前の段を半分に、プリマルチプライドの面積の平均）。
//! - 色のチャンネルへ色を塗るときの読み方も塗りつぶしの画像と同じ（C# の FillImageColor）: 色のチャンネルではリニアの画像を sRGB に、
//!   スカラーのチャンネルは輝度、Normal は保存された値のまま。

use std::sync::{Arc, OnceLock};

use rayon::prelude::*;

use crate::error::CoreError;
use crate::math::{require_finite, to_byte};
use crate::types::{Channel, ChannelKind, Rgba8};

/// ステンシルの画素の効き方（C# の StencilMode）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum StencilMode {
    /// 灰色の画像（どの画素も R = G = B）は量、ほかは色。
    #[default]
    Auto,
    /// 塗る量: 輝度 × α（白が通し、黒と透明は止める）。
    Mask,
    /// 塗る色（ステンシルが名指すチャンネルへ）。α が量。
    Color,
}

/// 画像の端の外で、どの軸に繰り返すか（C# の StencilTiling）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum StencilTiling {
    #[default]
    None,
    Horizontal,
    Vertical,
    Both,
}

/// 画像の色空間（C# の ResourceColorSpace）。リニアの画像を色のチャンネルへ塗るときは sRGB に直す。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ImageColorSpace {
    #[default]
    Unspecified,
    Srgb,
    Linear,
}

/// ミップマップ（C# の ImageMipChain の、変換なし・輝度なしの形）。0 段は画像そのもの。
struct MipChain {
    widths: Vec<usize>,
    heights: Vec<usize>,
    data: Vec<Arc<Vec<u8>>>,
    bytes: u64,
}

impl MipChain {
    fn level_count(mut width: usize, mut height: usize) -> usize {
        let mut n = 1;
        while width > 1 || height > 1 {
            width = (width / 2).max(1);
            height = (height / 2).max(1);
            n += 1;
        }
        n
    }

    /// width × height の画像の 1 段目より後のバイト。
    fn extra_bytes(mut width: usize, mut height: usize) -> u64 {
        let mut sum = 0u64;
        while width > 1 || height > 1 {
            width = (width / 2).max(1);
            height = (height / 2).max(1);
            sum += (width * height * 4) as u64;
        }
        sum
    }

    fn build(width: usize, height: usize, pixels: Arc<Vec<u8>>) -> MipChain {
        let levels = Self::level_count(width, height);
        let mut chain = MipChain {
            widths: vec![width],
            heights: vec![height],
            data: vec![pixels],
            bytes: 0,
        };
        for k in 1..levels {
            let (w, h) = (
                (chain.widths[k - 1] / 2).max(1),
                (chain.heights[k - 1] / 2).max(1),
            );
            chain.widths.push(w);
            chain.heights.push(h);
            let next = chain.halve(k - 1);
            chain.bytes += next.len() as u64;
            chain.data.push(Arc::new(next));
        }
        chain
    }

    #[inline]
    fn read(&self, level: usize, x: usize, y: usize) -> (u8, u8, u8, u8) {
        let d = &self.data[level];
        let o = (y * self.widths[level] + x) * 4;
        (d[o], d[o + 1], d[o + 2], d[o + 3])
    }

    /// 半分にした軸の i 番目が読む元の画素: 2 つ、奇数の辺の最後は 3 つ、辺が 1 なら 1 つ。
    fn span(i: usize, n: usize, halved: usize) -> (usize, usize) {
        if n == 1 {
            return (0, 1);
        }
        (2 * i, if i == halved - 1 && n & 1 == 1 { 3 } else { 2 })
    }

    /// 段 level を半分にした段（C# の Halve。行ごとにワーカーで。各画素は自分の元の画素だけで決まる）。
    fn halve(&self, level: usize) -> Vec<u8> {
        let (w, h) = (self.widths[level], self.heights[level]);
        let (nw, nh) = (self.widths[level + 1], self.heights[level + 1]);
        let mut result = vec![0u8; nw * nh * 4];
        result
            .par_chunks_mut(nw * 4)
            .enumerate()
            .for_each(|(j, out)| {
                let (y0, ny) = Self::span(j, h, nh);
                for i in 0..nw {
                    let (x0, nx) = Self::span(i, w, nw);
                    let weight = 1.0 / (nx * ny) as f64;
                    let (mut a, mut cr, mut cg, mut cb) = (0.0, 0.0, 0.0, 0.0);
                    let (mut zw, mut zr, mut zg, mut zb) = (0.0, 0.0, 0.0, 0.0);
                    let mut same = true;
                    let mut first = (0u8, 0u8, 0u8, 0u8);
                    for v in 0..ny {
                        for u in 0..nx {
                            let p = self.read(level, x0 + u, y0 + v);
                            if u == 0 && v == 0 {
                                first = p;
                            } else if same && p != first {
                                same = false;
                            }
                            let (r, g, b, al) = (p.0 as f64, p.1 as f64, p.2 as f64, p.3);
                            if al == 0 {
                                zw += weight;
                                zr += weight * r;
                                zg += weight * g;
                                zb += weight * b;
                                continue;
                            }
                            let k = weight * al as f64;
                            a += k;
                            cr += k * r;
                            cg += k * g;
                            cb += k * b;
                        }
                    }
                    let o = &mut out[i * 4..i * 4 + 4];
                    if same {
                        o.copy_from_slice(&[first.0, first.1, first.2, first.3]);
                        continue;
                    }
                    let alpha = to_byte(a / 255.0);
                    if alpha == 0 {
                        // 透明でも、読んだ透明な画素の色の平均を残す
                        if zw > 0.0 {
                            o[0] = to_byte(zr / zw / 255.0);
                            o[1] = to_byte(zg / zw / 255.0);
                            o[2] = to_byte(zb / zw / 255.0);
                        }
                        continue;
                    }
                    o[0] = to_byte(cr / a / 255.0);
                    o[1] = to_byte(cg / a / 255.0);
                    o[2] = to_byte(cb / a / 255.0);
                    o[3] = alpha;
                }
            });
        result
    }
}

/// ステンシルの画像（C# の StencilImage）: straight RGBA8・左下原点の画素、色空間、灰色か、ミップマップ。作った後は変わらない
/// （ワーカーのストロークも読む）。ミップマップは画像のバイトの 3 分の 1 ほどで、予算を超えれば断る。
pub struct StencilImage {
    width: usize,
    height: usize,
    color_space: ImageColorSpace,
    is_grey: bool,
    mips: MipChain,
}

impl std::fmt::Debug for StencilImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "StencilImage({}×{}, {:?}, 灰色 {}, ミップ {} 段)",
            self.width,
            self.height,
            self.color_space,
            self.is_grey,
            self.mips.widths.len()
        )
    }
}

/// 画像の 1 点の読み（C# の StencilTexel）: 点が画像の上か、α（0〜1）、輝度 × α（0〜1）、straight の色。
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct StencilTexel {
    pub inside: bool,
    pub alpha: f64,
    pub luma_alpha: f64,
    pub color: Rgba8,
}

impl StencilImage {
    /// ミップマップの既定の予算（C# の塗りつぶしの画像のキャッシュの予算 256 MiB）。
    pub const DEFAULT_MIP_BUDGET_BYTES: u64 = 256 * 1024 * 1024;
    /// 画像の原点からこれ以上離れた点は、繰り返していても読まない（カメラの後ろなどの「ここには無い」の印）。
    pub const FAR_AWAY: f64 = 1e8;
    /// 1 辺の上限（C# の ImageContent.MaxSide）。
    pub const MAX_SIDE: usize = 8192;

    /// 幅・高さ（1〜8192）と straight RGBA8（行は下から）。ミップマップが mip_budget_bytes を超えるなら断る。
    pub fn new(
        width: usize,
        height: usize,
        rgba: Vec<u8>,
        color_space: ImageColorSpace,
        mip_budget_bytes: u64,
    ) -> Result<StencilImage, CoreError> {
        if width < 1 || height < 1 || width > Self::MAX_SIDE || height > Self::MAX_SIDE {
            return Err(CoreError::InvalidArgument(
                "ステンシルの画像の大きさ（1〜8192）",
            ));
        }
        if rgba.len() != width * height * 4 {
            return Err(CoreError::InvalidArgument(
                "ステンシルの画像のバイト数が幅 × 高さ × 4 でない",
            ));
        }
        if MipChain::extra_bytes(width, height) > mip_budget_bytes {
            return Err(CoreError::InvalidArgument(
                "ステンシルのミップマップが予算を超える（小さい画像にする）",
            ));
        }
        let is_grey = rgba.chunks_exact(4).all(|p| p[0] == p[1] && p[0] == p[2]);
        Ok(StencilImage {
            width,
            height,
            color_space,
            is_grey,
            mips: MipChain::build(width, height, Arc::new(rgba)),
        })
    }
    pub fn width(&self) -> usize {
        self.width
    }
    pub fn height(&self) -> usize {
        self.height
    }
    pub fn color_space(&self) -> ImageColorSpace {
        self.color_space
    }
    /// どの画素も R = G = B か（Auto が量と読む）。
    pub fn is_grey(&self) -> bool {
        self.is_grey
    }
    /// 1 段目より後のミップマップのバイト。
    pub fn mip_bytes(&self) -> u64 {
        self.mips.bytes
    }

    /// 画像の画素の単位の (x, y)（x は左から、y は下から。画素 (i, j) は [i, i + 1) × [j, j + 1)）を、描く 1 画素あたり footprint 画像の画素で
    /// 読む（C# の StencilImage.Read）: 双線形、footprint が 1 を超えればミップマップの段の間の三線形。繰り返さない軸で画像の外は読まない。
    // 否定の比較は C# と同じく NaN を「外」に倒すため（partial_cmp にしない）
    #[allow(clippy::neg_cmp_op_on_partial_ord)]
    pub fn read(&self, x: f64, y: f64, footprint: f64, tiling: StencilTiling) -> StencilTexel {
        let repeat_x = matches!(tiling, StencilTiling::Horizontal | StencilTiling::Both);
        let repeat_y = matches!(tiling, StencilTiling::Vertical | StencilTiling::Both);
        if x.is_nan()
            || y.is_nan()
            || footprint.is_nan()
            || !(x.abs() < Self::FAR_AWAY)
            || !(y.abs() < Self::FAR_AWAY)
        {
            return StencilTexel::default();
        }
        if !(x >= 0.0 && x <= self.width as f64) && !repeat_x
            || !(y >= 0.0 && y <= self.height as f64) && !repeat_y
        {
            return StencilTexel::default();
        }
        let mut acc = Acc::default();
        let (u0, v0) = (x - 0.5, y - 0.5);
        let last = self.mips.widths.len() - 1;
        if !(footprint > 1.0) || last == 0 {
            self.bilinear(0, u0, v0, 1.0, repeat_x, repeat_y, &mut acc);
        } else {
            let lod = footprint.ln() * std::f64::consts::LOG2_E; // C# の 1.4426950408889634 と同じ double
            if lod >= last as f64 {
                self.bilinear(
                    last,
                    self.level(last, u0, true),
                    self.level(last, v0, false),
                    1.0,
                    repeat_x,
                    repeat_y,
                    &mut acc,
                );
            } else {
                let k = lod as usize;
                let f = lod - k as f64;
                self.bilinear(
                    k,
                    self.level(k, u0, true),
                    self.level(k, v0, false),
                    1.0 - f,
                    repeat_x,
                    repeat_y,
                    &mut acc,
                );
                if f > 0.0 {
                    self.bilinear(
                        k + 1,
                        self.level(k + 1, u0, true),
                        self.level(k + 1, v0, false),
                        f,
                        repeat_x,
                        repeat_y,
                        &mut acc,
                    );
                }
            }
        }
        acc.resolve()
    }

    fn level(&self, level: usize, c0: f64, horizontal: bool) -> f64 {
        if level == 0 {
            return c0;
        }
        let ratio = if horizontal {
            self.mips.widths[level] as f64 / self.width as f64
        } else {
            self.mips.heights[level] as f64 / self.height as f64
        };
        (c0 + 0.5) * ratio - 0.5
    }

    #[allow(clippy::too_many_arguments)]
    fn bilinear(
        &self,
        level: usize,
        u: f64,
        v: f64,
        weight: f64,
        repeat_x: bool,
        repeat_y: bool,
        acc: &mut Acc,
    ) {
        let (w, h) = (
            self.mips.widths[level] as i64,
            self.mips.heights[level] as i64,
        );
        let (fu, fv) = (u.floor(), v.floor());
        let (fx, fy) = (u - fu, v - fv);
        let (iu, iv) = (fu as i32 as i64, fv as i32 as i64);
        for dy in 0..2 {
            let wy = if dy == 0 { 1.0 - fy } else { fy };
            if wy <= 0.0 {
                continue;
            }
            let ty = wrap(iv + dy, h, repeat_y);
            for dx in 0..2 {
                let wgt = weight * wy * (if dx == 0 { 1.0 - fx } else { fx });
                if wgt <= 0.0 {
                    continue;
                }
                let p = self.mips.read(level, wrap(iu + dx, w, repeat_x), ty);
                acc.add(wgt, p);
            }
        }
    }
}

fn wrap(i: i64, n: i64, repeat: bool) -> usize {
    if !repeat {
        return i.clamp(0, n - 1) as usize;
    }
    let i = i % n;
    (if i < 0 { i + n } else { i }) as usize
}

/// 重みを付けた画素の和（プリマルチプライド。重み × α × 色、どれも 0〜255）。
#[derive(Default)]
struct Acc {
    a: f64,
    r: f64,
    g: f64,
    b: f64,
    count: u32,
    first: (u8, u8, u8, u8),
    same: bool,
}

impl Acc {
    #[inline]
    fn add(&mut self, w: f64, p: (u8, u8, u8, u8)) {
        if self.count == 0 {
            self.first = p;
            self.same = true;
        } else if p != self.first {
            self.same = false;
        }
        self.count += 1;
        let k = w * p.3 as f64;
        self.a += k;
        self.r += k * p.0 as f64;
        self.g += k * p.1 as f64;
        self.b += k * p.2 as f64;
    }

    fn resolve(&self) -> StencilTexel {
        if self.count == 0 {
            return StencilTexel::default();
        }
        if self.same {
            let (cr, cg, cb, ca) = self.first;
            return StencilTexel {
                inside: true,
                alpha: ca as f64 / 255.0,
                luma_alpha: luminance(cr, cg, cb) as f64 / 255.0 * (ca as f64 / 255.0),
                color: Rgba8::new(cr, cg, cb, ca),
            };
        }
        let mut alpha = self.a / 255.0;
        if alpha > 1.0 {
            alpha = 1.0;
        }
        // 輝度は luminance と同じ重み（灰色ならその値）。プリマルチプライドの和から: Σ w·α·(2126 R + 7152 G + 722 B) / 10000
        let mut luma =
            (2126.0 * self.r + 7152.0 * self.g + 722.0 * self.b) / 10000.0 / (255.0 * 255.0);
        if luma > alpha {
            luma = alpha;
        }
        let color = if self.a > 0.0 {
            Rgba8::new(
                to_byte(self.r / self.a / 255.0),
                to_byte(self.g / self.a / 255.0),
                to_byte(self.b / self.a / 255.0),
                to_byte(alpha),
            )
        } else {
            Rgba8::TRANSPARENT
        };
        StencilTexel {
            inside: true,
            alpha,
            luma_alpha: luma,
            color,
        }
    }
}

/// キャンバスの点がステンシルの画像のどこに来るか（C# の StencilMapping）: 画像 = (xx·x + xy·y + x0, yx·x + yy·y + y0)（画素 (i, j) の中心は
/// (i + 0.5, j + 0.5)）。画面は 2D の表示とステンシルの置き場所から作る。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StencilMapping {
    pub xx: f64,
    pub xy: f64,
    pub x0: f64,
    pub yx: f64,
    pub yy: f64,
    pub y0: f64,
}

impl StencilMapping {
    /// 有限でない値は断る。
    pub fn new(xx: f64, xy: f64, x0: f64, yx: f64, yy: f64, y0: f64) -> Result<Self, CoreError> {
        for v in [xx, xy, x0, yx, yy, y0] {
            require_finite(v, "stencil mapping")?;
        }
        Ok(StencilMapping {
            xx,
            xy,
            x0,
            yx,
            yy,
            y0,
        })
    }
    /// キャンバスの画素の格子を画像に 1 対 1 で重ね、(dx, dy) 画像の画素だけずらす。
    pub fn translation(dx: f64, dy: f64) -> Self {
        StencilMapping {
            xx: 1.0,
            xy: 0.0,
            x0: dx,
            yx: 0.0,
            yy: 1.0,
            y0: dy,
        }
    }
    /// キャンバスの 1 画素あたりの画像の画素（2 つの軸の像の長い方。ミップマップの段を選ぶ）。
    pub fn footprint(&self) -> f64 {
        let a = (self.xx * self.xx + self.yx * self.yx).sqrt();
        let b = (self.xy * self.xy + self.yy * self.yy).sqrt();
        if a > b {
            a
        } else {
            b
        }
    }
    /// キャンバスの画素 (px, py) の中心の、画像の点。
    pub fn map(&self, px: i64, py: i64) -> (f64, f64) {
        self.map_point(px as f64 + 0.5, py as f64 + 0.5)
    }
    /// 元の空間の連続した点 (x, y) の、画像の点（3D のビューが画面の点を渡す）。
    pub fn map_point(&self, x: f64, y: f64) -> (f64, f64) {
        (
            self.xx * x + self.xy * y + self.x0,
            self.yx * x + self.yy * y + self.y0,
        )
    }
}

/// 塗る 1 画素がステンシルのどこに来るか（画像の画素の単位）と footprint（描く 1 画素あたりの画像の画素）。3D のビューが模型の上の
/// テクセルの点を画面へ写して渡す（C# の StencilPoint）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StencilPoint {
    pub x: f64,
    pub y: f64,
    pub footprint: f64,
}

impl StencilPoint {
    /// ±1e9 画像の画素の内、footprint は 0 以上。
    pub fn new(x: f64, y: f64, footprint: f64) -> Result<Self, CoreError> {
        require_finite(x, "stencil point")?;
        require_finite(y, "stencil point")?;
        require_finite(footprint, "stencil footprint")?;
        if x.abs() > 1e9 || y.abs() > 1e9 {
            return Err(CoreError::InvalidArgument("ステンシルの点（±1e9）"));
        }
        if footprint < 0.0 {
            return Err(CoreError::InvalidArgument("ステンシルの footprint"));
        }
        Ok(StencilPoint { x, y, footprint })
    }
}

/// ステンシルが 1 画素にすること（C# の StencilSample）: 天井に掛ける量（0〜1）と、色のモードではそこで塗る色。
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct StencilSample {
    pub amount: f64,
    pub color: Rgba8,
    pub has_color: bool,
}

/// ストロークに付けたステンシル（C# の BrushStencil）: 画像、決まったモード（Auto は作るときに決める）、繰り返しと反転、2D のダブの
/// キャンバスからの写し（3D のビューのように呼び手が画素ごとの点を渡すなら None）、色のモードで色を受けるチャンネル。
#[derive(Clone, Debug)]
pub struct BrushStencil {
    image: Arc<StencilImage>,
    mode: StencilMode,
    tiling: StencilTiling,
    invert: bool,
    canvas_to_image: Option<StencilMapping>,
    color_channels: Vec<Channel>,
}

impl PartialEq for BrushStencil {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.image, &other.image)
            && self.mode == other.mode
            && self.tiling == other.tiling
            && self.invert == other.invert
            && self.canvas_to_image == other.canvas_to_image
            && self.color_channels == other.color_channels
    }
}

impl BrushStencil {
    pub fn new(
        image: Arc<StencilImage>,
        mode: StencilMode,
        tiling: StencilTiling,
        invert: bool,
        canvas_to_image: Option<StencilMapping>,
        color_channels: &[Channel],
    ) -> BrushStencil {
        let mode = Self::resolve(mode, &image);
        BrushStencil {
            image,
            mode,
            tiling,
            invert,
            canvas_to_image,
            color_channels: color_channels.to_vec(),
        }
    }
    /// Auto がこの画像で何を意味するか（灰色の画像は量）。
    pub fn resolve(mode: StencilMode, image: &StencilImage) -> StencilMode {
        match mode {
            StencilMode::Auto if image.is_grey() => StencilMode::Mask,
            StencilMode::Auto => StencilMode::Color,
            m => m,
        }
    }
    pub fn image(&self) -> &Arc<StencilImage> {
        &self.image
    }
    /// 量か色（Auto は決めてある）。
    pub fn mode(&self) -> StencilMode {
        self.mode
    }
    pub fn tiling(&self) -> StencilTiling {
        self.tiling
    }
    pub fn invert(&self) -> bool {
        self.invert
    }
    pub fn canvas_to_image(&self) -> Option<StencilMapping> {
        self.canvas_to_image
    }
    /// ストロークのそのチャンネルの面がステンシルの色を受けるか（色のモードで、チャンネルが名指されている）。
    pub fn paints_color_into(&self, channel: Channel) -> bool {
        self.mode == StencilMode::Color && self.color_channels.contains(&channel)
    }

    /// 画像の点 (x, y) を footprint で読んだステンシル。
    pub fn sample(&self, x: f64, y: f64, footprint: f64) -> StencilSample {
        let t = self.image.read(x, y, footprint, self.tiling);
        if !t.inside {
            return StencilSample::default();
        }
        if self.mode == StencilMode::Color {
            return StencilSample {
                amount: t.alpha,
                color: t.color,
                has_color: true,
            };
        }
        let amount = if self.invert {
            t.alpha - t.luma_alpha
        } else {
            t.luma_alpha
        };
        StencilSample {
            amount: if amount < 0.0 {
                0.0
            } else if amount > 1.0 {
                1.0
            } else {
                amount
            },
            color: Rgba8::TRANSPARENT,
            has_color: false,
        }
    }
    pub fn sample_at(&self, at: StencilPoint) -> StencilSample {
        self.sample(at.x, at.y, at.footprint)
    }
    /// キャンバスの画素 (px, py) のステンシル（キャンバスからの写しで）。写しが無ければ None。
    pub fn sample_canvas(&self, px: i64, py: i64) -> Option<StencilSample> {
        let m = self.canvas_to_image?;
        let (x, y) = m.map(px, py);
        Some(self.sample(x, y, m.footprint()))
    }

    /// 色を塗るチャンネルが、ステンシルの色から塗る値（塗りつぶしの画像と同じ読み方。アルファはブラシの色のもの）。
    pub(crate) fn paint_for(&self, kind: ChannelKind, sampled: Rgba8, alpha: u8) -> Rgba8 {
        let (mut r, mut g, mut b) = (sampled.r, sampled.g, sampled.b);
        if self.image.color_space == ImageColorSpace::Linear && is_color_channel(kind) {
            let t = srgb_table();
            r = t[r as usize];
            g = t[g as usize];
            b = t[b as usize];
        }
        if is_scalar_channel(kind) {
            let l = luminance(r, g, b);
            r = l;
            g = l;
            b = l;
        }
        Rgba8::new(r, g, b, alpha)
    }
}

/// 色のチャンネル（C# の FillImageColor.IsColorChannel、標準では Color と Emission）。
pub(crate) fn is_color_channel(kind: ChannelKind) -> bool {
    kind == ChannelKind::Color
}

/// スカラーのチャンネル（輝度を読む。C# の FillImageColor.IsScalarChannel、標準では Roughness・Metallic・Height）。
pub(crate) fn is_scalar_channel(kind: ChannelKind) -> bool {
    kind == ChannelKind::Scalar
}

/// 輝度（整数の Rec. 709 の重み、灰色ならその値。C# の FillImageColor.Luminance）。
#[inline]
pub fn luminance(r: u8, g: u8, b: u8) -> u8 {
    ((2126 * r as u32 + 7152 * g as u32 + 722 * b as u32 + 5000) / 10000) as u8
}

/// リニアの 8 bit の値を sRGB へ（IEC 61966-2-1。C# の FillImageColor の表と同じ値）。
pub fn linear_to_srgb(value: u8) -> u8 {
    srgb_table()[value as usize]
}

fn srgb_table() -> &'static [u8; 256] {
    static TABLE: OnceLock<[u8; 256]> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = [0u8; 256];
        for (i, v) in t.iter_mut().enumerate() {
            let c = i as f64 / 255.0;
            *v = to_byte(if c <= 0.0031308 {
                12.92 * c
            } else {
                1.055 * c.powf(std::hint::black_box(1.0 / 2.4)) - 0.055
            });
        }
        t
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(w: usize, h: usize, f: impl Fn(usize, usize) -> Rgba8) -> Vec<u8> {
        let mut v = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                v[(y * w + x) * 4..][..4].copy_from_slice(&f(x, y).to_array());
            }
        }
        v
    }

    #[test]
    fn the_mip_budget_counts_the_levels_beyond_the_image() {
        // C# StencilTests: 64² のミップは 32² + 16² + … + 1² = 1365 画素 = 5,460 バイト
        let px = image(64, 64, |x, y| Rgba8::new(x as u8, y as u8, 7, 255));
        assert!(StencilImage::new(64, 64, px.clone(), ImageColorSpace::Srgb, 5459).is_err());
        let s = StencilImage::new(64, 64, px, ImageColorSpace::Srgb, 5460).unwrap();
        assert_eq!(s.mip_bytes(), 5460);
        assert_eq!(s.mips.widths.len(), 7);
    }

    #[test]
    fn auto_reads_a_grey_image_as_a_mask() {
        let grey = Arc::new(
            StencilImage::new(
                4,
                4,
                image(4, 4, |x, y| {
                    let v = (x * 60) as u8;
                    Rgba8::new(v, v, v, (y * 80) as u8)
                }),
                ImageColorSpace::Srgb,
                u64::MAX,
            )
            .unwrap(),
        );
        let colour = Arc::new(
            StencilImage::new(
                4,
                4,
                image(4, 4, |x, y| {
                    if x == 3 && y == 3 {
                        Rgba8::new(10, 11, 10, 255)
                    } else {
                        Rgba8::new(255, 255, 255, 255)
                    }
                }),
                ImageColorSpace::Srgb,
                u64::MAX,
            )
            .unwrap(),
        );
        assert!(grey.is_grey());
        assert!(!colour.is_grey(), "色の付いた画素 1 つで十分");
        let mode = |i: &Arc<StencilImage>, m| {
            BrushStencil::new(i.clone(), m, StencilTiling::None, false, None, &[]).mode()
        };
        assert_eq!(mode(&grey, StencilMode::Auto), StencilMode::Mask);
        assert_eq!(mode(&colour, StencilMode::Auto), StencilMode::Color);
        assert_eq!(
            mode(&grey, StencilMode::Color),
            StencilMode::Color,
            "選んだモードは保つ"
        );
    }

    #[test]
    fn points_and_mappings_refuse_bad_values() {
        assert!(StencilMapping::new(f64::NAN, 0.0, 0.0, 0.0, 1.0, 0.0).is_err());
        assert!(StencilPoint::new(f64::INFINITY, 0.0, 1.0).is_err());
        assert!(StencilPoint::new(0.0, 0.0, -1.0).is_err());
        assert_eq!(StencilMapping::translation(2.0, 3.0).map(0, 0), (2.5, 3.5));
        assert!(
            (StencilMapping::new(0.6, 0.0, 0.0, 0.8, 0.5, 0.0)
                .unwrap()
                .footprint()
                - 1.0)
                .abs()
                < 1e-12
        );
        assert_eq!(linear_to_srgb(0), 0);
        assert_eq!(linear_to_srgb(255), 255);
        assert_eq!(luminance(77, 77, 77), 77);
    }
}
