//! プロジェクトの構成: 下書きの操作・計画（検査と、適用の前の確かめの一覧）・適用。
//!
//! 適用は、失敗しうる所（足すセットの文書・大きさの変更の準備）を先に全部やり、そのあと失敗しない入れ替え（モデル・セットの削除と
//! 並び・名前・マテリアル・法線の形式）をする。大きさの変更は core の `prepare_resize_image` で全セットの結果を先に作り（文書も履歴も
//! 変えない）、1 つでも予算で断られたら何も変えずに返す。全部が作れたら `commit_prepared_resize`（1 回の Undo の段）で入れる。
//! 変えたセットは履歴を消す（元へ戻せない）。

use super::{
    groups_of, new_set_document, size_text, unique, used_channels, DraftOp, NpWindow, Prep,
    SetDraft, MAX_NAME, MAX_SETS, RESOLUTIONS,
};
use crate::engine::{CanvasResampling, Document, PreparedResize};
use crate::lang::Lang;
use crate::model::SceneModel;
use crate::sets::{guid_string, MaterialRef};
use crate::state::AppState;
use crate::view3d::pose;

/// 確かめる理由（一覧の窓の見出しに使う）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfirmKind {
    /// セットを消す（その作業が消える）。
    Removal,
    /// 大きさを変える（再標本化。履歴が消える）。
    Resize,
    /// モデルを替える・読み直す。
    Model,
}

/// 確かめの一覧の 1 行。
#[derive(Clone, Debug, PartialEq)]
pub struct PlanRow {
    pub left: String,
    pub middle: String,
    pub right: String,
    pub warning: bool,
}

/// 適用する前に検めた計画。
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    /// 消すセット（uid）。
    pub removed: Vec<u32>,
    /// 大きさを変えるセット（uid・新しい大きさ・再標本化）。
    pub resizes: Vec<(u32, (u32, u32), CanvasResampling)>,
    /// モデルを替える・読み直す（新しいモデルの名前）。
    pub model: Option<String>,
    pub kinds: Vec<ConfirmKind>,
    pub rows: Vec<PlanRow>,
}

impl Plan {
    /// 適用の前に一覧で確かめるか（消す・大きさを変える・モデルを替えるのどれか）。
    pub fn needs_confirm(&self) -> bool {
        !self.kinds.is_empty()
    }
}

/// 再標本化の名前。
pub fn resampling_name(lang: Lang, method: Option<CanvasResampling>) -> &'static str {
    match method {
        Some(CanvasResampling::Bilinear) => lang.pick("バイリニア", "Bilinear"),
        Some(CanvasResampling::Area) => lang.pick("面積平均", "Area average"),
        Some(CanvasResampling::Nearest) => lang.pick("ニアレストネイバー", "Nearest"),
        None => lang.pick("自動", "Automatic"),
    }
}

/// 選んだ再標本化（None は自動: 面積が減るなら面積平均、ほかはバイリニア）。
fn method_for(
    chosen: Option<CanvasResampling>,
    from: (u32, u32),
    to: (u32, u32),
) -> CanvasResampling {
    chosen.unwrap_or(
        if (to.0 as u64 * to.1 as u64) < (from.0 as u64 * from.1 as u64) {
            CanvasResampling::Area
        } else {
            CanvasResampling::Bilinear
        },
    )
}

/// セットの名前の決まり（1〜256 文字・空白だけでない・制御文字なし）。決まりに合えば、前後の空白を除いた名前。
fn clean_name(name: &str) -> Option<String> {
    let name = name.trim();
    (!name.is_empty()
        && name.encode_utf16().count() <= MAX_NAME
        && !name.chars().any(|c| c.is_control()))
    .then(|| name.to_owned())
}

// ───────── 下書きの操作 ─────────

