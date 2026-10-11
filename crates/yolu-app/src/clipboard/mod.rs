//! レイヤーの画素のコピー・カット・ペースト（編集メニューと Ctrl+C / Ctrl+X / Ctrl+V / Ctrl+Shift+C）。
//!
//! 計算と断りは core（`copy_pixels`・`cut_pixels`・`copy_merged`・`paste_as_layer`）に任せ、ここは「どのレイヤーの何を」（選んでいるレイヤー・描く
//! チャンネル・マスクを描いているか）と、アプリの中のクリップボード、OS のクリップボードの画像との受け渡し、知らせだけを持つ。
//!
//! アプリの中のクリップボード（core の `PixelClipboard`）が基本: 透明画素の RGB・写した位置・出どころのチャンネルまで正確に持つ。
//! コピー・カットは同じ画素を OS のクリップボードへも画像で書き、ペーストは OS の画像が「最後に書いた画像」と違えば外の画像
//! （ほかのアプリでコピーしたもの）を貼り、同じ・画像が無い・読めないならアプリの中の写しを貼る（読めなかったときは、その理由を
//! 知らせに添える）。OS への書き込みに失敗したあとは、そのとき OS に残っていた古い画像が OS にある間、新しくコピーした写しを
//! 優先する（古い外の画像を貼らない）。外の画像はキャンバスより大きければ、行を並べ替える前に断る（大きさを合わせる操作は無い）。
//! OS 側は [`os::OsClipboard`] の口で、試験は差し替える。

pub mod image;
pub mod keys;
pub mod os;

use crate::engine::{Channel, LayerKind, PixelClipboard};
use crate::notice::Source;
use crate::state::{Action, AppState};
use crate::ui::menu::Entry;
use os::{ClipImage, NoClipboard, OsClipboard, OsClipboardError, SystemClipboard};

/// 編集メニューとキーの操作。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipAction {
    /// 選んでいるレイヤーの今のチャンネル（マスクを描いているならマスク）の、選択範囲の中（無ければ全体）を写す。
    Copy,
    /// 写してから消す（1 回の Undo）。
    Cut,
    /// 見えている合成（今のチャンネル）を写す。
    CopyMerged,
    /// 新しいレイヤーとして貼る（1 回の Undo）。
    Paste,
}

/// OS への書き込みに失敗した時点で OS にあった画像。それが OS に残っている間は、新しくコピーしたアプリの中の写しを貼る
/// （古い外の画像が、コピーしたばかりの画素より優先されない）。OS の画像が変わったら（別の画像・画像なし）印を外す。
/// OS の変化は通知されないので、貼るときに見るだけ: 貼る前に別の画像を挟んでまた元の画像に戻すと、変わったことに気づけない。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stale {
    /// 失敗の時点の画像が読めなかった。次に読めた画像を、その画像とみなす。
    Unknown,
    /// 失敗の時点の画像の指紋。
    Image(u64),
}

/// クリップボードの状態（アプリの中の写しと、OS との口）。
pub struct ClipState {
    /// アプリの中のクリップボード。
    pub pixels: Option<PixelClipboard>,
    os: Box<dyn OsClipboard>,
    /// 最後に OS へ書いた画像の指紋（今 OS にある画像が自分の書いたものかを見る）。
    written: Option<u64>,
    /// 書き込みに失敗したあとの、OS に残っている古い画像（[`Stale`]）。
    stale: Option<Stale>,
    /// Ctrl+V を処理した印（V を離したときの二重の貼り付けを避ける。`keys` が使う）。
    pub(crate) v_handled: bool,
    /// 前のフレームの終わりの修飾キー（`keys` が、フレームの中の事象の時点の Shift を見るのに使う）。
    pub(crate) last_modifiers: egui::Modifiers,
}

impl Default for ClipState {
    /// OS には繋がない（試験と画面の無い場面）。ウィンドウのアプリは [`ClipState::use_system`] で繋ぐ。
    fn default() -> Self {
        ClipState {
            pixels: None,
            os: Box::new(NoClipboard),
            written: None,
            stale: None,
            v_handled: false,
            last_modifiers: egui::Modifiers::NONE,
        }
    }
}

impl ClipState {
    /// OS のクリップボードへの口を替える（試験は [`os::MemoryClipboard`]）。
    pub fn set_os(&mut self, os: Box<dyn OsClipboard>) {
        self.os = os;
        self.written = None;
        self.stale = None;
    }

