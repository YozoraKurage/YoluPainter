//! 重なった UV のベイクの優先（今のテクスチャセットの文書の `bake_priority`）: ウィンドウの欄からの変更（1 回の Undo）と、手でアイランドを選ぶ。
//!
//! 手で選ぶアイランドは、範囲のツールと同じ当たり（2D は UV の点の下、3D は当たった面）と同じ強調（3D は薄い面、2D は UV の輪郭）で選ぶ。アイランドの
//! 決まりはベイクと同じ `bake_islands`（UV と位置の両方でつながる三角形。ミラーで UV がぴったり重なった両側は別のアイランド）で、受けたままの
//! 形の三角形の番号（ベイクの入力の並び）で覚える（隠した面のある見せる形の番号は `full_triangle` で直す）。2D で重なった所は、同じ所を
//! 続けて押すたびに重なったアイランドを順に選び替える（前の選びを取り消して次のアイランドを入れる）。
//!
//! アイランドのメニュー（「優先して焼く」「焼かない」、ウィンドウの UV の見取り図からは「外す」も）は、ポリゴン塗りつぶしの右クリック（2D・3D）と
//! ウィンドウの見取り図の押下から開く。メニューを開いている間と、見取り図・一覧の行にポインタを置いている間は、そのアイランドを同じ強調で見せる。

use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::Instant;

use egui::Pos2;
use yolu_core::mesh_maps::{
    bake_islands, MeshBakeInput, MeshOverlapList, MeshOverlapPriority, MeshOverlapRule,
};

use super::BakeAction;
use crate::lang::Lang;
use crate::notice::Source;
use crate::region::tools::{
    canvas_triangles, cycle_applied, cycle_index, under, CycleKind, Hover, Under, Where,
};
use crate::state::{Action, AppState, OpenPopup, PopupKind, Tool};
use crate::ui::menu::{Entry, PopupState};

/// ベイクの優先の操作。
#[derive(Clone, Debug, PartialEq)]
pub enum PriorityOp {
    Rule(MeshOverlapRule),
    SkipOutside(bool),
    /// 次に押したアイランドをこの一覧へ入れる（もう選んでいる同じ一覧なら選ぶのをやめる。None でやめる）。
    Pick(Option<MeshOverlapList>),
    /// アイランド（代表の三角形）を一覧から外す。
    Remove(usize),
    /// アイランドのメニューから: テクスチャセット `set` のアイランド（代表の三角形）を一覧 `list` に入れる（もう一方からは外す）。None なら外す。
    Set {
        set: u32,
        island: usize,
        list: Option<MeshOverlapList>,
    },
}

/// 手でアイランドを選んでいる（どの一覧へ、どのセットで）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Picking {
    pub list: MeshOverlapList,
    pub set: u32,
}

/// 手で選ぶアイランドの索引（ベイクの入力ごとに 1 回）。
pub struct Islands {
    input: Arc<MeshBakeInput>,
    /// 三角形ごとのアイランドの番号。
    of: Vec<usize>,
    /// アイランドごとの三角形（受けたままの形の番号の昇順。先頭が代表）。
    members: Vec<Vec<u32>>,
}

impl Islands {
    fn new(input: Arc<MeshBakeInput>) -> Islands {
        let of = bake_islands(&input);
        let count = of.iter().max().map_or(0, |m| m + 1);
        let mut members: Vec<Vec<u32>> = vec![Vec::new(); count];
        for (t, i) in of.iter().enumerate() {
            members[*i].push(t as u32);
        }
        Islands { input, of, members }
    }

    /// この入力から作った索引か（索引は入力を握るので、入力を手放すときに一緒に手放す判断に使う）。
    pub(super) fn is_on(&self, input: &Arc<MeshBakeInput>) -> bool {
        Arc::ptr_eq(&self.input, input)
    }

    /// 三角形のアイランドの番号。
    pub fn island(&self, triangle: usize) -> Option<usize> {
        self.of.get(triangle).copied()
    }

    /// アイランドの三角形（受けたままの形の番号の昇順）。
    pub fn members(&self, island: usize) -> &[u32] {
        self.members.get(island).map_or(&[], Vec::as_slice)
    }

