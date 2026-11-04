//! アセットへ取り込む画像（棚の格子へ落とした画像・メニューの「画像…」）の読みを別のスレッドで行う。
//!
//! - 読み（`imagefile::read_file_as`。浮動小数の画素の色の変換を含む）と、行の上下の入れ替え（文書と同じ向き）・組の画素の変換
//!   （値の反転・ノーマルの緑の反転・ORM の分割）・PNG への圧縮と中身のハッシュ（`yolu_io::shelf::encode_image`。大きな画像ではここが
//!   一番長い）は別のスレッド。画面のスレッドは、圧縮した画像を棚の索引へ入れるだけ（`add_encoded_image`）で、組をマテリアルにするのも
//!   画面のスレッド（`pbrset`）。
//! - 2 枚以上の組は、全部が読み終わってから判別（`pbr::plan`）の結果で組み立てる（1 枚ずつは足さない）。判別は名前だけで決まるので、
//!   読む前に画面のスレッドで行い、画像ごとの値の扱い（色は sRGB・データは線形）を読む側へ渡す。
//! - 仕事は 1 本の列に並び、前の仕事の読みが終わってから次のスレッドを始める（足す順は落とした順。別々に落とした大きな画像の読みが
//!   並んでメモリを倍にしない）。
//! - 仕事は棚（`ShelfState`）が持つ。プロジェクトを開く・新しくすると棚ごと捨てられ、読み終わっていない仕事は取り消され、
//!   読み終わった物も足さない。別のスレッドの素材の保存が棚の写しへ足している間に読み終わった物は、保存が終わるまで置いておく。
//! - 札と「終わる前に止める」の行は `shelf::IMAGE_READ_JOB`、毎フレームの受け取りは `shelf_poll` から（モジュールの依存の線を増やさないため、棚の側に置く）。
//! - 取り消しは次のファイルの読み始めの前に効く（読んでいる 1 枚は読み切る。結果は捨てる）。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::imagefile::{self, ReadError, Transfer};
use crate::jobs::{Polled, Worker};
use crate::lang::Lang;
use crate::notice::Source;
use crate::pbr::{self, Plan};
use crate::state::AppState;
use yolu_core::fill_image::ProjectionMode;
use yolu_core::Rgba8;
use yolu_io::shelf::{self, EncodedImage};

use super::pbrset;

/// 読んだ画像から作る物（名前の判別で決まる。画面のスレッドで決めて、読む側へ渡す）。
#[derive(Clone, Copy)]
pub(super) enum Make {
    /// 1 枚の画像。`invert` は値を反転（Glossiness）、`flip_green` は緑を反転（DirectX のノーマル）、`space` は棚の色空間。
    One {
        invert: bool,
        flip_green: bool,
        space: &'static str,
    },
    /// ORM・ARM を AO・Roughness・Metallic の 3 枚（線形）へ分ける。
    Packed,
}

impl Make {
    /// 印の無い画像（色。変換しない）。
    pub(super) const PLAIN: Make = Make::One {
        invert: false,
        flip_green: false,
        space: "srgb",
    };
}

/// 圧縮まで済ませた画像 1 枚（文書と同じ向き＝下の行が先の画素から作った物）。
pub(super) struct Piece {
    pub image: EncodedImage,
    /// 圧縮の前の画素の平均の色（不透明。マテリアルの値の既定）。
    pub mean: Rgba8,
    /// 棚の色空間（"srgb"・"linear"）。
    pub space: &'static str,
}

/// 読んで、棚へ入れる形まで作った画像（`Make::One` は 1 枚、`Make::Packed` は `pbr::PACKED` の順に 3 枚）。
pub(super) struct Loaded {
    pub pieces: Vec<Piece>,
}

/// 読めなかった理由。
pub(super) enum Failure {
    /// 読める形式の拡張子でない。
    Unsupported,
    Read(image::ImageFormat, ReadError),
    /// PNG への圧縮ができない（寸法の上限など）。
    Encode(yolu_io::Error),
    /// 読む前にスレッドが止まった。
    Stopped,
}

impl Failure {
    /// 断りの文に添える理由。
    pub(super) fn reason(&self, lang: Lang) -> String {
        match self {
            Failure::Unsupported => lang.t("dropin-unsupported-format").to_owned(),
            Failure::Read(format, e) => lang.image_read_error(*format, e),
            Failure::Encode(e) => crate::lang::shelf_io_error(lang, e),
            Failure::Stopped => lang.t("dropin-not-read").to_owned(),
        }
    }
}

pub(super) type Read = Result<Loaded, Failure>;

/// 1 枚を棚へ足した後にすること。
#[derive(Clone, Copy)]
pub(crate) enum Then {
    /// 足すだけ（棚の選びが、足した（か、もうあった）画像になる）。
    Nothing,
    /// 足した画像で、新しい塗りつぶしレイヤーを作る（レイヤーのメニューの「画像から作る」）。読み終わった時点の状態で作る（描いている途中・
    /// ロックされたグループ・選んでいるレイヤーは、その時に見る）。
    FillLayer(ProjectionMode),
}

