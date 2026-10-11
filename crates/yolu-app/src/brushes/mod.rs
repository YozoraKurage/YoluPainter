//! ブラシの一覧（クリスタのサブツールに当たる）と、ツールごとに最後に使ったブラシの覚え。
//!
//! 一覧の 1 つ（`Entry`）は名前・グループ・設定の元（`baseline`）と、変えたままの設定（`edited`）を持つ。組み込みは消せない元で、
//! 利用者のブラシは設定のフォルダに 1 つ 1 ファイルで保存する（`store`）。今のブラシの設定は、これまでどおり `AppState::brush` と
//! `AppState::m2.brush`（スライダーやオプションバーがその場で変える）が持ち、一覧はそれとの差だけを見る: 別のブラシへ替える・ツールを
//! 替えるときに今の設定を一覧の側へ書き戻し（`brush_sync`）、替えた先の設定を今の設定へ写す（`brush_load`）。
//! 手ぶれ補正と入り抜き（`assist`）・対称・ステンシル・背景色・乱数の種は描き手の設定なので、ブラシには入れない（替えても残る）。
//! ブラシの設定は文書ではない（Undo に入れない）。ストロークの最中は、ブラシを替える操作を断る。

pub mod builtin;
pub mod clipstudio;
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

use crate::engine::{
    Brush, BrushEffect, BrushSettings, CanvasSymmetry, ColorDynamics, ColorMix, StrokeAssist,
};
use crate::lang::Lang;
use crate::m2;
use crate::state::{AppState, BrushState, Tool};
use crate::toolset::{GroupId, SlotId};

pub use gaps::Gap;
use yolu_io::brushes::SutMapped;

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

    /// 消しゴムのツール（E）で使うグループ。
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
        model_symmetry: None,
        ..brush.clone()
    }
}

/// ブラシが持てる入り抜き・手ぶれ補正（曲線は持たない）。何も無ければ（全部 0）None。範囲は呼ぶ側が検査済みのものを渡す。
pub fn carried_assist(assist: Option<StrokeAssist>) -> Option<StrokeAssist> {
    let a = assist?;
    let carried = StrokeAssist {
        stabilizer: a.stabilizer,
        taper_in: a.taper_in,
        taper_out: a.taper_out,
        curve: false,
    };
    (carried != StrokeAssist::default()).then_some(carried)
}

/// 取り込んだブラシの出どころと、ファイルから写せた項目・表せなかった項目（どちらもブラシのファイルに残す）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportMeta {
    /// 出どころの名前（`yolu_io::brushes::Source::label`。固有名詞と版だけで、言語によらない）。
    pub source: String,
    /// Photoshop の模様から作ったブラシか（質感の画像の選びに、模様として並べる）。
    pub pattern: bool,
    /// 表せなかった項目（並び順に、重ならない）。
    pub gaps: Vec<Gap>,
    /// ファイルの設定から、このアプリの設定へ写せた項目（並び順に、重ならない。今は CLIP STUDIO のものだけ。取り込みの結果
    /// `ImportedBrush::mapped` をそのまま残す。取り込んだあとに利用者が設定を変えても変わらない）。
    pub mapped: Vec<SutMapped>,
}

impl ImportMeta {
    /// 項目の並びを整えて作る（写せた項目は空。`with_mapped` で足す）。
    pub fn new(source: String, pattern: bool, gaps: Vec<Gap>) -> ImportMeta {
        ImportMeta {
            source,
            pattern,
            gaps,
            mapped: Vec::new(),
        }
        .normalized()
    }

    /// 写せた項目を足す。
    pub fn with_mapped(mut self, mapped: Vec<SutMapped>) -> ImportMeta {
        self.mapped = mapped;
        self.normalized()
    }

    /// 項目を並び順にして重なりを除く（保存して読み戻しても同じ値になる形）。
    pub fn normalized(&self) -> ImportMeta {
        let mut gaps = self.gaps.clone();
        gaps.sort();
        gaps.dedup();
        let mut mapped = self.mapped.clone();
        mapped.sort();
        mapped.dedup();
        ImportMeta {
            source: self.source.clone(),
            pattern: self.pattern,
            gaps,
            mapped,
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
    /// ブラシが持つ入り抜き・手ぶれ補正（取り込んだブラシが持つ分だけ。ふつうのブラシは None）。手ぶれ補正と入り抜きは描き手の設定
    /// （ブラシを替えても残る）なので、持つブラシを選んでいる間だけ今の設定に重ね、ほかのブラシへ替えると描き手の設定へ戻す
    /// （`AppState::brush_apply_carried_assist`）。
    pub assist: Option<StrokeAssist>,
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
    /// ブラシが持つ入り抜き・手ぶれ補正（`Entry::assist`）。
    pub assist: Option<StrokeAssist>,
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

/// 一覧の全体（組み込みの全部と、読んだ利用者のブラシのファイルの全部）。並び（どのツールのどのグループに、どの順で出すか）はツールの並び
/// （`toolset`）が持ち、ここは中身と今のブラシだけ。並びから外したブラシのファイルもここに残る（「＋」のウィンドウから戻せる）。
pub struct BrushLibrary {
    entries: Vec<Entry>,
    current: BrushKey,
    /// 次に付ける利用者のブラシの番号。
    ids: IdSource,
}

/// ドラッグの落とす先（グループの中）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropAt {
    /// このブラシの前。
    Before(BrushKey),
    /// グループの一番後ろ。
    End,
}

impl DropAt {
    /// 並びの操作に渡す「この前」（一番後ろなら None）。
    pub fn before(self) -> Option<BrushKey> {
        match self {
            DropAt::Before(key) => Some(key),
            DropAt::End => None,
        }
    }
}

impl BrushLibrary {
    /// 組み込みと、読んだ利用者のブラシ。`order` に載っているものを先頭からその順に、載っていないものを元の並びで後ろに（ツールの並びの
    /// ファイルが無いとき、今までの並び `order.conf` から最初の並びを作るのに使う）。
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
                assist: None,
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
                assist: carried_assist(u.assist),
            });
        }
        let mut ordered = Vec::with_capacity(entries.len());
        for key in order {
            if let Some(at) = entries.iter().position(|e| e.key == *key) {
                ordered.push(entries.remove(at));
            }
        }
        ordered.extend(entries);
        BrushLibrary {
            entries: ordered,
            current: BrushKey::Builtin(builtin::STANDARD),
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

    /// 全部のブラシ（並びとは関係ない順。組み込みが先で、利用者のブラシは読んだ・作った順）。
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

    pub fn user_count(&self) -> usize {
        self.entries.iter().filter(|e| e.key.is_user()).count()
    }

    /// 今のブラシの設定が元と違うか（`live` は今の設定）。ほかのブラシは覚えている変更で見る。
    pub fn is_modified(&self, key: BrushKey, live: &Brush) -> bool {
        crate::userfiles::is_modified(
            self.entry(key).map(|e| (&e.baseline, e.edited.is_some())),
            key == self.current,
            live,
        )
    }

    fn unused_name(&self, base: &str) -> String {
        crate::userfiles::unused_name(base, |name| {
            self.entries
                .iter()
                .any(|e| e.key.is_user() && e.name == name)
        })
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

/// 詳細のウィンドウのカテゴリ（今の `brush_props` の全部の欄をこの 11 に分ける）。
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
}

impl Category {
    pub const ALL: [Category; 10] = [
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
        }
    }
}

