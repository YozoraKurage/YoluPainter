//! ID の色: 焼いた ID マップの色から選択範囲を作る（ID の色で選択）と、手動の ID の色（メッシュの塊ごとの色）の編集。
//!
//! - 選択は、押した所の ID の色（2D はその画素、3D は当たった面の UV のテクセル）から、許し幅の中の色の画素を選択範囲にする
//!   （core の `SelectionMask::from_id_colors`。Shift 追加・Ctrl 削除・Shift+Ctrl 交差。1 回の Undo）。ID マップが無い・古い
//!   （大きさやモデルが今と違う）ときは何も選ばず、短い理由を出す。読むのはマップだけで、元のモデルには触れない。
//! - 手動の ID の色は文書の状態（`Document::id_colors`。メッシュの塊の番号 → 色とモデルの指紋）。1 回の Undo（色のウィンドウのドラッグは
//!   まとめて 1 回）。.ylp には正本の版 19 の塊として書き、開き直すと戻る（yolu-io）。
//!
//! 焼いた ID マップは、ベイク（`bake`）がセットごとに持つメッシュマップ。使えるのは、今の条件（モデル・文書の大きさ・セットのスロット・
//! 設定・手動の ID の色）で焼いたものだけ（`bake` の照合）。手動の ID の色を直すと、前の ID マップは「古い」になり、焼き直すと色が入る。

use std::sync::Arc;

use egui::Pos2;
use yolu_core::geometry::pick;
use yolu_core::glam::Vec2;
use yolu_core::id_colors::{hex, near, try_get, try_get_at_uv};
use yolu_core::mesh_maps::{
    id_part_binding, BakedMeshMap, IdColorAssignments, MeshMapKind, MeshMapState,
};
use yolu_core::SelectionMask;

use super::tools::{read_only_message, Hover, Where};
use crate::lang::Lang;
use crate::notice::Source;
use crate::selection::{combine_name, combine_of};
use crate::state::AppState;
use crate::view3d::model::ViewModel;

/// 手動の ID の色の変更。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IdColorOp {
    /// 部品の色を決める（None は自動に戻す）。
    Set { part: usize, rgb: Option<u32> },
    /// 全部を自動に戻す。
    ResetAll,
    /// 色のウィンドウのドラッグの途中: 部品の色をその場で変え、続くドラッグの変更と 1 回の取り消しにまとめる（知らせは出さない）。
    Drag { part: usize, rgb: u32 },
    /// 色のウィンドウのドラッグの終わり: まとめを切り、ドラッグで色を変えていれば 1 回だけ知らせる。
    EndDrag,
}

/// モデルの部品（メッシュの塊）の割り当てと、そのモデルの指紋。
struct Parts {
    model: Arc<ViewModel>,
    /// 三角形ごとの部品の番号（モデルの全体で、`MeshBakeInput` の並び）。
    of_triangle: Vec<usize>,
    binding: String,
}

/// ID のツールの状態。
#[derive(Default)]
pub struct IdState {
    /// 手動の ID の色を直している部品（今のセットの部品の並びの番号）。
    pub part: usize,
    parts: Option<Parts>,
    /// 色のウィンドウのドラッグで手動の ID の色を変えた（まだ知らせていない）。そのときの履歴の段の数（捨てられていないかを見る）。
    dragged: Option<usize>,
}