    /// アイランドの代表（一番小さい三角形の番号）。
    pub fn representative(&self, triangle: usize) -> Option<usize> {
        let island = self.island(triangle)?;
        self.members(island).first().map(|t| *t as usize)
    }

    /// 結び付けるモデルの指紋。
    pub fn binding(&self) -> &str {
        self.input.topology_hash()
    }

    /// 三角形ごとのアイランドの番号（受けたままの形の番号の並び）。
    pub fn of(&self) -> &[usize] {
        &self.of
    }

    /// アイランドの数。
    pub fn count(&self) -> usize {
        self.members.len()
    }
}

/// 別のスレッドで作っているアイランドの索引（入力ごと。三角形の数に比例して時間がかかるので、UI のスレッドでは作らない）。
pub(super) struct PendingIslands {
    input: Arc<MeshBakeInput>,
    rx: Receiver<Islands>,
}

impl PendingIslands {
    pub(super) fn is_on(&self, input: &Arc<MeshBakeInput>) -> bool {
        Arc::ptr_eq(&self.input, input)
    }
}

/// ポリゴン塗りつぶしの右クリックで、アイランドの索引ができていなかったので、できたら開くメニュー（押した所の三角形は、押したときの画面で
/// 決めておく。待つ間に画面が動いても、押した所のアイランドを開く）。
pub(crate) struct MenuWait {
    model: super::ModelId,
    /// 押した所の三角形（受けたままの形の番号）。
    triangles: Vec<u32>,
    /// 押した点（2D の続けて押すの照合に使う）と、メニューを開く点。
    from: Pos2,
    at: Pos2,
    /// 3D ビューの右クリックか（強調を出す側）。
    surface: bool,
}

/// 一覧の 1 行。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IslandRow {
    /// アイランドの代表（一番小さい三角形の番号）。
    pub representative: usize,
    /// メッシュの名前と、そのメッシュの中のアイランドの何番目か。
    pub label: String,
}

/// 一覧の名前（手で選ぶアイランドの一覧の見出し・知らせ）。
pub fn list_name(lang: Lang, list: MeshOverlapList) -> &'static str {
    match list {
        MeshOverlapList::Skip => lang.pick("焼かないアイランド", "Islands Not Baked"),
        MeshOverlapList::Prefer => lang.pick("優先するアイランド", "Preferred Islands"),
    }
}

/// 決め方の短い名前（ウィンドウの切り替えのボタン）。
pub fn rule_label(lang: Lang, rule: MeshOverlapRule) -> &'static str {
    match rule {
        MeshOverlapRule::LowestIndex => lang.pick("番号", "Index"),
        MeshOverlapRule::LargerArea => lang.pick("面積", "Area"),
        MeshOverlapRule::PositiveX => "+X",
        MeshOverlapRule::NegativeX => "−X",
    }
}

/// 決め方の意味（ツールチップ）。
pub fn rule_help(lang: Lang, rule: MeshOverlapRule) -> &'static str {
    match rule {
        MeshOverlapRule::LowestIndex => lang.pick(
            "重なったテクセルには、番号の小さい三角形の値を焼く",
            "An overlapping texel takes the triangle with the lower index",
        ),
        MeshOverlapRule::LargerArea => lang.pick(
            "重なったテクセルには、3D の面積の大きいアイランドの値を焼く",
            "An overlapping texel takes the island with the larger 3D area",
        ),
        MeshOverlapRule::PositiveX => lang.pick(
            "重なったテクセルには、モデルの +X の側のアイランドの値を焼く（ミラーの片側）",
            "An overlapping texel takes the island on the model's +X side (one side of a mirror)",
        ),
        MeshOverlapRule::NegativeX => lang.pick(
            "重なったテクセルには、モデルの −X の側のアイランドの値を焼く（ミラーの片側）",
            "An overlapping texel takes the island on the model's −X side (one side of a mirror)",
        ),
    }
}

