//! 書き出しのウィンドウ（ファイル → テクスチャを書き出す…）。左の一覧で書き出すテクスチャセットにチェックを入れ、右で出力先・出力テンプレート
//! （今のチャンネルの PNG・チャンネルごとの PNG・Unity Standard / URP Lit・HDRP Lit・lilToon）・パディングを決め、その下の「書き出すファイル」の一覧で、
//! 書く前にファイルの名前と色空間を確かめてから「書き出す」を押すと、それぞれの書き出しの道（`ChannelTo`／`ChannelNamed`・`ChannelsTo`・`TemplateTo`）へ渡す。
//! 書く手順・置き換えの確かめ・取消・結果のウィンドウは、道の先（`export/mod.rs`）が持つ。一覧の求め方は `list`。
//!
//! - 出力テンプレートは設定に覚える（`Settings::export_form`。文書ごとではない）。出力先は、今のチャンネルの PNG ならファイル、ほかはフォルダ。
//!   選んでいない間の既定は、Live Link の相手の文書なら Unity が知らせた置き場、そうでなければ開いたプロジェクトのフォルダ。
//!   選んだ先とセットのチェックは、プロジェクトを替えるまで覚える。
//! - パディングは設定の「書き出しのパディング」と同じ値（このウィンドウから替えると設定も替わる）。ほかに新しい項目は持たない。
//! - 外からの操作（MCP・コマンドライン・オートアクションの再生）は `yolu_ops::export` を通り、このウィンドウを経ない。

use std::path::{Path, PathBuf};

use egui::{pos2, vec2, Id, Key, Rect, Vec2};

use super::list::{FileRow, Preview, SetRow};
use super::{default_channel_file_name, ExportAction};
use crate::dialog::places::Place;
use crate::lang::Lang;
use crate::notice::Source;
use crate::prefs::PrefChoice;
use crate::state::{Action, AppState, DialogRequest, OpenPopup, PopupKind};
use crate::ui::menu::{Entry, PopupState};
use crate::ui::scroll::Scroll;
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, Align};
use crate::ui::window::{self, Spec};

/// ウィンドウの名前（`windows::window_rect` で矩形を引く名前）。
pub const WINDOW: &str = "export";

const WIDTH: f32 = 800.0;
const HEIGHT: f32 = 520.0;
const LEFT_WIDTH: f32 = 230.0;
const ROW: f32 = 24.0;
const GAP: f32 = 8.0;
const MARGIN: f32 = 14.0;
const LABEL_WIDTH: f32 = 128.0;
const FOOTER: f32 = 48.0;
/// 左の一覧・右の一覧の行の高さ。
const SET_ROW: f32 = 26.0;
const FILE_ROW: f32 = 22.0;

/// 出力テンプレート（書き出しのウィンドウの選び。設定のキーは `export_form`）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ExportForm {
    /// 描くチャンネルを 1 枚の PNG に。
    #[default]
    ChannelPng,
    /// 全テクスチャセットの使っている全チャンネルを、フォルダの PNG に。
    AllChannels,
    /// テンプレート「Unity Standard / URP Lit」。
    UnityStandard,
    /// テンプレート「HDRP Lit」。
    Hdrp,
    /// テンプレート「lilToon」。
    LilToon,
}

impl ExportForm {
    /// ドロップダウンに並べる順。
    pub const ALL: [ExportForm; 5] = [
        ExportForm::ChannelPng,
        ExportForm::AllChannels,
        ExportForm::UnityStandard,
        ExportForm::Hdrp,
        ExportForm::LilToon,
    ];

