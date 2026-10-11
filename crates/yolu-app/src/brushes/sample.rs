//! ブラシの見本のストローク: そのブラシの実際の設定で、core が小さな文書に S 字の 1 本（筆圧 0 → 1 → 0）を描き、画像にする。
//! 毎フレームは描かない。画像は「描く設定の札」（描く直前の正規のブラシと大きさの Debug の文字列のハッシュ）で覚え、札が変わった
//! 見本だけを描き直す。描く量にも覚える量にも上限がある（1 フレームに描く数・覚える枚数とバイト数。超えた分は次のフレームへ送り、
//! 一番使っていない画像から捨てる）。
//!
//! 見本は実寸の関係を保ったまま、画像に収まるよう縮める（直径が画像の高さの 62% を超えるブラシは縮め、細すぎるものは 3 画素まで
//! 太らせる。質感の大きさ・入り抜き・デュアルの半径・ぼかしの半径・クローンのずれ・筆の速さも同じ倍率）。手ぶれ補正は糸の遅れで
//! 線を短くするだけなので見本では 0、描く色は黒、背景色は白、乱数の種は 0（同じブラシは同じ絵）、対称とステンシルは無し。
//! 消しゴムは灰色を一面に敷いて消し、効果のブラシ（ぼかし・指先・クローン）と色を混ぜるブラシは縦の帯を並べた絵の上に描く。
//!
//! 描く場所は 2 通り。既定は画面のスレッドで 1 フレームに数枚まで描く（試験・画面を持たない使い方）。`render_in_background` を呼ぶと、
//! 描くのを別のスレッド（見本専用の小さな rayon の池。`POOL_THREADS` 本）へ出し、できた絵は次のフレームで受ける（取り込んだ大きな筆先の
//! 見本で画面が止まらない）。見本の中の並列（core の筆の計算・合成）もこの池の中で回るので、全体の rayon の池（合成・保存の並列）を
//! 見本が塞がない。描いている最中の札は重ねて頼まず、同時に頼む数にも上限がある。
//!
//! 見本を出す所（ツールのプロパティ・詳細のウィンドウ・一覧の行・アセットの欄）は、所ごとに最後に出した札を覚え、設定を変えて新しい札の絵が
//! まだ無い間は前の絵を出し続ける（[`SampleCache::shown`]。つまみを動かすたびに紙だけになって点滅しない）。

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, OnceLock, Weak};

use egui::{ColorImage, TextureHandle, TextureId, TextureOptions};

use super::canonical;
use crate::engine::{
    Brush, BrushEffect, BrushSample, BrushSettings, BrushTip, Channel, CoreError, DVec2, Document,
    Rgba8, RowOrder, Stroke,
};

/// 画像の大きさの上限（画素）。
pub const MAX_WIDTH: u32 = 1024;
pub const MAX_HEIGHT: u32 = 256;
/// 1 フレームに描く見本の数の上限。
pub const RENDERS_PER_FRAME: usize = 4;
/// 別のスレッドへ同時に頼む見本の数の上限。
pub const MAX_IN_FLIGHT: usize = 6;
/// 覚える見本の枚数とバイト数の上限。
pub const MAX_ENTRIES: usize = 96;
pub const MAX_BYTES: usize = 8 * 1024 * 1024;
/// 見本を描く専用の池のスレッド数。一覧の見本は画面に見えている数だけなので、少数でよい（残りは全体の池の仕事に残す）。
pub const POOL_THREADS: usize = 2;
/// 見本の池のスレッドの名前の頭（試験が、見本が専用の池で描かれることを確かめる）。
pub const POOL_THREAD_PREFIX: &str = "yolu-brush-sample-";

/// 見本を描く専用の池。全体の rayon の池へ出すと、取り込んだ大きな筆先の見本が池のスレッドを長く握り、その間の合成・保存の並列が
/// 待たされる。池を作れなければ None（呼び手が 1 本のスレッドへ落とす）。
fn pool() -> Option<&'static rayon::ThreadPool> {
    static POOL: OnceLock<Option<rayon::ThreadPool>> = OnceLock::new();
    POOL.get_or_init(|| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(POOL_THREADS)
            .thread_name(|n| format!("{POOL_THREAD_PREFIX}{n}"))
            // 仕事の panic は仕事の中で受け止める。取りこぼしても池の panic でプロセスを止めない
            .panic_handler(|_| {})
            .build()
            .ok()
    })
    .as_ref()
}

/// 見本の仕事を専用の池で走らせる（池が無ければ名前つきの 1 本のスレッド）。スレッドも立てられなければ Err（仕事は走らない）。
fn spawn_in_pool(job: impl FnOnce() + Send + 'static) -> Result<(), std::io::Error> {
    match pool() {
        Some(pool) => {
            pool.spawn(job);
            Ok(())
        }
        None => std::thread::Builder::new()
            .name(format!("{POOL_THREAD_PREFIX}solo"))
            .spawn(job)
            .map(drop),
    }
}

/// 試験用: 見本の仕事と同じ道（専用の池）で仕事を走らせる。
#[doc(hidden)]
pub fn spawn_for_test(job: impl FnOnce() + Send + 'static) -> Result<(), std::io::Error> {
    spawn_in_pool(job)
}

/// 見本の大きさ（画素）と、消しゴムとして描くか。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SampleSpec {
    pub width: u32,
    pub height: u32,
    pub eraser: bool,
}