/// 下書き `i` を直す。
pub(super) fn draft_op(app: &AppState, win: &mut NpWindow, i: usize, op: DraftOp) {
    let groups = win.groups(app);
    win.error = None;
    match op {
        DraftOp::Remove => {
            if win.drafts.len() > 1 && i < win.drafts.len() {
                win.drafts.remove(i);
            }
        }
        DraftOp::Name(name) => {
            if let Some(d) = win.drafts.get_mut(i) {
                if !d.read_only {
                    d.name = name;
                }
            }
        }
        DraftOp::Material(group) => {
            let taken = group.is_some_and(|g| {
                win.drafts
                    .iter()
                    .enumerate()
                    .any(|(j, d)| j != i && d.material == Some(g))
            });
            if taken {
                return;
            }
            if let Some(d) = win.drafts.get_mut(i) {
                match group.and_then(|g| groups.get(g)) {
                    Some(g) => {
                        d.material = Some(g.index);
                        d.key = g.key.clone();
                    }
                    None => d.material = None,
                }
            }
        }
        DraftOp::Size(w, h) => {
            if let Some(d) = win.drafts.get_mut(i) {
                let standard = w == h && RESOLUTIONS.contains(&w);
                if !d.read_only && (standard || (w, h) == d.current) {
                    d.size = (w, h);
                }
            }
        }
    }
}

/// 足すセットの下書きを作る（`group` がマテリアルの組、None ならどの組にも付けない）。
fn new_draft(app: &AppState, win: &NpWindow, group: Option<&super::Group>) -> SetDraft {
    let lang = app.lang;
    let base = match group {
        Some(g) => g.name.clone(),
        None => format!(
            "{} {}",
            lang.pick("テクスチャセット", "Texture Set"),
            win.drafts.len() + 1
        ),
    };
    let name = unique(&base, win.drafts.iter().map(|d| d.name.as_str()));
    let key = match group {
        Some(g) => g.key.clone(),
        None => {
            let also: Vec<&MaterialRef> = win.drafts.iter().map(|d| &d.key).collect();
            MaterialRef::PendingSlot(app.unbound_pending_slot(&also))
        }
    };
    SetDraft {
        uid: None,
        name,
        material: group.map(|g| g.index),
        key,
        size: (win.resolution, win.resolution),
        current: (0, 0),
        read_only: false,
    }
}

/// 空のセットを足す: 下書きにまだ無いマテリアルの組があれば最初のそれに付け、無ければどの組にも付けない。
pub(super) fn add_draft(app: &AppState, win: &mut NpWindow) {
    win.error = None;
    if win.drafts.len() >= MAX_SETS {
        win.error = Some(crate::lang::refusals::set_limit(app.lang));
        return;
    }
    let groups = win.groups(app);
    let free = groups
        .iter()
        .find(|g| win.drafts.iter().all(|d| d.material != Some(g.index)));
    let draft = new_draft(app, win, free);
    win.drafts.push(draft);
}

/// セットの無いマテリアルの組に、空のセットを 1 つずつ足す（上限まで。上限で足せないものがあれば理由を窓に出す）。
pub(super) fn add_unused(app: &AppState, win: &mut NpWindow) {
    win.error = None;
    let groups = win.groups(app);
    for g in &groups {
        if win.drafts.iter().all(|d| d.material != Some(g.index)) {
            if win.drafts.len() >= MAX_SETS {
                // 上限で足せなかったマテリアルがある
                win.error = Some(crate::lang::refusals::set_limit(app.lang));
                break;
            }
            let draft = new_draft(app, win, Some(g));
            win.drafts.push(draft);
        }
    }
}

/// セットの無いマテリアルの組の数（窓の状態に出す）。
pub fn unused_groups(app: &AppState, win: &NpWindow) -> usize {
    win.groups(app)
        .iter()
        .filter(|g| win.drafts.iter().all(|d| d.material != Some(g.index)))
        .count()
}

// ───────── 計画 ─────────