/// ブラシの詳細のウィンドウの状態（開いているか・カテゴリ・位置・スクロール）。
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

/// ブラシの画面だけの状態。
pub struct BrushUi {
    pub list_scroll: f32,
    pub list_content: f32,
    /// 次に一覧を描くとき、今のブラシの行が見えるところまでスクロールする（ブラシが替わったとき）。
    pub reveal: bool,
    /// 名前を変えているブラシと、入力欄がフォーカスを取った後か。
    pub renaming: Option<BrushKey>,
    pub rename_started: bool,
    /// 右クリックのメニューの対象。
    pub context: Option<BrushKey>,
    pub detail: DetailWindow,
    /// 前のフレームに一覧を描いた範囲（描かなかったフレームでは消える。ファイルを落とした所が一覧の上かを見る）。
    pub list_rect: Option<Rect>,
}

impl Default for BrushUi {
    fn default() -> Self {
        BrushUi {
            list_scroll: 0.0,
            list_content: 0.0,
            reveal: false,
            renaming: None,
            rename_started: false,
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
    /// 起動のときフォルダにあった利用者のブラシのファイルの番号の最大（読めなかった物も。ツールの並びのファイルに書く）。
    pub loaded_through: u32,
    /// 起動のとき、フォルダにあるのに読み込まなかった利用者のブラシのファイルの番号（ツールの並びが、その札を消さずに覚える）。
    pub unloaded: std::collections::HashSet<u32>,
    pub ui: BrushUi,
    pub samples: sample::SampleCache,
    /// ファイルの取り込み（裏のスレッドの仕事）。
    pub import: import::ImportState,
    /// 詳細のウィンドウの筆先の格子に出す Krita の筆先（読み込みと見本は別のスレッド）。
    pub krita: krita::KritaTips,
    /// 取り込みのウィンドウの「CLIP STUDIO から」。
    pub csp: clipstudio::CspState,
    /// 入り抜き・手ぶれ補正を持つブラシを選んでいる間、そのブラシへ替える前の描き手の設定を覚えておく場所（持たないブラシへ替えたら戻す）。
    pub drawer_assist: Option<StrokeAssist>,
}

impl Default for BrushesState {
    fn default() -> Self {
        BrushesState {
            lib: BrushLibrary::new(Vec::new(), &[]),
            store: None,
            problems: Vec::new(),
            loaded_through: 0,
            unloaded: std::collections::HashSet::new(),
            ui: BrushUi::default(),
            samples: sample::SampleCache::default(),
            import: import::ImportState::default(),
            krita: krita::KritaTips::default(),
            csp: clipstudio::CspState::default(),
            drawer_assist: None,
        }
    }
}

/// 一覧の操作（メニュー・ボタン・右クリックから）。
#[derive(Clone, Debug, PartialEq)]
pub enum BrushAction {
    /// このブラシに替える（ツールも、そのブラシのあるツールになる）。並びに無いブラシ（ライブラリから）は、今のグループの後ろへ置いてから。
    Select(BrushKey),
    /// 今の設定を新しいブラシとして、今のグループの一番後ろへ足す。
    Add,
    /// このブラシ（今のブラシなら今の設定）の写しを、すぐ後ろへ作る。
    Duplicate(BrushKey),
    /// 並びから外す（利用者のブラシのファイルは消さない。「＋」のウィンドウから戻せる）。
    Delete(BrushKey),
    StartRename(BrushKey),
    /// このブラシに替えて、ブラシの詳細のウィンドウを開く（右クリックのメニューの「ブラシの設定…」）。
    OpenDetail(BrushKey),
    /// 名前を変える（組み込みは、その場でファイルの写しに替えてから）。
    Rename(BrushKey, String),
    /// 同じグループの中で動かす。
    Move {
        key: BrushKey,
        at: DropAt,
    },
    /// グループ `group` の `at` へ動かす（別のグループ・別のツールへも）。`copy` なら写しを置く（元は動かない）。
    Place {
        key: BrushKey,
        group: crate::toolset::GroupId,
        at: DropAt,
        copy: bool,
    },
    /// 「＋」のウィンドウで選んだ物を、今のグループの後ろへ置く（並びにある物は写しを作る）。
    AddFrom(Vec<crate::toolset::catalog::CatalogItem>),
    /// 利用者のブラシのファイルを消す前に確かめる（ウィンドウの頼み）。
    DeleteFileDialog(BrushKey),
    /// 利用者のブラシのファイルを消す（並びからも外す）。
    DeleteFile(BrushKey),
    /// 元の設定へ戻す。
    Revert(BrushKey),
    /// 今の設定を、そのブラシの元として登録する（組み込みは、その場でファイルの写しに替えてから）。
    Register(BrushKey),
    /// 取り込むファイルを選ぶウィンドウを開く。
    ImportDialog,
    /// これらのファイルのブラシを取り込む（裏のスレッドで読んで置く）。
    Import(Vec<PathBuf>),
    /// 取り込みをやめる（置いた分は残る）。
    ImportCancel,
    /// 「CLIP STUDIO から」のウィンドウを開く（CLIP STUDIO のサブツールのフォルダを探す。読むだけ）。
    ClipStudioOpen,
    ClipStudioClose,
    /// フォルダを手で選ぶウィンドウを頼む。
    ClipStudioPickFolder,
    /// 手で選んだフォルダを探す。
    ClipStudioFolder(PathBuf),
    /// 今の場所を探し直す。
    ClipStudioRescan,
    /// 一覧の行の選びを反転する。
    ClipStudioToggle(usize),
    /// 読めた行を全部選ぶ・全部外す。
    ClipStudioSelectAll(bool),
    /// 選んだ行を取り込む。
    ClipStudioImport,
}

/// 新しい利用者のブラシの中身。
pub(crate) struct NewBrush {
    pub name: String,
    /// 元のグループ（ファイルの `group=`。最初の並びに戻すときの置き場）。
    pub group: Group,
    pub baseline: Brush,
    pub import: Option<ImportMeta>,
    pub assist: Option<StrokeAssist>,
}

impl AppState {
    /// 今の設定（`AppState::brush` と `m2.brush`）を、比べられる正規の形で。
    pub fn brush_live(&self) -> Brush {
        let mut brush = self.m2.brush.clone();
        brush.base = self.brush.settings([1.0; 4], false);
        // クローンの元から決めた offset は、ブラシの設定でなく、作業の位置: 比べる・覚えるときは、入れる前のブラシの offset と見なす
        // （「変えた」の印を付けない。変えたままの設定にも登録にも入れない）
        if let BrushEffect::Clone { offset } = &mut brush.effect {
            if let Some(own) = self.clone.brush_offset(*offset) {
                *offset = own;
            }
        }
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
            anti_alias: base.anti_alias,
        };
        let assist = self.m2.brush.assist;
        self.m2.brush = Brush {
            assist,
            ..brush.clone()
        };
        // 読んだブラシの offset をブラシの持つ値として覚え直す（2D の揃える offset が決まっていれば、それを入れ直して続ける）
        self.clone.brush_loaded(&mut self.m2.brush.effect);
    }

    /// 今の設定を、今のブラシの「変えたままの設定」として一覧へ書き戻す（元と同じなら変更なし）。
    pub fn brush_sync(&mut self) {
        let live = self.brush_live();
        let current = self.brushes.lib.current;
        if let Some(entry) = self.brushes.lib.entry_mut(current) {
            entry.edited = (live != entry.baseline).then_some(live);
        }
    }

    /// 今のブラシの設定が元と違うか。入り抜き・手ぶれ補正を持つブラシは、今の値がそのブラシの持つ値と違うことも「変えた」に数える
    /// （描き手の設定だけのブラシは、手ぶれ補正・入り抜きを変えても「変えた」にならない）。
    pub fn brush_is_modified(&self, key: BrushKey) -> bool {
        self.brushes.lib.is_modified(key, &self.brush_live()) || self.brush_assist_modified(key)
    }

    /// 持つブラシを選んでいる間に、今の入り抜き・手ぶれ補正を、持つ値から変えたか（曲線の切り替えは描き手の設定なので見ない）。
    fn brush_assist_modified(&self, key: BrushKey) -> bool {
        key == self.brushes.lib.current
            && self
                .brushes
                .lib
                .entry(key)
                .and_then(|e| e.assist)
                .is_some_and(|own| carried_assist(Some(self.m2.brush.assist)) != Some(own))
    }

    /// 一覧の行の見本に使う入り抜き・手ぶれ補正。持つブラシ（`carried`）はその持つ値、持たないブラシは描き手の設定
    /// （持つブラシを選んでいる間は今の設定にその値が重なっているので、重なる前に覚えた設定）。曲線の切り替えは今の値のまま。
    pub fn brush_row_assist(&self, carried: Option<StrokeAssist>) -> StrokeAssist {
        let drawer = self.brushes.drawer_assist.unwrap_or(self.m2.brush.assist);
        StrokeAssist {
            curve: self.m2.brush.assist.curve,
            ..carried.unwrap_or(drawer)
        }
    }

    fn brush_refuse(&mut self) {
        self.refuse(
            crate::notice::Source::Brush,
            crate::lang::refusals::during_stroke(self.lang),
        );
    }

    /// 取り込みの仕事が走っている間は、ブラシの数・名前・画像・並びを変える操作（追加・複製・削除・登録・並べ替え）を断る。取り込みは
    /// 始めに取った空きの数と名前で置き、置く画像をほかのブラシが指しているかどうかで画像の掃除が決まるので、同じときに変えると、数の上限を
    /// 超えたり、置いたブラシが指す画像を消したりする。断ったなら true。
    pub(crate) fn brush_refuse_while_importing(&mut self) -> bool {
        if !self.brushes.import.is_busy() {
            return false;
        }
        self.refuse(
            crate::notice::Source::Brush,
            self.lang
                .pick("ブラシを取り込み中です。", "Importing brushes."),
        );
        true
    }

    fn brush_notice(&mut self, kind: crate::notice::Kind, ja: String, en: String) {
        let text = self.lang.pick(ja, en);
        self.notify(kind, crate::notice::Source::Brush, text);
    }

    /// 設定のフォルダのブラシを読んで一覧に入れる（起動のとき 1 回）。読めなかったファイルは読み飛ばし、理由を残す。続けて、並び（どのツールの
    /// どのグループに出すか）を隣の `tools.json` から読む（`attach_toolset`。無ければ、ここで読んだ `order.conf` の順から作る）。
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
        self.brushes.loaded_through = report.max_file_id.unwrap_or(0);
        self.brushes.unloaded = report.unloaded.iter().copied().collect();
        self.brushes.store = Some(store::BrushStore::new(dir.clone()));
        self.brushes.problems = report.problems;
        // ツールの並びは、ブラシのフォルダの隣（設定のフォルダの直下）の tools.json
        match dir.parent() {
            Some(parent) => {
                self.attach_toolset(parent.join(crate::toolset::file::FILE_NAME));
            }
            None => {
                self.toolset.set = crate::toolset::ToolSet::initial(
                    self.brushes.lib.entries().iter().map(|e| (e.key, e.group)),
                );
                self.toolset_follow_current();
            }
        }
        self.brush_sync();
    }