impl AppState {
    /// 手で選ぶアイランドの索引。`wait` なら入力と索引を作り終えるまで待つ。待たないときは、作っている最中（入力も索引も別のスレッドで作る）なら
    /// None で、作り終えるまで毎フレーム呼ぶ。
    pub fn overlap_islands(&mut self, wait: bool) -> Option<Result<Arc<Islands>, String>> {
        let input = if wait {
            self.bake_input()
        } else {
            self.bake_input_nowait()?
        };
        let input = match input {
            Ok(i) => i,
            Err(e) => return Some(Err(e)),
        };
        if let Some(c) = self.bake.islands.as_ref().filter(|c| c.is_on(&input)) {
            return Some(Ok(c.clone()));
        }
        // 別の入力の索引は使わない（作り終えても捨てる）
        let pending = self.bake.pending_islands.take().filter(|p| p.is_on(&input));
        let built = match pending {
            Some(p) if wait => p.rx.recv().ok(),
            Some(p) => match p.rx.try_recv() {
                Ok(islands) => Some(islands),
                Err(TryRecvError::Empty) => {
                    self.bake.pending_islands = Some(p);
                    return None;
                }
                Err(TryRecvError::Disconnected) => None,
            },
            None if wait => None,
            None => {
                let (tx, rx) = channel();
                let worker_input = input.clone();
                let spawned = std::thread::Builder::new()
                    .name("yolu-bake-islands".into())
                    .spawn(move || {
                        let _ = tx.send(Islands::new(worker_input));
                    });
                if spawned.is_ok() {
                    self.bake.pending_islands = Some(PendingIslands { input, rx });
                    return None;
                }
                None
            }
        };
        // 待つ頼み・別のスレッドを作れなかった・作った側が落ちた: ここで作る
        let islands = Arc::new(built.unwrap_or_else(|| Islands::new(input)));
        self.bake.islands = Some(islands.clone());
        Some(Ok(islands))
    }

    /// 今のセットの手で選んだアイランドの一覧（`list` の、代表の昇順）。入力を作り終えていなければ名前は仮（アイランドの番号だけ）。
    pub fn overlap_island_rows(&mut self, list: MeshOverlapList) -> Vec<IslandRow> {
        let lang = self.lang;
        let priority = self.doc.bake_priority().clone();
        let chosen = match list {
            MeshOverlapList::Skip => priority.skipped(),
            MeshOverlapList::Prefer => priority.preferred(),
        };
        if chosen.is_empty() {
            return Vec::new();
        }
        let islands = self
            .overlap_islands(false)
            .and_then(Result::ok)
            .filter(|i| i.binding() == priority.binding());
        let model = self.view3d.full_model().cloned();
        // メッシュごとの三角形の始まり（ベイクの入力の並び）
        let mut starts = Vec::new();
        if let Some(m) = &model {
            let mut at = 0usize;
            for mesh in &m.meshes {
                starts.push((at, mesh.name.clone()));
                at += mesh.triangle_count();
            }
        }
        chosen
            .iter()
            .map(|&t| {
                let mesh = starts.iter().rposition(|(s, _)| *s <= t);
                let label = match (&islands, mesh) {
                    (Some(islands), Some(m)) => {
                        let (start, name) = &starts[m];
                        let end = starts.get(m + 1).map_or(usize::MAX, |(s, _)| *s);
                        // そのメッシュの中で、代表が t 以下のアイランドの数（1 から）
                        let n = islands
                            .members
                            .iter()
                            .filter_map(|m| m.first().map(|r| *r as usize))
                            .filter(|r| *r >= *start && *r < end && *r <= t)
                            .count();
                        format!("{name} · {} {n}", lang.pick("アイランド", "Island"))
                    }
                    _ => format!("{} {}", lang.pick("アイランド", "Island"), t),
                };
                IslandRow {
                    representative: t,
                    label,
                }
            })
            .collect()
    }

    /// 手で選んだアイランドの一覧が今のモデルのものでないか（一覧があり、入力を作り終えていて、指紋が違う）。
    pub fn overlap_islands_foreign(&mut self) -> bool {
        let priority = self.doc.bake_priority();
        if priority.skipped().is_empty() && priority.preferred().is_empty() {
            return false;
        }
        let binding = priority.binding().to_owned();
        match self.overlap_islands(false) {
            Some(Ok(i)) => i.binding() != binding,
            _ => false,
        }
    }

