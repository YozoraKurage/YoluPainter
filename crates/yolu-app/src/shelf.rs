//! アセットの棚（Substance のシェルフ）の状態と操作。棚は .ylp の resources（yolu-io の `Shelf`）で、画像・ブラシ・マテリアル・
//! スマートマテリアル・スマートマスクを並べる。画面（`panels::assets`）は絞り込み・探す・選ぶだけを直に持ち、棚の中身と文書を変える
//! 操作は `Action::Shelf(ShelfOp)` を通す。
//!
//! - 組み込み: 同梱のスマートマテリアル（`yolu_core::smart_library`）は、棚の項目の後ろに「組み込み」として並ぶ。プロジェクトの棚には入らず
//!   （保存・書き出し・消すの対象外）、置くと文書へ写る。ID は `builtin:` で始まり、素材は項目を見るとき・置くときにコードから組む。
//! - 層からの保存: 選んだ層（グループなら中身ごと）・層のマスクを core で捕まえ、.ylsmart（`SmartFile::from_core`）にして棚へ入れる。
//!   文書は変えない。棚の変更は文書の Undo の履歴に入らず、未保存にはなる（Unity 版と同じ）。
//! - 置く: 棚の .ylsmart を core の素材へ戻し、`place_smart_material`（層の組。1 回の Undo）・`apply_smart_mask`（マスクの入れ替え。
//!   1 回の Undo）で今の文書へ。置けないときは何も変えず、理由を短く出す（画像入り・Generator の再固定・core が持てない中身・
//!   チャンネルの不一致・予算）。
//! - 画面のスレッドを止める時間: 棚へ足す・消す・読み込むは、yolu-io が棚全体を検証し直すので棚の大きさに比例する。大きな素材
//!   （画素 8 MiB 以上）か大きな棚（素材と棚の使用量の合計が 8 MiB 以上）への「層を保存」は、変換・圧縮・サムネイルに加えて
//!   棚の写しへの追加も別のスレッドで済ませ、画面のスレッドは足した後の棚に差し替えるだけにする（その間は消す・読み込むを断る）。
//!   消す・読み込むは画面のスレッドで行い、棚が 240〜420 MiB だと、消すは 1.4〜2.8 秒、読み込みは 2〜4 秒止まる。サムネイルのための
//!   展開は 1 項目 0.1 秒前後（画素 64 MiB の素材で 0.5 秒。1 フレームに 2 個まで）。測定は tests/shelf.rs の `measure_`
//!   （最適化 1 のビルドの値。本番のビルドでは短くなる）。
//! - 棚を .ylp に書くのは「変えたとき」だけ（`write_into`）。変えていなければ、開いたファイルの resources をバイト列のまま残す。
//! - サムネイルと一覧の説明は、棚の項目を最初に見るときに 1 度だけ作って覚える（`Inspected`。作るのは 1 フレームに数個まで）。
//!   画素は文書の合成の参照の式で 64 点に標本化する（保存の正本には使わない）。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use egui::{ColorImage, TextureHandle, TextureOptions};
use yolu_core::smart::{SmartKind, SmartMaterial, SmartPlacement};
use yolu_core::smart_library;
use yolu_io::shelf::{
    ResourceKind, Shelf, MAX_RESOURCES, REFUSAL_ARCHIVE_BUDGET, REFUSAL_MEMORY_BUDGET,
    REFUSAL_RESOURCE_COUNT,
};
use yolu_io::smart::{
    SmartFile, REFUSAL_GENERATORS, REFUSAL_IMAGES, REFUSAL_RUST_ADJUSTMENTS,
    REFUSAL_RUST_GENERATORS, REFUSAL_USER_CHANNELS,
};
use yolu_io::{NativeValue, Project, Resource};

use crate::engine::{Channel, CoreError, Document, LayerId};
use crate::lang::Lang;
use crate::state::{AppState, DialogRequest};

/// 棚の素材のメモリの予算（Unity 版の既定は RAM の 1/16 を 256〜4096 MiB に収めた値。ここでは固定）。
pub const SHELF_BUDGET: u64 = 512 * 1024 * 1024;
/// 棚に入れる .ylsmart の大きさの上限（読み込みの前に長さで断る）。
pub const IMPORT_LIMIT: u64 = 256 * 1024 * 1024;
/// サムネイルの 1 辺（点）。
pub const THUMB: u32 = 64;
/// 素材のサムネイルのために展開してよい画素の量（バイト。超える素材は種類のアイコンで見せるが、置くことはできる）。
pub const PREVIEW_BUDGET: u64 = 128 * 1024 * 1024;
/// 画像のサムネイルを作る画素数の上限（これを超える画像は種類のアイコンで見せる）。
pub const IMAGE_THUMB_PIXELS: u64 = 4096 * 4096;

/// 同梱の素材の項目の ID の前置き（プロジェクトの棚の ID は UUID なので重ならない）。
pub const BUILTIN_PREFIX: &str = "builtin:";

/// 同梱の素材の項目か。
pub fn is_builtin(id: &str) -> bool {
    id.starts_with(BUILTIN_PREFIX)
}

/// 棚の素材の種類。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ItemKind {
    Image,
    Brush,
    Material,
    SmartMaterial,
    SmartMask,
}

impl ItemKind {
    pub const ALL: [ItemKind; 5] = [
        ItemKind::Image,
        ItemKind::Brush,
        ItemKind::Material,
        ItemKind::SmartMaterial,
        ItemKind::SmartMask,
    ];

    /// 索引の種類の文字列から。
    pub fn of(kind: &str) -> Option<ItemKind> {
        Some(match kind {
            "image" => ItemKind::Image,
            "brush" => ItemKind::Brush,
            "material" => ItemKind::Material,
            "smartMaterial" => ItemKind::SmartMaterial,
            "smartMask" => ItemKind::SmartMask,
            _ => return None,
        })
    }

    pub fn icon(self) -> &'static str {
        match self {
            ItemKind::Image => "texture",
            ItemKind::Brush => "paint_brush",
            ItemKind::Material => "view_in_ar",
            ItemKind::SmartMaterial => "layers",
            ItemKind::SmartMask => "vignette",
        }
    }

    pub fn name(self, lang: Lang) -> &'static str {
        match self {
            ItemKind::Image => lang.pick("画像", "Images"),
            ItemKind::Brush => lang.pick("ブラシ", "Brushes"),
            ItemKind::Material => lang.pick("マテリアル", "Materials"),
            ItemKind::SmartMaterial => lang.pick("スマートマテリアル", "Smart Materials"),
            ItemKind::SmartMask => lang.pick("スマートマスク", "Smart Masks"),
        }
    }

    /// 1 つの名前（フッター・ツールチップ）。
    pub fn singular(self, lang: Lang) -> &'static str {
        match self {
            ItemKind::Image => lang.pick("画像", "Image"),
            ItemKind::Brush => lang.pick("ブラシ", "Brush"),
            ItemKind::Material => lang.pick("マテリアル", "Material"),
            ItemKind::SmartMaterial => lang.pick("スマートマテリアル", "Smart Material"),
            ItemKind::SmartMask => lang.pick("スマートマスク", "Smart Mask"),
        }
    }

    pub fn is_smart(self) -> bool {
        matches!(self, ItemKind::SmartMaterial | ItemKind::SmartMask)
    }

    fn resource_kind(self) -> Option<ResourceKind> {
        match self {
            ItemKind::SmartMaterial => Some(ResourceKind::SmartMaterial),
            ItemKind::SmartMask => Some(ResourceKind::SmartMask),
            _ => None,
        }
    }
}

/// 置けない理由（項目を見たときにわかる分。チャンネルの不一致や予算は置くときに core が断る）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    /// ブラシ・マテリアルは、ここではまだ置けない（画像は 1 枚のペイント層として置ける）。
    Kind(ItemKind),
    /// 画像を同梱した素材（編集用に開けない）。
    Images,
    /// Generator の再固定を持つ素材。
    Generators,
    /// core が持てない中身（フィルター・Generator・Anchor など）を含む素材。棚には入り、書き出せば原本のまま Unity 版が使える。
    /// 理由は両方の言語で持つ。
    Unsupported(Unreadable),
    /// 読めなかった（壊れている・形式が合わない）。理由は両方の言語で持つ（項目の見方は言語に依らず 1 度だけ覚える）。
    Unreadable(Unreadable),
}

impl Block {
    /// 画面に出す短い理由。
    pub fn reason(&self, lang: Lang) -> String {
        match self {
            Block::Kind(k) => format!(
                "{}{}",
                k.singular(lang),
                lang.pick("はまだ置けません", " cannot be placed yet")
            ),
            Block::Images => lang.pick("画像入りは置けません", "Contains images").into(),
            Block::Generators => lang
                .pick("Generator 付きは置けません", "Has pinned generators")
                .into(),
            Block::Unsupported(e) => e.reason(lang).to_owned(),
            Block::Unreadable(e) => format!(
                "{}: {}",
                lang.pick("読めません", "Unreadable"),
                e.reason(lang)
            ),
        }
    }
}

