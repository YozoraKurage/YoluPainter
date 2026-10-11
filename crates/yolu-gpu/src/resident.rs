//! 正本のタイルの常駐コピーと、読み戻しを伴わない表示更新。
use super::{
    error,
    plan::{self, Plan},
    source::Fetcher,
    tile_programs, GpuError, GpuPainter, LayerData, Options,
};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Arc,
    },
    time::Duration,
};
use yolu_core::{Channel, Document, LayerId, Rect, TileCoord};

#[derive(Clone, Copy, Debug)]
pub struct ResidentOptions {
    /// 表示テクスチャ、レイヤータイルと同量の CPU コピー、作業域と 1 束ぶんの転送を含む。1 回の更新の上げの一時の入れ物は、1 束ぶんを
    /// この固定の量に数え、それを超える分は、この予算の空き（固定の量と常駐のタイルを引いた残り）の中で決める（`transfer_limit_bytes`）。
    pub resident_budget_bytes: u64,
    /// 未完了・取得前の読み戻しバッファを合計した別予算。
    pub readback_budget_bytes: u64,
    pub batch_tiles: u32,
    /// 表示テクスチャを乗算済みのアルファで書く（egui などの乗算済みのテクスチャとして見せるとき）。
    /// 式は `Color32::from_rgba_unmultiplied` と同じ整数で、同じ合成結果から CPU で変換した値とバイトまで一致する。
    /// 読み戻しも乗算済みの値になる。false なら straight RGBA8。
    pub premultiplied_display: bool,
    /// 1 回に流す上げの上限（バイト）。None は予算から決める: 予算の空きの 1/3（GPU の一時の入れ物・`write_buffer` が中で作る入れ物・CPU の
    /// 詰める並びの 3 つが同じ量を持つ）を `TRANSFER_CEILING` まで。Some は固い上限（試験用）。どちらも 1 束ぶん（固定の量に数えてある転送）
    /// より小さくしない。
    pub transfer_limit_bytes: Option<u64>,
}
impl Default for ResidentOptions {
    fn default() -> Self {
        Self {
            resident_budget_bytes: 768 << 20,
            readback_budget_bytes: 64 << 20,
            batch_tiles: 16,
            premultiplied_display: false,
            transfer_limit_bytes: None,
        }
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct UpdateStats {
    pub updated_tiles: usize,
    pub uploaded_tiles: usize,
    pub uploaded_bytes: u64,
    pub cache_hits: usize,
    pub evicted_tiles: usize,
    pub resident_bytes: u64,
    pub cached_tiles: usize,
    /// CPU から GPU への書き込み（`write_buffer`）の回数。上げるタイルと束の定数を 1 回にまとめるので、普通は 1（何も変わらなければ 0）。
    /// 上げの上限（`ResidentOptions::transfer_limit_bytes`）を超える前・予算が狭くて流していない束のタイルを追い出す前に流すときだけ増える。
    pub transfers: usize,
}
/// 借用した表示テクスチャ。次の更新で中身は変わるため、スナップショットとして保存しない。
pub struct Display<'a> {
    pub texture: &'a wgpu::Texture,
    pub view: &'a wgpu::TextureView,
    pub generation: u64,
    pub document_id: u128,
    pub revision: u64,
    pub channel: Channel,
}
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct Key {
    layer: LayerId,
    /// マスクの面か（false はチャンネルの面）。
    mask: bool,
    coord: TileCoord,
}
struct Cached {
    gpu: wgpu::Buffer,
    bytes: Vec<u8>,
    generation: u64,
    touched: u64,
}
struct Binding {
    id: u128,
    channel: Channel,
    width: u32,
    height: u32,
    ts: u32,
    serial: u64,
    revision: u64,
}
struct Work {
    input: wgpu::Buffer,
    metadata: wgpu::Buffer,
    /// 調整の表。
    tables: wgpu::Buffer,
    /// 面ごと・束のタイルごとの、画素の有無（1 語）。
    presence: wgpu::Buffer,
    /// タイルごとの命令の番号の列（plan.rs の `tile_program`）。
    programs: wgpu::Buffer,
    params: wgpu::Buffer,
    coords: wgpu::Buffer,
    capacity: usize,
    /// 上げる面の数（作業域の入力の大きさ）。
    slot_count: usize,
    /// 命令の数（設定の大きさ）。
    entry_count: usize,
    /// 調整の表の語数。
    table_words: usize,
    fixed_bytes: u64,
    /// 固定の量に数えてある 1 束ぶんの転送（入力・命令の並び・調整の表・有無の印・命令の列・定数・座標）。上げの上限の下限。
    batch_transfer_bytes: u64,
    /// 1 タイルのバイト数。
    tile_bytes: u64,
}
struct Lease {
    used: Arc<AtomicU64>,
    bytes: u64,
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.used.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}
/// 予算から決める 1 回の上げの上限の天井（予算に空きが多くても、一時の入れ物はこれより大きくしない）。
const TRANSFER_CEILING: u64 = 64 << 20;

/// まだ流していない上げと束。上げるタイル・命令の並び・束の定数を 1 本の並びに詰め、流すときに 1 つの一時の入れ物へ `write_buffer`
/// 1 回で書いて、GPU の中で行き先へ写す（タイルごと・束ごとに `write_buffer` すると、呼びの回数の分だけ重い装置がある）。
#[derive(Default)]
struct Transfer {
    bytes: Vec<u8>,
    /// 一時の入れ物から写す先（ずらし・行き先・バイト数）。
    copies: Vec<(u64, wgpu::Buffer, u64)>,
    batches: Vec<Batch>,
    /// 流していない束が読むタイル（流す前に追い出さない）。
    keys: HashSet<Key>,
}
impl Transfer {
    /// 並びに詰めて、その（ずらし, バイト数）を返す。行き先へ写すバイト数は 4 の倍数。
    fn stage(&mut self, data: &[u8]) -> (u64, u64) {
        debug_assert!(data.len().is_multiple_of(4));
        let at = self.bytes.len() as u64;
        self.bytes.extend_from_slice(data);
        (at, data.len() as u64)
    }
    /// 並びに詰めて、流すときに `target` の頭へ写す。
    fn copy_to(&mut self, target: &wgpu::Buffer, data: &[u8]) {
        let (at, size) = self.stage(data);
        self.copies.push((at, target.clone(), size));
    }
}
/// 流していない 1 つの束。
struct Batch {
    tiles: usize,
    /// 合成する画素の数（シェーダーの `count`）。
    pixels: u32,
    /// 一時の入れ物の中の（ずらし, バイト数）: 座標・定数・有無の印・命令の列。
    coords: (u64, u64),
    params: (u64, u64),
    presence: (u64, u64),
    programs: (u64, u64),
    /// 作業域へ写すタイルの入れ物と、作業域の中のずらし。
    inputs: Vec<(wgpu::Buffer, u64)>,
    tile: u64,
}
/// 読み戻し先を所有する要求。破棄しても GPU の転送が終わるまで予算の予約を保持する。
pub struct Readback {
    device: wgpu::Device,
    buffer: wgpu::Buffer,
    receiver: mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>,
    submission: wgpu::SubmissionIndex,
    owner: u64,
    generation: u64,
    rect: Rect,
    pitch: u32,
    _lease: Arc<Lease>,
}
impl Readback {
    /// 所有元の表示を捨てた後でも、要求自身がデバイスを保持して完了を処理できる。
    pub fn wait_ready(&self) -> Result<(), GpuError> {
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(self.submission.clone()),
                timeout: Some(Duration::from_secs(30)),
            })
            .map_err(error)?;
        Ok(())
    }
}
impl Drop for Readback {
    fn drop(&mut self) {
        self.buffer.unmap();
    }
}