    /// ベイクの優先の操作を当てる。
    pub fn bake_priority_apply(&mut self, op: PriorityOp) {
        let lang = self.lang;
        if self.is_stroking() {
            self.refuse(Source::Bake, crate::lang::refusals::during_stroke(lang));
            return;
        }
        let current = self.doc.bake_priority().clone();
        let next = match op {
            PriorityOp::Pick(list) => {
                let set = self.sets.current().uid;
                self.bake.pick = match (list, self.bake.pick) {
                    (Some(l), Some(p)) if p.list == l && p.set == set => None,
                    (Some(l), _) => Some(Picking { list: l, set }),
                    (None, _) => None,
                };
                self.region.hover = None;
                self.region.cycle = None;
                return;
            }
            PriorityOp::Rule(rule) => {
                let mut p = current.clone();
                p.rule = rule;
                Ok(p)
            }
            PriorityOp::SkipOutside(on) => {
                let mut p = current.clone();
                p.skip_outside = on;
                Ok(p)
            }
            PriorityOp::Remove(island) => current.with_island("", island, None),
            PriorityOp::Set { set, island, list } => {
                // メニューを開いた後にセットを替えた: 今のセットのアイランドではない
                if set != self.sets.current().uid {
                    return;
                }
                self.set_island(island, list);
                return;
            }
        };
        let next = match next {
            Ok(p) => p,
            Err(e) => {
                self.fail(Source::Bake, lang.mesh_map_error(&e));
                return;
            }
        };
        self.set_bake_priority(next, None);
    }

    /// アイランド（代表の三角形）を一覧 `list` に入れる（もう一方からは外す）か、None なら外す。変えたら true。
    fn set_island(&mut self, island: usize, list: Option<MeshOverlapList>) -> bool {
        let lang = self.lang;
        let current = self.doc.bake_priority().clone();
        let was = if current.preferred().contains(&island) {
            Some(MeshOverlapList::Prefer)
        } else if current.skipped().contains(&island) {
            Some(MeshOverlapList::Skip)
        } else {
            None
        };
        if was == list {
            return false;
        }
        let next = match list {
            None => current.with_island("", island, None),
            Some(l) => {
                let islands = match self.overlap_islands(true) {
                    Some(Ok(i)) => i,
                    Some(Err(e)) => {
                        self.refuse(Source::Bake, e);
                        return false;
                    }
                    None => return false,
                };
                // 開いた後にモデルが替わった（番号が今のアイランドの代表でない）
                if islands.representative(island) != Some(island) {
                    return false;
                }
                current.with_island(islands.binding(), island, Some(l))
            }
        };
        let next = match next {
            Ok(p) => p,
            Err(e) => {
                self.refuse(Source::Bake, lang.mesh_map_error(&e));
                return false;
            }
        };
        let done = match (list, was) {
            (Some(l), _) => {
                let name = lang.quote(list_name(lang, l));
                lang.pick(
                    format!("アイランドを{name}に追加しました。"),
                    format!("Island added to {name}."),
                )
            }
            (None, Some(w)) => {
                let name = lang.quote(list_name(lang, w));
                lang.pick(
                    format!("アイランドを{name}から外しました。"),
                    format!("Island removed from {name}."),
                )
            }
            (None, None) => return false,
        };
        self.set_bake_priority(next, Some(done))
    }

    /// 文書のベイクの優先を変える（読むだけのセット・描いている間は断る）。変えたら true。
    fn set_bake_priority(&mut self, next: MeshOverlapPriority, done: Option<String>) -> bool {
        let lang = self.lang;
        if let Some(reason) = self.read_only_reason() {
            let text = crate::lang::refusals::read_only_set(lang, reason);
            self.refuse(Source::Bake, text);
            return false;
        }
        if *self.doc.bake_priority() == next {
            return false;
        }
        match self.doc.set_bake_priority(next) {
            Ok(()) => {
                self.modified = true;
                self.info(
                    Source::Bake,
                    done.unwrap_or_else(|| {
                        lang.pick(
                            "重なった UV の決め方を変えました。",
                            "Overlapping UV priority changed.",
                        )
                        .to_owned()
                    }),
                );
                true
            }
            Err(e) => {
                self.notify(
                    crate::notice::Kind::of_core(&e),
                    Source::Bake,
                    lang.core_error(&e),
                );
                false
            }
        }
    }
}