enum Thumb {
    None,
    Pending(ColorImage),
    Ready(TextureHandle),
}

/// 項目を見て 1 度だけ作る情報（言語に依らない部分）。
pub struct Inspected {
    pub width: u32,
    pub height: u32,
    pub layers: usize,
    pub channels: Vec<Channel>,
    pub block: Option<Block>,
    thumb: Thumb,
}

impl Inspected {
    fn bare(block: Option<Block>) -> Inspected {
        Inspected {
            width: 0,
            height: 0,
            layers: 0,
            channels: Vec::new(),
            block,
            thumb: Thumb::None,
        }
    }
}

/// 棚を読めなかった理由。日本語は yolu-io の診断のまま、英語は種類ごとの短い文（診断の本文は日本語なので、英語の窓には出さない）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unreadable {
    ja: String,
    en: String,
}

impl Unreadable {
    pub fn reason(&self, lang: Lang) -> &str {
        lang.pick(&self.ja, &self.en)
    }

    /// yolu-io の失敗の種類から（日本語は診断のまま、英語は短い文）。
    pub fn from_error(e: &yolu_io::Error) -> Unreadable {
        Unreadable {
            ja: Lang::Ja.io_error(e),
            en: Lang::En.io_error(e),
        }
    }

    /// 両方の言語で同じ文。
    pub fn same(text: &str) -> Unreadable {
        Unreadable {
            ja: text.to_owned(),
            en: text.to_owned(),
        }
    }
}

/// 棚の状態。
pub struct ShelfState {
    shelf: Shelf,
    /// 棚を読めなかった理由（あれば棚を変える操作を断る。保存では開いたファイルの resources をそのまま残す）。
    pub unavailable: Option<Unreadable>,
    /// 開いた・保存した後に変えたか（変えていなければ保存で resources に触らない）。
    pub changed: bool,
    /// 種類の絞り込み（None はすべて）。
    pub filter: Option<ItemKind>,
    pub search: String,
    /// 選んでいる素材の ID。
    pub selected: Option<String>,
    pub scroll: f32,
    /// 消す確認の待ち（ファイルの窓と同じく `YoluApp` が確かめる）。
    pub pending_remove: Option<String>,
    /// 書き出す素材（ファイルを選ぶ窓の待ち）。
    pub export_id: Option<String>,
    /// 別のスレッドで書き出している保存（1 つだけ。取り消すか、終わると外れる）。
    saving: Option<PendingSave>,
    /// 走っている書き出しのスレッドの数。「やめる」はスレッドを止めない（結果を捨てるだけ）ので、やめた後もスレッドが
    /// 終わるまで次の保存を断る（4096² の書き出しが並列に積み上がらない）。プロジェクトを替えても引き継ぐ。
    running: Arc<AtomicUsize>,
    /// 画素がこの大きさ以上の素材は別のスレッドで書き出す。
    pub async_bytes: u64,
    /// 素材のサムネイルのために展開してよい画素の量（バイト。試験で小さくする）。
    pub preview_budget: u64,
    /// 画像のサムネイルを作る画素数の上限（試験で小さくする）。
    pub image_thumb_pixels: u64,
    /// 読み込む .ylsmart の大きさの上限（バイト。試験で小さくする）。
    pub import_limit: u64,
    /// 書き出しが終わったことを画面に知らせる口（毎フレーム `YoluApp` が渡す）。
    pub context: Option<egui::Context>,
    /// 試験用: true の間、別のスレッドの書き出しは結果を渡さずに待つ。
    hold: Arc<AtomicBool>,
    inspected: HashMap<String, Inspected>,
    /// 同梱の素材の項目（今の言語の名前。`use_language` で作り直す）。
    builtin: Vec<Resource>,
    builtin_lang: Option<Lang>,
    /// 同梱の素材を一覧に出すか（既定は出す。プロジェクトの棚だけの並びを調べる試験が false にする）。
    pub show_builtin: bool,
}

/// 別のスレッドで書き出している素材。
struct PendingSave {
    name: String,
    kind: ItemKind,
    rx: std::sync::mpsc::Receiver<Result<Staged, yolu_io::Error>>,
    /// やめた・捨てたことをスレッドへ伝える旗（サムネイルを作らず切り上げる。変換と圧縮の途中では止められない）。
    cancel: Arc<AtomicBool>,
}

impl Drop for PendingSave {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::SeqCst);
    }
}

/// 走っている書き出しのスレッドの数え（作った時に増え、落ちた時に減る。パニックでも減る）。
struct Running(Arc<AtomicUsize>);