/// 常駐の入れ物の大きさ（予算とデバイスの上限から決まる）。`prepare` と [`resident_requirements`] が同じ式を使う。
struct Layout {
    /// 1 タイルのバイト数。
    tile: u64,
    /// 1 束に要る入力（面の数 × タイル）。
    per_batch: u64,
    /// 命令の並びのバイト数。
    meta: u64,
    /// 調整の表のバイト数。
    tables: u64,
    /// 1 束のタイル数。
    capacity: usize,
    /// 表示・作業域・1 束ぶんの転送（常駐のタイルを除く固定の量）。
    fixed: u64,
    /// `fixed` のうち 1 束ぶんの転送。
    batch_transfer: u64,
}

/// デバイスの上限に収まらなければ Err、予算が表示と 1 束を保持できなければ None。
fn layout(
    options: &ResidentOptions,
    limits: &wgpu::Limits,
    doc: &Document,
    slots: usize,
    entries: usize,
    table_words: usize,
) -> Result<Option<Layout>, GpuError> {
    let ts = u64::from(doc.tile_size());
    let tile = ts * ts * 4;
    let frame = u64::from(doc.width()) * u64::from(doc.height()) * 4;
    if doc.width() > limits.max_texture_dimension_2d
        || doc.height() > limits.max_texture_dimension_2d
    {
        return Err(error("表示テクスチャがデバイス上限を超える"));
    }
    let meta = entries.max(1) as u64 * std::mem::size_of::<LayerData>() as u64;
    // 調整の表と値（調整レイヤーが無ければ 0。空の束縛は作れないので、作業域の確保では 16 バイトを下限にする）
    let tables = table_words as u64 * 4;
    // 作業入力と 1 束ぶんの転送、座標・定数を先に予約。束の全入力を常駐できる最小量も確保（1 束を超える転送は、`update` が予算の空きの
    // 中で決める）。
    // 1 タイルあたりの量: 入力（GPU と転送用で 2 つ）と、束の全入力の常駐（GPU と CPU のコピーで 2 つ）で `per_batch * 4`、座標 16、
    // 面ごとのタイルの有無の印と命令の番号の列（先頭の（始まり, 長さ）と最大で命令の数。どちらも GPU と転送用で 2 つ）で
    // `8 * (面 + 2 + 命令)`。割る数が `fixed` に足した量と食い違うと、予算の境で 1 束が常駐に収まらなくなる。
    let per_batch = tile * slots.max(1) as u64;
    let per_capacity = per_batch * 4 + 16 + 8 * (slots.max(1) + 2 + entries.max(1)) as u64;
    let available = options
        .resident_budget_bytes
        .saturating_sub(frame + (meta + tables) * 2 + 32);
    let capacity = u64::from(options.batch_tiles)
        .min(available / per_capacity)
        .min(limits.max_storage_buffer_binding_size / per_batch)
        .min(u64::from(limits.max_compute_workgroups_per_dimension) * 64 / (ts * ts))
        as usize;
    if capacity == 0
        || meta > limits.max_storage_buffer_binding_size
        || tables > limits.max_storage_buffer_binding_size
    {
        return Ok(None);
    }
    // 束の面ごとのタイルの有無（1 語ずつ）と、タイルごとの命令の番号の列（先頭に（始まり, 長さ）、続けて最大で命令の数だけ）も作業域に数える
    let flags = slots.max(1) as u64 * capacity as u64 * 4;
    let programs = capacity as u64 * (2 + entries.max(1) as u64) * 4;
    let batch_transfer =
        per_batch * capacity as u64 + meta + tables + flags + programs + 16 + capacity as u64 * 8;
    let fixed = frame + 2 * batch_transfer;
    Ok(Some(Layout {
        tile,
        per_batch,
        meta,
        tables,
        capacity,
        fixed,
        batch_transfer,
    }))
}