/// 押した・ポインタを置いた所の三角形（受けたままの形の番号。隠した面は出さない）。2D で重なった所は 2 つ以上（見せる形の番号の昇順）。
/// アイランドの索引は要らない（右クリックで索引を作っている間も、押した所はこの場で決められる）。
fn triangles_at(app: &mut AppState, w: Where, at: Pos2) -> Vec<u32> {
    let shown: Vec<u32> = match w {
        Where::Canvas(view) => canvas_triangles(app, view, at),
        Where::Surface(_) => match under(app, w, at) {
            Under::Triangle(t) => vec![t],
            _ => Vec::new(),
        },
    };
    shown
        .into_iter()
        .filter_map(|t| app.view3d.full_triangle(t))
        .collect()
}

/// 三角形のアイランドの候補（受けたままの形の番号の代表・ベイクのアイランドの番号。同じアイランドは 1 つ）。
fn islands_of(islands: &Islands, triangles: &[u32]) -> Vec<(u64, usize)> {
    let mut out: Vec<(u64, usize)> = Vec::new();
    for &full in triangles {
        let (Some(island), Some(rep)) = (
            islands.island(full as usize),
            islands.representative(full as usize),
        ) else {
            continue;
        };
        if !out.iter().any(|(k, _)| *k == island as u64) {
            out.push((island as u64, rep));
        }
    }
    out
}

/// 手で選んでいるときの、押した所のアイランドの候補（受けたままの形の番号の代表・ベイクのアイランドの番号）。2D で重なった所は 2 つ以上（番号の小さい順）。
fn candidates(app: &mut AppState, islands: &Islands, w: Where, at: Pos2) -> Vec<(u64, usize)> {
    let triangles = triangles_at(app, w, at);
    islands_of(islands, &triangles)
}

/// 手で選んでいるか（今のセットで）。違うセットに移っていたらやめる（ウィンドウを閉じたときは `CloseWindow` がやめる）。
pub fn picking(app: &mut AppState) -> Option<MeshOverlapList> {
    let p = app.bake.pick?;
    if p.set != app.sets.current().uid {
        app.bake.pick = None;
        return None;
    }
    Some(p.list)
}

/// 手で選んでいるときの押下: 押した所のアイランドを一覧へ入れる（もう入っていれば外す）。2D で重なった所を続けて押したら、前の選びを
/// 取り消して次のアイランドを入れる。押下を受けたら true（ツールの押下にしない）。
pub fn press(app: &mut AppState, w: Where, at: Pos2) -> bool {
    let Some(list) = picking(app) else {
        return false;
    };
    let lang = app.lang;
    if let Some(reason) = app.read_only_reason() {
        let text = crate::lang::refusals::read_only_set(lang, reason);
        app.refuse(Source::Bake, text);
        return true;
    }
    if app.region_model().is_none() {
        app.refuse(Source::Bake, app.region_missing_reason());
        return true;
    }
    if let Under::OtherSet(name) = under(app, w, at) {
        app.refuse(
            Source::Bake,
            crate::region::tools::other_set_face(lang, &name),
        );
        return true;
    }
    let islands = match app.overlap_islands(true) {
        Some(Ok(i)) => i,
        Some(Err(e)) => {
            app.refuse(Source::Bake, e);
            return true;
        }
        None => return true,
    };
    let found = candidates(app, &islands, w, at);
    if found.is_empty() {
        app.refuse(
            Source::Bake,
            lang.pick(
                "ポインタの下にこのテクスチャセットの三角形がありません。",
                "No triangle of this texture set under the pointer.",
            ),
        );
        return true;
    }
    let keys: Vec<u64> = found.iter().map(|(k, _)| *k).collect();
    let (index, undo) = match w {
        Where::Canvas(_) => cycle_index(app, CycleKind::BakeIsland, at, &keys, true),
        Where::Surface(_) => {
            app.region.cycle = None;
            (0, false)
        }
    };
    if undo {
        match app.doc.undo() {
            Ok(_) => app.modified = true,
            Err(e) => {
                app.region.cycle = None;
                app.notify(
                    crate::notice::Kind::of_core(&e),
                    Source::Bake,
                    lang.core_error(&e),
                );
                return true;
            }
        }
    }
    let island = found[index].1;
    let current = app.doc.bake_priority();
    let has = match list {
        MeshOverlapList::Skip => current.skipped().contains(&island),
        MeshOverlapList::Prefer => current.preferred().contains(&island),
    };
    let changed = app.set_island(island, (!has).then_some(list));
    if matches!(w, Where::Canvas(_)) {
        cycle_applied(app, CycleKind::BakeIsland, changed);
    }
    true
}

