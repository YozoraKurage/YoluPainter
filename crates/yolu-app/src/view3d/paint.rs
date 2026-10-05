//! 3D ビューが見せる塗った絵: テクスチャセット（文書）の標準の 6 チャンネルを GPU のテクスチャへ持つ（見せるだけ。正本は core の straight RGBA8）。
//!
//! - チャンネルごとに別のテクスチャで、**使っているチャンネルだけ**作る（どれかの層が有効にしている。`export::uses`）。使っていないチャンネルは
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
//! - 今のセットでないセットの絵も同じ `Paint` の兄弟（`sibling`）で持つ。辺の上限（`set_cap`）で縮めて持ち、文書が変わったとき
//!   （版・変化の記録）だけ同期する。今のセットが替わったとき、前のセットの絵は、ミップの段をコピーして上限の大きさへ縮める
//!   （`demote`。文書を合成し直さない）。

use std::sync::atomic::{AtomicU64, Ordering};

use eframe::egui_wgpu::{self, wgpu};
use yolu_core::export::uses;
use yolu_core::normal::output_from_composites;
use yolu_core::{
    Channel, Document, HeightEdgeMode, LayerKind, NormalSettings, Rect as DocRect, RowOrder,
    TileCoord,
};

/// 塗った絵のテクスチャの一辺の上限（GPU の上限がもっと小さければそれ）。
const MAX_PAINT_SIZE: u32 = 8192;
/// 使っているチャンネルのテクスチャ全部（ミップ込み）の GPU のバイト数の予算（既定）。超える文書は縮めて持つ。4096² で 6 チャンネルすべて
/// （320 MiB）は収まり、8192² の 6 チャンネル（約 1.28 GiB）は 1 段縮めて 4096² にする。
pub const PAINT_BUDGET_BYTES: u64 = 512 << 20;
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
}

struct ChannelTexture {
    size: [u32; 2],
    texture: wgpu::Texture,
    /// 段ごとの 1 段だけの見え方（ミップを作るとき、上の段を読んで下の段へ描く）。
    levels: Vec<wgpu::TextureView>,
    /// 段 i を作るときに読む、段 i − 1 の束ね。
    mip_binds: Vec<wgpu::BindGroup>,
    view: wgpu::TextureView,
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
    /// Normal を作ったときの出力の設定。層の合成を変えない設定（Height → Normal・強さ・端）は変化の記録にタイルを足さないので、
    /// 変わったら Normal を作り直す。
    normal_settings: NormalSettings,
}