impl AppState {
    /// 受けたままのモデルの部品（モデルが替わるまで作り直さない）。入力はベイクと同じもの（モデルの指紋が同じになる）。
    /// `wait` が false（毎フレームの表示用）なら、入力を作り終えていないときは別のスレッドで作らせて None を返す（UI を止めない）。
    /// true（押した・色を直したとき）なら、作り終えるまで待つ。
    fn id_parts(&mut self, wait: bool) -> Option<Result<&Parts, String>> {
        let lang = self.lang;
        let Some(model) = self.view3d.full_model().cloned() else {
            return Some(Err(lang.pick("モデルがありません", "No model").into()));
        };
        if !self
            .region
            .id
            .parts
            .as_ref()
            .is_some_and(|p| Arc::ptr_eq(&p.model, &model))
        {
            let input = if wait {
                self.bake_input()
            } else {
                self.bake_input_nowait()?
            };
            match input {
                Ok(input) => {
                    let (of_triangle, binding) = id_part_binding(&input);
                    self.region.id.parts = Some(Parts {
                        model,
                        of_triangle,
                        binding,
                    });
                }
                Err(e) => return Some(Err(e)),
            }
        }
        Some(Ok(self.region.id.parts.as_ref().expect("作った")))
    }

    /// 今のセットのメッシュの塊（番号の昇順）。モデルが無い・今のセットがモデルに無ければ空。モデルの入力を作り終えていなければ None
    /// （毎フレームの表示用。待たない。画面は「確かめています」を出す）。
    pub fn id_set_parts(&mut self) -> Option<Vec<usize>> {
        let Some(material) = self.region_model().map(|(_, m)| m) else {
            return Some(Vec::new());
        };
        let Ok(parts) = self.id_parts(false)? else {
            return Some(Vec::new());
        };
        let mut out: Vec<usize> = parts
            .model
            .geometry
            .triangles()
            .iter()
            .zip(&parts.of_triangle)
            .filter(|(t, _)| t.material == material)
            .map(|(_, p)| *p)
            .collect();
        out.sort_unstable();
        out.dedup();
        Some(out)
    }

    /// 部品の色の目安（手動の色が無いとき、焼いた ID マップの、その部品の最初の三角形の中心の色）。入力を作り終えるまでは None。
    pub fn id_part_hint(&mut self, part: usize) -> Option<u32> {
        let map = self.usable_id_map().ok()?;
        let parts = self.id_parts(false)?.ok()?;
        let i = parts.of_triangle.iter().position(|p| *p == part)?;
        let t = &parts.model.geometry.triangles()[i];
        let c = (t.uv_a + t.uv_b + t.uv_c) / 3.0;
        try_get_at_uv(&map, c.x as f64, c.y as f64).ok().flatten()
    }

    /// 手動の ID の色が別のモデルのものか（色があって、今のモデルの指紋と違う）。モデルが無い・入力を作り終えるまでは false。
    pub fn id_colors_foreign(&mut self) -> bool {
        let colors = self.doc.id_colors().clone();
        if colors.colors().is_empty() {
            return false;
        }
        match self.id_parts(false) {
            Some(Ok(p)) => colors.binding() != p.binding,
            _ => false,
        }
    }

    /// 今のセットの使える ID マップ（今の条件で焼いたもの）。無い・古いときは短い理由。モデルの入力を作り終えていなければ「確かめている」
    /// （毎フレームの表示用。待たない）。
    pub fn usable_id_map(&mut self) -> Result<Arc<BakedMeshMap>, String> {
        self.id_map(false)
    }

    /// 同じで、入力を作り終えるまで待つ（押したときの選択用）。
    pub fn usable_id_map_waiting(&mut self) -> Result<Arc<BakedMeshMap>, String> {
        self.id_map(true)
    }

    fn id_map(&mut self, wait: bool) -> Result<Arc<BakedMeshMap>, String> {
        let lang = self.lang;
        let index = self.sets.current_index();
        let Some(map) = self.sets.current().mesh_maps.get(MeshMapKind::Id).cloned() else {
            return Err(lang.pick("ID マップがありません", "No ID map").into());
        };
        let input = if wait {
            Some(self.bake_input())
        } else {
            self.bake_input_nowait()
        };
        let check = match input {
            None => {
                return Err(lang
                    .pick("ID マップを確かめています", "Checking the ID map")
                    .into())
            }
            Some(Ok(input)) => self.mesh_map_check_with(index, MeshMapKind::Id, Some(&input)),
            // モデルが無い・入力を作れない: 照合できない
            Some(Err(_)) => self.mesh_map_check_with(index, MeshMapKind::Id, None),
        };
        match check.map(|c| c.state) {
            Some(MeshMapState::Current) => Ok(map),
            Some(MeshMapState::Stale) => Err(lang
                .pick(
                    "ID マップが古いです（ベイクし直し）",
                    "The ID map is stale (bake again)",
                )
                .into()),
            _ => Err(lang
                .pick(
                    "ID マップを確かめられません",
                    "The ID map cannot be checked",
                )
                .into()),
        }
    }

