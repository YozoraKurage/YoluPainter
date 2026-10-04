//! ブラシの一覧（クリスタのサブツールに当たる）と、道具ごとに最後に使ったブラシの覚え。
//!
//! 一覧の 1 つ（`Entry`）は名前・グループ・設定の元（`baseline`）と、変えたままの設定（`edited`）を持つ。組み込みは消せない元で、
//! 利用者のブラシは設定のフォルダに 1 つ 1 ファイルで保存する（`store`）。今のブラシの設定は、これまでどおり `AppState::brush` と
//! `AppState::m2.brush`（スライダーやオプションバーがその場で変える）が持ち、一覧はそれとの差だけを見る: 別のブラシへ替える・道具を
//! 替えるときに今の設定を一覧の側へ書き戻し（`brush_sync`）、替えた先の設定を今の設定へ写す（`brush_load`）。
//! 手ぶれ補正と入り抜き（`assist`）・対称・ステンシル・背景色・乱数の種は描き手の設定なので、ブラシには入れない（替えても残る）。
//! ブラシの設定は文書ではない（Undo に入れない）。ストロークの最中は、ブラシを替える操作を断る。

pub mod builtin;
pub mod gaps;
pub mod images;
pub mod import;
pub mod krita;
pub mod sample;
pub mod store;

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use egui::{Rect, Vec2};

use crate::engine::{Brush, BrushSettings, CanvasSymmetry, ColorDynamics, ColorMix, StrokeAssist};
use crate::lang::Lang;
use crate::m2;
use crate::state::{AppState, BrushState, Tool};

pub use gaps::Gap;

/// 利用者のブラシの数の上限（ファイルとメモリを抑える。起動のときに読むファイルの数の上限と同じ。ABR 1 本のプリセットが
/// 数百になる）。
pub const MAX_USER_BRUSHES: usize = 1024;
/// ブラシの名前の長さの上限（文字数）。
pub const MAX_NAME_CHARS: usize = 40;

/// ブラシのグループ（一覧の上のタブ）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Group {
    Pen,
    Brush,
    Airbrush,
    Eraser,
    /// ぼかし・指先・クローン。
    Effect,
    Special,
    /// 取り込んだブラシ（ファイルから。組み込みは無く、1 つでもあるときだけタブを出す）。
    Imported,
}

impl Group {
    /// 組み込みのブラシがあり、いつもタブを出すグループ。
    pub const ALL: [Group; 6] = [
        Group::Pen,
        Group::Brush,
        Group::Airbrush,
        Group::Eraser,
        Group::Effect,
        Group::Special,
    ];

    /// 保存できるグループの全部（取り込んだブラシのグループを含む）。
    pub const EVERY: [Group; 7] = [
        Group::Pen,
        Group::Brush,
        Group::Airbrush,
        Group::Eraser,
        Group::Effect,
        Group::Special,
        Group::Imported,
    ];

    /// ファイルに書く名前。
    pub fn id(self) -> &'static str {
        match self {
            Group::Pen => "pen",
            Group::Brush => "brush",
            Group::Airbrush => "airbrush",
            Group::Eraser => "eraser",
            Group::Effect => "effect",
            Group::Special => "special",
            Group::Imported => "imported",
        }
    }

    pub fn from_id(id: &str) -> Option<Group> {
        Group::EVERY.into_iter().find(|g| g.id() == id)
    }

    pub fn name(self, lang: Lang) -> &'static str {
        match self {
            Group::Pen => lang.pick("ペン", "Pen"),
            Group::Brush => lang.pick("筆", "Brush"),
            Group::Airbrush => lang.pick("エアブラシ", "Airbrush"),
            Group::Eraser => lang.pick("消しゴム", "Eraser"),
            Group::Effect => lang.pick("効果", "Effects"),
            Group::Special => lang.pick("特殊", "Special"),
            Group::Imported => lang.pick("取り込み", "Imported"),
        }
    }

    /// タブに書く短い名前（全名はツールチップ）。
    pub fn short(self, lang: Lang) -> &'static str {
        match self {
            Group::Pen => lang.pick("ペン", "Pen"),
            Group::Brush => lang.pick("筆", "Brush"),
            Group::Airbrush => lang.pick("エア", "Air"),
            Group::Eraser => lang.pick("消し", "Erase"),
            Group::Effect => lang.pick("効果", "FX"),
            Group::Special => lang.pick("特殊", "Misc"),
            Group::Imported => lang.pick("取込", "Import"),
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Group::Pen => "stylus",
            Group::Brush => "tools/brush",
            Group::Airbrush => "blur_on",
            Group::Eraser => "tools/eraser",
            Group::Effect => "ink_stroke",
            Group::Special => "data_scatter",
            Group::Imported => "import",
        }
    }

    /// 消しゴムの道具（E）で使うグループ。
    pub fn is_eraser(self) -> bool {
        self == Group::Eraser
    }
}

/// 一覧の 1 つを指す印。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BrushKey {
    /// 組み込み（`builtin` の id）。
    Builtin(&'static str),
    /// 利用者のブラシ（ファイル名の番号）。
    User(u32),
}

impl BrushKey {
    /// 並びのファイルに書く札。
    pub fn token(self) -> String {
        match self {
            BrushKey::Builtin(id) => format!("b:{id}"),
            BrushKey::User(n) => format!("u:{n}"),
        }
    }