/// 強調をアイランドに固定している理由（アイランド・強調を出す側が 3D か）: アイランドのメニューを開いている（右クリックで開いた側。ウィンドウの見取り図からなら
/// 下と同じ）、ウィンドウの見取り図か一覧の行にポインタを置いている（3D ビューが見えていれば 3D、無ければ 2D）。
fn pinned(app: &mut AppState) -> Option<(usize, bool)> {
    let shown_side = app.view3d.visible;
    if let Some(PopupKind::BakeIsland {
        set,
        island,
        map,
        surface,
    }) = app.popup.as_ref().map(|p| p.kind)
    {
        if set == app.sets.current().uid {
            return Some((island, if map { shown_side } else { surface }));
        }
    }
    app.bake.map_hover.map(|i| (i, shown_side))
}

/// アイランドを強調しているか（手で選んでいる・アイランドのメニューを開いている・ウィンドウの見取り図でアイランドを指している）。ツールが範囲を出さなくても強調を出す。
pub fn highlighting(app: &mut AppState) -> bool {
    pinned(app).is_some() || picking(app).is_some()
}

/// 手で選んでいるとき・アイランドのメニューを開いているとき・ウィンドウの見取り図でアイランドを指しているときの強調: そのアイランド（手で選んでいるときは
/// ポインタの下のアイランド。2D で重なった所は、続けて押している候補）を範囲のツールの強調と同じ形で出す。どれでもなければ false
/// （範囲のツールの強調に任せる）。
pub fn update_hover(app: &mut AppState, w: Where, at: Option<Pos2>) -> bool {
    if let Some((representative, surface)) = pinned(app) {
        // 強調を出す側だけが作り直す（もう一方の側は、出している強調に触らない）
        if surface == w.is_surface() {
            show_island(app, w, representative);
        }
        return true;
    }
    if picking(app).is_none() {
        return false;
    }
    let Some(at) = at else {
        if app
            .region
            .hover
            .as_ref()
            .is_some_and(|h| h.on_surface == w.is_surface())
        {
            app.region.hover = None;
        }
        return true;
    };
    let Some(Ok(islands)) = app.overlap_islands(false) else {
        app.region.hover = None;
        return true;
    };
    let found = candidates(app, &islands, w, at);
    let keys: Vec<u64> = found.iter().map(|(k, _)| *k).collect();
    let index = match w {
        Where::Canvas(_) => cycle_index(app, CycleKind::BakeIsland, at, &keys, false).0,
        Where::Surface(_) => 0,
    };
    match found.get(index) {
        Some((_, representative)) => show_island(app, w, *representative),
        None => app.region.hover = None,
    }
    true
}

/// アイランド（代表の三角形）を `w` の側の強調にする（同じアイランドの強調が出ていれば作り直さない）。
fn show_island(app: &mut AppState, w: Where, representative: usize) {
    let Some((model, _)) = app.region_model() else {
        app.region.hover = None;
        return;
    };
    let Some(Ok(islands)) = app.overlap_islands(false) else {
        app.region.hover = None;
        return;
    };
    let Some(island) = islands.island(representative) else {
        app.region.hover = None;
        return;
    };
    // 範囲のツールの鍵（1〜4 << 60）と重ならない鍵
    let key = (5u64 << 60) | island as u64;
    if app.region.hover.as_ref().is_some_and(|h| {
        h.id_key.is_none()
            && h.key == key
            && h.on_surface == w.is_surface()
            && Arc::ptr_eq(&h.geometry, &model.geometry)
    }) {
        return;
    }
    // アイランドの三角形を見せる形の番号に直す（隠した面は出さない）
    let total = model.triangle_count();
    let tris: Vec<u32> = (0..total as u32)
        .filter(|&t| {
            app.view3d
                .full_triangle(t)
                .and_then(|f| islands.island(f as usize))
                == Some(island)
        })
        .collect();
    let Some(&first) = tris.first() else {
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
        key,
        tris: Arc::new(tris),
        outline: Arc::new(outline),
        geometry: model.geometry.clone(),
        erase: false,
        id_key: None,
        visible: None,
    });
}