    /// 手動の ID の色を変える（1 回の Undo。色のウィンドウのドラッグは離すまでを 1 回にまとめる。断られたら理由をステータスバーへ）。
    pub fn id_color_edit(&mut self, op: IdColorOp) {
        let lang = self.lang;
        if self.is_stroking() {
            self.refuse(Source::Bake, crate::lang::refusals::during_stroke(lang));
            return;
        }
        let drag = matches!(op, IdColorOp::Drag { .. });
        let next = match op {
            IdColorOp::EndDrag => {
                self.doc.end_coalescing();
                // まとめていた段が、押したままの Esc などで捨てられていれば（そのあとウィンドウの戻しが次のフレームに届く）、変わっていないので
                // 知らせない（直前の「取り消しました。」を上書きしない）。段を捨てると履歴の段の数が減るので、ドラッグで入れたときの数と比べる
                let kept = self.region.id.dragged.take() == Some(self.doc.undo_count());
                if kept {
                    self.info(Source::Bake, changed(lang));
                }
                return;
            }
            IdColorOp::ResetAll => Some(IdColorAssignments::default()),
            IdColorOp::Set { part, rgb } => self.id_color_next(part, rgb),
            IdColorOp::Drag { part, rgb } => self.id_color_next(part, Some(rgb)),
        };
        let Some(next) = next else {
            return;
        };
        match self.doc.set_id_colors(next, drag) {
            Ok(()) => {
                self.modified = true;
                self.region.hover = None;
                if drag {
                    self.region.id.dragged = Some(self.doc.undo_count());
                } else {
                    self.region.id.dragged = None;
                    self.info(Source::Bake, changed(lang));
                }
            }
            Err(e) => self.notify(
                crate::notice::Kind::of_core(&e),
                Source::Bake,
                lang.core_error(&e),
            ),
        }
    }

    /// 部品の色を `rgb`（None は自動）にした手動の ID の色。断るときは理由を出して None。
    fn id_color_next(&mut self, part: usize, rgb: Option<u32>) -> Option<IdColorAssignments> {
        let lang = self.lang;
        let colors = self.doc.id_colors().clone();
        // 押して直すときは、入力を作り終えるまで待つ
        let parts = match self.id_parts(true)? {
            Ok(p) => p,
            Err(m) => {
                self.refuse(Source::Bake, m);
                return None;
            }
        };
        let count = parts.of_triangle.iter().max().map_or(0, |m| m + 1);
        if part >= count {
            self.refuse(
                Source::Bake,
                lang.pick("その部品はありません", "No such part"),
            );
            return None;
        }
        if !colors.colors().is_empty() && colors.binding() != parts.binding {
            self.refuse(
                Source::Bake,
                lang.pick(
                    "手動の ID の色は別のモデルのものです",
                    "These manual ID colors belong to another model",
                ),
            );
            return None;
        }
        if rgb.is_some_and(|c| c > 0xffffff) {
            return None;
        }
        match colors.with_color(&parts.binding, part, rgb) {
            Ok(c) => Some(c),
            Err(e) => {
                self.fail(
                    Source::Bake,
                    lang.with_reason(
                        lang.pick("ID の色を変えられません", "Cannot change the ID color"),
                        lang.mesh_map_error(&e),
                    ),
                );
                None
            }
        }
    }
}

/// 手動の ID の色を変えた知らせ。
fn changed(lang: Lang) -> &'static str {
    lang.pick("手動の ID の色を変えました。", "Manual ID colors changed.")
}