    /// 札から読み戻す（今の版が持たない組み込みは None）。
    pub fn parse_token(token: &str) -> Option<BrushKey> {
        if let Some(id) = token.strip_prefix("b:") {
            builtin::find(id).map(|b| BrushKey::Builtin(b.id))
        } else {
            token.strip_prefix("u:")?.parse().ok().map(BrushKey::User)
        }
    }

    pub fn is_user(self) -> bool {
        matches!(self, BrushKey::User(_))
    }
}

/// ブラシの設定を、比べられる正規の形にする: 画面の設定が持たないもの（色・消しゴムの印・乱数の種・背景色・描き手の設定）を
/// 既定へそろえ、基本の値は画面と同じ f32 の精度に丸める（画面の設定へ写して読み戻しても同じ値になる）。
pub fn canonical(brush: &Brush) -> Brush {
    let base = brush.base;
    let f = |v: f64| v as f32 as f64;
    Brush {
        base: BrushSettings {
            radius: f(base.radius),
            hardness: f(base.hardness),
            spacing: f(base.spacing),
            opacity: f(base.opacity),
            flow: f(base.flow),
            color: BrushSettings::default().color,
            erase: false,
            ..base
        },
        seed: 0,
        pressure: brush.pressure.rounded_to_f32(),
        // 混ぜ方が切のブラシは、混ぜの値を持たない（書かないので、読み戻しても同じ形）
        mix: if brush.mix.is_active() {
            brush.mix.rounded_to_f32()
        } else {
            ColorMix::default()
        },
        color: ColorDynamics {
            secondary: ColorDynamics::default().secondary,
            ..brush.color
        },
        assist: StrokeAssist::default(),
        stencil: None,
        symmetry: CanvasSymmetry::default(),
        ..brush.clone()
    }
}

/// 取り込んだブラシの出どころと、表せなかった項目（ブラシのファイルに残す）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportMeta {
    /// 出どころの名前（`yolu_io::brushes::Source::label`。固有名詞と版だけで、言語によらない）。
    pub source: String,
    /// Photoshop の模様から作ったブラシか（質感の画像の選びに、模様として並べる）。
    pub pattern: bool,
    /// 表せなかった項目（並び順に、重ならない）。
    pub gaps: Vec<Gap>,
}

impl ImportMeta {
    /// 項目の並びを整えて作る。
    pub fn new(source: String, pattern: bool, gaps: Vec<Gap>) -> ImportMeta {
        ImportMeta {
            source,
            pattern,
            gaps,
        }
        .normalized()
    }

    /// 項目を並び順にして重なりを除く（保存して読み戻しても同じ値になる形）。
    pub fn normalized(&self) -> ImportMeta {
        let mut gaps = self.gaps.clone();
        gaps.sort();
        gaps.dedup();
        ImportMeta {
            source: self.source.clone(),
            pattern: self.pattern,
            gaps,
        }
    }
}

/// 一覧の 1 つ。
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub key: BrushKey,
    /// 利用者のブラシの名前（組み込みは言語ごとの名前なので空）。
    pub name: String,
    pub group: Group,
    /// 元の設定（組み込みは出荷時、利用者のブラシは登録した設定。正規の形）。
    pub baseline: Brush,
    /// 元と違う設定のまま使っている間の設定（元と同じなら None）。
    pub edited: Option<Brush>,
    /// 取り込んだブラシなら、出どころと表せなかった項目。
    pub import: Option<ImportMeta>,
}

impl Entry {
    /// 今使う設定（変えていればそれ、なければ元）。
    pub fn effective(&self) -> &Brush {
        self.edited.as_ref().unwrap_or(&self.baseline)
    }

    pub fn name_in(&self, lang: Lang) -> String {
        match self.key {
            BrushKey::Builtin(id) => builtin::name(lang, id),
            BrushKey::User(_) => self.name.clone(),
        }
    }
}

/// 保存から読んだ利用者のブラシ 1 つ。
#[derive(Clone, Debug, PartialEq)]
pub struct UserBrush {
    pub id: u32,
    pub name: String,
    pub group: Group,
    pub brush: Brush,
    pub import: Option<ImportMeta>,
}

/// 利用者のブラシの番号の取り出し口。取り込みの仕事（別のスレッド）も同じ口から取るので、画面で足すブラシと番号が重ならない。
#[derive(Clone, Debug)]
pub struct IdSource(Arc<AtomicU64>);

/// 番号を使い切った印（u32 の外）。
const IDS_EXHAUSTED: u64 = u32::MAX as u64 + 1;

impl IdSource {
    fn new() -> IdSource {
        IdSource(Arc::new(AtomicU64::new(1)))
    }

    /// 番号を 1 つ取る（使い切っていたら None）。`taken` が true を返す番号は飛ばす。
    pub fn take(&self, taken: impl Fn(u32) -> bool) -> Option<u32> {
        loop {
            let n = self.0.fetch_add(1, Ordering::Relaxed);
            if n >= IDS_EXHAUSTED {
                self.0.store(IDS_EXHAUSTED, Ordering::Relaxed);
                return None;
            }
            if !taken(n as u32) {
                return Some(n as u32);
            }
        }
    }

    /// `id` までの番号は取らない。
    fn reserve_through(&self, id: u32) {
        self.0.fetch_max(id as u64 + 1, Ordering::Relaxed);
    }
}

