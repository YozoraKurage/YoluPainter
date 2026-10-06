//! 保存と「配布用に保存」が共有する、今の状態の完全な写し（`Project`）の材料と組み立て。
//!
//! - 材料（`capture`）は画面のスレッドで、描いていない区切りに取る。変わったセットの文書は `Document::capture_snapshot`（履歴を持たず、
//!   タイルは共有。元を次に書くときに元の側が複製するので、写しは動かない）と、小さな値だけ。重い所は取らない。
//! - 組み立て（`build`）は別のスレッドで動かす: セットごとの正本（写しから流して作る）・合成の PNG・選択範囲・見た目・メッシュマップ・
//!   棚・モデルの参照を、`Project` の置き換えとして重ねる。材料だけを読み、`AppState` には触らない（保存の間も、描く・見るは止まらない）。
//!
//! 画面のスレッドで同期に保存していた並びと同じで、書く .ylp のバイトは変わらない。

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use yolu_core::mesh_maps::BakedMeshMap;
use yolu_core::Document;
use yolu_io::{composite_pngs, shelf::Shelf, DocumentSource, Project, SetSpec};

use crate::lang::Lang;
use crate::newproject::relative_model_path;
use crate::sets::MaterialRef;
use crate::state::AppState;

/// 合成の PNG を作るスレッドのスタック（正本への詰め直しと合成の再帰に足りる大きさ）。
pub(crate) const THREAD_STACK: usize = 8 * 1024 * 1024;
/// 合成の PNG を同時に作るセットの数の上限（1 セットが作業のバッファを何枚も持つので、セットの数だけ並べない）。
const COMPOSITE_PARALLEL: usize = 4;

/// 1 セット分の材料。
pub(crate) struct SetCapture {
    pub id: String,
    pub name: String,
    pub material: MaterialRef,
    /// 文書の写し（読むだけのセットは無い。選択範囲・見た目・Unity の値はここから）。
    pub snapshot: Option<Arc<Document>>,
    /// 正本・合成の PNG を作り直すか（開いた時のファイルに無い、または開いた・保存した時から変わった）。
    pub rewrite: bool,
    /// 写しを取った時の文書の ID と版（書き直したら、保存済みの印にする）。
    pub mark: (u128, u64),
}

/// 組み立ての材料。
pub(crate) struct Capture {
    pub sets: Vec<SetCapture>,
    pub current: String,
    pub base: Option<Arc<Project>>,
    /// 開いたあとに文書を別の物に替えたセット（古い PSD の原本を持ち越さない）。
    pub replaced: Vec<String>,
    /// アセットの棚（変えていて読めるときだけ。変えていなければ開いたファイルのバイト列のまま）。
    pub shelf: Option<Shelf>,
    /// まだ .ylp に書いていないメッシュマップ（セットの ID・名前ごと）。
    pub maps: Vec<(String, String, Vec<Arc<BakedMeshMap>>)>,
    pub model: Option<PathBuf>,
    /// プロジェクトのモデルのポーズ（`pose.json` へ書く材料）と、保存できなかった項目（骨・BlendShape の名前）。
    pub pose: crate::view3d::pose::stored::PoseCapture,
    pub pose_unsaved: Vec<String>,
    /// モデルの相対のパスの基準にする .ylp の場所。
    pub anchor: PathBuf,
    /// Unity から受けた値を書くか（設定。切っていれば外す）。
    pub keep_received: bool,
    pub lang: Lang,
}