/// ポインタの下の ID の色。取れなければ理由。
fn color_under(app: &mut AppState, w: Where, at: Pos2, map: &BakedMeshMap) -> Result<u32, String> {
    let lang = app.lang;
    let none = |s: &'static str, e: &'static str| lang.pick(s, e).to_owned();
    match w {
        Where::Surface(rect) => {
            let Some((model, material)) = app.region_model() else {
                return Err(app.region_missing_reason());
            };
            let view = app.view3d.camera.view(rect.width(), rect.height());
            let p = Vec2::new(at.x - rect.left(), at.y - rect.top());
            let Some(hit) = pick(&model.geometry, &view, p) else {
                return Err(none(
                    "ポインタの下にモデルがありません",
                    "Nothing of the model under the pointer",
                ));
            };
            if hit.material != material {
                let name = model.material_name(hit.material as usize, lang);
                return Err(super::tools::other_set_face(lang, &name));
            }
            match try_get_at_uv(map, hit.uv.x as f64, hit.uv.y as f64) {
                Ok(Some(rgb)) => Ok(rgb),
                _ => Err(none(
                    "その面には ID の色がありません",
                    "No ID color at that face",
                )),
            }
        }
        Where::Canvas(view) => {
            let (x, y) = view.to_canvas(at);
            if x < 0.0 || y < 0.0 || x >= app.doc.width() as f64 || y >= app.doc.height() as f64 {
                return Err(none("キャンバスの外です", "Outside the canvas"));
            }
            match try_get(map, x.floor() as i64, y.floor() as i64) {
                Ok(Some(rgb)) => Ok(rgb),
                _ => Err(none("そこには部品がありません", "No part there")),
            }
        }
    }
}

/// ID の色で選択: 押した所の色の画素を、今の選択範囲と組み合わせて選択範囲にする（1 回の Undo）。
pub fn select_by_id(app: &mut AppState, w: Where, at: Pos2) {
    let lang = app.lang;
    // 読むだけのセットは選択範囲も変えない（変えても、Undo が読むだけのセットで断られて戻せない）
    if let Some(message) = read_only_message(app) {
        app.refuse(Source::Selection, message);
        return;
    }
    let map = match app.usable_id_map_waiting() {
        Ok(m) => m,
        Err(reason) => {
            app.refuse(Source::Selection, reason);
            return;
        }
    };
    let rgb = match color_under(app, w, at, &map) {
        Ok(c) => c,
        Err(reason) => {
            app.refuse(Source::Selection, reason);
            return;
        }
    };
    // Generator「ID の色」の色を選んでいる間は、押した所の ID の色をその段に足す（選択範囲は作らない）
    if app.pick_id_color(rgb) {
        return;
    }
    let tolerance = app.region.id_tolerance;
    // 組み合わせ方は選択のツールと同じ（キーの修飾が無ければ、オプションバーで選んだ方）
    let mode = combine_of(
        app.sel.combine,
        egui::PointerButton::Primary,
        app.region.modifiers,
    );
    let mask = match SelectionMask::from_id_colors(&app.doc, &map, &[rgb], tolerance) {
        Ok(m) => m,
        Err(e) => {
            app.notify(
                crate::notice::Kind::of_core(&e),
                Source::Selection,
                lang.core_error(&e),
            );
            return;
        }
    };
    if let Err(e) = app.doc.combine_selection(&mask, mode) {
        app.notify(
            crate::notice::Kind::of_core(&e),
            Source::Selection,
            lang.core_error(&e),
        );
        return;
    }
    app.modified = true;
    app.info(
        Source::Selection,
        if app.doc.selection().is_none() {
            lang.pick("何も選択されていません。", "Nothing selected.")
                .into()
        } else {
            format!(
                "{} {} ± {} ({})",
                lang.pick("ID の色", "ID color"),
                hex(rgb),
                tolerance,
                combine_name(lang, mode)
            )
        },
    );
}

