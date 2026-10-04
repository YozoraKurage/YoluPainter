//! 非破壊フィルター。入力は左下原点の straight RGBA8、マスクは隠す量。
//! 文書・履歴・キャッシュに依存せず、成功時だけ完成した領域を返す。
//! C# FilterEngine のアルゴリズム版 1。カーブ・HSL は調整層の責務。
//! Normalize の全域統計は `statistics` で単独に求められ、`Options::statistics` で評価へ渡せる（タイルごとの再走査を避けられる）。

mod pixels;
#[cfg(test)]
mod tests;
use crate::{
    math::{clamp01, to_byte},
    Rect,
};
use rayon::prelude::*;
use std::{
    fmt,
    sync::atomic::{AtomicBool, Ordering},
};

pub const MAX_STACK: usize = 32;
pub const MAX_HALO: u32 = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueType {
    Color,
    Scalar,
    TangentNormal,
    Mask,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Locality {
    Point,
    Neighborhood,
    Global,
}

/// Generator のマップ解決は呼び出し側。値は 0..1、欠損は None。
/// 同じ座標は評価中いつも同じ値を返すこと。ランプ適用後は Mapped を返す。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Generated {
    Scalar(f64),
    Mapped([u8; 4]),
}
pub trait GeneratorInput: Sync {
    fn sample(&self, slot: u32, x: u32, y: u32) -> Option<Generated>;
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeneratorBlend {
    Multiply,
    Replace,
    Screen,
    Max,
    Min,
    Add,
    Subtract,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Settings {
    GaussianBlur {
        radius: u32,
    },
    Sharpen {
        radius: u32,
        amount: f64,
        threshold: u32,
    },
    Noise {
        amount: f64,
        seed: i32,
        monochrome: bool,
    },
    Levels {
        input_black: f64,
        input_white: f64,
        gamma: f64,
        output_black: f64,
        output_white: f64,
    },
    Invert,
    Normalize,
    /// マップの種類・ランプの計算ではなく、解決済み Generator 出力の合成段。
    Generator {
        slot: u32,
        blend: GeneratorBlend,
    },
}
impl Settings {
    pub const ALGORITHM_VERSION: u32 = 1;
    pub fn halo(&self) -> u32 {
        match *self {
            Self::GaussianBlur { radius } | Self::Sharpen { radius, .. } => radius,
            _ => 0,
        }
    }
    pub fn locality(&self) -> Locality {
        if matches!(self, Self::Normalize) {
            Locality::Global
        } else if self.halo() > 0 {
            Locality::Neighborhood
        } else {
            Locality::Point
        }
    }
    pub fn validate(&self, value_type: ValueType) -> Result<(), Error> {
        let valid = match *self {
            Self::GaussianBlur { radius } => (1..=256).contains(&radius),
            Self::Sharpen {
                radius,
                amount,
                threshold,
            } => {
                (1..=64).contains(&radius)
                    && amount.is_finite()
                    && (0.0..=5.0).contains(&amount)
                    && threshold <= 255
            }
            Self::Noise { amount, .. } => amount.is_finite() && (0.0..=1.0).contains(&amount),
            Self::Levels {
                input_black: b,
                input_white: w,
                gamma: g,
                output_black: ob,
                output_white: ow,
            } => {
                [b, w, g, ob, ow].iter().all(|v| v.is_finite())
                    && b >= 0.0
                    && w <= 1.0
                    && w - b >= 1.0 / 255.0
                    && (0.1..=9.99).contains(&g)
                    && (0.0..=1.0).contains(&ob)
                    && (0.0..=1.0).contains(&ow)
            }
            _ => true,
        };
        if !valid {
            return Err(Error::Invalid("フィルターの設定が範囲外です"));
        }
        if value_type == ValueType::TangentNormal && !matches!(self, Self::GaussianBlur { .. }) {
            return Err(Error::Invalid(
                "接空間法線には再正規化するぼかしだけを適用できます",
            ));
        }
        if matches!(value_type, ValueType::Scalar | ValueType::Mask)
            && matches!(
                self,
                Self::Noise {
                    monochrome: false,
                    ..
                }
            )
        {
            return Err(Error::Invalid("スカラーとマスクは色ノイズを受け付けません"));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Stage {
    pub settings: Settings,
    pub enabled: bool,
    pub strength: f64,
}
impl Stage {
    pub fn new(settings: Settings) -> Self {
        Self {
            settings,
            enabled: true,
            strength: 1.0,
        }
    }
    fn active(&self) -> bool {
        self.enabled && self.strength > 0.0
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Invalid(&'static str),
    Budget { needed: u64, budget: u64 },
    Cancelled,
    Allocation,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(s) => f.write_str(s),
            Self::Budget { needed, budget } => write!(
                f,
                "フィルターの作業メモリが予算を超えます: {needed} > {budget} バイト"
            ),
            Self::Cancelled => f.write_str("フィルターの評価を取り消しました"),
            Self::Allocation => f.write_str("フィルターの作業メモリを確保できません"),
        }
    }
}
impl std::error::Error for Error {}

/// 読み取り専用。タイルの境界でもキャンバス座標を受け取る。
/// Mask の場合は返す画素の A が隠す量で、RGB は無視する。
pub trait Source: Sync {
    fn dimensions(&self) -> (u32, u32);
    fn pixel(&self, x: u32, y: u32) -> [u8; 4];
    /// 行の一部（x から `out.len() / 4` 画素）をまとめて読む。既定は 1 画素ずつ。タイルの面など、まとめて写せる読み元は置き換える。
    fn read_row(&self, x: u32, y: u32, out: &mut [u8]) {
        for (i, p) in out.chunks_exact_mut(4).enumerate() {
            p.copy_from_slice(&self.pixel(x + i as u32, y));
        }
    }
}
pub struct Image<'a> {
    data: &'a [u8],
    width: u32,
    height: u32,
}
impl<'a> Image<'a> {
    pub fn new(data: &'a [u8], width: u32, height: u32) -> Result<Self, Error> {
        if width == 0
            || height == 0
            || u64::from(width)
                .checked_mul(u64::from(height))
                .and_then(|n| n.checked_mul(4))
                != Some(data.len() as u64)
        {
            return Err(Error::Invalid("画像は幅×高さ×4バイト必要です"));
        }
        Ok(Self {
            data,
            width,
            height,
        })
    }
}
impl Source for Image<'_> {
    fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }
    fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = (y as usize * self.width as usize + x as usize) * 4;
        self.data[i..i + 4].try_into().unwrap()
    }
}

/// 予算は返却画像と同時に生きている作業画素を含む。入力画像は借用なので除外。
/// メタデータ・rayon のスレッドスタックは含まない。C# のブロック上限式を使う。
pub struct Options<'a> {
    pub working_budget: u64,
    pub block_size: u32,
    pub cancel: Option<&'a AtomicBool>,
    pub generators: Option<&'a dyn GeneratorInput>,
    /// `statistics` が返した値（段と同じ並び）。Some の段は走査せずその値を使う。None の段は評価の中で求める。
    /// 入力・スタック・Generator の値が変わったら取り直すのは呼び出し側。長さと段の種類だけ検証する。
    pub statistics: Option<&'a [Option<Statistics>]>,
}
impl Default for Options<'_> {
    fn default() -> Self {
        Self {
            working_budget: 256 * 1024 * 1024,
            block_size: 256,
            cancel: None,
            generators: None,
            statistics: None,
        }
    }
}
impl Options<'_> {
    fn check(&self) -> Result<(), Error> {
        if self.cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }
}
fn zeros<T: Default + Clone>(n: usize) -> Result<Vec<T>, Error> {
    let mut v = Vec::new();
    v.try_reserve_exact(n).map_err(|_| Error::Allocation)?;
    v.resize(n, T::default());
    Ok(v)
}
fn area(r: Rect) -> usize {
    r.width as usize * r.height as usize
}
fn grow(r: Rect, m: u32, w: u32, h: u32) -> Rect {
    let x = r.x.saturating_sub(m);
    let y = r.y.saturating_sub(m);
    Rect::new(
        x,
        y,
        (r.x + r.width).saturating_add(m).min(w) - x,
        (r.y + r.height).saturating_add(m).min(h) - y,
    )
}
/// C# WorkingBytes の画素バッファ上限に縦パスの行累積を加える。
/// 返却画像の予算は別途加算する。
pub fn block_working_bytes(
    stack: &[Stage],
    block: u32,
    width: u32,
    height: u32,
) -> Result<u64, Error> {
    if block == 0
        || block > 4096
        || width == 0
        || height == 0
        || width > i32::MAX as u32
        || height > i32::MAX as u32
    {
        return Err(Error::Invalid("画像・ブロックの寸法が範囲外です"));
    }
    let mut halo = 0u32;
    for s in stack.iter().filter(|s| s.active()) {
        halo = halo
            .checked_add(s.settings.halo())
            .ok_or(Error::Invalid("halo が大きすぎます"))?;
    }
    if halo > MAX_HALO {
        return Err(Error::Invalid("スタックの半径の合計は512画素までです"));
    }
    let a = |m: u32| {
        u64::from(width.min(block.saturating_add(2 * m)))
            * u64::from(height.min(block.saturating_add(2 * m)))
    };
    let mut peak = a(halo) * 4;
    for s in stack.iter().filter(|s| s.active()) {
        let input = a(halo);
        let row_sums = u64::from(width.min(block.saturating_add(2 * halo))) * 16;
        halo -= s.settings.halo();
        let output = a(halo);
        peak = peak.max(if s.settings.halo() > 0 {
            input * 28 + output * 4 + row_sums
        } else {
            input * 4
        });
    }
    Ok(peak)
}