    /// 起動のとき読めなかったブラシのファイルの知らせ（無ければ None）。
    pub fn brush_problem_message(&self) -> Option<String> {
        let problems = &self.brushes.problems;
        let first = problems.first()?;
        let lang = self.lang;
        let file = lang.quote(&first.file);
        let one = lang.with_reason(
            lang.pick(
                format!("ブラシのファイル{file}を読めません"),
                format!("Cannot read the brush file {file}"),
            ),
            first.describe(lang),
        );
        Some(if problems.len() == 1 {
            one
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
            assist: entry.assist,
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
                    crate::notice::Kind::Error,
                    Lang::Ja.with_reason("ブラシを保存できません", &reason),
                    Lang::En.with_reason("Cannot save the brush", &reason),
                );
                false
            }
        }
    }

    /// ツールをブラシか消しゴムのツールへ替えるときの、ブラシの切り替え（そのツールの最後のブラシへ。今のブラシがもうそのツールのものなら
    /// そのまま。ブラシの無いツールなら今のブラシのまま）。ストロークの最中にブラシが替わるなら false（断って、何も変えない）。
    pub(crate) fn brush_for_slot(&mut self, tool: Tool, slot: Option<SlotId>) -> bool {
        if !tool.paints() {
            return true;
        }
        let Some(slot) = slot else {
            return true;
        };
        let set = &self.toolset.set;
        if set.slot_of(self.brushes.lib.current) == Some(slot) {
            return true;
        }
        let Some(key) = set.pick_for(slot) else {
            return true;
        };
        if self.is_stroking() {
            self.brush_refuse();
            return false;
        }
        self.brush_sync();
        self.brush_activate(key);
        true
    }

    /// `key` の設定を今の設定にして、今のブラシ・ツールごとの覚え・出すグループを替える（ツールは替えない）。
    fn brush_activate(&mut self, key: BrushKey) {
        let Some(entry) = self.brushes.lib.entry(key) else {
            return;
        };
        let (brush, carried) = (entry.effective().clone(), entry.assist);
        self.brush_load(&brush);
        self.brush_apply_carried_assist(carried);
        self.brushes.lib.current = key;
        self.toolset.set.note_used(key);
        self.brushes.ui.reveal = true;
        self.m2.preset = m2::presets()
            .iter()
            .position(|p| BrushKey::Builtin(p.id) == key);
    }

    /// 入り抜き・手ぶれ補正を持つブラシ（`Some`）を選んだら、その値を今の設定に重ねる（重ねる前の描き手の設定は覚えておく。持つブラシから
    /// 持つブラシへ替えるときは、覚えたままにする）。持たないブラシ（`None`）を選んだら、覚えた描き手の設定へ戻す。
    fn brush_apply_carried_assist(&mut self, carried: Option<StrokeAssist>) {
        match carried {
            Some(assist) => {
                if self.brushes.drawer_assist.is_none() {
                    self.brushes.drawer_assist = Some(self.m2.brush.assist);
                }
                // 曲線の切り替えはブラシが持たない（描き手の設定のまま）
                self.m2.brush.assist = StrokeAssist {
                    curve: self.m2.brush.assist.curve,
                    ..assist
                };
            }
            None => {
                if let Some(saved) = self.brushes.drawer_assist.take() {
                    self.m2.brush.assist = StrokeAssist {
                        curve: self.m2.brush.assist.curve,
                        ..saved
                    };
                }
            }
        }
    }

    /// ブラシのあるツールへ替える（消しゴムのツールの中のブラシなら消しゴムのツール）。
    fn brush_follow_slot(&mut self, slot: Option<SlotId>) {
        let Some(slot) = slot else {
            return;
        };
        let Some(tool) = self.toolset.set.slot(slot).map(|s| s.tool) else {
            return;
        };
        if self.tool != tool {
            // `switch_tool` は `brush_for_slot` と互いに呼び合うので通らない。離れるツールの変えたままの設定を一覧へ書き戻す
            // （入るツールはブラシか消しゴムで、サブツールの一覧は一覧自体がブラシのものなので、入るほうの写しは要らない）
            self.subtool_leave(self.tool);
            self.sel_tool_changed();
            self.tool = tool;
        }
        self.toolset.set.set_active(Some(slot));
    }

    /// 新しいブラシを置くグループ（今のツールの出しているグループ。今のツールがブラシを持たなければ今のブラシのグループ、それも無ければ
    /// ペンのグループか最初のブラシのツールの先頭のグループ）。
    pub(crate) fn brush_target_group(&mut self) -> Option<GroupId> {
        let set = &self.toolset.set;
        if let Some(slot) = set.active_slot().filter(|s| s.holds_brushes()) {
            if let Some(group) = set.shown_group(slot.id) {
                return Some(group);
            }
        }
        if let Some(group) = set.group_of(self.brushes.lib.current) {
            return Some(group);
        }
        if set.locked.is_some() {
            return None;
        }
        self.toolset.set.builtin_group_or_first(Group::Pen)
    }

    /// グループ `group` に置く新しいファイルの元のグループ（消しゴムのツールの中なら消しゴム。ほかは `own`、それが消しゴムならペン）。
    fn brush_origin_for(&self, group: GroupId, own: Group) -> Group {
        let erases = self
            .toolset
            .set
            .group(group)
            .is_some_and(|(s, _)| s.tool.erases());
        match (erases, own.is_eraser()) {
            (true, _) => Group::Eraser,
            (false, true) => Group::Pen,
            (false, false) => own,
        }
    }

    /// 並びを変えられるか（新しい版・読めなかった並びのファイルなら断って false）。
    fn brush_layout_open(&mut self) -> bool {
        match self.toolset.set.lock_refusal() {
            Some(r) => {
                self.toolset_refuse(r);
                false
            }
            None => true,
        }
    }

    /// グループに、あと `n` 個置けるか（置けなければ断って false）。
    fn brush_group_has_room(&mut self, group: GroupId, n: usize) -> bool {
        let len = self
            .toolset
            .set
            .group(group)
            .map_or(0, |(_, g)| g.file_len());
        if len + n > crate::toolset::MAX_GROUP_BRUSHES {
            self.toolset_refuse(crate::toolset::Refusal::TooManyBrushes);
            return false;
        }
        true
    }

    /// 新しい利用者のブラシを一覧に足す（ファイルは書かない。呼ぶ側が `brush_persist`。並びにも置かない）。数・番号を使い切っていれば
    /// 断って None。
    pub(crate) fn brush_new_user(&mut self, new: NewBrush) -> Option<BrushKey> {
        if self.brushes.lib.user_count() >= MAX_USER_BRUSHES {
            self.brush_notice(
                crate::notice::Kind::Refusal,
                format!("ブラシは {MAX_USER_BRUSHES} 個までです。"),
                format!("At most {MAX_USER_BRUSHES} brushes."),
            );
            return None;
        }
        let lang = self.lang;
        let base = clean_name(&new.name).unwrap_or_else(|| lang.pick("ブラシ", "Brush").into());
        let store = self.brushes.store.as_ref();
        let lib = &mut self.brushes.lib;
        // 番号は読んだファイルの続き。起動のあとに別の所で置かれたファイルの番号は飛ばす（保存は置換なので、当たると上書きする）
        let Some(id) = lib.take_id(|id| store.is_some_and(|s| s.is_taken(id))) else {
            self.brush_notice(
                crate::notice::Kind::Refusal,
                "ブラシの番号を使い切りました。".into(),
                "Out of brush numbers.".into(),
            );
            return None;
        };
        let name = lib.unused_name(&base);
        let key = BrushKey::User(id);
        lib.entries.push(Entry {
            key,
            name,
            group: new.group,
            baseline: canonical(&new.baseline),
            edited: None,
            import: new.import,
            assist: carried_assist(new.assist),
        });
        Some(key)
    }

    /// `source` の写しのファイルを作る（今のブラシなら今の設定、ほかは変えたままの設定も含めた今使う設定。`pristine` なら元の設定）。
    /// 名前は `name`（重なれば番号を付ける）。グループ `group` に置く前提で、元のグループを決める。
    fn brush_copy_of(
        &mut self,
        source: BrushKey,
        name: String,
        group: GroupId,
        pristine: bool,
    ) -> Option<BrushKey> {
        let live_assist = self.m2.brush.assist;
        let current = source == self.brushes.lib.current;
        if current && !pristine {
            self.brush_sync();
        }
        let entry = self.brushes.lib.entry(source).cloned()?;
        let assist = entry.assist.and_then(|own| {
            carried_assist(Some(if current && !pristine {
                live_assist
            } else {
                own
            }))
        });
        let baseline = if pristine {
            entry.baseline.clone()
        } else {
            entry.effective().clone()
        };
        let origin = self.brush_origin_for(group, entry.group);
        self.brush_new_user(NewBrush {
            name,
            group: origin,
            baseline,
            // 写しは元の出どころと表せなかった項目を引き継ぐ（模様のブラシの写しは、模様の一覧に重ねて並べない）
            import: entry.import.map(|m| ImportMeta {
                pattern: false,
                ..m
            }),
            assist,
        })
    }

    /// 新しい利用者のブラシを足して、それに替える。`from` が None なら今の設定を「ブラシ N」として今のグループの一番後ろへ、
    /// Some なら、そのブラシ（今のブラシなら今の設定）の写しをすぐ後ろへ。
    fn brush_create(&mut self, from: Option<BrushKey>) {
        if self.is_stroking() {
            return self.brush_refuse();
        }
        if self.brush_refuse_while_importing() || !self.brush_layout_open() {
            return;
        }
        let lang = self.lang;
        self.brush_sync();
        let source_key = from.unwrap_or(self.brushes.lib.current);
        let Some(source) = self.brushes.lib.entry(source_key).cloned() else {
            return;
        };
        let placed_after = from.filter(|k| self.toolset.set.contains(*k));
        let group = match placed_after.and_then(|k| self.toolset.set.group_of(k)) {
            Some(g) => g,
            None => match self.brush_target_group() {
                Some(g) => g,
                None => return,
            },
        };
        if !self.brush_group_has_room(group, 1) {
            return;
        }
        let name = match from {
            None => lang.pick("ブラシ", "Brush").to_owned(),
            Some(_) => lang.pick(
                format!("{} のコピー", source.name_in(lang)),
                format!("{} copy", source.name_in(lang)),
            ),
        };
        let key = match from {
            Some(_) => self.brush_copy_of(source_key, name.clone(), group, false),
            None => {
                // 今の設定からの追加は、利用者が作ったブラシ（出どころは持たない）
                let live_assist = self.m2.brush.assist;
                let assist = source
                    .assist
                    .and_then(|_| carried_assist(Some(live_assist)));
                let origin = self.brush_origin_for(group, source.group);
                self.brush_new_user(NewBrush {
                    name,
                    group: origin,
                    baseline: source.effective().clone(),
                    import: None,
                    assist,
                })
            }
        };
        let Some(key) = key else {
            return;
        };
        // すぐ後ろ（写し）か、グループの一番後ろ
        let before = placed_after.and_then(|k| {
            let (_, g) = self.toolset.set.group(group)?;
            let at = g.brushes.iter().position(|b| *b == k)?;
            g.brushes.get(at + 1).copied()
        });
        if self.toolset.set.insert_brush(key, group, before).is_err() {
            return;
        }
        self.brush_activate(key);
        self.brush_follow_slot(self.toolset.set.slot_of(key));
        let name = self
            .brushes
            .lib
            .entry(key)
            .map(|e| e.name.clone())
            .unwrap_or_default();
        // 知らせを先に（保存できなければ、その理由が知らせを上書きする）
        self.brush_notice(
            crate::notice::Kind::Info,
            format!("ブラシを追加しました: {name}"),
            format!("Brush added: {name}"),
        );
        // ブラシのファイルを書けなかったなら、並びは書かない（同じ理由で書けず、知らせを上書きするだけ）
        if self.brush_persist(key) {
            self.toolset_persist();
        }
    }

    /// 並びから外れた・今のツールの外へ動いたブラシがあったあと: 今のブラシが今のツールの中に無くなったら、今のツールのブラシへ替える
    /// （今のツールにブラシが 1 つも無ければ、今のブラシのまま）。
    pub(crate) fn brush_after_unplaced(&mut self) {
        let Some(active) = self.toolset.set.active_slot().map(|s| s.id) else {
            return;
        };
        if !self
            .toolset
            .set
            .slot(active)
            .is_some_and(|s| s.holds_brushes())
        {
            return;
        }
        if self.toolset.set.slot_of(self.brushes.lib.current) == Some(active) {
            return;
        }
        if let Some(key) = self.toolset.set.pick_for(active) {
            self.brush_sync();
            self.brush_activate(key);
        }
    }

    /// 並びにある組み込み `key` を、同じ場所の利用者のブラシのファイルの写しに替える（名前を変える・この設定で登録するとき）。
    /// `register` なら変えたままの設定を元にし、ほかは元の設定のまま変えたままの設定を引き継ぐ。替えた印。
    fn brush_convert_builtin(
        &mut self,
        key: BrushKey,
        name: String,
        register: bool,
    ) -> Option<BrushKey> {
        if key.is_user() || !self.toolset.set.contains(key) {
            return None;
        }
        if self.brush_refuse_while_importing() || !self.brush_layout_open() {
            return None;
        }
        let current = self.brushes.lib.current == key;
        if current {
            self.brush_sync();
        }
        let entry = self.brushes.lib.entry(key).cloned()?;
        let group = self.toolset.set.group_of(key)?;
        let origin = self.brush_origin_for(group, entry.group);
        let (baseline, edited) = if register {
            (entry.effective().clone(), None)
        } else {
            (entry.baseline.clone(), entry.edited.clone())
        };
        let new = self.brush_new_user(NewBrush {
            name,
            group: origin,
            baseline,
            import: None,
            assist: None,
        })?;
        if let Some(e) = self.brushes.lib.entry_mut(new) {
            e.edited = edited;
        }
        if let Some(e) = self.brushes.lib.entry_mut(key) {
            e.edited = None;
        }
        self.toolset.set.replace_brush(key, new);
        if current {
            // 今の設定はそのまま（同じ設定の写しへ替えるだけ）
            self.brushes.lib.current = new;
            self.m2.preset = None;
        }
        if self.brushes.ui.renaming == Some(key) {
            self.brushes.ui.renaming = None;
        }
        if self.brush_persist(new) {
            self.toolset_persist();
        }
        Some(new)
    }

    /// グループの写し（中のブラシの写しのファイルも作る）を、すぐ後ろに作る。
    pub(crate) fn brush_duplicate_group(&mut self, group: GroupId) {
        if self.toolset_blocked() || !self.brush_layout_open() {
            return;
        }
        let lang = self.lang;
        let Some((slot, source)) = self
            .toolset
            .set
            .group(group)
            .map(|(s, g)| (s.id, g.clone()))
        else {
            return;
        };
        let room = MAX_USER_BRUSHES.saturating_sub(self.brushes.lib.user_count());
        if source.brushes.len() > room {
            return self.brush_notice(
                crate::notice::Kind::Refusal,
                format!("ブラシは {MAX_USER_BRUSHES} 個までです。"),
                format!("At most {MAX_USER_BRUSHES} brushes."),
            );
        }
        let name = lang.pick(
            format!("{} のコピー", source.name_in(lang)),
            format!("{} copy", source.name_in(lang)),
        );
        let before = self.toolset.set.slot(slot).and_then(|s| {
            let at = s.groups.iter().position(|g| g.id == group)?;
            s.groups.get(at + 1).map(|g| g.id)
        });
        let new_group = match self.toolset.set.add_group(slot, before, name, Vec::new()) {
            Ok(g) => g,
            Err(r) => return self.toolset_refuse(r),
        };
        self.brush_sync();
        let mut failed = false;
        for key in source.brushes {
            let entry_name = self
                .brushes
                .lib
                .entry(key)
                .map(|e| e.name_in(lang))
                .unwrap_or_default();
            let Some(copy) = self.brush_copy_of(key, entry_name, new_group, false) else {
                failed = true;
                break;
            };
            let _ = self.toolset.set.insert_brush(copy, new_group, None);
            if !self.brush_persist(copy) {
                failed = true;
                break;
            }
        }
        self.toolset.set.show_group(new_group);
        if !failed {
            let name = self
                .toolset
                .set
                .group(new_group)
                .map(|(_, g)| g.name_in(lang))
                .unwrap_or_default();
            self.brush_notice(
                crate::notice::Kind::Info,
                format!("グループを複製しました: {name}"),
                format!("Group duplicated: {name}"),
            );
        }
        self.toolset_persist();
    }

    /// 「＋」のウィンドウで選んだ物を、今のグループの後ろへ置く（並びにある物・同梱の Krita は写しのファイルを作る）。最初に置いた物に替える。
    pub(crate) fn brush_add_from(&mut self, items: Vec<crate::toolset::catalog::CatalogItem>) {
        use crate::toolset::catalog::CatalogItem;
        if items.is_empty() {
            return;
        }
        if self.is_stroking() {
            return self.brush_refuse();
        }
        if self.brush_refuse_while_importing() || !self.brush_layout_open() {
            return;
        }
        let lang = self.lang;
        let Some(group) = self.brush_target_group() else {
            return;
        };
        if !self.brush_group_has_room(group, items.len()) {
            return;
        }
        self.brush_sync();
        let mut added: Vec<BrushKey> = Vec::new();
        let mut write_failed = false;
        for item in items {
            let key = match item {
                CatalogItem::Builtin(id) => {
                    let Some(b) = builtin::find(id) else { continue };
                    let key = BrushKey::Builtin(b.id);
                    if self.toolset.set.contains(key) {
                        self.brush_copy_of(key, builtin::name(lang, b.id), group, true)
                    } else {
                        Some(key)
                    }
                }
                CatalogItem::User(id) => {
                    let key = BrushKey::User(id);
                    let Some(entry) = self.brushes.lib.entry(key).cloned() else {
                        continue;
                    };
                    if self.toolset.set.contains(key) {
                        self.brush_copy_of(key, entry.name, group, true)
                    } else {
                        Some(key)
                    }
                }
                CatalogItem::Krita(index) => {
                    let Some(b) = store::krita().brushes.get(index) else {
                        continue;
                    };
                    let mut brush = b.brush.clone();
                    let assist = carried_assist(Some(brush.assist));
                    brush.base.radius = brush
                        .base
                        .radius
                        .clamp(0.5, crate::state::MAX_RADIUS as f64);
                    let import =
                        ImportMeta::new(b.source.label(), false, gaps::fold(&b.unrepresented));
                    let own = match self.toolset.set.group(group).and_then(|(_, g)| g.builtin) {
                        Some(g) => g,
                        None => Group::Brush,
                    };
                    let origin = self.brush_origin_for(group, own);
                    self.brush_new_user(NewBrush {
                        name: b.name.clone(),
                        group: origin,
                        baseline: brush,
                        import: Some(import),
                        assist,
                    })
                }
            };
            let Some(key) = key else {
                break;
            };
            if key.is_user()
                && !matches!(item, CatalogItem::User(id) if BrushKey::User(id) == key)
                && !self.brush_persist(key)
            {
                write_failed = true;
                // 一覧には残る（並びにも置く。次に保存し直したときに書く）
            }
            if self.toolset.set.insert_brush(key, group, None).is_ok() {
                added.push(key);
            }
            if write_failed {
                break;
            }
        }
        let Some(first) = added.first().copied() else {
            return;
        };
        self.brush_activate(first);
        self.brush_follow_slot(self.toolset.set.slot_of(first));
        if !write_failed {
            if added.len() == 1 {
                let name = self
                    .brushes
                    .lib
                    .entry(first)
                    .map(|e| e.name_in(lang))
                    .unwrap_or_default();
                self.brush_notice(
                    crate::notice::Kind::Info,
                    format!("ブラシを追加しました: {name}"),
                    format!("Brush added: {name}"),
                );
            } else {
                let n = added.len();
                self.brush_notice(
                    crate::notice::Kind::Info,
                    format!("ブラシを {n} 個追加しました。"),
                    format!("{n} brushes added."),
                );
            }
        }
        self.toolset_persist();
    }

    /// 一覧の操作を当てる。
    pub fn brush_action(&mut self, action: BrushAction) {
        let lang = self.lang;
        match action {
            BrushAction::Select(key) => {
                if self.is_stroking() {
                    return self.brush_refuse();
                }
                if self.brushes.lib.entry(key).is_none() {
                    return;
                }
                if !self.toolset.set.contains(key) {
                    // 並びに無いブラシ（ライブラリの「使う」）は、今のグループの後ろへ置いてから
                    if self.brush_refuse_while_importing() || !self.brush_layout_open() {
                        return;
                    }
                    let Some(group) = self.brush_target_group() else {
                        return;
                    };
                    if !self.brush_group_has_room(group, 1) {
                        return;
                    }
                    if self.toolset.set.insert_brush(key, group, None).is_err() {
                        return;
                    }
                    self.toolset_persist();
                }
                self.brush_sync();
                self.brush_activate(key);
                self.brush_follow_slot(self.toolset.set.slot_of(key));
            }
            BrushAction::Add => self.brush_create(None),
            BrushAction::Duplicate(key) => self.brush_create(Some(key)),
            BrushAction::Delete(key) => {
                if self.is_stroking() {
                    return self.brush_refuse();
                }
                if self.brush_refuse_while_importing() || !self.brush_layout_open() {
                    return;
                }
                let Some(place) = self.toolset.set.find(key) else {
                    return;
                };
                let name = self
                    .brushes
                    .lib
                    .entry(key)
                    .map(|e| e.name_in(lang))
                    .unwrap_or_default();
                self.brush_sync();
                // 今のブラシなら、同じグループの隣（なければツールの最後のブラシ）へ移る
                let next = if self.brushes.lib.current == key {
                    let group = &self.toolset.set.slots()[place.slot].groups[place.group].brushes;
                    group
                        .get(place.index + 1)
                        .or_else(|| place.index.checked_sub(1).and_then(|i| group.get(i)))
                        .copied()
                } else {
                    None
                };
                if let Err(r) = self.toolset.set.remove_brush(key) {
                    return self.toolset_refuse(r);
                }
                if self.brushes.ui.renaming == Some(key) {
                    self.brushes.ui.renaming = None;
                }
                if self.brushes.lib.current == key {
                    match next {
                        Some(next) => self.brush_activate(next),
                        None => self.brush_after_unplaced(),
                    }
                }
                self.brush_notice(
                    crate::notice::Kind::Info,
                    format!("ブラシを削除しました: {name}"),
                    format!("Brush deleted: {name}"),
                );
                self.toolset_persist();
            }
            BrushAction::DeleteFileDialog(key) => {
                if !key.is_user() || self.brushes.lib.entry(key).is_none() {
                    return;
                }
                if self.is_stroking() {
                    return self.brush_refuse();
                }
                if self.brush_refuse_while_importing() {
                    return;
                }
                self.toolset.catalog.pending_delete = Some(key);
                self.dialog_request = Some(crate::state::DialogRequest::BrushFileDelete);
            }
            BrushAction::DeleteFile(key) => {
                if self.is_stroking() {
                    return self.brush_refuse();
                }
                if self.brush_refuse_while_importing() {
                    return;
                }
                let BrushKey::User(id) = key else {
                    return;
                };
                let Some(entry) = self.brushes.lib.entry(key).cloned() else {
                    return;
                };
                let placed = self.toolset.set.contains(key);
                if placed && !self.brush_layout_open() {
                    return;
                }
                self.brush_sync();
                if let Some(store) = &self.brushes.store {
                    if let Err(e) = store.delete_brush(id) {
                        let reason = e.describe(lang);
                        return self.brush_notice(
                            crate::notice::Kind::Error,
                            Lang::Ja.with_reason("ブラシを消せません", &reason),
                            Lang::En.with_reason("Cannot delete the brush", &reason),
                        );
                    }
                }
                let _ = self.toolset.set.remove_brush(key);
                self.brushes.lib.entries.retain(|e| e.key != key);
                self.toolset.catalog.forget(key);
                if self.brushes.ui.renaming == Some(key) {
                    self.brushes.ui.renaming = None;
                }
                if self.brushes.lib.current == key {
                    let next = self
                        .toolset
                        .set
                        .active_slot()
                        .and_then(|s| self.toolset.set.pick_for(s.id))
                        .unwrap_or(BrushKey::Builtin(builtin::STANDARD));
                    self.brush_activate(next);
                }
                let name = entry.name;
                self.brush_notice(
                    crate::notice::Kind::Info,
                    format!("ブラシのファイルを削除しました: {name}"),
                    format!("Brush file deleted: {name}"),
                );
                if placed {
                    self.toolset_persist();
                }
            }
            BrushAction::OpenDetail(key) => {
                self.brush_action(BrushAction::Select(key));
                // 替えられたとき（描いている間・並びが読めないときの断りでは替わらない）だけ開く
                if self.brushes.lib.current == key {
                    self.brushes.ui.detail.open = true;
                }
            }
            BrushAction::StartRename(key) => {
                if self.brushes.lib.entry(key).is_none() {
                    return;
                }
                if !key.is_user() && !self.toolset.set.contains(key) {
                    return;
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
                if let BrushKey::Builtin(id) = key {
                    // 組み込みの名前のままなら、何もしない（写しを作らない）
                    if builtin::name(lang, id) != name {
                        self.brush_convert_builtin(key, name, false);
                    }
                    return;
                }
                if self.brush_refuse_while_importing() {
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
                if self.toolset_blocked() {
                    return;
                }
                let Some(group) = self.toolset.set.group_of(key) else {
                    return;
                };
                let result = self.toolset.set.move_brush(key, group, at.before());
                match result {
                    Ok(true) => {
                        self.toolset_persist();
                    }
                    Ok(false) => {}
                    Err(r) => self.toolset_refuse(r),
                }
            }
            BrushAction::Place {
                key,
                group,
                at,
                copy,
            } => {
                if self.toolset_blocked() || !self.brush_layout_open() {
                    return;
                }
                if self.toolset.set.group(group).is_none() {
                    return;
                }
                if copy || !self.toolset.set.contains(key) {
                    if !self.brush_group_has_room(group, 1) {
                        return;
                    }
                    let Some(source) = self.brushes.lib.entry(key).map(|e| e.name_in(lang)) else {
                        return;
                    };
                    let name = lang.pick(format!("{source} のコピー"), format!("{source} copy"));
                    let Some(new) = self.brush_copy_of(key, name, group, false) else {
                        return;
                    };
                    if self
                        .toolset
                        .set
                        .insert_brush(new, group, at.before())
                        .is_err()
                    {
                        return;
                    }
                    if self.brush_persist(new) {
                        self.toolset_persist();
                    }
                    return;
                }
                match self.toolset.set.move_brush(key, group, at.before()) {
                    Ok(true) => {
                        self.brush_after_unplaced();
                        self.toolset_persist();
                    }
                    Ok(false) => {}
                    Err(r) => self.toolset_refuse(r),
                }
            }
            BrushAction::AddFrom(items) => self.brush_add_from(items),
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
                    let carried = entry.assist;
                    self.brush_load(&baseline);
                    self.brush_apply_carried_assist(carried);
                } else if let Some(entry) = self.brushes.lib.entry_mut(key) {
                    entry.edited = None;
                }
            }
            BrushAction::ImportDialog => self.brush_import_dialog(),
            BrushAction::Import(paths) => self.brush_import_start(paths),
            BrushAction::ImportCancel => self.brush_import_cancel(),
            BrushAction::ClipStudioOpen => self.brush_csp_open(),
            BrushAction::ClipStudioClose => self.brush_csp_close(),
            BrushAction::ClipStudioPickFolder => self.brush_csp_pick_folder(),
            BrushAction::ClipStudioFolder(folder) => self.brush_csp_folder(folder),
            BrushAction::ClipStudioRescan => self.brush_csp_scan(),
            BrushAction::ClipStudioToggle(index) => self.brush_csp_toggle(index),
            BrushAction::ClipStudioSelectAll(on) => self.brush_csp_select_all(on),
            BrushAction::ClipStudioImport => self.brush_csp_import(),
            BrushAction::Register(key) => {
                if self.brush_refuse_while_importing() {
                    return;
                }
                if !key.is_user() {
                    // 組み込みは、変えたままの設定を元にした写しのファイルに替える
                    if self.brush_is_modified(key) {
                        let name = match key {
                            BrushKey::Builtin(id) => builtin::name(lang, id),
                            BrushKey::User(_) => return,
                        };
                        self.brush_convert_builtin(key, name, true);
                    }
                    return;
                }
                let is_current = self.brushes.lib.current == key;
                if is_current {
                    self.brush_sync();
                }
                // 持つブラシは、今の入り抜き・手ぶれ補正（全部 0 なら持たない）も、そのブラシの持つ値として登録する
                let live_assist = carried_assist(Some(self.m2.brush.assist));
                let mut changed = false;
                if let Some(entry) = self.brushes.lib.entry_mut(key) {
                    if let Some(edited) = entry.edited.take() {
                        entry.baseline = edited;
                        changed = true;
                    }
                    if is_current && entry.assist.is_some() && entry.assist != live_assist {
                        entry.assist = live_assist;
                        changed = true;
                    }
                }
                if changed {
                    self.brush_persist(key);
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
            workers
                .into_iter()
                .flat_map(|w| w.join().unwrap())
                .collect()
        });
        let mut sorted = taken.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), 1000, "どのスレッドの番号も重ならない");
        assert!(
            taken.iter().all(|id| id % 7 != 0),
            "使われている番号は飛ばす"
        );
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