/// ID の色で選択の強調: ポインタの下の色のマップの部分を含む今のセットの三角形（三角形の中心の UV のテクセルが許し幅の中のもの）。
pub fn update_hover(app: &mut AppState, w: Where, at: Pos2) {
    let Ok(map) = app.usable_id_map() else {
        app.region.hover = None;
        return;
    };
    let Some((model, material)) = app.region_model() else {
        app.region.hover = None;
        return;
    };
    let Ok(rgb) = color_under(app, w, at, &map) else {
        app.region.hover = None;
        return;
    };
    let tolerance = app.region.id_tolerance;
    let key = (
        Arc::as_ptr(&map) as usize,
        Arc::as_ptr(&model.geometry) as usize,
        rgb,
        tolerance,
    );
    // 強調が今も ID の色のもの（持ち主の印が強調の側にある）で、同じ条件・同じ画面のときだけ使い回す
    if app
        .region
        .hover
        .as_ref()
        .is_some_and(|h| h.id_key == Some(key) && h.on_surface == w.is_surface())
    {
        return;
    }
    let mut tris = Vec::new();
    for (i, t) in model.geometry.triangles().iter().enumerate() {
        if t.material != material {
            continue;
        }
        let c = (t.uv_a + t.uv_b + t.uv_c) / 3.0;
        if let Ok(Some(found)) = try_get_at_uv(&map, c.x as f64, c.y as f64) {
            if near(found, rgb, tolerance) {
                tris.push(i as u32);
            }
        }
    }
    let Some(first) = tris.first().copied() else {
        app.region.hover = None;
        return;
    };
    let Some(index) = app.region_index() else {
        return;
    };
    let outline = index.outline(&tris);
    app.region.hover = Some(Hover {
        on_surface: w.is_surface(),
        triangle: first,
        key: ((rgb as u64) << 8) | tolerance as u64,
        tris: Arc::new(tris),
        outline: Arc::new(outline),
        geometry: model.geometry.clone(),
        erase: false,
        id_key: Some(key),
        visible: None,
    });
}

/// 手動の ID の色の見本と一覧の見出しに使う、部品の色（手動があればそれ、無ければ None）。
pub fn manual_color(app: &AppState, part: usize) -> Option<u32> {
    app.doc.id_colors().colors().get(&part).copied()
}

/// 色の 16 進（#RRGGBB）。
pub fn hex_of(rgb: u32) -> String {
    hex(rgb)
}

/// 16 進（#RGB・#RRGGBB。# は無くてもよい）から 0xRRGGBB。
pub fn parse_rgb(s: &str) -> Option<u32> {
    crate::state::parse_hex(s).map(|c| {
        let b = |v: f32| (v * 255.0).round() as u32;
        b(c[0]) << 16 | b(c[1]) << 8 | b(c[2])
    })
}

/// 手動の ID の色の状態（個数。色は文書の状態で、.ylp に保存される）。
pub fn manual_state_line(lang: Lang, count: usize) -> Option<String> {
    (count > 0).then(|| {
        lang.pick(
            format!("手動の色 {count} 個"),
            format!("{count} manual color{}", if count == 1 { "" } else { "s" }),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 手動の ID の色は .ylp に保存できる（正本の版 19 の塊）ので、状態の行は個数だけを言う。色が無ければ行も無い。
    #[test]
    fn the_manual_color_state_is_the_count_only() {
        assert_eq!(manual_state_line(Lang::Ja, 0), None);
        assert_eq!(manual_state_line(Lang::En, 0), None);
        assert_eq!(
            manual_state_line(Lang::Ja, 3).as_deref(),
            Some("手動の色 3 個")
        );
        assert_eq!(
            manual_state_line(Lang::En, 3).as_deref(),
            Some("3 manual colors")
        );
        assert_eq!(
            manual_state_line(Lang::En, 1).as_deref(),
            Some("1 manual color")
        );
    }
}