// ───────── アイランドのメニュー ─────────

/// アイランドのメニューの項目: 「優先して焼く」「焼かない」（今の状態にチェック。チェックのある項目を選ぶと外す）、ウィンドウの見取り図からは
/// 「外す」も。読むだけのセット・描いている間・開いた後にセットを替えたときは選べない。
pub fn menu_entries(app: &AppState, set: u32, island: usize, map: bool) -> Vec<Entry<Action>> {
    let lang = app.lang;
    let priority = app.doc.bake_priority();
    let preferred = priority.preferred().contains(&island);
    let skipped = priority.skipped().contains(&island);
    let enabled =
        set == app.sets.current().uid && !app.is_stroking() && app.read_only_reason().is_none();
    let op = |list: Option<MeshOverlapList>| {
        Action::Bake(BakeAction::Priority(PriorityOp::Set { set, island, list }))
    };
    let mut out = vec![
        Entry::item(
            if map {
                lang.pick("優先する", "Prefer")
            } else {
                lang.pick("優先して焼く", "Prefer in Bake")
            },
            op((!preferred).then_some(MeshOverlapList::Prefer)),
        )
        .checked(preferred)
        .enabled(enabled)
        .tooltip(lang.pick(
            "重なったテクセルに、このアイランドの値を焼く",
            "Overlapping texels take this island's values",
        )),
        Entry::item(
            if map {
                lang.pick("焼かない", "Skip")
            } else {
                lang.pick("焼かない", "Skip in Bake")
            },
            op((!skipped).then_some(MeshOverlapList::Skip)),
        )
        .checked(skipped)
        .enabled(enabled)
        .tooltip(lang.pick(
            "このアイランドをベイクの割り当てから外す",
            "Leaves this island out of the bake",
        )),
    ];
    if map {
        out.push(Entry::Separator);
        out.push(
            Entry::item(lang.pick("外す", "Remove"), op(None))
                .enabled(enabled && (preferred || skipped)),
        );
    }
    out
}

/// アイランドのメニューを、押した点の上下にこれだけ空けて開く（重なった所を続けて押す点を、開いたメニューで覆わない。続けて押したと
/// みなす距離 4 px より広く）。
const MENU_CLEARANCE: f32 = 6.0;

/// アイランドのメニューを開く（`at` はポインタ。`map` はウィンドウの見取り図から、`surface` は 3D ビューの右クリックから）。
pub fn open_menu(
    app: &mut AppState,
    ctx: &egui::Context,
    island: usize,
    at: Pos2,
    map: bool,
    surface: bool,
) {
    app.popup = Some(OpenPopup {
        kind: PopupKind::BakeIsland {
            set: app.sets.current().uid,
            island,
            map,
            surface,
        },
        state: PopupState::new(
            ctx,
            egui::Rect::from_center_size(at, egui::vec2(0.0, MENU_CLEARANCE * 2.0)),
        ),
    });
}

/// ポリゴン塗りつぶしの右ボタンを押した（2D・3D）。離したときに近ければメニューを開く。
pub fn menu_press(app: &mut AppState, w: Where, at: Pos2) {
    app.bake.menu_wait = None;
    app.bake.menu_press =
        (app.right_opens_island_menu() && !app.is_stroking() && app.region.drag.is_none())
            .then_some((at, w.is_surface(), Instant::now()));
    if app.bake.menu_press.is_some() {
        // 離すまでのあいだに、入力とアイランドの索引を別のスレッドで作り始める（離したときに待たずに済むことが多い）。
        // 作ってあれば何もしない
        let _ = app.overlap_islands(false);
    }
}

