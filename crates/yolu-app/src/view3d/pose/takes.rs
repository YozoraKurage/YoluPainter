//! FBX のテイク（中のアニメ）から、テイクとフレームを選んでポーズにする。
//!
//! - 一覧は FBX を読んだときに取る（`yolu_model::Takes`。名前・始まりと終わりのフレーム・フレームの速さだけ）。ポーズは裏の仕事で
//!   ファイルを読み直して求め（`yolu_model::evaluate_take_with`。曲線は持たない）、終わったら今のポーズに当てる。当てるのはポーズの取り消しの
//!   1 段で、当てたポーズはほかのポーズと同じく手で直せる。描いている間・続けて変えている操作の途中は、終わるまで当てるのを待つ。
//! - 骨はテイクの値（テイクが動かさない骨はファイルの休みの値）、BlendShape はテイクが動かすものだけ（ほかは今のまま）。
//! - まとめた Rig（Live Link の相手: FBX を並べたもの）では、テイクを持つ FBX の骨と BlendShape にだけ当てる（ほかの FBX の骨は今のまま）。
//!   部分の倍率は骨の移動に掛ける（`yolu_core::skin::merge` と同じ）。BlendShape を入れなかった部分は、骨だけに当てる。
//! - 欄で選んでいるテイクとフレームは、ポーズと一緒に .ylp の `pose.json` に残る（`stored`）。選びを変えるのも「変更あり」に数える。

use std::ops::Range;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use yolu_model::{evaluate_take_with, LoadControl, ModelLimits, Take, TakePose, Takes};

use super::View3dState;
use crate::jobs::{Polled, Worker};
use crate::lang::Lang;
use crate::notice::Source;
use crate::state::AppState;
use crate::view3d::model::ViewError;

/// テイクを持つ FBX 1 つ（ポーズのセッションの Rig のどこに入ったか）。
#[derive(Clone, Debug)]
pub struct TakeSource {
    /// 読み直すファイル。
    pub path: PathBuf,
    pub takes: Takes,
    /// FBX の骨が入った所（FBX の Rig の骨 i は、セッションの Rig の骨 `bones.start + i`）。
    pub bones: Range<usize>,
    /// 骨の移動に掛ける倍率（まとめた Rig の部分の倍率。FBX をそのまま読んだなら 1）。
    pub scale: f32,
    /// FBX の Rig のメッシュ → セッションの Rig のメッシュ（入れなかったメッシュ・BlendShape を入れなかった部分は None）。
    pub meshes: Vec<Option<usize>>,
}

impl TakeSource {
    /// FBX をそのまま読んだモデル（骨もメッシュも同じ並び）。テイクが無ければ None。
    pub fn whole(path: PathBuf, takes: Takes, bones: usize, meshes: usize) -> Option<TakeSource> {
        (!takes.is_empty()).then(|| TakeSource {
            path,
            takes,
            bones: 0..bones,
            scale: 1.0,
            meshes: (0..meshes).map(Some).collect(),
        })
    }

    /// ファイルの名前（欄でテイクの頭に添える）。
    fn file_name(&self) -> String {
        self.path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
}

/// 求めている途中のテイクのポーズ。
struct Job {
    worker: Worker<Result<TakePose, ViewError>>,
    source: usize,
    take: usize,
    frame: i64,
}

/// ポーズのセッションのテイク（一覧・欄の選び・求めている途中の仕事）。
#[derive(Default)]
pub struct TakeState {
    pub sources: Vec<TakeSource>,
    /// 欄で選んでいるテイク（`sources` の番号・その中のテイクの番号）。
    chosen: Option<(usize, usize)>,
    /// 欄のフレーム（選んでいるテイクの始まりから終わりまで）。
    frame: i64,
    job: Option<Job>,
}

impl TakeState {
    /// 最初のテイクの始まりのフレームを選んだ状態。
    pub fn new(sources: Vec<TakeSource>) -> TakeState {
        let mut out = TakeState {
            sources,
            ..TakeState::default()
        };
        out.reset_choice();
        out
    }

    /// 最初のテイクと、その始まりのフレーム。
    fn first(&self) -> Option<((usize, usize), i64)> {
        self.sources
            .iter()
            .enumerate()
            .find_map(|(i, s)| s.takes.list.first().map(|t| ((i, 0), t.first_frame)))
    }

    /// 欄の選びを最初のテイクの始まりへ戻す。
    fn reset_choice(&mut self) {
        let first = self.first();
        self.chosen = first.map(|f| f.0);
        self.frame = first.map_or(0, |f| f.1);
    }

    /// テイクがあるか（無ければ欄を出さない）。
    pub fn is_empty(&self) -> bool {
        self.sources.iter().all(|s| s.takes.is_empty())
    }