/// 一覧の全体（並びは全グループをまたぐ 1 本の列。画面は今のグループのものだけを、この並びで出す）。
pub struct BrushLibrary {
    entries: Vec<Entry>,
    current: BrushKey,
    /// 道具ごとの一覧で選んでいるブラシ（[ブラシの一覧, 消しゴムの一覧]。道具を替えると、その道具の一覧の選びへ戻る）。
    last: [BrushKey; 2],
    /// 次に付ける利用者のブラシの番号。
    ids: IdSource,
}

/// ドラッグの落とす先。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropAt {
    /// このブラシの前（同じグループの中）。
    Before(BrushKey),
    /// グループの一番後ろ。
    End,
}

impl BrushLibrary {
    /// 組み込みと、読んだ利用者のブラシ。`order` に載っているものを先頭からその順に、載っていないものを元の並びで後ろに。
    pub fn new(users: Vec<UserBrush>, order: &[BrushKey]) -> BrushLibrary {
        let mut entries: Vec<Entry> = builtin::all()
            .iter()
            .map(|b| Entry {
                key: BrushKey::Builtin(b.id),
                name: String::new(),
                group: b.group,
                baseline: b.brush.clone(),
                edited: None,
                import: None,
            })
            .collect();
        let ids = IdSource::new();
        let mut users = users;
        users.sort_by_key(|u| u.id);
        for u in users {
            ids.reserve_through(u.id);
            entries.push(Entry {
                key: BrushKey::User(u.id),
                name: u.name,
                group: u.group,
                baseline: canonical(&u.brush),
                edited: None,
                import: u.import,
            });
        }
        let mut ordered = Vec::with_capacity(entries.len());
        for key in order {
            if let Some(at) = entries.iter().position(|e| e.key == *key) {
                ordered.push(entries.remove(at));
            }
        }
        ordered.extend(entries);
        let standard = BrushKey::Builtin(builtin::STANDARD);
        BrushLibrary {
            entries: ordered,
            current: standard,
            last: [standard, BrushKey::Builtin(builtin::STANDARD_ERASER)],
            ids,
        }
    }

    /// 読めなかった・読まなかったファイルも含めて、`id` までの番号は新しいブラシに使わない。
    pub fn reserve_ids_through(&mut self, id: u32) {
        self.ids.reserve_through(id);
    }

    /// 番号の取り出し口（取り込みの仕事が使う。画面の側と同じ番号の列を共有する）。
    pub fn id_source(&self) -> IdSource {
        self.ids.clone()
    }

    /// 新しいブラシの番号を 1 つ取る（使い切っていたら None）。`taken` が true を返す番号は飛ばす。
    fn take_id(&mut self, taken: impl Fn(u32) -> bool) -> Option<u32> {
        self.ids.take(taken)
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn entry(&self, key: BrushKey) -> Option<&Entry> {
        self.entries.iter().find(|e| e.key == key)
    }

    fn entry_mut(&mut self, key: BrushKey) -> Option<&mut Entry> {
        self.entries.iter_mut().find(|e| e.key == key)
    }

    /// 今のブラシ。
    pub fn current(&self) -> BrushKey {
        self.current
    }

    /// 今のグループのブラシ（一覧の並び）。
    pub fn in_group(&self, group: Group) -> Vec<&Entry> {
        self.entries.iter().filter(|e| e.group == group).collect()
    }

    /// 道具（消しゴムか）で最後に使ったブラシ。
    pub fn last_for(&self, eraser: bool) -> BrushKey {
        self.last[eraser as usize]
    }

    /// 全体の並び（保存する）。
    pub fn order(&self) -> Vec<BrushKey> {
        self.entries.iter().map(|e| e.key).collect()
    }

    pub fn user_count(&self) -> usize {
        self.entries.iter().filter(|e| e.key.is_user()).count()
    }

    /// 今のブラシの設定が元と違うか（`live` は今の設定）。ほかのブラシは覚えている変更で見る。
    pub fn is_modified(&self, key: BrushKey, live: &Brush) -> bool {
        match self.entry(key) {
            Some(e) if key == self.current => *live != e.baseline,
            Some(e) => e.edited.is_some(),
            None => false,
        }
    }

    /// `key` を `at` へ動かす（同じグループの中だけ。動かしたら true）。
    pub fn move_entry(&mut self, key: BrushKey, at: DropAt) -> bool {
        let Some(from) = self.entries.iter().position(|e| e.key == key) else {
            return false;
        };
        let group = self.entries[from].group;
        if let DropAt::Before(target) = at {
            if target == key || self.entry(target).map(|e| e.group) != Some(group) {
                return false;
            }
        }
        let before = self.order();
        let entry = self.entries.remove(from);
        let to = match at {
            DropAt::Before(target) => self
                .entries
                .iter()
                .position(|e| e.key == target)
                .unwrap_or(0),
            DropAt::End => self
                .entries
                .iter()
                .rposition(|e| e.group == group)
                .map_or(self.entries.len(), |i| i + 1),
        };
        self.entries.insert(to, entry);
        self.order() != before
    }

    fn unused_name(&self, base: &str) -> String {
        let taken = |name: &str| {
            self.entries
                .iter()
                .any(|e| e.key.is_user() && e.name == name)
        };
        if !taken(base) {
            return base.to_owned();
        }
        (2..)
            .map(|n| format!("{base} {n}"))
            .find(|name| !taken(name))
            .expect("名前は尽きない")
    }
}

/// 名前の整え: 制御文字は空白にし、前後の空白を落とし、長さを上限に切る。空になったら None。
pub fn clean_name(name: &str) -> Option<String> {
    let text: String = name
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let text: String = text.trim().chars().take(MAX_NAME_CHARS).collect();
    let text = text.trim().to_owned();
    (!text.is_empty()).then_some(text)
}

/// 詳細の窓のカテゴリ（今の `brush_props` の全部の欄をこの 11 に分ける）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Category {
    Shape,
    Stroke,
    /// 筆圧（大きさ・不透明度・流量・硬さの、切り替え・最小値・曲線）。
    Pressure,
    /// 入り抜き・フェード・ペンの傾き・回転・速さ。
    Dynamics,
    Jitter,
    Texture,
    Dual,
    Color,
    /// 色の混ぜ（厚塗り。混ぜ方・絵の具の量と濃さ・色延び・下地・筆圧）。
    Mix,
    Effect,
    Symmetry,
}

