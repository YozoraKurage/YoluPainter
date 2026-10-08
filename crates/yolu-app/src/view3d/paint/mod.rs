//! 3D ビューが見せる塗った絵: テクスチャセット（文書）の標準の 6 チャンネルを GPU のテクスチャへ持つ（見せるだけ。正本は core の straight RGBA8）。
//!
//! - チャンネルごとに別のテクスチャで、**使っているチャンネルだけ**作る（どれかのレイヤーが有効にしている。`export::uses`）。使っていないチャンネルは
//!   1 × 1 の既定の値（Roughness 0.5・Metallic 0・Normal は平ら・Emission 黒・Color は透明）を見せる。書き出しと同じ約束。
//! - 値の意味は書き出し（`export::build`）と同じ。ただし Color は、書き出しが straight RGBA（透明の画素の RGB を保つ）なのに対し、見せるために
//!   乗算済み（透明の縁が黒くならない）にして上げる。不透明な画素では同じ値。Emission の RGB は書き出しと同じ（値 × アルファ。アルファは持つだけで
//!   読まない）、Roughness・Metallic・Height は値 × アルファ（塗っていない所は 0。1 チャンネル 8 bit）、Normal は Normal の出力（塗った法線を
//!   平らな法線に載せ、Height → Normal が有効なら Height から作った法線を土台に重ねたもの）。書き出しとのテクセルの一致は試験が照らす。
//! - sRGB の色（Color・Emission）は sRGB の形式（`Rgba8UnormSrgb`）で持つ: GPU が読むときにテクセルをリニアへ直してから補間し、ミップも
//!   リニアで平均する（Unity がリニアの色空間で sRGB のテクスチャを読むのと同じ。ガンマの値のまま補間すると、色の境目の中間の色が暗い）。
//!   Color の乗算済みはリニアで掛ける（テクセル = sRGB(リニア(色) × α)。不透明なら書き出しと同じバイト）。Emission は書き出しと同じバイト
//!   （ガンマの値 × α）を、読むときにリニアにする（Unity が書き出した PNG を読むのと同じ）。
//! - 初めと文書が変わったときだけ全部を作り、あとは core が「変わった」と言うタイルだけを合成して上げる（変わらなかったチャンネルは触らない）。
//!   Normal は、Normal の変わったタイルと、Height → Normal が有効なら Height の変わったタイルの 1 画素外側まで（Sobel が隣を読む）。
//!   ミップマップは変わった範囲だけ作り直す。行は core と同じ下から上（UV の v がそのまま文書の y）。
//! - GPU のテクスチャの辺の上限か、使っているチャンネルのミップ込みの合計のバイト数の予算（`PAINT_BUDGET_BYTES`）を超える文書は、2 の累乗で
//!   縮めて持つ（`shift`。縮めるときは箱で平均する）。新しいチャンネルを使い始めて予算を超えるときは、縮めを上げて全部を作り直す
//!   （縮めは上げるだけ。文書が替わると決め直す）。メッシュマップの 1 枚も同じく予算（半分）と辺の上限で縮める。
//! - 表示の写しは UV の外へ塗り広げる（`set_padding`。書き出しと同じ式の `yolu_core::padding`。正本は変えない）: UV の覆いから作る
//!   段の地図（`padding::Rings`）をモデルの UV の形・文書の大きさ・縮めごとに作って覚え、上げる矩形ごとに、その周りを塗り広げの幅
//!   だけ広げて合成し、矩形の中を画像全体を塗り広げたときと同じ値にして上げる（`DISPLAY_PAD_TEXELS`）。段 0 のバイリニアがアイランドの外の
//!   透明・0 を混ぜないための塗り広げ。今のセットは 1 度作ったら覚えたまま使い、ほかのセットは同期のたびに作り直す（`release_scratch` が手放す。
//!   文書の大きさ・1 画素 1 バイトのメモリを、ほかのセットの数に比例して残さないため）。
//! - 塗り広げるときは、ミップマップ（段 1 以降）を UV の上の画素（覆い + 塗り広げ）だけで作る（押し引き。`shaders/mip_weighted.wgsl`）。塗り広げの幅より
//!   大きい箱（段 4 以上）に、塗り広げの外の色（塗りつぶしの色・透明）が混ざって継ぎ目に線が出るのを防ぐ。段 0 の塗り広げを外側の全部へ広げると、描いた
//!   矩形 1 つの変化が画像全体の外側へ届き、描いている間の同期が幅に比例して重い。重みの絵（`Weights`。R8・ミップつき・セットごとに 1 つでチャンネルで共通）の
//!   段 0 は、そのテクセルの箱の文書の画素が全部、覆い・塗り広げの中なら 255、そうでなければ 0（縮めて持つ絵の箱の外縁の色には外の色が混ざるので、
//!   重みで外す）、段 1 以降は 1 つ上の段の箱の平均（切り上げ。重みのある子が 1 つでもあれば最低 1）。押し（`fs_push`）は 1 つ上の段の箱の重みつきの平均、
//!   引き（`fs_pull`）は重みが 0 のテクセルを 1 つ粗い段から埋める（粗い段の重みが 0 でないテクセルだけを双線形に数える）。重みが 0 でないテクセルとその
//!   となりのテクセル（描画が読むのはここだけ）は、読む 4 つの中の重みが 0 でないテクセルの値だけで決まるので、描いた所だけの作り直しは、押した範囲とその
//!   すぐ外までで足りる（全部を作り直したものと、描画が読むテクセルで同じ）。そこから離れたテクセルは粗い段の埋めた値を読むので、範囲の外では前の値のまま残りうる
//!   （描画では読まれない）。粗い絵を見せている間（ギズモ・スライダーのドラッグ）は、仮の絵のまま重みを使わない今までの作り（全部のテクセルの箱の平均）で
//!   作り、書き換えた段ごとの範囲を覚える（`plain_dirty`）。離して正確に上げ直したときの重みつきの作り直しが、その範囲も作り直す（`weighted_mip_builds`・
//!   `coarse_mip_builds` が通った道を数える）。重みの絵は予算に入る（1 テクセル 1 バイト、ミップ込み。`bytes_per_texel_planned`）。塗り広げない（幅 0・UV の三角形が
//!   無い）セットは重みを使わず、全部のテクセルの箱の平均。
//! - 今のセットでないセットの絵も同じ `Paint` の兄弟（`sibling`）で持つ。辺の上限（`set_cap`）で縮めて持ち、文書が変わったとき
//!   （版・変化の記録）だけ同期する。今のセットが替わったとき、前のセットの絵は、ミップの段をコピーして上限の大きさへ縮める
//!   （`demote`。文書を合成し直さない）。

