//! テキストツール（T）とテキストレイヤーの値。2D のキャンバスを押すと、その点を 1 行目の上端にして文字を打てる（テキストレイヤーを作るのは
//! 最初の 1 文字）。テキストレイヤーを選んで箱の中を押す（移動・変形ツールならダブルクリック）と、そのレイヤーの文字を打ち直せる。打っている間は
//! 文字を描き直しながら見せ、打ち終わり（Esc・外を押す）までを 1 回の Undo にする（core の `set_text` のまとめ）。
//!
//! フォントは同梱の BIZ UDPGothic・OS に入っているフォント（一覧は別のスレッドで `yolu_io::fonts` がなめる）・利用者が選んだファイル。文書には
//! 道・SHA-256・ファミリー名・PostScript 名・太さだけが入る。開いた文書のフォントは `yolu_io::fonts::find` の順で探し、中身が違えば
//! 「フォントが違います」と知らせて描いた画素のまま（打ち直すと見つけたフォントで描く）、無ければ画素のまま値の編集を理由を添えて断る。
//!
//! テキストの値（フォント・サイズ・色・行間・字間・揃え・折り返しの幅）は、打っている間はその文字、テキストレイヤーを選んでいればそのレイヤー、
//! どちらでもなければ次に作る文字の既定（`defaults`）に当たる。移動・変形ツールで動かす・回すと、テキストレイヤーは画素でなく値（位置・回転）が変わる。
//!
//! 文字の色の元（`ColorSource`。ツールプロパティの「テキストの色」）は 2 つ。「描画色」（既定）は、新しい文字を描画色で作り、テキストのツールを使っている
//! 間（テキストレイヤーを選んでいる間・打っている間）に描画色が変わると、そのレイヤーの色も変える（`text_follow_paint_color`。描画色の円のドラッグは
//! 1 回の取り消し。ブラシなど、ほかのツールで描画色を選ぶときは、選んだままのテキストの色を変えない）。
//! 「ツールの色」は、ツールが持つ色（`defaults.color`）で新しい文字を作り、その色を変えると、選んでいるレイヤーの色も変える。色は 1 つのテキストに 1 色
//! （文字ごとの色は持たない）。

pub mod canvas;
pub mod props;

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use yolu_core::text::{self, TextAlign, TextFont, TextLayout, TextSettings};
use yolu_io::fonts::{self as sysfonts, Lookup, SystemFonts};

use crate::engine::{LayerId, Rgba8};
use crate::jobs::{JobSpec, Polled, Worker};
use crate::lang::{refusals, Lang};
use crate::notice::{Kind, Source};
use crate::state::{AppState, DialogRequest, Tool};

/// フォントを探した結果の覚えの数。
const FOUND_CACHE: usize = 16;

/// 同梱のフォントの中身（名前は `yolu_core::text::BUNDLED_FONTS`）。画面のフォントと同じファイル。
pub fn bundled_font(name: &str) -> Option<Arc<[u8]>> {
    static FACES: OnceLock<[Arc<[u8]>; 2]> = OnceLock::new();
    let faces = FACES.get_or_init(|| {
        [
            Arc::from(crate::ui::fonts::REGULAR),
            Arc::from(crate::ui::fonts::BOLD_FACE),
        ]
    });
    let at = text::BUNDLED_FONTS.iter().position(|n| *n == name)?;
    Some(faces[at].clone())
}

/// フォントの画面の名前（同梱はフォントの名前、ファイルはファミリー名とスタイル（Regular は省く）。名前が無ければファイルの名前）。
/// 日本語の画面で OS の一覧に日本語のファミリー名があれば、それを使う。
pub fn font_label(lang: Lang, font: &TextFont, list: Option<&SystemFonts>) -> String {
    match font {
        TextFont::Bundled(name) if name == "biz-udpgothic-bold" => lang
            .pick("BIZ UDPゴシック 太字", "BIZ UDPGothic Bold")
            .into(),
        TextFont::Bundled(name) if name == text::DEFAULT_FONT => {
            lang.pick("BIZ UDPゴシック", "BIZ UDPGothic").into()
        }
        TextFont::Bundled(name) => name.clone(),
        TextFont::File {
            path, index, names, ..
        } => {
            if names.family.is_empty() {
                let name = Path::new(path)
                    .file_name()
                    .map_or_else(|| path.clone(), |n| n.to_string_lossy().into_owned());
                return if *index == 0 {
                    name
                } else {
                    format!("{name} #{index}")
                };
            }
            let face = list.and_then(|l| l.by_postscript(&names.postscript));
            let family = list
                .filter(|_| lang == Lang::Ja)
                .and_then(|l| {
                    l.families()
                        .iter()
                        .find(|f| f.name.eq_ignore_ascii_case(&names.family))
                })
                .and_then(|f| f.name_ja.clone())
                .unwrap_or_else(|| names.family.clone());
            let style = face.map_or_else(
                || sysfonts::weight_name(names.weight, names.italic),
                |f| f.style(),
            );
            if style.eq_ignore_ascii_case("regular") {
                family
            } else {
                format!("{family} {style}")
            }
        }
    }
}