#[derive(Clone)]
struct Mips {
    layout: wgpu::BindGroupLayout,
    rgba: wgpu::RenderPipeline,
    /// sRGB の形式の段を作る（読むときにリニアへ、書くときに sRGB へ: リニアで平均する）。
    srgb: wgpu::RenderPipeline,
    scalar: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
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
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/mip.wgsl").into()),
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
            stats: PaintStats::default(),
        }
    }

    /// 同じ装置・同じ道具（ミップのパイプライン・既定の 1 × 1）を使う、まっさらな別の絵（ほかのセット用）。
    pub fn sibling(&self) -> Paint {
        Paint {
            device: self.device.clone(),
            queue: self.queue.clone(),
            mips: self.mips.clone(),
            defaults: self.defaults.clone(),
            set: None,
            version: 0,
            layout_version: 0,
            scratch: Vec::new(),
            scratch_height: Vec::new(),
            budget: self.budget,
            cap: None,
            uncapped: false,
            uid: next_uid(),
            stats: PaintStats::default(),
        }
    }

    /// 辺の上限を決める（ほかのセットを縮めて持つ。None は GPU の上限だけ）。ゆるめたときは、次の同期が縮めていた絵を元の大きさで
    /// 作り直す。きつくしたときは、`demote` が GPU の中で縮める（できなければ次の同期が作り直す）。
    pub fn set_cap(&mut self, cap: Option<u32>) {
        if self.cap == cap {
            return;
        }
        let looser = match (self.cap, cap) {
            (Some(_), None) => true,
            (Some(old), Some(new)) => new > old,
            (None, _) => false,
        };
        self.uncapped |= looser;
        self.cap = cap;
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

    /// 文書を最後に同期してから変わっていないか（文書の ID・変化の通し番号・版・Normal の設定が同じ）。同じなら同期は何もしない。
    pub fn synced_with(&self, doc: &Document) -> bool {
        self.set.as_ref().is_some_and(|s| {
            s.doc_id == doc.id()
                && s.serial == doc.change_serial()
                && s.revision == doc.revision()
                && s.normal_settings == doc.normal_settings()
        })
    }

    /// 次の同期が絵を全部作り直すか（初め・文書が替わった・縮めを上げる・上限をゆるめた・変化の記録が切れた）。ほかのセットの作り直しは
    /// 1 フレームに数を絞る。
    pub fn needs_rebuild(&self, doc: &Document) -> bool {
        let (want_shift, _) = self.shift_for(doc);
        self.rebuild_needed(doc, want_shift)
    }

    fn rebuild_needed(&self, doc: &Document, want_shift: u32) -> bool {
        match &self.set {
            None => true,
            Some(s) => {
                s.doc_id != doc.id()
                    || s.doc_size != (doc.width(), doc.height())
                    // 新しく使い始めたチャンネルで予算を超えるときは、縮めを上げて作り直す（下げるのは文書が替わるとき）
                    || s.shift < want_shift
                    // 上限をゆるめたときは、縮めていた絵を元の大きさへ戻す
                    || (self.uncapped && s.shift > want_shift)
                    // 変化の記録がこの文書のものでない（since がこの文書の番号でない）
                    || s.serial > doc.change_serial()
            }
        }
    }

    /// この文書を `cap`（辺の上限）で縮めて持つとしたときのバイト数（使っているチャンネルのミップ込み。持つかどうかの計画に使う）。
    pub fn estimate_bytes(&self, doc: &Document, cap: u32) -> u64 {
        self.estimate(doc, cap).1
    }

    /// `estimate_bytes` の、縮めの段とバイト数（ユーザーチャンネルの配列は、この段から縮める）。
    pub fn estimate(&self, doc: &Document, cap: u32) -> (u32, u64) {
        let limit = self.side_limit().min(cap);
        let per_texel = used_bytes_per_texel(doc);
        let shift = choose_shift([doc.width(), doc.height()], per_texel, limit, u64::MAX);
        let size = [
            doc.width().div_ceil(1 << shift).max(1),
            doc.height().div_ceil(1 << shift).max(1),
        ];
        (shift, mip_bytes(size, per_texel))
    }

    /// 次の同期のあとに、この文書の絵が取るバイト数（使っているチャンネルのミップ込み）。作り直すなら今の予算と上限で決まる大きさ、
    /// 作り直さないなら今の大きさ。今のセットが替わるとき、新しい絵を作る前に予算の残りを決めるのに使う。
    pub fn planned_bytes(&self, doc: &Document) -> u64 {
        let shift = self.planned_level(doc);
        let size = [
            doc.width().div_ceil(1 << shift).max(1),
            doc.height().div_ceil(1 << shift).max(1),
        ];
        mip_bytes(size, used_bytes_per_texel(doc))
    }

    /// `planned_bytes` の縮めの段。
    pub fn planned_level(&self, doc: &Document) -> u32 {
        let (want, _) = self.shift_for(doc);
        match &self.set {
            Some(s) if !self.rebuild_needed(doc, want) => s.shift,
            _ => want,
        }
    }

    /// 同期の作業用のバッファを、容量ごと手放す。合成の作業は文書の大きさ（帯の分、Height から作る Wrap の Normal は文書全体）に
    /// なるので、作り終えたあとも抱えたままだと、ほかのセットの数に比例して CPU のメモリが残る。
    pub fn release_scratch(&mut self) {
        self.scratch = Vec::new();
        self.scratch_height = Vec::new();
    }

    /// 同期の作業用のバッファが抱えているバイト数（容量。試験と計測用）。
    pub fn scratch_bytes(&self) -> usize {
        self.scratch.capacity() + self.scratch_height.capacity()
    }

    /// 持っている絵の縮めの段（作っていなければ 0）。
    pub fn level(&self) -> u32 {
        self.set.as_ref().map_or(0, |s| s.shift)
    }

    /// 持っている絵のバイト数（作ってあるチャンネルのミップ込み）。
    pub fn bytes(&self) -> u64 {
        self.gpu_bytes()
    }

    /// 上限できつくなった分を、持っている絵のミップの段をコピーして縮める（GPU の中だけ。文書を合成し直さない）。コピーした段は
    /// 文書から縮めて作る段と同じ大きさのときだけ使う（奇数の辺で段の大きさが切り上げと合わないとき・文書が替わっているときは
    /// 何もせず false。次の同期が作り直す）。縮めた絵は、持っていた変化の通し番号のまま。続く同期が、その後に変わったタイルを足す。
    /// コピーは別の積みですぐに出す: 続く同期の書き込み（`write_texture`）は次の `submit` の頭で行われるので、同じ積みの中でコピーすると
    /// 足したタイルを古い絵で上書きしてしまう（`clear_flat_normal` と同じ事情）。
    pub fn demote(&mut self, doc: &Document) -> bool {
        let (want, by_budget) = self.shift_for(doc);
        let Some(set) = self.set.as_ref() else {
            return false;
        };
        if set.doc_id != doc.id()
            || set.doc_size != (doc.width(), doc.height())
            || set.shift >= want
        {
            return false;
        }
        let k = want - set.shift;
        let size = [
            doc.width().div_ceil(1 << want),
            doc.height().div_ceil(1 << want),
        ];
        if k >= set.levels
            || [(set.size[0] >> k).max(1), (set.size[1] >> k).max(1)] != size
            || set.serial > doc.change_serial()
        {
            return false;
        }
        let (old_levels, old_size) = (set.levels, set.size);
        let levels = old_levels - k;
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("yolu-3d-demote"),
            });
        let mut moved: [Option<ChannelTexture>; 6] = Default::default();
        for slot in Slot::ALL {
            let Some(old) = set.textures[slot.index()].as_ref() else {
                continue;
            };
            let new = self.make_texture(slot.format(), size, levels);
            for j in k..old_levels {
                let (w, h) = ((old_size[0] >> j).max(1), (old_size[1] >> j).max(1));
                encoder.copy_texture_to_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &old.texture,
                        mip_level: j,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::TexelCopyTextureInfo {
                        texture: &new.texture,
                        mip_level: j - k,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::Extent3d {
                        width: w,
                        height: h,
                        depth_or_array_layers: 1,
                    },
                );
            }
            moved[slot.index()] = Some(new);
        }
        self.queue.submit(Some(encoder.finish()));
        let set = self.set.as_mut().expect("確かめた");
        set.textures = moved;
        set.size = size;
        set.levels = levels;
        set.shift = want;
        set.by_budget = by_budget;
        self.layout_version += 1;
        self.version += 1;
        self.stats.level = want;
        self.stats.by_budget = by_budget;
        self.stats.gpu_bytes = self.gpu_bytes();
        true
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

    /// 試験用: チャンネルの 1 段の中身を CPU へ読む（形式のバイト列、行は下から。大きさつき）。使っていないチャンネル・段が無いときは None。
    pub fn read_level(&self, slot: Slot, level: u32) -> Option<(Vec<u8>, [u32; 2])> {
        let texture = self.set.as_ref()?.textures[slot.index()].as_ref()?;
        if level as usize >= texture.levels.len() {
            return None;
        }
        let (w, h) = (
            (texture.size[0] >> level).max(1),
            (texture.size[1] >> level).max(1),
        );
        let row = (w * slot.bytes_per_texel()).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("yolu-3d-read"),
            size: row as u64 * h as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("yolu-3d-read"),
            });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &texture.texture,
                mip_level: level,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(Some(encoder.finish()));
        buffer.slice(..).map_async(wgpu::MapMode::Read, |r| {
            r.expect("読み出しの対応付け");
        });
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        let mapped = buffer.slice(..).get_mapped_range().expect("読み出しの範囲");
        let line = (w * slot.bytes_per_texel()) as usize;
        let mut out = Vec::with_capacity(line * h as usize);
        for y in 0..h as usize {
            out.extend_from_slice(&mapped[y * row as usize..y * row as usize + line]);
        }
        drop(mapped);
        buffer.unmap();
        Some((out, [w, h]))
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
                }
                self.layout_version += 1;
                changed = true;
                continue;
            }
            if !used {
                continue;
            }
            let created = !has;
            let rects = if created {
                self.create_texture(slot);
                self.layout_version += 1;
                changed = true;
                full_rects(doc, slot, shift)
            } else {
                changed_rects(doc, slot, since, shift)
            };
            let rects = plan_regions(rects, shift, doc.bounds(), keeps_whole(doc, slot));
            let (uploaded, mut dirty) = self.upload(doc, slot, &rects);
            last_slot_tiles[slot.index()] = uploaded;
            // 作った直後は、全部の段を作り直す（空のテクスチャの下の段も、Normal の平らな法線も）
            if created {
                let size = self.set.as_ref().expect("作った").size;
                dirty = Some([0, 0, size[0], size[1]]);
            }
            if let Some(d) = dirty {
                let set = self.set.as_ref().expect("作った");
                let texture = set.textures[slot.index()].as_ref().expect("作った");
                self.rebuild_mips(slot.format(), texture, encoder, d);
            }
            if uploaded > 0 {
                changed = true;
            }
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
        self.stats.gpu_bytes = self.gpu_bytes();
    }

    fn gpu_bytes(&self) -> u64 {
        let Some(set) = &self.set else { return 0 };
        Slot::ALL
            .iter()
            .filter(|s| set.textures[s.index()].is_some())
            .map(|s| {
                (0..set.levels)
                    .map(|l| {
                        let w = (set.size[0] >> l).max(1) as u64;
                        let h = (set.size[1] >> l).max(1) as u64;
                        w * h * s.bytes_per_texel() as u64
                    })
                    .sum::<u64>()
            })
            .sum()
    }

    /// 絵の一辺の上限（GPU の上限と `MAX_PAINT_SIZE` の小さい方。上限 `cap` は含めない）。
    fn side_limit(&self) -> u32 {
        self.device
            .limits()
            .max_texture_dimension_2d
            .min(MAX_PAINT_SIZE)
    }

    /// 文書を持つ縮めの段（2 の shift 乗）と、それが予算で決まったか。使っているチャンネルのミップ込みの合計が予算に収まり、辺が GPU の
    /// 上限（と、あれば `cap`）に収まるまで上げる。`cap` で決まる縮めは予算で決まるのではないので、`by_budget` にしない。
    fn shift_for(&self, doc: &Document) -> (u32, bool) {
        let limit = self.side_limit().min(self.cap.unwrap_or(u32::MAX));
        let per_texel = used_bytes_per_texel(doc);
        let (w, h) = (doc.width(), doc.height());
        let shift = choose_shift([w, h], per_texel, limit, self.budget);
        let by_limit = choose_shift([w, h], 0, limit, u64::MAX);
        (shift, shift > by_limit)
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

    /// ミップを作れる形のテクスチャ（段ごとの見え方と、上の段を読む束ねつき）。
    fn make_texture(
        &self,
        format: wgpu::TextureFormat,
        size: [u32; 2],
        levels: u32,
    ) -> ChannelTexture {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("yolu-3d-paint"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let level_views: Vec<wgpu::TextureView> = (0..levels)
            .map(|i| {
                texture.create_view(&wgpu::TextureViewDescriptor {
                    base_mip_level: i,
                    mip_level_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();
        let mip_binds = (1..levels as usize)
            .map(|i| {
                self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("yolu-3d-mip"),
                    layout: &self.mips.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&level_views[i - 1]),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&self.mips.sampler),
                        },
                    ],
                })
            })
            .collect();
        let view = texture.create_view(&Default::default());
        ChannelTexture {
            size,
            texture,
            levels: level_views,
            mip_binds,
            view,
        }
    }

    /// 文書の大きさと関係なく、1 枚の絵（straight RGBA8、行は下から。メッシュマップの表示用の 8 bit・マットキャップの画像）をミップつきの
    /// テクスチャにして返す（乗算済みにして上げる）。`srgb` の絵は sRGB の形式で、リニアで乗算済みにする（読むとリニアの乗算済み。
    /// Color と同じ）。そうでない絵は値のまま乗算済みにする（メッシュマップはガンマの値のまま見せる）。GPU のテクスチャの辺の上限か
    /// 予算（`budget` の半分。ミップ込み）を超える大きさは、2 の累乗で縮めて持つ（`ImageTexture::level`）。大きさ 0・バイト数が合わない絵は None。
    pub fn create_image(
        &self,
        rgba: &[u8],
        size: [u32; 2],
        srgb: bool,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Option<ImageTexture> {
        if size[0] == 0 || size[1] == 0 || rgba.len() as u64 != size[0] as u64 * size[1] as u64 * 4
        {
            return None;
        }
        let limit = self.device.limits().max_texture_dimension_2d;
        let shift = choose_shift(size, 4, limit, self.budget / 2);
        let region = DocRect::new(0, 0, size[0], size[1]);
        let (data, w, h) = if srgb {
            reduce_srgb_premultiplied(rgba, region, shift)
        } else {
            reduce_premultiplied(rgba, region, shift)
        };
        let format = if srgb { RGBA_SRGB } else { RGBA };
        let levels = 32 - w.max(h).leading_zeros();
        let texture = self.make_texture(format, [w, h], levels);
        self.queue.write_texture(
            texture.texture.as_image_copy(),
            &data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * 4),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        self.rebuild_mips(format, &texture, encoder, [0, 0, w, h]);
        Some(ImageTexture {
            texture,
            level: shift,
        })
    }

    /// Normal のテクスチャの段 0 を平らな法線で塗る（上げたタイルの外 = 塗っていない所は平ら）。別の積みですぐに出す: 後から上げるタイルの
    /// 書き込み（`write_texture`）は次の `submit` の頭で行われるので、同じ積みの中で塗ると上げたタイルを上書きしてしまう。
    fn clear_flat_normal(&self) {
        let set = self.set.as_ref().expect("作った");
        let texture = set.textures[Slot::Normal.index()].as_ref().expect("作った");
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("yolu-3d-normal-flat"),
            });
        drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("yolu-3d-normal-flat"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &texture.levels[0],
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 128.0 / 255.0,
                        g: 128.0 / 255.0,
                        b: 1.0,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        }));
        self.queue.submit(Some(encoder.finish()));
    }

    /// 矩形（文書の画素。縮めの境に合わせてある。タイルの数つき）ごとに合成して上げる。上げたタイルの数と、段 0 の変わった範囲（x0 y0 x1 y1）。
    fn upload(
        &mut self,
        doc: &Document,
        slot: Slot,
        rects: &[(DocRect, usize)],
    ) -> (usize, Option<[u32; 4]>) {
        let shift = self.set.as_ref().expect("作った").shift;
        let mut dirty: Option<[u32; 4]> = None;
        let mut uploaded = 0;
        for (rect, tiles) in rects {
            let Some((data, dw, dh)) = self.region_texels(doc, slot, *rect, shift) else {
                continue;
            };
            let (dx, dy) = (rect.x >> shift, rect.y >> shift);
            let set = self.set.as_ref().expect("作った");
            let texture = &set.textures[slot.index()].as_ref().expect("作った").texture;
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x: dx, y: dy, z: 0 },
                    aspect: wgpu::TextureAspect::All,
                },
                &data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(dw * slot.bytes_per_texel()),
                    rows_per_image: Some(dh),
                },
                wgpu::Extent3d {
                    width: dw,
                    height: dh,
                    depth_or_array_layers: 1,
                },
            );
            let r = [dx, dy, dx + dw, dy + dh];
            dirty = Some(match dirty {
                None => r,
                Some(d) => [
                    d[0].min(r[0]),
                    d[1].min(r[1]),
                    d[2].max(r[2]),
                    d[3].max(r[3]),
                ],
            });
            uploaded += tiles;
        }
        (uploaded, dirty)
    }

    /// 矩形のテクセル（チャンネルの形式で、縮めた後）。
    fn region_texels(
        &mut self,
        doc: &Document,
        slot: Slot,
        rect: DocRect,
        shift: u32,
    ) -> Option<(Vec<u8>, u32, u32)> {
        let n = (rect.width * rect.height * 4) as usize;
        self.scratch.resize(n, 0);
        match slot {
            Slot::Normal => {
                // 作る設定なら、Height を 1 画素ずつ外へ広げて Sobel の隣を読めるようにする（画布の端は Clamp と同じ端のまま）
                let settings = doc.normal_settings();
                let pad = u32::from(settings.derive_from_height());
                let outer = expand(rect, pad, doc.bounds());
                let m = (outer.width * outer.height * 4) as usize;
                self.scratch.resize(m, 0);
                doc.composite_into(
                    Channel::Normal,
                    outer,
                    &mut self.scratch,
                    RowOrder::BottomUp,
                )
                .ok()?;
                let height = if settings.derive_from_height() {
                    self.scratch_height.resize(m, 0);
                    doc.composite_into(
                        Channel::Height,
                        outer,
                        &mut self.scratch_height,
                        RowOrder::BottomUp,
                    )
                    .ok()?;
                    Some(&self.scratch_height[..])
                } else {
                    None
                };
                let output = output_from_composites(
                    &self.scratch,
                    height,
                    outer.width,
                    outer.height,
                    &settings,
                )
                .ok()?;
                // 外へ広げた分を切り取る
                let (cx, cy) = (rect.x - outer.x, rect.y - outer.y);
                let mut cut = Vec::with_capacity((rect.width * rect.height * 4) as usize);
                for row in 0..rect.height {
                    let start = (((cy + row) * outer.width + cx) * 4) as usize;
                    cut.extend_from_slice(&output[start..start + rect.width as usize * 4]);
                }
                Some(reduce(&cut, rect, shift, 4, |p| {
                    [p[0] as u32, p[1] as u32, p[2] as u32, 255]
                }))
            }
            Slot::Color => {
                doc.composite_into(slot.channel(), rect, &mut self.scratch, RowOrder::BottomUp)
                    .ok()?;
                Some(reduce_srgb_premultiplied(&self.scratch, rect, shift))
            }
            Slot::Emission => {
                doc.composite_into(slot.channel(), rect, &mut self.scratch, RowOrder::BottomUp)
                    .ok()?;
                Some(reduce_emission(&self.scratch, rect, shift))
            }
            Slot::Metallic | Slot::Roughness | Slot::Height => {
                doc.composite_into(slot.channel(), rect, &mut self.scratch, RowOrder::BottomUp)
                    .ok()?;
                // 値 × アルファ（書き出しと同じ整数の式。塗っていない所は 0）
                Some(reduce(&self.scratch, rect, shift, 1, |p| {
                    [((p[0] as u32 * p[3] as u32 + 127) / 255), 0, 0, 0]
                }))
            }
        }
    }

    /// 段 0 の dirty（x0 y0 x1 y1）の下の段を作り直す（その範囲だけを鋏で切って描く）。
    fn rebuild_mips(
        &self,
        format: wgpu::TextureFormat,
        texture: &ChannelTexture,
        encoder: &mut wgpu::CommandEncoder,
        dirty: [u32; 4],
    ) {
        let pipeline = match format {
            SCALAR => &self.mips.scalar,
            RGBA_SRGB => &self.mips.srgb,
            _ => &self.mips.rgba,
        };
        for level in 1..texture.levels.len() {
            let w = (texture.size[0] >> level).max(1);
            let h = (texture.size[1] >> level).max(1);
            // 奇数の大きさの補間がはみ出す分、1 画素ずつ広げる
            let x0 = (dirty[0] >> level).saturating_sub(1);
            let y0 = (dirty[1] >> level).saturating_sub(1);
            let x1 = (dirty[2].div_ceil(1 << level) + 1).min(w);
            let y1 = (dirty[3].div_ceil(1 << level) + 1).min(h);
            if x1 <= x0 || y1 <= y0 {
                continue;
            }
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("yolu-3d-mip"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &texture.levels[level],
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &texture.mip_binds[level - 1], &[]);
            pass.set_scissor_rect(x0, y0, x1 - x0, y1 - y0);
            pass.draw(0..3, 0..1);
        }
    }
}

