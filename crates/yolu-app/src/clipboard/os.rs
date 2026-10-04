//! OS のクリップボードの画像との口。アプリの中のクリップボード（core の `PixelClipboard`）が基本で、ここは外の画像を貼る・
//! コピーした画素を外へ出すためだけに使う。画面の側は [`OsClipboard`] だけを見るので、試験は [`MemoryClipboard`] に差し替える。
//!
//! 本物（[`SystemClipboard`]）は専用のスレッドが `arboard::Clipboard` を 1 つ持ち続ける（X11 は持ち主のプロセスが生きている間だけ
//! 中身を渡せるので、書くたびに作って捨てない。画像を PNG にする時間で画面を止めないよう、書くのは待たない）。画像は straight
//! RGBA8・上の行から（OS の形）。Linux は Wayland の data-control を先に試し、使えなければ X11 に戻る（arboard の選び方）。
//! 読むのは答えを待つが、持ち主が答えないとき（arboard の X11 は 4 秒待つ）に画面を止め続けないよう、待つ長さを区切る。

use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::lang::Lang;

/// OS のクリップボードの画像（straight RGBA8、行は上から）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl ClipImage {
    /// 幅 × 高さ × 4 バイトの画像。大きさとバイト数が合わなければ None。
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Option<ClipImage> {
        (width > 0 && height > 0 && rgba.len() as u64 == width as u64 * height as u64 * 4)
            .then_some(ClipImage {
                width,
                height,
                rgba,
            })
    }
}

/// OS のクリップボードを使えなかった理由。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OsClipboardError {
    /// OS のクリップボードに届かない（画面が無い・プロトコルが使えない）。
    Unavailable,
    /// 別のアプリが使っている。
    Busy,
    /// 画像を読み書きできる形にできない。
    Unreadable,
    /// 持ち主が答えない（読み出しが時間切れ）。
    NotResponding,
}

impl OsClipboardError {
    pub fn text(self, lang: Lang) -> &'static str {
        match self {
            OsClipboardError::Unavailable => lang.pick(
                "OS のクリップボードを使えません",
                "OS clipboard unavailable",
            ),
            OsClipboardError::Busy => {
                lang.pick("OS のクリップボードが使用中です", "OS clipboard is busy")
            }
            OsClipboardError::Unreadable => lang.pick(
                "OS のクリップボードの画像を扱えません",
                "Cannot convert the OS clipboard image",
            ),
            OsClipboardError::NotResponding => lang.pick(
                "OS のクリップボードが応答しません",
                "OS clipboard is not responding",
            ),
        }
    }
}

/// OS のクリップボードの画像への口。
pub trait OsClipboard {
    /// 今の画像（無ければ `Ok(None)`）。書きかけの `write_image` があれば、その後の中身を返す。
    fn read_image(&mut self) -> Result<Option<ClipImage>, OsClipboardError>;
    /// 画像を書く。待たずに返し、書けなかったときは [`OsClipboard::take_error`] で後から知らせる。
    fn write_image(&mut self, image: ClipImage);
    /// 書けなかった理由（あれば 1 つ取り出す）。
    fn take_error(&mut self) -> Option<OsClipboardError>;
}

/// OS のクリップボードに繋がない（既定。試験と、画面の無い場面）。
#[derive(Default)]
pub struct NoClipboard;

impl OsClipboard for NoClipboard {
    fn read_image(&mut self) -> Result<Option<ClipImage>, OsClipboardError> {
        Ok(None)
    }
    fn write_image(&mut self, _image: ClipImage) {}
    fn take_error(&mut self) -> Option<OsClipboardError> {
        None
    }
}

/// メモリの上の OS のクリップボード（試験用）。複製は同じ中身を指すので、アプリに渡した後も外の画像を置く・書かれた画像を見る・
/// 失敗を起こすことができる。
#[derive(Clone, Default)]
pub struct MemoryClipboard(Arc<Mutex<MemoryState>>);

#[derive(Default)]
struct MemoryState {
    image: Option<ClipImage>,
    writes: usize,
    reads: usize,
    fail_write: Option<OsClipboardError>,
    fail_read: Option<OsClipboardError>,
    error: Option<OsClipboardError>,
}

impl MemoryClipboard {
    pub fn new() -> MemoryClipboard {
        MemoryClipboard::default()
    }
    /// 外のアプリがコピーしたことにする（None で、画像が無い状態）。
    pub fn set(&self, image: Option<ClipImage>) {
        self.0.lock().expect("試験用の中身").image = image;
    }
    /// 今の中身。
    pub fn get(&self) -> Option<ClipImage> {
        self.0.lock().expect("試験用の中身").image.clone()
    }
    /// 書かれた回数・読まれた回数。
    pub fn counts(&self) -> (usize, usize) {
        let s = self.0.lock().expect("試験用の中身");
        (s.writes, s.reads)
    }
    /// 次からの書き込み（読み出し）を失敗させる（None で戻す）。
    pub fn fail_writes(&self, error: Option<OsClipboardError>) {
        self.0.lock().expect("試験用の中身").fail_write = error;
    }
    pub fn fail_reads(&self, error: Option<OsClipboardError>) {
        self.0.lock().expect("試験用の中身").fail_read = error;
    }
}