/// OS のフォントの一覧の画面の側（別のスレッドでなめる）。
#[derive(Default)]
pub struct Fonts {
    /// できた一覧（なめ終わるまで None）。試験は決まったフォルダの一覧を置ける。
    pub list: Option<Arc<SystemFonts>>,
    pub(crate) worker: Option<Worker<Arc<SystemFonts>>>,
}

impl std::fmt::Debug for Fonts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Fonts")
            .field("faces", &self.list.as_ref().map(|l| l.faces().len()))
            .field("loading", &self.worker.is_some())
            .finish()
    }
}

impl Clone for Fonts {
    fn clone(&self) -> Self {
        Fonts {
            list: self.list.clone(),
            worker: None,
        }
    }
}

impl Fonts {
    pub fn is_loading(&self) -> bool {
        self.worker.is_some()
    }
}

impl TextState {
    /// フォントを探した結果の覚えを捨てる（開いた文書のフォントを探し直す）。
    pub fn forget_fonts(&mut self) {
        self.found.clear();
    }
}

/// フォントの一覧をなめる仕事（動いている間は描き直し続けて、できたら受ける）。
pub const JOB: JobSpec = JobSpec {
    repaint: true,
    ..JobSpec::new("text.fonts", |app| app.text.fonts.is_loading())
};

/// テキストレイヤーのフォントの今の様子（値の欄の短い状態）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FontStatus {
    /// 文書の値と同じフォントがある。
    Ready,
    /// 中身の違うフォントしか無い（描き直すと見つけたフォントで描く）。
    Different,
    /// どこにも無い（値の編集を断る）。
    Missing,
    /// OS のフォントの一覧をなめている（終わるまで値の編集を待つ）。
    Searching,
}

/// フォントを探した結果（覚え）。
#[derive(Clone)]
enum Found {
    Ready {
        font: TextFont,
        bytes: Arc<[u8]>,
        different: bool,
    },
    Missing,
}

/// 打っている文字（テキストツールで押してから、打ち終わりまで）。
#[derive(Clone, Debug)]
pub struct Editing {
    /// テキストレイヤー（新しい文字は最初の 1 文字で作るので、それまでは None）。
    pub layer: Option<LayerId>,
    /// この打ち込みで作ったレイヤーか（何も打たずに終えたら消す）。
    pub created: bool,
    /// 今の値（文は `buffer` と同じ）。
    pub value: TextSettings,
    /// 入力欄の文。
    pub buffer: String,
    /// 入力欄にフォーカスを渡す（押した次のフレームから）。
    pub focus_wanted: bool,
    /// 入力欄がフォーカスを持ったことがあるか（失ったら打ち終わり）。
    pub had_focus: bool,
    /// 入力欄のカーソル（文字の番号の主・副）。
    pub cursor: Option<(usize, usize)>,
    /// 入力欄のカーソルを動かす頼み（文字の番号。箱の中を押した）。
    pub move_cursor: Option<usize>,
    /// 並べの覚え（同じ値なら並べ直さない）。
    layout: Option<(TextSettings, Arc<TextLayout>)>,
}

/// テキストツールとテキストレイヤーの画面の状態。
#[derive(Clone, Debug)]
pub struct TextState {
    /// 次に作る文字の値（文と位置は押した所で決まる）。
    pub defaults: TextSettings,
    pub editing: Option<Editing>,
    /// 押して離すまで（離したところで打ち始める。押した所はキャンバスの座標）。
    pub press: Option<((f64, f64), crate::state::StrokeSource)>,
    /// 移動・変形ツールで最後に押した時刻と所（ダブルクリックを見る）。
    pub move_click: Option<(f64, egui::Pos2)>,
    /// 移動・変形ツールのダブルクリックの押し（離したときにテキストツールで打ち直すレイヤーと、押した所のキャンバスの座標）。
    pub move_press: Option<(LayerId, (f64, f64), crate::state::StrokeSource)>,
    /// スライダーを動かしている間の値（離したときに 1 回で描き直す）。
    pub pending: Option<Pending>,
    /// フォントを探した結果の覚え（同じフォントを読み直さない。OS の一覧ができたら捨てる）。
    found: Vec<(TextFont, Found)>,
    /// OS のフォントの一覧。
    pub fonts: Fonts,
    /// 開いた文書のテキストレイヤーのフォントを確かめて知らせる（一覧が要れば、できるまで待つ）。
    pub check_fonts: bool,
    /// 文字の色の元（ツールプロパティの「テキストの色」）。
    pub color_source: ColorSource,
    /// 描画色を前のフレームに見たときの値（変わったときだけ、選んでいるテキストへ当てる）。
    followed_main: Option<[f32; 4]>,
    /// 描画色の変更を、選んでいるテキストレイヤーへまとめて当てている最中か（ドラッグが終わったら、まとめを切る）。
    following: bool,
}

/// 文字の色の元。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColorSource {
    /// 描画色（既定）。
    #[default]
    PaintColor,
    /// ツールの色（`TextState::defaults` の色）。
    ToolColor,
}