/// Normalize の段が読む全域統計。その段への入力全体で、アルファが正の画素の RGB の最小と最大（マスクは全画素）。
/// 該当する画素が無いときは 0・0（何も変えない）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Statistics {
    pub min: u8,
    pub max: u8,
}

/// 評価の前に決まる計画。設定の検証・寸法・予算の見積りまでを 1 か所に集める。
struct Plan {
    /// 有効な段だけ（強さ 0・無効の段は除く）。
    chain: Vec<Stage>,
    /// `chain` と同じ並びで、呼び出し側が渡した統計。
    supplied: Vec<Option<Statistics>>,
    width: u32,
    height: u32,
    /// 1 ブロックの作業バイト（`block_working_bytes`）。
    working: u64,
}

fn prepare(
    source: &dyn Source,
    stack: &[Stage],
    value_type: ValueType,
    region: Option<Rect>,
    options: &Options<'_>,
) -> Result<Plan, Error> {
    options.check()?;
    let (w, h) = source.dimensions();
    if w == 0
        || h == 0
        || w > i32::MAX as u32
        || h > i32::MAX as u32
        || region.is_some_and(|r| {
            r.width == 0
                || r.height == 0
                || u64::from(r.x) + u64::from(r.width) > u64::from(w)
                || u64::from(r.y) + u64::from(r.height) > u64::from(h)
        })
        || options.block_size == 0
        || options.block_size > 4096
    {
        return Err(Error::Invalid("画像・領域・ブロックの寸法が範囲外です"));
    }
    if stack.len() > MAX_STACK {
        return Err(Error::Invalid("スタックは32段までです"));
    }
    if options.statistics.is_some_and(|s| s.len() != stack.len()) {
        return Err(Error::Invalid("統計の数が段の数と合いません"));
    }
    let mut chain = Vec::new();
    let mut supplied = Vec::new();
    for (i, s) in stack.iter().enumerate() {
        s.settings.validate(value_type)?;
        if !s.strength.is_finite() || !(0.0..=1.0).contains(&s.strength) {
            return Err(Error::Invalid("強さは有限の0..1です"));
        }
        if s.active()
            && matches!(s.settings, Settings::Generator { .. })
            && options.generators.is_none()
        {
            return Err(Error::Invalid("Generator の入力解決器がありません"));
        }
        let given = options.statistics.and_then(|all| all[i]);
        if let Some(st) = given {
            if !s.active() || !matches!(s.settings, Settings::Normalize) || st.min > st.max {
                return Err(Error::Invalid(
                    "統計は有効な Normalize の段にだけ、最小 <= 最大で付けられます",
                ));
            }
        }
        if s.active() {
            chain.push(s.clone());
            supplied.push(given);
        }
    }
    let working = block_working_bytes(&chain, options.block_size, w, h)?;
    Ok(Plan {
        chain,
        supplied,
        width: w,
        height: h,
        working,
    })
}

