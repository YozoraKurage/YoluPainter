//! 取り込みの窓の「CLIP STUDIO から」: CLIP STUDIO PAINT のサブツールのフォルダを探して `.sut` を一覧にし（名前と筆先の見本）、
//! 選んだものだけを、ふつうの `.sut` の取り込み（`BrushAction::Import`）と同じ道で取り込む。
//!
//! 探す・中身を覗く仕事は別のスレッド（`yolu_io::brushes::clipstudio`。**読むだけ**で、CLIP STUDIO のフォルダへ何も書かず、
//! ロックもしない）。窓を閉じる・探し直す・フォルダを替えると、走っている仕事は取り消す。フォルダの場所の見つけ方と、読んでいる間の
//! 書き込みへの備えは `yolu_io::brushes::clipstudio` と docs/BRUSH_IMPORT.md を参照。

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use egui::Vec2;
use yolu_io::brushes::clipstudio as io;
use yolu_io::brushes::clipstudio::{Missing, Peek, Places};
use yolu_io::brushes::BrushImportError;

use crate::jobs::{JobSpec, Polled, Worker};
use crate::state::{AppState, DialogRequest};

/// 一覧の 1 行の中身（見本まで読めたか）。
#[derive(Debug)]
pub enum RowState {
    /// まだ読んでいない。
    Pending,
    Ready(Peek),
    /// 読めなかった（取り込みの対象にしない）。
    Failed(BrushImportError),
}

/// 一覧の 1 行（見つけた `.sut` 1 つ）。
pub struct Row {
    pub path: PathBuf,
    /// 探し始めのフォルダからの相対の表示（フォルダの区切りは `/`）。
    pub shown: String,
    pub size: u64,
    pub state: RowState,
    pub selected: bool,
    /// 筆先の見本の絵（描くときに作る。窓を閉じると捨てる）。
    pub texture: Option<egui::TextureHandle>,
}

/// 探した結果。
pub struct Listing {
    /// 実際に開けて探したフォルダ。
    pub searched: Vec<PathBuf>,
    /// 上限に達して見ていない所が残った。
    pub truncated: bool,
    /// 1 つも見つからなかった理由。
    pub missing: Option<Missing>,
    pub rows: Vec<Row>,
}

impl Listing {
    /// 見本まで読み終えた行の数。
    pub fn peeked(&self) -> usize {
        self.rows
            .iter()
            .filter(|r| !matches!(r.state, RowState::Pending))
            .count()
    }

    /// 取り込める（読めた）行の数。
    pub fn ready(&self) -> usize {
        self.rows
            .iter()
            .filter(|r| matches!(r.state, RowState::Ready(_)))
            .count()
    }

    /// 選んでいる（読めた）行の数。
    pub fn selected(&self) -> usize {
        self.rows
            .iter()
            .filter(|r| r.selected && matches!(r.state, RowState::Ready(_)))
            .count()
    }
}

enum Source {
    /// 既定の場所（CELSYS の設定のフォルダ）。
    Default(Places),
    /// 手で選んだフォルダ。
    Folder(PathBuf),
}

enum Msg {
    Scan(io::Scan),
    Peeked(usize, Result<Peek, BrushImportError>),
    Done,
}

/// 「CLIP STUDIO から」の窓の状態。
#[derive(Default)]
pub struct CspState {
    pub open: bool,
    /// 窓の位置（見出しのドラッグでずれた量）と、一覧のずらした量。
    pub offset: Vec2,
    pub scroll: f32,
    /// 手で選んだフォルダ（None は既定の場所）。
    pub folder: Option<PathBuf>,
    pub listing: Option<Listing>,
    /// 走っている探す仕事（受け口を捨てると取り消す）。
    job: Option<Worker<Msg>>,
    /// 探し始めのフォルダ（相対の表示を作る）。
    base: Vec<PathBuf>,
    /// 試験用: 既定の場所の代わりに使う環境の手がかり（実機の環境変数ではなく、試験用の一時フォルダを探す）。
    #[doc(hidden)]
    pub places: Option<Places>,
    /// 試験用: 次の仕事を、取消が来るまで始めずに止めておく。
    #[doc(hidden)]
    pub park_next: bool,
}

impl CspState {
    /// 探している途中か（見本を読んでいる間も）。
    pub fn is_busy(&self) -> bool {
        self.job.is_some()
    }

    /// 走っている仕事を取り消す（結果は残す）。
    fn cancel(&mut self) {
        self.job = None;
    }
}

/// 「CLIP STUDIO から」の探す仕事（描き直すだけ。読むだけなので止めない）。窓はキーの割り当てを止める。
pub(crate) const JOB: JobSpec = JobSpec {
    repaint: true,
    modal: Some(|app| app.brushes.csp.open),
    ..JobSpec::new("brushes.csp", |app| app.brushes.csp.is_busy())
};