/// 材料を取る（文書は変えない）。`anchor` は、モデルのファイルの参照を相対にする基準の .ylp の場所。取れなければ理由。
pub(crate) fn capture(state: &AppState, anchor: PathBuf) -> Result<Capture, String> {
    let lang = state.lang;
    if state.is_stroking() {
        return Err(lang
            .pick("描いている間は保存しません", "Cannot save during a stroke")
            .into());
    }
    let base = state.project.as_ref().map(|p| p.project_shared());
    let mut sets = Vec::with_capacity(state.sets.len());
    let mut maps = Vec::new();
    for (i, set) in state.sets.iter().enumerate() {
        let doc = state.set_doc(i);
        let in_base = base
            .as_ref()
            .is_some_and(|b| b.sets().iter().any(|s| s.id == set.id));
        let read_only = set.read_only.is_some();
        if read_only && !in_base {
            return Err(lang.pick(
                format!("読むだけのセット「{}」の元の文書がありません", set.name),
                format!("Original document missing for read-only set “{}”", set.name),
            ));
        }
        let unchanged = set.saved == Some((doc.id(), doc.revision()));
        let rewrite = !(in_base && (read_only || unchanged));
        let snapshot = if read_only {
            None
        } else {
            let what = |e: &yolu_core::CoreError| {
                format!(
                    "{}: {}",
                    lang.pick(
                        format!("セット「{}」の文書の写しを取れません", set.name),
                        format!("Cannot copy texture set “{}”", set.name)
                    ),
                    lang.core_error(e)
                )
            };
            Some(Arc::new(doc.capture_snapshot().map_err(|e| what(&e))?))
        };
        let unsaved = set.mesh_maps.unsaved();
        if !unsaved.is_empty() {
            maps.push((set.id.clone(), set.name.clone(), unsaved));
        }
        sets.push(SetCapture {
            id: set.id.clone(),
            name: set.name.clone(),
            material: set.material.clone(),
            snapshot,
            rewrite,
            mark: (doc.id(), doc.revision()),
        });
    }
    let (pose, pose_unsaved) = crate::view3d::pose::stored::capture(state);
    let replaced = super::replaced_sets(
        base.as_deref(),
        // 読むだけのセットの文書は見せるだけの写し（保存の正本は開いたときのバイト列のまま）なので数えない
        state
            .sets
            .iter()
            .enumerate()
            .filter(|(_, set)| set.read_only.is_none())
            .map(|(i, set)| (set.id.as_str(), state.set_doc(i).id())),
    );
    Ok(Capture {
        sets,
        current: state.sets.current().id.clone(),
        base,
        replaced,
        shelf: (state.shelf.changed && state.shelf.unavailable.is_none())
            .then(|| state.shelf.shelf().clone()),
        maps,
        model: state.np.model_file.clone(),
        pose,
        pose_unsaved,
        anchor,
        keep_received: state.prefs.settings.livelink_keep_values,
        lang,
    })
}

/// 組み立てが済まなかった理由。
#[derive(Debug)]
pub(crate) enum BuildError {
    Canceled,
    Message(String),
}

/// 書き直すセットの合成の PNG（チャンネルごと）。
type Composites = Vec<(yolu_core::Channel, Vec<u8>)>;
/// 書き直すセットの（正本の元・合成の PNG）。
type Composed = (DocumentSource, Composites);

/// 組み立ての結果。
pub(crate) struct Built {
    pub project: Project,
    /// 開くときに読めなかった見た目の設定を、変えた見た目で上書いたセットの ID。
    pub looks_overwritten: Vec<String>,
    /// 開くときに読めなかった、名前を付けて残した選択範囲の項目を、変えた並びで置き換えたセットの ID。
    pub saved_overwritten: Vec<String>,
    /// 開くときに読めなかったポーズ（`pose.json`）を、今のポーズで上書きしたか。
    pub pose_overwritten: bool,
    /// 書き直したセットの、効いていない効果が合成の PNG に入っていないことの知らせ（セットごとに 1 文。無ければ空）。
    pub inactive_effects: String,
}

/// 進み具合（組み立ての間に、合成の PNG を作り終えたセットの数）。
#[derive(Default)]
pub(crate) struct BuildProgress {
    pub sets_total: AtomicUsize,
    pub sets_done: AtomicUsize,
}