impl Category {
    pub const ALL: [Category; 11] = [
        Category::Shape,
        Category::Stroke,
        Category::Pressure,
        Category::Dynamics,
        Category::Jitter,
        Category::Texture,
        Category::Dual,
        Category::Color,
        Category::Mix,
        Category::Effect,
        Category::Symmetry,
    ];

    pub fn name(self, lang: Lang) -> &'static str {
        match self {
            Category::Shape => lang.pick("形状", "Shape"),
            Category::Stroke => lang.pick("ストローク", "Stroke"),
            Category::Pressure => lang.pick("筆圧", "Pen Pressure"),
            Category::Dynamics => lang.pick("入り抜きとペン", "Taper & Pen"),
            Category::Jitter => lang.pick("ゆらぎ", "Jitter"),
            Category::Texture => lang.pick("テクスチャ", "Texture"),
            Category::Dual => lang.pick("デュアルブラシ", "Dual Brush"),
            Category::Color => lang.pick("色の揺らぎ", "Color Dynamics"),
            Category::Mix => lang.pick("色の混ぜ", "Color Mixing"),
            Category::Effect => lang.pick("効果", "Effect"),
            Category::Symmetry => lang.pick("対称", "Symmetry"),
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Category::Shape => "shapes",
            Category::Stroke => "ink_stroke",
            Category::Pressure => "stylus",
            Category::Dynamics => "tune",
            Category::Jitter => "data_scatter",
            Category::Texture => "texture",
            Category::Dual => "content_copy",
            Category::Color => "palette",
            Category::Mix => "paint_brush",
            Category::Effect => "blur_on",
            Category::Symmetry => "flip",
        }
    }
}

/// ブラシの詳細の窓の状態（開いているか・カテゴリ・位置・スクロール）。
pub struct DetailWindow {
    pub open: bool,
    pub category: Category,
    /// 見出しのドラッグで動いた量（画面の真ん中からの）。
    pub offset: Vec2,
    /// 開いたとき、キャンバスの邪魔にならない初めの位置へ置いたか。
    pub placed: bool,
    pub scroll: f32,
    pub content: f32,
}

impl Default for DetailWindow {
    fn default() -> Self {
        DetailWindow {
            open: false,
            category: Category::Shape,
            offset: Vec2::ZERO,
            placed: false,
            scroll: 0.0,
            content: 0.0,
        }
    }
}

/// 一覧の行のドラッグ（動かしているブラシと、今の落とす先）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BrushDrag {
    pub key: BrushKey,
    pub target: Option<DropAt>,
}

/// ブラシの画面だけの状態。
pub struct BrushUi {
    /// 一覧に出しているグループ。
    pub group: Group,
    pub list_scroll: f32,
    pub list_content: f32,
    /// 次に一覧を描くとき、今のブラシの行が見えるところまでスクロールする（ブラシが替わったとき）。
    pub reveal: bool,
    /// 左のパネル全体のスクロール（一覧とツールプロパティとブラシサイズが入りきらないとき）。
    pub panel_scroll: f32,
    pub panel_content: f32,
    /// 名前を変えているブラシと、入力欄がフォーカスを取った後か。
    pub renaming: Option<BrushKey>,
    pub rename_started: bool,
    pub drag: Option<BrushDrag>,
    /// 右クリックのメニューの対象。
    pub context: Option<BrushKey>,
    pub detail: DetailWindow,
    /// 前のフレームに一覧を描いた範囲（描かなかったフレームでは消える。ファイルを落とした所が一覧の上かを見る）。
    pub list_rect: Option<Rect>,
}

impl Default for BrushUi {
    fn default() -> Self {
        BrushUi {
            group: Group::Pen,
            list_scroll: 0.0,
            list_content: 0.0,
            reveal: false,
            panel_scroll: 0.0,
            panel_content: 0.0,
            renaming: None,
            rename_started: false,
            drag: None,
            context: None,
            detail: DetailWindow::default(),
            list_rect: None,
        }
    }
}

