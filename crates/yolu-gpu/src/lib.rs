//! CPU の正本を変更しない、タイル合成と丸いダブの compute プレビュー。
//! 読み戻しは同期で完了させ、結果に文書 ID・世代を添える。保存には core のタイルを使う。
use bytemuck::{Pod, Zeroable};
use std::{fmt, sync::mpsc, time::Duration};
use wgpu::util::DeviceExt;
use yolu_core::{BrushSettings, Channel, Document, Rect, TileCoord};

mod plan;
mod source;
use plan::Plan;
pub use plan::{supports, Unsupported};

/// 装置が、タイルの合成（`GpuPainter`・`ResidentCompositor` の compute のシェーダー）に要る上限を持つか。storage の入れ物を 1 段に 7 つ
/// （合成・ダブの束ね）、storage のテクスチャを 1 つ（表示）、1 つの組に 64 の呼び（`@workgroup_size(64)`）。WebGL2 の上限で作った装置
/// （egui-wgpu が OpenGL のときに作る）は storage も compute も持たない。
pub fn device_can_composite(limits: &wgpu::Limits) -> bool {
    limits.max_storage_buffers_per_shader_stage >= 7
        && limits.max_storage_textures_per_shader_stage >= 1
        && limits.max_storage_buffer_binding_size > 0
        && limits.max_compute_workgroups_per_dimension > 0
        && limits.max_compute_invocations_per_workgroup >= 64
        && limits.max_compute_workgroup_size_x >= 64
}

#[derive(Debug)]
pub struct GpuError(pub String);
impl fmt::Display for GpuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for GpuError {}
impl From<yolu_core::CoreError> for GpuError {
    fn from(e: yolu_core::CoreError) -> Self {
        Self(e.to_string())
    }
}
fn error(e: impl fmt::Display) -> GpuError {
    GpuError(e.to_string())
}

/// GPU の作業バッファ（入力・出力・読み戻し・転送用の余裕）の上限。
#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub budget_bytes: u64,
    pub force_fallback_adapter: bool,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            budget_bytes: 64 << 20,
            force_fallback_adapter: false,
        }
    }
}

/// タイルの有効矩形だけを持つ、左下からの straight RGBA8。
#[derive(Debug)]
pub struct TileResult {
    pub coord: TileCoord,
    pub rect: Rect,
    pub pixels: Vec<u8>,
}
#[derive(Debug)]
pub struct CompositeResult {
    pub document_id: u128,
    pub revision: u64,
    pub channel: Channel,
    pub tiles: Vec<TileResult>,
}
impl CompositeResult {
    /// 同じ ID の別の読み込みも revision が一致し得るため、呼び手は文書を差し替えたら結果を捨てる。
    pub fn is_current(&self, doc: &Document) -> bool {
        self.document_id == doc.id() && self.revision == doc.revision()
    }
}

/// 補間済みの丸い判子。ストロークの点の補間・Undo・保存は core が受け持つ。
#[derive(Clone, Copy, Debug)]
pub struct Dab {
    pub x: f64,
    pub y: f64,
    pub pressure: f64,
}
/// 合成の 1 つの命令（シェーダーの `Layer` と同じ並び・32 バイト）。描かないレイヤーは計画に入れない。命令の種類と並びは `plan.rs`。
/// 調整の命令は面も塗りつぶしの色も持たないので、`slot` に調整の式の種類（`plan::adj`）、`fill` に値・表の語の番号を置く。
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Pod, Zeroable)]
struct LayerData {
    /// 命令の種類（`plan::op`）。
    kind: u32,
    /// そのチャンネルでの不透明度。
    opacity: f32,
    /// 合成モードの番号（`BlendMode as u32`）。
    mode: u32,
    /// 上げた面の番号（NONE は塗りつぶし）。
    slot: u32,
    /// マスクの面の番号（NONE はマスク無し）。アルファが隠す量。
    mask: u32,
    /// 塗りつぶしの色（RGBA8 をリトルエンディアンで）。
    fill: u32,
    /// マスクを反転するか。
    mask_invert: u32,
    /// マスクの濃度。
    mask_density: f32,
}
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct DabData {
    x: f32,
    y: f32,
    radius: f32,
    hardness: f32,
    ceiling: f32,
    flow: f32,
    color: u32,
    erase: u32,
}