impl SampleSpec {
    /// 一覧の行の見本。
    pub fn row(eraser: bool) -> SampleSpec {
        SampleSpec {
            width: 340,
            height: 60,
            eraser,
        }
    }
    /// ツールプロパティの上の見本。
    pub fn tool(eraser: bool) -> SampleSpec {
        SampleSpec {
            width: 448,
            height: 72,
            eraser,
        }
    }
    /// 詳細のウィンドウの上の見本。
    pub fn detail(eraser: bool) -> SampleSpec {
        SampleSpec {
            width: 640,
            height: 88,
            eraser,
        }
    }
    fn clamped(self) -> SampleSpec {
        SampleSpec {
            width: self.width.clamp(16, MAX_WIDTH),
            height: self.height.clamp(8, MAX_HEIGHT),
            eraser: self.eraser,
        }
    }
}

/// 描いた見本（straight RGBA8、1 行目が上）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SampleImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// 画像に収まる倍率: 直径が高さの 62% を超えれば縮め、3 画素に満たなければ 3 画素まで。
fn fit_scale(diameter: f64, height: f64) -> f64 {
    let diameter = diameter.max(1.0);
    let (min, max) = (3.0, 0.62 * height);
    if diameter > max {
        max / diameter
    } else if diameter < min {
        min / diameter
    } else {
        1.0
    }
}

/// 描く直前のブラシ（見本の決まりを当てて縮めたもの）。手ぶれ補正以外の描き手の設定（入り抜き・曲線）は渡されたまま使う。
fn preview_brush(brush: &Brush, spec: SampleSpec) -> Brush {
    let spec = spec.clamped();
    let assist = brush.assist;
    let mut b = canonical(brush);
    let f = fit_scale(b.base.radius * 2.0, spec.height as f64);
    b.base.radius = (b.base.radius * f).max(0.5);
    b.base.color = Rgba8::new(0, 0, 0, 255);
    b.base.erase = spec.eraser;
    b.color.secondary = Rgba8::new(255, 255, 255, 255);
    b.assist = crate::engine::StrokeAssist {
        stabilizer: 0.0,
        taper_in: (assist.taper_in * f).min(10_000.0),
        taper_out: (assist.taper_out * f).min(10_000.0),
        curve: assist.curve,
    };
    if let Some(t) = &mut b.texture {
        t.scale = (t.scale * f).clamp(0.05, 64.0);
    }
    if let Some(d) = &mut b.dual {
        d.radius = (d.radius * f).clamp(0.5, 65536.0);
    }
    b.controls.speed_max = (b.controls.speed_max * f).max(1.0);
    b.effect = match b.effect {
        BrushEffect::Blur { radius } => BrushEffect::Blur {
            radius: ((radius as f64 * f).round() as u32).clamp(1, 64),
        },
        BrushEffect::Clone { offset } => BrushEffect::Clone { offset: offset * f },
        other => other,
    };
    if spec.eraser {
        b.effect = BrushEffect::Paint;
    }
    b
}

/// 画像（筆先・質感・デュアルの先端）の指紋: 名前・大きさ・画素。
fn pixel_print(tip: &BrushTip) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    tip.name().hash(&mut hasher);
    (tip.width(), tip.height()).hash(&mut hasher);
    tip.alpha().hash(&mut hasher);
    hasher.finish()
}

/// 画像の指紋の覚え（画像の Arc ごと）。画像は作ったあと変わらないので、同じ Arc は 1 度だけ計算する（毎フレーム・行ごとに
/// 数 MB の画素を読まない）。弱い参照を持つので、覚えている間はその置き場所が別の画像に使い回されない。
#[derive(Default)]
struct TipPrints(HashMap<usize, (Weak<BrushTip>, u64)>);

/// 覚える画像の数の上限（超えたら、もう無い画像の分を捨て、それでも多ければ全部捨てて数え直す）。
const MAX_TIP_PRINTS: usize = 256;

impl TipPrints {
    fn print(&mut self, tip: &Arc<BrushTip>) -> u64 {
        let at = Arc::as_ptr(tip) as usize;
        if let Some((_, print)) = self.0.get(&at) {
            return *print;
        }
        if self.0.len() >= MAX_TIP_PRINTS {
            self.0.retain(|_, (alive, _)| alive.strong_count() > 0);
        }
        if self.0.len() >= MAX_TIP_PRINTS {
            self.0.clear();
        }
        let print = pixel_print(tip);
        self.0.insert(at, (Arc::downgrade(tip), print));
        print
    }
}

fn key_with(brush: &Brush, spec: SampleSpec, print: &mut dyn FnMut(&Arc<BrushTip>) -> u64) -> u64 {
    let spec = spec.clamped();
    let preview = preview_brush(brush, spec);
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    spec.hash(&mut hasher);
    format!("{preview:?}").hash(&mut hasher);
    // Debug は画像の名前と大きさしか出さない。同じ名前・大きさで画素だけが違う画像は別の絵になるので、画素の指紋を混ぜる
    let tips = preview
        .tip
        .image
        .iter()
        .chain(&preview.tip.images)
        .chain(preview.texture.iter().map(|t| &t.image))
        .chain(preview.dual.iter().filter_map(|d| d.tip.as_ref()));
    for tip in tips {
        print(tip).hash(&mut hasher);
    }
    hasher.finish()
}