mod cpu;
mod mips;
mod pad;
mod sets;
mod shrink;
#[cfg(test)]
mod tests;
mod texture;

use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use eframe::egui_wgpu::{self, wgpu};
use rayon::prelude::*;
use yolu_core::export::uses;
use yolu_core::geometry::SurfaceGeometry;
use yolu_core::glam::DVec2;
use yolu_core::normal::output_from_composites;
use yolu_core::padding::{self, Rings};
use yolu_core::{
    Channel, Document, HeightEdgeMode, LayerKind, NormalSettings, Rect as DocRect, RowOrder,
    TileCoord,
};

use cpu::*;
pub use cpu::{reduce_premultiplied, reduce_srgb_premultiplied};
use mips::*;
use shrink::*;
pub(super) use shrink::{align, choose_shift, mip_bytes, plan_regions};

/// 塗った絵のテクスチャの一辺の上限（GPU の上限がもっと小さければそれ）。
const MAX_PAINT_SIZE: u32 = 8192;
/// 使っているチャンネルのテクスチャ全部（ミップ込み）の GPU のバイト数の予算（既定）。超える文書は縮めて持つ。4096² で 6 チャンネルすべて
/// （320 MiB）は収まり、8192² の 6 チャンネル（約 1.28 GiB）は 1 段縮めて 4096² にする。
pub const PAINT_BUDGET_BYTES: u64 = 512 << 20;
/// 文書が変更をまとめている間（ギズモ・スライダーのドラッグ）に、効果の出力がまだ評価されていないタイルを粗く合成する歩幅（1/4 の
/// 大きさで評価して、離したら正確に上げ直す）。
pub const DRAG_STRIDE: u32 = 4;
/// 表示の写しを UV の外へ塗り広げる幅（表示のテクスチャのテクセル。縮めて持つ絵は 2^縮め 倍の文書の画素で塗り広げてから縮める）。
/// 段 0 のバイリニアは隣の 1 テクセルまで読み、段 1 以降は塗り広げの外を重みで外して作る（押し引き）ので、塗り広げの幅に頼るのは段 0 だけ。
/// 測った（`view3d_padding` の計測、2048²・アイランドの外を広くあけた UV の球、表示域 454 × 494 画素）: 球の内側で継ぎ目がにじむ画素は、幅 0 で
/// 487・259・149・70（球の直径 312・154・78・40 画素。lavapipe の描画）、幅 2 以上は全部 0。押し引きにする前は、16 で 0・0・1・24、32 で全部 0、64 でも寝た面で
/// 6〜19 画素残った。描いている最中の 1 フレームの同期（4096²・Color と Roughness の 2 タイルずつ、lavapipe）は、幅 0 の 0.71 ms に対して
/// 16 で 1.16 ms・32 で 1.84 ms（押し引きにする前の測り。幅に比例して増える）。幅は 16 のままにしてある。
pub const DISPLAY_PAD_TEXELS: u32 = 16;
/// 8 bit ずつのリニアの値（Normal・メッシュマップ・リニアの画像）と 1 チャンネル 8 bit の値の形式。
const RGBA: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const SCALAR: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;
/// sRGB の色（Color・Emission・sRGB の画像）の形式: 読むときに GPU がテクセルをリニアへ直してから補間する。
const RGBA_SRGB: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// 見せるチャンネル（シェーダーの束縛の順 = `binding(1 + 番号)`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Slot {
    Color = 0,
    Metallic = 1,
    Roughness = 2,
    Normal = 3,
    Emission = 4,
    Height = 5,
}