impl Running {
    fn start(count: &Arc<AtomicUsize>) -> Running {
        count.fetch_add(1, Ordering::SeqCst);
        Running(count.clone())
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// 書き出し終えた素材（.ylsmart のバイト列と、一覧に出す情報・サムネイル）。
struct Encoded {
    bytes: Vec<u8>,
    /// 一覧の情報（無ければ最初に見るときに作る）。
    info: Option<Inspected>,
}

/// 素材を棚の写しへ足した結果。`next` が足した後の棚（同じ中身が既にあったときは足す前のまま）。足すときの棚全体の検証は棚の
/// 大きさに比例して長いので、別のスレッドの保存では、そちらで済ませて画面のスレッドは `next` に差し替えるだけにする。
struct Staged {
    next: Shelf,
    id: String,
    added: bool,
    info: Option<Inspected>,
}

/// 棚の写し `shelf` へ素材を足す（拒否では何も返さない。元の棚は変わらない）。
fn stage(
    mut shelf: Shelf,
    name: &str,
    kind: ItemKind,
    done: Encoded,
) -> Result<Staged, yolu_io::Error> {
    let before = shelf.resources().len();
    let id = shelf.add_file_without_origin(
        &new_resource_id(),
        name,
        kind.resource_kind().expect("スマートな種類"),
        &done.bytes,
    )?;
    let added = shelf.resources().len() > before;
    Ok(Staged {
        next: shelf,
        id,
        added,
        info: done.info,
    })
}

/// この大きさ（画素のバイト）以上の素材の書き出しは別のスレッドで行う（Unity 版と同じ 8 MiB）。
pub const ASYNC_SAVE_BYTES: u64 = 8 * 1024 * 1024;

impl Default for ShelfState {
    fn default() -> Self {
        ShelfState::with_shelf(Shelf::new(SHELF_BUDGET))
    }
}

impl ShelfState {
    pub fn with_shelf(shelf: Shelf) -> ShelfState {
        ShelfState {
            shelf,
            unavailable: None,
            changed: false,
            filter: None,
            search: String::new(),
            selected: None,
            scroll: 0.0,
            pending_remove: None,
            export_id: None,
            saving: None,
            running: Arc::new(AtomicUsize::new(0)),
            async_bytes: ASYNC_SAVE_BYTES,
            preview_budget: PREVIEW_BUDGET,
            image_thumb_pixels: IMAGE_THUMB_PIXELS,
            import_limit: IMPORT_LIMIT,
            context: None,
            hold: Default::default(),
            inspected: HashMap::new(),
            builtin: Vec::new(),
            builtin_lang: None,
            show_builtin: true,
        }
    }

    /// 同梱の素材の項目の名前を言語に合わせる（変わったときだけ作り直す。サムネイルは言語に依らないので捨てない）。
    pub fn use_language(&mut self, lang: Lang) {
        if self.builtin_lang == Some(lang) {
            return;
        }
        let japanese = lang == Lang::Ja;
        self.builtin = smart_library::entries()
            .iter()
            .map(|e| {
                let id = format!("{BUILTIN_PREFIX}{}", e.id);
                let name = e.name(japanese).to_owned();
                Resource {
                    // 棚のファイルの索引の項目ではない（読む所は `metadata` の寸法だけで、無ければ 0）
                    metadata: Default::default(),
                    id,
                    kind: "smartMaterial".into(),
                    name,
                    content: String::new(),
                    entry: String::new(),
                }
            })
            .collect();
        self.builtin_lang = Some(lang);
    }

    /// 同梱の素材を今の言語で組む（棚に無い ID・組めないときは None）。
    pub fn build_builtin(&self, id: &str, lang: Lang) -> Option<SmartMaterial> {
        let entry = smart_library::entries()
            .iter()
            .find(|e| format!("{BUILTIN_PREFIX}{}", e.id) == id)?;
        let japanese = lang == Lang::Ja;
        entry.build(entry.name(japanese), japanese).ok()
    }

    /// 試験用: true の間、別のスレッドの書き出しは結果を渡さずに待つ（途中の画面・取り消しを確かめるため）。
    pub fn hold_saves(&self, hold: bool) {
        self.hold.store(hold, Ordering::SeqCst);
    }

    /// 走っている書き出しのスレッドの数（やめた保存のスレッドも、終わるまで数える）。
    pub fn saves_running(&self) -> usize {
        self.running.load(Ordering::SeqCst)
    }

    /// 走っている書き出しのスレッドがすべて終わるまで待つ（試験用）。
    pub fn wait_idle(&self) {
        while self.saves_running() > 0 {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    /// プロジェクトを替えるとき、走っているスレッドの数えを引き継ぐ（新しい棚でも前のスレッドが終わるまで次の保存は始めない）。
    /// 試験用の「待つ」旗も同じものにする（新しい棚から前のスレッドを放せる）。
    pub fn inherit_running_from(mut self, old: &ShelfState) -> ShelfState {
        self.running = old.running.clone();
        self.hold = old.hold.clone();
        self
    }

    /// 別のスレッドで保存している素材の名前（保存中でなければ None）。
    pub fn saving_name(&self) -> Option<&str> {
        self.saving.as_ref().map(|p| p.name.as_str())
    }

    /// 次の保存を始められない理由（書き出しが走っている間。やめた保存のスレッドが終わるまでも）。
    pub fn busy_reason(&self, lang: Lang) -> Option<&'static str> {
        if self.saving.is_some() {
            Some(lang.pick("保存中です", "Already saving"))
        } else if self.saves_running() > 0 {
            Some(lang.pick("やめた保存を終えています", "Finishing the cancelled save"))
        } else {
            None
        }
    }

    /// 開いた .ylp の棚。読めなければ空の棚にして理由を覚える（棚を変える操作を断り、保存では resources に触らない）。
    pub fn from_project(project: &Project) -> ShelfState {
        ShelfState::from_read(project.shelf(SHELF_BUDGET))
    }

    /// 棚を読んだ結果から。読めなければ空の棚にして理由を覚える。
    pub fn from_read(read: Result<Shelf, yolu_io::Error>) -> ShelfState {
        match read {
            Ok(shelf) => ShelfState::with_shelf(shelf),
            Err(e) => ShelfState {
                unavailable: Some(Unreadable::from_error(&e)),
                ..ShelfState::default()
            },
        }
    }

    /// 読めなかった棚（空。棚を変える操作は断る）。理由は両方の言語で同じ文（試験用。実際は `from_read` が種類から作る）。
    pub fn unreadable(reason: &str) -> ShelfState {
        ShelfState {
            unavailable: Some(Unreadable::same(reason)),
            ..ShelfState::default()
        }
    }

    pub fn shelf(&self) -> &Shelf {
        &self.shelf
    }

    /// 画像（左下原点・straight RGBA8）を棚へ足す（出どころの記録なし。外のパスを .ylp に書かない）。同じ中身が既にあれば、その ID
    /// （棚は変わらない）。足したら棚は「変えた」になる。棚を読めなかった・予算や個数の上限・壊れた画素は理由を返す（棚は変わらない）。
    pub fn add_image(
        &mut self,
        lang: Lang,
        name: &str,
        rgba: &[u8],
        width: u32,
        height: u32,
    ) -> Result<String, String> {
        if let Some(reason) = &self.unavailable {
            return Err(reason.reason(lang).to_owned());
        }
        let before = self.shelf.resources().len();
        let id = self
            .shelf
            .add_image_without_origin(&new_resource_id(), name, rgba, width, height, "srgb")
            .map_err(|e| io_reason(lang, &e))?;
        if self.shelf.resources().len() > before {
            self.changed = true;
        }
        Ok(id)
    }

    /// 棚の画像の色空間（"srgb"・"linear"・"unspecified"）を替える。色空間は索引にだけあり、文書の履歴に入らない（Unity 版と同じ）。
    /// 棚を読めなかった・無い画像・同じ値は false か理由。替えたら棚は「変えた」になる。
    pub fn set_image_color_space(
        &mut self,
        lang: Lang,
        id: &str,
        space: &str,
    ) -> Result<bool, String> {
        if let Some(reason) = &self.unavailable {
            return Err(reason.reason(lang).to_owned());
        }
        let changed = self
            .shelf
            .set_image_color_space(id, space)
            .map_err(|e| io_reason(lang, &e))?;
        if changed {
            self.changed = true;
        }
        Ok(changed)
    }

    pub fn resources(&self) -> &[Resource] {
        self.shelf.resources()
    }

    pub fn get(&self, id: &str) -> Option<&Resource> {
        self.shelf
            .resources()
            .iter()
            .chain(self.builtin_items())
            .find(|r| r.id == id)
    }

    /// 一覧に出す同梱の素材の項目。
    fn builtin_items(&self) -> &[Resource] {
        if self.show_builtin {
            &self.builtin
        } else {
            &[]
        }
    }

    pub fn selected_resource(&self) -> Option<&Resource> {
        self.selected.as_deref().and_then(|id| self.get(id))
    }

    /// 絞り込み・名前の検索（大文字小文字を区別しない）に合う素材（棚の並びのまま）。
    pub fn visible(&self) -> Vec<&Resource> {
        let needle = self.search.trim().to_lowercase();
        self.shelf
            .resources()
            .iter()
            .chain(self.builtin_items())
            .filter(|r| self.filter.is_none_or(|f| ItemKind::of(&r.kind) == Some(f)))
            .filter(|r| needle.is_empty() || r.name.to_lowercase().contains(&needle))
            .collect()
    }

    pub fn info(&self, id: &str) -> Option<&Inspected> {
        self.inspected.get(id)
    }

    /// 項目の情報とサムネイルを（まだなら）作る。
    pub fn inspect(&mut self, id: &str) {
        if self.inspected.contains_key(id) {
            return;
        }
        let Some(r) = self.get(id) else { return };
        let Some(kind) = ItemKind::of(&r.kind) else {
            return;
        };
        let info = match kind {
            _ if is_builtin(id) => match self.builtin_lang.and_then(|l| self.build_builtin(id, l)) {
                Some(material) => inspect_builtin(&material),
                None => Inspected::bare(Some(Block::Unreadable(Unreadable::same("?")))),
            },
            ItemKind::Image => {
                inspect_image(r, self.shelf.content_bytes(id), self.image_thumb_pixels)
            }
            ItemKind::SmartMaterial | ItemKind::SmartMask => {
                inspect_smart(self.shelf.content_bytes(id), self.preview_budget)
            }
            ItemKind::Brush | ItemKind::Material => Inspected::bare(Some(Block::Kind(kind))),
        };
        self.inspected.insert(id.to_owned(), info);
    }

    /// 見える項目のうちまだ見ていないものを `limit` 個まで見る。まだ残っていれば true（次のフレームでも続ける）。
    pub fn inspect_pending(&mut self, limit: usize) -> bool {
        let todo: Vec<String> = self
            .visible()
            .iter()
            .filter(|r| !self.inspected.contains_key(&r.id))
            .map(|r| r.id.clone())
            .collect();
        for id in todo.iter().take(limit) {
            self.inspect(id);
        }
        todo.len() > limit
    }

    /// サムネイルのテクスチャ（作ってあれば。初めて使うときに GPU へ上げる）。
    pub fn texture(&mut self, ctx: &egui::Context, id: &str) -> Option<TextureHandle> {
        let info = self.inspected.get_mut(id)?;
        if let Thumb::Pending(image) = &mut info.thumb {
            let image = std::mem::take(image);
            let handle = ctx.load_texture(format!("shelf:{id}"), image, TextureOptions::LINEAR);
            info.thumb = Thumb::Ready(handle);
        }
        match &info.thumb {
            Thumb::Ready(h) => Some(h.clone()),
            _ => None,
        }
    }

    /// 一覧に出す説明（層の数・大きさ・チャンネル）。
    pub fn detail(&self, lang: Lang, r: &Resource) -> String {
        let Some(kind) = ItemKind::of(&r.kind) else {
            return String::new();
        };
        let size = |w: u32, h: u32| format!("{w} × {h}");
        match (kind, self.info(&r.id)) {
            (ItemKind::Image, _) => size(
                r.metadata["width"].as_u64().unwrap_or(0) as u32,
                r.metadata["height"].as_u64().unwrap_or(0) as u32,
            ),
            (ItemKind::SmartMaterial, Some(i)) if i.width > 0 => {
                let mut text = format!(
                    "{} {}",
                    i.layers,
                    lang.pick("層", if i.layers == 1 { "layer" } else { "layers" }),
                );
                // 同梱の素材は大きさに依らない（値と Generator だけ）ので、大きさを出さない
                if !is_builtin(&r.id) {
                    text += &format!(" · {}", size(i.width, i.height));
                }
                if !i.channels.is_empty() {
                    let names: Vec<_> =
                        i.channels.iter().map(|c| channel_label(lang, *c)).collect();
                    text += &format!(" · {}", names.join(", "));
                }
                text
            }
            (ItemKind::SmartMask, Some(i)) if i.width > 0 => size(i.width, i.height),
            _ => String::new(),
        }
    }

    /// 棚の素材の置けない理由（まだ見ていなければ None）。
    pub fn block_of(&self, id: &str) -> Option<&Block> {
        self.info(id)?.block.as_ref()
    }

    /// 一覧に置けないしるし（警告の印）を出す素材か。素材の中身で置けないものだけで、種類で置けないもの（ブラシ・マテリアル）は
    /// 印を付けず、名前の帯に理由を出す。
    pub fn warns(&self, id: &str) -> bool {
        self.block_of(id)
            .is_some_and(|b| !matches!(b, Block::Kind(_)))
    }

    /// 項目が文書の層から使われているか（読むだけのセットの塗りつぶし画像など）。使われている画像は消さない。
    fn used_by(&self, project: Option<&Project>, id: &str) -> bool {
        let Some(project) = project else { return false };
        project.sets().iter().any(|s| {
            s.document
                .fields()
                .iter()
                .any(|f| matches!(&f.value, NativeValue::Guid(g) if guid_text(g) == id))
        })
    }

    /// 変えていれば、棚を差し替えたプロジェクトを返す（変えていなければそのまま）。書き手は保存したアプリ。
    pub fn write_into(&self, project: Project, lang: Lang) -> Result<Project, String> {
        if !self.changed {
            return Ok(project);
        }
        project
            .with_shelf(&self.shelf, crate::project::writer())
            .map_err(|e| {
                format!(
                    "{}: {}",
                    lang.pick("棚を書けません", "Cannot write the shelf"),
                    lang.io_error(&e)
                )
            })
    }
}

/// .NET の GUID の並びから、.ylp の ID の文字列へ（yolu-io の `guid` と同じ）。
fn guid_text(b: &[u8; 16]) -> String {
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[3],
        b[2],
        b[1],
        b[0],
        b[5],
        b[4],
        b[7],
        b[6],
        b[8],
        b[9],
        b[10],
        b[11],
        b[12],
        b[13],
        b[14],
        b[15]
    )
}

fn channel_from_name(name: &str) -> Option<Channel> {
    Some(match name {
        "Color" => Channel::Color,
        "Roughness" => Channel::Roughness,
        "Metallic" => Channel::Metallic,
        "Height" => Channel::Height,
        "Normal" => Channel::Normal,
        "Emission" => Channel::Emission,
        _ => return None,
    })
}

fn channel_label(lang: Lang, c: Channel) -> &'static str {
    match c {
        Channel::Color => lang.pick("カラー", "Color"),
        Channel::Roughness => lang.pick("ラフネス", "Roughness"),
        Channel::Metallic => lang.pick("メタリック", "Metallic"),
        Channel::Height => lang.pick("ハイト", "Height"),
        Channel::Normal => lang.pick("ノーマル", "Normal"),
        Channel::Emission => lang.pick("エミッション", "Emission"),
        _ => "",
    }
}

// ───────── 項目を見る（サムネイル・置けない理由） ─────────

fn inspect_image(r: &Resource, bytes: Option<&[u8]>, thumb_pixels: u64) -> Inspected {
    let width = r.metadata["width"].as_u64().unwrap_or(0) as u32;
    let height = r.metadata["height"].as_u64().unwrap_or(0) as u32;
    let mut info = Inspected::bare(None);
    info.width = width;
    info.height = height;
    if width as u64 * height as u64 > thumb_pixels {
        return info;
    }
    let Some(bytes) = bytes else { return info };
    if let Ok(img) = image::load_from_memory_with_format(bytes, image::ImageFormat::Png) {
        let (w, h) = fit_size(img.width(), img.height());
        // 小さい画像は拡大して見せる（ぼかさず、画素のまま）
        let filter = if w >= img.width() {
            image::imageops::FilterType::Nearest
        } else {
            image::imageops::FilterType::Triangle
        };
        let small = image::imageops::resize(&img.to_rgba8(), w, h, filter);
        info.thumb = Thumb::Pending(ColorImage::from_rgba_unmultiplied(
            [w as usize, h as usize],
            small.as_raw(),
        ));
    }
    info
}

/// 長い辺が `THUMB` に収まる大きさ（細長くても 1 以上）。
fn fit_size(w: u32, h: u32) -> (u32, u32) {
    let k = THUMB as f32 / w.max(h).max(1) as f32;
    (
        ((w as f32 * k).round() as u32).clamp(1, THUMB),
        ((h as f32 * k).round() as u32).clamp(1, THUMB),
    )
}

fn inspect_smart(bytes: Option<&[u8]>, preview_budget: u64) -> Inspected {
    let Some(bytes) = bytes else {
        return Inspected::bare(Some(Block::Unreadable(Unreadable::same("?"))));
    };
    let file = match SmartFile::read(bytes) {
        Ok(f) => f,
        Err(e) => return Inspected::bare(Some(block_from(&e))),
    };
    let info = file.info();
    let mut out = Inspected::bare(None);
    out.width = info["width"].as_u64().unwrap_or(0) as u32;
    out.height = info["height"].as_u64().unwrap_or(0) as u32;
    out.layers = info["layers"].as_u64().unwrap_or(0) as usize;
    if let Some(list) = info["channels"].as_array() {
        out.channels = list
            .iter()
            .filter_map(|v| v.as_str().and_then(channel_from_name))
            .collect();
    }
    // サムネイルのために展開する量には上限がある（超える素材はアイコンで見せる。置くときには上限なしで展開する）
    match file.to_core_within(preview_budget) {
        Ok(material) => {
            out.thumb = smart_thumbnail(&material)
                .map(Thumb::Pending)
                .unwrap_or(Thumb::None)
        }
        // 展開の予算を超えただけの素材は、アイコンで見せて置ける（置くときの展開に上限は無い）。io が予算の種類で返す
        Err(yolu_io::Error::Budget(_) | yolu_io::Error::Core(CoreError::SourceBudgetExceeded)) => {}
        Err(e) => out.block = Some(smart_block(&file, &e)),
    }
    out
}

/// 捕まえた素材を .ylsmart のバイト列にして、一覧の情報（サムネイル）を作る（別のスレッドでも呼ぶ）。`cancel` が立っていれば
/// （やめた・捨てた）、結果は使われないのでサムネイルを作らず切り上げる。変換と圧縮の途中では止められない。
fn encode_material(
    material: &SmartMaterial,
    cancel: Option<&AtomicBool>,
) -> Result<Encoded, yolu_io::Error> {
    let cancelled = || cancel.is_some_and(|c| c.load(Ordering::SeqCst));
    if cancelled() {
        return Err(yolu_io::Error::Core(CoreError::Cancelled));
    }
    let file = SmartFile::from_core(material, &crate::project::writer())?;
    Ok(Encoded {
        bytes: file.file_bytes().to_vec(),
        info: (!cancelled()).then(|| inspect_material(material)),
    })
}

/// 同梱の素材の情報。サムネイルは文書全体を 1 回合成して標本化する（画素を持たない小さな文書なので軽い。
/// `smart_thumbnail` は 1 点ずつ合成するので、Generator の入った文書では遅い）。
fn inspect_builtin(material: &SmartMaterial) -> Inspected {
    let mut out = Inspected::bare(None);
    out.width = material.width();
    out.height = material.height();
    out.layers = material.layers().len();
    out.channels = material.channels();
    out.thumb = builtin_thumbnail(material)
        .map(Thumb::Pending)
        .unwrap_or(Thumb::None);
    out
}

fn builtin_thumbnail(material: &SmartMaterial) -> Option<ColorImage> {
    let (w, h) = (material.width(), material.height());
    let doc = material.fragment_document().ok()?;
    let all = doc
        .composite_channel(Channel::Color, yolu_core::Rect::new(0, 0, w, h))
        .ok()?;
    let (tw, th) = fit_size(w, h);
    let mut pixels = Vec::with_capacity((tw * th) as usize);
    for ty in 0..th {
        for tx in 0..tw {
            let x = (((tx as f32 + 0.5) * w as f32 / tw as f32) as u32).min(w - 1);
            // 画像は上の行から、文書は下の行から
            let y = h - 1 - (((ty as f32 + 0.5) * h as f32 / th as f32) as u32).min(h - 1);
            let p = &all[(y as usize * w as usize + x as usize) * 4..][..4];
            pixels.push(egui::Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3]));
        }
    }
    let mut image = ColorImage::new([tw as usize, th as usize], pixels);
    image.source_size = egui::vec2(tw as f32, th as f32);
    Some(image)
}