/// 描く設定の札（同じ札なら同じ絵）。
pub fn key_of(brush: &Brush, spec: SampleSpec) -> u64 {
    key_with(brush, spec, &mut |tip| pixel_print(tip))
}

/// 画像の座標（左上原点・y は下向き）の点で線を描く。
fn stroke_line(
    doc: &mut Document,
    layer: crate::engine::LayerId,
    brush: &Brush,
    points: &[(f64, f64, f64)],
    height: f64,
    speed: f64,
) -> Result<(), CoreError> {
    let mut stroke: Stroke = doc.begin_brush_stroke(layer, brush)?;
    let mut time = 0.0;
    let mut last: Option<(f64, f64)> = None;
    for &(x, y, pressure) in points {
        if let Some((px, py)) = last {
            time += ((x - px).powi(2) + (y - py).powi(2)).sqrt() / speed;
        }
        last = Some((x, y));
        let sample = BrushSample::new(x, height - y, pressure, time, DVec2::ZERO)?;
        stroke.add_sample(doc, sample)?;
    }
    doc.end_stroke(stroke)?;
    Ok(())
}

/// 見本を描く。
pub fn render(brush: &Brush, spec: SampleSpec) -> Result<SampleImage, CoreError> {
    let spec = spec.clamped();
    let (w, h) = (spec.width as f64, spec.height as f64);
    let b = preview_brush(brush, spec);
    let mut doc = Document::new(spec.width, spec.height)?;
    let layer = doc.add_layer("sample")?;
    if spec.eraser {
        doc.fill(
            layer,
            Channel::Color,
            Rgba8::new(96, 96, 96, 255),
            1.0,
            None,
            false,
        )?;
    } else if !b.effect.is_paint() || b.mix.is_active() {
        doc.fill(
            layer,
            Channel::Color,
            Rgba8::new(236, 236, 236, 255),
            1.0,
            None,
            false,
        )?;
        // 縦の帯: 暗い・赤・暗い・青
        let bar = BrushSettings {
            radius: (h * 0.07).max(2.0),
            hardness: 1.0,
            spacing: 0.1,
            pressure_size: false,
            pressure_opacity: false,
            ..BrushSettings::default()
        };
        for (i, color) in [
            Rgba8::new(40, 40, 44, 255),
            Rgba8::new(214, 66, 60, 255),
            Rgba8::new(40, 40, 44, 255),
            Rgba8::new(52, 108, 214, 255),
        ]
        .into_iter()
        .enumerate()
        {
            let x = w * (0.18 + 0.215 * i as f64);
            let line = Brush::from(BrushSettings { color, ..bar });
            stroke_line(
                &mut doc,
                layer,
                &line,
                &[(x, 0.0, 1.0), (x, h, 1.0)],
                h,
                1000.0,
            )?;
        }
    }
    // S 字（画像の左右の端から半径の分だけ離す）
    let radius = b.base.radius;
    let pad = (radius * 1.3).max(0.07 * w).max(4.0);
    let amp = (h * 0.5 - radius * 1.15 - 2.0).clamp(0.0, 0.3 * h);
    let length = (w - 2.0 * pad) * 1.1;
    let n = ((length / 1.5).ceil() as usize).clamp(16, 900);
    let points: Vec<(f64, f64, f64)> = (0..=n)
        .map(|i| {
            let t = i as f64 / n as f64;
            (
                pad + t * (w - 2.0 * pad),
                h * 0.5 - amp * (std::f64::consts::TAU * t).sin(),
                (std::f64::consts::PI * t).sin(),
            )
        })
        .collect();
    // 速さは効きの上限の 35% ほどで進める（筆の速さの設定が見本でも見える）
    let speed = 0.35 * b.controls.speed_max;
    stroke_line(&mut doc, layer, &b, &points, h, speed)?;
    let mut rgba = vec![0u8; spec.width as usize * spec.height as usize * 4];
    doc.composite_into(Channel::Color, doc.bounds(), &mut rgba, RowOrder::TopDown)?;
    Ok(SampleImage {
        width: spec.width,
        height: spec.height,
        rgba,
    })
}

/// 見本を出す所で出す絵（[`SampleCache::shown`]）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shown {
    /// 出す絵の札（その所でまだ一度も絵を出していなければ None で、紙だけ）。
    pub key: Option<u64>,
    /// 今の設定の絵か（false なら、新しい絵ができるまで前の絵を出している）。
    pub current: bool,
}

/// 覚える「最後に出した札」の数の上限（超えたら、絵の無くなった所の分を捨てる）。
const MAX_SHOWN: usize = 4 * MAX_ENTRIES;

/// 描いた回数などの数（試験が「描き直す条件」を確かめる）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SampleStats {
    pub renders: u64,
    pub hits: u64,
    /// 1 フレームの上限で次へ送った数。
    pub deferred: u64,
    pub evicted: u64,
    pub failed: u64,
}

struct Slot {
    image: SampleImage,
    texture: Option<TextureHandle>,
    used: u64,
}

/// 別のスレッドで描く仕組み（頼み中の札と、できた絵の受け口）。
struct Background {
    ctx: egui::Context,
    tx: Sender<(u64, SampleSpec, Option<SampleImage>)>,
    rx: Receiver<(u64, SampleSpec, Option<SampleImage>)>,
    in_flight: HashSet<u64>,
}