fn shown_path(base: &[PathBuf], path: &std::path::Path) -> String {
    let relative = base
        .iter()
        .find_map(|b| path.strip_prefix(b).ok())
        .unwrap_or(path);
    relative
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

fn run(source: Source, cancel: &AtomicBool, tx: &std::sync::mpsc::Sender<Msg>, park: bool) {
    if park {
        crate::windows::park_until_canceled(cancel);
    }
    let scan = match source {
        Source::Default(places) => io::find(&places),
        Source::Folder(dir) => io::scan(std::slice::from_ref(&dir), io::Limits::default()),
    };
    let files: Vec<PathBuf> = scan.files.iter().map(|f| f.path.clone()).collect();
    if tx.send(Msg::Scan(scan)).is_err() {
        return;
    }
    for (i, path) in files.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        if tx.send(Msg::Peeked(i, io::peek(path))).is_err() {
            return;
        }
    }
    let _ = tx.send(Msg::Done);
}

impl AppState {
    /// 窓を開いて、既定の場所を探す。
    pub(super) fn brush_csp_open(&mut self) {
        self.brushes.csp.open = true;
        self.brushes.csp.folder = None;
        self.brush_csp_scan();
    }

    /// 窓を閉じる（走っている仕事は取り消し、一覧は捨てる）。
    pub(super) fn brush_csp_close(&mut self) {
        let csp = &mut self.brushes.csp;
        csp.open = false;
        csp.cancel();
        csp.listing = None;
    }

    /// 手で選んだフォルダを探す。
    pub(super) fn brush_csp_folder(&mut self, folder: PathBuf) {
        self.brushes.csp.folder = Some(folder);
        self.brush_csp_scan();
    }

    /// 探し直す（手で選んだフォルダがあればそこを、無ければ既定の場所を）。
    pub(super) fn brush_csp_scan(&mut self) {
        let csp = &mut self.brushes.csp;
        csp.cancel();
        let source = match &csp.folder {
            Some(dir) => Source::Folder(dir.clone()),
            None => Source::Default(csp.places.clone().unwrap_or_else(Places::from_env)),
        };
        csp.base = match &source {
            Source::Default(places) => places.celsys_folders(),
            Source::Folder(dir) => vec![dir.clone()],
        };
        csp.listing = None;
        let park = std::mem::take(&mut csp.park_next);
        let spawned = Worker::spawn("yolu-brush-csp", move |tx, cancel| {
            run(source, cancel.flag(), &tx, park)
        });
        match spawned {
            Ok(worker) => csp.job = Some(worker.cancel_on_drop()),
            Err(e) => self.message = e.to_string(),
        }
    }

    /// 別のスレッドの探す仕事から届いたものを受ける（フレームの初めに）。
    pub fn poll_brush_csp(&mut self) {
        let csp = &mut self.brushes.csp;
        loop {
            let Some(job) = &csp.job else {
                return;
            };
            let msg = match job.poll() {
                Polled::Message(m) => m,
                Polled::Empty => return,
                Polled::Lost => Msg::Done,
            };
            match msg {
                Msg::Scan(scan) => {
                    let base = csp.base.clone();
                    csp.listing = Some(Listing {
                        searched: scan.searched,
                        truncated: scan.truncated,
                        missing: scan.missing,
                        rows: scan
                            .files
                            .into_iter()
                            .map(|f| Row {
                                shown: shown_path(&base, &f.path),
                                path: f.path,
                                size: f.size,
                                state: RowState::Pending,
                                selected: false,
                                texture: None,
                            })
                            .collect(),
                    });
                }
                Msg::Peeked(i, result) => {
                    if let Some(row) = csp.listing.as_mut().and_then(|l| l.rows.get_mut(i)) {
                        row.state = match result {
                            Ok(peek) => RowState::Ready(peek),
                            Err(e) => RowState::Failed(e),
                        };
                    }
                }
                Msg::Done => {
                    csp.job = None;
                    return;
                }
            }
        }
    }

    /// 行の選びを反転する（読めなかった行は選べない）。
    pub(super) fn brush_csp_toggle(&mut self, index: usize) {
        if let Some(row) = self
            .brushes
            .csp
            .listing
            .as_mut()
            .and_then(|l| l.rows.get_mut(index))
        {
            if matches!(row.state, RowState::Ready(_)) {
                row.selected = !row.selected;
            }
        }
    }

    /// 読めた行を全部選ぶ・全部外す。
    pub(super) fn brush_csp_select_all(&mut self, on: bool) {
        if let Some(listing) = self.brushes.csp.listing.as_mut() {
            for row in &mut listing.rows {
                row.selected = on && matches!(row.state, RowState::Ready(_));
            }
        }
    }

    /// 選んだ行だけを取り込み、窓を閉じる（取り込みは裏で進み、結果は状態の帯に出る）。何も選んでいなければ何もしない。
    pub(super) fn brush_csp_import(&mut self) {
        let paths: Vec<PathBuf> = self
            .brushes
            .csp
            .listing
            .as_ref()
            .map(|l| {
                l.rows
                    .iter()
                    .filter(|r| r.selected && matches!(r.state, RowState::Ready(_)))
                    .map(|r| r.path.clone())
                    .collect()
            })
            .unwrap_or_default();
        if paths.is_empty() {
            return;
        }
        if self.brushes.import.is_busy() {
            self.brush_refuse_while_importing();
            return;
        }
        self.brush_csp_close();
        self.brush_import_start(paths);
    }

    /// フォルダを手で選ぶ窓を頼む。
    pub(super) fn brush_csp_pick_folder(&mut self) {
        self.dialog_request = Some(DialogRequest::ClipStudioFolder);
    }
}