/// 文書のこのチャンネルを全部常駐させるのに要る量（予算に数える量と同じ数え方）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Requirements {
    /// 表示のテクスチャ・作業域・転送の余裕。予算が表示と 1 束を保持できないなら `u64::MAX`。
    pub fixed_bytes: u64,
    /// 描いたタイル（レイヤーとマスクの、無いタイルを除く）の GPU コピーと同量の CPU コピー。
    pub tile_bytes: u64,
}

impl Requirements {
    /// 合計。これが `ResidentOptions::resident_budget_bytes` 以内なら、追い出しなしで全部が常駐する。
    pub fn total_bytes(&self) -> u64 {
        self.fixed_bytes.saturating_add(self.tile_bytes)
    }
}

/// 全部を常駐させるのに要る量を、GPU に触れずに見積もる。レイヤーの並びだけを見る軽い計算で、`update` が実際に数える
/// `UpdateStats::resident_bytes` と同じ式（全タイルが常駐したとき一致する）。予算を超える文書は、追い出しながら合成すると
/// 全面の変更のたびにタイルを上げ直すので、呼び手は CPU の合成を選べる。デバイスの上限（表示のテクスチャの大きさ）を超える・
/// GPU が扱えない文書は Err。
pub fn resident_requirements(
    doc: &Document,
    channel: Channel,
    options: &ResidentOptions,
    limits: &wgpu::Limits,
) -> Result<Requirements, GpuError> {
    let plan = Plan::build(doc, channel, false).map_err(error)?;
    let Some(l) = layout(
        options,
        limits,
        doc,
        plan.slots.len(),
        plan.entries.len(),
        plan.table_words,
    )?
    else {
        return Ok(Requirements {
            fixed_bytes: u64::MAX,
            tile_bytes: 0,
        });
    };
    let tiles: u64 = plan
        .slots
        .iter()
        .filter_map(|slot| {
            let layer = &doc.layers()[slot.layer];
            if slot.evaluated {
                // 評価の出力は、全部を評価するまで描いたタイルの数が決まらないので、持ち得る数の上限で見積もる
                return Some(plan::evaluated_tile_bound(doc, channel, slot));
            }
            if slot.mask {
                layer.mask().map(|m| m.surface().tile_count() as u64)
            } else {
                layer.surface(channel).map(|s| s.tile_count() as u64)
            }
        })
        .sum();
    Ok(Requirements {
        fixed_bytes: l.fixed,
        tile_bytes: tiles * l.tile * 2,
    })
}

/// 表示のパイプライン（シェーダーの形ごとに別のシェーダー）。
fn display_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    variant: plan::Variant,
    options: &ResidentOptions,
) -> wgpu::ComputePipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("yolu-resident-display"),
        source: wgpu::ShaderSource::Wgsl(plan::shader_source(variant).into()),
    });
    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("常駐表示"),
        layout: Some(layout),
        module: &shader,
        entry_point: Some(if options.premultiplied_display {
            "display_premultiplied"
        } else {
            "display"
        }),
        compilation_options: Default::default(),
        cache: None,
    })
}

