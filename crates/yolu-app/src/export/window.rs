//! 書き出しのウィンドウ（ファイル → 書き出し…）。形（描くチャンネルの PNG・全チャンネルの PNG・Unity Standard / URP Lit・HDRP Lit・lilToon）を選び、
//! 書き出す先と余白を決めて「書き出す」を押すと、それぞれの書き出しの道（`ChannelTo`／`ChannelNamed`・`ChannelsTo`・`TemplateTo`）へ渡す。
//! 書く手順・置き換えの確かめ・取消・結果のウィンドウは、道の先（`export/mod.rs`）が持つ。
//!
//! - 形は設定に覚える（`Settings::export_form`。文書ごとではない）。書き出す先は、描くチャンネルの PNG ならファイル、ほかはフォルダ。
//!   選んでいない間の既定は、Live Link の相手の文書なら Unity が知らせた置き場、そうでなければ開いたプロジェクトのフォルダ。
//!   選んだ先はプロジェクトを替えるまで覚える。
//! - 余白は設定の「書き出しの余白」と同じ値（このウィンドウから替えると設定も替わる）。ほかに新しい項目は持たない。
//! - 外からの操作（MCP・コマンドライン・オートアクションの再生）は `yolu_ops::export` を通り、このウィンドウを経ない。

use std::path::{Path, PathBuf};

use egui::{pos2, vec2, Id, Key, Rect, Vec2};

use super::{default_channel_file_name, ExportAction};
use crate::dialog::places::Place;
use crate::lang::Lang;
use crate::notice::Source;
use crate::prefs::PrefChoice;
use crate::state::{Action, AppState, DialogRequest, OpenPopup, PopupKind};
use crate::ui::menu::{Entry, PopupState};
use crate::ui::theme as t;
use crate::ui::widgets::{self as w, Align};
use crate::ui::window::{self, Spec};

/// ウィンドウの名前（`windows::window_rect` で矩形を引く名前）。
pub const WINDOW: &str = "export";

const WIDTH: f32 = 460.0;
const ROW: f32 = 24.0;
const GAP: f32 = 8.0;
const MARGIN: f32 = 14.0;
const LABEL_WIDTH: f32 = 96.0;
const FOOTER: f32 = 48.0;

/// 書き出しの形。
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
            ExportForm::ChannelPng => lang.pick("PNG（今のチャンネル）", "PNG (Current Channel)"),
            ExportForm::AllChannels => lang.pick("PNG（全チャンネル）", "PNG (All Channels)"),
            ExportForm::UnityStandard => "Unity Standard / URP Lit",
            ExportForm::Hdrp => "HDRP Lit",
            ExportForm::LilToon => "lilToon",
        }
    }
}

/// ウィンドウの状態（開いているか・動かした量・選んだ書き出す先）。アプリの状態で、.ylp には入れない。
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
    /// 選んだ先がどのプロジェクトのものか（`AppState::project_epoch`）。替わったら忘れる。
    epoch: u64,
}

impl AppState {
    /// 今の書き出しの形（設定に覚えている）。
    pub fn export_form(&self) -> ExportForm {
        self.prefs.settings.export_form
    }

    /// 開いたプロジェクトのフォルダ（保存していなければ、またフォルダが無ければ None）。
    fn project_folder(&self) -> Option<PathBuf> {
        self.project
            .as_ref()
            .and_then(|p| p.path().parent())
            .filter(|d| d.is_dir())
            .map(Path::to_path_buf)
    }

    /// 選んだ書き出す先。別のプロジェクトで選んだものは、ウィンドウを開いたままプロジェクトを替えられても（新規・開く・Live Link・外からの操作）
    /// 今のプロジェクトのものではないので、無いものとして扱う。
    fn export_chosen(&self) -> Option<&WindowState> {
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
            }
            ExportAction::CloseWindow => self.export.window.open = false,
            ExportAction::SetForm(form) => self.prefs.settings.export_form = form,
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

