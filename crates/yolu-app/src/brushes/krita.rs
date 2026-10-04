//! 同梱の Krita 4 の筆先（76 個）を、詳細の窓の「形状」の筆先の格子に出すための読み込みとサムネイル。
//!
//! 76 個を読んで画素にするのは数百ミリ秒かかる（`yolu_io::brushes::bundled`）ので、実際の窓（`load_in_background`）は、格子を初めて
//! 出すときに別のスレッドで読み、見本の絵（灰色の 48 画素）まで作って、できたら描き直しを頼む。画面のスレッドは、できた分を
//! 受けるだけ。既定（試験・画面を持たない使い方）は、初めて出すときにその場で読む（撮る絵が読み込みの速さで変わらない）。
//! 格子の並びは同梱の並び（ファイル名の順）のままで、添字がそのまま `BrushOp::KritaTip` の指す筆先になる。

use std::sync::mpsc::{channel, Receiver};

use egui::{ColorImage, TextureHandle, TextureId, TextureOptions};
use yolu_core::BrushTip;

use super::store::krita;
use crate::engine::TipShape;

/// 筆先の見本の大きさ（画素）。
pub const THUMB: usize = 48;

/// 筆先の見本（白地に黒。画像の先端は縦横比を保って長い辺を枠に合わせる。`None` は縁がやわらかい丸）。
pub fn thumbnail(tip: Option<&BrushTip>) -> ColorImage {
    let n = THUMB;
    let mut pixels = Vec::with_capacity(n * n);
    for y in 0..n {
        for x in 0..n {
            let fx = (x as f64 + 0.5) / n as f64 - 0.5;
            let fy = 0.5 - (y as f64 + 0.5) / n as f64; // 画像の上が先端の上（行は下から）
            let coverage = match tip {
                Some(t) => {
                    // 長い辺を見本の幅に合わせて、縦横比を保つ
                    let aspect = t.width() as f64 / t.height() as f64;
                    let (u, v) = if aspect >= 1.0 {
                        (0.5 + fx, 0.5 + fy * aspect)
                    } else {
                        (0.5 + fx / aspect, 0.5 + fy)
                    };
                    t.sample(u, v)
                }
                None => {
                    let d = (fx * fx + fy * fy).sqrt() * 2.0;
                    ((0.95 - d) / 0.4).clamp(0.0, 1.0)
                }
            };
            pixels.push(egui::Color32::from_gray(
                (255.0 * (1.0 - coverage)).round() as u8
            ));
        }
    }
    ColorImage::new([n, n], pixels)
}

/// 格子の 1 つ。
struct Item {
    name: String,
    /// 名前の検索に使う小文字（ID のファイル名も含める）。
    haystack: String,
    image: ColorImage,
    texture: Option<TextureHandle>,
}

enum Load {
    NotStarted,
    Loading(Receiver<Vec<(String, String, ColorImage)>>),
    Ready(Vec<Item>),
}

/// Krita の筆先の格子の状態（読み込みと見本・名前の検索）。
pub struct KritaTips {
    load: Load,
    background: bool,
    /// 名前の検索の文字。
    pub search: String,
}

impl Default for KritaTips {
    fn default() -> Self {
        KritaTips {
            load: Load::NotStarted,
            background: false,
            search: String::new(),
        }
    }
}

/// 同梱の筆先 76 個の格子の中身（名前・検索の文字・見本）。
fn load_items() -> Vec<(String, String, ColorImage)> {
    krita()
        .brushes
        .iter()
        .map(|b| {
            let tip = b
                .brush
                .tip
                .image
                .as_deref()
                .or_else(|| b.brush.tip.images.first().map(|t| &**t));
            let file = b.id.rsplit('/').next().unwrap_or_default();
            (
                b.name.clone(),
                format!("{} {file}", b.name).to_lowercase(),
                thumbnail(tip),
            )
        })
        .collect()
}

fn ready(items: Vec<(String, String, ColorImage)>) -> Load {
    Load::Ready(
        items
            .into_iter()
            .map(|(name, haystack, image)| Item {
                name,
                haystack,
                image,
                texture: None,
            })
            .collect(),
    )
}

impl KritaTips {
    /// 読み込みを別のスレッドで行う（実際の窓。できたら描き直しを頼む）。
    pub fn load_in_background(&mut self) {
        self.background = true;
    }

    /// 格子を出すとき: 読み込みを始めていなければ始め、終わっていれば受ける。読み終えたら true。
    pub fn poll(&mut self, ctx: &egui::Context) -> bool {
        match &self.load {
            Load::NotStarted if !self.background => {
                self.load = ready(load_items());
                true
            }
            Load::NotStarted => {
                let (tx, rx) = channel();
                let ctx = ctx.clone();
                let spawned = std::thread::Builder::new()
                    .name("yolu-krita-tips".into())
                    .spawn(move || {
                        let _ = tx.send(load_items());
                        ctx.request_repaint();
                    });
                self.load = match spawned {
                    Ok(_) => Load::Loading(rx),
                    // スレッドを作れないときは出さない（格子は空のまま）
                    Err(_) => Load::Ready(Vec::new()),
                };
                false
            }
            Load::Loading(rx) => {
                if let Ok(items) = rx.try_recv() {
                    self.load = ready(items);
                    true
                } else {
                    false
                }
            }
            Load::Ready(_) => true,
        }
    }

    pub fn is_ready(&self) -> bool {
        matches!(self.load, Load::Ready(_))
    }

    /// 検索に合う筆先の添字（同梱の並びの添字）。検索が空なら全部。
    pub fn matches(&self) -> Vec<usize> {
        let Load::Ready(items) = &self.load else {
            return Vec::new();
        };
        let query = self.search.trim().to_lowercase();
        items
            .iter()
            .enumerate()
            .filter(|(_, item)| query.is_empty() || item.haystack.contains(&query))
            .map(|(i, _)| i)
            .collect()
    }

    pub fn name(&self, index: usize) -> Option<&str> {
        match &self.load {
            Load::Ready(items) => items.get(index).map(|i| i.name.as_str()),
            _ => None,
        }
    }

    /// 見本の絵（初めて出すときに作る）。
    pub fn texture(&mut self, ctx: &egui::Context, index: usize) -> Option<TextureId> {
        let Load::Ready(items) = &mut self.load else {
            return None;
        };
        let item = items.get_mut(index)?;
        let handle = item.texture.get_or_insert_with(|| {
            ctx.load_texture(
                format!("krita-tip-{index}"),
                item.image.clone(),
                TextureOptions::LINEAR,
            )
        });
        Some(handle.id())
    }

    /// 試験用: 読み終わるまで待つ。
    #[doc(hidden)]
    pub fn wait_ready(&mut self, ctx: &egui::Context) {
        let start = std::time::Instant::now();
        while !self.poll(ctx) && start.elapsed() < std::time::Duration::from_secs(60) {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
}

/// 今の筆先が、同梱の Krita の筆先のどれか（添字）。読み込みが済んでいなければ None。
pub fn current_index(tip: &TipShape) -> Option<usize> {
    if tip.image.is_none() && tip.images.is_empty() {
        return None;
    }
    // 同梱の筆先は読み込み済みのときだけ探す（読み込みを画面のスレッドで待たない）
    let set = super::store::krita_loaded()?;
    set.brushes.iter().position(|b| {
        if tip.images.is_empty() {
            b.brush.tip.images.is_empty() && b.brush.tip.image == tip.image
        } else {
            tip.image.is_none() && b.brush.tip.images == tip.images
        }
    })
}