/// スライダーを動かしている間の値。同じ値のスライダーはオプションバー・ツールプロパティ・レイヤーのプロパティに同時に出るので、
/// どの欄の値かを `key`（値の名前。出ている欄みんなが動かしている値を見せる）と `owner`（動かしている欄の部品の id。
/// 当てる・捨てるのはこの欄だけ。動かしていないほかの欄が消さない）で持つ。`target` は動かし始めたときの当て先
/// （途中でレイヤーが替わったら、別のレイヤーには当てない）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pending {
    pub key: &'static str,
    pub owner: egui::Id,
    pub target: Target,
    pub value: f64,
}

impl std::fmt::Debug for Found {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Found::Ready {
                font, different, ..
            } => f
                .debug_struct("Ready")
                .field("font", font)
                .field("different", different)
                .finish(),
            Found::Missing => f.write_str("Missing"),
        }
    }
}

impl Default for TextState {
    fn default() -> Self {
        TextState {
            defaults: TextSettings::new("", TextFont::Bundled(text::DEFAULT_FONT.into()), 0.0, 0.0),
            editing: None,
            press: None,
            pending: None,
            move_click: None,
            move_press: None,
            found: Vec::new(),
            fonts: Fonts::default(),
            check_fonts: false,
            color_source: ColorSource::default(),
            followed_main: None,
            following: false,
        }
    }
}

/// 文字の値の 1 つの変更。
#[derive(Clone, Debug, PartialEq)]
pub enum Field {
    Font(TextFont),
    Size(f64),
    Color(Rgba8),
    LineHeight(f64),
    LetterSpacing(f64),
    Align(TextAlign),
    WrapWidth(f64),
}

impl Field {
    fn apply(&self, t: &mut TextSettings) {
        match self {
            Field::Font(f) => t.font = f.clone(),
            Field::Size(v) => t.size = v.clamp(text::MIN_SIZE, text::MAX_SIZE),
            Field::Color(c) => t.color = *c,
            Field::LineHeight(v) => {
                t.line_height = v.clamp(text::MIN_LINE_HEIGHT, text::MAX_LINE_HEIGHT)
            }
            Field::LetterSpacing(v) => {
                t.letter_spacing = v.clamp(text::MIN_LETTER_SPACING, text::MAX_LETTER_SPACING)
            }
            Field::Align(a) => t.align = *a,
            Field::WrapWidth(v) => t.wrap_width = v.clamp(0.0, text::MAX_COORDINATE),
        }
    }
}

/// テキストツールの操作（キー・試験と同じ道）。
#[derive(Clone, Debug, PartialEq)]
pub enum TextAction {
    /// 押した所（キャンバスの座標）で新しい文字を打ち始める。
    Begin { x: f64, y: f64 },
    /// テキストレイヤーを打ち直し始める。
    Edit(LayerId),
    /// 打ち終わる。
    Commit,
    /// 値を 1 つ変える（打っている文字・選んでいるテキストレイヤー・次の文字の既定）。
    Set(Field),
    /// 色のウィンドウのドラッグの途中の値（テキストレイヤーは前の変更とまとめて 1 回の取り消し）。
    Drag(Field),
    /// フォントのファイルを選ぶウィンドウを開く。
    PickFontFile,
    /// 選んだフォントのファイル。
    FontFile(PathBuf),
    /// OS のフォントの一覧から選んだフォント（ファイルと束の中の番号）。
    SystemFont { path: PathBuf, index: u32 },
    /// 文字の値を外して画素だけにする（1 回の Undo）。
    Rasterize(LayerId),
    /// 文字の色の元を替える（文書は変えない）。
    ColorSource(ColorSource),
    /// 「ツールの色」を変える（打っている文字・選んでいるテキストレイヤーの色も、同じ色にする）。`dragging` は色のウィンドウのドラッグの途中
    /// （テキストレイヤーは前の変更とまとめて 1 回の取り消し）。
    ToolColor { color: Rgba8, dragging: bool },
}

impl TextAction {
    /// 文書を変える操作か（読むだけのセットでは断る）。
    pub fn edits_document(&self) -> bool {
        matches!(
            self,
            TextAction::Begin { .. } | TextAction::Edit(_) | TextAction::Rasterize(_)
        )
    }
}

/// 値の当て先。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Editing,
    Layer(LayerId),
    Defaults,
}

impl AppState {
    /// 文字の値の当て先（打っている文字・選んでいるテキストレイヤー・次の文字の既定）。
    pub fn text_target(&self) -> Target {
        if self.text.editing.is_some() {
            return Target::Editing;
        }
        match self.selected_layer {
            Some(id) if self.doc.layer(id).is_some_and(|l| l.text().is_some()) => Target::Layer(id),
            _ => Target::Defaults,
        }
    }

    /// 当て先の今の値。
    pub fn text_value(&self) -> TextSettings {
        match self.text_target() {
            Target::Editing => self
                .text
                .editing
                .as_ref()
                .expect("打っている")
                .value
                .clone(),
            Target::Layer(id) => self
                .doc
                .layer(id)
                .and_then(|l| l.text())
                .cloned()
                .expect("テキストレイヤー"),
            Target::Defaults => self.text.defaults.clone(),
        }
    }