/// 下書きを検めて、適用の計画（消す・大きさ・モデル）と確かめの一覧を作る。断るときは理由。何も変えない。
pub(super) fn plan(app: &AppState, win: &NpWindow) -> Result<Plan, String> {
    let lang = app.lang;
    if app.is_stroking() {
        return Err(crate::lang::refusals::during_stroke(lang).into());
    }
    let drafts = &win.drafts;
    if drafts.is_empty() {
        return Err(lang
            .pick(
                "プロジェクトには少なくとも 1 つのテクスチャセットが要ります",
                "A project keeps at least one texture set",
            )
            .into());
    }
    if drafts.len() > MAX_SETS {
        return Err(crate::lang::refusals::set_limit(lang));
    }
    if win.model.is_some() && !matches!(win.prep, Prep::Ready { .. }) {
        return Err(lang
            .pick("モデルを準備できていません", "The model is not ready")
            .into());
    }
    let mut seen_names: Vec<String> = Vec::new();
    for d in drafts {
        let Some(name) = clean_name(&d.name) else {
            return Err(lang.pick(
                format!("テクスチャセットの名前は 1〜{MAX_NAME} 文字で、制御文字は使えません"),
                format!("A texture set name is 1–{MAX_NAME} characters with no control characters"),
            ));
        };
        let upper = name.to_uppercase();
        if seen_names.contains(&upper) {
            return Err(lang.pick(
                format!("「{name}」という名前のセットが 2 つあります"),
                format!("Two texture sets are named {name}"),
            ));
        }
        seen_names.push(upper);
        if let Some(uid) = d.uid {
            if app.sets.index_of(uid).is_none() {
                return Err(lang
                    .pick(
                        "このプロジェクトに無いセットです",
                        "The project has no such texture set",
                    )
                    .into());
            }
        }
        let standard = d.size.0 == d.size.1 && RESOLUTIONS.contains(&d.size.0);
        if d.uid.is_none() && !standard {
            return Err(crate::lang::refusals::set_size(lang));
        }
        if d.resizes() {
            if d.read_only {
                return Err(lang.pick(
                    format!("読むだけのセット「{}」は大きさを変えられません", d.name),
                    format!("The read-only texture set “{}” cannot be resized", d.name),
                ));
            }
            if !standard {
                return Err(crate::lang::refusals::set_size(lang));
            }
        }
    }
    // 同じマテリアルを 2 つのセットが描かない
    for (i, a) in drafts.iter().enumerate() {
        if let Some(group) = a.material {
            if let Some(b) = drafts[i + 1..].iter().find(|b| b.material == Some(group)) {
                return Err(lang.pick(
                    format!(
                        "「{}」と「{}」が同じマテリアルを描きます",
                        a.name.trim(),
                        b.name.trim()
                    ),
                    format!(
                        "“{}” and “{}” paint the same material",
                        a.name.trim(),
                        b.name.trim()
                    ),
                ));
            }
        }
    }
    let removed: Vec<u32> = app
        .sets
        .iter()
        .map(|s| s.uid)
        .filter(|uid| drafts.iter().all(|d| d.uid != Some(*uid)))
        .collect();
    let mut resizes: Vec<(u32, (u32, u32), CanvasResampling)> = drafts
        .iter()
        .filter(|d| d.resizes())
        .map(|d| {
            (
                d.uid.expect("大きさを変えるのはあるセット"),
                d.size,
                method_for(win.resampling, d.current, d.size),
            )
        })
        .collect();
    // 縮めるセットから先に（広げるセットに予算を回す）
    resizes.sort_by(|a, b| {
        let ratio = |(uid, to): (u32, (u32, u32))| {
            let from = drafts
                .iter()
                .find(|d| d.uid == Some(uid))
                .map_or((1, 1), |d| d.current);
            (to.0 as f64 * to.1 as f64) / (from.0 as f64 * from.1 as f64)
        };
        ratio((a.0, a.1)).total_cmp(&ratio((b.0, b.1)))
    });
    let model = match &win.prep {
        Prep::Ready { model, .. } if win.model.is_some() => Some(model.rig().name().to_owned()),
        _ => None,
    };
    let mut kinds = Vec::new();
    let mut rows = Vec::new();
    if !removed.is_empty() {
        kinds.push(ConfirmKind::Removal);
        for uid in &removed {
            let name = app
                .sets
                .by_uid(*uid)
                .map(|s| s.name.clone())
                .unwrap_or_default();
            rows.push(PlanRow {
                left: name,
                middle: String::new(),
                right: lang.pick("消す", "Remove").into(),
                warning: true,
            });
        }
    }
    if !resizes.is_empty() {
        kinds.push(ConfirmKind::Resize);
        for (uid, to, method) in &resizes {
            let d = drafts
                .iter()
                .find(|d| d.uid == Some(*uid))
                .expect("下書きにある");
            rows.push(PlanRow {
                left: d.name.trim().to_owned(),
                middle: format!("{} → {}", size_text(d.current), size_text(*to)),
                right: resampling_name(lang, Some(*method)).into(),
                warning: false,
            });
        }
    }
    if let Some(name) = &model {
        kinds.push(ConfirmKind::Model);
        // 前のモデルが無いときは、前の名前を出さず新しいモデルの名前だけ（「追加」）
        let was = app.model.as_ref().map(|m| m.name.clone());
        rows.push(PlanRow {
            left: match (&was, win.reload) {
                (Some(was), false) => format!("{was} → {name}"),
                _ => name.clone(),
            },
            middle: String::new(),
            right: if win.reload {
                lang.pick("読み直す", "Reload").into()
            } else if was.is_none() {
                lang.pick("追加", "Add").into()
            } else {
                lang.pick("差し替え", "Change").into()
            },
            warning: false,
        });
        let groups = win.groups(app);
        // 3D のパスを持つ層は、新しいモデルの形（指紋）と合わなければ、パスのまま新しいモデルには結び付かない（画素は残る。パスを
        // 新しいメッシュへ描き直す・画素にするのは、パスの道具ができたとき。Unity 版の SurfacePathRebind は移していない）
        if let Prep::Ready { model, .. } = &win.prep {
            let print = yolu_core::paths::fingerprint(model.geometry());
            let unbound = drafts
                .iter()
                .filter_map(|d| d.uid.and_then(|uid| app.sets.index_of(uid)))
                .flat_map(|i| app.set_doc(i).layers())
                .filter(|l| {
                    matches!(l.path(), Some(yolu_core::LayerPath::Surface(p)) if p.model_fingerprint != print)
                })
                .count();
            if unbound > 0 {
                rows.push(PlanRow {
                    left: lang.pick("3D のパスの層", "Layers with 3D paths").into(),
                    middle: unbound.to_string(),
                    right: lang.pick("画素だけ残る", "Pixels stay").into(),
                    warning: true,
                });
            }
        }
        for d in drafts.iter().filter(|d| d.uid.is_some()) {
            let (middle, warning) = match d.material.and_then(|g| groups.get(g)) {
                Some(g) => (format!("→ {}", g.name), false),
                None => (
                    format!("→ {}", lang.pick("モデルに無い", "not in this model")),
                    true,
                ),
            };
            rows.push(PlanRow {
                left: d.name.trim().to_owned(),
                middle,
                right: String::new(),
                warning,
            });
        }
    }
    Ok(Plan {
        removed,
        resizes,
        model,
        kinds,
        rows,
    })
}