    /// OS のクリップボードへの口を手放す（OS には繋がない状態に戻す）。本物（`SystemClipboard`）は手放すとき、書いた画像が X11 の
    /// クリップボードの管理に渡るのを少しだけ待つので、デストラクターを走らせずにプロセスを終えるときは、先にこれを呼ぶ。
    pub fn release_os(&mut self) {
        self.set_os(Box::new(NoClipboard));
    }

    /// 本物の OS のクリップボードに繋ぐ（スレッドは最初に使うときに起きる）。
    pub fn use_system(&mut self) {
        self.set_os(Box::new(SystemClipboard::new()));
    }

    /// 写しを覚え、同じ画素を OS へ画像で書く（待たない）。
    fn store(&mut self, clip: PixelClipboard) {
        let image = image::to_image(&clip);
        self.written = Some(image::fingerprint(&image));
        self.stale = None;
        self.pixels = Some(clip);
        self.os.write_image(image);
    }

    /// OS への書き込みの失敗を受けたとき: そのとき OS にある画像（コピーしたばかりの画素より古い外の画像や前に書いた画像）を、
    /// OS に残っている間は貼らない画像として覚える。
    fn write_failed(&mut self) {
        self.stale = match self.os.read_image() {
            Ok(Some(image)) => Some(Stale::Image(image::fingerprint(&image))),
            Ok(None) => None,
            Err(_) => Some(Stale::Unknown),
        };
    }

    /// 今 OS にある画像が、外の画像でなくアプリの中の写しを貼るべきものなら、その写し: 自分が最後に書いた画像、または書き込みに
    /// 失敗した時点で残っていた古い画像。指紋は画像を 1 度なぞるだけで、確保はしない（比べる相手がいなければ、なぞらない）。
    fn own_for(&mut self, image: &ClipImage) -> Option<PixelClipboard> {
        if self.written.is_none() && self.stale.is_none() {
            return None;
        }
        let print = image::fingerprint(image);
        let stale = match self.stale {
            Some(Stale::Image(old)) => old == print,
            Some(Stale::Unknown) => true,
            None => false,
        };
        self.stale = stale.then_some(Stale::Image(print));
        if stale || self.written == Some(print) {
            self.pixels.clone()
        } else {
            None
        }
    }
}

/// 貼る写しと、OS のクリップボードを読めなかった理由（読めなかったときは、アプリの中の写しを貼る）。
struct PasteSource {
    clip: PixelClipboard,
    os_error: Option<OsClipboardError>,
}

impl AppState {
    /// 操作を当てる（`Action::Clip`。読むだけのセットは `Action::apply` が先に断る）。断られたら何も変えず、理由をステータスバーへ。
    pub fn clip_action(&mut self, action: ClipAction) {
        if self.is_stroking() {
            self.refuse(
                Source::Clipboard,
                crate::lang::refusals::during_stroke(self.lang),
            );
            return;
        }
        let revision = self.doc.revision();
        let result = match action {
            ClipAction::Copy => self.clip_copy(),
            ClipAction::Cut => self.clip_cut(),
            ClipAction::CopyMerged => self.clip_copy_merged(),
            ClipAction::Paste => self.clip_paste(),
        };
        // 断られた理由（選んでいない・写した物が無い・ロック）は断り
        match result {
            Ok(text) => self.info(Source::Clipboard, text),
            Err(text) => self.refuse(Source::Clipboard, text),
        }
        if self.doc.revision() != revision {
            self.modified = true;
        }
    }

    /// OS への書き込みの失敗を受ける（フレームの初めに）。
    pub fn poll_clipboard(&mut self) {
        if let Some(error) = self.clip.os.take_error() {
            self.clip.write_failed();
            let text = error.text(self.lang);
            self.fail(
                Source::Clipboard,
                self.lang.with_reason(
                    self.lang.pick(
                        "画像を OS へコピーできません",
                        "Cannot copy the image to the OS",
                    ),
                    text,
                ),
            );
        }
    }

    /// 写す先のレイヤーとチャンネル、マスクを写すか。
    pub(crate) fn clip_target(&self) -> Result<(crate::engine::LayerId, Channel, bool), String> {
        let id = self
            .selected_layer
            .filter(|id| self.doc.layer(*id).is_some())
            .ok_or_else(|| {
                self.lang
                    .pick("レイヤーが選ばれていません。", "No layer is selected.")
                    .to_string()
            })?;
        let mask = self.m2.edit_mask && self.doc.layer(id).is_some_and(|l| l.mask().is_some());
        Ok((id, self.m2.paint_channel, mask))
    }