    /// OS のフォントの一覧を別のスレッドでなめ始める（まだ無く、なめていなければ）。
    pub fn text_fonts_wanted(&mut self) {
        let fonts = &mut self.text.fonts;
        if fonts.list.is_some() || fonts.worker.is_some() {
            return;
        }
        if let Some(list) = sysfonts::loaded() {
            fonts.list = Some(list);
            self.text.found.clear();
            return;
        }
        match Worker::spawn("yolu-fonts", |tx, _cancel| {
            let _ = tx.send(sysfonts::system());
        }) {
            Ok(worker) => fonts.worker = Some(worker),
            // スレッドを作れなければ、一覧の無いまま（同梱・ファイルのフォントと覚えた道だけで探す）
            Err(_) => fonts.list = Some(Arc::new(SystemFonts::default())),
        }
    }

    /// 毎フレーム: フォントの一覧ができたら受け、開いた文書のフォントの確かめを進める。
    pub fn text_poll(&mut self) {
        if let Some(worker) = &self.text.fonts.worker {
            match worker.poll() {
                Polled::Empty => {}
                Polled::Message(list) => {
                    self.text.fonts.list = Some(list);
                    self.text.fonts.worker = None;
                    // 一覧の無いときの「見つからない」を探し直す
                    self.text.found.clear();
                }
                Polled::Lost => {
                    self.text.fonts.worker = None;
                    self.text.fonts.list = Some(Arc::new(SystemFonts::default()));
                    self.text.found.clear();
                }
            }
        }
        if self.text.check_fonts {
            self.text_check_fonts();
        }
    }

    /// 開いた文書のテキストレイヤーのフォントを探し、違う・無いフォントを 1 度だけ知らせる（一覧が要るなら、できるまで待つ）。
    fn text_check_fonts(&mut self) {
        let layers: Vec<(String, TextFont)> = self
            .doc
            .layers()
            .iter()
            .filter_map(|l| Some((l.name().to_owned(), l.text()?.font.clone())))
            .collect();
        let mut different = Vec::new();
        let mut missing = Vec::new();
        for (name, font) in &layers {
            match self.text_font_lookup(font) {
                None => return,
                Some(Found::Ready {
                    different: true, ..
                }) => different.push(name.clone()),
                Some(Found::Ready { .. }) => {}
                Some(Found::Missing) => missing.push(name.clone()),
            }
        }
        self.text.check_fonts = false;
        let lang = self.lang;
        let names = |list: &[String]| {
            let mut quoted: Vec<String> = list.iter().take(3).map(|n| lang.quote(n)).collect();
            if list.len() > 3 {
                quoted.push("…".into());
            }
            quoted.join(lang.pick("・", ", "))
        };
        let mut texts = Vec::new();
        if !different.is_empty() {
            texts.push(lang.with_reason(
                lang.pick("フォントが違います", "Font differs"),
                names(&different),
            ));
        }
        if !missing.is_empty() {
            texts.push(lang.with_reason(
                lang.pick("フォントが見つかりません", "Font not found"),
                names(&missing),
            ));
        }
        if !texts.is_empty() {
            self.warn(Source::Text, texts.join(" "));
        }
    }

    /// フォントを探す（覚えがあればそれ）。OS の一覧が要るのにまだ無ければ、なめ始めて None。
    fn text_font_lookup(&mut self, font: &TextFont) -> Option<Found> {
        if let TextFont::Bundled(name) = font {
            return Some(match bundled_font(name) {
                Some(bytes) => Found::Ready {
                    font: font.clone(),
                    bytes,
                    different: false,
                },
                None => Found::Missing,
            });
        }
        if let Some((_, found)) = self.text.found.iter().find(|(f, _)| f == font) {
            return Some(found.clone());
        }
        let lookup = match sysfonts::find_at_path(font) {
            Some(same) => same,
            None => {
                let Some(list) = self.text.fonts.list.clone() else {
                    self.text_fonts_wanted();
                    // 一覧がもうできていれば（ほかの文書・CLI の命令がなめた）、ここで受けている
                    let list = self.text.fonts.list.clone()?;
                    return Some(self.text_remember(font, sysfonts::find(font, &list)));
                };
                sysfonts::find(font, &list)
            }
        };
        Some(self.text_remember(font, lookup))
    }

    fn text_remember(&mut self, font: &TextFont, lookup: Lookup) -> Found {
        let found = match lookup {
            Lookup::Same { font, bytes } => Found::Ready {
                font,
                bytes,
                different: false,
            },
            Lookup::Different { font, bytes } => Found::Ready {
                font,
                bytes,
                different: true,
            },
            Lookup::Missing => Found::Missing,
        };
        self.text.found.insert(0, (font.clone(), found.clone()));
        self.text.found.truncate(FOUND_CACHE);
        found
    }