    /// 設定のファイルに書く名前。テンプレートの形は、テンプレートの ID と同じ。
    pub fn key(self) -> &'static str {
        match self {
            ExportForm::ChannelPng => "channel",
            ExportForm::AllChannels => "channels",
            ExportForm::UnityStandard => "unity-standard",
            ExportForm::Hdrp => "unity-hdrp",
            ExportForm::LilToon => "liltoon",
        }
    }

    pub fn from_key(key: &str) -> Option<ExportForm> {
        ExportForm::ALL.into_iter().find(|f| f.key() == key)
    }

    /// テンプレートの形なら、そのテンプレートの ID。
    pub fn template_id(self) -> Option<&'static str> {
        match self {
            ExportForm::UnityStandard | ExportForm::Hdrp | ExportForm::LilToon => Some(self.key()),
            ExportForm::ChannelPng | ExportForm::AllChannels => None,
        }
    }

    /// 書き出す先がファイルか（でなければフォルダ）。
    pub fn writes_file(self) -> bool {
        self == ExportForm::ChannelPng
    }

    pub fn name(self, lang: Lang) -> &'static str {
        match self {
            ExportForm::ChannelPng => lang.pick("今のチャンネル", "Current Channel"),
            ExportForm::AllChannels => lang.pick("チャンネルごと", "Per Channel"),
            ExportForm::UnityStandard => "Unity Standard / URP Lit",
            ExportForm::Hdrp => "HDRP Lit",
            ExportForm::LilToon => "lilToon",
        }
    }
}

/// ウィンドウの状態（開いているか・動かした量・選んだ出力先・外したセット）。アプリの状態で、.ylp には入れない。
#[derive(Debug, Default)]
pub struct WindowState {
    pub open: bool,
    pub offset: Vec2,
    /// 選んだ PNG のファイル。名前が既定の名前（今のセット・チャンネルの名前）のときは持たず、フォルダだけ覚える
    /// （チャンネルやセットを替えても、名前が追いかける）。
    file: Option<PathBuf>,
    /// 選んだフォルダ（PNG のファイルを選んだときは、その置き場）。
    folder: Option<PathBuf>,
    /// 選ぶウィンドウが「置き換えてよい」と確かめたファイル。このファイルに書くときだけ、置き換えの確かめを重ねない。
    confirmed: Option<PathBuf>,
    /// チェックを外したセットの uid（外していないセットは全部入り。`list` が一覧にする）。
    pub(super) unchecked: Vec<u32>,
    /// 左・右の一覧のスクロール。
    sets_scroll: f32,
    files_scroll: f32,
    /// 書くファイルがもうあるかの調べ（`list`）。
    pub(super) exists: super::list::ExistsCache,
    /// 選んだ先とチェックがどのプロジェクトのものか（`AppState::project_epoch`）。替わったら忘れる。
    epoch: u64,
}

impl AppState {
    /// 今の出力テンプレート（設定に覚えている）。
    pub fn export_form(&self) -> ExportForm {
        self.prefs.settings.export_form
    }

    /// 書き出すセットが 1 つも無いときの理由: 今のチャンネルの 1 枚で今のセットが読むだけなら、その理由、そうでなければ書くセットが無い断り。
    pub(super) fn export_no_set_reason(&self) -> String {
        let lang = self.lang;
        if self.export_form().writes_file() {
            if let Some(reason) = self.sets.current().read_only.as_deref() {
                return crate::lang::refusals::read_only_set(lang, reason);
            }
        }
        super::no_sets_message(lang).into()
    }

    /// 開いたプロジェクトのフォルダ（保存していなければ、またフォルダが無ければ None）。
    fn project_folder(&self) -> Option<PathBuf> {
        self.project
            .as_ref()
            .and_then(|p| p.path().parent())
            .filter(|d| d.is_dir())
            .map(Path::to_path_buf)
    }

    /// 選んだ出力先とチェック。別のプロジェクトで選んだものは、ウィンドウを開いたままプロジェクトを替えられても（新規・開く・Live Link・外からの操作）
    /// 今のプロジェクトのものではないので、無いものとして扱う。
    pub(super) fn export_chosen(&self) -> Option<&WindowState> {
        (self.export.window.epoch == self.project_epoch).then_some(&self.export.window)
    }