/// 材料から、今の状態の完全な写しの `Project` を組む（別のスレッドで動かす。取消は区切りで効く）。
pub(crate) fn build(
    capture: &Capture,
    cancel: Option<&AtomicBool>,
    progress: &BuildProgress,
) -> Result<Built, BuildError> {
    let lang = capture.lang;
    let io = |e: yolu_io::Error| BuildError::Message(lang.io_error(&e));
    let canceled = || cancel.is_some_and(|c| c.load(Ordering::Relaxed));
    let rewriting: Vec<&SetCapture> = capture.sets.iter().filter(|s| s.rewrite).collect();
    progress.sets_total.store(rewriting.len(), Ordering::Relaxed);
    progress.sets_done.store(0, Ordering::Relaxed);
    // 書き直すセットの正本の元と合成の PNG。セットは互いに独立なので、いくつかを同時に作る（結果は 1 つずつ作るのと同じ並び・
    // 失敗の理由も、並びのいちばん前に失敗したセットのもの）
    let made = compose_sets(&rewriting, lang, &canceled, progress);
    let mut by_set: Vec<Composed> = Vec::new();
    for done in made {
        by_set.push(done.expect("失敗より前は飛ばさない")?);
    }
    if canceled() {
        return Err(BuildError::Canceled);
    }
    let mut by_set = by_set.into_iter();
    let mut specs = Vec::with_capacity(capture.sets.len());
    for set in &capture.sets {
        let (document, composites) = if set.rewrite {
            let (source, pngs) = by_set.next().expect("書き直すセットの数だけ作った");
            (Some(source), pngs)
        } else {
            (None, Vec::new())
        };
        specs.push(SetSpec {
            id: set.id.clone(),
            name: set.name.clone(),
            material: set.material.clone(),
            document,
            composites,
        });
    }
    let writer = crate::project::writer();
    // 書き直すセットの新しい大きさ。大きさを変えたセットは、古い大きさの選択範囲のエントリを正本の書き直しのあとに外す
    let sizes: Vec<(&str, (u32, u32, u32))> = capture
        .sets
        .iter()
        .filter(|s| s.rewrite)
        .filter_map(|s| s.snapshot.as_ref().map(|d| (s.id.as_str(), (d.width(), d.height(), d.tile_size()))))
        .collect();
    let resized = capture
        .base
        .as_ref()
        .map(|base| crate::selection::io::resized_sets(base, &sizes))
        .unwrap_or_default();
    let mut project = match &capture.base {
        Some(base) => {
            let upgraded;
            let base: &Project = if base.info().format < 7 {
                upgraded = base.upgraded(writer.clone()).map_err(io)?;
                &upgraded
            } else {
                base
            };
            // 大きさを変えた文書の、古い大きさの今の選択範囲は外す（新しい文書と合わず、書き直しの検証が断る。今の選択範囲はあとで書く）
            let fitted = crate::selection::io::without_stale(base, &sizes, lang).map_err(BuildError::Message)?;
            let base: &Project = &fitted;
            // 開いたあとに消したセット（プロジェクトの構成・テクスチャセットのパネルで確かめて消したもの）は、ファイルからも消す
            let dropped: Vec<&str> = base
                .sets()
                .iter()
                .map(|s| s.id.as_str())
                .filter(|id| !capture.sets.iter().any(|s| s.id == *id))
                .collect();
            base.with_sets_dropping(writer.clone(), &specs, &capture.current, &dropped)
                .map_err(io)?
        }
        None => Project::create(writer.clone(), &specs, &capture.current).map_err(io)?,
    };
    // 文書を別の物に替えたセット（PSD を「今のセットへ」取り込み直した）は、古い PSD の原本を持ち越さない（新しい文書の原本ではない）
    for id in &capture.replaced {
        project = project.without_imported_original(id).map_err(io)?;
    }
    let text = BuildError::Message;
    // 選択範囲（selection.bin）は正本と別のエントリ。読むだけのセットは元のまま、描けるセットは今の選択範囲と違えば書き換える
    let selections: Vec<(&str, Option<&yolu_core::SelectionMask>)> = capture
        .sets
        .iter()
        .filter_map(|s| s.snapshot.as_ref().map(|d| (s.id.as_str(), d.selection())))
        .collect();
    project = crate::selection::io::write_into(project, &selections, lang).map_err(text)?;
    // 名前を付けて残した選択範囲（sets/<ID>/selections.json。使う文書だけが形式 8）。残した選択範囲を変えたセットだけ書き換える
    let saved: Vec<(&str, &[yolu_core::SavedSelection])> = capture
        .sets
        .iter()
        .filter_map(|s| s.snapshot.as_ref().map(|d| (s.id.as_str(), d.saved_selections())))
        .collect();
    let (written, saved_overwritten) = crate::selection::io::write_saved_into(project, &saved, &resized, lang).map_err(text)?;
    project = written;
    // 見た目の設定（look.json。正本と別のエントリ。違うセットだけ書き換える）。Unity から受けた値は、設定が入のときだけ書く
    let looks: Vec<(&str, &yolu_core::look::MaterialLook)> = capture
        .sets
        .iter()
        .filter_map(|s| s.snapshot.as_ref().map(|d| (s.id.as_str(), d.look())))
        .collect();
    let (written, looks_overwritten) = crate::look::io::write_into(project, &looks, lang).map_err(text)?;
    project = written;
    let received: Vec<(&str, Option<&yolu_core::look::ReceivedLook>)> = capture
        .sets
        .iter()
        .filter_map(|s| {
            s.snapshot.as_ref().map(|d| {
                (s.id.as_str(), d.received_look().filter(|_| capture.keep_received))
            })
        })
        .collect();
    project = crate::look::io::write_received_into(project, &received, lang).map_err(text)?;
    // モデルのポーズ（pose.json。状態のエントリで、形式は上げない。違うときだけ書く）
    let (written, pose_overwritten) =
        crate::view3d::pose::stored::write_into(project, &capture.pose, lang).map_err(text)?;
    project = written;
    // 焼いてまだ書いていないメッシュマップ（開いた時のものは、ファイルのバイト列のまま残っている）
    for (id, name, maps) in &capture.maps {
        for map in maps {
            project = project.with_mesh_map(id, map).map_err(|e| {
                BuildError::Message(lang.pick(
                    format!("セット「{name}」のメッシュマップ: {}", lang.io_error(&e)),
                    format!("Mesh maps of set \"{name}\": {}", lang.io_error(&e)),
                ))
            })?;
        }
    }
    // アセットの棚: 変えたときだけ resources を書き直す（変えていなければ開いたファイルのバイト列のまま）
    if let Some(shelf) = &capture.shelf {
        project = project.with_shelf(shelf, writer).map_err(|e| {
            BuildError::Message(format!(
                "{}: {}",
                lang.pick("棚を書けません", "Cannot write the shelf"),
                lang.io_error(&e)
            ))
        })?;
    }
    if canceled() {
        return Err(BuildError::Canceled);
    }
    // モデルのファイル（FBX）の参照: .ylp からの相対のパスで view.json に残す。変わっていなければ view.json に触らない
    // （Unity 版が書いたものはそのまま）
    let wanted = capture
        .model
        .as_ref()
        .map(|m| relative_model_path(m, &capture.anchor));
    if project.view_model().ok().flatten() != wanted {
        project = project.with_view_model(wanted.as_deref()).map_err(io)?;
    }
    // 書き直したセットに入力のまま通る効果があれば、合成の PNG に入っていないことを言う（正本には設定が残る）
    let mut inactive_effects = String::new();
    for set in capture.sets.iter().filter(|s| s.rewrite) {
        if let Some(doc) = &set.snapshot {
            let inactive = doc.inactive_effect_list();
            if !inactive.is_empty() {
                inactive_effects += &lang.inactive_effects_not_in_composite(&set.name, &inactive);
            }
        }
    }
    Ok(Built { project, looks_overwritten, saved_overwritten, pose_overwritten, inactive_effects })
}