/// 見本の画像の置き場（札 → 画像）。
#[derive(Default)]
pub struct SampleCache {
    slots: HashMap<u64, Slot>,
    tip_prints: TipPrints,
    clock: u64,
    bytes: usize,
    frame: Option<u64>,
    renders_in_frame: usize,
    background: Option<Background>,
    /// このフレームに、同時の上限で頼めなかった札があった（次のフレームで頼み直す）。
    starved: bool,
    /// 見本を出す所ごとの、最後に出した札。
    shown: HashMap<egui::Id, u64>,
    pub stats: SampleStats,
}

impl SampleCache {
    /// フレームの頭（フレームの番号が変わったら、そのフレームに描いた数を数え直す）。
    pub fn begin_frame(&mut self, frame: u64) {
        if self.frame != Some(frame) {
            self.frame = Some(frame);
            self.renders_in_frame = 0;
            self.starved = false;
            self.collect();
        }
    }

    /// 見本を描くのを別のスレッドへ出す（できたら `ctx` へ描き直しを頼む）。
    pub fn render_in_background(&mut self, ctx: &egui::Context) {
        let (tx, rx) = channel();
        self.background = Some(Background {
            ctx: ctx.clone(),
            tx,
            rx,
            in_flight: HashSet::new(),
        });
    }

    /// 別のスレッドで描いている最中の見本の数。
    pub fn in_flight(&self) -> usize {
        self.background.as_ref().map_or(0, |b| b.in_flight.len())
    }

    /// 札を返さなかった（`request` が None）とき、次のフレームを待たずに描き直す必要があるか。別のスレッドで描いている最中なら
    /// 絵ができたとき（頼んだ側が描き直しを頼む）で足りる。
    pub fn needs_next_frame(&self) -> bool {
        self.background.is_none() || self.starved
    }

    /// 別のスレッドでできた絵を受ける。
    fn collect(&mut self) {
        let Some(background) = &mut self.background else {
            return;
        };
        let done: Vec<_> = background.rx.try_iter().collect();
        for (key, _, _) in &done {
            background.in_flight.remove(key);
        }
        for (key, spec, image) in done {
            let image = image.unwrap_or_else(|| {
                self.stats.failed += 1;
                // 描けなかった設定は空の画像で覚える（毎フレーム描き直さない）
                SampleImage {
                    width: spec.width,
                    height: spec.height,
                    rgba: vec![0; spec.width as usize * spec.height as usize * 4],
                }
            });
            self.clock += 1;
            self.bytes += image.rgba.len();
            if let Some(old) = self.slots.insert(
                key,
                Slot {
                    image,
                    texture: None,
                    used: self.clock,
                },
            ) {
                self.bytes -= old.image.rgba.len();
            }
            self.evict(key);
        }
    }

    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// 覚えている画像のバイト数。
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// 見本の札を返す（無ければ描く。このフレームの上限に達していれば None で、描くのは次のフレーム）。
    pub fn request(&mut self, brush: &Brush, spec: SampleSpec) -> Option<u64> {
        let prints = &mut self.tip_prints;
        let key = key_with(brush, spec, &mut |tip| prints.print(tip));
        self.clock += 1;
        if let Some(slot) = self.slots.get_mut(&key) {
            slot.used = self.clock;
            self.stats.hits += 1;
            return Some(key);
        }
        if let Some(background) = &mut self.background {
            if background.in_flight.contains(&key) {
                return None;
            }
            if background.in_flight.len() >= MAX_IN_FLIGHT {
                self.stats.deferred += 1;
                self.starved = true;
                return None;
            }
            background.in_flight.insert(key);
            self.stats.renders += 1;
            let (brush, spec) = (brush.clone(), spec.clamped());
            let (tx, ctx) = (background.tx.clone(), background.ctx.clone());
            let failed = (tx.clone(), ctx.clone());
            let spawned = spawn_in_pool(move || {
                // 描く途中で落ちても空の見本にする（池の仕事の panic がプロセスを止めない）
                let image = crate::crash::handled(std::panic::AssertUnwindSafe(|| {
                    render(&brush, spec).ok()
                }))
                .unwrap_or(None);
                let _ = tx.send((key, spec, image));
                ctx.request_repaint();
            });
            if spawned.is_err() {
                // スレッドを立てられなかった: 描けなかった見本として返す（札が頼みっぱなしで残らない）
                let _ = failed.0.send((key, spec, None));
                failed.1.request_repaint();
            }
            return None;
        }
        if self.renders_in_frame >= RENDERS_PER_FRAME {
            self.stats.deferred += 1;
            return None;
        }
        self.renders_in_frame += 1;
        self.stats.renders += 1;
        let spec = spec.clamped();
        let image = render(brush, spec).unwrap_or_else(|_| {
            self.stats.failed += 1;
            // 描けなかった設定は空の画像で覚える（毎フレーム描き直さない）
            SampleImage {
                width: spec.width,
                height: spec.height,
                rgba: vec![0; spec.width as usize * spec.height as usize * 4],
            }
        });
        self.bytes += image.rgba.len();
        self.slots.insert(
            key,
            Slot {
                image,
                texture: None,
                used: self.clock,
            },
        );
        self.evict(key);
        Some(key)
    }