// ───────── 縮めの決め方 ─────────

/// 文書が使っているチャンネルの 1 テクセルのバイト数の合計。
fn used_bytes_per_texel(doc: &Document) -> u64 {
    Slot::ALL
        .iter()
        .filter(|s| uses(doc, s.channel()))
        .map(|s| s.bytes_per_texel() as u64)
        .sum()
}

/// 大きさ（辺）のテクスチャ（全部の段）のバイト数。`bytes_per_texel` は 1 テクセルのバイト数（チャンネルを重ねるなら合計）。
pub(super) fn mip_bytes(size: [u32; 2], bytes_per_texel: u64) -> u64 {
    let levels = 32 - size[0].max(size[1]).leading_zeros();
    (0..levels)
        .map(|l| (size[0] >> l).max(1) as u64 * (size[1] >> l).max(1) as u64)
        .sum::<u64>()
        * bytes_per_texel
}

/// 大きさを 2 の shift 乗で縮める段数: 辺が `limit` に収まり、全部の段のバイト数（`bytes_per_texel` は重ねるチャンネルの合計）が
/// `budget` に収まる最小の段。1 × 1 まで縮めても収まらないなら、そこまで。
pub(super) fn choose_shift(size: [u32; 2], bytes_per_texel: u64, limit: u32, budget: u64) -> u32 {
    let mut shift = 0;
    while shift < 31 {
        let reduced = [
            size[0].div_ceil(1 << shift).max(1),
            size[1].div_ceil(1 << shift).max(1),
        ];
        let fits = reduced[0] <= limit
            && reduced[1] <= limit
            && mip_bytes(reduced, bytes_per_texel) <= budget;
        if fits || reduced == [1, 1] {
            break;
        }
        shift += 1;
    }
    shift
}