/// 命令列を流すシェーダーのパイプライン（`variant` ごとに別のシェーダー）。`entry` はシェーダーの入り口の名前。
fn shader_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    variant: plan::Variant,
    entry: &str,
) -> wgpu::ComputePipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("yolu-core-composite-dab"),
        source: wgpu::ShaderSource::Wgsl(plan::shader_source(variant).into()),
    });
    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(entry),
        layout: Some(layout),
        module: &module,
        entry_point: Some(entry),
        compilation_options: Default::default(),
        cache: None,
    })
}

/// 1 回の実行の入力（合成もブラシも同じ束縛で流す）。
struct Job<'a> {
    input: &'a [u8],
    /// 命令の並び（合成）。ブラシでは 1 つの空の命令。
    layers: &'a [u8],
    /// ダブの並び（ブラシ）。合成では 1 つの空のダブ。
    dabs: &'a [u8],
    /// 調整の表と値（合成）。
    tables: &'a [u8],
    /// 面ごと・束のタイルごとの、画素の有無（合成）。ブラシでは空。
    presence: &'a [u8],
    /// タイルごとの命令の番号の列（合成。`Plan::tile_program`）。ブラシでは空。
    programs: &'a [u8],
    params: [u32; 4],
    brush: bool,
    variant: plan::Variant,
}

/// `run` が予算に数える作業バッファの量（GPU の入力・出力と、転送用の写しで 2 つ分）。出力は読み戻し用にもう 1 つ数える。
fn work_bytes(
    input: u64,
    layers: u64,
    dabs: u64,
    tables: u64,
    presence: u64,
    programs: u64,
    output: u64,
) -> u64 {
    (input
        .saturating_add(layers)
        .saturating_add(dabs)
        .saturating_add(tables)
        .saturating_add(presence)
        .saturating_add(programs)
        .saturating_add(16)
        .saturating_add(output.saturating_mul(2)))
    .saturating_mul(2)
}

/// 合成の束（`tiles` タイル）を `run` に渡したときに数える量の上限。`composite_tiles` が予算から束のタイル数を決めるのに使い、`run` と
/// 同じ式（`work_bytes`）で数える。命令の番号の列は、全部のレイヤーが描かれたタイルの最大（先頭の（始まり, 長さ）と命令の数）で数える。
fn composite_bytes(plan: &Plan, layer_count: usize, tile_bytes: u64, tiles: u64) -> u64 {
    let slots = plan.slots.len().max(1) as u64;
    work_bytes(
        tiles.saturating_mul(tile_bytes).saturating_mul(slots),
        layer_count.max(1) as u64 * std::mem::size_of::<LayerData>() as u64,
        32,
        plan.tables.len() as u64 * 4,
        tiles.saturating_mul(slots * 4),
        tiles.saturating_mul((2 + plan.entries.len().max(1) as u64) * 4),
        tiles.saturating_mul(tile_bytes),
    )
}

/// 束のタイルごとの命令の番号の列。先頭にタイルごとの（始まり, 長さ）の組、続けて各タイルの命令の番号。`presence` は面ごと・束のタイルごと
/// の画素の有無（`面の番号 × 束のタイルの数 + 束の中のタイルの番号`）。
fn tile_programs(plan: &Plan, presence: &[u32], tiles: usize) -> Vec<u32> {
    let mut out = vec![0u32; tiles * 2];
    for j in 0..tiles {
        let start = out.len();
        plan.tile_program(&|slot| presence[slot as usize * tiles + j] != 0, &mut out);
        out[j * 2] = start as u32;
        out[j * 2 + 1] = (out.len() - start) as u32;
    }
    out
}