// ───────── 適用 ─────────

/// 計画どおりに適用する。断るときは何も変えず、理由を返す。うまくいけば知らせの文を返す。
/// 当てた結果の知らせ（種類と文。消えた覚えた選択範囲・モデルに無いセット・戻せなかったポーズの項目があれば注意）。
pub(super) fn apply(
    app: &mut AppState,
    win: &mut NpWindow,
) -> Result<(crate::notice::Kind, String), String> {
    let lang = app.lang;
    let plan = plan(app, win)?;
    // ── 失敗しうる所（まだ何も変えない）──
    // 足すセットの文書: 今のセット（消すなら残るセットの先頭）と同じチャンネルと Normal の設定で、下書きの大きさ
    let like_index = {
        let current = app.sets.current_index();
        let kept = |i: usize| {
            app.sets
                .get(i)
                .is_some_and(|s| !plan.removed.contains(&s.uid))
        };
        if kept(current) {
            current
        } else {
            (0..app.sets.len()).find(|i| kept(*i)).unwrap_or(current)
        }
    };
    let (channels, normal, tile) = {
        let like = app.set_doc(like_index);
        (
            used_channels(like),
            like.normal_settings().with_file_direction(win.normal),
            like.tile_size(),
        )
    };
    let mut fresh: Vec<Option<Document>> = Vec::with_capacity(win.drafts.len());
    for d in &win.drafts {
        fresh.push(match d.uid {
            Some(_) => None,
            None => Some(new_set_document(
                d.size.0, d.size.1, tile, lang, &channels, normal,
            )?),
        });
    }
    // 大きさを変える（途中で断られたら、先に変えたセットを元へ戻す）
    let resized = resample(app, &plan)?;
    // ── ここから失敗しない ──
    app.doc.end_coalescing();
    // モデル
    let mut model_note = None;
    let mut pose_note = None;
    if plan.model.is_some() {
        if let Prep::Ready { path, model, .. } = std::mem::replace(&mut win.prep, Prep::Idle) {
            pose::install_prepared(&mut app.view3d, *model);
            pose::request_focus(&mut app.view3d);
            if let Some(session) = app.view3d.pose.session.as_ref() {
                let record = SceneModel::from_rig(&session.rig);
                model_note = Some(record.name.clone());
                app.model = Some(record);
            }
            app.np.model_file = Some(path);
            win.model = None;
            win.reload = false;
            // 入れたモデルは休みの形から始まる。ファイルのポーズ（pose.json）は、開くときに読み込めなかったモデル（動かした先を選び直す）・
            // 読み直したモデルにも戻す。戻さないまま保存すると、休みの形として pose.json が消える
            pose_note = crate::view3d::pose::stored::restore_from_project(app);
        }
    }
    let groups = app.model.as_ref().map(groups_of).unwrap_or_default();
    // 足す・消す・並べる・名前とマテリアル
    let mut order: Vec<u32> = Vec::with_capacity(win.drafts.len());
    let mut added: Vec<u32> = Vec::new();
    for (d, doc) in win.drafts.iter().zip(fresh) {
        let name = clean_name(&d.name).expect("計画で検めた");
        match (d.uid, doc) {
            (Some(uid), _) => {
                order.push(uid);
                if let Some(i) = app.sets.index_of(uid) {
                    if let Some(set) = app.sets.get_mut(i) {
                        if set.name != name && set.read_only.is_none() {
                            set.name = name;
                            set.auto_name = false;
                        }
                        match d.material.and_then(|g| groups.get(g)) {
                            Some(g) => {
                                set.material = g.key.clone();
                                set.bound = Some(g.index as u32);
                            }
                            None => set.bound = None,
                        }
                    }
                }
            }
            (None, Some(doc)) => {
                let key = d.key.clone();
                let i = app
                    .sets
                    .push(guid_string(doc.id()), name, false, key, None, doc);
                if let Some(set) = app.sets.get_mut(i) {
                    set.bound = d
                        .material
                        .and_then(|g| groups.get(g))
                        .map(|g| g.index as u32);
                    order.push(set.uid);
                    added.push(set.uid);
                }
            }
            (None, None) => {}
        }
    }
    // 消す（足したあとなので、全部を入れ替えるときも、消す途中でセットが 0 にならない）
    let removed_names = app.remove_sets(&plan.removed).unwrap_or_default();
    app.sets.reorder(&order);
    app.dedupe_set_keys();
    // 法線の形式（利用者が窓で変えたときだけ、全部のセットへ。足したセットは作るときに入れてある）。セットごとに持てるので、
    // 触っていないときは、形式が混在したプロジェクトの別のセットを黙って書き換えない
    if win.normal != win.normal_opened {
        for i in 0..app.sets.len() {
            let doc = app.set_doc_mut(i);
            let settings = doc.normal_settings();
            if settings.file_direction() != win.normal {
                let _ = doc.set_normal_settings(settings.with_file_direction(win.normal), false);
            }
        }
    }
    // 大きさを変えたセットは、履歴を消す（元の大きさへ戻せないので、段が残ると壊れた状態へ戻せてしまう）
    for uid in resized.iter().map(|r| r.uid) {
        if let Some(i) = app.sets.index_of(uid) {
            let _ = app.set_doc_mut(i).clear_history();
            if i == app.sets.current_index() {
                app.view.fit();
            }
        }
    }
    app.sel_doc_changed();
    app.ensure_selection();
    app.ui.renaming_set = None;
    app.ui.set_scroll = 0.0;
    app.sync_mesh_map_view();
    app.sync_view3d();
    app.modified = true;
    app.np.generation += 1;
    app.np.reopening = None;
    // 知らせ
    let missing = app.sets.iter().filter(|s| s.bound.is_none()).count();
    // 縮小で外れた覚えた選択範囲（履歴を消すので取り消しでも戻らない）。名前の変更を済ませたあとのセットの名前と数
    let dropped: Vec<(String, usize)> = resized
        .iter()
        .filter(|r| r.dropped_saved_selections > 0)
        .filter_map(|r| {
            let i = app.sets.index_of(r.uid)?;
            Some((app.sets.get(i)?.name.clone(), r.dropped_saved_selections))
        })
        .collect();
    let mut text = lang
        .pick(
            "プロジェクトの構成を変えました。",
            "Project configuration applied.",
        )
        .to_owned();
    if let Some(name) = model_note {
        text += &lang.pick(format!(" モデル: {name}。"), format!(" Model: {name}."));
    }
    let pose_noted = pose_note.is_some();
    if let Some(note) = pose_note {
        text += &format!(" {note}");
    }
    if !removed_names.is_empty() {
        text += &lang.pick(
            format!(" 消したセット: {}。", removed_names.join("・")),
            format!(" Removed: {}.", removed_names.join(", ")),
        );
    }
    if !resized.is_empty() {
        text += &lang.pick(
            format!(" 大きさを変えたセット {}。", resized.len()),
            format!(" Resized {}.", resized.len()),
        );
    }
    if !dropped.is_empty() {
        let list = |sep: &str, unit: &dyn Fn(usize) -> String| {
            dropped
                .iter()
                .map(|(name, n)| format!("{name} {}", unit(*n)))
                .collect::<Vec<_>>()
                .join(sep)
        };
        text += " ";
        text += &lang.with_reason(
            lang.pick(
                "縮小で消えた覚えた選択範囲があります",
                "Some remembered selections were lost to the shrink",
            ),
            lang.pick(
                list("・", &|n| format!("{n} 件")),
                list(", ", &|n| n.to_string()),
            ),
        );
    }
    let not_in_model = app.model.is_some() && missing > 0;
    if not_in_model {
        text += &lang.pick(
            format!(" モデルに無いセット {missing}。"),
            format!(" Not in the model: {missing}."),
        );
    }
    let kind = if dropped.is_empty() && !not_in_model && !pose_noted {
        crate::notice::Kind::Info
    } else {
        crate::notice::Kind::Warning
    };
    Ok((kind, text))
}