    /// 選んだ先が今のプロジェクトのものでなければ忘れて、今のプロジェクトのものにする。
    fn sync_export_window(&mut self) {
        let epoch = self.project_epoch;
        let window = &mut self.export.window;
        if window.epoch != epoch {
            window.file = None;
            window.folder = None;
            window.confirmed = None;
            window.unchecked.clear();
            window.epoch = epoch;
        }
    }

    /// 書き出す先のフォルダ（選んだフォルダ、無ければ Live Link の置き場、無ければプロジェクトのフォルダ）。
    pub fn export_folder(&self) -> Option<PathBuf> {
        self.export_chosen()
            .and_then(|w| w.folder.clone())
            .or_else(|| self.link_export_dir())
            .or_else(|| self.project_folder())
    }

    /// 描くチャンネルの PNG の書き出し先（選んだファイル、無ければ書き出す先のフォルダの既定の名前）。
    pub fn export_file(&self) -> Option<PathBuf> {
        self.export_chosen()
            .and_then(|w| w.file.clone())
            .or_else(|| {
                self.export_folder()
                    .map(|dir| dir.join(default_channel_file_name(self)))
            })
    }

    /// 今の形の書き出し先（PNG ならファイル、ほかはフォルダ）。決まっていなければ None。
    pub fn export_destination(&self) -> Option<PathBuf> {
        if self.export_form().writes_file() {
            self.export_file()
        } else {
            self.export_folder()
        }
    }

    /// ウィンドウの操作（`ExportAction::OpenWindow` ほか）を当てる。
    pub(super) fn export_window_apply(&mut self, action: ExportAction) {
        let lang = self.lang;
        match action {
            ExportAction::OpenWindow => {
                if self.is_stroking() {
                    self.refuse(Source::Export, crate::lang::refusals::during_stroke(lang));
                    return;
                }
                self.sync_export_window();
                self.export.window.open = true;
                self.export.window.exists.invalidate();
            }
            ExportAction::CloseWindow => self.export.window.open = false,
            ExportAction::SetForm(form) => self.prefs.settings.export_form = form,
            ExportAction::SetChecked { uid, on } => {
                self.sync_export_window();
                let unchecked = &mut self.export.window.unchecked;
                unchecked.retain(|u| *u != uid);
                if !on {
                    unchecked.push(uid);
                }
            }
            ExportAction::ChooseDestination => {
                if self.is_stroking() {
                    self.refuse(Source::Export, crate::lang::refusals::during_stroke(lang));
                    return;
                }
                self.dialog_request = Some(DialogRequest::ExportDestination);
            }
            ExportAction::Destination(path) => self.choose_export_destination(path),
            ExportAction::Run => self.run_export_window(),
            _ => unreachable!("ウィンドウの操作ではない"),
        }
    }