/// 書き直すセットごとの（正本の元・合成の PNG）。並びはセットの並び。失敗したセットより後ろは作らず `None`（先に断りが見つかる）。
fn compose_sets(
    rewriting: &[&SetCapture],
    lang: Lang,
    canceled: &(dyn Fn() -> bool + Sync),
    progress: &BuildProgress,
) -> Vec<Option<Result<Composed, BuildError>>> {
    let results: Vec<std::sync::Mutex<Option<Result<_, BuildError>>>> =
        rewriting.iter().map(|_| std::sync::Mutex::new(None)).collect();
    let next = AtomicUsize::new(0);
    let failed = AtomicUsize::new(usize::MAX);
    let threads = rewriting.len().clamp(1, COMPOSITE_PARALLEL);
    // ここで作る物（正本の元の確かめ・合成の PNG）は、書く形の閾値（`Thresholds`。スレッドごとの値）を読まない。正本の形の計画は、組み立てを
    // 呼んだスレッドが `Project` を組むとき（`Project::create`・`with_sets_dropping`）に立てる。試験（`save_background` の
    // `several_sets_build_their_source_under_the_thresholds_of_the_save_request`）が、複数のセットでも小さくした閾値が効くことを見張る
    let one = |set: &SetCapture| -> Result<_, BuildError> {
        if canceled() {
            return Err(BuildError::Canceled);
        }
        let doc = set.snapshot.as_ref().expect("読むだけのセットは作り直さない");
        let native = DocumentSource::from_core(doc.clone()).map_err(|e| {
            BuildError::Message(format!(
                "{}: {}",
                lang.pick(
                    format!("セット「{}」の文書を作れません", set.name),
                    format!("Cannot convert texture set “{}” to a document", set.name)
                ),
                lang.io_error(&e)
            ))
        })?;
        let pngs = composite_pngs(doc).map_err(|e| {
            BuildError::Message(format!(
                "{}: {}",
                lang.pick(
                    format!("セット「{}」の合成の PNG を作れません", set.name),
                    format!("Cannot build the composite PNG of texture set “{}”", set.name)
                ),
                lang.io_error(&e)
            ))
        })?;
        Ok((native, pngs))
    };
    let work = || loop {
        let i = next.fetch_add(1, Ordering::Relaxed);
        let Some(set) = rewriting.get(i) else { return };
        // 前のセットが失敗していれば、この先は作らない（並びのいちばん前の失敗を返すのに要らない）
        if i > failed.load(Ordering::Relaxed) {
            continue;
        }
        let done = one(set);
        if done.is_err() {
            failed.fetch_min(i, Ordering::Relaxed);
        }
        progress.sets_done.fetch_add(1, Ordering::Relaxed);
        *results[i].lock().unwrap_or_else(|e| e.into_inner()) = Some(done);
    };
    if threads == 1 {
        // 1 セットのときは、呼んだスレッド（スタックは呼ぶ側が決めている）でそのまま作る
        work();
    } else {
        std::thread::scope(|scope| {
            for k in 0..threads {
                let spawned = std::thread::Builder::new()
                    .name(format!("yolu-save-compose-{k}"))
                    .stack_size(THREAD_STACK)
                    .spawn_scoped(scope, work);
                // 作れなかったときは、残りを呼んだスレッドで続ける（仕事は共有の番号から取るので、重ならない）
                if spawned.is_err() {
                    work();
                    break;
                }
            }
        });
    }
    let mut out: Vec<_> = results
        .into_iter()
        .map(|m| m.into_inner().unwrap_or_else(|e| e.into_inner()))
        .collect();
    // 失敗したセットより前は全部できている。後ろは、飛ばした・先に断りがあるので、断りまでを返せば足りる
    if let Some(at) = out.iter().position(|r| matches!(r, Some(Err(_)))) {
        out.truncate(at + 1);
    }
    out
}