/// ブラシの一覧に関わる状態の全部。
pub struct BrushesState {
    pub lib: BrushLibrary,
    pub store: Option<store::BrushStore>,
    /// 起動のとき読めなかったブラシのファイル。
    pub problems: Vec<store::Problem>,
    pub ui: BrushUi,
    pub samples: sample::SampleCache,
    /// ファイルの取り込み（裏のスレッドの仕事）。
    pub import: import::ImportState,
    /// 詳細の窓の筆先の格子に出す Krita の筆先（読み込みと見本は別のスレッド）。
    pub krita: krita::KritaTips,
}

impl Default for BrushesState {
    fn default() -> Self {
        BrushesState {
            lib: BrushLibrary::new(Vec::new(), &[]),
            store: None,
            problems: Vec::new(),
            ui: BrushUi::default(),
            samples: sample::SampleCache::default(),
            import: import::ImportState::default(),
            krita: krita::KritaTips::default(),
        }
    }
}

/// 一覧の操作（メニュー・ボタン・右クリックから）。
#[derive(Clone, Debug, PartialEq)]
pub enum BrushAction {
    /// このブラシに替える（道具も、そのブラシの道具になる）。
    Select(BrushKey),
    /// 今の設定を新しいブラシとして、今のブラシのグループの一番後ろへ足す。
    Add,
    /// このブラシ（今のブラシなら今の設定）を、すぐ後ろへ複製する。
    Duplicate(BrushKey),
    Delete(BrushKey),
    StartRename(BrushKey),
    Rename(BrushKey, String),
    Move {
        key: BrushKey,
        at: DropAt,
    },
    /// 元の設定へ戻す。
    Revert(BrushKey),
    /// 今の設定を、そのブラシの元として登録する（利用者のブラシだけ）。
    Register(BrushKey),
    /// 取り込むファイルを選ぶ窓を開く。
    ImportDialog,
    /// これらのファイルのブラシを取り込む（裏のスレッドで読んで置く）。
    Import(Vec<PathBuf>),
    /// 取り込みをやめる（置いた分は残る）。
    ImportCancel,
}

impl AppState {
    /// 今の設定（`AppState::brush` と `m2.brush`）を、比べられる正規の形で。
    pub fn brush_live(&self) -> Brush {
        let mut brush = self.m2.brush.clone();
        brush.base = self.brush.settings([1.0; 4], false);
        canonical(&brush)
    }

    /// 設定を今の設定へ写す（手ぶれ補正と入り抜きは今のまま）。
    pub fn brush_load(&mut self, brush: &Brush) {
        let base = brush.base;
        self.brush = BrushState {
            radius: base.radius as f32,
            hardness: base.hardness as f32,
            spacing: base.spacing as f32,
            opacity: base.opacity as f32,
            flow: base.flow as f32,
            pressure_size: base.pressure_size,
            pressure_opacity: base.pressure_opacity,
            pressure_flow: base.pressure_flow,
        };
        let assist = self.m2.brush.assist;
        self.m2.brush = Brush {
            assist,
            ..brush.clone()
        };
    }

    /// 今の設定を、今のブラシの「変えたままの設定」として一覧へ書き戻す（元と同じなら変更なし）。
    pub fn brush_sync(&mut self) {
        let live = self.brush_live();
        let current = self.brushes.lib.current;
        if let Some(entry) = self.brushes.lib.entry_mut(current) {
            entry.edited = (live != entry.baseline).then_some(live);
        }
    }

    /// 今のブラシの設定が元と違うか。
    pub fn brush_is_modified(&self, key: BrushKey) -> bool {
        self.brushes.lib.is_modified(key, &self.brush_live())
    }

    fn brush_refuse(&mut self) {
        self.message = self
            .lang
            .pick("描いている間はできません。", "Not while drawing.")
            .into();
    }

    /// 取り込みの仕事が走っている間は、ブラシの数・名前・画像を変える操作（追加・複製・削除・登録）を断る。取り込みは始めに取った
    /// 空きの数と名前で置き、置く画像をほかのブラシが指しているかどうかで画像の掃除が決まるので、同じときに変えると、数の上限を
    /// 超えたり、置いたブラシが指す画像を消したりする。断ったなら true。
    fn brush_refuse_while_importing(&mut self) -> bool {
        if !self.brushes.import.is_busy() {
            return false;
        }
        self.message = self
            .lang
            .pick("ブラシを取り込み中です。", "Importing brushes.")
            .into();
        true
    }

    fn brush_notice(&mut self, ja: String, en: String) {
        self.message = self.lang.pick(ja, en);
    }

    /// 設定のフォルダのブラシを読んで一覧に入れる（起動のとき 1 回）。読めなかったファイルは読み飛ばし、理由を残す。
    pub fn attach_brush_store(&mut self, dir: PathBuf) {
        let report = store::load_all(&dir);
        let order: Vec<BrushKey> = report
            .order
            .iter()
            .filter_map(|t| BrushKey::parse_token(t))
            .collect();
        self.brushes.lib = BrushLibrary::new(report.brushes, &order);
        if let Some(id) = report.max_file_id {
            self.brushes.lib.reserve_ids_through(id);
        }
        self.brushes.store = Some(store::BrushStore::new(dir));
        self.brushes.problems = report.problems;
        self.brush_sync();
    }