    /// 選ぶウィンドウが返した先を覚える。PNG のファイルは拡張子が無ければ `.png` を足す（足した名前は、選ぶウィンドウが確かめていない）。
    fn choose_export_destination(&mut self, chosen: PathBuf) {
        self.sync_export_window();
        if !self.export_form().writes_file() {
            self.note_export_dir(&chosen);
            self.export.window.folder = Some(chosen);
            return;
        }
        let checked = chosen.extension().is_some();
        let path = if checked {
            chosen
        } else {
            chosen.with_extension("png")
        };
        let default_name = default_channel_file_name(self);
        let window = &mut self.export.window;
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            window.folder = Some(dir.to_path_buf());
        }
        let follows = path.file_name().is_some_and(|n| n == default_name.as_str());
        window.file = (!follows).then(|| path.clone());
        window.confirmed = checked.then_some(path);
    }

    /// 「書き出す」: 出力テンプレートに合う書き出しの道へ渡す。始まった（書く仕事が動き出した）ら、ウィンドウを閉じる。
    /// 置き換えの確かめを出したときは開いたままにして、やめたらここへ戻れるようにする。
    fn run_export_window(&mut self) {
        let lang = self.lang;
        let form = self.export_form();
        // 選ぶウィンドウの確かめは、選んだ 1 回の書き出しだけに効く（受け取って使い切る）。2 回目からは、ファイルがあれば確かめる
        let confirmed = (self.export.window.epoch == self.project_epoch)
            .then(|| self.export.window.confirmed.take())
            .flatten();
        let Some(destination) = self.export_destination() else {
            self.refuse(
                Source::Export,
                lang.pick("出力先がありません。", "There is no destination."),
            );
            return;
        };
        let sets = self.export_checked_uids();
        if sets.is_empty() {
            let why = self.export_no_set_reason();
            self.refuse(Source::Export, why);
            return;
        }
        let action = match form.template_id() {
            Some(id) => ExportAction::TemplateTo {
                id: id.to_owned(),
                dir: destination,
                sets: Some(sets),
            },
            None if form.writes_file() => {
                // 選ぶウィンドウが確かめたファイルはそのまま、ほかは書く前にもうあるか確かめる
                if confirmed.as_ref() == Some(&destination) {
                    ExportAction::ChannelTo(destination)
                } else {
                    ExportAction::ChannelNamed(destination)
                }
            }
            None => ExportAction::ChannelsTo {
                dir: destination,
                sets: Some(sets),
            },
        };
        let was_exporting = self.export.is_exporting();
        self.export_apply(action);
        self.close_export_window_if_started(was_exporting);
    }

    /// 書く仕事が（`was_exporting` でなかったのに）動き出していれば、書き出しのウィンドウを閉じる。ほかの書き出しが動いていたための断りでは閉じない。
    pub(super) fn close_export_window_if_started(&mut self, was_exporting: bool) {
        if !was_exporting && self.export.is_exporting() {
            self.export.window.open = false;
        }
    }
}

/// 出力テンプレートのドロップダウンの項目。
pub fn form_entries(app: &AppState) -> Vec<Entry<Action>> {
    let current = app.export_form();
    ExportForm::ALL
        .into_iter()
        .map(|form| {
            Entry::item(
                form.name(app.lang),
                Action::Export(ExportAction::SetForm(form)),
            )
            .radio(form == current)
        })
        .collect()
}

/// `path` のうち、今ある一番近いフォルダ（`path` 自身か、その親をさかのぼって）。選ぶウィンドウの始まりの場所に使う。
/// Live Link の置き場はまだ作っていないことがあり、ウィンドウを取り消しても Unity のプロジェクトに空のフォルダを残さないよう、ここでは作らない。
pub fn nearest_existing_folder(path: &Path) -> Option<PathBuf> {
    path.ancestors().find(|d| d.is_dir()).map(Path::to_path_buf)
}

/// ファイル・フォルダを選ぶウィンドウ（OS）を出し、選んだ先を `ExportAction::Destination` で返す。
/// 始まりの場所は、今の出力先（選んだ先・Live Link の置き場・プロジェクトのフォルダ）が決まっていればそこの今ある一番近いフォルダ、
/// 決まらないとき（保存していない文書）は選ぶウィンドウの既定（前に使った場所 → 文書のフォルダ → 書類）。選んだら場所を覚える。
pub fn run_dialog(state: &mut AppState) {
    let lang = state.lang;
    let destination = state.export_destination();
    let nearest = nearest_existing_folder;
    if state.export_form().writes_file() {
        let mut dialog = crate::dialog::file(state, Place::ImageExport)
            .set_title(lang.pick("チャンネルを PNG に書き出す", "Export the channel as PNG"))
            .add_filter("PNG", &["png"])
            .set_file_name(
                destination
                    .as_deref()
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| default_channel_file_name(state)),
            );
        if let Some(dir) = destination
            .as_deref()
            .and_then(Path::parent)
            .and_then(nearest)
        {
            dialog = dialog.set_directory(dir);
        }
        if let Some(path) = dialog.save_file() {
            dialog_returned(state, path);
        }
    } else {
        let mut dialog = crate::dialog::file(state, Place::ImageExport)
            .set_title(lang.pick("画像を書き出すフォルダ", "Folder for the exported images"));
        if let Some(dir) = destination.as_deref().and_then(nearest) {
            dialog = dialog.set_directory(dir);
        }
        if let Some(dir) = dialog.pick_folder() {
            dialog_returned(state, dir);
        }
    }
}