/// 同時に動かすブロック数。空き予算に収まる数を、rayon の現在のプールのスレッド数で頭打ちにする（最小 1）。
fn worker_count(free_budget: u64, working: u64) -> usize {
    (free_budget / working.max(1))
        .min(rayon::current_num_threads() as u64)
        .max(1) as usize
}

/// 各段の Normalize の全域統計だけを求める。返す並びは `stack` と同じで、有効な Normalize の段だけが Some。
/// 統計はその段より前の段の出力（画像全体）から取る。走査はブロック並列で、結果は並列度・ブロックの大きさによらない。
/// `Options::statistics` に渡した段はそのまま返す。文書側が世代ごとに持ち、領域ごとの `evaluate` へ渡せる。
pub fn statistics(
    source: &dyn Source,
    value_type: ValueType,
    stack: &[Stage],
    options: &Options<'_>,
) -> Result<Vec<Option<Statistics>>, Error> {
    let plan = prepare(source, stack, value_type, None, options)?;
    if plan.working > options.working_budget {
        return Err(Error::Budget {
            needed: plan.working,
            budget: options.working_budget,
        });
    }
    let mut engine = Engine::new(source, value_type, &plan, options);
    engine.resolve_statistics(worker_count(options.working_budget, plan.working))?;
    let mut out = vec![None; stack.len()];
    let mut k = 0;
    for (i, s) in stack.iter().enumerate() {
        if s.active() {
            out[i] = engine.stats[k];
            k += 1;
        }
    }
    Ok(out)
}