    /// 起動のとき読めなかったブラシのファイルの知らせ（無ければ None）。
    pub fn brush_problem_message(&self) -> Option<String> {
        let problems = &self.brushes.problems;
        let first = problems.first()?;
        let lang = self.lang;
        let one = format!("{}: {}", first.file, first.describe(lang));
        Some(if problems.len() == 1 {
            lang.pick(
                format!("ブラシを読めません。{one}"),
                format!("Cannot read a brush. {one}"),
            )
        } else {
            lang.pick(
                format!("ブラシを {} 件読めません。{one}", problems.len()),
                format!("Cannot read {} brushes. {one}", problems.len()),
            )
        })
    }

    /// 利用者のブラシのファイルを書く（失敗は知らせ、メモリの一覧は保つ）。書けた（書く必要が無かった）なら true。
    fn brush_persist(&mut self, key: BrushKey) -> bool {
        let BrushKey::User(id) = key else { return true };
        let Some(entry) = self.brushes.lib.entry(key) else {
            return true;
        };
        let user = UserBrush {
            id,
            name: entry.name.clone(),
            group: entry.group,
            brush: entry.baseline.clone(),
            import: entry.import.clone(),
        };
        let result = match &self.brushes.store {
            Some(store) => store.save_brush(&user),
            None => return true,
        };
        match result {
            Ok(()) => true,
            Err(e) => {
                let reason = e.describe(self.lang);
                self.brush_notice(
                    format!("ブラシを保存できません: {reason}"),
                    format!("Cannot save the brush: {reason}"),
                );
                false
            }
        }
    }

    fn brush_persist_order(&mut self) -> bool {
        let tokens: Vec<String> = self.brushes.lib.order().iter().map(|k| k.token()).collect();
        let result = match &self.brushes.store {
            Some(store) => store.save_order(&tokens),
            None => return true,
        };
        match result {
            Ok(()) => true,
            Err(e) => {
                let reason = e.describe(self.lang);
                self.brush_notice(
                    format!("ブラシの並びを保存できません: {reason}"),
                    format!("Cannot save the brush order: {reason}"),
                );
                false
            }
        }
    }

    /// 道具をブラシか消しゴムへ替えるときの、ブラシの切り替え（道具ごとに最後のブラシへ。今のブラシがもう同じ道具のものなら
    /// そのまま）。ストロークの最中に道具が替わるなら false（断って、何も変えない）。
    pub fn brush_for_tool(&mut self, tool: Tool) -> bool {
        if !tool.paints() {
            return true;
        }
        let eraser = tool.erases();
        let current_eraser = self
            .brushes
            .lib
            .entry(self.brushes.lib.current)
            .is_some_and(|e| e.group.is_eraser());
        if current_eraser == eraser {
            return true;
        }
        if self.is_stroking() {
            self.brush_refuse();
            return false;
        }
        self.brush_sync();
        let key = self.brushes.lib.last_for(eraser);
        self.brush_activate(key);
        true
    }

    /// `key` の設定を今の設定にして、今のブラシ・道具ごとの覚え・出すグループを替える（道具は替えない）。
    fn brush_activate(&mut self, key: BrushKey) {
        let Some(entry) = self.brushes.lib.entry(key) else {
            return;
        };
        let (group, brush) = (entry.group, entry.effective().clone());
        self.brush_load(&brush);
        let lib = &mut self.brushes.lib;
        lib.current = key;
        lib.last[group.is_eraser() as usize] = key;
        self.brushes.ui.group = group;
        self.brushes.ui.reveal = true;
        self.m2.preset = m2::presets()
            .iter()
            .position(|p| BrushKey::Builtin(p.id) == key);
    }

    /// ブラシに合わせて道具を替える（消しゴムのグループなら消しゴム、それ以外は描く道具）。
    fn brush_follow_tool(&mut self, group: Group) {
        let tool = if group.is_eraser() {
            Tool::Eraser
        } else {
            Tool::Brush
        };
        if self.tool != tool {
            // `switch_tool` は `brush_for_tool` と互いに呼び合うので通らない。離れる道具の変えたままの設定を一覧へ書き戻す
            // （入る道具はブラシか消しゴムで、サブツールの一覧は一覧自体がブラシのものなので、入るほうの写しは要らない）
            self.subtool_leave(self.tool);
            self.sel_tool_changed();
            self.tool = tool;
        }
    }