/// 右ボタンを離した: 押した所の近くで離したら、ポインタの下のアイランドのメニューを開く（2D で重なった所は、続けて右クリックするたびに
/// 次のアイランド）。押した所の面がほかのテクスチャセットなら断る。下に何も無ければ何もしない。
pub fn menu_release(app: &mut AppState, ctx: &egui::Context, w: Where, at: Pos2) {
    /// 押した所と同じとみなす距離（3D の右ドラッグは回転なので、動かさずに離したときだけ）。
    const CLICK: f32 = 4.0;
    let Some((from, surface, _)) = app.bake.menu_press.take() else {
        return;
    };
    if surface != w.is_surface()
        || from.distance(at) > CLICK
        || app.tool != Tool::PolygonFill
        || app.region_model().is_none()
    {
        return;
    }
    if let Under::OtherSet(name) = under(app, w, from) {
        let text = crate::region::tools::other_set_face(app.lang, &name);
        app.refuse(Source::Fill, text);
        return;
    }
    let triangles = triangles_at(app, w, from);
    if triangles.is_empty() {
        // 下に何も無い（2D は続けて押すの控えも切る）
        if !surface {
            let _ = cycle_index(app, CycleKind::Menu, from, &[], true);
        }
        return;
    }
    let Some(model) = app.view3d.full_model().map(super::ModelId::of) else {
        return;
    };
    let wait = MenuWait {
        model,
        triangles,
        from,
        at,
        surface,
    };
    // アイランドの索引は UI のスレッドでは作らない（入力と索引の両方が、三角形の数に比例して時間がかかる）。ができていなければ作りながら
    // 待ち、できたフレームで開く（`poll_menu`）
    match app.overlap_islands(false) {
        Some(Ok(islands)) => open_island_menu(app, ctx, &islands, wait),
        Some(Err(e)) => app.refuse(Source::Fill, e),
        None => {
            app.bake.menu_wait = Some(wait);
            ctx.request_repaint_after(WAIT_REPAINT);
        }
    }
}

/// 右クリックを押してから離すまで、アイランドの索引の準備を続ける長さ（押したまま離さなかったとき、入力を持ち続けない）。
const PRESS_HOLD: std::time::Duration = std::time::Duration::from_secs(10);

/// アイランドの索引を待つ間の描き直しの間隔。
const WAIT_REPAINT: std::time::Duration = std::time::Duration::from_millis(16);

/// 右クリックのアイランドのメニューを、アイランドの索引ができるのを待っている（試験・描き直しの判断用）。
pub fn menu_pending(app: &AppState) -> bool {
    app.bake.menu_wait.is_some()
}

/// 毎フレーム: 右クリックで待たせたメニューを、アイランドの索引ができていれば開く。ツールを替えた・描き始めた・ほかのポップアップが開いた・
/// モデルが替わったときは、開かずにやめる。
pub fn poll_menu(ctx: &egui::Context, app: &mut AppState) {
    // 右クリックを押している間に、入力とアイランドの索引を作り進める（離したときにできていれば、すぐ開く）。押したまま離さなかった・
    // 離す前にツールを替えたときは、少しで手放す
    if let Some((_, _, since)) = app.bake.menu_press {
        if app.tool != Tool::PolygonFill || since.elapsed() > PRESS_HOLD {
            app.bake.menu_press = None;
        } else {
            if app.overlap_islands(false).is_none() {
                ctx.request_repaint_after(WAIT_REPAINT);
            }
            return;
        }
    }
    let Some(wait) = app.bake.menu_wait.take() else {
        return;
    };
    let same_model = app.view3d.full_model().is_some_and(|m| wait.model.is(m));
    if app.tool != Tool::PolygonFill || app.is_stroking() || app.popup.is_some() || !same_model {
        return;
    }
    match app.overlap_islands(false) {
        Some(Ok(islands)) => open_island_menu(app, ctx, &islands, wait),
        Some(Err(e)) => app.refuse(Source::Fill, e),
        None => {
            app.bake.menu_wait = Some(wait);
            ctx.request_repaint_after(WAIT_REPAINT);
        }
    }
}

/// 押した所のアイランドのメニューを開く（2D で重なった所は、続けて右クリックするたびに次のアイランド）。
fn open_island_menu(app: &mut AppState, ctx: &egui::Context, islands: &Islands, wait: MenuWait) {
    let found = islands_of(islands, &wait.triangles);
    let keys: Vec<u64> = found.iter().map(|(k, _)| *k).collect();
    // 2D（3D は当たった面のアイランドだけ）
    let index = if wait.surface {
        0
    } else {
        cycle_index(app, CycleKind::Menu, wait.from, &keys, true).0
    };
    let Some((_, island)) = found.get(index).copied() else {
        return;
    };
    open_menu(app, ctx, island, wait.at, false, wait.surface);
}