/// 保存したばかりの素材の情報（読み直さない）。
fn inspect_material(material: &SmartMaterial) -> Inspected {
    let mut out = Inspected::bare(None);
    out.width = material.width();
    out.height = material.height();
    out.layers = material.layers().len();
    out.channels = material.channels();
    out.thumb = smart_thumbnail(material)
        .map(Thumb::Pending)
        .unwrap_or(Thumb::None);
    out
}

/// yolu-io が編集用に開けない素材を断る文言から、置けない理由へ。
fn block_from(e: &yolu_io::Error) -> Block {
    let message = e.to_string();
    if message.contains(REFUSAL_IMAGES) {
        Block::Images
    } else if message.contains(REFUSAL_GENERATORS) {
        Block::Generators
    } else {
        Block::Unreadable(Unreadable::from_error(e))
    }
}

/// 読めた .ylsmart を core の素材にできなかった理由から、置けない理由へ。画像・Generator の再固定は文言で見分け、
/// それ以外は、core が持てない中身があればその一覧（読めている素材を「読めません」とは言わない）、なければ読めなかった理由。
fn smart_block(file: &SmartFile, e: &yolu_io::Error) -> Block {
    match block_from(e) {
        Block::Unreadable(_) => {
            let issues = file.fragment().core_issues();
            if issues.is_empty() {
                Block::Unreadable(Unreadable::from_error(e))
            } else {
                Block::Unsupported(Unreadable {
                    ja: Lang::Ja.unsupported_features(&issues),
                    en: Lang::En.unsupported_features(&issues),
                })
            }
        }
        block => block,
    }
}