impl Slot {
    pub const COUNT: usize = 6;
    pub const ALL: [Slot; 6] = [
        Slot::Color,
        Slot::Metallic,
        Slot::Roughness,
        Slot::Normal,
        Slot::Emission,
        Slot::Height,
    ];

    pub fn channel(self) -> Channel {
        match self {
            Slot::Color => Channel::Color,
            Slot::Metallic => Channel::Metallic,
            Slot::Roughness => Channel::Roughness,
            Slot::Normal => Channel::Normal,
            Slot::Emission => Channel::Emission,
            Slot::Height => Channel::Height,
        }
    }

    pub fn of(channel: Channel) -> Option<Slot> {
        Slot::ALL.into_iter().find(|s| s.channel() == channel)
    }

    pub fn index(self) -> usize {
        self as usize
    }

    fn format(self) -> wgpu::TextureFormat {
        match self {
            Slot::Metallic | Slot::Roughness | Slot::Height => SCALAR,
            Slot::Color | Slot::Emission => RGBA_SRGB,
            Slot::Normal => RGBA,
        }
    }

    fn bytes_per_texel(self) -> u32 {
        match self.format() {
            SCALAR => 1,
            _ => 4,
        }
    }

    /// 使っていないチャンネルが見せる 1 テクセル（書き出しの既定と同じ）。
    fn default_texel(self) -> [u8; 4] {
        match self {
            Slot::Color => [0, 0, 0, 0],
            Slot::Metallic | Slot::Height => [0, 0, 0, 0],
            Slot::Roughness => [128, 0, 0, 0],
            Slot::Normal => [128, 128, 255, 255],
            Slot::Emission => [0, 0, 0, 255],
        }
    }
}