/// 選ぶウィンドウが返した先を、次に開くときの始まりの場所に覚えてから（PNG のファイルはその置き場、ほかはフォルダ）、`Destination` で渡す。
pub(super) fn dialog_returned(state: &mut AppState, chosen: PathBuf) {
    if state.export_form().writes_file() {
        state.note_file_chosen(Place::ImageExport, &chosen);
    } else {
        state.note_folder_chosen(Place::ImageExport, &chosen);
    }
    state.apply(Action::Export(ExportAction::Destination(chosen)));
}

/// 道を、幅に収まるように前を「…」で詰める（終わりのファイル名・フォルダ名を残す）。
fn fit_tail(p: &egui::Painter, text: &str, width: f32) -> String {
    if w::text_width(p, text, t::LABEL) <= width {
        return text.to_owned();
    }
    let chars: Vec<char> = text.chars().collect();
    let (mut lo, mut hi) = (0usize, chars.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let candidate: String = std::iter::once('…')
            .chain(chars[chars.len() - mid..].iter().copied())
            .collect();
        if w::text_width(p, &candidate, t::LABEL) <= width {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    std::iter::once('…')
        .chain(chars[chars.len() - lo..].iter().copied())
        .collect()
}

/// ウィンドウが描いたあとで当てる 1 つの頼み。
enum Request {
    Popup(crate::m2_menu::Popup, Rect),
    Do(Action),
}

/// 色空間の名前（書くファイルの一覧）。
fn color_space_name(lang: Lang, srgb: bool) -> &'static str {
    if srgb {
        "sRGB"
    } else {
        lang.pick("リニア", "Linear")
    }
}

/// 左の一覧: 書き出すテクスチャセットのチェック。
fn draw_sets(
    ui: &mut egui::Ui,
    area: Rect,
    rows: &[SetRow],
    scroll: &mut f32,
    requests: &mut Vec<Request>,
) {
    let content = rows.len() as f32 * SET_ROW;
    let bar = Scroll::begin(ui, area, content, scroll);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(area));
    child.set_clip_rect(area.intersect(ui.clip_rect()));
    for (i, row) in rows.iter().enumerate() {
        let r = Rect::from_min_size(
            pos2(
                area.left() + MARGIN,
                area.top() + i as f32 * SET_ROW - *scroll,
            ),
            vec2(area.width() - 2.0 * MARGIN - bar.reserved(), SET_ROW - 2.0),
        );
        if r.bottom() < area.top() || r.top() > area.bottom() {
            continue;
        }
        // 読むだけのセットは、右端の錠の印（セットのパネルと同じ）で見せる
        let toggle_rect = if row.read_only {
            let lock =
                Rect::from_min_size(pos2(r.right() - 18.0, r.center().y - 8.0), vec2(16.0, 16.0));
            w::icon(&child.painter().clone(), lock, "lock", t::WARNING, 14.0);
            Rect::from_min_max(r.min, pos2(lock.left() - 4.0, r.max.y))
        } else {
            r
        };
        let next = w::toggle(
            &mut child,
            toggle_rect,
            ("export.set", row.uid),
            &row.name,
            row.checked,
            None,
            row.enabled,
        );
        if next != row.checked {
            requests.push(Request::Do(Action::Export(ExportAction::SetChecked {
                uid: row.uid,
                on: next,
            })));
        }
    }
    bar.end(ui, "export.sets.scroll", scroll);
}