impl OsClipboard for MemoryClipboard {
    fn read_image(&mut self) -> Result<Option<ClipImage>, OsClipboardError> {
        let mut s = self.0.lock().expect("試験用の中身");
        s.reads += 1;
        match s.fail_read {
            Some(e) => Err(e),
            None => Ok(s.image.clone()),
        }
    }
    fn write_image(&mut self, image: ClipImage) {
        let mut s = self.0.lock().expect("試験用の中身");
        s.writes += 1;
        match s.fail_write {
            Some(e) => s.error = Some(e),
            None => s.image = Some(image),
        }
    }
    fn take_error(&mut self) -> Option<OsClipboardError> {
        self.0.lock().expect("試験用の中身").error.take()
    }
}

/// 読み出しの結果。
type ReadResult = Result<Option<ClipImage>, OsClipboardError>;

/// 読み出しの答えを待つ長さ（画面を止める長さの上限。arboard の X11 は、持ち主が答えないと 4 秒待つ）。
const READ_TIMEOUT: Duration = Duration::from_millis(1500);

/// 本物の OS のクリップボード。最初に使うときにスレッドを起こす。
#[derive(Default)]
pub struct SystemClipboard {
    worker: Option<Worker>,
    reads: Reads,
}

enum Job {
    Write(ClipImage),
    Read(Sender<ReadResult>),
    Quit,
}

/// 作業スレッドへの読み出しの頼みと、その答えの待ち。待つ長さを区切り、時間切れの読み出しがまだ続いているあいだは新しい頼みを
/// 積まない（答えない持ち主に、貼るたびに読み出しが溜まらない）。
#[derive(Default)]
struct Reads {
    pending: Option<Receiver<ReadResult>>,
}

impl Reads {
    fn ask(&mut self, jobs: &Sender<Job>, timeout: Duration) -> ReadResult {
        let answer = match self.pending.take() {
            // 前の読み出しがまだ続いている: 新しく頼まず、その答えをもう一度待つ
            Some(answer) if matches!(answer.try_recv(), Err(TryRecvError::Empty)) => answer,
            // 無い・もう終わっていた（遅れて来た答えは古いので捨てる）: 新しく頼む
            _ => {
                let (reply, answer) = channel();
                jobs.send(Job::Read(reply))
                    .map_err(|_| OsClipboardError::Unavailable)?;
                answer
            }
        };
        match answer.recv_timeout(timeout) {
            Ok(result) => result,
            Err(RecvTimeoutError::Timeout) => {
                self.pending = Some(answer);
                Err(OsClipboardError::NotResponding)
            }
            Err(RecvTimeoutError::Disconnected) => Err(OsClipboardError::Unavailable),
        }
    }
}

struct Worker {
    jobs: Sender<Job>,
    errors: Receiver<OsClipboardError>,
    done: Receiver<()>,
    thread: Option<JoinHandle<()>>,
}

impl SystemClipboard {
    pub fn new() -> SystemClipboard {
        SystemClipboard::default()
    }

    fn worker(&mut self) -> &mut Worker {
        self.worker.get_or_insert_with(Worker::start)
    }
}

impl OsClipboard for SystemClipboard {
    fn read_image(&mut self) -> ReadResult {
        let jobs = self.worker().jobs.clone();
        self.reads.ask(&jobs, READ_TIMEOUT)
    }
    fn write_image(&mut self, image: ClipImage) {
        let _ = self.worker().jobs.send(Job::Write(image));
    }
    fn take_error(&mut self) -> Option<OsClipboardError> {
        self.worker.as_ref()?.errors.try_recv().ok()
    }
}

impl Worker {
    fn start() -> Worker {
        let (jobs, queue) = channel();
        let (errors_tx, errors) = channel();
        let (done_tx, done) = channel();
        let thread = std::thread::Builder::new()
            .name("yolu-os-clipboard".into())
            .spawn(move || {
                serve(queue, errors_tx);
                let _ = done_tx.send(());
            })
            .ok();
        Worker {
            jobs,
            errors,
            done,
            thread,
        }
    }
}

impl Drop for SystemClipboard {
    /// 終わるときは、書いた画像が X11 のクリップボードの管理に渡るよう、スレッドが `Clipboard` を手放すのを少しだけ待つ
    /// （詰まっていても終了を止めない）。
    fn drop(&mut self) {
        if let Some(mut worker) = self.worker.take() {
            let _ = worker.jobs.send(Job::Quit);
            if worker.done.recv_timeout(Duration::from_millis(800)).is_ok() {
                if let Some(thread) = worker.thread.take() {
                    let _ = thread.join();
                }
            }
        }
    }
}

