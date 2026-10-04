//! CPU の正本を変更しない、タイル合成と丸いダブの compute プレビュー。
//! 読み戻しは同期で完了させ、結果に文書 ID・世代を添える。保存には core のタイルを使う。
use bytemuck::{Pod, Zeroable};
use std::{fmt, sync::mpsc, time::Duration};
use wgpu::util::DeviceExt;
use yolu_core::{BrushSettings, Channel, Document, Rect, TileCoord};

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
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct LayerData {
    opacity: f32,
    mode: u32,
    clip: u32,
    active: u32,
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

pub struct GpuPainter {
    device: wgpu::Device,
    queue: wgpu::Queue,
    layout: wgpu::BindGroupLayout,
    composite: wgpu::ComputePipeline,
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
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let memory = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
        let entries: Vec<_> = (0..5)
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
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("core の合成・ダブ"),
            source: wgpu::ShaderSource::Wgsl(include_str!("paint.wgsl").into()),
        });
        let pipeline = |entry| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                module: &shader,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let composite = pipeline("composite");
        let brush = pipeline("brush");
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
            composite,
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
        if channel == Channel::Normal {
            return Err(error("Normal の合成は core が未対応"));
        }
        for &coord in coords {
            if doc.tile_rect(coord).is_none() {
                return Err(error("タイルが画布の外"));
            }
        }
        let ts = doc.tile_size() as usize;
        let tile_bytes = ts * ts * 4;
        let metadata: Vec<_> = doc
            .layers()
            .iter()
            .enumerate()
            .map(|(i, l)| LayerData {
                opacity: l.opacity() as f32,
                mode: l.blend_mode() as u32,
                clip: u32::from(i > 0 && l.clipping()),
                active: u32::from(
                    l.visible()
                        && l.opacity() > 0.0
                        && l.is_channel_enabled(channel)
                        && l.surface(channel).is_some(),
                ),
            })
            .collect();
        let layer_count = metadata.len();
        let overhead = (metadata.len().max(1) * 16 + 48) as u64 * 2;
        let per_tile = (tile_bytes as u64)
            .checked_mul((layer_count.max(1) as u64 + 2) * 2)
            .ok_or_else(|| error("予算の計算が範囲外"))?;
        let limit = self.device.limits();
        let batch = self.options.budget_bytes.saturating_sub(overhead) / per_tile;
        let batch = batch
            .min(limit.max_storage_buffer_binding_size / (tile_bytes * layer_count.max(1)) as u64)
            .min(u64::from(limit.max_compute_workgroups_per_dimension) * 64 / (ts * ts) as u64)
            .min(64) as usize;
        if batch == 0 && !coords.is_empty() {
            return Err(error("GPU 予算では 1 タイルを処理できない"));
        }
        let mut tiles = Vec::new();
        for chunk in coords.chunks(batch.max(1)) {
            let mut input = vec![0u8; chunk.len() * tile_bytes * layer_count.max(1)];
            for (k, l) in doc.layers().iter().enumerate() {
                if let Some(s) = l.surface(channel) {
                    for (j, &coord) in chunk.iter().enumerate() {
                        let offset = (k * chunk.len() + j) * tile_bytes;
                        s.copy_tile(coord, &mut input[offset..offset + tile_bytes])?;
                    }
                }
            }
            let params = [
                (chunk.len() * ts * ts) as u32,
                layer_count as u32,
                ts as u32,
                0,
            ];
            let empty = [LayerData::zeroed()];
            let output = self.run(
                &input,
                bytemuck::cast_slice(if metadata.is_empty() {
                    &empty
                } else {
                    &metadata
                }),
                &[0u8; 32],
                params,
                false,
            )?;
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
        self.run(
            start,
            &[0; 16],
            bytemuck::cast_slice(if data.is_empty() { &empty } else { &data }),
            [count as u32, 0, width, dabs.len() as u32],
            true,
        )
    }

    fn run(
        &mut self,
        input: &[u8],
        layers: &[u8],
        dabs: &[u8],
        params: [u32; 4],
        brush: bool,
    ) -> Result<Vec<u8>, GpuError> {
        if let Some(reason) = &self.failed {
            return Err(error(format!("GPU は再作成が必要: {reason}")));
        }
        let output_bytes = u64::from(params[0]) * 4;
        let total =
            (input.len() as u64 + layers.len() as u64 + dabs.len() as u64 + 16 + output_bytes * 2)
                * 2;
        let limit = self.device.limits();
        if total > self.options.budget_bytes {
            return Err(error("GPU 作業バッファの予算超過"));
        }
        if [
            input.len() as u64,
            layers.len() as u64,
            dabs.len() as u64,
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
            let entries: Vec<_> = [&src, &meta, &dst, &uniform, &dab]
                .iter()
                .enumerate()
                .map(|(i, b)| wgpu::BindGroupEntry {
                    binding: i as u32,
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
                pass.set_pipeline(if brush { &self.brush } else { &self.composite });
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
        if let Some(gpu) = &mut self.gpu {
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
                .ok_or_else(|| error("タイルが画布の外"))?;
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

mod resident;
pub use resident::{Display, Readback, ResidentCompositor, ResidentOptions, UpdateStats};