/// 上げた量（試験と状態の表示用）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PaintStats {
    /// 最後の同期で上げたタイルの数（全チャンネルの合計）。
    pub last_tiles: usize,
    /// 最後の同期でチャンネルごとに上げたタイルの数。
    pub last_slot_tiles: [usize; 6],
    /// 最後の同期で全部を作り直したか。
    pub last_rebuilt: bool,
    /// これまでに上げたタイルの数（全チャンネルの合計）。
    pub total_tiles: usize,
    pub total_slot_tiles: [usize; 6],
    /// 文書から塗った絵へ縮めた段（0 なら同じ大きさ）。
    pub level: u32,
    /// 縮めが、GPU のテクスチャの辺の上限でなくバイトの予算で決まったか。
    pub by_budget: bool,
    /// 作ってあるチャンネルのテクスチャ（ミップ込み）のバイト数。
    pub gpu_bytes: u64,
    /// 粗く合成した絵を見せているタイルの数（全チャンネルの合計。ドラッグが終われば正確に上げ直して 0 に戻る）。
    pub coarse_tiles: usize,
    /// ミップマップを UV の上の画素だけで作った（押し引き）回数と、粗い絵を見せているために全部のテクセルの箱の平均で作った回数（チャンネルごとに 1 回）。
    pub weighted_mip_builds: u64,
    pub coarse_mip_builds: u64,
}

struct ChannelTexture {
    size: [u32; 2],
    texture: wgpu::Texture,
    /// 段ごとの 1 段だけの見え方（ミップを作るとき、上の段を読んで下の段へ描く）。
    levels: Vec<wgpu::TextureView>,
    /// 段 i を作るときに読む、段 i − 1 の束ね。
    mip_binds: Vec<wgpu::BindGroup>,
    view: wgpu::TextureView,
    /// 重みつきのミップ（押し引き）の束ね。重みの絵が替わるたびに作り直す。
    weighted_binds: Option<WeightedBinds>,
    /// 粗い絵を見せている間に、全部のテクセルの箱の平均で書き換えた段ごとの範囲（x0 y0 x1 y1。添え字は段）。次の重みつきの作り直しが、その範囲も
    /// 作り直す（外側の色が混ざった値を残さない）。
    plain_dirty: Vec<Option<[u32; 4]>>,
}

/// `rebuild_mips` が作った道。
enum MipsBuilt {
    /// 重みつき（UV の上の画素だけ）。持ち越した範囲も作り直したので空にしてよい。
    Weighted,
    /// 全部のテクセルの箱の平均。書き換えた段ごとの範囲。
    Plain(Vec<Option<[u32; 4]>>),
}

/// 重みつきのミップの、段ごとの束ね（`Weights::uid` の重みの絵に対するもの）。
struct WeightedBinds {
    weights_uid: u64,
    /// 段 i（1 から）を押すときの束ね: 段 i − 1 の色・重みと、段 i の重み。添え字は i − 1。
    push: Vec<wgpu::BindGroup>,
    /// 段 i（1 から最後の 1 つ手前まで）を引くときの束ね: 段 i + 1 の色・重みと、段 i の重み。添え字は i − 1。
    pull: Vec<wgpu::BindGroup>,
}

/// 文書の外の 1 枚の絵（メッシュマップ）。
pub struct ImageTexture {
    texture: ChannelTexture,
    /// 元の絵から縮めた段（0 なら同じ大きさ）。
    level: u32,
}

impl ImageTexture {
    pub fn view(&self) -> &wgpu::TextureView {
        &self.texture.view
    }

    /// 元の絵から縮めた段（0 なら同じ大きさ）。
    pub fn level(&self) -> u32 {
        self.level
    }
}

struct PaintSet {
    textures: [Option<ChannelTexture>; 6],
    size: [u32; 2],
    levels: u32,
    /// 文書 → 絵の縮め（2 の shift 乗）。
    shift: u32,
    /// 縮めがバイトの予算で決まったか（辺の上限だけなら false）。
    by_budget: bool,
    doc_id: u128,
    doc_size: (u32, u32),
    serial: u64,
    /// 最後に同期した文書の版（`Document::revision`。`synced_with` が、変わっていないセットの同期を飛ばすのに使う）。
    revision: u64,
    /// Normal を作ったときの出力の設定。レイヤーの合成を変えない設定（Height → Normal・強さ・端）は変化の記録にタイルを足さないので、
    /// 変わったら Normal を作り直す。
    normal_settings: NormalSettings,
    /// チャンネルごとの、ドラッグの間に粗く合成して上げたタイル（並べて重なり無し）。ドラッグが終わった同期で正確に上げ直す。
    coarse: [Vec<TileCoord>; 6],
    /// 塗り広げた UV の形（None は塗り広げていない）。違う形を求められたら全部を作り直す（前の形の塗り広げを残さない）。
    pad: Option<PadShape>,
}