// ───────── 矩形の決め方 ─────────

/// 矩形を `by` 画素だけ外へ広げる（`bounds` で切る）。
fn expand(rect: DocRect, by: u32, bounds: DocRect) -> DocRect {
    let x0 = rect.x.saturating_sub(by);
    let y0 = rect.y.saturating_sub(by);
    let x1 = (rect.x + rect.width + by).min(bounds.width);
    let y1 = (rect.y + rect.height + by).min(bounds.height);
    DocRect::new(x0, y0, x1 - x0, y1 - y0)
}

/// 矩形を縮めの境（2^shift）に合わせて外へ広げる（画布の端で切る）。
pub(super) fn align(rect: DocRect, shift: u32, bounds: DocRect) -> DocRect {
    if shift == 0 {
        return rect;
    }
    let block = 1u32 << shift;
    let x0 = (rect.x >> shift) << shift;
    let y0 = (rect.y >> shift) << shift;
    let x1 = (rect.x + rect.width).div_ceil(block) * block;
    let y1 = (rect.y + rect.height).div_ceil(block) * block;
    DocRect::new(
        x0,
        y0,
        x1.min(bounds.width) - x0,
        y1.min(bounds.height) - y0,
    )
}

/// 全部を作るときに合成するタイル: 画布全体に効く層（塗りつぶし・調整）か、元の画素の無いタイルへ出力が広がる効果（そのチャンネルに当たる
/// フィルター・Generator、層のマスクのフィルター。ぼかしの広がり・画素の無い層の Generator）があれば全タイル、無ければどれかの層が
/// 面を持つタイル。全タイルを合成するのは 2D の表示と同じ見た目にするためで、透明のままのタイルも上げる（段 0 の作った直後の値と同じ）。
fn full_rects(doc: &Document, slot: Slot, shift: u32) -> Vec<(DocRect, usize)> {
    let channel = slot.channel();
    let whole = doc.layers().iter().any(|l| {
        (matches!(l.kind(), LayerKind::Fill | LayerKind::Adjustment) && l.is_channel_enabled(channel))
            || l.has_active_filters(channel)
            || l.mask().is_some_and(|m| m.has_active_filters())
    });
    let mut coords: Vec<TileCoord> = if whole || slot == Slot::Normal && doc.derives_normal() {
        doc.canvas_tiles().collect()
    } else {
        let mut all: Vec<TileCoord> = doc
            .layers()
            .iter()
            .filter_map(|l| l.surface(channel))
            .flat_map(|s| s.tile_coords())
            .collect();
        if slot == Slot::Normal {
            all.extend(
                doc.layers()
                    .iter()
                    .filter_map(|l| l.surface(Channel::Height))
                    .flat_map(|s| s.tile_coords()),
            );
        }
        all
    };
    coords.sort();
    coords.dedup();
    rects_of(doc, slot, &coords, &[], shift)
}

