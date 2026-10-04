//! 正本のタイルの常駐コピーと、読み戻しを伴わない表示更新。
use super::{error, GpuError, GpuPainter, LayerData, Options};
use std::{
    collections::{BTreeSet, HashMap},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Arc,
    },
    time::Duration,
};
use yolu_core::{Channel, Document, LayerId, Rect, TileCoord};

#[derive(Clone, Copy, Debug)]
pub struct ResidentOptions {
    /// 表示テクスチャ、層タイルと同量の CPU コピー、作業域と転送の余裕を含む。
    pub resident_budget_bytes: u64,
    /// 未完了・取得前の読み戻しバッファを合計した別予算。
    pub readback_budget_bytes: u64,
    pub batch_tiles: u32,
}
impl Default for ResidentOptions {
    fn default() -> Self {
        Self {
            resident_budget_bytes: 768 << 20,
            readback_budget_bytes: 64 << 20,
            batch_tiles: 16,
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
    layers: Vec<LayerId>,
}
struct Work {
    input: wgpu::Buffer,
    metadata: wgpu::Buffer,
    params: wgpu::Buffer,
    coords: wgpu::Buffer,
    capacity: usize,
    layer_count: usize,
    fixed_bytes: u64,
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

pub struct ResidentCompositor {
    gpu: GpuPainter,
    options: ResidentOptions,
    pipeline: wgpu::ComputePipeline,
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
        let entries: Vec<_> = [0, 1, 3, 5, 6]
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
        let shader = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("常駐表示"),
                source: wgpu::ShaderSource::Wgsl(include_str!("paint.wgsl").into()),
            });
        let pipeline = gpu
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("常駐表示"),
                layout: Some(&pl),
                module: &shader,
                entry_point: Some("display"),
                compilation_options: Default::default(),
                cache: None,
            });
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
            pipeline,
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
    fn prepare(&mut self, doc: &Document, layers: usize) -> Result<(), GpuError> {
        let ts = u64::from(doc.tile_size());
        let tile = ts * ts * 4;
        let frame = u64::from(doc.width()) * u64::from(doc.height()) * 4;
        let limits = self.gpu.device.limits();
        if doc.width() > limits.max_texture_dimension_2d
            || doc.height() > limits.max_texture_dimension_2d
        {
            return Err(error("表示テクスチャがデバイス上限を超える"));
        }
        let meta = layers.max(1) as u64 * 16;
        // 作業入力と転送ステージング、座標・定数を先に予約。束の全入力を常駐できる最小量も確保。
        let per_batch = tile * layers.max(1) as u64;
        let available = self
            .options
            .resident_budget_bytes
            .saturating_sub(frame + meta * 2 + 32);
        let capacity = u64::from(self.options.batch_tiles)
            .min(available / (per_batch * 4 + 16))
            .min(limits.max_storage_buffer_binding_size / per_batch)
            .min(u64::from(limits.max_compute_workgroups_per_dimension) * 64 / (ts * ts))
            as usize;
        if capacity == 0 || meta > limits.max_storage_buffer_binding_size {
            return Err(error("常駐予算では表示と1束を保持できない"));
        }
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
        if self
            .work
            .as_ref()
            .is_none_or(|w| w.layer_count != layers || w.capacity != capacity)
        {
            self.work = None;
            let fixed = frame + 2 * (per_batch * capacity as u64 + meta + 16 + capacity as u64 * 8);
            while fixed + self.cache.len() as u64 * tile * 2 > self.options.resident_budget_bytes {
                self.evict();
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
                params: self.buffer(
                    16,
                    wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                ),
                coords: self.buffer(
                    capacity as u64 * 8,
                    wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                ),
                capacity,
                layer_count: layers,
                fixed_bytes: fixed,
            });
        }
        Ok(())
    }
    fn evict(&mut self) {
        if let Some((_, key)) = self.lru.pop_first() {
            self.cache.remove(&key);
            self.stats.evicted_tiles += 1;
        }
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
        if channel == Channel::Normal {
            return Err(error("Normal の合成は core が未対応"));
        }
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
        let result = self.update_inner(doc, channel);
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
    fn update_inner(&mut self, doc: &Document, channel: Channel) -> Result<UpdateStats, GpuError> {
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
        let ids: Vec<_> = doc.layers().iter().map(|l| l.id()).collect();
        let structure = self.binding.as_ref().is_none_or(|b| b.layers != ids);
        let coords = if structure {
            (0..doc.height().div_ceil(doc.tile_size()))
                .flat_map(|y| {
                    (0..doc.width().div_ceil(doc.tile_size())).map(move |x| TileCoord::new(x, y))
                })
                .collect()
        } else {
            doc.changed_tiles(channel, self.binding.as_ref().expect("確認済み").serial)
                .ok_or_else(|| error("文書の世代が巻き戻った。reset が必要"))?
        };
        if self
            .binding
            .as_ref()
            .is_some_and(|b| b.revision == doc.revision())
            && !structure
        {
            return Ok(self.account());
        }
        self.generation += 1;
        let dead: Vec<_> = self
            .cache
            .keys()
            .filter(|k| !ids.contains(&k.layer))
            .copied()
            .collect();
        for k in dead {
            if let Some(v) = self.cache.remove(&k) {
                self.lru.remove(&(v.touched, k));
            }
        }
        self.prepare(doc, ids.len())?;
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
        let tile = doc.tile_size() as usize * doc.tile_size() as usize * 4;
        let capacity = self.work.as_ref().expect("準備済み").capacity;
        let mut bytes = vec![0u8; tile];
        for chunk in coords.chunks(capacity) {
            // この束の全タイルを触ってからコピーを記録。予算はこの束を保持できる量以上。
            for &coord in chunk {
                for l in doc.layers() {
                    bytes.fill(0);
                    if let Some(s) = l.surface(channel) {
                        s.copy_tile(coord, &mut bytes)?;
                    }
                    let key = Key {
                        layer: l.id(),
                        coord,
                    };
                    self.clock += 1;
                    if let Some(v) = self.cache.get_mut(&key) {
                        self.lru.remove(&(v.touched, key));
                        if v.bytes != bytes {
                            self.gpu.queue.write_buffer(&v.gpu, 0, &bytes);
                            v.bytes.copy_from_slice(&bytes);
                            self.stats.uploaded_tiles += 1;
                            self.stats.uploaded_bytes += tile as u64;
                        } else {
                            self.stats.cache_hits += 1;
                        }
                        debug_assert!(v.generation <= doc.change_serial());
                        v.generation = doc.change_serial();
                        v.touched = self.clock;
                    } else {
                        while self.work.as_ref().expect("準備済み").fixed_bytes
                            + (self.cache.len() as u64 + 1) * tile as u64 * 2
                            > self.options.resident_budget_bytes
                        {
                            self.evict();
                        }
                        let buffer = self.buffer(
                            tile as u64,
                            wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
                        );
                        self.gpu.queue.write_buffer(&buffer, 0, &bytes);
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
                }
            }
            let w = self.work.as_ref().expect("準備済み");
            let raw_coords: Vec<[u32; 2]> = chunk.iter().map(|c| [c.x, c.y]).collect();
            self.gpu
                .queue
                .write_buffer(&w.coords, 0, bytemuck::cast_slice(&raw_coords));
            if !metadata.is_empty() {
                self.gpu
                    .queue
                    .write_buffer(&w.metadata, 0, bytemuck::cast_slice(&metadata));
            }
            self.gpu.queue.write_buffer(
                &w.params,
                0,
                bytemuck::cast_slice(&[
                    (chunk.len() * tile / 4) as u32,
                    ids.len() as u32,
                    doc.tile_size(),
                    0,
                ]),
            );
            let group = self
                .gpu
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
                    ],
                });
            let mut encoder = self.gpu.device.create_command_encoder(&Default::default());
            for (k, l) in doc.layers().iter().enumerate() {
                for (j, &coord) in chunk.iter().enumerate() {
                    let v = &self.cache[&Key {
                        layer: l.id(),
                        coord,
                    }];
                    encoder.copy_buffer_to_buffer(
                        &v.gpu,
                        0,
                        &w.input,
                        ((k * chunk.len() + j) * tile) as u64,
                        tile as u64,
                    );
                }
            }
            {
                let mut pass = encoder.begin_compute_pass(&Default::default());
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &group, &[]);
                pass.dispatch_workgroups(((chunk.len() * tile / 4) as u32).div_ceil(64), 1, 1);
            }
            self.wait(self.gpu.queue.submit([encoder.finish()]))?;
            self.stats.updated_tiles += chunk.len();
        }
        self.binding = Some(Binding {
            id: doc.id(),
            channel,
            width: doc.width(),
            height: doc.height(),
            ts: doc.tile_size(),
            serial: doc.change_serial(),
            revision: doc.revision(),
            layers: ids,
        });
        Ok(self.account())
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
