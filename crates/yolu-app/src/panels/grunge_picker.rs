//! グランジのプリセットを選ぶ格子。プリセットごとの絵（サムネイル）を別のスレッドで 1 度だけ作り、できた分から出す
//! （`yolu_core::generator::preview` の UV 空間の見本。文書にも棚にも触らない）。文字は名前だけで、説明はツールチップ。
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use egui::{pos2, vec2, ColorImage, Context, Id, Rect, Sense, TextureHandle, TextureOptions, Ui};
use yolu_core::generator::{self, GrungePreset};

use crate::lang::Lang;
use crate::ui::theme as t;
use crate::ui::widgets as w;

/// 見本の 1 辺（画素）。
pub const THUMB: u32 = 64;
/// 格子の 1 つの大きさ（点）と隙間。
pub const CELL: f32 = 48.0;
pub const GAP: f32 = 4.0;

/// プリセットの名前。
pub fn preset_name(lang: Lang, preset: GrungePreset) -> &'static str {
    match preset {
        GrungePreset::Stain => lang.pick("汚れの斑", "Stains"),
        GrungePreset::Rust => lang.pick("錆の斑", "Rust"),
        GrungePreset::Scratches => lang.pick("傷の筋", "Scratches"),
        GrungePreset::Dust => lang.pick("ほこり", "Dust"),
        GrungePreset::Fingerprints => lang.pick("指紋", "Fingerprints"),
        GrungePreset::Weave => lang.pick("布目", "Weave"),
        GrungePreset::Cracks => lang.pick("ひび", "Cracks"),
        GrungePreset::Splatter => lang.pick("飛沫", "Splatter"),
        GrungePreset::Peeling => lang.pick("塗装の剥げ", "Peeling"),
        GrungePreset::WoodGrain => lang.pick("木目", "Wood Grain"),
        GrungePreset::Pebbles => lang.pick("革のしぼ", "Leather Grain"),
    }
}

/// 全プリセットの見本（画像の向き。プロセスで 1 度だけ作る）。
type Images = Arc<Vec<(GrungePreset, ColorImage)>>;

#[derive(Default)]
struct Store {
    started: bool,
    images: Option<Images>,
}

fn store() -> &'static Mutex<Store> {
    static STORE: OnceLock<Mutex<Store>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(Store::default()))
}

/// 1 つのプリセットの見本（既定の設定。画像は上の行から）。
pub fn render(preset: GrungePreset) -> ColorImage {
    let settings = generator::Settings::grunge(preset);
    let size = THUMB as usize;
    // 見本は生成のエラーにならない設定（既定）。万一でも空の灰色にする
    let px =
        generator::preview(&settings, THUMB, THUMB).unwrap_or_else(|_| vec![128; size * size * 4]);
    let mut pixels = Vec::with_capacity(size * size);
    for row in (0..size).rev() {
        for col in 0..size {
            let g = px[(row * size + col) * 4];
            pixels.push(egui::Color32::from_gray(g));
        }
    }
    let mut image = ColorImage::new([size, size], pixels);
    image.source_size = vec2(size as f32, size as f32);
    image
}

/// 全部の見本。まだなら（最初の呼び出しで）別のスレッドで作り始めて None を返す。
fn images(ctx: &Context) -> Option<Images> {
    let mut s = store().lock().unwrap();
    if let Some(images) = &s.images {
        return Some(images.clone());
    }
    if !s.started {
        s.started = true;
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let images: Images =
                Arc::new(GrungePreset::ALL.iter().map(|p| (*p, render(*p))).collect());
            store().lock().unwrap().images = Some(images);
            ctx.request_repaint();
        });
    }
    // 別の窓（コンテキスト）からも、できるまで見に来る
    ctx.request_repaint_after(std::time::Duration::from_millis(30));
    None
}

type Textures = Arc<Mutex<HashMap<GrungePreset, TextureHandle>>>;

/// プリセットの見本のテクスチャ（できていれば。初めて使うときにこの窓の GPU へ上げる）。
pub fn texture(ctx: &Context, preset: GrungePreset) -> Option<TextureHandle> {
    let images = images(ctx)?;
    let textures: Textures = ctx.data_mut(|d| {
        d.get_temp_mut_or_insert_with::<Textures>(Id::new("grunge.textures"), Default::default)
            .clone()
    });
    let mut textures = textures.lock().unwrap();
    if let Some(h) = textures.get(&preset) {
        return Some(h.clone());
    }
    let (_, image) = images.iter().find(|(p, _)| *p == preset)?;
    let handle = ctx.load_texture(
        format!("grunge:{}", preset.id()),
        image.clone(),
        TextureOptions::LINEAR,
    );
    textures.insert(preset, handle.clone());
    Some(handle)
}

/// 見本が全部できているか（この窓のテクスチャに上げたかは問わない）。
pub fn ready() -> bool {
    store().lock().unwrap().images.is_some()
}

/// 幅 `width` の格子の高さ。
pub fn height(width: f32) -> f32 {
    let rows = GrungePreset::ALL.len().div_ceil(columns(width));
    rows as f32 * (CELL + GAP) - GAP
}

fn columns(width: f32) -> usize {
    (((width + GAP) / (CELL + GAP)).floor() as usize).max(1)
}

/// プリセットの格子を `area` の左上から描く。選んだものを返す（`current` は今のプリセット）。
pub fn show(ui: &mut Ui, area: Rect, current: GrungePreset, lang: Lang) -> Option<GrungePreset> {
    let columns = columns(area.width());
    let mut picked = None;
    for (i, preset) in GrungePreset::ALL.iter().copied().enumerate() {
        let (cx, cy) = (i % columns, i / columns);
        let cell = Rect::from_min_size(
            pos2(
                area.left() + cx as f32 * (CELL + GAP),
                area.top() + cy as f32 * (CELL + GAP),
            ),
            vec2(CELL, CELL),
        );
        let name = preset_name(lang, preset);
        let response = ui.interact(
            cell,
            ui.id().with(("grunge.preset", preset.id())),
            Sense::click(),
        );
        let painter = ui.painter();
        match texture(ui.ctx(), preset) {
            Some(handle) => {
                painter.image(
                    handle.id(),
                    cell,
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            }
            None => {
                // 見本ができるまで（別のスレッド）は灰色の箱
                w::rounded(painter, cell, t::PANEL_HEADER, 3.0);
                w::icon(painter, cell, "texture", t::TEXT_DIM, 20.0);
            }
        }
        if preset == current {
            w::outline(painter, cell, t::ACCENT, 2.0, 0.0);
        } else if response.hovered() {
            w::outline(painter, cell, t::TEXT_DIM, 1.0, 0.0);
        } else {
            w::outline(painter, cell, t::BORDER, 1.0, 0.0);
        }
        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::SelectableLabel,
                true,
                preset == current,
                name,
            )
        });
        if response.clicked() {
            picked = Some(preset);
        }
        let _ = response.on_hover_text(name);
    }
    picked
}