/// スレッドの本体: 仕事を順に受け、`arboard::Clipboard` は最初に要るときに作る（作れなければ、次の仕事でまた試す）。
fn serve(queue: Receiver<Job>, errors: Sender<OsClipboardError>) {
    let mut clipboard: Option<arboard::Clipboard> = None;
    let open = |clipboard: &mut Option<arboard::Clipboard>| -> Result<(), OsClipboardError> {
        if clipboard.is_none() {
            *clipboard = Some(arboard::Clipboard::new().map_err(classify)?);
        }
        Ok(())
    };
    while let Ok(job) = queue.recv() {
        match job {
            Job::Quit => break,
            Job::Write(image) => {
                let result = open(&mut clipboard).and_then(|()| {
                    let data = arboard::ImageData {
                        width: image.width as usize,
                        height: image.height as usize,
                        bytes: std::borrow::Cow::Owned(image.rgba),
                    };
                    clipboard
                        .as_mut()
                        .expect("開けた")
                        .set_image(data)
                        .map_err(classify)
                });
                if let Err(error) = result {
                    let _ = errors.send(error);
                }
            }
            Job::Read(reply) => {
                let result = open(&mut clipboard).and_then(|()| {
                    match clipboard.as_mut().expect("開けた").get_image() {
                        Ok(data) => ClipImage::new(
                            data.width as u32,
                            data.height as u32,
                            data.bytes.into_owned(),
                        )
                        .map(Some)
                        .ok_or(OsClipboardError::Unreadable),
                        Err(arboard::Error::ContentNotAvailable) => Ok(None),
                        Err(error) => Err(classify(error)),
                    }
                });
                let _ = reply.send(result);
            }
        }
    }
}

fn classify(error: arboard::Error) -> OsClipboardError {
    match error {
        arboard::Error::ClipboardOccupied => OsClipboardError::Busy,
        arboard::Error::ConversionFailure => OsClipboardError::Unreadable,
        _ => OsClipboardError::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    const SHORT: Duration = Duration::from_millis(30);

    fn read_job(queue: &Receiver<Job>) -> Sender<ReadResult> {
        match queue.try_recv() {
            Ok(Job::Read(reply)) => reply,
            _ => panic!("読み出しの頼みが積まれているはず"),
        }
    }

    #[test]
    fn a_read_that_is_answered_returns_the_answer() {
        let (jobs, queue) = channel();
        let image = ClipImage::new(1, 1, vec![1, 2, 3, 4]).unwrap();
        let sent = image.clone();
        let owner = std::thread::spawn(move || {
            if let Ok(Job::Read(reply)) = queue.recv() {
                let _ = reply.send(Ok(Some(sent)));
            }
        });
        let mut reads = Reads::default();
        assert_eq!(reads.ask(&jobs, Duration::from_secs(10)), Ok(Some(image)));
        owner.join().unwrap();
        assert!(reads.pending.is_none());
    }

    #[test]
    fn a_read_nobody_answers_times_out_and_is_not_asked_again_while_it_lasts() {
        let (jobs, queue) = channel();
        let mut reads = Reads::default();
        let started = Instant::now();
        assert_eq!(
            reads.ask(&jobs, SHORT),
            Err(OsClipboardError::NotResponding)
        );
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "待つ長さは区切られる"
        );
        let first = read_job(&queue);
        // 続いている間に貼り直しても、頼みは積み上がらない
        assert_eq!(
            reads.ask(&jobs, SHORT),
            Err(OsClipboardError::NotResponding)
        );
        assert!(queue.try_recv().is_err(), "2 つ目の頼みは積まない");
        // 続いている頼みに、待っている間に答えが着けば、その答えを受け取る
        let owner = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            first.send(Ok(None)).unwrap();
        });
        assert_eq!(reads.ask(&jobs, Duration::from_secs(10)), Ok(None));
        owner.join().unwrap();
        assert!(queue.try_recv().is_err(), "新しい頼みは積んでいない");
        assert!(reads.pending.is_none());
    }

    #[test]
    fn an_answer_that_arrived_after_the_timeout_is_old_so_the_next_read_asks_afresh() {
        let (jobs, queue) = channel();
        let mut reads = Reads::default();
        assert_eq!(
            reads.ask(&jobs, SHORT),
            Err(OsClipboardError::NotResponding)
        );
        let first = read_job(&queue);
        first
            .send(Ok(Some(ClipImage::new(1, 1, vec![9, 9, 9, 9]).unwrap())))
            .unwrap();
        // 待ち終わったあとに着いた答えは使わず、新しく頼む
        assert_eq!(
            reads.ask(&jobs, SHORT),
            Err(OsClipboardError::NotResponding)
        );
        let _second = read_job(&queue);
    }

    #[test]
    fn a_read_with_no_worker_left_is_unavailable() {
        let (jobs, queue) = channel();
        drop(queue);
        assert_eq!(
            Reads::default().ask(&jobs, SHORT),
            Err(OsClipboardError::Unavailable)
        );
    }
}