    fn clip_limit(&self) -> u64 {
        self.doc.stroke_budget_bytes()
    }

    fn clip_copy(&mut self) -> Result<String, String> {
        let (id, channel, mask) = self.clip_target()?;
        let clip = self
            .doc
            .copy_pixels(id, channel, mask, self.clip_limit())
            .map_err(|e| self.lang.core_error(&e))?;
        let text = copied_text(self.lang, &clip, false);
        self.clip.store(clip);
        Ok(text)
    }

    fn clip_copy_merged(&mut self) -> Result<String, String> {
        let clip = self
            .doc
            .copy_merged(self.m2.paint_channel, self.clip_limit())
            .map_err(|e| self.lang.core_error(&e))?;
        let text = copied_text(self.lang, &clip, false);
        self.clip.store(clip);
        Ok(text)
    }

    fn clip_cut(&mut self) -> Result<String, String> {
        let (id, channel, mask) = self.clip_target()?;
        let clip = self
            .doc
            .cut_pixels(id, channel, mask, self.clip_limit())
            .map_err(|e| self.lang.core_error(&e))?;
        let text = copied_text(self.lang, &clip, true);
        self.clip.store(clip);
        self.ensure_selection();
        Ok(text)
    }

    /// 貼る写し: OS の画像が自分の書いたものでなければ外の画像、そうでなければアプリの中の写し。外の画像がキャンバスより大きければ、
    /// 行を並べ替える前（読んだ画像のほかに何も確保しない）に断る。
    fn clip_source(&mut self) -> Result<PasteSource, String> {
        let lang = self.lang;
        let from_app = |clip: &Option<PixelClipboard>, os_error, empty: String| {
            clip.clone()
                .map(|clip| PasteSource { clip, os_error })
                .ok_or(empty)
        };
        match self.clip.os.read_image() {
            Ok(Some(image)) => {
                if let Some(clip) = self.clip.own_for(&image) {
                    return Ok(PasteSource {
                        clip,
                        os_error: None,
                    });
                }
                let (doc_w, doc_h) = (self.doc.width(), self.doc.height());
                if image.width > doc_w || image.height > doc_h {
                    return Err(lang.pick(
                        format!(
                            "画像がテクスチャセットより大きい（{} × {} > {} × {}）",
                            image.width, image.height, doc_w, doc_h
                        ),
                        format!(
                            "Image is larger than the texture set ({} × {} > {} × {})",
                            image.width, image.height, doc_w, doc_h
                        ),
                    ));
                }
                image::from_image(image, self.m2.paint_channel)
                    .map(|clip| PasteSource {
                        clip,
                        os_error: None,
                    })
                    .map_err(|e| lang.core_error(&e))
            }
            Ok(None) => {
                self.clip.stale = None;
                from_app(
                    &self.clip.pixels,
                    None,
                    lang.pick("クリップボードが空です。", "The clipboard is empty.")
                        .to_string(),
                )
            }
            Err(error) => from_app(&self.clip.pixels, Some(error), error.text(lang).to_string()),
        }
    }

    fn clip_paste(&mut self) -> Result<String, String> {
        let lang = self.lang;
        let PasteSource { clip, os_error } = self.clip_source()?;
        let name = self.layer_name_for_new();
        let above = self
            .selected_layer
            .filter(|id| self.doc.layer(*id).is_some());
        // 大きさの上限は core の予算（0 でないタイルの実際の大きさ）に任せる。矩形の大きさで先に断ると、まばらな画像まで断る
        let result = self
            .doc
            .paste_as_layer(&clip, self.m2.paint_channel, Some(&name), above)
            .map_err(|e| lang.core_error(&e))?;
        self.selected_layer = Some(result.layer);
        self.set_edit_mask(false);
        self.ensure_selection();
        let mut notes = Vec::new();
        if result.centered {
            notes.push(lang.pick("中央", "centred").to_string());
        }
        if result.clipped_pixels > 0 {
            notes.push(lang.pick(
                format!("はみ出した {} 画素を切り落とし", result.clipped_pixels),
                format!("{} pixels cut off", result.clipped_pixels),
            ));
        }
        if let Some(error) = os_error {
            notes.push(error.text(lang).to_string());
        }
        Ok(pasted_text(lang, &name, &notes))
    }
}