/// 大きさを変えたセット（`resample` の結果）。
struct Resized {
    uid: u32,
    /// 縮小で外れた覚えた選択範囲の数。
    dropped_saved_selections: usize,
}

/// 大きさを変える。先に全部のセットの結果を準備し（文書も履歴も変えない）、1 つでも断られたら何も変えずに理由を返す。全部が準備できて
/// から 1 つずつ入れる。変えたセットの uid と外れた覚えた選択範囲の数を返す（履歴は呼ぶ側が消す）。
fn resample(app: &mut AppState, plan: &Plan) -> Result<Vec<Resized>, String> {
    let lang = app.lang;
    let mut prepared: Vec<(u32, usize, PreparedResize)> = Vec::new();
    for (uid, size, method) in &plan.resizes {
        let Some(index) = app.sets.index_of(*uid) else {
            continue;
        };
        app.set_doc_mut(index).end_coalescing();
        match app
            .set_doc(index)
            .prepare_resize_image(size.0, size.1, *method, &mut || false)
        {
            Ok(Some(one)) => prepared.push((*uid, index, one)),
            Ok(None) => {}
            Err(e) => {
                let name = app
                    .sets
                    .get(index)
                    .map(|s| s.name.clone())
                    .unwrap_or_default();
                return Err(lang.with_reason(
                    lang.pick(
                        format!("テクスチャセット{}を変えられません", lang.quote(&name)),
                        format!("Cannot change texture set {}", lang.quote(&name)),
                    ),
                    lang.core_error(&e),
                ));
            }
        }
    }
    // 準備はもう済んでいるので、入れるのが断られるのは文書が変わったときだけ（ここでは起きない）。起きたら先に入れたセットを戻す
    // （積んだ段を捨てるだけなので、そのセットの前からの Undo の履歴は戻らない）
    let mut done: Vec<(Resized, usize)> = Vec::new();
    for (uid, index, one) in prepared {
        match app.set_doc_mut(index).commit_prepared_resize(one) {
            Ok(report) => done.push((
                Resized {
                    uid,
                    dropped_saved_selections: report.dropped_saved_selections,
                },
                index,
            )),
            Err(e) => {
                for (_, i) in done.iter().rev() {
                    let _ = app.set_doc_mut(*i).discard_last_step();
                }
                return Err(lang.core_error(&e));
            }
        }
    }
    Ok(done.into_iter().map(|(r, _)| r).collect())
}