/// 塗り広げの元: モデルの面の形と、このセットが受け持つマテリアルの番号。
#[derive(Clone)]
pub struct UvSource {
    pub geometry: Arc<SurfaceGeometry>,
    pub material: i32,
}

/// 塗り広げる UV の形（マテリアルの三角形の UV の値の要約と、その数）。ポーズで面の世代が変わっても、UV が同じなら同じ。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PadShape {
    hash: u64,
    triangles: usize,
}

/// 段の地図とその元（形・文書の大きさ・文書の画素での幅）。
struct PadRings {
    shape: PadShape,
    size: (u32, u32),
    reach: u32,
    /// None: 覆うテクセルが無い（三角形が全部 UV の外など）。塗り広げない。
    rings: Option<Arc<Rings>>,
}

/// 重みの絵の元（形・文書の大きさ・文書の画素での塗り広げの幅・縮め）。同じなら同じ絵。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct WeightKey {
    shape: PadShape,
    doc_size: (u32, u32),
    reach: u32,
    shift: u32,
}

/// 表示の写しと同じ大きさの 1 チャンネル 8 bit の重みの絵（ミップつき。セットごとに 1 つで、チャンネルで共通）。段 0 は、そのテクセルの箱の文書の
/// 画素が全部、覆い・塗り広げの中なら 255、そうでなければ 0（`coverage_weights`）、段 1 以降は 1 つ上の段の箱の平均（0〜255 が 0〜1。切り上げで、
/// 1 つでも重みのある子があれば最低 1）。重みが 0 のテクセルは、UV の上の画素だけでできた色を持たない。
struct Weights {
    /// 作った順の番号（束ねの鍵）。
    uid: u64,
    key: WeightKey,
    size: [u32; 2],
    /// 段ごとの別のテクスチャ（1 段だけ）。同じテクスチャの違う段を 1 つの描きで 2 つ読む（引きの粗い段の重みと描き先の重み）と、段の範囲を
    /// テクスチャごとの状態で持つ GL では、片方の段しか読めない。
    textures: Vec<wgpu::Texture>,
    /// 段ごとの見え方。
    views: Vec<wgpu::TextureView>,
    /// 段ごとに、重みが 0 のテクセルがあるか（無い段は引かない）。
    holes: Vec<bool>,
    bytes: u64,
}

#[derive(Clone)]
struct Mips {
    layout: wgpu::BindGroupLayout,
    rgba: wgpu::RenderPipeline,
    /// sRGB の形式の段を作る（読むときにリニアへ、書くときに sRGB へ: リニアで平均する）。
    srgb: wgpu::RenderPipeline,
    scalar: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    weighted: WeightedMips,
}

/// UV の上の画素だけでミップを作る（押し引き）パイプライン。形式ごと（RGBA・sRGB・scalar の順）に、押しと引き。
#[derive(Clone)]
struct WeightedMips {
    layout: wgpu::BindGroupLayout,
    push: [wgpu::RenderPipeline; 3],
    pull: [wgpu::RenderPipeline; 3],
}

impl WeightedMips {
    fn index(format: wgpu::TextureFormat) -> usize {
        match format {
            RGBA_SRGB => 1,
            SCALAR => 2,
            _ => 0,
        }
    }
}

