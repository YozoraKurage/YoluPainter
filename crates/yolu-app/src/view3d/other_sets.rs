//! 3D ビューに、今のセットでないセットの絵も見せるための口。
//!
//! 3D ビューは全部のテクスチャセットの絵を見せる（Substance Painter と同じ）。今のセットは今までどおり（変わったタイルだけ上げる）、
//! ほかのセットは縮めた段で GPU に持ち、そのセットの文書が変わったときだけ上げ直す（`render::View3dRenderer::prepare_sets`）。
//! ここは、アプリの状態から「見せるほかのセット」を集める所と、描いたあとの結果（絵を見せられなかったセット）を状態へ返す所。
//! 隠したセット（目を閉じたもの）はモデルの面ごと見せる形から除かれているので、ここでも渡さない。

use yolu_core::Document;

use crate::state::AppState;

/// 3D に見せる、今のセットでないセット 1 つ。
pub struct OtherSet<'a> {
    /// このセットが受け持つモデルのマテリアルの番号。
    pub material: i32,
    /// セットの文書（しまってあるもの）。
    pub doc: &'a Document,
    /// 持つ優先（小さいほど先。今のセットから並びの上で近い順）。メモリの予算が足りないとき、大きい方から持たない。
    pub rank: u32,
}

/// 見せるほかのセット（マテリアルに付いていて、目を開いている、今のセットでないもの）。文書の ID が今のセットや先のセットと重なるものは
/// 渡さない（絵の持ち物を文書の ID で見分けるので、同じ ID の 2 つの文書が 1 つの持ち物を取り合わないように）。
pub fn collect(app: &AppState) -> Vec<OtherSet<'_>> {
    let current = app.sets.current_index();
    let mut seen = vec![app.doc.id()];
    app.sets
        .iter()
        .enumerate()
        .filter_map(|(i, set)| {
            if i == current || !set.visible {
                return None;
            }
            let doc = app.sets.stashed_doc(i)?;
            if seen.contains(&doc.id()) {
                return None;
            }
            seen.push(doc.id());
            Some(OtherSet {
                material: set.bound? as i32,
                doc,
                rank: i.abs_diff(current) as u32,
            })
        })
        .collect()
}

/// 描いたあとの結果を状態へ返す（予算が足りずに絵を見せていないセットのマテリアル。テクスチャセットの一覧が印とツールチップで出す）。
pub fn finish(app: &mut AppState, unpainted: &[i32]) {
    if app.view3d.unpainted != unpainted {
        app.view3d.unpainted = unpainted.to_vec();
    }
}