pub struct GpuPainter {
    device: wgpu::Device,
    queue: wgpu::Queue,
    layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    /// 合成のパイプライン（シェーダーの形ごと。グループも調整も無い形は作るときに作り、ほかは初めて要るときに作る）。
    composite: std::collections::HashMap<plan::Variant, wgpu::ComputePipeline>,
    brush: wgpu::ComputePipeline,
    info: wgpu::AdapterInfo,
    options: Options,
    failed: Option<String>,
}
impl GpuPainter {
    /// アダプター・デバイス・シェーダーが使えなければ理由を返す。
    pub fn new(options: Options) -> Result<Self, GpuError> {
        pollster::block_on(Self::create(options))
    }
    async fn create(options: Options) -> Result<Self, GpuError> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                force_fallback_adapter: options.force_fallback_adapter,
                ..Default::default()
            })
            .await
            .map_err(|e| error(format!("GPU 利用不可: {e}")))?;
        let info = adapter.get_info();
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("タイル合成"),
                ..Default::default()
            })
            .await
            .map_err(|e| error(format!("GPU 利用不可: {e}")))?;
        Self::from_parts(info, device, queue, options).await
    }
    /// 呼び手が持つデバイスで作る（アプリの表示と同じデバイスを渡すと、表示のテクスチャをそのまま見せられる）。
    /// デバイスとキューは呼び手のもので、ここでは作り直さない。パイプラインが作れなければ（compute や storage の上限が
    /// 足りないデバイスなど）理由を返す。
    pub fn from_device(
        info: wgpu::AdapterInfo,
        device: wgpu::Device,
        queue: wgpu::Queue,
        options: Options,
    ) -> Result<Self, GpuError> {
        pollster::block_on(Self::from_parts(info, device, queue, options))
    }
    async fn from_parts(
        info: wgpu::AdapterInfo,
        device: wgpu::Device,
        queue: wgpu::Queue,
        options: Options,
    ) -> Result<Self, GpuError> {
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let memory = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
        let entries: Vec<_> = [0, 1, 2, 3, 4, 7, 8, 9]
            .into_iter()
            .map(|binding| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: if binding == 3 {
                        wgpu::BufferBindingType::Uniform
                    } else {
                        wgpu::BufferBindingType::Storage {
                            read_only: binding != 2,
                        }
                    },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            })
            .collect();
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &entries,
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let flat = plan::Variant::FLAT;
        let composite = shader_pipeline(&device, &pipeline_layout, flat, "composite");
        let brush = shader_pipeline(&device, &pipeline_layout, flat, "brush");
        let mut failure = None;
        for scope in [internal, memory, validation] {
            if let Some(e) = scope.pop().await {
                failure = Some(error(e));
            }
        }
        if let Some(e) = failure {
            return Err(e);
        }
        Ok(Self {
            device,
            queue,
            layout,
            pipeline_layout,
            composite: std::collections::HashMap::from([(flat, composite)]),
            brush,
            info,
            options,
            failed: None,
        })
    }
    pub fn adapter_info(&self) -> &wgpu::AdapterInfo {
        &self.info
    }

    /// 指定タイルだけを合成する。変更の一覧は Document::changed_tiles から渡せる。
    pub fn composite_tiles(
        &mut self,
        doc: &Document,
        channel: Channel,
        coords: &[TileCoord],
    ) -> Result<CompositeResult, GpuError> {
        for &coord in coords {
            if doc.tile_rect(coord).is_none() {
                return Err(error("タイルがキャンバスの外"));
            }
        }
        let plan = Plan::build(doc, channel, true).map_err(error)?;
        let ts = doc.tile_size() as usize;
        let tile_bytes = ts * ts * 4;
        let metadata = plan.metadata();
        let layer_count = metadata.len();
        let slot_count = plan.slots.len();
        let limit = self.device.limits();
        // 束のタイル数は、`run` が数える量（入力・出力・命令・調整の表・タイルの有無の印・命令の番号の列）が予算に収まる最大
        let mut batch = (limit.max_storage_buffer_binding_size
            / (tile_bytes * slot_count.max(1)) as u64)
            .min(u64::from(limit.max_compute_workgroups_per_dimension) * 64 / (ts * ts) as u64)
            .min(64) as usize;
        while batch > 0
            && composite_bytes(&plan, metadata.len(), tile_bytes as u64, batch as u64)
                > self.options.budget_bytes
        {
            batch -= 1;
        }
        if batch == 0 && !coords.is_empty() {
            return Err(error("GPU 予算では 1 タイルを処理できない"));
        }
        let mut fetcher = source::Fetcher::new(doc, channel, slot_count);
        let mut tiles = Vec::new();
        for chunk in coords.chunks(batch.max(1)) {
            let mut input = vec![0u8; chunk.len() * tile_bytes * slot_count.max(1)];
            let mut presence = vec![0u32; chunk.len() * slot_count.max(1)];
            for (k, slot) in plan.slots.iter().enumerate() {
                fetcher.prefetch(k, slot, chunk)?;
                for (j, &coord) in chunk.iter().enumerate() {
                    let offset = (k * chunk.len() + j) * tile_bytes;
                    presence[k * chunk.len() + j] = u32::from(fetcher.tile(
                        k,
                        slot,
                        coord,
                        &mut input[offset..offset + tile_bytes],
                    )?);
                }
                fetcher.release(k);
            }
            let programs = tile_programs(&plan, &presence, chunk.len());
            let params = [
                (chunk.len() * ts * ts) as u32,
                layer_count as u32,
                ts as u32,
                0,
            ];
            let empty = [LayerData::zeroed()];
            let output = self.run(Job {
                input: &input,
                layers: bytemuck::cast_slice(if metadata.is_empty() {
                    &empty
                } else {
                    &metadata
                }),
                dabs: &[0u8; 32],
                tables: bytemuck::cast_slice(&plan.tables),
                presence: bytemuck::cast_slice(&presence),
                programs: bytemuck::cast_slice(&programs),
                params,
                brush: false,
                variant: plan.variant,
            })?;
            for (j, &coord) in chunk.iter().enumerate() {
                let rect = doc.tile_rect(coord).expect("確認済み");
                let mut pixels = Vec::with_capacity(rect.width as usize * rect.height as usize * 4);
                for y in 0..rect.height as usize {
                    let offset = j * tile_bytes + y * ts * 4;
                    pixels.extend_from_slice(&output[offset..offset + rect.width as usize * 4]);
                }
                tiles.push(TileResult {
                    coord,
                    rect,
                    pixels,
                });
            }
        }
        Ok(CompositeResult {
            document_id: doc.id(),
            revision: doc.revision(),
            channel,
            tiles,
        })
    }

    /// 同じストロークの判子を順番に適用する。start はストローク開始前の矩形。
    /// 戻り値はプレビューのみ。取消は戻り値を捨て、正本への描画は core の Stroke で行う。
    pub fn brush_dabs(
        &mut self,
        width: u32,
        height: u32,
        start: &[u8],
        settings: &BrushSettings,
        dabs: &[Dab],
    ) -> Result<Vec<u8>, GpuError> {
        settings.validate()?;
        // 縁のアンチエイリアス（帯・小さなダブの濃さ）は core のストロークだけが持つ。黙って今の式で描かない
        if settings.anti_alias != yolu_core::AntiAlias::None {
            return Err(error(
                "ブラシのアンチエイリアスは GPU のプレビューでは描けない（core のストロークで描く）",
            ));
        }
        let count = u64::from(width) * u64::from(height);
        if width == 0 || height == 0 || count > u32::MAX as u64 || count * 4 != start.len() as u64 {
            return Err(error("ブラシの矩形と入力が不正"));
        }
        if dabs.len() > 1_000_000
            || (dabs.len().max(1) as u64 * 32 + count * 12 + 32) * 2 > self.options.budget_bytes
        {
            return Err(error("ブラシのダブ数または GPU 予算の上限超過"));
        }
        let mut data = Vec::with_capacity(dabs.len());
        for d in dabs {
            if !d.x.is_finite()
                || !d.y.is_finite()
                || d.x.abs() > 10_000_000.0
                || d.y.abs() > 10_000_000.0
                || !(0.0..=1.0).contains(&d.pressure)
            {
                return Err(error("ダブの座標または筆圧が不正"));
            }
            data.push(DabData {
                x: d.x as f32,
                y: d.y as f32,
                radius: (settings.radius
                    * if settings.pressure_size {
                        d.pressure
                    } else {
                        1.0
                    }) as f32,
                hardness: settings.hardness as f32,
                ceiling: (settings.opacity
                    * if settings.pressure_opacity {
                        d.pressure
                    } else {
                        1.0
                    }) as f32,
                flow: (settings.flow
                    * if settings.pressure_flow {
                        d.pressure
                    } else {
                        1.0
                    }) as f32,
                color: u32::from_le_bytes(settings.color.to_array()),
                erase: u32::from(settings.erase),
            });
        }
        let empty = [DabData::zeroed()];
        self.run(Job {
            input: start,
            layers: bytemuck::bytes_of(&LayerData::zeroed()),
            dabs: bytemuck::cast_slice(if data.is_empty() { &empty } else { &data }),
            tables: &[],
            presence: &[],
            programs: &[],
            params: [count as u32, 0, width, dabs.len() as u32],
            brush: true,
            variant: plan::Variant::FLAT,
        })
    }

    fn run(&mut self, job: Job<'_>) -> Result<Vec<u8>, GpuError> {
        let Job {
            input,
            layers,
            dabs,
            tables,
            presence,
            programs,
            params,
            brush,
            variant,
        } = job;
        if let Some(reason) = &self.failed {
            return Err(error(format!("GPU は再作成が必要: {reason}")));
        }
        let output_bytes = u64::from(params[0]) * 4;
        let total = work_bytes(
            input.len() as u64,
            layers.len() as u64,
            dabs.len() as u64,
            tables.len() as u64,
            presence.len() as u64,
            programs.len() as u64,
            output_bytes,
        );
        let limit = self.device.limits();
        if total > self.options.budget_bytes {
            return Err(error("GPU 作業バッファの予算超過"));
        }
        if [
            input.len() as u64,
            layers.len() as u64,
            dabs.len() as u64,
            tables.len() as u64,
            presence.len() as u64,
            programs.len() as u64,
            output_bytes,
        ]
        .iter()
        .any(|&n| n > limit.max_storage_buffer_binding_size)
            || params[0].div_ceil(64) > limit.max_compute_workgroups_per_dimension
        {
            return Err(error("GPU のバッファまたはディスパッチ上限超過"));
        }
        let validation = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let memory = self.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let internal = self.device.push_error_scope(wgpu::ErrorFilter::Internal);
        // グループ・調整のある文書で初めて使う形のシェーダーは、ここで作る（作れなければ下の誤りの検査で失敗にする）
        if !brush && !self.composite.contains_key(&variant) {
            let made = shader_pipeline(&self.device, &self.pipeline_layout, variant, "composite");
            self.composite.insert(variant, made);
        }
        let result = (|| {
            let buffer = |bytes: &[u8], usage| {
                self.device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: None,
                        contents: bytes,
                        usage,
                    })
            };
            let src = buffer(input, wgpu::BufferUsages::STORAGE);
            let meta = buffer(layers, wgpu::BufferUsages::STORAGE);
            let dab = buffer(dabs, wgpu::BufferUsages::STORAGE);
            // 空の束縛は作れないので、表が無いときは 1 語の 0 を渡す（式の種類が表を引かなければ読まれない）
            let table = buffer(
                if tables.is_empty() {
                    &[0u8; 16]
                } else {
                    tables
                },
                wgpu::BufferUsages::STORAGE,
            );
            let flags = buffer(
                if presence.is_empty() {
                    &[0u8; 16]
                } else {
                    presence
                },
                wgpu::BufferUsages::STORAGE,
            );
            let program = buffer(
                if programs.is_empty() {
                    &[0u8; 16]
                } else {
                    programs
                },
                wgpu::BufferUsages::STORAGE,
            );
            let uniform = buffer(bytemuck::cast_slice(&params), wgpu::BufferUsages::UNIFORM);
            let dst = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: output_bytes,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            let read = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: output_bytes,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let entries: Vec<_> = [
                (0, &src),
                (1, &meta),
                (2, &dst),
                (3, &uniform),
                (4, &dab),
                (7, &table),
                (8, &flags),
                (9, &program),
            ]
            .iter()
            .map(|&(binding, b)| wgpu::BindGroupEntry {
                binding,
                resource: b.as_entire_binding(),
            })
            .collect();
            let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &self.layout,
                entries: &entries,
            });
            let mut encoder = self.device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_compute_pass(&Default::default());
                pass.set_pipeline(if brush {
                    &self.brush
                } else {
                    &self.composite[&variant]
                });
                pass.set_bind_group(0, &group, &[]);
                pass.dispatch_workgroups(params[0].div_ceil(64), 1, 1);
            }
            encoder.copy_buffer_to_buffer(&dst, 0, &read, 0, output_bytes);
            let submission = self.queue.submit([encoder.finish()]);
            let (tx, rx) = mpsc::sync_channel(1);
            read.slice(..).map_async(wgpu::MapMode::Read, move |r| {
                let _ = tx.send(r);
            });
            self.device
                .poll(wgpu::PollType::Wait {
                    submission_index: Some(submission),
                    timeout: Some(Duration::from_secs(30)),
                })
                .map_err(error)?;
            rx.recv_timeout(Duration::from_secs(1))
                .map_err(error)?
                .map_err(error)?;
            let bytes = read.slice(..).get_mapped_range().map_err(error)?.to_vec();
            read.unmap();
            Ok(bytes)
        })();
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
        // 時間切れのバッファはドライバーが所有し続け得るため、次の投入で予算を積み増さない。
        if let Err(e) = &result {
            self.failed = Some(e.to_string());
        }
        result
    }
}