/// since の後に変わった矩形（Normal は Normal の変わったタイルと、Height → Normal が有効なら Height の変わったタイルの外側 1 画素まで）。
fn changed_rects(doc: &Document, slot: Slot, since: u64, shift: u32) -> Vec<(DocRect, usize)> {
    let coords = doc.changed_tiles(slot.channel(), since).unwrap_or_default();
    let height = if slot == Slot::Normal && doc.derives_normal() {
        doc.changed_tiles(Channel::Height, since)
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    rects_of(doc, slot, &coords, &height, shift)
}

/// 端が反対側の端を読む Height → Normal（Wrap）か。部分では足りないので、全部を 1 つの矩形で作る。
fn keeps_whole(doc: &Document, slot: Slot) -> bool {
    slot == Slot::Normal
        && doc.derives_normal()
        && doc.normal_settings().edges() == HeightEdgeMode::Wrap
}

/// タイルの矩形（縮めの境に合わせる。Normal の Height 側は 1 画素外へ。同じ矩形は 1 つに）と、それが表すタイルの数。
fn rects_of(
    doc: &Document,
    slot: Slot,
    coords: &[TileCoord],
    height_coords: &[TileCoord],
    shift: u32,
) -> Vec<(DocRect, usize)> {
    let bounds = doc.bounds();
    if keeps_whole(doc, slot) && (!coords.is_empty() || !height_coords.is_empty()) {
        return vec![(bounds, coords.len() + height_coords.len())];
    }
    let mut out: Vec<(DocRect, usize)> = Vec::new();
    for c in coords {
        if let Some(r) = doc.tile_rect(*c) {
            out.push((align(r, shift, bounds), 1));
        }
    }
    for c in height_coords {
        if let Some(r) = doc.tile_rect(*c) {
            out.push((align(expand(r, 1, bounds), shift, bounds), 1));
        }
    }
    out.sort_by_key(|(r, _)| (r.y, r.x, r.width, r.height));
    out.dedup_by(|a, b| a.0 == b.0);
    out
}

/// 1 回に合成して上げる帯の高さの上限（画素。縮めの境の倍数）。全面を作るときの作業メモリの上限（4096 幅で 16 MiB）。
const MAX_BAND_ROWS: u32 = 1024;

fn area(r: &DocRect) -> u64 {
    r.width as u64 * r.height as u64
}

fn union(a: &DocRect, b: &DocRect) -> DocRect {
    let x0 = a.x.min(b.x);
    let y0 = a.y.min(b.y);
    let x1 = (a.x + a.width).max(b.x + b.width);
    let y1 = (a.y + a.height).max(b.y + b.height);
    DocRect::new(x0, y0, x1 - x0, y1 - y0)
}

/// 上げる矩形をまとめる: 隣り合って合わせても余計な面積がほとんど出ない矩形を 1 つにし（1 回の書き込みの手間が大きい API でも、
/// 1 回のストロークの隣り合うタイルは 1 回で上がる）、全面のように大きくなったものは帯に切る（`keep_whole` なら切らない）。
/// 各矩形のタイルの数は足し合わせる。
pub(super) fn plan_regions(
    mut items: Vec<(DocRect, usize)>,
    shift: u32,
    bounds: DocRect,
    keep_whole: bool,
) -> Vec<(DocRect, usize)> {
    // まず、ぴったり隣り合うもの（同じ行の横の並び → 同じ列の縦の並び）を線形に 1 つにする（全面を作るときの 1000 を超えるタイル）。
    items.sort_by_key(|(r, _)| (r.y, r.x, r.width, r.height));
    let mut rows: Vec<(DocRect, usize)> = Vec::with_capacity(items.len());
    for (r, n) in items {
        match rows.last_mut() {
            Some((p, pn)) if p.y == r.y && p.height == r.height && p.x + p.width == r.x => {
                p.width += r.width;
                *pn += n;
            }
            _ => rows.push((r, n)),
        }
    }
    rows.sort_by_key(|(r, _)| (r.x, r.width, r.y, r.height));
    let mut items: Vec<(DocRect, usize)> = Vec::with_capacity(rows.len());
    for (r, n) in rows {
        match items.last_mut() {
            Some((p, pn)) if p.x == r.x && p.width == r.width && p.y + p.height == r.y => {
                p.height += r.height;
                *pn += n;
            }
            _ => items.push((r, n)),
        }
    }
    // 残ったものが少なければ、合わせても面積が 1.25 倍までのものを 1 つにする（斜めに離れたタイルは別のまま）
    while items.len() <= 64 {
        let mut merged = false;
        'search: for i in 0..items.len() {
            for j in i + 1..items.len() {
                let u = union(&items[i].0, &items[j].0);
                if area(&u) * 4 <= (area(&items[i].0) + area(&items[j].0)) * 5 {
                    let tiles = items[i].1 + items[j].1;
                    items[i] = (u, tiles);
                    items.remove(j);
                    merged = true;
                    break 'search;
                }
            }
        }
        if !merged {
            break;
        }
    }
    if keep_whole {
        return items;
    }
    let rows = (MAX_BAND_ROWS >> shift).max(1) << shift;
    let mut out = Vec::with_capacity(items.len());
    for (rect, tiles) in items {
        if rect.height <= rows {
            out.push((rect, tiles));
            continue;
        }
        let bands = rect.height.div_ceil(rows);
        let mut y = rect.y;
        for k in 0..bands {
            let h = rows.min(rect.y + rect.height - y);
            // タイルの数は帯の面積に比例して配る（合計は変えない）
            let (k, n) = (k as usize, bands as usize);
            let share = tiles * (k + 1) / n - tiles * k / n;
            out.push((
                align(DocRect::new(rect.x, y, rect.width, h), shift, bounds),
                share,
            ));
            y += h;
        }
    }
    out
}