    /// 見本を出す所 place に出す絵: 今の設定の絵があればそれ（その所の最後に出した札として覚える）。まだ無ければ（描いている最中・
    /// 1 フレームの上限で次へ送った）、その所で最後に出した絵を出し続ける（新しい絵が上がったら替わる）。
    pub fn shown(&mut self, place: egui::Id, brush: &Brush, spec: SampleSpec) -> Shown {
        if let Some(key) = self.request(brush, spec) {
            if self.shown.len() >= MAX_SHOWN && !self.shown.contains_key(&place) {
                let slots = &self.slots;
                self.shown.retain(|_, k| slots.contains_key(k));
            }
            self.shown.insert(place, key);
            return Shown {
                key: Some(key),
                current: true,
            };
        }
        let previous = self.shown.get(&place).copied();
        let key = previous.filter(|key| match self.slots.get_mut(key) {
            Some(slot) => {
                // 出している間は、一番使っていない画像として捨てられないように
                self.clock += 1;
                slot.used = self.clock;
                true
            }
            None => false,
        });
        if previous.is_some() && key.is_none() {
            self.shown.remove(&place);
        }
        Shown {
            key,
            current: false,
        }
    }

    /// 上限を超えていれば、一番使っていない画像から捨てる（今入れた `keep` は残す）。
    fn evict(&mut self, keep: u64) {
        while self.slots.len() > MAX_ENTRIES || self.bytes > MAX_BYTES {
            let Some((&oldest, _)) = self
                .slots
                .iter()
                .filter(|(k, _)| **k != keep)
                .min_by_key(|(_, s)| s.used)
            else {
                break;
            };
            if let Some(slot) = self.slots.remove(&oldest) {
                self.bytes -= slot.image.rgba.len();
                self.stats.evicted += 1;
                self.shown.retain(|_, k| *k != oldest);
            }
        }
    }

    pub fn image(&self, key: u64) -> Option<&SampleImage> {
        self.slots.get(&key).map(|s| &s.image)
    }