/// GPU 初期化・実行の失敗理由を保った CPU フォールバック。
pub struct Compositor {
    gpu: Option<GpuPainter>,
    reason: Option<String>,
}
impl Compositor {
    pub fn new(options: Options) -> Self {
        match GpuPainter::new(options) {
            Ok(gpu) => Self {
                gpu: Some(gpu),
                reason: None,
            },
            Err(e) => Self {
                gpu: None,
                reason: Some(e.to_string()),
            },
        }
    }
    pub fn fallback_reason(&self) -> Option<&str> {
        self.reason.as_deref()
    }
    pub fn composite_tiles(
        &mut self,
        doc: &Document,
        channel: Channel,
        coords: &[TileCoord],
    ) -> Result<CompositeResult, GpuError> {
        // GPU が扱えない文書は、この呼び出しだけ CPU で合成する（GPU は使い続ける。文書が変われば扱えることもある）。
        let supported = supports(doc, channel).is_ok();
        if let (true, Some(gpu)) = (supported, &mut self.gpu) {
            match gpu.composite_tiles(doc, channel, coords) {
                Ok(r) => return Ok(r),
                Err(e) => {
                    self.reason = Some(e.to_string());
                    self.gpu = None;
                }
            }
        }
        let mut tiles = Vec::new();
        for &coord in coords {
            let rect = doc
                .tile_rect(coord)
                .ok_or_else(|| error("タイルがキャンバスの外"))?;
            let mut pixels = vec![0; rect.width as usize * rect.height as usize * 4];
            doc.composite_into(channel, rect, &mut pixels, yolu_core::RowOrder::BottomUp)?;
            tiles.push(TileResult {
                coord,
                rect,
                pixels,
            });
        }
        Ok(CompositeResult {
            document_id: doc.id(),
            revision: doc.revision(),
            channel,
            tiles,
        })
    }
}

