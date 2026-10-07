//! 編集の部品の「離した直後」の表示（ランプの分岐点・カーブの点のドラッグ）。
//!
//! ドラッグは下書きに溜め、離したフレームに新しい値を呼び手へ返す。呼び手が文書へ当てるのはそのフレームの描画のあとなので、
//! 離したフレームの部品は、まだ古い値（ドラッグの前）を受け取って描いてしまい、点が一瞬元の位置へ戻って次のフレームで動かした位置へ
//! 戻ってくる。そこで、離した値を覚えておき、呼び手の値がまだ元のままの間（文書が追いつくまで）は、離した値で描き続ける。
//! 呼び手が値を別のものにした（Undo・別のレイヤー）か、`LIMIT` 秒たっても変わらない（文書が断った）ときは覚えを捨てる。

use egui::{Id, Ui};

/// 文書が追いつくのを待つ長さ（秒）。1〜2 フレームで足りるので、断られても長く嘘の表示を続けない。
const LIMIT: f64 = 0.25;

#[derive(Clone)]
struct Pending<T> {
    from: T,
    to: T,
    since: f64,
}

/// 離したときに呼ぶ。`from` は離す前に呼び手が持っていた値、`to` は離して返した値。
pub fn start<T: Clone + Send + Sync + 'static>(ui: &Ui, id: Id, from: &T, to: &T) {
    let since = ui.input(|i| i.time);
    ui.data_mut(|d| {
        d.insert_temp(
            id.with("settle"),
            Pending {
                from: from.clone(),
                to: to.clone(),
                since,
            },
        )
    });
    ui.ctx().request_repaint();
}

/// 今見せる値。離した値を覚えていて、呼び手の値 `current` がまだ離す前のままなら離した値、そうでなければ `None`（`current` を見せる）。
pub fn shown<T: Clone + PartialEq + Send + Sync + 'static>(
    ui: &Ui,
    id: Id,
    current: &T,
) -> Option<T> {
    let key = id.with("settle");
    let pending: Pending<T> = ui.data(|d| d.get_temp(key))?;
    let now = ui.input(|i| i.time);
    if pending.from == *current && now - pending.since < LIMIT {
        // 文書が追いつく次のフレームを逃さない
        ui.ctx().request_repaint();
        Some(pending.to)
    } else {
        ui.data_mut(|d| d.remove::<Pending<T>>(key));
        None
    }
}