    /// 描くフォント（見つけた所に直した値と中身）。見つからなければ画面の言語の理由。
    pub fn text_font(&mut self, font: &TextFont) -> Result<(TextFont, Arc<[u8]>), String> {
        let lang = self.lang;
        match self.text_font_lookup(font) {
            Some(Found::Ready { font, bytes, .. }) => Ok((font, bytes)),
            Some(Found::Missing) => Err(lang
                .pick("フォントが見つかりません", "Font not found")
                .to_owned()),
            None => Err(lang
                .pick("フォントを探しています", "Searching for the font")
                .to_owned()),
        }
    }

    /// テキストレイヤーのフォントの様子。
    pub fn text_font_status(&mut self, id: LayerId) -> FontStatus {
        let Some(font) = self
            .doc
            .layer(id)
            .and_then(|l| l.text())
            .map(|t| t.font.clone())
        else {
            return FontStatus::Ready;
        };
        match self.text_font_lookup(&font) {
            Some(Found::Ready {
                different: false, ..
            }) => FontStatus::Ready,
            Some(Found::Ready { .. }) => FontStatus::Different,
            Some(Found::Missing) => FontStatus::Missing,
            None => FontStatus::Searching,
        }
    }

    /// テキストレイヤーの値を変えられないフォントの理由（無い・探している）。違うフォントは変えられる（見つけたフォントで描き直す）。
    pub fn text_font_problem(&mut self, id: LayerId) -> Option<String> {
        let lang = self.lang;
        match self.text_font_status(id) {
            FontStatus::Ready | FontStatus::Different => None,
            FontStatus::Missing => Some(
                lang.pick("フォントが見つかりません", "Font not found")
                    .to_owned(),
            ),
            FontStatus::Searching => Some(
                lang.pick("フォントを探しています", "Searching for the font")
                    .to_owned(),
            ),
        }
    }

    pub fn text_apply(&mut self, action: TextAction) {
        match action {
            TextAction::Begin { x, y } => self.text_begin(x, y),
            TextAction::Edit(id) => self.text_edit(id),
            TextAction::Commit => self.text_commit(),
            TextAction::Set(field) => self.text_set(field, false),
            TextAction::Drag(field) => self.text_set(field, true),
            TextAction::PickFontFile => self.dialog_request = Some(DialogRequest::TextFont),
            TextAction::FontFile(path) => self.text_font_file(&path, 0),
            TextAction::SystemFont { path, index } => self.text_font_file(&path, index),
            TextAction::Rasterize(id) => self.text_rasterize(id),
            TextAction::ColorSource(source) => {
                self.text.color_source = source;
                // 描画色を見る基準を今の色にする（替えただけで、選んでいるテキストの色を変えない）
                self.text.followed_main = Some(self.color.main);
                self.text_end_follow();
            }
            TextAction::ToolColor { color, dragging } => {
                self.text.defaults.color = color;
                self.text_set(Field::Color(color), dragging);
            }
        }
    }

    /// 打ち始めの共通の断り（描いている間・読むだけのセット）。断ったら true。
    fn text_refused(&mut self) -> bool {
        let lang = self.lang;
        if self.is_stroking() {
            self.refuse(Source::Text, refusals::during_stroke(lang));
            return true;
        }
        if let Some(reason) = self.read_only_reason().map(str::to_owned) {
            self.refuse(Source::Text, refusals::read_only_set(lang, &reason));
            return true;
        }
        false
    }

    fn text_begin(&mut self, x: f64, y: f64) {
        self.text_commit();
        if self.text_refused() {
            return;
        }
        let mut value = self.text.defaults.clone();
        value.text.clear();
        value.x = x.clamp(-text::MAX_COORDINATE, text::MAX_COORDINATE);
        value.y = y.clamp(-text::MAX_COORDINATE, text::MAX_COORDINATE);
        // 新しい文字の色は、描画色（既定）かツールの色
        value.color = match self.text.color_source {
            ColorSource::PaintColor => crate::matpaint::single_value(self.color.main),
            ColorSource::ToolColor => self.text.defaults.color,
        };
        match self.text_font(&value.font.clone()) {
            Ok((font, _)) => value.font = font,
            Err(reason) => {
                self.refuse(Source::Text, self.text_cannot(&reason));
                return;
            }
        }
        self.text.editing = Some(Editing::new(None, false, value));
    }

    fn text_edit(&mut self, id: LayerId) {
        if self
            .text
            .editing
            .as_ref()
            .is_some_and(|e| e.layer == Some(id))
        {
            return;
        }
        self.text_commit();
        if self.text_refused() {
            return;
        }
        let Some(mut value) = self.doc.layer(id).and_then(|l| l.text()).cloned() else {
            return;
        };
        // 違うフォントしか無ければ、打ったときに見つけたフォントで描き直す
        match self.text_font(&value.font.clone()) {
            Ok((font, _)) => value.font = font,
            Err(reason) => {
                self.refuse(Source::Text, self.text_cannot(&reason));
                return;
            }
        }
        self.selected_layer = Some(id);
        self.tool = Tool::Text;
        let mut editing = Editing::new(Some(id), false, value);
        let end = editing.buffer.chars().count();
        editing.move_cursor = Some(end);
        self.text.editing = Some(editing);
    }

    /// 「文字を描けません（理由）。」
    fn text_cannot(&self, reason: &str) -> String {
        self.lang.with_reason(
            self.lang
                .pick("テキストを描けません", "Cannot draw the text"),
            reason,
        )
    }