pub struct ResidentCompositor {
    gpu: GpuPainter,
    options: ResidentOptions,
    /// 表示のパイプライン（シェーダーの形ごと。グループも調整も無い形は作るときに作り、ほかは初めて要るときに作る）。
    pipelines: HashMap<plan::Variant, wgpu::ComputePipeline>,
    pipeline_layout: wgpu::PipelineLayout,
    layout: wgpu::BindGroupLayout,
    cache: HashMap<Key, Cached>,
    lru: BTreeSet<(u64, Key)>,
    clock: u64,
    texture: Option<wgpu::Texture>,
    view: Option<wgpu::TextureView>,
    work: Option<Work>,
    binding: Option<Binding>,
    owner: u64,
    generation: u64,
    stats: UpdateStats,
    readback_used: Arc<AtomicU64>,
    last_copy: Option<wgpu::SubmissionIndex>,
}
impl ResidentCompositor {
    pub fn new(options: ResidentOptions) -> Result<Self, GpuError> {
        Self::with_gpu(GpuPainter::new(Options::default())?, options)
    }
    pub fn with_gpu(gpu: GpuPainter, options: ResidentOptions) -> Result<Self, GpuError> {
        if options.batch_tiles == 0 || options.batch_tiles > 64 {
            return Err(error("束のタイル数は1〜64"));
        }
        let validation = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let memory = gpu.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let internal = gpu.device.push_error_scope(wgpu::ErrorFilter::Internal);
        let entries: Vec<_> = [0, 1, 3, 5, 6, 7, 8, 9]
            .into_iter()
            .map(|binding| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::COMPUTE,
                count: None,
                ty: if binding == 5 {
                    wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    }
                } else {
                    wgpu::BindingType::Buffer {
                        ty: if binding == 3 {
                            wgpu::BufferBindingType::Uniform
                        } else {
                            wgpu::BufferBindingType::Storage { read_only: true }
                        },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    }
                },
            })
            .collect();
        let layout = gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("常駐表示"),
                entries: &entries,
            });
        let pl = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            });
        let pipeline = display_pipeline(&gpu.device, &pl, plan::Variant::FLAT, &options);
        let mut failure = None;
        for scope in [internal, memory, validation] {
            if let Some(e) = pollster::block_on(scope.pop()) {
                failure = Some(error(e));
            }
        }
        if let Some(e) = failure {
            return Err(e);
        }
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Ok(Self {
            gpu,
            options,
            pipelines: HashMap::from([(plan::Variant::FLAT, pipeline)]),
            pipeline_layout: pl,
            layout,
            cache: HashMap::new(),
            lru: BTreeSet::new(),
            clock: 0,
            texture: None,
            view: None,
            work: None,
            binding: None,
            owner: NEXT.fetch_add(1, Ordering::Relaxed),
            generation: 0,
            stats: UpdateStats::default(),
            readback_used: Arc::new(AtomicU64::new(0)),
            last_copy: None,
        })
    }
    pub fn adapter_info(&self) -> &wgpu::AdapterInfo {
        self.gpu.adapter_info()
    }
    pub fn stats(&self) -> UpdateStats {
        self.stats
    }
    pub fn pending_readback_bytes(&self) -> u64 {
        self.readback_used.load(Ordering::Acquire)
    }
    /// 同じIDの文書の再読込にも必ず呼ぶ。既発行の読み戻しは古い世代として拒否される。
    pub fn reset(&mut self) -> Result<(), GpuError> {
        self.generation += 1;
        // 旧テクスチャを参照するコピーが終わるまで、次世代の予算として再利用しない。
        if let Some(submission) = self.last_copy.take() {
            if let Err(e) = self.wait(submission.clone()) {
                self.last_copy = Some(submission);
                self.gpu.failed = Some(e.to_string());
                return Err(e);
            }
        }
        self.cache.clear();
        self.lru.clear();
        self.work = None;
        self.view = None;
        self.texture = None;
        self.binding = None;
        self.stats = UpdateStats::default();
        Ok(())
    }
    pub fn display(&self) -> Result<Display<'_>, GpuError> {
        if let Some(e) = &self.gpu.failed {
            return Err(error(e));
        }
        let b = self
            .binding
            .as_ref()
            .ok_or_else(|| error("表示を更新していない"))?;
        Ok(Display {
            texture: self.texture.as_ref().ok_or_else(|| error("表示が無効"))?,
            view: self.view.as_ref().ok_or_else(|| error("表示が無効"))?,
            generation: self.generation,
            document_id: b.id,
            revision: b.revision,
            channel: b.channel,
        })
    }
    fn buffer(&self, size: u64, usage: wgpu::BufferUsages) -> wgpu::Buffer {
        self.gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("常駐作業域"),
            size,
            usage,
            mapped_at_creation: false,
        })
    }
    fn prepare(
        &mut self,
        doc: &Document,
        slots: usize,
        entries: usize,
        table_words: usize,
    ) -> Result<(), GpuError> {
        let Layout {
            tile,
            per_batch,
            meta,
            tables,
            capacity,
            fixed,
            batch_transfer,
        } = layout(
            &self.options,
            &self.gpu.device.limits(),
            doc,
            slots,
            entries,
            table_words,
        )?
        .ok_or_else(|| error("常駐予算では表示と1束を保持できない"))?;
        if self.texture.is_none() {
            let texture = self.gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("合成表示"),
                size: wgpu::Extent3d {
                    width: doc.width(),
                    height: doc.height(),
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::STORAGE_BINDING
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            self.view = Some(texture.create_view(&Default::default()));
            self.texture = Some(texture);
        }
        if self.work.as_ref().is_none_or(|w| {
            w.slot_count != slots
                || w.entry_count != entries
                || w.table_words != table_words
                || w.capacity != capacity
        }) {
            self.work = None;
            while fixed + self.cache.len() as u64 * tile * 2 > self.options.resident_budget_bytes {
                if !self.evict() {
                    return Err(error("常駐予算では表示と1束を保持できない"));
                }
            }
            self.work = Some(Work {
                input: self.buffer(
                    per_batch * capacity as u64,
                    wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                ),
                metadata: self.buffer(
                    meta,
                    wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                ),
                tables: self.buffer(
                    tables.max(16),
                    wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                ),
                presence: self.buffer(
                    (slots.max(1) * capacity * 4) as u64,
                    wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                ),
                programs: self.buffer(
                    (capacity * (2 + entries.max(1)) * 4) as u64,
                    wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                ),
                params: self.buffer(
                    16,
                    wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                ),
                coords: self.buffer(
                    capacity as u64 * 8,
                    wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                ),
                capacity,
                slot_count: slots,
                entry_count: entries,
                table_words,
                fixed_bytes: fixed,
                batch_transfer_bytes: batch_transfer,
                tile_bytes: tile,
            });
        }
        Ok(())
    }
    /// 一番古いタイルを 1 枚手放す。手放すタイルが無ければ false（呼び手は予算を満たせない）。
    fn evict(&mut self) -> bool {
        let Some((_, key)) = self.lru.pop_first() else {
            return false;
        };
        self.cache.remove(&key);
        self.stats.evicted_tiles += 1;
        true
    }
    fn wait(&self, index: wgpu::SubmissionIndex) -> Result<(), GpuError> {
        self.gpu
            .device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(index),
                timeout: Some(Duration::from_secs(30)),
            })
            .map_err(error)?;
        Ok(())
    }
    /// 変更記録を見て表示を更新する。転送の寿命を束ごとの GPU 完了で区切り、画素の読み戻しは行わない。
    pub fn update(&mut self, doc: &Document, channel: Channel) -> Result<UpdateStats, GpuError> {
        if let Some(e) = &self.gpu.failed {
            return Err(error(format!("GPU は再作成が必要: {e}")));
        }
        // GPU が扱えない文書は、状態を変えず（GPU を失敗扱いにせず）断る。呼び手は CPU の合成を使う。
        let plan = Plan::build(doc, channel, true).map_err(error)?;
        let validation = self
            .gpu
            .device
            .push_error_scope(wgpu::ErrorFilter::Validation);
        let memory = self
            .gpu
            .device
            .push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let internal = self
            .gpu
            .device
            .push_error_scope(wgpu::ErrorFilter::Internal);
        // グループ・調整のある文書で初めて使う形のシェーダーは、ここで作る（作れなければ下の誤りの検査で失敗にする）
        if !self.pipelines.contains_key(&plan.variant) {
            let made = display_pipeline(
                &self.gpu.device,
                &self.pipeline_layout,
                plan.variant,
                &self.options,
            );
            self.pipelines.insert(plan.variant, made);
        }
        let result = self.update_inner(doc, channel, &plan);
        let mut failure = None;
        for scope in [internal, memory, validation] {
            if let Some(e) = pollster::block_on(scope.pop()) {
                failure = Some(error(e));
            }
        }
        let result = if let Some(e) = failure {
            Err(e)
        } else {
            result
        };
        if let Err(e) = &result {
            self.gpu.failed = Some(e.to_string());
            let _ = self.reset();
        }
        result
    }
    fn update_inner(
        &mut self,
        doc: &Document,
        channel: Channel,
        plan: &Plan,
    ) -> Result<UpdateStats, GpuError> {
        let switched = self.binding.as_ref().is_none_or(|b| {
            b.id != doc.id()
                || b.channel != channel
                || b.width != doc.width()
                || b.height != doc.height()
                || b.ts != doc.tile_size()
        });
        if switched {
            self.reset()?;
        }
        self.stats = UpdateStats::default();
        // レイヤーの追加・削除・並べ替え・入れ子・表示・マスクなどの変更も、core の変更記録がそのレイヤーのタイルとして持つ。
        // 全部の合成し直しは、初めて・文書やチャンネルが替わったとき（binding が無いとき）だけ。
        let first = self.binding.is_none();
        let coords = if first {
            (0..doc.height().div_ceil(doc.tile_size()))
                .flat_map(|y| {
                    (0..doc.width().div_ceil(doc.tile_size())).map(move |x| TileCoord::new(x, y))
                })
                .collect()
        } else {
            doc.changed_tiles(channel, self.binding.as_ref().expect("確認済み").serial)
                .ok_or_else(|| error("プロジェクトの世代が巻き戻った。reset が必要"))?
        };
        if self
            .binding
            .as_ref()
            .is_some_and(|b| b.revision == doc.revision())
            && !first
        {
            return Ok(self.account());
        }
        self.generation += 1;
        // 今の計画が読む面（レイヤー・マスク）だけを常駐に残す。消えたレイヤーに加えて、隠したレイヤー・不透明度 0・チャンネルが無効・外した
        // マスクなど計画から外れた面のタイルも手放す（GPU バッファと同量の CPU のコピーを抱え続けて予算に数えられ、LRU の
        // 追い出しで生きているタイルを押し出さないように）。戻したとき（表示・不透明度など）は core の変更記録がそのレイヤーの
        // タイルを返すので、そこで上げ直す。
        let wanted: HashSet<(LayerId, bool)> = plan
            .slots
            .iter()
            .map(|slot| (doc.layers()[slot.layer].id(), slot.mask))
            .collect();
        let dead: Vec<_> = self
            .cache
            .keys()
            .filter(|k| !wanted.contains(&(k.layer, k.mask)))
            .copied()
            .collect();
        for k in dead {
            if let Some(v) = self.cache.remove(&k) {
                self.lru.remove(&(v.touched, k));
            }
        }
        self.prepare(doc, plan.slots.len(), plan.entries.len(), plan.table_words)?;
        let metadata = plan.metadata();
        let tile = doc.tile_size() as usize * doc.tile_size() as usize * 4;
        let capacity = self.work.as_ref().expect("準備済み").capacity;
        let mut bytes = vec![0u8; tile];
        // 面の持ち主（平らな計画の面の番号の順）。
        let sources: Vec<(LayerId, bool)> = plan
            .slots
            .iter()
            .map(|slot| (doc.layers()[slot.layer].id(), slot.mask))
            .collect();
        let mut fetcher = Fetcher::new(doc, channel, plan.slots.len());
        // 上げは 1 つの一時の入れ物に詰め、流すときに GPU の中で写す（タイルごとに `write_buffer` しない）
        let mut transfer = Transfer::default();
        {
            let w = self.work.as_ref().expect("準備済み");
            if !metadata.is_empty() {
                transfer.copy_to(&w.metadata, bytemuck::cast_slice(&metadata));
            }
            if !plan.tables.is_empty() {
                transfer.copy_to(&w.tables, bytemuck::cast_slice(&plan.tables));
            }
        }
        for chunk in coords.chunks(capacity) {
            let mut touched = Vec::with_capacity(chunk.len() * sources.len());
            for (k, slot) in plan.slots.iter().enumerate() {
                let (layer, mask) = sources[k];
                // 評価の出力を持つ面は、束のタイルを含む矩形をまとめて評価する（保存した面では何もしない）
                fetcher.prefetch(k, slot, chunk)?;
                for &coord in chunk {
                    let key = Key { layer, mask, coord };
                    // 無いタイル（保存した面にタイルが無い・評価の出力が全部 0）は全画素 0。常駐させず、作業域を 0 で埋める
                    // （予算と転送を使わない）。
                    if !fetcher.tile(k, slot, coord, &mut bytes)? {
                        if let Some(v) = self.cache.remove(&key) {
                            self.lru.remove(&(v.touched, key));
                        }
                        continue;
                    }
                    self.clock += 1;
                    let cached = self
                        .cache
                        .get(&key)
                        .map(|v| (v.touched, v.bytes != bytes, v.gpu.clone()));
                    if let Some((touched_at, changed, target)) = cached {
                        self.lru.remove(&(touched_at, key));
                        if changed {
                            // 足すと上げの上限を超えるなら、先にそこまでを流す
                            if self.over_transfer_limit(&transfer, tile as u64, 0) {
                                self.flush(&mut transfer, plan)?;
                            }
                            transfer.copy_to(&target, &bytes);
                            self.stats.uploaded_tiles += 1;
                            self.stats.uploaded_bytes += tile as u64;
                        } else {
                            self.stats.cache_hits += 1;
                        }
                        let v = self.cache.get_mut(&key).expect("ある");
                        if changed {
                            v.bytes.copy_from_slice(&bytes);
                        }
                        debug_assert!(v.generation <= doc.change_serial());
                        v.generation = doc.change_serial();
                        v.touched = self.clock;
                    } else {
                        while self.work.as_ref().expect("準備済み").fixed_bytes
                            + (self.cache.len() as u64 + 1) * tile as u64 * 2
                            > self.options.resident_budget_bytes
                        {
                            // 一番古いタイルが、まだ流していない束の読むタイルなら、先に流す（流した束のタイルは追い出してよい）
                            if self
                                .lru
                                .first()
                                .is_some_and(|(_, oldest)| transfer.keys.contains(oldest))
                            {
                                self.flush(&mut transfer, plan)?;
                                continue;
                            }
                            // この束のタイルは直前に触って一番新しい。古いタイルが尽きても足りないなら、束を保持できていない
                            if !self.evict() {
                                return Err(error("常駐予算では束のタイルを保持できない"));
                            }
                        }
                        let buffer = self.buffer(
                            tile as u64,
                            wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
                        );
                        if self.over_transfer_limit(&transfer, tile as u64, tile + tile) {
                            self.flush(&mut transfer, plan)?;
                        }
                        transfer.copy_to(&buffer, &bytes);
                        self.cache.insert(
                            key,
                            Cached {
                                gpu: buffer,
                                bytes: bytes.clone(),
                                generation: doc.change_serial(),
                                touched: self.clock,
                            },
                        );
                        self.stats.uploaded_tiles += 1;
                        self.stats.uploaded_bytes += tile as u64;
                    }
                    self.lru.insert((self.clock, key));
                    touched.push(key);
                }
                fetcher.release(k);
            }
            // 画素のあるタイルだけを作業域へ写す。無いタイルは、シェーダーが有無の印で読まない（0 で埋めない）。
            let mut flags = vec![0u32; sources.len().max(1) * chunk.len()];
            let mut inputs = Vec::new();
            for (k, &(layer, mask)) in sources.iter().enumerate() {
                for (j, &coord) in chunk.iter().enumerate() {
                    if let Some(v) = self.cache.get(&Key { layer, mask, coord }) {
                        inputs.push((v.gpu.clone(), ((k * chunk.len() + j) * tile) as u64));
                        flags[k * chunk.len() + j] = 1;
                    }
                }
            }
            let programs = tile_programs(plan, &flags, chunk.len());
            let raw_coords: Vec<[u32; 2]> = chunk.iter().map(|c| [c.x, c.y]).collect();
            let params = [
                (chunk.len() * tile / 4) as u32,
                metadata.len() as u32,
                doc.tile_size(),
                0,
            ];
            let parts = (raw_coords.len() * 8 + 16 + flags.len() * 4 + programs.len() * 4) as u64;
            if self.over_transfer_limit(&transfer, parts, tile) {
                self.flush(&mut transfer, plan)?;
            }
            let batch = Batch {
                tiles: chunk.len(),
                pixels: (chunk.len() * tile / 4) as u32,
                coords: transfer.stage(bytemuck::cast_slice(&raw_coords)),
                params: transfer.stage(bytemuck::cast_slice(&params)),
                presence: transfer.stage(bytemuck::cast_slice(&flags)),
                programs: transfer.stage(bytemuck::cast_slice(&programs)),
                inputs,
                tile: tile as u64,
            };
            transfer.batches.push(batch);
            transfer.keys.extend(touched);
        }
        self.flush(&mut transfer, plan)?;
        self.binding = Some(Binding {
            id: doc.id(),
            channel,
            width: doc.width(),
            height: doc.height(),
            ts: doc.tile_size(),
            serial: doc.change_serial(),
            revision: doc.revision(),
        });
        Ok(self.account())
    }
    /// `add` バイトを足すと、今の上げの上限を超えるか（まだ何も溜めていなければ超えない: 1 つの塊は上限より大きくても流す）。上限は
    /// `ResidentOptions::transfer_limit_bytes`、無ければ予算の空き（固定の量と常駐のタイル。`incoming` はこれから常駐に入るタイルの GPU と
    /// CPU のコピー）の 1/3 を `TRANSFER_CEILING` まで。どちらも 1 束ぶんより小さくしない。
    fn over_transfer_limit(&self, t: &Transfer, add: u64, incoming: usize) -> bool {
        if t.bytes.is_empty() {
            return false;
        }
        let w = self.work.as_ref().expect("準備済み");
        // 常駐のタイル（GPU と CPU のコピー。タイルはどれも同じ大きさ）。追い出しの判定と同じ数え方
        let resident = self.cache.len() as u64 * w.tile_bytes * 2 + incoming as u64;
        let free = self
            .options
            .resident_budget_bytes
            .saturating_sub(w.fixed_bytes + resident);
        let limit = self
            .options
            .transfer_limit_bytes
            .unwrap_or((free / 3).min(TRANSFER_CEILING))
            .max(w.batch_transfer_bytes);
        t.bytes.len() as u64 + add > limit
    }
    /// 溜めた上げと束を流す: 一時の入れ物を 1 つ作って `write_buffer` 1 回で詰め、GPU の中で常駐のタイル・作業域の命令の並びと調整の表・束ごとの
    /// 定数・座標・有無・命令の列へ写してから、束ごとに作業域へタイルを写して合成する。作業域と定数の入れ物は束で使い回すので、束ごとに
    /// GPU の完了を待つ（次の束の写しが前の束の合成の後になるように）。
    fn flush(&mut self, t: &mut Transfer, plan: &Plan) -> Result<(), GpuError> {
        if t.copies.is_empty() && t.batches.is_empty() {
            return Ok(());
        }
        // 一時の入れ物は GPU の側に作り、`write_buffer` 1 回で詰める（キューの写しの帯を使う）。呼び手が対応付けの入れ物（MAP_WRITE）を
        // 毎回作ると、作りと最初の書き込みが重い装置がある（dzn で 16 MiB に約 2 秒）
        let staging = self.buffer(
            t.bytes.len() as u64,
            wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
        );
        self.gpu.queue.write_buffer(&staging, 0, &t.bytes);
        self.stats.transfers += 1;
        let w = self.work.as_ref().expect("準備済み");
        let mut encoder = Some(self.gpu.device.create_command_encoder(&Default::default()));
        for (at, target, size) in t.copies.drain(..) {
            encoder
                .as_mut()
                .expect("作った")
                .copy_buffer_to_buffer(&staging, at, &target, 0, size);
        }
        let batches = std::mem::take(&mut t.batches);
        let group = (!batches.is_empty()).then(|| {
            self.gpu
                .device
                .create_bind_group(&wgpu::BindGroupDescriptor {
                    label: None,
                    layout: &self.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: w.input.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: w.metadata.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: w.params.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 5,
                            resource: wgpu::BindingResource::TextureView(
                                self.view.as_ref().expect("準備済み"),
                            ),
                        },
                        wgpu::BindGroupEntry {
                            binding: 6,
                            resource: w.coords.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 7,
                            resource: w.tables.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 8,
                            resource: w.presence.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 9,
                            resource: w.programs.as_entire_binding(),
                        },
                    ],
                })
        });
        for b in &batches {
            let mut encoder = encoder
                .take()
                .unwrap_or_else(|| self.gpu.device.create_command_encoder(&Default::default()));
            for (part, target) in [
                (b.coords, &w.coords),
                (b.params, &w.params),
                (b.presence, &w.presence),
                (b.programs, &w.programs),
            ] {
                if part.1 > 0 {
                    encoder.copy_buffer_to_buffer(&staging, part.0, target, 0, part.1);
                }
            }
            for (source, offset) in &b.inputs {
                encoder.copy_buffer_to_buffer(source, 0, &w.input, *offset, b.tile);
            }
            {
                let mut pass = encoder.begin_compute_pass(&Default::default());
                pass.set_pipeline(&self.pipelines[&plan.variant]);
                pass.set_bind_group(0, group.as_ref().expect("束がある"), &[]);
                pass.dispatch_workgroups(b.pixels.div_ceil(64), 1, 1);
            }
            self.wait(self.gpu.queue.submit([encoder.finish()]))?;
            self.stats.updated_tiles += b.tiles;
        }
        // 束が無い（上げだけ）ときも出す。次の更新の束は、同じキューの後ろで読む
        if let Some(encoder) = encoder {
            self.gpu.queue.submit([encoder.finish()]);
        }
        t.bytes.clear();
        t.keys.clear();
        Ok(())
    }
    fn account(&mut self) -> UpdateStats {
        self.stats.cached_tiles = self.cache.len();
        self.stats.resident_bytes = self.work.as_ref().map_or(0, |w| w.fixed_bytes)
            + self
                .cache
                .values()
                .map(|v| v.bytes.len() as u64 * 2)
                .sum::<u64>();
        self.stats
    }
    /// コピーを先にキューへ積むので、後続の表示更新は要求時の内容を変更しない。
    pub fn request_readback(&mut self, rect: Rect) -> Result<Readback, GpuError> {
        let validation = self
            .gpu
            .device
            .push_error_scope(wgpu::ErrorFilter::Validation);
        let memory = self
            .gpu
            .device
            .push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let internal = self
            .gpu
            .device
            .push_error_scope(wgpu::ErrorFilter::Internal);
        let result = self.request_readback_inner(rect);
        let mut failure = None;
        for scope in [internal, memory, validation] {
            if let Some(e) = pollster::block_on(scope.pop()) {
                failure = Some(error(e));
            }
        }
        if let Some(e) = failure {
            self.gpu.failed = Some(e.to_string());
            let _ = self.reset();
            return Err(e);
        }
        result
    }
    fn request_readback_inner(&mut self, rect: Rect) -> Result<Readback, GpuError> {
        let display = self.display()?;
        let size = display.texture.size();
        if rect.is_empty()
            || u64::from(rect.x) + u64::from(rect.width) > u64::from(size.width)
            || u64::from(rect.y) + u64::from(rect.height) > u64::from(size.height)
        {
            return Err(error("読み戻しの矩形が不正"));
        }
        let pitch = (rect.width * 4).div_ceil(256) * 256;
        let bytes = u64::from(pitch) * u64::from(rect.height);
        if bytes > self.gpu.device.limits().max_buffer_size {
            return Err(error("読み戻しがデバイス上限を超える"));
        }
        let mut used = self.readback_used.load(Ordering::Acquire);
        loop {
            let total = used
                .checked_add(bytes)
                .filter(|&v| v <= self.options.readback_budget_bytes)
                .ok_or_else(|| error("読み戻しの予算超過"))?;
            match self.readback_used.compare_exchange_weak(
                used,
                total,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err(current) => used = current,
            }
        }
        let lease = Arc::new(Lease {
            used: Arc::clone(&self.readback_used),
            bytes,
        });
        let buffer = self.buffer(
            bytes,
            wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        );
        let mut encoder = self.gpu.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: display.texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: rect.x,
                    y: rect.y,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(pitch),
                    rows_per_image: Some(rect.height),
                },
            },
            wgpu::Extent3d {
                width: rect.width,
                height: rect.height,
                depth_or_array_layers: 1,
            },
        );
        let submission = self.gpu.queue.submit([encoder.finish()]);
        self.last_copy = Some(submission.clone());
        let held = Arc::clone(&lease);
        self.gpu.queue.on_submitted_work_done(move || drop(held));
        let (tx, receiver) = mpsc::sync_channel(1);
        buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        Ok(Readback {
            device: self.gpu.device.clone(),
            buffer,
            receiver,
            submission,
            owner: self.owner,
            generation: self.generation,
            rect,
            pitch,
            _lease: lease,
        })
    }
    /// 古い要求、別インスタンスの要求は採用しない。破棄した要求も GPU 完了までは予算に残る。
    pub fn finish_readback(&mut self, request: Readback) -> Result<Vec<u8>, GpuError> {
        if request.owner != self.owner
            || request.generation != self.generation
            || self.binding.is_none()
        {
            return Err(error("読み戻しが古い世代または別の表示"));
        }
        self.wait(request.submission.clone())?;
        request
            .receiver
            .recv_timeout(Duration::from_secs(1))
            .map_err(error)?
            .map_err(error)?;
        let mapped = request.buffer.slice(..).get_mapped_range().map_err(error)?;
        let mut output =
            Vec::with_capacity(request.rect.width as usize * request.rect.height as usize * 4);
        for row in mapped.chunks_exact(request.pitch as usize) {
            output.extend_from_slice(&row[..request.rect.width as usize * 4]);
        }
        drop(mapped);
        Ok(output)
    }
    /// 破棄した読み戻しの完了通知を処理する。表示更新がないフレームにも呼べる。
    pub fn poll(&self) -> Result<(), GpuError> {
        self.gpu.device.poll(wgpu::PollType::Poll).map_err(error)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 予算がどこにあっても、`layout` が返す入れ物は「固定の量＋1 束の全入力の常駐（GPU と CPU のコピー）」を予算の中に収める
    /// （束の途中で今の束のタイルを追い出さない前提）。面・命令・調整の表・タイルの大きさ・束のタイル数を振って、予算を細かく走査する。
    #[test]
    fn layout_always_holds_one_whole_batch_inside_the_budget() {
        let limits = wgpu::Limits::default();
        let mut some = 0usize;
        for ts in [16u32, 32] {
            let doc = Document::with_tile_size(53, 37, ts).unwrap();
            for slots in [0usize, 1, 2, 5, 9] {
                for entries in [0usize, 1, 3, 12, 40] {
                    for table_words in [0usize, 7, 300] {
                        for batch_tiles in [1u32, 3, 16] {
                            for budget in (0u64..60_000).step_by(3) {
                                let options = ResidentOptions {
                                    resident_budget_bytes: budget,
                                    batch_tiles,
                                    ..Default::default()
                                };
                                let Some(l) =
                                    layout(&options, &limits, &doc, slots, entries, table_words)
                                        .unwrap()
                                else {
                                    continue;
                                };
                                some += 1;
                                assert!(l.capacity >= 1);
                                assert!(l.capacity <= batch_tiles as usize);
                                let whole_batch = l.per_batch * l.capacity as u64 * 2;
                                assert!(
                                    l.fixed + whole_batch <= budget,
                                    "ts {ts} 面 {slots} 命令 {entries} 表 {table_words} 束 {batch_tiles} 予算 {budget}: \
                                     固定 {} + 1 束 {whole_batch} が予算を超える",
                                    l.fixed
                                );
                            }
                        }
                    }
                }
            }
        }
        assert!(some > 1000, "入れ物が作れる予算を走査できている: {some}");
    }

    /// 束の入れ物が作れる一番小さい予算の 1 バイト下では作れない（余りを捨てて小さくしすぎない）。
    #[test]
    fn layout_is_tight_at_the_smallest_budget() {
        let limits = wgpu::Limits::default();
        let doc = Document::with_tile_size(53, 37, 16).unwrap();
        for (slots, entries) in [(1usize, 1usize), (3, 12), (9, 40)] {
            let at = |budget: u64| {
                let options = ResidentOptions {
                    resident_budget_bytes: budget,
                    batch_tiles: 1,
                    ..Default::default()
                };
                layout(&options, &limits, &doc, slots, entries, 0).unwrap()
            };
            let first = (0u64..200_000)
                .find(|&b| at(b).is_some())
                .expect("作れる予算がある");
            let l = at(first).unwrap();
            assert_eq!(l.capacity, 1);
            assert_eq!(
                l.fixed + l.per_batch * 2,
                first,
                "面 {slots} 命令 {entries}: 作れる最小の予算は、固定の量と 1 束の常駐にちょうど一致する"
            );
            assert!(at(first - 1).is_none());
        }
    }
}