/// ペーストした知らせ: 名前に、あれば補足（中央・切り落とし・OS を読めなかった理由）を括弧で並べる。コピーの知らせと同じく
/// 文の形にしない（句点・ピリオドを付けない）。
fn pasted_text(lang: crate::lang::Lang, name: &str, notes: &[String]) -> String {
    let head = lang.pick(
        format!("ペーストしました: {name}"),
        format!("Pasted as {name}"),
    );
    if notes.is_empty() {
        return head;
    }
    let (open, separator, close) = lang.pick(("（", "、", "）"), (" (", ", ", ")"));
    format!("{head}{open}{}{close}", notes.join(separator))
}

/// コピー・カットした知らせ（出どころと大きさ）。
fn copied_text(lang: crate::lang::Lang, clip: &PixelClipboard, cut: bool) -> String {
    use crate::engine::ClipboardSource::*;
    let size = format!("{} × {}", clip.width(), clip.height());
    match (clip.source(), cut) {
        (Mask, true) => lang.pick(
            format!("マスクをカットしました（{size}）"),
            format!("Cut mask ({size})"),
        ),
        (Mask, false) => lang.pick(
            format!("マスクをコピーしました（{size}）"),
            format!("Copied mask ({size})"),
        ),
        (Composite, _) => lang.pick(
            format!("結合してコピーしました（{size}）"),
            format!("Copied merged ({size})"),
        ),
        (_, true) => lang.pick(format!("カットしました（{size}）"), format!("Cut ({size})")),
        _ => lang.pick(
            format!("コピーしました（{size}）"),
            format!("Copied ({size})"),
        ),
    }
}

/// 編集メニューのクリップボードの項目（カット・コピー・結合してコピー・ペースト）。描いている間と読むだけのセットでは使えない。
pub fn menu_entries(app: &AppState) -> Vec<Entry<Action>> {
    let l = app.lang;
    let free = app.can_edit();
    let layer = app
        .selected_layer
        .and_then(|id| app.doc.layer(id))
        .map(|layer| {
            let mask = app.m2.edit_mask && layer.mask().is_some();
            (mask, layer.kind())
        });
    let copyable = layer
        .is_some_and(|(mask, kind)| mask || matches!(kind, LayerKind::Raster | LayerKind::Fill));
    let cuttable = layer.is_some_and(|(mask, kind)| mask || kind == LayerKind::Raster);
    let item = |name, action, command: &str, on: bool| {
        Entry::item(name, Action::Clip(action))
            .command_key(command)
            .enabled(on)
    };
    vec![
        item(
            l.pick("カット", "Cut"),
            ClipAction::Cut,
            "clip.cut",
            free && cuttable,
        ),
        item(
            l.pick("コピー", "Copy"),
            ClipAction::Copy,
            "clip.copy",
            free && copyable,
        ),
        item(
            l.pick("結合してコピー", "Copy Merged"),
            ClipAction::CopyMerged,
            "clip.copy_merged",
            free,
        ),
        item(
            l.pick("ペースト", "Paste"),
            ClipAction::Paste,
            "clip.paste",
            free,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    /// 手放されたこと（`Drop`）だけを覚える OS のクリップボード。
    struct Released(Arc<AtomicBool>);
    impl OsClipboard for Released {
        fn read_image(&mut self) -> Result<Option<ClipImage>, OsClipboardError> {
            Ok(None)
        }
        fn write_image(&mut self, _image: ClipImage) {}
        fn take_error(&mut self) -> Option<OsClipboardError> {
            None
        }
    }
    impl Drop for Released {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    /// デストラクターを走らせずにプロセスを終えるとき、`release_os` が OS への口を手放す（本物は、書いた画像が X11 のクリップボードの管理に
    /// 渡るのを待ってから手放す）。
    #[test]
    fn releasing_the_os_connection_drops_it() {
        let dropped = Arc::new(AtomicBool::new(false));
        let mut clip = ClipState::default();
        clip.set_os(Box::new(Released(dropped.clone())));
        assert!(!dropped.load(Ordering::SeqCst));
        clip.release_os();
        assert!(dropped.load(Ordering::SeqCst), "口を手放した");
        // 手放したあとにもう一度呼んでも、何も繋がっていないだけ（落ちない）
        clip.release_os();
    }
}