/// 塗った絵の GPU の持ち物。
pub struct Paint {
    device: wgpu::Device,
    queue: wgpu::Queue,
    mips: Mips,
    defaults: Vec<wgpu::TextureView>,
    set: Option<PaintSet>,
    /// 中身が変わるたびに増える（描き直しの鍵）。
    version: u64,
    /// テクスチャの作り・捨てがあった世代（束ねの作り直しの鍵）。
    layout_version: u64,
    scratch: Vec<u8>,
    scratch_height: Vec<u8>,
    /// 使っているチャンネル全部のバイトの予算（メッシュマップ 1 枚はこの半分）。
    budget: u64,
    /// 辺の上限（今のセットでないセットの縮め。None は GPU の上限だけ）。
    cap: Option<u32>,
    /// 上限をゆるめた（次の同期で、上限で縮めていた絵を元の大きさで作り直す）。
    uncapped: bool,
    /// 作った順の番号（プロセスの中で一意。束ねの鍵に使う: 絵を持つ入れ物が替わっても、世代の数が同じに戻らない）。
    uid: u64,
    /// 表示の写しを塗り広げる幅（表示のテクセル。既定は `DISPLAY_PAD_TEXELS`、0 は塗り広げない。試験・計測が変える）。
    pad_texels: u32,
    /// 塗り広げの元（`set_padding`）と、求める形。
    pad_source: Option<UvSource>,
    pad_want: Option<PadShape>,
    /// 形の要約を作った元（面の世代とマテリアル）。同じなら数え直さない。
    pad_seen: Option<(u32, i32, Option<PadShape>)>,
    /// 覚えている段の地図。
    pad_rings: Option<PadRings>,
    /// 表示の写しの重みの絵（塗り広げるときだけ。UV の形・文書の大きさ・縮め・塗り広げの幅が同じなら作り直さない）。
    weights: Option<Weights>,
    pub stats: PaintStats,
}

fn next_uid() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