    /// 新しい利用者のブラシを足して、それに替える。`from` が None なら今の設定を「ブラシ N」として今のグループの一番後ろへ、
    /// Some なら、そのブラシ（今のブラシなら今の設定）の複製をすぐ後ろへ。
    fn brush_create(&mut self, from: Option<BrushKey>) {
        if self.is_stroking() {
            return self.brush_refuse();
        }
        if self.brush_refuse_while_importing() {
            return;
        }
        if self.brushes.lib.user_count() >= MAX_USER_BRUSHES {
            return self.brush_notice(
                format!("ブラシは {MAX_USER_BRUSHES} 個までです。"),
                format!("At most {MAX_USER_BRUSHES} brushes."),
            );
        }
        let lang = self.lang;
        self.brush_sync();
        let source_key = from.unwrap_or(self.brushes.lib.current);
        let Some(source) = self.brushes.lib.entry(source_key).cloned() else {
            return;
        };
        let base = match from {
            None => lang.pick("ブラシ", "Brush").to_owned(),
            Some(_) => lang.pick(
                format!("{} のコピー", source.name_in(lang)),
                format!("{} copy", source.name_in(lang)),
            ),
        };
        let base = clean_name(&base).unwrap_or_else(|| lang.pick("ブラシ", "Brush").into());
        let store = self.brushes.store.as_ref();
        let lib = &mut self.brushes.lib;
        // 番号は読んだファイルの続き。起動のあとに別の所で置かれたファイルの番号は飛ばす（保存は置換なので、当たると上書きする）
        let Some(id) = lib.take_id(|id| store.is_some_and(|s| s.is_taken(id))) else {
            return self.brush_notice(
                "ブラシの番号を使い切りました。".into(),
                "Out of brush numbers.".into(),
            );
        };
        let name = lib.unused_name(&base);
        let key = BrushKey::User(id);
        let after = |lib: &BrushLibrary, pred: &dyn Fn(&Entry) -> bool| {
            lib.entries
                .iter()
                .rposition(pred)
                .map_or(lib.entries.len(), |i| i + 1)
        };
        let at = match from {
            None => after(lib, &|e| e.group == source.group),
            Some(_) => after(lib, &|e| e.key == source_key),
        };
        lib.entries.insert(
            at,
            Entry {
                key,
                name: name.clone(),
                group: source.group,
                baseline: source.effective().clone(),
                edited: None,
                // 複製は元の出どころと表せなかった項目を引き継ぐ（模様のブラシの複製は、模様の一覧に重ねて並べない）。
                // 今の設定からの追加は、利用者が作ったブラシ
                import: from.and(source.import.clone()).map(|m| ImportMeta {
                    pattern: false,
                    ..m
                }),
            },
        );
        self.brush_activate(key);
        self.brush_follow_tool(source.group);
        // 知らせを先に（保存できなければ、その理由が知らせを上書きする）
        self.brush_notice(
            format!("ブラシを追加しました: {name}"),
            format!("Brush added: {name}"),
        );
        // ブラシのファイルを書けなかったなら、並びは書かない（同じ理由で書けず、知らせを上書きするだけ）
        if self.brush_persist(key) {
            self.brush_persist_order();
        }
    }