/// 領域の完成画像。Mask も RGBA8 で返し、RGB=0・A=隠す量。
/// Normalize は指定領域の外を含む各段の入力全体で統計を取る（`Options::statistics` があればそれを使う）。
/// 取消・予算拒否・設定不正では入力を一切変更しない。
pub fn evaluate(
    source: &dyn Source,
    value_type: ValueType,
    stack: &[Stage],
    region: Rect,
    options: &Options<'_>,
) -> Result<Vec<u8>, Error> {
    let plan = prepare(source, stack, value_type, Some(region), options)?;
    let output_bytes = u64::from(region.width) * u64::from(region.height) * 4;
    let needed = plan
        .working
        .checked_add(output_bytes)
        .ok_or(Error::Allocation)?;
    if needed > options.working_budget {
        return Err(Error::Budget {
            needed,
            budget: options.working_budget,
        });
    }
    let mut engine = Engine::new(source, value_type, &plan, options);
    // 統計は返却画像を確保する前に済ませる（走査中は返却画像が生きていないので、予算いっぱいまで並列にできる）。
    engine.resolve_statistics(worker_count(options.working_budget, plan.working))?;
    let chain = &plan.chain;
    let mut output = zeros::<u8>(usize::try_from(output_bytes).map_err(|_| Error::Allocation)?)?;
    let workers = worker_count(options.working_budget - output_bytes, plan.working);
    let band_rows = options.block_size as usize;
    let stride = region.width as usize * 4;
    // 同時に確保するブロック数を予算内に制限する。rayon の既存プールを使い、新たなスレッドは作らない。
    for (batch, bands) in output.chunks_mut(stride * band_rows * workers).enumerate() {
        bands
            .par_chunks_mut(stride * band_rows)
            .enumerate()
            .try_for_each(|(band, dst)| -> Result<(), Error> {
                let y = region.y + ((batch * workers + band) * band_rows) as u32;
                let bh = (dst.len() / stride) as u32;
                for dx in (0..region.width).step_by(options.block_size as usize) {
                    let r = Rect::new(
                        region.x + dx,
                        y,
                        options.block_size.min(region.width - dx),
                        bh,
                    );
                    let data = engine.rect(chain.len(), r)?;
                    for row in 0..bh as usize {
                        let start = row * stride + dx as usize * 4;
                        dst[start..start + r.width as usize * 4].copy_from_slice(
                            &data[row * r.width as usize * 4..(row + 1) * r.width as usize * 4],
                        );
                    }
                }
                Ok(())
            })?;
    }
    options.check()?;
    if value_type == ValueType::Mask {
        for p in output.chunks_exact_mut(4) {
            let hide = p[0];
            p.copy_from_slice(&[0, 0, 0, hide]);
        }
    }
    Ok(output)
}
struct Engine<'a> {
    source: &'a dyn Source,
    value_type: ValueType,
    chain: &'a [Stage],
    options: &'a Options<'a>,
    /// `chain` と同じ並び。Normalize の段だけ、渡された値か走査の結果が入る。
    stats: Vec<Option<Statistics>>,
    width: u32,
    height: u32,
}
/// アルファが正の画素の RGB の最小・最大。該当が無ければ (255, 0)。
fn min_max(data: &[u8]) -> (u8, u8) {
    let (mut min, mut max) = (255u8, 0u8);
    for p in data.chunks_exact(4).filter(|p| p[3] > 0) {
        for &v in &p[..3] {
            min = min.min(v);
            max = max.max(v);
        }
    }
    (min, max)
}
impl<'a> Engine<'a> {
    fn new(
        source: &'a dyn Source,
        value_type: ValueType,
        plan: &'a Plan,
        options: &'a Options<'a>,
    ) -> Self {
        Self {
            source,
            value_type,
            chain: &plan.chain,
            options,
            stats: plan.supplied.clone(),
            width: plan.width,
            height: plan.height,
        }
    }
    /// 渡されていない Normalize の統計を、前の段から順に求める（後の段の統計は前の段の統計に依る）。
    fn resolve_statistics(&mut self, workers: usize) -> Result<(), Error> {
        for k in 0..self.chain.len() {
            if self.stats[k].is_none() && matches!(self.chain[k].settings, Settings::Normalize) {
                self.stats[k] = Some(self.scan(k, workers)?);
            }
        }
        Ok(())
    }
    /// 段 k の入力（段 0..k の出力）を画像全体で、ブロックを捨てながら集約する。`workers` ブロックずつ並列に処理し、
    /// 最小・最大は順序によらないので並列度で結果は変わらない。ブロックの一覧は持たず、番号から矩形を作る。
    fn scan(&self, k: usize, workers: usize) -> Result<Statistics, Error> {
        let block = self.options.block_size;
        let across = u64::from(self.width.div_ceil(block));
        let total = across * u64::from(self.height.div_ceil(block));
        let (mut min, mut max) = (255u8, 0u8);
        let mut start = 0u64;
        while start < total {
            let end = (start + workers as u64).min(total);
            let parts = (start..end)
                .into_par_iter()
                .map(|i| {
                    let x = (i % across) as u32 * block;
                    let y = (i / across) as u32 * block;
                    let r = Rect::new(x, y, block.min(self.width - x), block.min(self.height - y));
                    Ok(min_max(&self.rect(k, r)?))
                })
                .collect::<Result<Vec<_>, Error>>()?;
            for (lo, hi) in parts {
                min = min.min(lo);
                max = max.max(hi);
            }
            start = end;
        }
        Ok(if min > max {
            Statistics { min: 0, max: 0 }
        } else {
            Statistics { min, max }
        })
    }
    fn rect(&self, count: usize, target: Rect) -> Result<Vec<u8>, Error> {
        self.options.check()?;
        let mut after = self.chain[..count].iter().map(|s| s.settings.halo()).sum();
        let mut cur = grow(target, after, self.width, self.height);
        let mut buf = zeros::<u8>(area(cur) * 4)?;
        let row_bytes = cur.width as usize * 4;
        for y in 0..cur.height {
            self.options.check()?;
            let row = &mut buf[y as usize * row_bytes..(y as usize + 1) * row_bytes];
            self.source.read_row(cur.x, cur.y + y, row);
            if self.value_type == ValueType::Mask {
                for p in row.chunks_exact_mut(4) {
                    p.copy_from_slice(&[p[3], p[3], p[3], 255]);
                }
            }
        }
        for (k, s) in self.chain[..count].iter().enumerate() {
            self.options.check()?;
            after -= s.settings.halo();
            let next = grow(target, after, self.width, self.height);
            if s.settings.halo() > 0 {
                buf = pixels::neighborhood(
                    &buf,
                    cur,
                    next,
                    self.width,
                    self.height,
                    s,
                    self.value_type,
                    &|| self.options.check(),
                )?;
            } else {
                self.point(&mut buf, cur, k, s)?;
            }
            cur = next;
        }
        Ok(buf)
    }
    fn point(&self, buf: &mut [u8], r: Rect, k: usize, s: &Stage) -> Result<(), Error> {
        let mut lut = [0u8; 256];
        for (v, b) in lut.iter_mut().enumerate() {
            *b = match s.settings {
                Settings::Invert => 255 - v as u8,
                Settings::Levels {
                    input_black,
                    input_white,
                    gamma,
                    output_black,
                    output_white,
                } => to_byte(
                    output_black
                        + clamp01((v as f64 / 255.0 - input_black) / (input_white - input_black))
                            .powf(1.0 / gamma)
                            * (output_white - output_black),
                ),
                Settings::Normalize => {
                    let st = self.stats[k].expect("統計は評価の前に段の順で確定している");
                    if st.max > st.min {
                        to_byte(clamp01(
                            (v as f64 - f64::from(st.min)) / f64::from(st.max - st.min),
                        ))
                    } else {
                        v as u8
                    }
                }
                _ => v as u8,
            };
        }
        for (y, row) in buf.chunks_exact_mut(r.width as usize * 4).enumerate() {
            self.options.check()?;
            for (x, p) in row.chunks_exact_mut(4).enumerate() {
                match s.settings {
                    Settings::Noise {
                        amount,
                        seed,
                        monochrome,
                    } => {
                        let h = pixels::hash(
                            pixels::hash(pixels::hash(seed as u32) ^ (r.x + x as u32))
                                ^ (r.y + y as u32),
                        );
                        for (c, v) in p[..3].iter_mut().enumerate() {
                            let u = f64::from(
                                pixels::hash(h ^ if monochrome { 0 } else { c as u32 }) >> 8,
                            ) / 16777215.0
                                * 2.0
                                - 1.0;
                            let n = (f64::from(*v) + amount * 127.5 * u + 0.5)
                                .floor()
                                .clamp(0.0, 255.0) as u8;
                            *v = pixels::lerp(*v, n, s.strength);
                        }
                    }
                    Settings::Generator { slot, blend } => {
                        if p[3] == 0 && self.value_type != ValueType::Mask {
                            continue;
                        }
                        let Some(g) = self
                            .options
                            .generators
                            .and_then(|g| g.sample(slot, r.x + x as u32, r.y + y as u32))
                        else {
                            continue;
                        };
                        if matches!(g,Generated::Scalar(v) if !v.is_finite() || !(0.0..=1.0).contains(&v))
                        {
                            return Err(Error::Invalid("Generator の値は有限の0..1です"));
                        }
                        pixels::generate(
                            p,
                            g,
                            blend,
                            s.strength,
                            self.value_type == ValueType::Mask,
                        );
                    }
                    _ => {
                        for v in &mut p[..3] {
                            *v = pixels::lerp(*v, lut[*v as usize], s.strength);
                        }
                    }
                }
            }
        }
        Ok(())
    }
}