/// 素材の絵（長い辺が `THUMB`。文書の合成の参照の式で標本化する。マスクは見せる量の灰色）。
fn smart_thumbnail(material: &SmartMaterial) -> Option<ColorImage> {
    let (w, h) = (material.width(), material.height());
    let (tw, th) = fit_size(w, h);
    let mut pixels = Vec::with_capacity((tw * th) as usize);
    let sample = |tx: u32, ty: u32| {
        let x = (((tx as f32 + 0.5) * w as f32 / tw as f32) as u32).min(w - 1);
        // 画像は上の行から、文書は下の行から
        let y = h - 1 - (((ty as f32 + 0.5) * h as f32 / th as f32) as u32).min(h - 1);
        (x, y)
    };
    if material.kind() == SmartKind::Mask {
        let mask = material.layers().first()?.mask()?;
        for ty in 0..th {
            for tx in 0..tw {
                let (x, y) = sample(tx, ty);
                let f = mask.factor_at(x, y).unwrap_or(1.0).clamp(0.0, 1.0);
                let g = (f * 255.0).round() as u8;
                pixels.push(egui::Color32::from_gray(g));
            }
        }
    } else {
        let doc = material.fragment_document().ok()?;
        let channels = material.channels();
        let channel = if channels.is_empty() || channels.contains(&Channel::Color) {
            Channel::Color
        } else {
            channels[0]
        };
        for ty in 0..th {
            for tx in 0..tw {
                let (x, y) = sample(tx, ty);
                let p = doc
                    .composite_pixel(channel, x, y)
                    .map(|p| p.to_array())
                    .unwrap_or([0; 4]);
                pixels.push(egui::Color32::from_rgba_unmultiplied(
                    p[0], p[1], p[2], p[3],
                ));
            }
        }
    }
    let mut image = ColorImage::new([tw as usize, th as usize], pixels);
    image.source_size = egui::vec2(tw as f32, th as f32);
    Some(image)
}

// ───────── 操作 ─────────

/// 置く先。
#[derive(Clone, Debug, PartialEq)]
pub enum PlaceTarget {
    /// 選んでいる層の上（スマートマスクは選んでいる層のマスク）。
    Selected,
    /// 層の一覧の落とした所（親のグループと、その子の中の位置。0 が一番下。None の位置は一番上）。
    At {
        parent: Option<LayerId>,
        position: Option<usize>,
    },
    /// このレイヤーのマスクへ（スマートマスク）。
    Mask(LayerId),
}

/// 棚の操作（棚の中身を変えるもの・文書へ置くもの）。文書を変えるのは `Place` だけで、1 回の Undo。
#[derive(Clone, Debug, PartialEq)]
pub enum ShelfOp {
    /// 層（グループなら中身ごと）をスマートマテリアルとして棚へ。
    SaveMaterial(LayerId),
    /// 層のマスクをスマートマスクとして棚へ。
    SaveMask(LayerId),
    Place {
        id: String,
        target: PlaceTarget,
    },
    /// 棚から消す（消す前に確かめる窓は `AskRemove`）。
    Remove(String),
    AskRemove(String),
    /// 別のスレッドで書き出している保存をやめる（できた素材は棚へ入れない）。
    CancelSave,
    /// 外の .ylsmart を棚へ。
    ImportFile(PathBuf),
    /// 外の .ylsmart をまとめて棚へ（入れた数と断ったファイルの理由を 1 つの知らせにする）。
    ImportFiles(Vec<PathBuf>),
    ImportDialog,
    /// 棚の素材を .ylsmart のファイルへ。
    ExportFile {
        id: String,
        path: PathBuf,
    },
    ExportDialog(String),
}

/// 棚へ入れた結果（新しく入ったか、同じ中身が既にあったか）。
enum Kept {
    Added,
    Existing,
}

/// 層の一覧へ落としている棚の素材（egui のドラッグの荷物）。
#[derive(Clone, Debug, PartialEq)]
pub struct ShelfDrag {
    pub id: String,
    pub kind: ItemKind,
    pub name: String,
}

/// 棚の画像を、1 枚のペイントの層の素材にする（置くときに文書の大きさへ引き伸ばす。置くのは 1 回の Undo の層の挿入と同じ道）。
/// 画素は straight RGBA8 のまま（透明の画素の RGB も保つ）。読めない・大きさが索引と違うときの理由は棚の画像の言葉で言う。
fn image_as_material(
    lang: Lang,
    png: &[u8],
    width: u32,
    height: u32,
    name: &str,
) -> Result<SmartMaterial, String> {
    let unreadable = || {
        lang.pick("画像を読めません", "Cannot read the image")
            .to_owned()
    };
    let reader =
        image::ImageReader::with_format(std::io::Cursor::new(png), image::ImageFormat::Png);
    let (found_w, found_h) = reader.into_dimensions().map_err(|_| unreadable())?;
    if (found_w, found_h) != (width, height) {
        return Err(lang
            .pick(
                "画像の大きさが索引と違います",
                "Image size differs from the index",
            )
            .to_owned());
    }
    let (mut doc, note) = crate::project::preview_document(Some(png), width, height, lang);
    if note.is_some() {
        return Err(unreadable());
    }
    let id = doc
        .layers()
        .first()
        .map(|l| l.id())
        .ok_or_else(|| lang.pick("レイヤーがありません", "No layer").to_owned())?;
    doc.set_layer_name(id, name)
        .map_err(|e| core_reason(lang, &e))?;
    doc.capture_smart_material(&[id], name)
        .map_err(|e| core_reason(lang, &e))
}

/// 新しい素材の ID（小文字のハイフン付き GUID。文書の ID の作り方と同じ乱数）。
fn new_resource_id() -> String {
    let doc = Document::new(1, 1).expect("1×1 の文書");
    crate::sets::guid_string(doc.id())
}

/// core の断りの短い理由。
pub fn core_reason(lang: Lang, e: &CoreError) -> String {
    match e {
        CoreError::Unsupported("ユーザーチャンネルの対応が一致しません") => lang
            .pick("チャンネルが合いません", "Channels do not match")
            .into(),
        CoreError::SourceBudgetExceeded => lang
            .pick("画素の予算を超えます", "Over the pixel budget")
            .into(),
        CoreError::InvalidArgument("層は2048個までです") => {
            lang.pick("層が多すぎます", "Too many layers").into()
        }
        CoreError::InvalidArgument("保存するマスクがありません") => {
            lang.pick("マスクがありません", "No mask").into()
        }
        CoreError::InvalidArgument("保存する層がありません") | CoreError::LayerNotFound => {
            lang.pick("レイヤーがありません", "No layer").into()
        }
        CoreError::InvalidArgument("スマート素材の名前") => {
            lang.pick("名前が使えません", "Name not allowed").into()
        }
        CoreError::InvalidArgument("配置先がグループではありません") => lang
            .pick("置き先がグループではありません", "Target is not a group")
            .into(),
        other => lang.core_error(other),
    }
}

/// 棚の個数が上限（`MAX_RESOURCES`）に達しているときの短い理由。
fn shelf_full_reason(lang: Lang) -> &'static str {
    lang.pick("棚がいっぱいです", "Shelf is full")
}