// ───────── 縮め ─────────

/// straight の RGBA8（行は下から）を乗算済みにし、2^shift の箱で縮める（文書の端の欠けた箱は中の画素だけで平均する）。
pub fn reduce_premultiplied(straight: &[u8], region: DocRect, shift: u32) -> (Vec<u8>, u32, u32) {
    reduce(straight, region, shift, 4, |p| {
        let a = p[3] as u32;
        [
            (p[0] as u32 * a + 127) / 255,
            (p[1] as u32 * a + 127) / 255,
            (p[2] as u32 * a + 127) / 255,
            a,
        ]
    })
}

/// straight の sRGB の RGBA8（行は下から）を、リニアで乗算済みにして sRGB に符号化し直し（`Rgba8UnormSrgb` が読むとリニアの乗算済み）、
/// 2^shift の箱でリニアのまま平均して縮める。不透明な画素は元のバイトのまま（8 bit の sRGB → 16 bit のリニア → 8 bit の sRGB は元に戻る）。
pub fn reduce_srgb_premultiplied(straight: &[u8], region: DocRect, shift: u32) -> (Vec<u8>, u32, u32) {
    let (decode, encode) = (srgb_decode16(), srgb_encode16());
    reduce_with(
        straight,
        region,
        shift,
        4,
        |p| {
            let a = p[3] as u32;
            [
                (decode[p[0] as usize] as u32 * a + 127) / 255,
                (decode[p[1] as usize] as u32 * a + 127) / 255,
                (decode[p[2] as usize] as u32 * a + 127) / 255,
                a,
            ]
        },
        |q| [encode[q[0] as usize], encode[q[1] as usize], encode[q[2] as usize], q[3] as u8],
    )
}

/// Emission: 書き出しと同じバイト（ガンマの値 × α。整数の式）。縮めるときはリニアで平均する（sRGB の形式のミップと同じ）。
fn reduce_emission(straight: &[u8], region: DocRect, shift: u32) -> (Vec<u8>, u32, u32) {
    let (decode, encode) = (srgb_decode16(), srgb_encode16());
    reduce_with(
        straight,
        region,
        shift,
        4,
        |p| {
            let a = p[3] as u32;
            let g = |v: u8| decode[((v as u32 * a + 127) / 255) as usize] as u32;
            [g(p[0]), g(p[1]), g(p[2]), a]
        },
        |q| [encode[q[0] as usize], encode[q[1] as usize], encode[q[2] as usize], q[3] as u8],
    )
}

/// sRGB の 8 bit → リニアの 16 bit（0〜65535）。
fn srgb_decode16() -> &'static [u16; 256] {
    static TABLE: std::sync::OnceLock<[u16; 256]> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        std::array::from_fn(|i| (super::brdf::srgb_to_linear(i as f32 / 255.0) * 65535.0).round() as u16)
    })
}

/// リニアの 16 bit → sRGB の 8 bit（四捨五入）。
fn srgb_encode16() -> &'static [u8] {
    static TABLE: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        (0..=u16::MAX)
            .map(|i| (super::brdf::linear_to_srgb(i as f32 / 65535.0) * 255.0).round().min(255.0) as u8)
            .collect()
    })
}

/// 矩形の画素ごとに `map`（4 バイト → 最大 4 つの値）を当てて、2^shift の箱で平均して縮める。`channels` は出力のバイト数（1 か 4）。
fn reduce(
    src: &[u8],
    region: DocRect,
    shift: u32,
    channels: usize,
    map: impl Fn(&[u8]) -> [u32; 4],
) -> (Vec<u8>, u32, u32) {
    reduce_with(src, region, shift, channels, map, |q| q.map(|v| v as u8))
}

