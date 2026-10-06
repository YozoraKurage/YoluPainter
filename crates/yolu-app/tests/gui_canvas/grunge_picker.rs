//! グランジのプリセットの格子: 見本は別のスレッドで 1 度だけ作り、できた分から出る。選ぶ・名前（日英）・見本の違い。
use crate::common;

use egui::{pos2, vec2, Rect};
use egui_kittest::{kittest::Queryable, Harness};
use yolu_app::lang::Lang;
use yolu_app::panels::grunge_picker as picker;
use yolu_app::YoluApp;
use yolu_core::generator::GrungePreset;

struct Shown {
    current: GrungePreset,
    lang: Lang,
    picked: Vec<GrungePreset>,
}

fn harness(lang: Lang) -> Harness<'static, Shown> {
    let mut ready = false;
    common::gpu_thread::builder()
        .with_size(vec2(240.0, 200.0))
        .renderer(common::shared_gpu::renderer())
        .build_ui_state(
            move |ui, state: &mut Shown| {
                if !ready {
                    YoluApp::setup(ui.ctx());
                    ready = true;
                    ui.ctx().request_repaint();
                    return;
                }
                let area = Rect::from_min_size(pos2(8.0, 8.0), vec2(224.0, picker::height(224.0)));
                if let Some(p) = picker::show(ui, area, state.current, state.lang) {
                    state.current = p;
                    state.picked.push(p);
                }
            },
            Shown {
                current: GrungePreset::Rust,
                lang,
                picked: Vec::new(),
            },
        )
}

fn wait_for_thumbnails(h: &mut Harness<'_, Shown>) {
    for _ in 0..600 {
        h.step();
        if picker::ready() {
            h.run();
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    panic!("見本ができない");
}

#[test]
fn every_preset_has_a_distinct_nonuniform_thumbnail() {
    let images: Vec<_> = GrungePreset::ALL
        .iter()
        .map(|p| picker::render(*p))
        .collect();
    for (p, image) in GrungePreset::ALL.iter().zip(&images) {
        assert_eq!(image.size, [picker::THUMB as usize; 2]);
        let first = image.pixels[0];
        assert!(
            image.pixels.iter().any(|c| *c != first),
            "{p:?}: 一様な見本"
        );
        assert!(image
            .pixels
            .iter()
            .all(|c| c.r() == c.g() && c.g() == c.b() && c.a() == 255));
    }
    for i in 0..images.len() {
        for j in i + 1..images.len() {
            assert_ne!(
                images[i].pixels,
                images[j].pixels,
                "{:?} と {:?}",
                GrungePreset::ALL[i],
                GrungePreset::ALL[j]
            );
        }
    }
    // 同じ設定は同じ見本（スレッドで作っても変わらない）
    let again = picker::render(GrungePreset::Cracks);
    assert_eq!(again.pixels, images[GrungePreset::Cracks as usize].pixels);
}

#[test]
fn preset_names_are_short_distinct_and_english_has_no_japanese() {
    let mut ja = std::collections::HashSet::new();
    let mut en = std::collections::HashSet::new();
    for p in GrungePreset::ALL {
        let (j, e) = (
            picker::preset_name(Lang::Ja, p),
            picker::preset_name(Lang::En, p),
        );
        assert!(ja.insert(j) && en.insert(e), "{p:?}");
        assert!(
            !e.chars()
                .any(|c| matches!(c, '\u{3000}'..='\u{30ff}' | '\u{4e00}'..='\u{9fff}')),
            "{e}"
        );
        assert!(j.chars().count() <= 6 && e.chars().count() <= 14, "{j} {e}");
    }
}

#[test]
fn the_grid_fills_in_as_thumbnails_arrive_and_a_click_picks_one() {
    let mut h = harness(Lang::Ja);
    wait_for_thumbnails(&mut h);
    let cols = ((224.0 + picker::GAP) / (picker::CELL + picker::GAP)).floor() as usize;
    // 4 番目（ほこり）の格子の中心
    let target = GrungePreset::Dust as usize;
    let (cx, cy) = (target % cols, target / cols);
    let at = pos2(
        8.0 + cx as f32 * (picker::CELL + picker::GAP) + picker::CELL / 2.0,
        8.0 + cy as f32 * (picker::CELL + picker::GAP) + picker::CELL / 2.0,
    );
    h.event(egui::Event::PointerMoved(at));
    h.step();
    h.event(egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    h.step();
    h.event(egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    h.run();
    assert_eq!(h.state().picked, [GrungePreset::Dust]);
    assert_eq!(h.state().current, GrungePreset::Dust);
    // 選んだ印と名前（アクセシビリティの名前）
    assert!(h.query_by_label("ほこり").is_some());
    assert!(h.query_by_label("汚れの斑").is_some());
}

#[test]
fn snapshot_grunge_picker() {
    let mut h = harness(Lang::Ja);
    wait_for_thumbnails(&mut h);
    h.snapshot("grunge_picker");
    let mut e = harness(Lang::En);
    wait_for_thumbnails(&mut e);
    assert!(e.query_by_label("Wood Grain").is_some());
    assert!(e.query_by_label("Rust").is_some());
}
