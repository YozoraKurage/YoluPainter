//! ローカルの診断記録。保存前に伏せ字にし、送信は行わない。
use std::{
    cell::Cell,
    collections::VecDeque,
    fs::{self, File, OpenOptions},
    hash::{Hash, Hasher},
    io::{Read, Write},
    panic::UnwindSafe,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

mod native;
mod oom;
pub mod window;
pub use native::cleanup;
pub use oom::RecordingAlloc;
#[cfg(test)]
mod robust_tests;
#[cfg(test)]
mod tests;

pub const MAX_BYTES: usize = 1024 * 1024;
const CRASH_KEEP: usize = 20;
const LOG_KEEP: usize = 5;
/// 直前の操作の種類名を残す数（同じ種類の連続は 1 件にまとめる）。
const ACTION_KEEP: usize = 30;
/// 失敗・断りの文として覚えておく数（画面の描画で何度も作られる同じ文は 1 件）。
const PROBLEM_KEEP: usize = 256;
/// 同じ panic の記録をまとめる間。
const PANIC_REPEAT: Duration = Duration::from_secs(10);
static LOGGER: OnceLock<Recorder> = OnceLock::new();

thread_local! {
    /// この中で起きた panic は、呼び出し側が受け止めて動き続ける（落ちではない）。
    static HANDLED: Cell<u32> = const { Cell::new(0) };
}

#[derive(Default)]
struct Context {
    gpu: String,
    /// 直前の操作の種類名と連続した回数。
    actions: VecDeque<(&'static str, u32)>,
    /// 最後に見た画面の文（生のまま。毎フレーム比べるので、伏せ字より先に比べる）。
    last_message: String,
    /// 最後に書いた知らせ（種類・出どころ・生の文。同じ知らせが続いたら 1 行だけ書く）。
    last_notice: String,
    /// 直前に記録した panic の見出しと時刻。
    last_panic: Option<(String, Instant)>,
}

pub struct Recorder {
    dir: PathBuf,
    redactor: Redactor,
    context: Mutex<Context>,
    /// 失敗・断りの文（`problem` が通したもの）の hash。画面の文がこれに当たるときだけ普段のログに書く。
    problems: Mutex<VecDeque<u64>>,
    writes: Mutex<()>,
}

#[derive(Default)]
struct Redactor {
    homes: Vec<String>,
    users: Vec<String>,
}
impl Redactor {
    fn environment() -> Self {
        Self {
            homes: ["HOME", "USERPROFILE"]
                .iter()
                .filter_map(|key| std::env::var(key).ok())
                .filter(|v| !v.is_empty())
                .collect(),
            users: ["USER", "USERNAME"]
                .iter()
                .filter_map(|key| std::env::var(key).ok())
                .filter(|v| !v.is_empty())
                .collect(),
        }
    }
    fn redact(&self, input: &str) -> String {
        let mut text = input.to_owned();
        for home in &self.homes {
            text = replace_case(&text, home, "~");
            text = replace_case(&text, &home.replace('\\', "/"), "~");
            text = replace_case(&text, &home.replace('\\', "\\\\"), "~");
        }
        for user in &self.users {
            // 利用者名が a〜f と数字だけの短い名前（ed・dad・123 など）でも、番地（0x…）の途中は伏せない。番地が壊れると PDB で引けない
            let addresses = hex_words(&text);
            text = replace_case_skipping(&text, user, "[user]", &addresses);
        }
        // 空白入り・引用符入りのファイル名も漏らさないため、対象拡張子がある行は全体を伏せる。
        text.lines()
            .map(|line| {
                let lower = line.to_ascii_lowercase();
                if [
                    ".ylp", ".ylbrush", ".ylsmart", ".fbx", ".psd", ".psb", ".obj", ".gltf",
                    ".glb", ".png", ".jpg", ".jpeg", ".tga", ".exr", ".hdr", ".kra", ".abr",
                    ".blend",
                ]
                .iter()
                .any(|ext| lower.contains(ext))
                {
                    "[file details redacted]"
                } else {
                    line
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn replace_case(text: &str, needle: &str, replacement: &str) -> String {
    replace_case_skipping(text, needle, replacement, &[])
}

/// `skip` の範囲（`text` のバイト位置）に 1 文字でも掛かる一致は、置き換えずに残す。
fn replace_case_skipping(
    text: &str,
    needle: &str,
    replacement: &str,
    skip: &[std::ops::Range<usize>],
) -> String {
    if needle.is_empty() {
        return text.into();
    }
    // ASCII の大小だけを揃えるので、UTF-8 のバイト境界は変わらない。
    let lower = text.to_ascii_lowercase();
    let needle = needle.to_ascii_lowercase();
    let mut out = String::new();
    let mut start = 0;
    while let Some(at) = lower[start..].find(&needle) {
        let at = start + at;
        let end = at + needle.len();
        if skip.iter().any(|range| at < range.end && range.start < end) {
            // 範囲に掛かる一致は残して、次の 1 文字から探し直す（重なった一致も見落とさない）
            let step = text[at..].chars().next().map_or(1, char::len_utf8);
            out.push_str(&text[start..at + step]);
            start = at + step;
            continue;
        }
        out.push_str(&text[start..at]);
        out.push_str(replacement);
        start = end;
    }
    out.push_str(&text[start..]);
    out
}

/// `0x` と 16 進の数字が続く語（番地・基底）の範囲。語の途中の `0x`（`a0x1` など）は数えない。
fn hex_words(text: &str) -> Vec<std::ops::Range<usize>> {
    let bytes = text.as_bytes();
    let mut words = Vec::new();
    let mut at = 0;
    while at + 1 < bytes.len() {
        let word_start =
            at == 0 || !(bytes[at - 1].is_ascii_alphanumeric() || bytes[at - 1] == b'_');
        if word_start && bytes[at] == b'0' && matches!(bytes[at + 1], b'x' | b'X') {
            let digits = bytes[at + 2..]
                .iter()
                .take_while(|b| b.is_ascii_hexdigit())
                .count();
            if digits > 0 {
                words.push(at..at + 2 + digits);
                at += 2 + digits;
                continue;
            }
        }
        at += 1;
    }
    words
}

pub fn directory() -> Option<PathBuf> {
    Some(crate::settings::path()?.parent()?.join("logs"))
}
fn stamp() -> String {
    format!(
        "{:020}-{:010}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        std::process::id()
    )
}
fn bounded(mut text: String) -> String {
    if text.len() > MAX_BYTES {
        let mut end = MAX_BYTES - 32;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push_str("\n[truncated]\n");
    }
    text
}
fn files(dir: &Path, prefix: &str) -> Vec<PathBuf> {
    let mut paths: Vec<_> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(prefix) && n.ends_with(".log"))
        })
        .collect();
    paths.sort_by_cached_key(|path| {
        (
            fs::metadata(path).and_then(|m| m.modified()).ok(),
            path.clone(),
        )
    });
    paths
}
fn is_empty(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|m| m.len() == 0)
}
/// 上限を超えた古い記録を消す。長さ 0 のファイルは数えず、消しもしない（動いている別の起動のネイティブ記録先かもしれず、
/// 持ち主のいないものは `native::sweep` が片付ける）。この起動のネイティブ記録先だけは、落ちたときに中身が入るので数える。
fn prune(dir: &Path, prefix: &str, keep: usize) {
    let paths: Vec<_> = files(dir, prefix)
        .into_iter()
        .filter(|path| native::reserved(path) || !is_empty(path))
        .collect();
    let remove = paths.len().saturating_sub(keep);
    for path in paths
        .into_iter()
        .filter(|path| !native::reserved(path))
        .take(remove)
    {
        let _ = fs::remove_file(path);
    }
}
fn hash(text: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}
/// 文を行の 1 本にする（普段のログは 1 行 1 件）。
fn one_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" | ")
}
impl Recorder {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            redactor: Redactor::environment(),
            context: Mutex::new(Context::default()),
            problems: Mutex::new(VecDeque::new()),
            writes: Mutex::new(()),
        }
    }
    fn header(&self) -> String {
        format!(
            "YoluPainter {}\nGit: {}\nOS: {} {}\n",
            env!("CARGO_PKG_VERSION"),
            option_env!("YOLU_GIT_REV").unwrap_or("unknown"),
            std::env::consts::OS,
            std::env::consts::ARCH
        )
    }
    pub fn record(&self, kind: &str, detail: &str) -> Option<PathBuf> {
        let _writing = self.writes.try_lock().ok()?;
        fs::create_dir_all(&self.dir).ok()?;
        let mut report = format!("{}Kind: {kind}\n", self.header());
        if let Ok(context) = self.context.try_lock() {
            report.push_str(&format!(
                "GPU: {}\nActions: {}\n",
                context.gpu,
                context
                    .actions
                    .iter()
                    .map(|(name, count)| if *count > 1 {
                        format!("{name} x{count}")
                    } else {
                        (*name).to_owned()
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        report.push_str(detail);
        report.push('\n');
        let report = bounded(self.redactor.redact(&report));
        let path = self.dir.join(format!("crash-{}.log", stamp()));
        let result = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .and_then(|mut file| {
                file.write_all(report.as_bytes())?;
                file.sync_data()
            });
        prune(&self.dir, "crash-", CRASH_KEEP);
        result.ok().map(|_| path)
    }
    pub fn action(&self, name: &'static str) {
        if let Ok(mut context) = self.context.try_lock() {
            if let Some((last, count)) = context.actions.back_mut() {
                if *last == name {
                    *count = count.saturating_add(1);
                    return;
                }
            }
            if context.actions.len() == ACTION_KEEP {
                context.actions.pop_front();
            }
            context.actions.push_back((name, 1));
        }
    }
    /// 失敗・断り・警告の文として覚える（`Lang` の失敗の文の作り手が通す）。画面の文がこれに当たるときだけ書く。
    pub fn note_problem(&self, text: &str) {
        if text.is_empty() {
            return;
        }
        let key = hash(text);
        if let Ok(mut problems) = self.problems.try_lock() {
            if !problems.contains(&key) {
                if problems.len() == PROBLEM_KEEP {
                    problems.pop_front();
                }
                problems.push_back(key);
            }
        }
    }
    /// 画面の文のうち、失敗・断りの文として覚えた部分（文全体、または「名前: 理由」の理由のような後ろの部分）。
    /// 制作物の名前などが前に付いた文は、名前の付かない後ろの部分だけを返す。
    fn problem_in<'a>(&self, message: &'a str) -> Option<&'a str> {
        let problems = self.problems.try_lock().ok()?;
        if problems.is_empty() {
            return None;
        }
        let known = |text: &str| !text.is_empty() && problems.contains(&hash(text));
        if known(message) {
            return Some(message);
        }
        // 長い文は全体だけを見る（後ろの部分の総当たりを避ける）。
        if message.len() > 2048 {
            return None;
        }
        message
            .char_indices()
            .filter(|(_, c)| c.is_whitespace() || *c == '：')
            .map(|(at, c)| &message[at + c.len_utf8()..])
            .find(|rest| known(rest.trim_start()))
            .map(str::trim_start)
    }
    /// 画面の文のうち、失敗・断り・警告だけを回転するログへ書く。毎フレーム呼ばれるので、生の文が前と同じなら何もしない。
    pub fn message(&self, message: &str) {
        if message.is_empty() {
            return;
        }
        {
            let Ok(mut context) = self.context.try_lock() else {
                return;
            };
            if context.last_message == message {
                return;
            }
            context.last_message.clear();
            context.last_message.push_str(message);
        }
        if let Some(problem) = self.problem_in(message) {
            self.write_line(&one_line(problem));
        }
    }
    /// 注意・失敗の知らせを 1 行書く（`notice` の `notify` が呼ぶ）。種類と出どころの名前（言語によらない）に、文のうち失敗の文として
    /// 覚えた部分（名前の付かない理由）だけを添える。覚えた部分が無い文は、種類と出どころだけ（名前やパスを書かない決まりのまま）。
    /// 同じ知らせが続いたら、2 つ目からは書かない。
    pub fn notice(&self, kind: &str, source: &str, text: &str) {
        {
            let Ok(mut context) = self.context.try_lock() else {
                return;
            };
            let key = format!("{kind}\u{1f}{source}\u{1f}{text}");
            if context.last_notice == key {
                return;
            }
            context.last_notice = key;
        }
        match self.problem_in(text) {
            Some(problem) => self.write_line(&format!("{kind} {source}: {}", one_line(problem))),
            None => self.write_line(&format!("{kind} {source}")),
        }
    }
    fn write_line(&self, text: &str) {
        let text = bounded(self.redactor.redact(text));
        let Ok(_writing) = self.writes.try_lock() else {
            return;
        };
        if fs::create_dir_all(&self.dir).is_err() {
            return;
        }
        let mut path = files(&self.dir, "session-")
            .pop()
            .unwrap_or_else(|| self.dir.join(format!("session-{}.log", stamp())));
        let line = bounded(format!("{} {text}\n", stamp()));
        if fs::metadata(&path).is_ok_and(|m| m.len() + line.len() as u64 > MAX_BYTES as u64) {
            path = self.dir.join(format!("session-{}.log", stamp()));
        }
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
            let _ = file.write_all(line.as_bytes());
        }
        prune(&self.dir, "session-", LOG_KEEP);
    }
    fn panic(&self, info: &std::panic::PanicHookInfo<'_>) {
        let summary = info.to_string();
        if HANDLED.with(Cell::get) > 0 {
            // 受け止めて動き続ける panic は落ちではない。印も落ちた記録の枠も使わず、1 行だけ普段のログへ書く。
            self.write_line(&format!("Handled panic: {}", one_line(&summary)));
            return;
        }
        if let Ok(mut context) = self.context.try_lock() {
            let now = Instant::now();
            match &context.last_panic {
                Some((last, at)) if *last == summary && now.duration_since(*at) < PANIC_REPEAT => {
                    return;
                }
                _ => context.last_panic = Some((summary.clone(), now)),
            }
        }
        let thread = std::thread::current();
        self.record(
            "Rust panic",
            &format!(
                "Thread: {}\n{summary}\nImage base: {}\nBacktrace:\n{:#}",
                thread.name().unwrap_or("unnamed"),
                image_base_text(),
                // `{:#}` は各フレームの命令の番地を書く形（`{}` の短い形は、記号が解決できないと `<unknown>` だけで番地が残らない）。
                // 配布物には PDB が無いので、記号の解決は書いた側ではできない。番地から実行ファイルの基底（上の `Image base`）を引いた
                // 相対の番地を、同じ版の PDB（リリースの付属物）で引く。
                std::backtrace::Backtrace::force_capture()
            ),
        );
    }
}

/// 実行ファイルが読み込まれた基底の番地。落ちた記録の各フレームの番地から引くと、PDB・シンボルファイルの中の相対の番地になる
/// （アドレス空間のランダム化で、起動のたびに基底が違う）。Windows は `GetModuleHandleW(NULL)`、Linux は主プログラムの読み込みの差分
/// （`dl_iterate_phdr` の最初）、Mac は最初のイメージの先頭。分からなければ None。
pub(crate) fn image_base() -> Option<usize> {
    #[cfg(windows)]
    {
        // SAFETY: 引数 NULL は自分の実行ファイルのハンドルを返すだけ（参照は増やさない）。
        unsafe { windows::Win32::System::LibraryLoader::GetModuleHandleW(None) }
            .ok()
            .map(|module| module.0 as usize)
            .filter(|base| *base != 0)
    }
    #[cfg(target_os = "linux")]
    {
        unsafe extern "C" fn first(
            info: *mut libc::dl_phdr_info,
            _size: libc::size_t,
            data: *mut libc::c_void,
        ) -> libc::c_int {
            // SAFETY: ローダーが渡す構造体と、呼び出し側が渡した usize の書き込み先。
            unsafe { *data.cast::<usize>() = (*info).dlpi_addr as usize };
            1 // 最初（主プログラム）だけで止める
        }
        let mut base = 0usize;
        // SAFETY: コールバックはこの呼び出しの間だけ、base への書き込みに使う。
        unsafe { libc::dl_iterate_phdr(Some(first), std::ptr::from_mut(&mut base).cast()) };
        Some(base)
    }
    #[cfg(target_os = "macos")]
    {
        extern "C" {
            fn _dyld_get_image_header(image_index: u32) -> *const std::ffi::c_void;
        }
        // SAFETY: 番号 0 は主の実行ファイル。ヘッダーの番地を返すだけで、読み書きはしない。
        let header = unsafe { _dyld_get_image_header(0) } as usize;
        (header != 0).then_some(header)
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        None
    }
}

fn image_base_text() -> String {
    image_base().map_or_else(|| "unknown".to_owned(), |base| format!("0x{base:x}"))
}

/// main の最初で一度だけ呼ぶ。既存の hook（標準の表示を含む）は必ず引き継ぐ。
pub fn install() {
    let Some(dir) = directory() else { return };
    install_at(dir);
}
pub(crate) fn install_at(dir: PathBuf) {
    if LOGGER.set(Recorder::new(dir)).is_err() {
        return;
    }
    let recorder = LOGGER.get().unwrap();
    let _ = fs::create_dir_all(&recorder.dir);
    prune(&recorder.dir, "crash-", CRASH_KEEP);
    prune(&recorder.dir, "session-", LOG_KEEP);
    native::install(&recorder.dir, &recorder.header());
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        recorder.panic(info);
        previous(info);
    }));
}
pub fn action(name: &'static str) {
    if let Some(r) = LOGGER.get() {
        r.action(name);
    }
}
pub fn message(text: &str) {
    if let Some(r) = LOGGER.get() {
        r.message(text);
    }
}
/// 注意・失敗の知らせを普段のログへ 1 行書く（`Recorder::notice`）。
pub fn notice(kind: &str, source: &str, text: &str) {
    if let Some(r) = LOGGER.get() {
        r.notice(kind, source, text);
    }
}
/// 普段のログへ診断の 1 行を書く（伏せ字にして、同じ回転のログへ。失敗の文として覚える必要のない、起きたことの記録）。
pub fn note(text: &str) {
    if let Some(r) = LOGGER.get() {
        r.write_line(&one_line(text));
    }
}
/// 落ちた記録の枠（`crash-*.log`）へ、panic ではない出来事（GPU の装置を失った、など）を書く。次の起動で報告の印が出る。
pub fn event(kind: &str, detail: &str) -> Option<PathBuf> {
    LOGGER.get()?.record(kind, detail)
}
/// 失敗・断り・警告の文を作ったときに通す（文はそのまま返す）。画面に出た文がこれに当たるときだけ、普段のログへ書く。
pub fn problem<T: AsRef<str>>(text: T) -> T {
    if let Some(r) = LOGGER.get() {
        r.note_problem(text.as_ref());
    }
    text
}
/// `catch_unwind` の代わり。受け止めて動き続ける panic を、落ちた記録にしない（復旧の書き手・見本の描画）。
pub fn handled<R>(work: impl FnOnce() -> R + UnwindSafe) -> std::thread::Result<R> {
    HANDLED.with(|depth| depth.set(depth.get() + 1));
    let result = std::panic::catch_unwind(work);
    HANDLED.with(|depth| depth.set(depth.get() - 1));
    result
}
/// 起動の本体を守る。panic で止まったら、ウィンドウの知らせを出し、空のネイティブ記録先を片付けてから panic を渡し直す。
pub fn guard<R>(start: impl FnOnce() -> R + UnwindSafe, on_panic: impl FnOnce()) -> R {
    match std::panic::catch_unwind(start) {
        Ok(result) => result,
        Err(payload) => {
            on_panic();
            cleanup();
            std::panic::resume_unwind(payload)
        }
    }
}
pub fn gpu(name: &str, backend: &str) {
    if let Some(r) = LOGGER.get() {
        if let Ok(mut c) = r.context.try_lock() {
            c.gpu = format!("{name} ({backend})");
        }
    }
}
pub fn record_startup_failure(recorder: &Recorder, reason: &str) -> Option<PathBuf> {
    recorder.record("Startup failure", reason)
}
pub fn startup_failure(reason: &str) {
    if let Some(r) = LOGGER.get() {
        record_startup_failure(r, reason);
    }
    failure_dialog_with(Some(reason));
}