    /// 選んでいるテイク（`sources` の番号・テイクの番号）。
    pub fn chosen(&self) -> Option<(usize, usize)> {
        self.chosen
    }

    /// 選んでいるテイクの中身。
    pub fn chosen_take(&self) -> Option<&Take> {
        let (s, t) = self.chosen?;
        self.sources.get(s)?.takes.list.get(t)
    }

    pub fn frame(&self) -> i64 {
        self.frame
    }

    /// 選びが最初のテイクの始まりのまま（`pose.json` に書かない）か。
    pub fn is_default_choice(&self) -> bool {
        self.first().map(|f| (Some(f.0), f.1)) == Some((self.chosen, self.frame))
    }

    /// 求めている途中か。
    pub fn is_running(&self) -> bool {
        self.job.is_some()
    }

    /// テイクの名前（FBX が 2 つ以上なら、頭にファイルの名前）。
    pub fn label(&self, source: usize, take: usize) -> String {
        let Some(s) = self.sources.get(source) else {
            return String::new();
        };
        let name = s
            .takes
            .list
            .get(take)
            .map(|t| t.name.as_str())
            .unwrap_or_default();
        if self.sources.len() > 1 {
            format!("{} / {name}", s.file_name())
        } else {
            name.to_owned()
        }
    }

    /// 選べるテイクの全部（`sources` の番号・テイクの番号）。
    pub fn all(&self) -> Vec<(usize, usize)> {
        self.sources
            .iter()
            .enumerate()
            .flat_map(|(i, s)| (0..s.takes.list.len()).map(move |t| (i, t)))
            .collect()
    }

    /// テイクを選ぶ（フレームはそのテイクの範囲へ寄せる）。変わったら true。
    pub fn choose(&mut self, source: usize, take: usize) -> bool {
        let Some(t) = self
            .sources
            .get(source)
            .and_then(|s| s.takes.list.get(take))
        else {
            return false;
        };
        let frame = self.frame.clamp(t.first_frame, t.last_frame);
        let changed = self.chosen != Some((source, take)) || frame != self.frame;
        self.chosen = Some((source, take));
        self.frame = frame;
        changed
    }

    /// フレームを選ぶ（選んでいるテイクの範囲へ寄せる）。変わったら true。
    pub fn set_frame(&mut self, frame: i64) -> bool {
        let Some(t) = self.chosen_take() else {
            return false;
        };
        let frame = frame.clamp(t.first_frame, t.last_frame);
        let changed = frame != self.frame;
        self.frame = frame;
        changed
    }

