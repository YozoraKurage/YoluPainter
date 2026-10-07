//! モデルの差し替えでの 3D のパスの付け直し（C# の `TexturePaintWindow.ModelChange` のパスの扱い）。付け直しの式は core の
//! `rebind_surface_path`（制御点を前のモデルの三角形と重心座標から 3D の位置に戻し、新しいモデルのいちばん近い面の点 = 法線が同じ向き・
//! 描くテクスチャセットのマテリアルの三角形だけ・許す距離の内側、へ置き直す）。ここは文書ごとに、描き直す・画素にする・残すを決める。
//! 1 点でも置けなければ付け直さず、今の画素を残してパスを外す。画素・透明度のロックで描き直せない層も、画素にする（画素は変えない）。
//! 前のモデルに結び付けたまま残すのは、すべてのロックの層だけ（画素にもできない）。位置は両方のモデルの空間で比べるので、ルートの
//! 置き方が同じモデルどうしを前提にする。
//!
//! 付け直せないパスは画素にする（前のモデルに結び付けたまま残さない）: 残したパスは前のモデルが戻るまで編集も描き直しもできず、
//! 画素だけが新しいモデルの上に出る。画素にしたあとは Undo（文書ごと）で元のパスに戻せる。知らせには件数と最初の理由
//! （レイヤー名つき）を出す。

use yolu_core::geometry::SurfaceGeometry;
use yolu_core::paths::{fingerprint, rebind_surface_path, render_list, LayerPathEntry, Options};
use yolu_core::{Document, LayerId, LayerLocks, LayerPath};

use crate::lang::Lang;

/// 1 つのパスの付け直しの結果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// 新しいモデルに置き直して描き直した。
    Redrawn,
    /// 置き直せない: 今の画素のまま、パスを外した。
    Rasterized,
    /// ロックで描き直しも画素にもできない: 前のモデルに結び付いたまま残した。
    KeptLocked,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PathReport {
    pub layer: LayerId,
    pub name: String,
    pub outcome: Outcome,
    /// 描き直せなかった理由（描き直したときは、面に投影できなかった標本の数があれば）。
    pub reason: Option<String>,
}

/// 文書の中の、`old` に結び付いた 3D のパスを `new` へ付け直す（層ごとに 1 回の Undo）。`material` は新しいモデルでの、この文書の
/// テクスチャセットのマテリアルの組（モデルに無ければ負）。指紋が同じなら何もしない。描き直せない層（画素・透明度のロックを含む）は
/// 画素にし、すべてのロックの層だけ残す。
pub fn rebind_document(
    doc: &mut Document,
    old: &SurfaceGeometry,
    new: &SurfaceGeometry,
    material: i32,
    lang: Lang,
) -> Vec<PathReport> {
    let (old_print, new_print) = (fingerprint(old), fingerprint(new));
    if old_print == new_print {
        return Vec::new();
    }
    // 一覧のパスはどれも同じモデルに結び付くので、1 本目で見る
    let targets: Vec<(LayerId, String, Vec<LayerPathEntry>)> = doc
        .layers()
        .iter()
        .filter_map(|l| match l.path() {
            Some(LayerPath::Surface(p)) if p.model_fingerprint == old_print => {
                Some((l.id(), l.name().to_owned(), l.paths().to_vec()))
            }
            _ => None,
        })
        .collect();
    let mut reports = Vec::new();
    for (layer, name, path) in targets {
        let locks = doc.effective_locks(layer).unwrap_or_default();
        let can_redraw = !locks.contains(LayerLocks::ALL)
            && !locks.contains(LayerLocks::PIXELS)
            && !locks.contains(LayerLocks::TRANSPARENCY);
        let can_rasterize = !locks.contains(LayerLocks::ALL);
        let redraw = if can_redraw {
            redraw_one(doc, layer, &path, old, new, material, lang)
        } else {
            Err(lang
                .pick("レイヤーがロックされています", "The layer is locked")
                .into())
        };
        let why = match redraw {
            Ok(note) => {
                reports.push(PathReport {
                    layer,
                    name,
                    outcome: Outcome::Redrawn,
                    reason: note,
                });
                continue;
            }
            Err(why) => why,
        };
        // 描き直せない: 画素にする（すべてのロックでは画素にもできない）
        let outcome = if can_rasterize && doc.rasterize(layer).is_ok() {
            Outcome::Rasterized
        } else {
            Outcome::KeptLocked
        };
        reports.push(PathReport {
            layer,
            name,
            outcome,
            reason: Some(why),
        });
    }
    reports
}

/// 層のパスの一覧を新しいモデルへ置き直して描き直す（1 本でも置けなければ、どれも置き直さない）。描き直したら、面に投影できなかった
/// 標本の知らせ（あれば）。できなければ理由。
fn redraw_one(
    doc: &mut Document,
    layer: LayerId,
    entries: &[LayerPathEntry],
    old: &SurfaceGeometry,
    new: &SurfaceGeometry,
    material: i32,
    lang: Lang,
) -> Result<Option<String>, String> {
    let mut rebound = Vec::with_capacity(entries.len());
    for e in entries {
        let LayerPath::Surface(path) = &e.path else {
            continue;
        };
        let moved = rebind_surface_path(path, old, new, material)
            .map_err(|e| crate::lang::rebind_error(lang, e))?;
        rebound.push(LayerPathEntry {
            path: LayerPath::Surface(moved),
            ..e.clone()
        });
    }
    let options = Options {
        width: doc.width(),
        height: doc.height(),
        tile_size: doc.tile_size(),
        source_budget_bytes: doc.source_budget_bytes(),
        stroke_budget_bytes: doc.stroke_budget_bytes(),
        images: doc.effect_inputs().images().clone(),
        ..Options::default()
    };
    let rendered = render_list(&rebound, Some(new), &options)
        .map_err(|e| crate::lang::path_error(lang, &e))?;
    let gaps = rendered.gaps;
    doc.set_paths(layer, rebound, rendered.channels)
        .map_err(|e| lang.core_error(&e))?;
    Ok((gaps > 0).then(|| {
        lang.pick(
            format!("{gaps} 個の標本は新しい面に投影できませんでした"),
            format!("{gaps} sample(s) could not be projected onto the new surface"),
        )
    }))
}