/// panic hook の記録を上書きせず、ウィンドウが無い場合にも短い理由を見せる。
pub fn failure_dialog() {
    failure_dialog_with(None);
}

/// ウィンドウのボタンの文言を OS が受け付けるか。Windows の `MessageBoxW` は OK とキャンセルだけで、文言は捨てられる。
const LABELLED_BUTTONS: bool = cfg!(not(windows));

fn failure_dialog_with(reason: Option<&str>) {
    // アプリの起動と同じ決め方（設定に言語が無いときは OS の言語）
    let system = crate::lang::system_lang();
    let lang = crate::settings::path()
        .map(|p| crate::settings::load_for_startup(&p, system).0.lang)
        .unwrap_or(system);
    let folder = lang.pick("ログのフォルダを開く", "Open Log Folder");
    let result = rfd::MessageDialog::new()
        .set_title("YoluPainter")
        .set_description(dialog_text(lang, reason, LABELLED_BUTTONS))
        .set_level(rfd::MessageLevel::Error)
        .set_buttons(rfd::MessageButtons::OkCancelCustom(
            folder.into(),
            lang.pick("閉じる", "Close").into(),
        ))
        .show();
    if opens_folder(&result, folder) {
        if let Some(dir) = directory() {
            let _ = open_folder(&dir);
        }
    }
}