    /// 「書き出す」: 形に合う書き出しの道へ渡す。始まった（書く仕事が動き出した）ら、ウィンドウを閉じる。
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
                lang.pick("書き出す先がありません。", "There is no destination."),
            );
            return;
        };
        let action = match form.template_id() {
            Some(id) => ExportAction::TemplateTo {
                id: id.to_owned(),
                dir: destination,
            },
            None if form.writes_file() => {
                // 選ぶウィンドウが確かめたファイルはそのまま、ほかは書く前にもうあるか確かめる
                if confirmed.as_ref() == Some(&destination) {
                    ExportAction::ChannelTo(destination)
                } else {
                    ExportAction::ChannelNamed(destination)
                }
            }
            None => ExportAction::ChannelsTo(destination),
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

/// 形のドロップダウンの項目。
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
/// 始まりの場所は、今の書き出す先（選んだ先・Live Link の置き場・プロジェクトのフォルダ）が決まっていればそこの今ある一番近いフォルダ、
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

/// 開いていればウィンドウを描き、押された操作を当てる。
pub fn show(ctx: &egui::Context, app: &mut AppState) {
    if !app.export.window.open {
        return;
    }
    let lang = app.lang;
    let form = app.export_form();
    let destination = app.export_destination();
    let exporting = app.export.is_exporting();
    let padding = crate::prefs::padding_name(lang, app.prefs.settings.export_padding);
    let popup_open = app.ui.popup_was_open;
    // 置き換えの確認などのモーダルが前にある間の Esc は、そちらだけが受ける（ここは閉じない）
    let modal = crate::windows::modal_open(app);
    let height = window::HEADER_HEIGHT + MARGIN + 3.0 * ROW + 2.0 * GAP + MARGIN + FOOTER;
    let spec = Spec {
        title: lang.pick("書き出し", "Export"),
        icon: Some("folder_open"),
        size: vec2(WIDTH, height),
        modal: false,
        close_label: lang.pick("ウィンドウを閉じる", "Close Window"),
    };
    let id = Id::new(("yolu.window", WINDOW));
    let mut offset = app.export.window.offset;
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
        let row = |n: usize| {
            Rect::from_min_size(
                pos2(
                    body.left() + MARGIN,
                    body.top() + MARGIN + n as f32 * (ROW + GAP),
                ),
                vec2(body.width() - 2.0 * MARGIN, ROW),
            )
        };
        // 形
        let (response, anchor) = w::dropdown(
            ui,
            row(0),
            (id, "form"),
            Some(lang.pick("形", "Type")),
            form.name(lang),
            None,
            true,
            LABEL_WIDTH,
        );
        if response.clicked() {
            requests.push(Request::Popup(crate::m2_menu::Popup::ExportForm, anchor));
        }
        // 書き出す先（今の先を出し、「選ぶ…」で替える）
        let r = row(1);
        let p = ui.painter().clone();
        w::text(
            &p,
            Rect::from_min_size(r.min, vec2(LABEL_WIDTH, r.height())),
            lang.pick("書き出す先", "Destination"),
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
        // 余白（設定の「書き出しの余白」と同じ値）
        let (response, anchor) = w::dropdown(
            ui,
            row(2),
            (id, "padding"),
            Some(lang.pick("余白", "Padding")),
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
        // 下の帯
        let footer = Rect::from_min_max(pos2(body.left(), body.bottom() - FOOTER), body.max);
        w::fill(&p, footer, t::PANEL_HEADER);
        w::hline(&p, footer.left(), footer.right(), footer.top(), t::BORDER);
        let reason = if exporting {
            Some(lang.pick("書き出し中", "An export is running"))
        } else if destination.is_none() {
            Some(lang.pick("書き出す先が未選択", "No destination"))
        } else {
            None
        };
        let mut x = footer.right() - MARGIN;
        let buttons = [
            (
                lang.pick("やめる", "Cancel"),
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