    /// 名前でテイクを選ぶ（開いた .ylp の選びを戻す。名前の合うテイクが無ければ選びを変えず false）。
    pub fn choose_by_name(&mut self, name: &str, frame: i64) -> bool {
        let found = self
            .sources
            .iter()
            .enumerate()
            .find_map(|(i, s)| s.takes.find(name).map(|t| (i, t)));
        match found {
            Some((s, t)) => {
                let take = &self.sources[s].takes.list[t];
                self.frame = frame.clamp(take.first_frame, take.last_frame);
                self.chosen = Some((s, t));
                true
            }
            None => false,
        }
    }
}

/// 欄でテイクを選ぶ（変われば「変更あり」に数える）。
pub fn choose(app: &mut AppState, source: usize, take: usize) {
    if let Some(s) = app.view3d.pose.session.as_mut() {
        if s.takes.choose(source, take) {
            s.edits += 1;
        }
    }
}

/// 欄でフレームを選ぶ（変われば「変更あり」に数える）。
pub fn set_frame(app: &mut AppState, frame: i64) {
    if let Some(s) = app.view3d.pose.session.as_mut() {
        if s.takes.set_frame(frame) {
            s.edits += 1;
        }
    }
}

/// 選んでいるテイクとフレームのポーズを、裏の仕事で求め始める（求めている途中のものは取り消す）。描いている最中・続けて変えている
/// 操作の途中は断る。
pub fn start(view3d: &mut View3dState) -> Result<(), ViewError> {
    if view3d.input.stroke.is_some() {
        return Err(ViewError::Stroking);
    }
    let s = view3d.pose.session.as_mut().ok_or(ViewError::NoPoseModel)?;
    if s.is_editing() {
        return Err(ViewError::NoPoseEdit);
    }
    let (source, take) = s.takes.chosen.ok_or(ViewError::NoPoseModel)?;
    let frame = s.takes.frame;
    let src = &s.takes.sources[source];
    let path = src.path.clone();
    let takes = src.takes.clone();
    // 前の仕事は捨てる（受け口を捨てると、次の区切りで止まる）
    s.takes.job = None;
    let cancel = Arc::new(AtomicBool::new(false));
    let finished = view3d.pose.loads.register(&cancel);
    let worker = Worker::spawn_on(
        std::thread::Builder::new().name("yolu-fbx-take".into()),
        cancel,
        move |tx, flag| {
            let _finished = finished;
            let result = evaluate_take_with(
                &path,
                &takes,
                take,
                frame,
                &ModelLimits::default(),
                LoadControl {
                    cancel: Some(flag.flag()),
                    progress: None,
                },
            )
            .map_err(ViewError::from);
            let _ = tx.send(result);
        },
    )
    .expect("failed to spawn thread")
    .cancel_on_drop();
    if let Some(s) = view3d.pose.session.as_mut() {
        s.takes.job = Some(Job {
            worker,
            source,
            take,
            frame,
        });
    }
    Ok(())
}

/// 求めたテイクのポーズを、今のポーズに重ねる（そのテイクの FBX の骨と BlendShape だけ）。
fn overlay(
    view3d: &View3dState,
    source: usize,
    take: &TakePose,
) -> Result<yolu_core::skin::Pose, ViewError> {
    let s = view3d.pose.session.as_ref().ok_or(ViewError::NoPoseModel)?;
    let src = s.takes.sources.get(source).ok_or(ViewError::NoPoseModel)?;
    if take.locals.len() != src.bones.len() || src.bones.end > s.rig.bones().len() {
        return Err(ViewError::Model(yolu_model::ModelError::Changed));
    }
    let mut pose = s.pose().clone();
    for (i, local) in take.locals.iter().enumerate() {
        let mut local = *local;
        local.translation *= src.scale;
        pose.locals[src.bones.start + i] = local;
    }
    for &(m, k, w) in &take.weights {
        let Some(Some(target)) = src.meshes.get(m) else {
            continue;
        };
        if let Some(slot) = pose
            .blend_weights
            .get_mut(*target)
            .and_then(|v| v.get_mut(k))
        {
            *slot = w;
        }
    }
    Ok(pose)
}

/// 「「名前」のフレーム N」（知らせの頭。数で終わるので、続く語との間に空白を置く）。
fn what(lang: Lang, name: &str, frame: i64) -> String {
    let name = lang.quote(name);
    lang.pick(
        format!("{name}のフレーム {frame}"),
        format!("frame {frame} of {name}"),
    )
}

/// フレームの初めに: 求め終えたテイクのポーズを当てて知らせる。描いている間・続けて変えている操作の途中は、終わるまで待つ。
pub fn poll(app: &mut AppState) {
    let waiting = app.is_stroking() || app.view3d.pose.drag.is_some();
    let Some(s) = app.view3d.pose.session.as_mut() else {
        return;
    };
    let Some(job) = &s.takes.job else {
        return;
    };
    if waiting || s.is_editing() {
        return;
    }
    let result = match job.worker.poll() {
        Polled::Empty => return,
        Polled::Message(r) => r,
        Polled::Lost => Err(ViewError::LoadStopped),
    };
    let Job {
        source,
        take,
        frame,
        ..
    } = s.takes.job.take().expect("上で見た");
    let lang = app.lang;
    let label = s.takes.label(source, take);
    let head = what(lang, &label, frame);
    let applied = result.and_then(|p| {
        let pose = overlay(&app.view3d, source, &p)?;
        super::set_pose(&mut app.view3d, pose)
    });
    match applied {
        Ok(()) => app.info(
            Source::Pose,
            lang.pick(
                format!("{head} をポーズにしました。"),
                format!("Posed from {head}."),
            ),
        ),
        Err(ViewError::Cancelled) => {}
        Err(e) => {
            let text = lang.with_reason(
                lang.pick(
                    format!("{head} をポーズにできません"),
                    format!("Cannot pose from {head}"),
                ),
                lang.view_error(&e),
            );
            app.notify(e.notice_kind(), Source::Pose, text);
        }
    }
}

/// 試験用: 求めている途中の仕事が終わるまで待ってから `poll` する（上限 120 秒）。
#[doc(hidden)]
pub fn wait(app: &mut AppState) {
    if let Some(job) = app
        .view3d
        .pose
        .session
        .as_mut()
        .and_then(|s| s.takes.job.as_mut())
    {
        let r = match job.worker.wait(std::time::Duration::from_secs(120)) {
            Polled::Message(r) => r,
            Polled::Empty | Polled::Lost => Err(ViewError::LoadStopped),
        };
        let (worker, tx) = Worker::parked();
        let _ = tx.send(r);
        job.worker = worker.cancel_on_drop();
    }
    poll(app);
}