/// 右の下: 書き出すファイルの一覧（名前・色空間・もうあるファイルの印）か、書けない理由。
fn draw_files(
    ui: &mut egui::Ui,
    area: Rect,
    preview: &Preview,
    scroll: &mut f32,
    lang: Lang,
    id: Id,
) {
    if let Some(problem) = &preview.problem {
        let p = ui.painter().clone();
        w::wrapped_text(&p, area, problem, t::LABEL.with_color(t::WARNING));
        return;
    }
    let content = preview.files.len() as f32 * FILE_ROW;
    let bar = Scroll::begin(ui, area, content, scroll);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(area));
    child.set_clip_rect(area.intersect(ui.clip_rect()));
    let p = child.painter().clone();
    let width = area.width() - bar.reserved();
    let space_width = preview
        .files
        .iter()
        .map(|f| w::text_width(&p, color_space_name(lang, f.srgb), t::LABEL))
        .fold(0.0f32, f32::max);
    for (i, file) in preview.files.iter().enumerate() {
        let r = Rect::from_min_size(
            pos2(area.left(), area.top() + i as f32 * FILE_ROW - *scroll),
            vec2(width, FILE_ROW),
        );
        if r.bottom() < area.top() || r.top() > area.bottom() {
            continue;
        }
        draw_file(
            &mut child,
            &p,
            r,
            file,
            space_width,
            lang,
            id.with(("file", i)),
        );
    }
    bar.end(ui, "export.files.scroll", scroll);
}

fn draw_file(
    ui: &mut egui::Ui,
    p: &egui::Painter,
    r: Rect,
    file: &FileRow,
    space_width: f32,
    lang: Lang,
    id: Id,
) {
    // もうあるファイルは、名前の前に印（点）。置き換えるかは書く前に確かめる
    let dot = Rect::from_center_size(pos2(r.left() + 6.0, r.center().y), vec2(8.0, 8.0));
    if file.exists {
        p.circle_filled(dot.center(), 3.0, t::WARNING);
        ui.interact(dot.expand(4.0), id, egui::Sense::hover())
            .on_hover_text(lang.pick("上書き", "Overwrite"));
    }
    let space = color_space_name(lang, file.srgb);
    w::text(
        p,
        Rect::from_min_max(pos2(r.right() - space_width, r.top()), r.max),
        space,
        t::LABEL.with_color(t::TEXT_DIM),
        Align::Left,
    );
    let name_rect = Rect::from_min_max(
        pos2(r.left() + 18.0, r.top()),
        pos2(r.right() - space_width - 12.0, r.bottom()),
    );
    let shown = w::fit(p, &file.name, name_rect.width(), t::LABEL);
    w::text(p, name_rect, &shown, t::LABEL, Align::Left);
}