/// 何を読んでいるか（読み終わった後の足し方）。
pub(super) enum Request {
    /// 1 枚（`import_image_file`）。
    One { path: PathBuf, then: Then },
    /// 2 枚以上（`pbrset`。`plan` は落とした順の名前の判別）。
    Set { paths: Vec<PathBuf>, plan: Plan },
}

/// 読む 1 枚。
pub(super) struct Entry {
    /// 落とした順の番号。
    pub index: usize,
    pub path: PathBuf,
    /// 浮動小数の画素の値の扱い。
    pub transfer: Transfer,
    /// 読んだ後に作る物。
    pub make: Make,
}

/// 読みの仕事 1 つ（`ImageReads` の列に並ぶ。前の仕事の読みが終わるまで、スレッドは始めない）。
struct ReadJob {
    request: Request,
    /// まだ読み始めていない画像（始めたら `worker` へ渡して None）。
    entries: Option<Vec<Entry>>,
    worker: Option<Worker<(usize, Read)>>,
    /// 落とした順に、読み終わった物。
    reads: Vec<Option<Read>>,
    received: usize,
    /// 受け口の送り口が捨てられた（スレッドが全部を知らせずに止まった）。
    lost: bool,
}

impl ReadJob {
    fn finished(&self) -> bool {
        self.lost || self.received == self.reads.len()
    }

    /// スレッドが走っている（始めていて、読み終わっていない）。
    fn reading(&self) -> bool {
        self.worker.is_some() && !self.finished()
    }
}

/// 棚の画像の読みの仕事（棚が持つ）。仕事は 1 本の列で、前の仕事の読みが終わってから次を始める（足す順は落とした順になり、
/// 別々に落とした大きな画像の読みが並んでメモリを倍にしない）。
#[derive(Default)]
pub struct ImageReads {
    jobs: Vec<ReadJob>,
    /// 試験用: true の間、別のスレッドは読み始めずに待つ。
    hold: Arc<AtomicBool>,
}

impl ImageReads {
    /// 読んでいる（か、読み終わって足すのを待っている）仕事がある。
    pub fn is_busy(&self) -> bool {
        !self.jobs.is_empty()
    }

    /// スレッドが走っている仕事の数（列は 1 つずつなので、0 か 1）。
    pub(crate) fn running(&self) -> usize {
        self.jobs.iter().filter(|j| j.reading()).count()
    }

    /// 読み終わった枚数と全部の枚数（仕事が無ければ None）。
    pub(crate) fn progress(&self) -> Option<(usize, usize)> {
        if self.jobs.is_empty() {
            return None;
        }
        let total = self.jobs.iter().map(|j| j.reads.len()).sum();
        let done = self.jobs.iter().map(|j| j.received).sum();
        Some((done, total))
    }

    /// 試験用: true の間、別のスレッドは読み始めずに待つ（途中の状態を確かめるため）。
    #[doc(hidden)]
    pub fn hold(&self, hold: bool) {
        self.hold.store(hold, Ordering::SeqCst);
    }
}

/// 画像 1 枚を読んで、棚へ入れる形まで作る（別のスレッドで呼ぶ）。
fn read_one(path: &Path, transfer: Transfer, make: Make) -> Read {
    let Some(format) = imagefile::format_of(path) else {
        return Err(Failure::Unsupported);
    };
    let (mut rgba, width, height) =
        imagefile::read_file_as(path, format, transfer).map_err(|e| Failure::Read(format, e))?;
    // 文書と同じ向き（下の行が先）で渡す（読んだ画素の並びをその場で返す。大きな画像の写しを重ねない）
    super::flip_rows(&mut rgba, width as usize * 4);
    let piece = |px: &[u8], space| -> Result<Piece, Failure> {
        let image = shelf::encode_image(px, width, height).map_err(Failure::Encode)?;
        Ok(Piece {
            image,
            mean: pbrset::mean_color(px),
            space,
        })
    };
    let mut pieces = Vec::new();
    match make {
        Make::One {
            invert,
            flip_green,
            space,
        } => {
            if invert {
                pbr::invert(&mut rgba);
            }
            if flip_green {
                pbr::flip_green(&mut rgba);
            }
            pieces.push(piece(&rgba, space)?);
        }
        // 成分を 1 つずつ作って圧縮し、捨ててから次を作る（元の画像と 3 枚を同時に持たない）
        Make::Packed => {
            for part in 0..pbr::PACKED.len() {
                pieces.push(piece(&pbr::packed_part(&rgba, part), "linear")?);
            }
        }
    }
    Ok(Loaded { pieces })
}

impl AppState {
    /// `entries` の画像の読みを列へ入れる（前の仕事が無ければ、別のスレッドで読み始める。読めたら `poll_image_reads` が `request` の形で
    /// 棚へ足す）。スレッドを作れなければ、画像ごとに理由つきで断る。
    pub(super) fn start_image_read(&mut self, request: Request, entries: Vec<Entry>) {
        let reads = entries.iter().map(|_| None).collect();
        self.shelf.image_reads.jobs.push(ReadJob {
            request,
            entries: Some(entries),
            worker: None,
            reads,
            received: 0,
            lost: false,
        });
        self.start_next_image_read();
    }