/// `reduce` の、平均した値をバイトへ直す式（`finish`）を選べる形（`map` の値は 8 bit を超えてよい）。
fn reduce_with(
    src: &[u8],
    region: DocRect,
    shift: u32,
    channels: usize,
    map: impl Fn(&[u8]) -> [u32; 4],
    finish: impl Fn([u32; 4]) -> [u8; 4],
) -> (Vec<u8>, u32, u32) {
    let (w, h) = (region.width, region.height);
    if shift == 0 {
        let mut out = Vec::with_capacity((w * h) as usize * channels);
        for p in src.as_chunks::<4>().0 {
            let q = finish(map(p));
            out.extend_from_slice(&q[..channels]);
        }
        return (out, w, h);
    }
    let block = 1u32 << shift;
    let (dw, dh) = (w.div_ceil(block), h.div_ceil(block));
    let mut out = Vec::with_capacity((dw * dh) as usize * channels);
    for by in 0..dh {
        for bx in 0..dw {
            let mut sum = [0u32; 4];
            let mut n = 0u32;
            for y in by * block..((by + 1) * block).min(h) {
                for x in bx * block..((bx + 1) * block).min(w) {
                    let i = ((y * w + x) * 4) as usize;
                    let q = map(&src[i..i + 4]);
                    for k in 0..4 {
                        sum[k] += q[k];
                    }
                    n += 1;
                }
            }
            let q = finish(sum.map(|s| (s + n / 2) / n));
            out.extend_from_slice(&q[..channels]);
        }
    }
    (out, dw, dh)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reduce_averages_premultiplied_blocks_and_partial_edges() {
        // 3 × 2 の赤（不透明・半透明・透明）を 2 で縮めると 2 × 1
        let px = |r: u8, a: u8| [r, 0, 0, a];
        let mut s = Vec::new();
        for p in [
            px(255, 255),
            px(255, 128),
            px(9, 0),
            px(255, 255),
            px(255, 128),
            px(9, 0),
        ] {
            s.extend_from_slice(&p);
        }
        let (out, w, h) = reduce_premultiplied(&s, DocRect::new(0, 0, 3, 2), 1);
        assert_eq!((w, h), (2, 1));
        // 左の箱: (255 + 128 + 255 + 128) / 4、右の箱は透明だけ（RGB も 0。乗算済みなので色は消える）
        assert_eq!(&out[0..4], &[192, 0, 0, 192]);
        assert_eq!(&out[4..8], &[0, 0, 0, 0]);
        let (same, w, h) = reduce_premultiplied(&s, DocRect::new(0, 0, 3, 2), 0);
        assert_eq!((w, h), (3, 2));
        assert_eq!(&same[4..8], &[128, 0, 0, 128]);
    }

    #[test]
    fn srgb_texels_keep_opaque_bytes_and_premultiply_and_average_in_linear() {
        // 不透明な画素は元のバイトのまま（8 bit の sRGB → 16 bit のリニア → 8 bit の sRGB が元に戻る。書き出しと同じテクセル）
        let all: Vec<u8> = (0..=255u8).flat_map(|v| [v, 255 - v, v / 2, 255]).collect();
        let (out, _, _) = reduce_srgb_premultiplied(&all, DocRect::new(0, 0, 256, 1), 0);
        assert_eq!(out, all);
        let (out, _, _) = reduce_emission(&all, DocRect::new(0, 0, 256, 1), 0);
        assert_eq!(out, all);
        // 半透明: リニアで掛ける（sRGB 255 × α 0.5 → リニア 0.5 → sRGB 188）。ガンマのまま掛ける Emission は書き出しと同じ 128
        let half = [255u8, 255, 255, 128];
        let (out, _, _) = reduce_srgb_premultiplied(&half, DocRect::new(0, 0, 1, 1), 0);
        assert_eq!(out, [188, 188, 188, 128]);
        let (out, _, _) = reduce_emission(&half, DocRect::new(0, 0, 1, 1), 0);
        assert_eq!(out, [128, 128, 128, 128]);
        // 白と黒（不透明）を縮めると、リニアの平均 0.5 → sRGB 188（ガンマの平均の 128 より明るい。Unity のミップと同じ）
        let bw = [255u8, 255, 255, 255, 0, 0, 0, 255];
        let (out, w, h) = reduce_srgb_premultiplied(&bw, DocRect::new(0, 0, 2, 1), 1);
        assert_eq!((out.as_slice(), w, h), (&[188u8, 188, 188, 255][..], 1, 1));
        let (out, _, _) = reduce_emission(&bw, DocRect::new(0, 0, 2, 1), 1);
        assert_eq!(out, [188, 188, 188, 255]);
    }

    #[test]
    fn scalar_reduce_flattens_value_times_alpha() {
        // 値 128・アルファ 255 と、値 255・アルファ 0（塗っていない所は 0）
        let s = [128u8, 0, 0, 255, 255, 0, 0, 0];
        let map = |p: &[u8]| [(p[0] as u32 * p[3] as u32 + 127) / 255, 0, 0, 0];
        let (out, w, h) = reduce(&s, DocRect::new(0, 0, 2, 1), 0, 1, map);
        assert_eq!((out.as_slice(), w, h), (&[128u8, 0][..], 2, 1));
        let (out, w, h) = reduce(&s, DocRect::new(0, 0, 2, 1), 1, 1, map);
        assert_eq!((out.as_slice(), w, h), (&[64u8][..], 1, 1));
    }

    #[test]
    fn rects_align_to_the_reduction_blocks_and_expand_inside_the_canvas() {
        let bounds = DocRect::new(0, 0, 300, 200);
        // 外へ 1 画素: 画布の端で切る
        assert_eq!(
            expand(DocRect::new(256, 0, 44, 200), 1, bounds),
            DocRect::new(255, 0, 45, 200)
        );
        // 縮めの境（4 画素）に合わせて広げる
        assert_eq!(
            align(DocRect::new(255, 1, 3, 3), 2, bounds),
            DocRect::new(252, 0, 8, 4)
        );
        assert_eq!(
            align(DocRect::new(297, 197, 3, 3), 2, bounds),
            DocRect::new(296, 196, 4, 4),
            "画布の端では端で切る"
        );
        assert_eq!(
            align(DocRect::new(5, 6, 7, 8), 0, bounds),
            DocRect::new(5, 6, 7, 8)
        );
    }

    fn tiles(grid: u32, size: u32, only: impl Fn(u32, u32) -> bool) -> Vec<(DocRect, usize)> {
        let mut v = Vec::new();
        for y in 0..grid {
            for x in 0..grid {
                if only(x, y) {
                    v.push((DocRect::new(x * size, y * size, size, size), 1));
                }
            }
        }
        v
    }

    #[test]
    fn adjacent_tiles_become_one_write_and_far_ones_stay_apart() {
        let bounds = DocRect::new(0, 0, 512, 512);
        // 2 × 1 の隣り合うタイル → 1 つ（2 タイル）
        let two = plan_regions(
            tiles(4, 128, |x, y| y == 1 && (x == 1 || x == 2)),
            0,
            bounds,
            false,
        );
        assert_eq!(two, vec![(DocRect::new(128, 128, 256, 128), 2)]);
        // 2 × 2 → 1 つ（4 タイル）
        let four = plan_regions(
            tiles(4, 128, |x, y| (1..=2).contains(&x) && (1..=2).contains(&y)),
            0,
            bounds,
            false,
        );
        assert_eq!(four, vec![(DocRect::new(128, 128, 256, 256), 4)]);
        // 斜めに離れた 2 つは別（合わせると 4 倍の面積）
        let apart = plan_regions(
            tiles(4, 128, |x, y| x == y && x != 1 && x != 2),
            0,
            bounds,
            false,
        );
        assert_eq!(apart.len(), 2);
        assert_eq!(apart.iter().map(|(_, n)| n).sum::<usize>(), 2);
        // 面積が少し増えるだけの L 字は合わせる（3 タイル → 1.33 倍で、1.25 倍を超えるので別）
        let l = plan_regions(
            tiles(4, 128, |x, y| {
                (x, y) != (1, 1) && (1..=2).contains(&x) && (1..=2).contains(&y)
            }),
            0,
            bounds,
            false,
        );
        assert_eq!(l.iter().map(|(_, n)| n).sum::<usize>(), 3);
        assert!(l.len() >= 2, "{l:?}");
    }

    #[test]
    fn a_whole_canvas_is_cut_into_bands_with_the_tile_count_kept() {
        let bounds = DocRect::new(0, 0, 4096, 4096);
        let all = tiles(32, 128, |_, _| true);
        assert_eq!(all.len(), 1024);
        let bands = plan_regions(all.clone(), 0, bounds, false);
        assert_eq!(bands.len(), 4, "1024 行ずつの帯");
        assert_eq!(bands.iter().map(|(_, n)| n).sum::<usize>(), 1024);
        assert!(bands
            .iter()
            .all(|(r, _)| r.width == 4096 && r.height == 1024));
        // 縮めの境（8 画素）に合っている
        let reduced = plan_regions(all.clone(), 3, bounds, false);
        assert!(reduced
            .iter()
            .all(|(r, _)| r.y % 8 == 0 && r.height % 8 == 0));
        // 端が反対側を読む設定は切らない（1 つの矩形のまま）
        let whole = plan_regions(all, 0, bounds, true);
        assert_eq!(whole.len(), 1);
        assert_eq!(whole[0].0, bounds);
    }

    #[test]
    fn shrink_follows_the_texture_limit_and_the_byte_budget() {
        let all = 15; // Color・Emission・Normal が 4、Metallic・Roughness・Height が 1
                      // 4096² の 6 チャンネルは予算（512 MiB）に収まり、縮めない（約 320 MiB）
        assert_eq!((mip_bytes([4096, 4096], all) + (1 << 19)) >> 20, 320);
        assert_eq!(choose_shift([4096, 4096], all, 8192, PAINT_BUDGET_BYTES), 0);
        // 8192² の 6 チャンネルは約 1.28 GiB なので 1 段縮めて 4096²（予算が決める）
        assert!(mip_bytes([8192, 8192], all) > 1 << 30);
        assert_eq!(choose_shift([8192, 8192], all, 8192, PAINT_BUDGET_BYTES), 1);
        // 使うのが Color だけなら 8192² も縮めない（約 341 MiB）
        assert_eq!(choose_shift([8192, 8192], 4, 8192, PAINT_BUDGET_BYTES), 0);
        // 辺の上限が先に効く（GPU の上限 4096 で 8192² は 1 段）。予算が無限でも
        assert_eq!(choose_shift([8192, 8192], 1, 4096, u64::MAX), 1);
        assert_eq!(choose_shift([8192, 4096], 1, 4096, u64::MAX), 1);
        // 奇数の大きさは切り上げて数える
        assert_eq!(choose_shift([4097, 100], 1, 4096, u64::MAX), 1);
        // 縮めたあとのバイト数は必ず予算以下（1 × 1 で止まる場合を除く）
        for budget in [1u64 << 20, 5 << 20, 64 << 20] {
            for per_texel in [1u64, 4, 15] {
                let shift = choose_shift([6000, 5000], per_texel, 8192, budget);
                let reduced = [6000u32.div_ceil(1 << shift), 5000u32.div_ceil(1 << shift)];
                assert!(
                    mip_bytes(reduced, per_texel) <= budget,
                    "{budget} {per_texel} {shift}"
                );
                if shift > 0 {
                    let before = [
                        6000u32.div_ceil(1 << (shift - 1)),
                        5000u32.div_ceil(1 << (shift - 1)),
                    ];
                    assert!(mip_bytes(before, per_texel) > budget, "最小の段");
                }
            }
        }
        // 予算が 1 テクセルにも足りなくても、止まる（1 × 1）
        assert_eq!(choose_shift([4, 4], 15, 8192, 1), 2);
    }

    #[test]
    fn a_cap_on_the_side_shrinks_other_sets_like_the_texture_limit() {
        // ほかのセットの上限（辺 1024）: 4096² は 1/4、2048² は 1/2、1024² 以下は縮めない。予算には依らない（u64::MAX）
        assert_eq!(choose_shift([4096, 4096], 15, 1024, u64::MAX), 2);
        assert_eq!(choose_shift([2048, 2048], 15, 1024, u64::MAX), 1);
        assert_eq!(choose_shift([1024, 1024], 15, 1024, u64::MAX), 0);
        assert_eq!(choose_shift([1000, 700], 15, 1024, u64::MAX), 0);
        // 細長い絵は長い辺で決まる
        assert_eq!(choose_shift([4096, 512], 4, 1024, u64::MAX), 2);
        // 縮めた後のバイト数（持つかどうかの計画が見積もる値）: 4096² の Color だけは 1024² のミップ込み
        let reduced = [1024u32, 1024];
        assert_eq!(mip_bytes(reduced, 4), 4 * 1_398_101);
    }

    #[test]
    fn slots_match_standard_channels_and_defaults_match_the_export() {
        for slot in Slot::ALL {
            assert_eq!(Slot::of(slot.channel()), Some(slot));
        }
        assert_eq!(Slot::of(Channel::from_index(6).unwrap()), None);
        // 使っていないチャンネルの既定: Roughness は Standard の平滑度 0.5、法線は平ら
        assert_eq!(Slot::Roughness.default_texel()[0], 128);
        assert_eq!(&Slot::Normal.default_texel()[..3], &[128, 128, 255]);
        assert_eq!(Slot::Metallic.default_texel()[0], 0);
        assert_eq!(&Slot::Emission.default_texel()[..3], &[0, 0, 0]);
    }
}