/// 開いていればウィンドウを描き、押された操作を当てる。
pub fn show(ctx: &egui::Context, app: &mut AppState) {
    if !app.export.window.open {
        return;
    }
    let lang = app.lang;
    let form = app.export_form();
    let destination = app.export_destination();
    let exporting = app.export.is_exporting();
    let rows = app.export_set_rows();
    let preview = app.export_preview();
    let any_checked = rows.iter().any(|r| r.checked);
    // 書き出すセットが無いときの理由（今のチャンネルの 1 枚で、今のセットが読むだけなら、その理由）
    let no_set_reason = app.export_no_set_reason();
    let padding = crate::prefs::padding_name(lang, app.prefs.settings.export_padding);
    let popup_open = app.ui.popup_was_open;
    // 置き換えの確認などのモーダルが前にある間の Esc は、そちらだけが受ける（ここは閉じない）
    let modal = crate::windows::modal_open(app);
    let spec = Spec {
        title: lang.pick("テクスチャを書き出す", "Export Textures"),
        icon: Some("folder_open"),
        size: vec2(WIDTH, HEIGHT),
        modal: false,
        close_label: lang.pick("ウィンドウを閉じる", "Close Window"),
    };
    let id = Id::new(("yolu.window", WINDOW));
    let mut offset = app.export.window.offset;
    let mut sets_scroll = app.export.window.sets_scroll;
    let mut files_scroll = app.export.window.files_scroll;
    let mut requests: Vec<Request> = Vec::new();
    let mut esc = false;
    let closed = window::show(ctx, id, &spec, &mut offset, false, |ui, frame| {
        // Esc: ウィンドウの上にポインタがあるときに閉じる（ポップアップやモーダルのウィンドウを開いていたら、閉じるのはそちらだけ）
        esc = !popup_open
            && !modal
            && !window::escape_taken(ui.ctx())
            && ui.input(|i| i.key_pressed(Key::Escape))
            && ui
                .input(|i| i.pointer.hover_pos())
                .is_some_and(|p| frame.rect.contains(p));
        let body = frame.body;
        let p = ui.painter().clone();
        let footer = Rect::from_min_max(pos2(body.left(), body.bottom() - FOOTER), body.max);
        // 左: 書き出すテクスチャセット
        let left = Rect::from_min_max(body.min, pos2(body.left() + LEFT_WIDTH, footer.top()));
        w::text(
            &p,
            Rect::from_min_size(
                pos2(left.left() + MARGIN, left.top() + 8.0),
                vec2(left.width() - 2.0 * MARGIN, 16.0),
            ),
            lang.pick("テクスチャセット", "Texture Sets"),
            t::HEADER.with_color(t::TEXT_DIM),
            Align::Left,
        );
        draw_sets(
            ui,
            Rect::from_min_max(pos2(left.left(), left.top() + 32.0), left.max),
            &rows,
            &mut sets_scroll,
            &mut requests,
        );
        w::vline(&p, left.right(), body.top(), footer.top(), t::SEPARATOR);
        // 右: 出力先・出力テンプレート・パディング
        let right = Rect::from_min_max(
            pos2(left.right() + 1.0, body.top()),
            pos2(body.right(), footer.top()),
        );
        let row = |n: usize| {
            Rect::from_min_size(
                pos2(
                    right.left() + MARGIN,
                    right.top() + MARGIN + n as f32 * (ROW + GAP),
                ),
                vec2(right.width() - 2.0 * MARGIN, ROW),
            )
        };
        // 出力先（今の先を出し、「選ぶ…」で替える）
        let r = row(0);
        w::text(
            &p,
            Rect::from_min_size(r.min, vec2(LABEL_WIDTH, r.height())),
            lang.pick("出力先", "Output Path"),
            t::LABEL,
            Align::Left,
        );
        let choose = lang.pick("選ぶ…", "Choose…");
        let choose_width = w::text_width(&p, choose, t::LABEL) + 24.0;
        let choose_rect = Rect::from_min_size(
            pos2(r.right() - choose_width, r.top()),
            vec2(choose_width, r.height()),
        );
        let path_rect = Rect::from_min_max(
            pos2(r.left() + LABEL_WIDTH, r.top()),
            pos2(choose_rect.left() - 6.0, r.bottom()),
        );
        w::rounded(&p, path_rect, t::CONTROL_BG, 3.0);
        w::outline(&p, path_rect, t::BORDER, 1.0, 3.0);
        if let Some(path) = &destination {
            let full = path.display().to_string();
            let shown = fit_tail(&p, &full, path_rect.width() - 14.0);
            w::text(
                &p,
                Rect::from_min_max(pos2(path_rect.left() + 7.0, path_rect.top()), path_rect.max),
                &shown,
                t::LABEL,
                Align::Left,
            );
            if shown != full {
                ui.interact(path_rect, id.with("path"), egui::Sense::hover())
                    .on_hover_text(full);
            }
        }
        if w::button(
            ui,
            choose_rect,
            (id, "choose"),
            choose,
            false,
            true,
            None,
            None,
        )
        .clicked()
        {
            requests.push(Request::Do(Action::Export(ExportAction::ChooseDestination)));
        }
        // 出力テンプレート
        let (response, anchor) = w::dropdown(
            ui,
            row(1),
            (id, "template"),
            Some(lang.pick("出力テンプレート", "Output Template")),
            form.name(lang),
            None,
            true,
            LABEL_WIDTH,
        );
        if response.clicked() {
            requests.push(Request::Popup(crate::m2_menu::Popup::ExportForm, anchor));
        }
        // パディング（設定の「書き出しのパディング」と同じ値）
        let (response, anchor) = w::dropdown(
            ui,
            row(2),
            (id, "padding"),
            Some(lang.pick("パディング", "Padding")),
            &padding,
            None,
            true,
            LABEL_WIDTH,
        );
        if response.clicked() {
            requests.push(Request::Popup(
                crate::m2_menu::Popup::Pref(PrefChoice::ExportPadding),
                anchor,
            ));
        }
        // 書き出すファイル
        let heading_top = row(2).bottom() + GAP + 6.0;
        w::text(
            &p,
            Rect::from_min_size(
                pos2(right.left() + MARGIN, heading_top),
                vec2(right.width() - 2.0 * MARGIN, 16.0),
            ),
            lang.pick("書き出すファイル", "Files to Export"),
            t::HEADER.with_color(t::TEXT_DIM),
            Align::Left,
        );
        w::hline(
            &p,
            right.left() + MARGIN,
            right.right() - MARGIN,
            heading_top + 22.0,
            t::SEPARATOR,
        );
        draw_files(
            ui,
            Rect::from_min_max(
                pos2(right.left() + MARGIN, heading_top + 28.0),
                pos2(right.right() - MARGIN, right.bottom() - 8.0),
            ),
            &preview,
            &mut files_scroll,
            lang,
            id,
        );
        // 下の帯
        w::fill(&p, footer, t::PANEL_HEADER);
        w::hline(&p, footer.left(), footer.right(), footer.top(), t::BORDER);
        let reason = if exporting {
            Some(lang.pick("書き出し中", "An export is running"))
        } else if destination.is_none() {
            Some(lang.pick("出力先が未選択", "No output path"))
        } else if !any_checked {
            Some(no_set_reason.as_str())
        } else {
            None
        };
        let mut x = footer.right() - MARGIN;
        let buttons = [
            (
                lang.pick("キャンセル", "Cancel"),
                false,
                true,
                None,
                ExportAction::CloseWindow,
            ),
            (
                lang.pick("書き出す", "Export"),
                true,
                reason.is_none(),
                reason,
                ExportAction::Run,
            ),
        ];
        for (i, (label, primary, enabled, tip, action)) in buttons.into_iter().enumerate().rev() {
            let bw = w::text_width(&p, label, t::LABEL) + 32.0;
            let r = Rect::from_min_size(pos2(x - bw, footer.top() + 10.0), vec2(bw, 28.0));
            x = r.left() - 8.0;
            if w::button(ui, r, (id, "button", i), label, primary, enabled, tip, None).clicked() {
                requests.push(Request::Do(Action::Export(action)));
            }
        }
    });
    app.export.window.offset = offset;
    app.export.window.sets_scroll = sets_scroll;
    app.export.window.files_scroll = files_scroll;
    for request in requests {
        match request {
            Request::Popup(popup, anchor) => {
                app.popup = Some(OpenPopup {
                    kind: PopupKind::M2(popup),
                    state: PopupState::new(ctx, anchor).with_min_width(anchor.width()),
                });
            }
            Request::Do(action) => app.apply(action),
        }
    }
    if closed || esc {
        app.apply(Action::Export(ExportAction::CloseWindow));
    }
}