/// 押されたのが「ログのフォルダを開く」か。文言を出せる OS は文言つきの結果、Windows は OK で返る。
fn opens_folder(result: &rfd::MessageDialogResult, folder: &str) -> bool {
    use rfd::MessageDialogResult as Result;
    match result {
        Result::Ok => true,
        Result::Custom(label) => label == folder,
        _ => false,
    }
}

/// ウィンドウの文。ボタンの文言が出ない OS では、OK が何をするかを 1 行の問いで示す。
fn dialog_text(lang: crate::lang::Lang, reason: Option<&str>, labelled: bool) -> String {
    let reason = short_reason(lang, reason);
    if labelled {
        reason.into()
    } else {
        format!(
            "{reason}\n{}",
            lang.pick("ログのフォルダを開きますか？", "Open the log folder?")
        )
    }
}

pub fn open_folder(path: &Path) -> std::io::Result<()> {
    fs::create_dir_all(path)?;
    #[cfg(windows)]
    let mut command = std::process::Command::new("explorer.exe");
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(all(not(windows), not(target_os = "macos")))]
    let mut command = std::process::Command::new("xdg-open");
    command.arg(path);
    spawn_reaped(&mut command).map(drop)
}

/// 起動して、終わりを別のスレッドで受けておく（ゾンビを残さない。`update::launch::open_page` と同じ）。起動したプロセスの番号を返す。
fn spawn_reaped(command: &mut std::process::Command) -> std::io::Result<u32> {
    use std::process::Stdio;
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let id = child.id();
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(id)
}

pub(crate) fn read_report(path: &Path) -> Option<String> {
    let mut bytes = Vec::new();
    File::open(path)
        .ok()?
        .take(MAX_BYTES as u64)
        .read_to_end(&mut bytes)
        .ok()?;
    (!bytes.is_empty()).then(|| Redactor::environment().redact(&String::from_utf8_lossy(&bytes)))
}

fn short_reason(lang: crate::lang::Lang, reason: Option<&str>) -> &'static str {
    match reason {
        Some(reason)
            if ["gpu", "adapter"]
                .iter()
                .any(|word| reason.to_ascii_lowercase().contains(word)) =>
        {
            lang.pick("GPU を使えません", "GPU unavailable")
        }
        Some(_) => lang.pick("ウィンドウを開けません", "Cannot open the window"),
        None => lang.pick("予期しないエラー", "Unexpected error"),
    }
}