    /// 入力欄の文が変わった（打っている文字を描き直す。新しい文字は最初の 1 文字でレイヤーを作る）。
    pub fn text_typed(&mut self, buffer: String) {
        let Some(editing) = self.text.editing.as_ref() else {
            return;
        };
        if editing.buffer == buffer {
            return;
        }
        let mut value = editing.value.clone();
        value.text = buffer.clone();
        let layer = editing.layer;
        if let Some(e) = self.text.editing.as_mut() {
            e.buffer = buffer;
        }
        self.text_redraw(layer, value);
    }

    /// 打っている文字の値を文書へ（まとめる段へ）。断ったら入力欄の文を文書の値へ戻す。
    fn text_redraw(&mut self, layer: Option<LayerId>, mut value: TextSettings) {
        let font = match self.text_font(&value.font) {
            Ok((found, bytes)) => {
                value.font = found;
                bytes
            }
            Err(reason) => {
                self.refuse(Source::Text, self.text_cannot(&reason));
                return;
            }
        };
        let result = match layer {
            Some(id) => self
                .doc
                .set_text(id, value.clone(), &font, true)
                .map(|_| id),
            None if value.text.is_empty() => {
                if let Some(e) = self.text.editing.as_mut() {
                    e.value = value;
                }
                return;
            }
            None => {
                let name = self.text_layer_name();
                let above = self
                    .selected_layer
                    .filter(|id| self.doc.layer(*id).is_some());
                self.doc
                    .add_text_layer(&name, value.clone(), &font, above, true)
            }
        };
        match result {
            Ok(id) => {
                self.modified = true;
                if let Some(e) = self.text.editing.as_mut() {
                    e.created |= e.layer.is_none();
                    e.layer = Some(id);
                    e.value = value;
                }
                if self.selected_layer != Some(id) {
                    self.select_new(id);
                }
            }
            Err(e) => {
                self.notify(Kind::of_core(&e), Source::Text, self.lang.core_error(&e));
                if let Some(ed) = self.text.editing.as_mut() {
                    ed.buffer = ed.value.text.clone();
                }
            }
        }
    }

    fn text_layer_name(&self) -> String {
        let n = self
            .doc
            .layers()
            .iter()
            .filter(|l| l.text().is_some())
            .count()
            + 1;
        format!("{} {n}", self.lang.pick("テキスト", "Text"))
    }

    /// 打ち終わる（打っている間の変更は 1 回の Undo にまとまっている）。何も打たずに終えた新しいレイヤーは消す。
    pub fn text_commit(&mut self) {
        let Some(editing) = self.text.editing.take() else {
            return;
        };
        self.doc.end_coalescing();
        if editing.created && editing.buffer.is_empty() {
            // 作ったレイヤーを追加した段へまとめているので、1 回の取り消しでレイヤーごと消える
            if self.doc.undo().is_ok() && self.selected_layer == editing.layer {
                self.selected_layer = self.doc.layers().last().map(|l| l.id());
            }
        }
    }

    /// 毎フレーム: 文字の色の元が描画色で、テキストのツールを使っている（か、文字を打っている）とき、描画色が変わったら、打っている文字・選んでいる
    /// テキストレイヤーの色を同じ色にする。ほかのツール（ブラシなど）のあいだは、描画色が変わっても選んだままのテキストの色を変えない（基準の色は
    /// 追い続けるので、テキストのツールへ替えただけでは色が変わらない）。
    /// `pointer_down` はマウス・ペンのボタンが押されている間（円のドラッグ）で、その間の変更は前の変更とまとめて 1 回の取り消しにし、離したら
    /// まとめを切る（16 進・色の列・スポイトなど、押していない間の変更は 1 回ずつ）。描いている間・読むだけのセット・ロックされたレイヤー・フォントが無い
    /// レイヤーは、黙って変えない。テキストを選んでいないときは何もしない（新しい文字は、作るときの描画色で作る）。
    pub fn text_follow_paint_color(&mut self, pointer_down: bool) {
        let main = self.color.main;
        // 最初の 1 回は基準を覚えるだけ（開いた直後に選んでいるテキストの色を、黙って変えない）
        let changed = self.text.followed_main.is_some_and(|before| before != main);
        self.text.followed_main = Some(main);
        let active = self.tool == Tool::Text || self.text.editing.is_some();
        let following = self.text.color_source == ColorSource::PaintColor && active;
        if following && changed {
            self.text_apply_paint_color(crate::matpaint::single_value(main));
        }
        if !pointer_down || !following {
            self.text_end_follow();
        }
    }

    /// 描画色のまとめを切る（まとめていなければ何もしない）。打っている最中は、打ち終わりまで 1 回の取り消しなので切らない。
    fn text_end_follow(&mut self) {
        if std::mem::take(&mut self.text.following) && self.text.editing.is_none() {
            self.doc.end_coalescing();
        }
    }

