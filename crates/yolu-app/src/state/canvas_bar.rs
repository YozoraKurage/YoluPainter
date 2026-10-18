//! キャンバスの右上の欄（`canvas::bar`）の中身の形: 描くチャンネルのボタンと、3 つの列の行。操作の型 `A` は呼び手が決める（状態の側が `Action` で作り、
//! 画面の側が並べて描く）。作るのは `AppState::canvas_bar_chip`・`canvas_bar_columns`（`sets.rs`）。

/// 描くチャンネルのボタンの中身。
#[derive(Clone, Debug)]
pub struct Chip {
    pub icon: &'static str,
    pub name: String,
}

/// チャンネルの行の右端の目（見せるチャンネル）。
#[derive(Clone, Debug)]
pub struct Eye<A> {
    /// 見せているチャンネルの行（青い目）。
    pub showing: bool,
    /// 部品の名前に使う番号（チャンネルの番号）。
    pub key: usize,
    pub tip: String,
    pub action: A,
}

/// 欄の 1 行。
#[derive(Clone, Debug)]
pub struct Row<A> {
    /// 部品の名前に使う値（種類, 番号, 添字）。
    pub key: (&'static str, u128, usize),
    pub icon: Option<&'static str>,
    pub label: String,
    /// 選んでいる行（青い行）。
    pub selected: bool,
    /// 押したときの操作（描いていない間だけ当てる）。
    pub action: A,
    pub eye: Option<Eye<A>>,
}

/// 列の見出しの右の ＋。
#[derive(Clone, Debug)]
pub struct Add<A> {
    pub tip: &'static str,
    pub enabled: bool,
    pub action: A,
}

/// 欄の 1 列。見出しは、アイコン（無ければ文字）と、ツールチップ（アイコンが無いときは文字）。
#[derive(Clone, Debug)]
pub struct Column<A> {
    pub icon: Option<&'static str>,
    pub head: &'static str,
    pub add: Option<Add<A>>,
    pub rows: Vec<Row<A>>,
}