impl Paint {
    pub fn new(rs: &egui_wgpu::RenderState) -> Paint {
        let device = rs.device.clone();
        let queue = rs.queue.clone();
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("yolu-3d-mip"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/mip.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("yolu-3d-mip"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("yolu-3d-mip"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let make = |format| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("yolu-3d-mip"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let rgba = make(RGBA);
        let srgb = make(RGBA_SRGB);
        let scalar = make(SCALAR);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("yolu-3d-mip"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let weighted = make_weighted_mips(&device);
        let defaults = Slot::ALL
            .iter()
            .map(|slot| {
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("yolu-3d-default"),
                    size: wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: slot.format(),
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                });
                let texel = slot.default_texel();
                queue.write_texture(
                    texture.as_image_copy(),
                    &texel[..slot.bytes_per_texel() as usize],
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(slot.bytes_per_texel()),
                        rows_per_image: Some(1),
                    },
                    wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                );
                texture.create_view(&Default::default())
            })
            .collect();
        Paint {
            device,
            queue,
            mips: Mips {
                layout,
                rgba,
                srgb,
                scalar,
                sampler,
                weighted,
            },
            defaults,
            set: None,
            version: 0,
            layout_version: 0,
            scratch: Vec::new(),
            scratch_height: Vec::new(),
            budget: PAINT_BUDGET_BYTES,
            cap: None,
            uncapped: false,
            uid: next_uid(),
            pad_texels: DISPLAY_PAD_TEXELS,
            pad_source: None,
            pad_want: None,
            pad_seen: None,
            pad_rings: None,
            weights: None,
            stats: PaintStats::default(),
        }
    }

    pub fn uid(&self) -> u64 {
        self.uid
    }

    /// 絵を作ってあるか（文書を一度も同期していなければ false。見え方は既定の 1 × 1 ばかり）。
    pub fn is_built(&self) -> bool {
        self.set.is_some()
    }

    /// 今持っている絵の文書の ID。
    pub fn doc_id(&self) -> Option<u128> {
        self.set.as_ref().map(|s| s.doc_id)
    }

    /// 持っている絵の縮めの段（作っていなければ 0）。
    pub fn level(&self) -> u32 {
        self.set.as_ref().map_or(0, |s| s.shift)
    }

    /// 持っている絵のバイト数（作ってあるチャンネルのミップ込み）。
    pub fn bytes(&self) -> u64 {
        self.gpu_bytes()
    }

    /// バイトの予算を決める（試験が小さくして、縮めの道を通す。ほかのセットは `u64::MAX` にして、辺の上限だけで縮める）。決め直したあと、
    /// 次の同期で必要なら全部を作り直す。上げても、予算で縮めていた絵は戻らない（縮めは上げるだけ。戻るのは文書が替わるとき）。
    /// セットを替えるたびに呼ぶ口なので、ここでは戻さない（戻すと、替えるたびに縮めていた絵を作り直す）。
    pub fn set_budget(&mut self, bytes: u64) {
        self.budget = bytes;
    }

    /// 利用者が決めた予算（設定の GPU のメモリ）を、今のセットの絵に入れる。`set_budget` と違い、上げたときは予算で縮めていた絵を
    /// 元の大きさへ戻す（`uncapped`。次の同期が作り直す）。設定の予算を入れる口（`View3dRenderer::set_paint_budget`）だけが使う。
    pub fn set_budget_and_regrow(&mut self, bytes: u64) {
        self.uncapped |= bytes > self.budget;
        self.budget = bytes;
    }

    /// 試験用: 持っている絵を捨てる（次の同期が文書から全部を作り直す。部分の更新と全面の構築を比べる）。
    pub fn invalidate(&mut self) {
        self.set = None;
        self.layout_version += 1;
    }

    /// 1 × 1 の透明（絵が無いときに束ねる）。
    pub fn blank_view(&self) -> &wgpu::TextureView {
        &self.defaults[Slot::Color.index()]
    }

    /// 中身が変わるたびに増える（描き直しの鍵）。
    pub fn version(&self) -> u64 {
        self.version
    }

    /// テクスチャの作り・捨て・作り直しがあった世代（束ねを作り直す鍵）。
    pub fn layout_version(&self) -> u64 {
        self.layout_version
    }

    /// チャンネルを使っているか（作ってあるか）。
    pub fn has(&self, slot: Slot) -> bool {
        self.set
            .as_ref()
            .is_some_and(|s| s.textures[slot.index()].is_some())
    }

    /// シェーダーへ渡す見え方（使っていなければ 1 × 1 の既定）。
    pub fn view(&self, slot: Slot) -> &wgpu::TextureView {
        self.set
            .as_ref()
            .and_then(|s| s.textures[slot.index()].as_ref())
            .map_or(&self.defaults[slot.index()], |t| &t.view)
    }

    /// 文書に合わせる: 初めと文書が変わったときは全部、ほかは変わったタイルだけを合成して上げる。
    pub fn sync(&mut self, doc: &Document, encoder: &mut wgpu::CommandEncoder) {
        let since = self.set.as_ref().map_or(0, |s| s.serial);
        let (want_shift, _) = self.shift_for(doc);
        let rebuild = self.rebuild_needed(doc, want_shift);
        if rebuild {
            self.create_set(doc);
            self.layout_version += 1;
        }
        let serial = doc.change_serial();
        let mut last_slot_tiles = [0usize; 6];
        let mut changed = rebuild;
        let shift = self.set.as_ref().expect("作った").shift;
        let interactive = doc.is_coalescing();
        for slot in Slot::ALL {
            if slot == Slot::Normal {
                let set = self.set.as_mut().expect("作った");
                if set.normal_settings != doc.normal_settings() {
                    set.normal_settings = doc.normal_settings();
                    if set.textures[slot.index()].take().is_some() {
                        self.layout_version += 1;
                    }
                }
            }
            let used = uses(doc, slot.channel());
            let has = self.has(slot);
            if !used && has {
                if let Some(set) = &mut self.set {
                    set.textures[slot.index()] = None;
                    set.coarse[slot.index()].clear();
                }
                self.layout_version += 1;
                changed = true;
                continue;
            }
            if !used {
                continue;
            }
            let created = !has;
            let mut coarse = Vec::new();
            let rects = if created {
                self.set.as_mut().expect("作った").coarse[slot.index()].clear();
                self.create_texture(slot);
                self.layout_version += 1;
                changed = true;
                full_rects(doc, slot, shift)
            } else {
                let mut coords = doc.changed_tiles(slot.channel(), since).unwrap_or_default();
                let held = &mut self.set.as_mut().expect("作った").coarse[slot.index()];
                if !interactive {
                    // ドラッグの間に粗く上げたタイルは、終わったら正確に上げ直す
                    coords.append(held);
                }
                coords.sort();
                coords.dedup();
                if interactive
                    && coarse_allowed(doc, slot)
                    && !coords.is_empty()
                    && doc.effects_pending(slot.channel(), &coords)
                {
                    coarse = std::mem::take(&mut coords);
                } else {
                    held.retain(|c| coords.binary_search(c).is_err());
                }
                changed_rects(doc, slot, &coords, since, shift)
            };
            let rects = plan_regions(rects, shift, doc.bounds(), keeps_whole(doc, slot));
            let (mut uploaded, mut dirty) = self.upload(doc, slot, &rects);
            if !coarse.is_empty() {
                let (covered, n, d) = self.upload_coarse(doc, slot, &coarse);
                uploaded += n;
                dirty = union_dirty(dirty, d);
                let held = &mut self.set.as_mut().expect("作った").coarse[slot.index()];
                held.extend_from_slice(&held_after_coarse(covered, &coarse));
                held.sort();
                held.dedup();
            }
            last_slot_tiles[slot.index()] = uploaded;
            // 作った直後は、全部の段を作り直す（空のテクスチャの下の段も、Normal の平らな法線も）
            if created {
                let size = self.set.as_ref().expect("作った").size;
                dirty = Some([0, 0, size[0], size[1]]);
            }
            if let Some(d) = dirty {
                // 粗い絵を見せている間は仮の絵なので、重みを使わない今までの作り（全部のテクセルの箱の平均）で作る。外側の色が混ざった範囲は覚えておき、
                // ドラッグが終わって正確に上げ直したときの重みつきの作り直しが、その範囲も作り直す
                let showing_coarse = !coarse.is_empty();
                if !showing_coarse {
                    self.ensure_weights(doc);
                    self.ensure_weighted_binds(slot);
                }
                let set = self.set.as_ref().expect("作った");
                let texture = set.textures[slot.index()].as_ref().expect("作った");
                let weights = if showing_coarse {
                    None
                } else {
                    self.weights.as_ref()
                };
                let built = self.rebuild_mips(slot.format(), texture, weights, encoder, d);
                let texture = self.set.as_mut().expect("作った").textures[slot.index()]
                    .as_mut()
                    .expect("作った");
                match built {
                    MipsBuilt::Weighted => {
                        texture.plain_dirty.clear();
                        self.stats.weighted_mip_builds += 1;
                    }
                    MipsBuilt::Plain(touched) => {
                        if showing_coarse {
                            merge_levels(&mut texture.plain_dirty, &touched);
                            self.stats.coarse_mip_builds += 1;
                        }
                    }
                }
            }
            if uploaded > 0 {
                changed = true;
            }
        }
        let set = self.set.as_mut().expect("作った");
        // 使っているチャンネルが 1 つも無くなったら、重みの絵も要らない
        if set.textures.iter().all(Option::is_none) {
            self.weights = None;
        }
        let set = self.set.as_mut().expect("作った");
        set.serial = serial;
        set.revision = doc.revision();
        self.uncapped = false;
        if changed {
            self.version += 1;
        }
        let tiles: usize = last_slot_tiles.iter().sum();
        self.stats.last_tiles = tiles;
        self.stats.last_slot_tiles = last_slot_tiles;
        self.stats.last_rebuilt = rebuild;
        self.stats.total_tiles += tiles;
        for (total, last) in self
            .stats
            .total_slot_tiles
            .iter_mut()
            .zip(last_slot_tiles.iter())
        {
            *total += last;
        }
        self.stats.level = set.shift;
        self.stats.by_budget = set.by_budget;
        self.stats.coarse_tiles = set.coarse.iter().map(Vec::len).sum();
        self.stats.gpu_bytes = self.gpu_bytes();
    }

    fn create_set(&mut self, doc: &Document) {
        let (w, h) = (doc.width(), doc.height());
        let (shift, by_budget) = self.shift_for(doc);
        let size = [w.div_ceil(1 << shift), h.div_ceil(1 << shift)];
        let levels = 32 - size[0].max(size[1]).leading_zeros();
        self.set = Some(PaintSet {
            textures: Default::default(),
            size,
            levels,
            shift,
            by_budget,
            doc_id: doc.id(),
            doc_size: (w, h),
            serial: 0,
            revision: 0,
            normal_settings: doc.normal_settings(),
            coarse: Default::default(),
            pad: self.pad_want,
        });
    }

    fn create_texture(&mut self, slot: Slot) {
        let set = self.set.as_ref().expect("作った");
        let (size, levels) = (set.size, set.levels);
        let texture = self.make_texture(slot.format(), size, levels);
        let set = self.set.as_mut().expect("作った");
        set.textures[slot.index()] = Some(texture);
        if slot == Slot::Normal {
            self.clear_flat_normal();
        }
    }
}