/// yolu-io の断りの短い理由（読み込み・棚の予算・保存）。
pub fn io_reason(lang: Lang, e: &yolu_io::Error) -> String {
    let m = e.to_string();
    if m.contains(REFUSAL_RESOURCE_COUNT) {
        shelf_full_reason(lang).into()
    } else if m.contains(REFUSAL_IMAGES) {
        Block::Images.reason(lang)
    } else if m.contains(REFUSAL_GENERATORS) {
        Block::Generators.reason(lang)
    } else if m.contains(REFUSAL_MEMORY_BUDGET) {
        lang.pick("棚の予算を超えます", "Over the shelf budget")
            .into()
    } else if m.contains(REFUSAL_ARCHIVE_BUDGET) {
        lang.pick(
            "ファイルの大きさの上限を超えます",
            "Over the file size limit",
        )
        .into()
    } else if m.contains(REFUSAL_USER_CHANNELS) {
        lang.pick(
            "ユーザーチャンネルは保存できません",
            "User channels cannot be saved",
        )
        .into()
    } else if m.contains(REFUSAL_RUST_GENERATORS) {
        lang.pick(
            "ノイズ・グランジは保存できません",
            "Noise and Grunge cannot be saved",
        )
        .into()
    } else if m.contains(REFUSAL_RUST_ADJUSTMENTS) {
        lang.pick(
            "色調補正は保存できません",
            "Colour adjustments cannot be saved",
        )
        .into()
    } else {
        lang.io_error(e)
    }
}

impl AppState {
    /// 棚の操作を当てる。断られたら何も変えず、理由をステータスバーへ。
    pub fn shelf_apply(&mut self, op: ShelfOp) {
        // 書き出しをやめるのは、描いている間でもできる（棚にも文書にも触らない）
        if op == ShelfOp::CancelSave {
            if let Some(p) = self.shelf.saving.take() {
                self.message = format!(
                    "{}: {}",
                    self.lang.pick("保存をやめました", "Cancelled saving"),
                    p.name
                );
            }
            return;
        }
        if self.is_stroking() {
            self.message = self
                .lang
                .pick("描いている間はできません。", "Not while drawing.")
                .into();
            return;
        }
        self.shelf.use_language(self.lang);
        // 同梱の素材は棚に入っていない: 消す・書き出すは断る（置くだけ）
        if let ShelfOp::Remove(id)
        | ShelfOp::AskRemove(id)
        | ShelfOp::ExportDialog(id)
        | ShelfOp::ExportFile { id, .. } = &op
        {
            if is_builtin(id) {
                let reason = match op {
                    ShelfOp::Remove(_) | ShelfOp::AskRemove(_) => self
                        .lang
                        .pick("組み込みは消せません", "Built-in items cannot be removed"),
                    _ => self
                        .lang
                        .pick("組み込みは書き出せません", "Built-in items cannot be exported"),
                };
                return self.shelf_refusal(reason.into());
            }
        }
        match op {
            ShelfOp::SaveMaterial(id) => self.shelf_save(id, false),
            ShelfOp::SaveMask(id) => self.shelf_save(id, true),
            ShelfOp::Place { id, target } => self.shelf_place(&id, target),
            ShelfOp::CancelSave => {}
            ShelfOp::Remove(id) => {
                if !self.shelf_refuse_while_saving() {
                    self.shelf_remove(&id)
                }
            }
            ShelfOp::AskRemove(id) => {
                if self.shelf.get(&id).is_some() && !self.shelf_refuse_while_saving() {
                    // 使われている画像は、確かめの窓を出す前に断る（確かめても消せない）
                    if let Some(why) = self.shelf_image_in_use(&id) {
                        self.shelf_refusal(why);
                    } else {
                        self.shelf.pending_remove = Some(id);
                        self.dialog_request = Some(DialogRequest::ShelfRemove);
                    }
                }
            }
            ShelfOp::ImportFile(path) => {
                if !self.shelf_refuse_while_saving() {
                    self.shelf_import(&[path])
                }
            }
            ShelfOp::ImportFiles(paths) => {
                if !self.shelf_refuse_while_saving() {
                    self.shelf_import(&paths)
                }
            }
            ShelfOp::ImportDialog => {
                if !self.shelf_refuse_while_saving() {
                    self.dialog_request = Some(DialogRequest::ShelfImport)
                }
            }
            ShelfOp::ExportFile { id, path } => self.shelf_export(&id, &path),
            ShelfOp::ExportDialog(id) => {
                if self.shelf.get(&id).is_some() {
                    self.shelf.export_id = Some(id);
                    self.dialog_request = Some(DialogRequest::ShelfExport);
                }
            }
        }
    }

    fn shelf_refusal(&mut self, reason: String) {
        self.message = format!("{}: {reason}", self.lang.pick("できません", "Cannot"));
    }

    /// 別のスレッドの保存が棚の写しへ足している間は、棚を変える操作（消す・読み込む）を断る（保存の結果は足した後の棚に
    /// 差し替えるので、その間に棚が変わると、変えた分が消える）。断ったら true。
    pub(crate) fn shelf_refuse_while_saving(&mut self) -> bool {
        if self.shelf.saving.is_none() {
            return false;
        }
        let reason = self.lang.pick("保存中です", "Already saving");
        self.shelf_refusal(reason.into());
        true
    }

    /// 棚を変えてよいか（読めなかった棚には足さない・消さない）。
    fn shelf_writable(&mut self) -> bool {
        match self.shelf.unavailable.clone() {
            Some(e) => {
                self.shelf_refusal(format!(
                    "{}: {}",
                    self.lang.pick("棚を読めません", "Shelf unreadable"),
                    e.reason(self.lang)
                ));
                false
            }
            None => true,
        }
    }