    fn text_apply_paint_color(&mut self, color: Rgba8) {
        match self.text_target() {
            // 次の文字は、作るときの描画色で作る
            Target::Defaults => {}
            Target::Editing => {
                let Some(e) = self.text.editing.as_ref() else {
                    return;
                };
                if e.value.color == color {
                    return;
                }
                let (layer, mut value) = (e.layer, e.value.clone());
                value.color = color;
                match layer {
                    Some(id) => self.text_redraw(Some(id), value),
                    None => {
                        if let Some(e) = self.text.editing.as_mut() {
                            e.value = value;
                        }
                    }
                }
            }
            Target::Layer(id) => {
                // 描いている間・読むだけのセット・ロック（すべて・画素・透明部分。親のグループのものも）は、黙って変えない
                // （毎フレーム断りを出さない。値の欄からの変更は、これまでどおり理由を出して断る）
                if !self.can_edit() || self.doc.ensure_pixels_editable(id, true).is_err() {
                    return;
                }
                let Some(mut value) = self.doc.layer(id).and_then(|l| l.text()).cloned() else {
                    return;
                };
                if value.color == color {
                    return;
                }
                value.color = color;
                if self.text_font(&value.font).is_err() {
                    return;
                }
                if !self.text.following {
                    // 前に開いたままのまとめ（別の欄のドラッグ）に混ぜない
                    self.doc.end_coalescing();
                }
                self.text.following = true;
                self.text_set_layer(id, value, true);
            }
        }
    }

    /// 値を 1 つ変える。`coalesce` ならテキストレイヤーの変更を前の変更とまとめる（色のウィンドウのドラッグ。終わりは `m2_end_drag`）。
    fn text_set(&mut self, field: Field, coalesce: bool) {
        match self.text_target() {
            Target::Defaults => field.apply(&mut self.text.defaults),
            Target::Editing => {
                let Some(e) = self.text.editing.as_ref() else {
                    return;
                };
                let mut value = e.value.clone();
                field.apply(&mut value);
                if e.layer.is_none() {
                    // まだレイヤーが無い: 値だけ（次に作る文字の既定にも）
                    field.apply(&mut self.text.defaults);
                    if let Some(e) = self.text.editing.as_mut() {
                        e.value = value;
                    }
                } else {
                    let layer = e.layer;
                    self.text_redraw(layer, value);
                }
            }
            Target::Layer(id) => {
                if self.text_refused() {
                    return;
                }
                let Some(mut value) = self.doc.layer(id).and_then(|l| l.text()).cloned() else {
                    return;
                };
                field.apply(&mut value);
                self.text_set_layer(id, value, coalesce);
            }
        }
    }

    /// テキストレイヤーの値を 1 回の Undo で変える（`coalesce` なら前のまとめに加える）。フォントが見つからなければ理由を添えて断る。
    pub fn text_set_layer(&mut self, id: LayerId, mut value: TextSettings, coalesce: bool) {
        let font = match self.text_font(&value.font) {
            Ok((found, bytes)) => {
                value.font = found;
                bytes
            }
            Err(reason) => {
                self.refuse(Source::Text, self.text_cannot(&reason));
                return;
            }
        };
        match self.doc.set_text(id, value, &font, coalesce) {
            Ok(()) => self.modified = true,
            Err(e) => self.notify(Kind::of_core(&e), Source::Text, self.lang.core_error(&e)),
        }
    }

    /// 選んだフォントのファイル（OS の一覧からも）を、当て先のフォントにする。
    fn text_font_file(&mut self, path: &Path, index: u32) {
        let lang = self.lang;
        let Ok(bytes) = sysfonts::read_font_file(path) else {
            self.refuse(
                Source::Text,
                lang.with_reason(
                    lang.pick("フォントを読めません", "Cannot read the font"),
                    lang.quote(&path.to_string_lossy()),
                ),
            );
            return;
        };
        if let Err(e) = text::check_font(&bytes, index) {
            self.refuse(Source::Text, lang.core_error(&e));
            return;
        }
        let font = text::file_font(&path.to_string_lossy(), index, &bytes);
        self.text_remember(
            &font,
            Lookup::Same {
                font: font.clone(),
                bytes,
            },
        );
        self.text_set(Field::Font(font), false);
    }

    fn text_rasterize(&mut self, id: LayerId) {
        let lang = self.lang;
        if self
            .text
            .editing
            .as_ref()
            .is_some_and(|e| e.layer == Some(id))
        {
            self.text_commit();
        }
        if self.text_refused() || self.doc.layer(id).is_none_or(|l| l.text().is_none()) {
            return;
        }
        match self.doc.rasterize(id) {
            Ok(()) => {
                self.modified = true;
                self.info(
                    Source::Text,
                    lang.pick("ラスタライズしました。", "Rasterized."),
                );
            }
            Err(e) => self.notify(Kind::of_core(&e), Source::Text, lang.core_error(&e)),
        }
    }

    /// 打っている文字の並べ（フォントが読めなければ None）。
    pub fn text_layout(&mut self) -> Option<Arc<TextLayout>> {
        let value = self.text.editing.as_ref()?.value.clone();
        if let Some((v, l)) = &self.text.editing.as_ref()?.layout {
            if *v == value {
                return Some(l.clone());
            }
        }
        let (found, font) = self.text_font(&value.font).ok()?;
        let layout = Arc::new(text::layout(&value, &font, found.index()).ok()?);
        if let Some(e) = self.text.editing.as_mut() {
            e.layout = Some((value, layout.clone()));
        }
        Some(layout)
    }