    /// 読んでいる仕事が無ければ、列の先頭の読み始めていない仕事のスレッドを始める。始められなければ、その仕事を捨てて画像ごとに理由つきで断る。
    fn start_next_image_read(&mut self) {
        let lang = self.lang;
        loop {
            let reads = &mut self.shelf.image_reads;
            if reads.jobs.iter().any(ReadJob::reading) {
                return;
            }
            let Some(at) = reads.jobs.iter().position(|j| j.entries.is_some()) else {
                return;
            };
            let entries = reads.jobs[at].entries.take().unwrap_or_default();
            let hold = reads.hold.clone();
            let spawned = Worker::spawn("yolu-image-read", move |tx, cancel| {
                for Entry {
                    index,
                    path,
                    transfer,
                    make,
                } in entries
                {
                    while hold.load(Ordering::SeqCst) && !cancel.is_set() {
                        std::thread::sleep(std::time::Duration::from_millis(1));
                    }
                    if cancel.is_set() {
                        return;
                    }
                    if tx.send((index, read_one(&path, transfer, make))).is_err() {
                        return;
                    }
                }
            });
            match spawned {
                Ok(worker) => {
                    reads.jobs[at].worker = Some(worker.cancel_on_drop());
                    return;
                }
                Err(e) => {
                    let reason = lang.thread_error(&e);
                    let job = reads.jobs.remove(at);
                    let paths: Vec<PathBuf> = match &job.request {
                        Request::One { path, .. } => vec![path.clone()],
                        Request::Set { paths, .. } => paths.clone(),
                    };
                    for path in &paths {
                        let name = super::pbrset::stem_of(path);
                        self.fail(
                            Source::Assets,
                            lang.with_reason(super::cannot_add_image(lang, &name), &reason),
                        );
                    }
                }
            }
        }
    }

    /// 読み終わった画像を棚へ足す（毎フレーム）。足すのは列の先頭から、読み終わった仕事だけ（始めた順＝落とした順）。別のスレッドの素材の
    /// 保存が棚の写しへ足している間は、保存が終わるまで足さずに置いておく（保存の結果は足した後の棚で丸ごと差し替えるので、その間に棚へ足すと
    /// 足した画像が消える）。前の仕事の読みが終われば、次の仕事のスレッドを始める（足すのを待たない）。
    pub fn poll_image_reads(&mut self) {
        for job in &mut self.shelf.image_reads.jobs {
            let Some(worker) = &job.worker else {
                continue;
            };
            loop {
                match worker.poll() {
                    Polled::Message((index, read)) => {
                        job.reads[index] = Some(read);
                        job.received += 1;
                    }
                    Polled::Empty => break,
                    Polled::Lost => {
                        job.lost = true;
                        break;
                    }
                }
            }
        }
        self.start_next_image_read();
        while self.shelf.pending_save().is_none()
            && self
                .shelf
                .image_reads
                .jobs
                .first()
                .is_some_and(ReadJob::finished)
        {
            let job = self.shelf.image_reads.jobs.remove(0);
            self.add_read_images(job);
            self.start_next_image_read();
        }
    }

    /// 読み終わった仕事の画像を棚へ足す。読めなかった画像は、理由つきで 1 つずつ断る。
    fn add_read_images(&mut self, job: ReadJob) {
        let reads: Vec<Read> = job
            .reads
            .into_iter()
            .map(|r| r.unwrap_or(Err(Failure::Stopped)))
            .collect();
        match job.request {
            Request::One { path, then } => {
                let Some(read) = reads.into_iter().next() else {
                    return;
                };
                let added = self.add_read_image(&path, read);
                if let (Some(id), Then::FillLayer(mode)) = (added, then) {
                    if let Some(image) = super::inputs::image_id(&id) {
                        self.new_image_fill(image, mode);
                    }
                }
            }
            Request::Set { paths, plan } => self.add_read_image_set(&paths, &plan, reads),
        }
    }

    /// 画像の読みをやめる（読んでいる 1 枚は読み切るが、結果は捨てる。足し終えていない物は足さない）。
    pub(crate) fn cancel_image_reads(&mut self) {
        if self.shelf.image_reads.jobs.is_empty() {
            return;
        }
        self.shelf.image_reads.jobs.clear();
        self.info(
            Source::Assets,
            self.lang.t("fillfx-cancelled-reading-images"),
        );
    }

    /// 試験用: 画像の読みが終わって棚へ足すまで待つ（待ちの上限は 120 秒）。
    #[doc(hidden)]
    pub fn wait_image_reads(&mut self) {
        crate::jobs::wait_until_idle(
            self,
            "画像の読み込みが終わらない（ハング検出上限）",
            |s| s.shelf.image_reads.is_busy(),
            Self::poll_image_reads,
        );
    }
}