    fn shelf_save(&mut self, id: LayerId, mask: bool) {
        let lang = self.lang;
        if let Some(reason) = self.read_only_reason() {
            let text = format!(
                "{}: {reason}",
                lang.pick("読むだけのテクスチャセットです", "Read-only texture set")
            );
            self.message = text;
            return;
        }
        if !self.shelf_writable() {
            return;
        }
        // 書き出しが走っている間は、捕まえる写し（大きな素材では数百 MiB）を作る前に断る
        if let Some(reason) = self.shelf.busy_reason(lang) {
            return self.shelf_refusal(reason.into());
        }
        // 捕まえるたびに別の素材になる（同じ中身が既にあって足りることは無い）ので、いっぱいの棚には写しを作る前に断る
        if self.shelf.shelf.resources().len() >= MAX_RESOURCES {
            return self.shelf_refusal(shelf_full_reason(lang).into());
        }
        let Some(layer_name) = self.doc.layer(id).map(|l| l.name().to_owned()) else {
            return self.shelf_refusal(lang.pick("レイヤーがありません", "No layer").into());
        };
        let name = if mask {
            format!("{layer_name}{}", lang.pick(" のマスク", " mask"))
        } else {
            layer_name
        };
        let captured = if mask {
            self.doc.capture_smart_mask(id, &name)
        } else {
            self.doc.capture_smart_material(&[id], &name)
        };
        let material = match captured {
            Ok(m) => m,
            Err(e) => return self.shelf_refusal(core_reason(lang, &e)),
        };
        let kind = if mask {
            ItemKind::SmartMask
        } else {
            ItemKind::SmartMaterial
        };
        // 大きな素材、または大きな棚への保存は、書き出し（正本への変換・圧縮・サムネイル）と棚への追加（棚全体の検証）を別のスレッドで行い、
        // 画面のスレッドには差し替えだけを残す。捕まえた写しは文書から独立、棚の写しは中身を共有する（保存の間、棚を変える操作は断る）
        if material
            .pixel_bytes()
            .saturating_add(self.shelf.shelf.used_bytes())
            >= self.shelf.async_bytes
        {
            let (tx, rx) = std::sync::mpsc::channel();
            let repaint = self.shelf.context.clone();
            let hold = self.shelf.hold.clone();
            let cancel = Arc::new(AtomicBool::new(false));
            let running = Running::start(&self.shelf.running);
            let thread_cancel = cancel.clone();
            let snapshot = self.shelf.shelf.clone();
            let thread_name = name.clone();
            std::thread::spawn(move || {
                let done = encode_material(&material, Some(&thread_cancel)).and_then(|encoded| {
                    if thread_cancel.load(Ordering::SeqCst) {
                        return Err(yolu_io::Error::Core(CoreError::Cancelled));
                    }
                    stage(snapshot, &thread_name, kind, encoded)
                });
                while hold.load(Ordering::SeqCst) {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                // 結果を渡す前に数えから外す（結果を受けた直後の保存が、終わりかけのスレッドのせいで断られない）
                drop(running);
                let _ = tx.send(done);
                if let Some(ctx) = repaint {
                    ctx.request_repaint();
                }
            });
            self.message = format!("{}: {name}", lang.pick("保存中", "Saving"));
            self.shelf.saving = Some(PendingSave {
                name,
                kind,
                rx,
                cancel,
            });
            return;
        }
        match encode_material(&material, None) {
            Ok(done) => self.shelf_keep(&name, kind, done),
            Err(e) => self.shelf_refusal(io_reason(lang, &e)),
        }
    }

    /// 書き出した素材を棚へ入れる（同じ中身が既にあればそれ）。入れたら選び、サムネイルを覚える。知らせは 1 件分を出す。
    fn shelf_keep(&mut self, name: &str, kind: ItemKind, done: Encoded) {
        match self.shelf_keep_quiet(name, kind, done) {
            Ok(kept) => self.shelf_keep_message(name, kept),
            Err(reason) => self.shelf_refusal(reason),
        }
    }

    fn shelf_keep_message(&mut self, name: &str, kept: Kept) {
        let lang = self.lang;
        self.message = match kept {
            Kept::Added => format!(
                "{}: {name}",
                lang.pick("棚に入れました", "Added to the shelf")
            ),
            Kept::Existing => format!(
                "{}: {name}",
                lang.pick("すでに棚にあります", "Already on the shelf")
            ),
        };
    }

    /// `shelf_keep` の知らせを出さない形（まとめて読み込むときに、知らせを 1 つにまとめるため）。断りは「名前: 理由」。
    /// 棚の検証はこのスレッドで行う（読み込みは同期。大きな棚では長い）。
    fn shelf_keep_quiet(
        &mut self,
        name: &str,
        kind: ItemKind,
        done: Encoded,
    ) -> Result<Kept, String> {
        let lang = self.lang;
        let staged = stage(self.shelf.shelf.clone(), name, kind, done)
            .map_err(|e| format!("{name}: {}", io_reason(lang, &e)))?;
        Ok(self.shelf_adopt(name, kind, staged))
    }

    /// 足した後の棚に差し替える（足していなければそのまま）。入れた素材は絞り込み・検索で隠さず、選ぶ。
    fn shelf_adopt(&mut self, name: &str, kind: ItemKind, staged: Staged) -> Kept {
        if staged.added {
            self.shelf.shelf = staged.next;
            if let Some(info) = staged.info {
                self.shelf.inspected.insert(staged.id.clone(), info);
            }
            self.shelf.changed = true;
            self.modified = true;
        }
        self.shelf_reveal(&staged.id, kind, name);
        if staged.added {
            Kept::Added
        } else {
            Kept::Existing
        }
    }

    /// 別のスレッドの書き出しが終わっていれば棚へ入れる（毎フレーム）。
    pub fn shelf_poll(&mut self) {
        let lang = self.lang;
        let Some(pending) = &self.shelf.saving else {
            return;
        };
        // スレッドが結果を渡さずに終わった（パニック）ときも、断りとして出す
        let result = match pending.rx.try_recv() {
            Ok(r) => Some(r.map_err(|e| io_reason(lang, &e))),
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(Err(lang
                .pick("書き出しが途中で止まりました", "The write stopped midway")
                .to_owned())),
        };
        if let Some(result) = result {
            let pending = self.shelf.saving.take().expect("上で確かめた");
            match result {
                Ok(staged) => {
                    let kept = self.shelf_adopt(&pending.name, pending.kind, staged);
                    self.shelf_keep_message(&pending.name, kept);
                }
                Err(reason) => self.shelf_refusal(format!("{}: {reason}", pending.name)),
            }
        }
    }

    /// 別のスレッドの書き出しが終わるまで待って棚へ入れる（試験用）。
    pub fn shelf_wait(&mut self) {
        while self.shelf.saving.is_some() {
            self.shelf_poll();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    /// 入れた素材が絞り込み・検索で隠れないようにして選ぶ。
    fn shelf_reveal(&mut self, id: &str, kind: ItemKind, name: &str) {
        if self.shelf.filter.is_some_and(|f| f != kind) {
            self.shelf.filter = Some(kind);
        }
        let needle = self.shelf.search.trim().to_lowercase();
        if !needle.is_empty() && !name.to_lowercase().contains(&needle) {
            self.shelf.search.clear();
        }
        self.shelf.selected = Some(id.to_owned());
    }

    fn shelf_place(&mut self, id: &str, target: PlaceTarget) {
        let lang = self.lang;
        let Some(res) = self.shelf.get(id) else {
            return;
        };
        let name = res.name.clone();
        let kind = ItemKind::of(&res.kind);
        let (width, height) = (
            res.metadata["width"].as_u64().unwrap_or(0) as u32,
            res.metadata["height"].as_u64().unwrap_or(0) as u32,
        );
        if !kind.is_some_and(|k| k.is_smart() || k == ItemKind::Image) {
            let block = Block::Kind(kind.unwrap_or(ItemKind::Brush));
            return self.shelf_refusal(block.reason(lang));
        }
        let builtin = is_builtin(id);
        let bytes = if builtin {
            &[][..]
        } else {
            let Some(bytes) = self.shelf.shelf.content_bytes(id) else {
                return;
            };
            bytes
        };
        let made = if builtin {
            // 同梱の素材はコードから組む（ファイルも棚の中身も無い）
            self.shelf.build_builtin(id, lang).ok_or_else(|| {
                lang.pick("組み込みの素材を組めません", "Cannot build the built-in item")
                    .to_owned()
            })
        } else if kind == Some(ItemKind::Image) {
            image_as_material(lang, bytes, width, height, &name)
        } else {
            match SmartFile::read(bytes) {
                Err(e) => Err(io_reason(lang, &e)),
                // 読めた素材を置けないときは、項目に出す理由と同じ言い方で断る
                Ok(file) => file.to_core().map_err(|e| match smart_block(&file, &e) {
                    Block::Unreadable(_) => io_reason(lang, &e),
                    block => block.reason(lang),
                }),
            }
        };
        let material = match made {
            Ok(m) => m,
            Err(reason) => return self.shelf_refusal(reason),
        };
        if material.kind() == SmartKind::Mask {
            let layer = match target {
                PlaceTarget::Mask(layer) => Some(layer),
                _ => self.selected_layer,
            };
            let Some(layer) = layer.filter(|l| self.doc.layer(*l).is_some()) else {
                return self.shelf_refusal(
                    lang.pick("選んだレイヤーがありません", "No layer selected")
                        .into(),
                );
            };
            let had_mask = self.doc.layer(layer).is_some_and(|l| l.mask().is_some());
            match self.doc.apply_smart_mask(&material, layer, None) {
                Ok(result) => {
                    self.selected_layer = Some(layer);
                    self.set_edit_mask(true);
                    self.modified = true;
                    let mut text = format!(
                        "{}: {name}",
                        if had_mask {
                            lang.pick("マスクを入れ替えました", "Replaced the mask")
                        } else {
                            lang.pick("マスクにしました", "Applied as the mask")
                        }
                    );
                    text += &resized_note(lang, &material, result.resampled);
                    self.message = text;
                }
                Err(e) => self.shelf_refusal(core_reason(lang, &e)),
            }
            self.ensure_selection();
            return;
        }
        let (parent, position) = match target {
            PlaceTarget::At { parent, position } => (parent, position),
            _ => self.above_selected_slot(),
        };
        let placement = SmartPlacement {
            parent,
            position,
            ..SmartPlacement::default()
        };
        match self.doc.place_smart_material(&material, &placement) {
            Ok(result) => {
                self.selected_layer = Some(result.layer_id);
                self.set_edit_mask(false);
                if let Some(p) = parent {
                    self.m2.collapsed.remove(&p);
                }
                self.modified = true;
                let n = result.layers.len();
                let mut text = format!(
                    "{}: {name}（{n} {}",
                    lang.pick("置きました", "Placed"),
                    lang.pick("層", if n == 1 { "layer" } else { "layers" })
                );
                // 同梱の素材は大きさに依らない（画素が無い）ので、変えたとは言わない
                text += &resized_note(lang, &material, result.resampled && !builtin);
                text += "）";
                self.message = text;
            }
            Err(e) => self.shelf_refusal(core_reason(lang, &e)),
        }
        self.ensure_selection();
    }

    /// 選んでいる層のすぐ上の置き場所（親と、その子の中の位置。選んだ層が無ければ一番上）。
    fn above_selected_slot(&self) -> (Option<LayerId>, Option<usize>) {
        let Some((id, parent)) = self
            .selected_layer
            .and_then(|id| self.doc.layer(id).map(|l| (id, l.parent())))
        else {
            return (None, None);
        };
        let siblings = self.doc.children_of(parent).unwrap_or_default();
        (
            parent,
            siblings.iter().position(|c| *c == id).map(|i| i + 1),
        )
    }

    /// 棚の画像を層が読んでいれば、消せない理由の文（読んでいなければ None。画像でない素材も None）。読み込んだプロジェクトの
    /// 原本の層と、いまのセッションの全部のテクスチャセットの層を見る。消すと、層は棚に無い画像を指し、保存して開き直すとそのセットは
    /// 読むだけになる。取り消しの履歴の中にだけある層（消した層を Undo で戻す）は見ない。
    fn shelf_image_in_use(&self, id: &str) -> Option<String> {
        let res = self.shelf.get(id).filter(|r| r.kind == "image")?;
        let lang = self.lang;
        let project = self.project.as_ref().map(|p| p.project());
        let users = crate::fillfx::image_users(self, id);
        if users.is_empty() && !self.shelf.used_by(project, id) {
            return None;
        }
        let mut text = format!(
            "{}: {}",
            res.name,
            lang.pick("レイヤーから使われています", "Used by a layer")
        );
        if !users.is_empty() {
            let list = users.iter().take(3).cloned().collect::<Vec<_>>().join(", ");
            let more = if users.len() > 3 { " …" } else { "" };
            text += &match lang {
                Lang::Ja => format!("（{list}{more}）"),
                Lang::En => format!(" ({list}{more})"),
            };
        }
        Some(text)
    }

    fn shelf_remove(&mut self, id: &str) {
        let lang = self.lang;
        if !self.shelf_writable() {
            return;
        }
        let Some(res) = self.shelf.get(id) else {
            return;
        };
        let name = res.name.clone();
        if let Some(why) = self.shelf_image_in_use(id) {
            return self.shelf_refusal(why);
        }
        match self.shelf.shelf.remove(id) {
            Ok(true) => {
                self.shelf.inspected.remove(id);
                if self.shelf.selected.as_deref() == Some(id) {
                    self.shelf.selected = None;
                }
                self.shelf.changed = true;
                self.modified = true;
                self.message = format!(
                    "{}: {name}",
                    lang.pick("棚から消しました", "Removed from the shelf")
                );
            }
            Ok(false) => {}
            Err(e) => self.shelf_refusal(io_reason(lang, &e)),
        }
        if self.shelf.pending_remove.as_deref() == Some(id) {
            self.shelf.pending_remove = None;
        }
    }

    /// 外の .ylsmart を棚へ（1 つでも複数でも、知らせは 1 つ。断ったファイルは名前と理由を残し、後ろの成功で消さない）。
    fn shelf_import(&mut self, paths: &[PathBuf]) {
        let lang = self.lang;
        if !self.shelf_writable() {
            return;
        }
        let (mut added, mut existing) = (Vec::new(), Vec::new());
        let mut refused = Vec::new();
        for path in paths {
            match self.shelf_import_one(path) {
                Ok((name, Kept::Added)) => added.push(name),
                Ok((name, Kept::Existing)) => existing.push(name),
                Err(reason) => refused.push(reason),
            }
        }
        let (new, old) = (added.len(), existing.len());
        if refused.is_empty() {
            let text = match (added.as_slice(), existing.as_slice()) {
                ([name], []) => format!(
                    "{}: {name}",
                    lang.pick("棚に入れました", "Added to the shelf")
                ),
                ([], [name]) => format!(
                    "{}: {name}",
                    lang.pick("すでに棚にあります", "Already on the shelf")
                ),
                _ => kept_note(lang, new, old),
            };
            if new + old > 0 {
                self.message = text;
            }
            return;
        }
        // 断りがあっても、入れた分とすでにあった分は別に数える（すでにあったものを「入れました」と言わない）
        let mut text = format!(
            "{}: {}",
            lang.pick("できません", "Cannot"),
            refused.join(" / ")
        );
        if new + old > 0 {
            text += &format!(" · {}", kept_note(lang, new, old));
        }
        self.message = text;
    }

    /// 1 つの .ylsmart を棚へ。入れた名前と、新しく入ったか。断りは「ファイル名: 理由」。
    fn shelf_import_one(&mut self, path: &std::path::Path) -> Result<(String, Kept), String> {
        let lang = self.lang;
        let shown = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        match std::fs::metadata(path) {
            Ok(m) if m.len() > self.shelf.import_limit => {
                return Err(format!(
                    "{shown}: {}",
                    lang.pick("大きすぎます", "Too large")
                ));
            }
            Ok(_) => {}
            Err(e) => return Err(format!("{shown}: {}", lang.file_error(&e))),
        }
        let bytes = std::fs::read(path).map_err(|e| format!("{shown}: {}", lang.file_error(&e)))?;
        let file = SmartFile::read(&bytes).map_err(|e| {
            format!(
                "{shown}: {} ({})",
                lang.pick("スマート素材として読めません", "Not a readable smart asset"),
                lang.io_error(&e)
            )
        })?;
        let kind = if file.kind() == SmartKind::Mask {
            ItemKind::SmartMask
        } else {
            ItemKind::SmartMaterial
        };
        let name = file.info()["name"].as_str().unwrap_or(&shown).to_owned();
        let kept = self.shelf_keep_quiet(&name, kind, Encoded { bytes, info: None })?;
        Ok((name, kept))
    }

    fn shelf_export(&mut self, id: &str, path: &std::path::Path) {
        let lang = self.lang;
        let Some(res) = self.shelf.get(id) else {
            return;
        };
        let name = res.name.clone();
        if !ItemKind::of(&res.kind).is_some_and(ItemKind::is_smart) {
            return self.shelf_refusal(format!(
                "{name}: {}",
                lang.pick(
                    "書き出せるのはスマート素材だけです",
                    "Only smart assets can be exported"
                )
            ));
        }
        let Some(bytes) = self.shelf.shelf.content_bytes(id) else {
            return;
        };
        // 置き換えは 1 回（一時ファイルへ書いて名前を付け替える。途中で失敗しても元のファイルは変わらない）
        let tmp = path.with_extension("ylsmart.tmp~");
        let result = std::fs::write(&tmp, bytes).and_then(|()| std::fs::rename(&tmp, path));
        match result {
            Ok(()) => {
                self.message = format!(
                    "{}: {}",
                    lang.pick("書き出しました", "Exported"),
                    path.display()
                )
            }
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                self.shelf_refusal(format!("{}: {}", path.display(), lang.file_error(&e)));
            }
        }
    }
}

/// 読み込みの結果の件数（入れた分とすでにあった分。0 の分は出さない）。
fn kept_note(lang: Lang, added: usize, existing: usize) -> String {
    let count = |n: usize| match lang {
        Lang::Ja => format!("{n} 件"),
        Lang::En => n.to_string(),
    };
    let mut parts = Vec::new();
    if added > 0 {
        parts.push(format!(
            "{}: {}",
            lang.pick("棚に入れました", "Added to the shelf"),
            count(added)
        ));
    }
    if existing > 0 {
        parts.push(format!(
            "{}: {}",
            lang.pick("すでに棚にあった", "Already there"),
            count(existing)
        ));
    }
    parts.join(" · ")
}

/// 大きさが違って拡大縮小したときの知らせ（括弧の中に足す）。
fn resized_note(lang: Lang, material: &SmartMaterial, resampled: bool) -> String {
    if !resampled {
        return String::new();
    }
    match lang {
        Lang::Ja => format!("、{}×{} から変更", material.width(), material.height()),
        Lang::En => format!(", resized from {}×{}", material.width(), material.height()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumbnails_fit_the_long_side() {
        assert_eq!(fit_size(2048, 1024), (64, 32));
        assert_eq!(fit_size(3, 2), (64, 43));
        assert_eq!(fit_size(1, 4096), (1, 64));
        assert_eq!(fit_size(8, 8), (64, 64));
    }

    #[test]
    fn guid_text_matches_the_dotnet_layout() {
        let b = [
            0x44, 0x33, 0x22, 0x11, 0x66, 0x55, 0x88, 0x77, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee,
            0xff, 0x00,
        ];
        assert_eq!(guid_text(&b), "11223344-5566-7788-99aa-bbccddeeff00");
    }

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::new();
        image::RgbaImage::new(w, h)
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    #[test]
    fn a_broken_or_mismatched_shelf_image_is_refused_in_the_shelfs_own_words() {
        let mut cut = png(2, 1);
        cut.truncate(cut.len() - 20);
        for (lang, unreadable, mismatch) in [
            (Lang::Ja, "画像を読めません", "画像の大きさが索引と違います"),
            (
                Lang::En,
                "Cannot read the image",
                "Image size differs from the index",
            ),
        ] {
            for bytes in [b"not a png".to_vec(), Vec::new(), cut.clone()] {
                let error = image_as_material(lang, &bytes, 2, 1, "x").unwrap_err();
                assert_eq!(error, unreadable);
            }
            // 大きさが索引と違う（どちら向きにも）
            for (w, h) in [(3, 1), (2, 2), (1, 1)] {
                let error = image_as_material(lang, &png(w, h), 2, 1, "x").unwrap_err();
                assert_eq!(error, mismatch, "{w}×{h}");
            }
            // 保存した合成の絵の話にはしない
            assert!(!unreadable.contains("合成") && !mismatch.contains("合成"));
            assert!(image_as_material(lang, &png(2, 1), 2, 1, "x").is_ok());
        }
    }

    #[test]
    fn kinds_round_trip_the_index_names() {
        for k in ItemKind::ALL {
            let name = match k {
                ItemKind::Image => "image",
                ItemKind::Brush => "brush",
                ItemKind::Material => "material",
                ItemKind::SmartMaterial => "smartMaterial",
                ItemKind::SmartMask => "smartMask",
            };
            assert_eq!(ItemKind::of(name), Some(k));
        }
        assert_eq!(ItemKind::of("future"), None);
    }
}