    /// 一覧の操作を当てる。
    pub fn brush_action(&mut self, action: BrushAction) {
        let lang = self.lang;
        match action {
            BrushAction::Select(key) => {
                if self.is_stroking() {
                    return self.brush_refuse();
                }
                self.brush_sync();
                let Some(group) = self.brushes.lib.entry(key).map(|e| e.group) else {
                    return;
                };
                self.brush_activate(key);
                self.brush_follow_tool(group);
            }
            BrushAction::Add => self.brush_create(None),
            BrushAction::Duplicate(key) => self.brush_create(Some(key)),
            BrushAction::Delete(key) => {
                if self.is_stroking() {
                    return self.brush_refuse();
                }
                if self.brush_refuse_while_importing() {
                    return;
                }
                let Some(entry) = self.brushes.lib.entry(key).cloned() else {
                    return;
                };
                if !key.is_user() {
                    return self.brush_notice(
                        "組み込みのブラシは消せません。".into(),
                        "Built-in brushes cannot be deleted.".into(),
                    );
                }
                self.brush_sync();
                // 今のブラシなら、同じグループの隣（なければ道具の標準）へ移る
                let next = if self.brushes.lib.current == key {
                    let group = self.brushes.lib.in_group(entry.group);
                    let at = group.iter().position(|e| e.key == key).unwrap_or(0);
                    group
                        .get(at + 1)
                        .or_else(|| at.checked_sub(1).and_then(|i| group.get(i)))
                        .map(|e| e.key)
                } else {
                    None
                };
                if let Some(store) = &self.brushes.store {
                    if let BrushKey::User(id) = key {
                        if let Err(e) = store.delete_brush(id) {
                            let reason = e.describe(lang);
                            return self.brush_notice(
                                format!("ブラシを消せません: {reason}"),
                                format!("Cannot delete the brush: {reason}"),
                            );
                        }
                    }
                }
                self.brushes.lib.entries.retain(|e| e.key != key);
                let standard = |eraser: bool| {
                    BrushKey::Builtin(if eraser {
                        builtin::STANDARD_ERASER
                    } else {
                        builtin::STANDARD
                    })
                };
                for eraser in [false, true] {
                    if self.brushes.lib.last[eraser as usize] == key {
                        self.brushes.lib.last[eraser as usize] = standard(eraser);
                    }
                }
                if self.brushes.ui.renaming == Some(key) {
                    self.brushes.ui.renaming = None;
                }
                if self.brushes.lib.current == key {
                    self.brush_activate(next.unwrap_or_else(|| standard(entry.group.is_eraser())));
                }
                let name = entry.name;
                self.brush_notice(
                    format!("ブラシを削除しました: {name}"),
                    format!("Brush deleted: {name}"),
                );
                self.brush_persist_order();
            }
            BrushAction::StartRename(key) => {
                if self.brushes.lib.entry(key).is_none() {
                    return;
                }
                if !key.is_user() {
                    return self.brush_notice(
                        "組み込みのブラシは名前を変えられません。".into(),
                        "Built-in brushes cannot be renamed.".into(),
                    );
                }
                self.brushes.ui.renaming = Some(key);
                self.brushes.ui.rename_started = false;
            }
            BrushAction::Rename(key, name) => {
                if self.brushes.ui.renaming == Some(key) {
                    self.brushes.ui.renaming = None;
                }
                let Some(name) = clean_name(&name) else {
                    return;
                };
                if !key.is_user() {
                    return;
                }
                if let Some(entry) = self.brushes.lib.entry_mut(key) {
                    if entry.name != name {
                        entry.name = name;
                        self.brush_persist(key);
                    }
                }
            }
            BrushAction::Move { key, at } => {
                if self.brushes.lib.move_entry(key, at) {
                    self.brush_persist_order();
                }
            }
            BrushAction::Revert(key) => {
                if self.is_stroking() {
                    return self.brush_refuse();
                }
                if self.brushes.lib.current == key {
                    let Some(entry) = self.brushes.lib.entry_mut(key) else {
                        return;
                    };
                    entry.edited = None;
                    let baseline = entry.baseline.clone();
                    self.brush_load(&baseline);
                } else if let Some(entry) = self.brushes.lib.entry_mut(key) {
                    entry.edited = None;
                }
            }
            BrushAction::ImportDialog => self.brush_import_dialog(),
            BrushAction::Import(paths) => self.brush_import_start(paths),
            BrushAction::ImportCancel => self.brush_import_cancel(),
            BrushAction::Register(key) => {
                if !key.is_user() {
                    return;
                }
                if self.brush_refuse_while_importing() {
                    return;
                }
                if self.brushes.lib.current == key {
                    self.brush_sync();
                }
                if let Some(entry) = self.brushes.lib.entry_mut(key) {
                    if let Some(edited) = entry.edited.take() {
                        entry.baseline = edited;
                        self.brush_persist(key);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_round_trip_and_unknown_builtins_are_dropped() {
        for key in [
            BrushKey::Builtin("pencil"),
            BrushKey::Builtin(builtin::STANDARD),
            BrushKey::User(7),
        ] {
            assert_eq!(BrushKey::parse_token(&key.token()), Some(key));
        }
        assert_eq!(BrushKey::parse_token("b:no-such-brush"), None);
        assert_eq!(BrushKey::parse_token("u:x"), None);
        assert_eq!(BrushKey::parse_token("pencil"), None);
    }

    #[test]
    fn every_group_has_a_builtin_and_every_core_preset_is_placed_once() {
        let all = builtin::all();
        for group in Group::ALL {
            assert!(all.iter().any(|b| b.group == group), "{group:?}");
        }
        for p in m2::presets() {
            assert_eq!(all.iter().filter(|b| b.id == p.id).count(), 1, "{}", p.id);
        }
        let mut ids: Vec<&str> = all.iter().map(|b| b.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), all.len(), "id は重ならない");
        // 消しゴムのグループだけが消しゴム
        for b in all {
            let preset = m2::presets().iter().find(|p| p.id == b.id);
            if let Some(p) = preset {
                assert_eq!(p.brush.base.erase, b.group.is_eraser(), "{}", b.id);
            }
        }
    }

    #[test]
    fn brush_numbers_never_repeat_across_threads_and_run_out_cleanly() {
        let ids = IdSource::new();
        let taken: Vec<u32> = std::thread::scope(|scope| {
            let workers: Vec<_> = (0..4)
                .map(|_| {
                    let ids = ids.clone();
                    scope.spawn(move || {
                        (0..250)
                            .map(|_| ids.take(|id| id % 7 == 0).unwrap())
                            .collect::<Vec<u32>>()
                    })
                })
                .collect();
            workers.into_iter().flat_map(|w| w.join().unwrap()).collect()
        });
        let mut sorted = taken.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), 1000, "どのスレッドの番号も重ならない");
        assert!(taken.iter().all(|id| id % 7 != 0), "使われている番号は飛ばす");
        // 番号を読んだファイルの続きから取り、使い切ったら None のまま
        ids.reserve_through(u32::MAX - 1);
        assert_eq!(ids.take(|_| false), Some(u32::MAX));
        assert_eq!(ids.take(|_| false), None);
        assert_eq!(ids.take(|_| false), None);
    }

    #[test]
    fn names_are_cleaned_and_bounded() {
        assert_eq!(clean_name("  ab\ncd\t"), Some("ab cd".into()));
        assert_eq!(clean_name(" \n "), None);
        let long: String = "あ".repeat(100);
        assert_eq!(clean_name(&long).unwrap().chars().count(), MAX_NAME_CHARS);
    }

    #[test]
    fn canonical_ignores_what_the_screen_does_not_keep() {
        let mut a = Brush::default();
        let mut b = Brush::default();
        a.seed = 5;
        a.base.color = crate::engine::Rgba8::new(1, 2, 3, 4);
        a.base.erase = true;
        a.assist.stabilizer = 30.0;
        a.color.secondary = crate::engine::Rgba8::new(9, 9, 9, 255);
        b.base.hardness = 0.1;
        assert_eq!(canonical(&a), canonical(&Brush::default()));
        assert_ne!(canonical(&b), canonical(&Brush::default()));
        // f32 の精度に丸める（画面の設定を通しても同じ）
        let mut c = Brush::default();
        c.base.spacing = 0.1;
        assert_eq!(canonical(&c).base.spacing, 0.1f32 as f64);
        assert_eq!(canonical(&canonical(&c)), canonical(&c));
    }
}