mod bake;
pub use bake::{
    bake_mesh_maps, is_off_value, ray_query_env_allows, BakeAdapter, BakeBackend, BakeGpu, BakeRun,
    FallbackKind, GpuBakeError, GpuBakeMethod, GpuBakeOptions, GpuBakeSlot, GpuBakeStats, GpuBaked,
    MidFailure, RayQueryWhy,
};
mod resident;
pub use resident::{
    resident_requirements, Display, Readback, Requirements, ResidentCompositor, ResidentOptions,
    UpdateStats,
};

#[cfg(test)]
mod tests {
    use super::*;
    use yolu_core::{AdjustmentSettings, BlendMode, Rgba8};

    #[test]
    fn webgl2_limits_cannot_composite_but_the_default_limits_can() {
        assert!(device_can_composite(&wgpu::Limits::default()));
        assert!(!device_can_composite(
            &wgpu::Limits::downlevel_webgl2_defaults()
        ));
    }

    /// 面・命令・調整の表が多い文書（クリッピング・マスク・独立のグループ・表を引く調整）。
    fn busy_doc() -> Document {
        let mut d = Document::with_tile_size(37, 21, 16).unwrap();
        let mut seed = 0x2468_ace1u32;
        let mut paint = |d: &mut Document, name: &str| {
            let l = d.add_layer(name).unwrap();
            for y in 0..d.height() {
                for x in 0..d.width() {
                    seed ^= seed << 13;
                    seed ^= seed >> 17;
                    seed ^= seed << 5;
                    let alpha = [255, 170, 60][(x as usize + y as usize) % 3];
                    d.set_pixel(
                        l,
                        x,
                        y,
                        Rgba8::new(seed as u8, (seed >> 8) as u8, (seed >> 16) as u8, alpha),
                    )
                    .unwrap();
                }
            }
            l
        };
        let _base = paint(&mut d, "下地");
        let a = paint(&mut d, "a");
        d.add_layer_mask(a).unwrap();
        for y in 0..d.height() {
            for x in 0..d.width() {
                d.set_mask_pixel(a, x, y, (x * 7 + y * 3) as u8).unwrap();
            }
        }
        let adj = d
            .add_adjustment_layer(
                "調整",
                AdjustmentSettings::levels(0.2, 0.8, 1.7, 0.1, 0.9).unwrap(),
                None,
                Some(a),
            )
            .unwrap();
        d.set_layer_clipping(adj, true).unwrap();
        let c = paint(&mut d, "c");
        d.set_layer_clipping(c, true).unwrap();
        d.set_layer_blend_mode(c, BlendMode::Multiply).unwrap();
        let l1 = paint(&mut d, "l1");
        let inner = d
            .add_adjustment_layer("中の調整", AdjustmentSettings::invert(), None, Some(l1))
            .unwrap();
        let grp = d.group_layers(&[l1, inner], "組").unwrap();
        d.set_layer_blend_mode(grp, BlendMode::Normal).unwrap();
        d
    }