    /// 画面に出す絵（初めて出すときに作る）。
    pub fn texture(&mut self, ctx: &egui::Context, key: u64) -> Option<TextureId> {
        let slot = self.slots.get_mut(&key)?;
        let handle = slot.texture.get_or_insert_with(|| {
            let image = ColorImage::from_rgba_unmultiplied(
                [slot.image.width as usize, slot.image.height as usize],
                &slot.image.rgba,
            );
            ctx.load_texture(
                format!("brush-sample-{key:016x}"),
                image,
                TextureOptions::LINEAR,
            )
        });
        Some(handle.id())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brushes::builtin;

    fn ink(image: &SampleImage) -> usize {
        image.rgba.chunks(4).filter(|p| p[3] > 0).count()
    }

    #[test]
    fn the_sample_is_deterministic_and_draws_something_inside_the_image() {
        let spec = SampleSpec::row(false);
        for b in builtin::all() {
            let eraser = b.group.is_eraser();
            let spec = SampleSpec { eraser, ..spec };
            let a = render(&b.brush, spec).unwrap();
            let again = render(&b.brush, spec).unwrap();
            assert_eq!(a, again, "{}: 同じブラシは同じ絵", b.id);
            assert_eq!((a.width, a.height), (340, 60));
            assert_eq!(a.rgba.len(), 340 * 60 * 4);
            assert!(ink(&a) > 0, "{}: 何か描く", b.id);
            // 端の列には描かない（S 字は端から離す）。消しゴムの灰色の地は全面にある
            if !eraser && b.brush.effect.is_paint() && !b.brush.mix.is_active() {
                for y in 0..60usize {
                    assert_eq!(a.rgba[(y * 340) * 4 + 3], 0, "{}: 左端", b.id);
                    assert_eq!(a.rgba[(y * 340 + 339) * 4 + 3], 0, "{}: 右端", b.id);
                }
            }
        }
    }

    #[test]
    fn the_sample_follows_pressure_from_nothing_through_full_and_back() {
        // 筆圧で直径が変わるブラシは、両端が細く真ん中が太い
        let mut b = Brush::default();
        b.base.radius = 10.0;
        b.base.pressure_size = true;
        let image = render(&b, SampleSpec::row(false)).unwrap();
        let column = |x: usize| {
            (0..60usize)
                .filter(|y| image.rgba[(y * 340 + x) * 4 + 3] > 0)
                .count()
        };
        let (left, middle, right) = (column(40), column(170), column(300));
        assert!(middle > left && middle > right, "{left} {middle} {right}");
        // 筆圧に従わなければ同じ太さ
        b.base.pressure_size = false;
        b.base.pressure_opacity = false;
        let flat = render(&b, SampleSpec::row(false)).unwrap();
        let flat_column = |x: usize| {
            (0..60usize)
                .filter(|y| flat.rgba[(y * 340 + x) * 4 + 3] > 0)
                .count()
        };
        assert!(flat_column(40).abs_diff(flat_column(170)) <= 4);
    }

    #[test]
    fn big_and_tiny_brushes_are_scaled_to_fit() {
        assert_eq!(fit_scale(30.0, 60.0), 1.0);
        assert!((fit_scale(200.0, 60.0) * 200.0 - 0.62 * 60.0).abs() < 1e-9);
        assert!((fit_scale(1.0, 60.0) - 3.0).abs() < 1e-9);
        let mut huge = Brush::default();
        huge.base.radius = 128.0;
        let image = render(&huge, SampleSpec::row(false)).unwrap();
        assert!(ink(&image) > 0 && ink(&image) < 340 * 60);
    }

    #[test]
    fn the_key_changes_with_what_is_drawn_and_not_with_what_is_not() {
        let spec = SampleSpec::row(false);
        let base = Brush::default();
        let key = key_of(&base, spec);
        // 見本に出ない設定（色・乱数の種・背景色・手ぶれ補正の糸・対称・ステンシル）では変わらない
        let mut same = base.clone();
        same.seed = 99;
        same.base.color = Rgba8::new(10, 20, 30, 255);
        same.color.secondary = Rgba8::new(1, 2, 3, 255);
        same.assist.stabilizer = 120.0;
        assert_eq!(key_of(&same, spec), key);
        // 見本に出る設定は、どれか 1 つが変わっても変わる
        type Change = (&'static str, Box<dyn Fn(&mut Brush)>);
        let changes: Vec<Change> = vec![
            ("radius", Box::new(|b| b.base.radius = 30.0)),
            ("hardness", Box::new(|b| b.base.hardness = 0.1)),
            (
                "anti alias",
                Box::new(|b| b.base.anti_alias = crate::engine::AntiAlias::Strong),
            ),
            ("spacing", Box::new(|b| b.base.spacing = 0.5)),
            ("opacity", Box::new(|b| b.base.opacity = 0.5)),
            ("flow", Box::new(|b| b.base.flow = 0.2)),
            ("pressure", Box::new(|b| b.base.pressure_size = false)),
            ("tip angle", Box::new(|b| b.tip.angle = 30.0)),
            (
                "tip image",
                Box::new(|b| b.tip.image = yolu_core::builtin_tip("dots")),
            ),
            ("jitter", Box::new(|b| b.jitter.scatter = 1.0)),
            (
                "texture",
                Box::new(|b| {
                    b.texture = Some(crate::engine::PaperTexture::new(
                        yolu_core::builtin_tip("grain").unwrap(),
                        0.5,
                    ))
                }),
            ),
            ("dual", Box::new(|b| b.dual = Some(Default::default()))),
            ("color", Box::new(|b| b.color.hue = 0.3)),
            ("controls", Box::new(|b| b.controls.fade_size = 50)),
            ("taper", Box::new(|b| b.assist.taper_in = 30.0)),
            ("curve", Box::new(|b| b.assist.curve = true)),
            ("effect", Box::new(|b| b.effect = BrushEffect::BLUR)),
        ];
        let mut seen = vec![key];
        for (name, change) in &changes {
            let mut b = base.clone();
            change(&mut b);
            let k = key_of(&b, spec);
            assert!(!seen.contains(&k), "{name}: 札が変わらない");
            seen.push(k);
        }
        assert_ne!(key_of(&base, spec), key_of(&base, SampleSpec::row(true)));
        assert_ne!(key_of(&base, spec), key_of(&base, SampleSpec::tool(false)));
    }

    #[test]
    fn images_with_the_same_name_and_size_but_other_pixels_get_other_keys() {
        use crate::engine::{DualBrush, PaperTexture};
        let tip =
            |pixels: [u8; 4]| Arc::new(BrushTip::new("取り込み", 2, 2, pixels.to_vec()).unwrap());
        let (a, b) = (tip([0, 255, 255, 0]), tip([255, 0, 0, 255]));
        let same_as_a = tip([0, 255, 255, 0]);
        assert_eq!(
            format!("{a:?}"),
            format!("{b:?}"),
            "Debug だけでは見分けられない画像"
        );
        let spec = SampleSpec::row(false);
        type Put = fn(&mut Brush, Arc<BrushTip>);
        let places: [(&str, Put); 4] = [
            ("tip", |br, t| br.tip.image = Some(t)),
            ("tips", |br, t| br.tip.images = vec![t]),
            ("texture", |br, t| {
                br.texture = Some(PaperTexture::new(t, 0.5))
            }),
            ("dual", |br, t| {
                br.dual = Some(DualBrush {
                    tip: Some(t),
                    ..DualBrush::default()
                })
            }),
        ];
        let mut cache = SampleCache::default();
        for (place, put) in places {
            let make = |t: &Arc<BrushTip>| {
                let mut br = Brush::default();
                put(&mut br, t.clone());
                br
            };
            let (ba, bb, bc) = (make(&a), make(&b), make(&same_as_a));
            assert_ne!(
                key_of(&ba, spec),
                key_of(&bb, spec),
                "{place}: 画素だけが違う"
            );
            assert_eq!(
                key_of(&ba, spec),
                key_of(&bc, spec),
                "{place}: 中身が同じなら同じ札"
            );
            // キャッシュも同じ札を使い、画素だけが違うブラシには別の見本を描く
            cache.begin_frame(1000 + cache.stats.renders);
            let ka = cache.request(&ba, spec).unwrap();
            let kb = cache.request(&bb, spec).unwrap();
            assert_eq!((ka, kb), (key_of(&ba, spec), key_of(&bb, spec)), "{place}");
            assert_ne!(ka, kb, "{place}");
        }
        // 同じ画像は何度要求しても指紋を 1 度しか計算せず、描き直さない
        let renders = cache.stats.renders;
        let prints = cache.tip_prints.0.len();
        cache.begin_frame(5000);
        let mut br = Brush::default();
        br.tip.image = Some(a.clone());
        cache.request(&br, spec);
        cache.request(&br, spec);
        assert_eq!(cache.stats.renders, renders);
        assert_eq!(cache.tip_prints.0.len(), prints);
        // 覚えた画像の数は上限を超えない（捨てられた画像の置き場所を別の画像が使っても、指紋は取り違えない）
        for i in 0..(MAX_TIP_PRINTS * 2) {
            let t = Arc::new(BrushTip::new("x", 1, 1, vec![i as u8]).unwrap());
            let mut br = Brush::default();
            br.tip.image = Some(t);
            let k = {
                let prints = &mut cache.tip_prints;
                key_with(&br, spec, &mut |t| prints.print(t))
            };
            assert_eq!(k, key_of(&br, spec), "{i}");
        }
        assert!(cache.tip_prints.0.len() <= MAX_TIP_PRINTS);
    }

    #[test]
    fn the_cache_redraws_only_changed_brushes_and_caps_work_and_memory() {
        let spec = SampleSpec::row(false);
        let mut cache = SampleCache::default();
        let mut brushes: Vec<Brush> = (0..10)
            .map(|i| {
                let mut b = Brush::default();
                b.base.radius = 4.0 + i as f64;
                b
            })
            .collect();
        // 1 フレームに描くのは上限まで。残りは次のフレーム
        cache.begin_frame(1);
        let ready: Vec<bool> = brushes
            .iter()
            .map(|b| cache.request(b, spec).is_some())
            .collect();
        assert_eq!(ready.iter().filter(|r| **r).count(), RENDERS_PER_FRAME);
        assert_eq!(cache.stats.renders, RENDERS_PER_FRAME as u64);
        assert_eq!(cache.stats.deferred, (10 - RENDERS_PER_FRAME) as u64);
        for frame in 2..=4 {
            cache.begin_frame(frame);
            for b in &brushes {
                cache.request(b, spec);
            }
        }
        assert_eq!(cache.len(), 10);
        assert_eq!(
            cache.stats.renders, 10,
            "全部が揃ったら、1 つも描き直さない"
        );
        // 同じフレームで begin_frame を重ねても数え直さない
        let hits = cache.stats.hits;
        cache.begin_frame(4);
        cache.request(&brushes[0], spec);
        assert_eq!(cache.stats.hits, hits + 1);
        // 設定が変わった 1 つだけ描き直す
        brushes[3].jitter.size = 0.4;
        cache.begin_frame(5);
        for b in &brushes {
            cache.request(b, spec).expect("1 つだけなので上限内");
        }
        assert_eq!(cache.stats.renders, 11);
        // 見本に出ない設定が変わっても描き直さない
        brushes[5].seed = 7;
        brushes[6].assist.stabilizer = 50.0;
        brushes[7].base.color = Rgba8::new(9, 9, 9, 255);
        cache.begin_frame(6);
        for b in &brushes {
            cache.request(b, spec).unwrap();
        }
        assert_eq!(cache.stats.renders, 11);
        // 覚える量の上限: 大きな見本を溜め続けても、バイト数・枚数の上限を超えない
        let big = SampleSpec::detail(false);
        for i in 0..40 {
            if i % RENDERS_PER_FRAME == 0 {
                cache.begin_frame(100 + i as u64);
            }
            let mut b = Brush::default();
            b.base.radius = 1.0 + i as f64 * 0.37;
            let _ = cache.request(&b, big);
            assert!(cache.bytes() <= MAX_BYTES, "{}", cache.bytes());
            assert!(cache.len() <= MAX_ENTRIES);
        }
        assert!(cache.stats.evicted > 0);
    }

    #[test]
    fn textures_are_made_lazily_from_the_cached_image() {
        let ctx = egui::Context::default();
        let mut cache = SampleCache::default();
        cache.begin_frame(1);
        let key = cache
            .request(&Brush::default(), SampleSpec::row(false))
            .unwrap();
        let a = cache.texture(&ctx, key).unwrap();
        assert_eq!(cache.texture(&ctx, key), Some(a), "同じ絵を作り直さない");
        assert_eq!(cache.texture(&ctx, key ^ 1), None);
        assert_eq!(cache.image(key).unwrap().width, 340);
    }

    /// 設定を変えた直後のフレームは、新しい見本ができるまで、その所の前の絵を出す（紙だけにしない）。できたら新しい絵に替わる。
    /// 一度も絵を出していない所は紙だけ。前の絵は、出している間は覚えの上限で捨てられない。
    #[test]
    fn a_place_keeps_showing_its_last_sample_until_the_new_one_is_drawn() {
        let ctx = egui::Context::default();
        let mut cache = SampleCache::default();
        cache.render_in_background(&ctx);
        let spec = SampleSpec::tool(false);
        let (tool, other) = (egui::Id::new("tool"), egui::Id::new("other"));
        let mut frame = 0;
        let mut until_current = |cache: &mut SampleCache, brush: &Brush| {
            let start = std::time::Instant::now();
            loop {
                frame += 1;
                cache.begin_frame(frame);
                let shown = cache.shown(tool, brush, spec);
                if shown.current {
                    return shown.key.unwrap();
                }
                assert!(start.elapsed() < std::time::Duration::from_secs(20));
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        };
        let before = Brush::default();
        let old = until_current(&mut cache, &before);
        // つまみを動かした直後のフレーム: 新しい札の絵はまだ無いので、前の絵
        let mut after = before.clone();
        after.base.radius = 30.0;
        cache.begin_frame(10_000);
        let shown = cache.shown(tool, &after, spec);
        assert_eq!(
            shown,
            Shown {
                key: Some(old),
                current: false
            }
        );
        assert!(cache.image(old).is_some());
        // ほかの所（一度も出していない）は紙だけ
        let mut third = before.clone();
        third.base.radius = 40.0;
        assert_eq!(
            cache.shown(other, &third, spec),
            Shown {
                key: None,
                current: false
            }
        );
        // できたら新しい絵
        let new = until_current(&mut cache, &after);
        assert_ne!(new, old);
        assert_eq!(cache.shown(tool, &after, spec).key, Some(new));
    }

    /// 池を塞ぐ試験どうしを 1 つずつにする（片方が全体の池を塞いだまま、もう片方が見本の池を塞いで、互いを待つのを避ける）。
    static POOL_TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn one_pool_test_at_a_time() -> std::sync::MutexGuard<'static, ()> {
        POOL_TESTS.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 旗が立つまで居座る仕事を、全体の rayon の池のスレッドの数だけ出して、池を塞ぐ。落とすと（試験が途中で失敗しても）旗を立てて放す。
    struct BlockedGlobalPool(Arc<std::sync::atomic::AtomicBool>);

    impl BlockedGlobalPool {
        fn new() -> Self {
            use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
            use std::time::{Duration, Instant};
            let release = Arc::new(AtomicBool::new(false));
            let started = Arc::new(AtomicUsize::new(0));
            let threads = rayon::current_num_threads();
            for _ in 0..threads {
                let (release, started) = (release.clone(), started.clone());
                rayon::spawn(move || {
                    started.fetch_add(1, Ordering::SeqCst);
                    let deadline = Instant::now() + Duration::from_secs(60);
                    while !release.load(Ordering::SeqCst) && Instant::now() < deadline {
                        std::thread::sleep(Duration::from_millis(1));
                    }
                });
            }
            let this = BlockedGlobalPool(release);
            let deadline = Instant::now() + Duration::from_secs(30);
            while started.load(Ordering::SeqCst) < threads {
                assert!(Instant::now() < deadline, "全体の池を塞げない");
                std::thread::sleep(Duration::from_millis(1));
            }
            this
        }
    }

    impl Drop for BlockedGlobalPool {
        fn drop(&mut self) {
            self.0.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }

    /// 全体の rayon の池のスレッドが全部ふさがっていても、見本は専用の池で描かれて届く。
    #[test]
    fn a_background_sample_is_drawn_even_while_every_thread_of_the_global_pool_is_busy() {
        let _one = one_pool_test_at_a_time();
        let ctx = egui::Context::default();
        let mut cache = SampleCache::default();
        cache.render_in_background(&ctx);
        let (brush, spec) = (Brush::default(), SampleSpec::row(false));
        let blocked = BlockedGlobalPool::new();
        let start = std::time::Instant::now();
        let mut frame = 0;
        let key = loop {
            frame += 1;
            cache.begin_frame(frame);
            if let Some(key) = cache.request(&brush, spec) {
                break key;
            }
            assert!(
                start.elapsed() < std::time::Duration::from_secs(20),
                "全体の池がふさがっている間、見本が描かれない"
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        };
        assert_eq!(cache.image(key).unwrap().width, spec.width);
        drop(blocked);
    }

    /// 見本の仕事は専用の少数のスレッドで走り、いくら積んでも全体の池のスレッドを握らない（全体の池の仕事は止まらず進む）。
    #[test]
    fn samples_run_on_their_own_few_threads_and_never_hold_the_global_pool() {
        let _one = one_pool_test_at_a_time();
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::mpsc::{channel, RecvTimeoutError};
        use std::time::Duration;
        struct Release(Arc<AtomicBool>);
        impl Drop for Release {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let release = Release(Arc::new(AtomicBool::new(false)));
        let (tx, started) = channel();
        // 全体の池のスレッドの数より多く、居座る見本の仕事を積む
        let jobs = rayon::current_num_threads() + POOL_THREADS + 2;
        for _ in 0..jobs {
            let (flag, tx) = (release.0.clone(), tx.clone());
            spawn_for_test(move || {
                let name = std::thread::current()
                    .name()
                    .map(str::to_owned)
                    .unwrap_or_default();
                let _ = tx.send(name);
                let deadline = std::time::Instant::now() + Duration::from_secs(60);
                while !flag.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(1));
                }
            })
            .unwrap();
        }
        // 走り出すのは専用の池のスレッドの数だけ（残りは待つ）
        let mut names = Vec::new();
        for _ in 0..POOL_THREADS {
            names.push(
                started
                    .recv_timeout(Duration::from_secs(20))
                    .expect("専用の池で走り出す"),
            );
        }
        assert!(
            names.iter().all(|n| n.starts_with(POOL_THREAD_PREFIX)),
            "見本は専用の池のスレッドで走る: {names:?}"
        );
        assert!(
            matches!(
                started.recv_timeout(Duration::from_millis(200)),
                Err(RecvTimeoutError::Timeout)
            ),
            "専用の池のスレッドの数を超えて走らない"
        );
        // そのあいだも、全体の池の仕事は進む
        let (done, finished) = channel();
        rayon::spawn(move || {
            let _ = done.send(rayon::current_thread_index().is_some());
        });
        assert_eq!(
            finished.recv_timeout(Duration::from_secs(20)),
            Ok(true),
            "見本が詰まっていても、全体の池の仕事は走る"
        );
        drop(release);
    }
}