    /// テキストレイヤーの並べ（箱の当たりと外枠。フォントが読めなければ None）。
    pub fn text_layer_layout(&mut self, id: LayerId) -> Option<(TextSettings, TextLayout)> {
        let value = self.doc.layer(id)?.text()?.clone();
        let (found, font) = self.text_font(&value.font).ok()?;
        let layout = text::layout(&value, &font, found.index()).ok()?;
        Some((value, layout))
    }
}

impl Editing {
    fn new(layer: Option<LayerId>, created: bool, value: TextSettings) -> Editing {
        Editing {
            layer,
            created,
            buffer: value.text.clone(),
            value,
            focus_wanted: true,
            had_focus: false,
            cursor: None,
            move_cursor: None,
            layout: None,
        }
    }
}

/// 箱の中の点（基準の点から、回転を戻した座標。y は上向き）。
pub fn to_local(value: &TextSettings, x: f64, y: f64) -> (f64, f64) {
    let (dx, dy) = (x - value.x, y - value.y);
    let r = -value.rotation.to_radians();
    (dx * r.cos() - dy * r.sin(), dx * r.sin() + dy * r.cos())
}

/// 箱の中の座標をキャンバスの座標へ。
pub fn to_document(value: &TextSettings, lx: f64, ly: f64) -> (f64, f64) {
    let r = value.rotation.to_radians();
    (
        value.x + lx * r.cos() - ly * r.sin(),
        value.y + lx * r.sin() + ly * r.cos(),
    )
}

/// 並べの箱（左, 下, 右, 上。基準の点から）。文字が無ければ、大きさ 1 文字ぶんの箱。
pub fn layout_box(value: &TextSettings, layout: &TextLayout) -> [f64; 4] {
    let b = layout.bounds();
    if b == [0.0; 4] {
        [
            0.0,
            -layout.line_advance.max(value.size),
            value.size * 0.6,
            0.0,
        ]
    } else {
        let mut b = b;
        if value.wrap_width > 0.0 {
            b[0] = b[0].min(0.0);
            b[2] = b[2].max(value.wrap_width);
        }
        b
    }
}

/// 縦横の倍率の差・傾きを一様とみなす相対の許容（ドラッグ・数値の入力の丸めより十分小さい）。
const UNIFORM_TOLERANCE: f64 = 1e-9;

/// テキストレイヤーを移動・変形した値（画素でなく基準の点・回転・サイズを動かす）。反転と、縦横で倍率が違う・傾く変形は断る（理由）。
/// 値はサイズ 1 つと回転 1 つしか持たないので、拡大は一様なものだけ受ける（`from_parts` の回転と軸ごとの拡大の積なら、
/// 2 つの列の長さが等しく、直交しているときだけ一様）。
pub fn transformed(
    lang: Lang,
    value: &TextSettings,
    t: &yolu_core::Affine2D,
) -> Result<TextSettings, String> {
    let det = t.a * t.d - t.b * t.c;
    if det <= 0.0 {
        return Err(lang.with_reason(
            lang.pick(
                "テキストレイヤーは反転できません",
                "Cannot flip a text layer",
            ),
            lang.pick("テキストの値は反転を持ちません", "text values have no flip"),
        ));
    }
    let (long, short) = (t.a.hypot(t.c), t.b.hypot(t.d));
    let skew = t.a * t.b + t.c * t.d;
    if (long - short).abs() > UNIFORM_TOLERANCE * long.max(short)
        || skew.abs() > UNIFORM_TOLERANCE * long * short
    {
        return Err(lang.with_reason(
            lang.pick(
                "テキストレイヤーは縦横の比を変えられません",
                "Cannot change the aspect ratio of a text layer",
            ),
            lang.pick(
                "サイズは縦横で共通で、傾きも持たない",
                "the size is one value for both directions and there is no skew",
            ),
        ));
    }
    let mut next = value.clone();
    let (x, y) = (
        t.a * value.x + t.b * value.y + t.tx,
        t.c * value.x + t.d * value.y + t.ty,
    );
    next.x = x.clamp(-text::MAX_COORDINATE, text::MAX_COORDINATE);
    next.y = y.clamp(-text::MAX_COORDINATE, text::MAX_COORDINATE);
    let turn = t.c.atan2(t.a).to_degrees();
    let mut rotation = value.rotation + turn;
    while rotation > 180.0 {
        rotation -= 360.0;
    }
    while rotation <= -180.0 {
        rotation += 360.0;
    }
    next.rotation = rotation;
    let scale = det.sqrt();
    if (scale - 1.0).abs() > 1e-9 {
        next.size = (value.size * scale).clamp(text::MIN_SIZE, text::MAX_SIZE);
        next.wrap_width = (value.wrap_width * scale).clamp(0.0, text::MAX_COORDINATE);
    }
    Ok(next)
}