    fn all_coords(d: &Document) -> Vec<TileCoord> {
        (0..d.height().div_ceil(d.tile_size()))
            .flat_map(|y| (0..d.width().div_ceil(d.tile_size())).map(move |x| TileCoord::new(x, y)))
            .collect()
    }

    /// 束の見積もり（`composite_bytes`）は、実際に `run` へ渡す量（全部のレイヤーが描かれたタイルの束）以上で、予算から束のタイル数を決めても
    /// `run` の予算の検査に引っかからない。タイルごとの面の数・命令の数・調整の表を含めて数える。
    #[test]
    fn composite_batch_estimate_covers_what_run_counts() {
        let d = busy_doc();
        let plan = Plan::build(&d, Channel::Color, true).unwrap();
        assert!(plan.slots.len() > 3 && plan.entries.len() > 6 && !plan.tables.is_empty());
        let tile_bytes = (d.tile_size() * d.tile_size() * 4) as u64;
        let metadata = plan.metadata();
        for tiles in 1..=64usize {
            let presence = vec![1u32; tiles * plan.slots.len()];
            let programs = tile_programs(&plan, &presence, tiles);
            let real = work_bytes(
                tiles as u64 * tile_bytes * plan.slots.len() as u64,
                (metadata.len() * std::mem::size_of::<LayerData>()) as u64,
                32,
                plan.tables.len() as u64 * 4,
                presence.len() as u64 * 4,
                programs.len() as u64 * 4,
                tiles as u64 * tile_bytes,
            );
            let estimate = composite_bytes(&plan, metadata.len(), tile_bytes, tiles as u64);
            assert!(
                real <= estimate,
                "{tiles} タイル: 実際 {real} > 見積もり {estimate}"
            );
            // 過大に見積もりすぎない: 命令の番号の列を全命令で数える差（束ごとに 2 つ分）だけ
            assert!(
                estimate - real <= (tiles * plan.entries.len() * 8) as u64,
                "{tiles} タイル: 実際 {real}、見積もり {estimate}"
            );
        }
    }

    /// 予算を 1 バイトずつ動かして、束のタイル数が 1・2 になる境の前後を通る。束のタイル数は予算に収まる最大で、`run` の予算の検査で
    /// 断られず（断られると表示の合成が以後ずっと CPU になる）、収まらない予算は「1 タイルを処理できない」と言う。
    #[test]
    fn composite_tiles_never_fails_the_run_budget_around_batch_boundaries() {
        let mut painter = match GpuPainter::new(Options::default()) {
            Ok(p) => p,
            Err(e) => {
                assert!(e.to_string().starts_with("GPU 利用不可:"), "{e}");
                eprintln!("GPU の試験をスキップ: {e}");
                // CI の画面の試験のジョブは YOLUPAINTER_REQUIRE_GPU を付け、アダプターを取れないときに通った扱いにしない
                assert!(
                    std::env::var_os("YOLUPAINTER_REQUIRE_GPU").is_none(),
                    "YOLUPAINTER_REQUIRE_GPU があるのに GPU を使えない: {e}"
                );
                return;
            }
        };
        let d = busy_doc();
        let plan = Plan::build(&d, Channel::Color, true).unwrap();
        let tile_bytes = (d.tile_size() * d.tile_size() * 4) as u64;
        let coords = all_coords(&d);
        let slack = (8 * (plan.slots.len() + plan.entries.len() + 2) * 2) as u64 + 16;
        let mut worked = 0;
        for tiles in [1u64, 2] {
            let boundary = composite_bytes(&plan, plan.entries.len(), tile_bytes, tiles);
            for budget in boundary.saturating_sub(slack)..boundary + 8 {
                painter.options.budget_bytes = budget;
                match painter.composite_tiles(&d, Channel::Color, &coords[..tiles as usize]) {
                    Ok(out) => {
                        assert!(budget >= boundary || tiles > 1, "{budget} < {boundary}");
                        for t in &out.tiles {
                            let expected = d.composite(t.rect).unwrap();
                            let max = expected
                                .iter()
                                .zip(&t.pixels)
                                .map(|(a, b)| a.abs_diff(*b))
                                .max()
                                .unwrap();
                            assert!(max <= 2, "予算 {budget}: 最大差 {max}");
                        }
                        worked += 1;
                    }
                    Err(e) => assert!(
                        e.to_string().contains("1 タイルを処理できない"),
                        "予算 {budget}（束 {tiles} の境 {boundary}）: {e}"
                    ),
                }
            }
        }
        assert!(worked > 0, "境を越える予算で動いている");
    }
}
