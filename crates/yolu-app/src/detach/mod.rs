//! ドックの欄を、アプリのウィンドウの外の OS のウィンドウ（egui の immediate の viewport）へ出す。
//!
//! 浮かせた欄はいつも OS のウィンドウで、アプリのウィンドウの上にも外（別の画面）にも置ける。egui_dock はウィンドウを viewport に出す仕組みを持たず、浮いたウィンドウは
//! アプリのウィンドウの中の `egui::Window` なので、別ウィンドウごとに `DockState<Tab>`（主の面だけ）を 1 つ持ち、そのウィンドウの viewport の中で `DockArea` を
//! 描く。egui_dock が浮いたウィンドウの面（`Surface::Window`）を作ったら、そのフレームのうちに別ウィンドウへ替える（`take_floats`）。
//!
//! 出し方: タブの右クリックの「別ウィンドウで開く」、タブをウィンドウの外で離す、egui_dock のウィンドウの落とし先（組の中ほど）で離す。戻し方: 別ウィンドウのタブを
//! アプリのウィンドウの中で離す（その点の下の組へ）、右クリックの「ドックに戻す」、OS のウィンドウを閉じる（中のタブを全部、戻る先の組へ）。
//! 戻る先は、出したときに同じ組にいたタブの組（`OsWindow::home`）、それが無ければ既定の並びでいた組、1 枚だけの組だったタブは既定の並びの隣のタブの組を分けた新しい組、それも無ければメインウィンドウの右の組。
//!
//! ここは並びの操作（どのウィンドウのどの組にどのタブがあるか）と置き場所の計算だけで、描くのは `app` の `detached`。

pub mod menu;
#[cfg(windows)]
pub(crate) mod native;
pub mod place;

use std::collections::HashMap;

use egui::{Pos2, Rect, ViewportId};
use egui_dock::{DockState, Node, NodeIndex, Split, SurfaceIndex, TabIndex, Tree};

use crate::layout::FloatRecord;
use crate::pen::PenInput;
use crate::Tab;

/// 別ウィンドウの内側の最小の大きさ（点）。
pub const MIN_SIZE: [f32; 2] = [200.0, 150.0];
/// 新しく出すウィンドウの内側の大きさの上限（点。出す元の組が大きいとき）。
pub const MAX_NEW_SIZE: [f32; 2] = [900.0, 1000.0];
/// 出す元の組の大きさが分からないときの、内側の大きさ（点）。
pub const DEFAULT_SIZE: [f32; 2] = [360.0, 480.0];
/// タブをウィンドウの外で離したとき、新しいウィンドウの外枠の左上を、離した点からどれだけ左上へずらすか（点。タブの帯がポインタの下に来るように）。
pub const GRAB_OFFSET: [f32; 2] = [40.0, 12.0];

/// ドックの操作（メニュー・右クリックから。`YoluApp` が同じフレームのうちに当てる）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DockOp {
    /// タブを新しい別ウィンドウへ出す。
    Detach(Tab),
    /// 別ウィンドウのタブを、アプリのウィンドウのドックへ戻す。
    Return(Tab),
    /// パネルを前に出す（タブを選び、別ウィンドウにあればそのウィンドウを前へ。どこにも無ければ、既定の並びでいた組へ開く）。
    Show(Tab),
    /// パネルを閉じる（メインウィンドウからも別ウィンドウからも外す。`layout::HIDEABLE` のタブだけ。空になった別ウィンドウは消える）。
    Hide(Tab),
}

/// 画面のメニューが読む、タブのありか（毎フレーム `YoluApp` が入れる）。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PanelIndex {
    /// どこかに開いているタブ（メインウィンドウ・別ウィンドウ）。
    pub open: Vec<Tab>,
    /// 別ウィンドウにあるタブ。
    pub outside: Vec<Tab>,
    /// 別ウィンドウの中の、ただ 1 つのタブ（「別ウィンドウで開く」は出さない）。
    pub alone: Vec<Tab>,
}

/// 別ウィンドウの最初の置き場所（作るときに 1 度だけ使う。作った後は利用者が動かした所）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Place {
    /// 外枠の左上と内側の大きさ（点）と、その点を書いたときの拡大率（画素 = 点 × 拡大率）。
    Record(FloatRecord),
    /// メインウィンドウの内側の左上からの点（前の版の、アプリの中の浮いた欄）。最初に描くときに、メインウィンドウの今の位置で決める。
    OverMain { offset: [f32; 2], size: [f32; 2] },
    /// メインウィンドウの上の中央（覚えた位置が読めなかったウィンドウ）。
    Center { size: [f32; 2] },
}

/// 作るときに viewport へ渡す値（点）と、作った後に合わせにいく置き場所（Windows）。
#[derive(Debug)]
pub(crate) struct Resolved {
    pub position: Option<[f32; 2]>,
    pub size: [f32; 2],
    pub settle: Option<crate::windowpos::Settle>,
}

/// ウィンドウに落としたファイルの行き先の矩形（そのウィンドウで描いたもの）。ブラシの一覧（PNG を筆先として取り込む）とライブラリの格子。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DropRects {
    pub brush_list: Option<Rect>,
    pub library_grid: Option<Rect>,
}

/// 別ウィンドウ 1 つ。
pub struct OsWindow {
    /// 起動してからの通し番号（viewport の名前のもと。保存しない）。
    pub serial: u64,
    /// 中のドック（主の面だけ。浮いたウィンドウの面ができたら、そのフレームで別の別ウィンドウへ替える）。
    pub dock: DockState<Tab>,
    /// 戻る先: 出したときに同じ組にいたタブ（並びの順）。
    pub home: Vec<Tab>,
    /// 最初の置き場所。
    pub place: Place,
    /// 最後に見た外枠の左上・内側の大きさ・拡大率（保存に使う。まだ見ていなければ `place` の値）。
    pub record: Option<FloatRecord>,
    /// 作るときに渡す値（最初に描くときに `place` から決める）。
    pub(crate) resolved: Option<Resolved>,
    /// 最後のフレームのタブの見出しの矩形（試験用。このウィンドウの点）。
    pub tab_rects: HashMap<Tab, Rect>,
    /// このウィンドウのペンの点（Windows Ink、または macOS のタブレットをウィンドウに繋いだら入る。位置はこのウィンドウの内側の画素）。
    pub(crate) pen: PenInput,
    /// 見つけた OS のウィンドウ（Windows の HWND の値。持ち主とペンを繋いだウィンドウ）。
    #[cfg(windows)]
    pub(crate) hwnd: Option<isize>,
    /// OS のウィンドウを探した回数（Windows と macOS。上限で諦める。macOS は題名が変わったら 0 に戻す）。
    #[cfg(any(windows, target_os = "macos"))]
    pub(crate) attach_tries: u32,
    /// OS のウィンドウを探したときの、ビューポートの題名（macOS。題名が変わったら探し直す）。
    #[cfg(target_os = "macos")]
    pub(crate) attach_title: String,
    /// このウィンドウで描いた、落としたファイルの行き先。
    pub(crate) drops: DropRects,
}

impl OsWindow {
    /// 試験用: このウィンドウのペンの受け口の、ウィンドウをアプリの側で動かす手を差し替える（None は手が無い受け口。Windows 以外と同じ）。
    #[doc(hidden)]
    pub fn set_pen_mover(&mut self, mover: Option<std::sync::Arc<dyn crate::pen::WindowMover>>) {
        self.pen = self.pen.clone().with_mover(mover);
    }

    /// このウィンドウの viewport。
    pub fn viewport_id(&self) -> ViewportId {
        viewport_of(self.serial)
    }

    /// 中のタブ（並びの順）。
    pub fn tabs(&self) -> Vec<Tab> {
        self.dock.iter_all_tabs().map(|(_, t)| *t).collect()
    }

    /// ウィンドウの題名（中のタブの名前を「 · 」でつなぐ。前のタブが先）。
    pub fn title(&self, lang: crate::lang::Lang) -> String {
        let mut tabs = Vec::new();
        for (_, node) in self.dock.iter_all_nodes() {
            if let Node::Leaf(leaf) = node {
                if let Some(active) = leaf.tabs.get(leaf.active.0) {
                    tabs.push(*active);
                }
                tabs.extend(
                    leaf.tabs
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| *i != leaf.active.0)
                        .map(|(_, t)| *t),
                );
            }
        }
        tabs.iter()
            .map(|t| t.title_in(lang))
            .collect::<Vec<_>>()
            .join(" · ")
    }
}

/// 内側の矩形が合った OS のウィンドウ（ハンドル・題名。列挙の順）から、別ウィンドウのものを決める（Windows の `native::find_window` が使う）。1 つだけならそれ。
/// 同じ矩形に別のウィンドウが重なっていて複数あるときは、題名が `title` と同じ 1 つ（題名は中のタブの名前で、ウィンドウごとに違う）。1 つに決まらなければ
/// None（繋がずに、次のフレームで探し直す）。
pub fn pick_window(found: &[(isize, String)], title: &str) -> Option<isize> {
    match found {
        [] => None,
        [(hwnd, _)] => Some(*hwnd),
        _ => {
            let mut same = found.iter().filter(|(_, t)| t == title);
            match (same.next(), same.next()) {
                (Some((hwnd, _)), None) => Some(*hwnd),
                _ => None,
            }
        }
    }
}

/// 別ウィンドウの viewport（通し番号から）。
pub fn viewport_of(serial: u64) -> ViewportId {
    ViewportId::from_hash_of(("yolu.detached", serial))
}

/// 別ウィンドウの全部。
#[derive(Default)]
pub struct Detached {
    pub windows: Vec<OsWindow>,
    next_serial: u64,
}

impl Detached {
    pub fn new() -> Detached {
        Detached::default()
    }

    /// 新しい別ウィンドウを足す（中身は `dock` の主の面）。通し番号を返す。
    pub fn open(&mut self, dock: DockState<Tab>, home: Vec<Tab>, place: Place) -> u64 {
        self.next_serial += 1;
        let serial = self.next_serial;
        let record = match place {
            Place::Record(r) => Some(r),
            Place::OverMain { .. } | Place::Center { .. } => None,
        };
        self.windows.push(OsWindow {
            serial,
            dock,
            home,
            place,
            record,
            resolved: None,
            tab_rects: HashMap::new(),
            pen: PenInput::detached(),
            #[cfg(windows)]
            hwnd: None,
            #[cfg(any(windows, target_os = "macos"))]
            attach_tries: 0,
            #[cfg(target_os = "macos")]
            attach_title: String::new(),
            drops: DropRects::default(),
        });
        serial
    }

    /// 通し番号のウィンドウ。
    pub fn get(&self, serial: u64) -> Option<&OsWindow> {
        self.windows.iter().find(|w| w.serial == serial)
    }

    /// viewport のウィンドウの番号（並びの中の位置）。
    pub fn index_of_viewport(&self, id: ViewportId) -> Option<usize> {
        self.windows.iter().position(|w| w.viewport_id() == id)
    }

    /// タブがある別ウィンドウ（並びの中の位置）。
    pub fn window_of(&self, tab: Tab) -> Option<usize> {
        self.windows
            .iter()
            .position(|w| w.dock.find_main_surface_tab(&tab).is_some())
    }

    /// タブが別ウィンドウにあるか。
    pub fn contains(&self, tab: Tab) -> bool {
        self.window_of(tab).is_some()
    }

    /// タブのありか（メニューが読む）。
    pub fn index(&self, main: &DockState<Tab>) -> PanelIndex {
        let mut out = PanelIndex::default();
        out.open.extend(main.iter_all_tabs().map(|(_, t)| *t));
        for w in &self.windows {
            let tabs = w.tabs();
            if tabs.len() == 1 {
                out.alone.push(tabs[0]);
            }
            out.outside.extend(tabs.iter().copied());
            out.open.extend(tabs);
        }
        out
    }

    /// 空になったウィンドウを除く。
    pub fn drop_empty(&mut self) {
        self.windows
            .retain(|w| w.dock.main_surface().num_tabs() > 0);
    }

    /// タブを、今ある所（メインウィンドウ・別ウィンドウ）から外す。外した所がメインウィンドウなら、同じ組に残ったタブを戻る先として返す（別ウィンドウなら、そのウィンドウの
    /// 戻る先）。どこにも無ければ None。空になった別ウィンドウは消す。
    fn take(&mut self, main: &mut DockState<Tab>, tab: Tab) -> Option<Vec<Tab>> {
        if let Some((node, index)) = main.find_main_surface_tab(&tab) {
            let path = egui_dock::TabPath::new(SurfaceIndex::main(), node, index);
            let mates: Vec<Tab> = main
                .leaf(path.node_path())
                .map(|leaf| leaf.tabs.iter().copied().filter(|t| *t != tab).collect())
                .unwrap_or_default();
            main.remove_tab(path);
            return Some(mates);
        }
        let w = self.window_of(tab)?;
        let window = &mut self.windows[w];
        let (node, index) = window.dock.find_main_surface_tab(&tab)?;
        window
            .dock
            .remove_tab(egui_dock::TabPath::new(SurfaceIndex::main(), node, index));
        let home = window.home.clone();
        self.drop_empty();
        Some(home)
    }

    /// タブを新しい別ウィンドウへ出す（`place` の置き場所）。どこにも無いタブは出さない（false）。
    pub fn detach(&mut self, main: &mut DockState<Tab>, tab: Tab, place: Place) -> bool {
        let Some(home) = self.take(main, tab) else {
            return false;
        };
        self.open(DockState::new(vec![tab]), home, place);
        true
    }

    /// 別ウィンドウのタブをメインウィンドウのドックへ戻す。`at` はメインウィンドウの点（その点の下の組へ。無ければ戻る先の組）。メインウィンドウにあるタブは何もしない。
    pub fn return_tab(&mut self, main: &mut DockState<Tab>, tab: Tab, at: Option<Pos2>) -> bool {
        if self.window_of(tab).is_none() {
            return false;
        }
        let Some(home) = self.take(main, tab) else {
            return false;
        };
        dock_into(main, tab, &home, at);
        true
    }

    /// タブを別ウィンドウ `serial` の、`at`（そのウィンドウの点）の下の組へ移す（無ければ最初の組）。
    pub fn move_into(
        &mut self,
        main: &mut DockState<Tab>,
        tab: Tab,
        serial: u64,
        at: Option<Pos2>,
    ) -> bool {
        if self
            .get(serial)
            .is_none_or(|w| w.dock.find_main_surface_tab(&tab).is_some())
        {
            return false;
        }
        let Some(home) = self.take(main, tab) else {
            return false;
        };
        // （外したウィンドウが空になって消えても、行き先のウィンドウは別のウィンドウなので残っている）
        match self.windows.iter_mut().find(|w| w.serial == serial) {
            Some(window) => {
                dock_into(&mut window.dock, tab, &[], at);
                if window.home.is_empty() {
                    window.home = home;
                }
                true
            }
            None => {
                dock_into(main, tab, &home, None);
                false
            }
        }
    }

    /// 別ウィンドウを閉じる: 中のタブを全部、戻る先の組へ戻す（組の分け方は捨てる）。前だったタブは戻した先でも前にする。
    pub fn close(&mut self, main: &mut DockState<Tab>, serial: u64) {
        let Some(at) = self.windows.iter().position(|w| w.serial == serial) else {
            return;
        };
        let window = self.windows.remove(at);
        let mut fronts = Vec::new();
        for (_, node) in window.dock.iter_all_nodes() {
            if let Node::Leaf(leaf) = node {
                fronts.extend(leaf.tabs.get(leaf.active.0).copied());
            }
        }
        for (_, tab) in window.dock.iter_all_tabs() {
            dock_into(main, *tab, &window.home, None);
        }
        for tab in fronts {
            if let Some(path) = main.find_tab(&tab) {
                let _ = main.set_active_tab(path);
            }
        }
    }

    /// パネルを前に出す: タブを選ぶ。別ウィンドウにあれば、そのウィンドウの viewport を返す（呼ぶ側がウィンドウを前へ）。どこにも無ければ、既定の並びで
    /// いた組（無ければメインウィンドウの右の組）へ開いて選ぶ。
    pub fn show(&mut self, main: &mut DockState<Tab>, tab: Tab) -> Option<ViewportId> {
        if let Some(path) = main.find_tab(&tab) {
            let _ = main.set_active_tab(path);
            return None;
        }
        if let Some(w) = self.window_of(tab) {
            let window = &mut self.windows[w];
            if let Some(path) = window.dock.find_tab(&tab) {
                let _ = window.dock.set_active_tab(path);
            }
            return Some(window.viewport_id());
        }
        dock_into(main, tab, &[], None);
        None
    }

    /// パネルを閉じる: メインウィンドウからも別ウィンドウからも外す（空になった別ウィンドウは消す）。閉じられるのは `layout::HIDEABLE` のタブだけ（違えば false）。
    /// どこにも無いタブも false。
    pub fn hide(&mut self, main: &mut DockState<Tab>, tab: Tab) -> bool {
        crate::layout::HIDEABLE.contains(&tab) && self.take(main, tab).is_some()
    }

    /// 別ウィンドウを全部閉じる（並びを既定へ戻すとき。タブは呼ぶ側が既定の並びで持つ）。
    pub fn clear(&mut self) {
        self.windows.clear();
    }
}

/// 主の面の、点 `at` の下の組（最後に描いた組の矩形で見る）。
pub fn leaf_at(dock: &DockState<Tab>, at: Pos2) -> Option<NodeIndex> {
    dock.main_surface()
        .iter()
        .enumerate()
        .find_map(|(i, node)| match node {
            Node::Leaf(leaf) if leaf.rect.is_finite() && leaf.rect.contains(at) => {
                Some(NodeIndex(i))
            }
            _ => None,
        })
}

/// 既定の並びで、タブと同じ組にいたタブ（自分を除く、並びの順）。1 つだけの組にいたタブは、上の組のタブ（ツールプロパティとブラシサイズは、縦に並んだ
/// サブツールの組）。
pub fn default_mates(tab: Tab) -> Vec<Tab> {
    // 既定では閉じているナビゲーターは、開くとテクスチャセットの組へ（前の既定の並びの組）
    if tab == Tab::Navigator {
        return vec![Tab::TextureSets];
    }
    let dock = crate::app::default_dock();
    let Some(path) = dock.find_tab(&tab) else {
        // 既定の並びに無いタブ（ポーズ）は、レイヤーの組へ（`panels::pose::ensure_tab` と同じ）
        return vec![Tab::Layers];
    };
    let mates: Vec<Tab> = dock
        .leaf(path.node_path())
        .map(|leaf| leaf.tabs.iter().copied().filter(|t| *t != tab).collect())
        .unwrap_or_default();
    if !mates.is_empty() {
        return mates;
    }
    match tab {
        Tab::ToolProperties | Tab::BrushSize => vec![Tab::SubTools],
        _ => mates,
    }
}

/// 既定の並びで 1 枚だけの組だったタブ（3D ビュー・キャンバス・サブツール）を、組を作り直して戻すときの分け方: 既定の並びで隣にいたタブ（`dock` の
/// 主の面にあるもののうち、先に挙げたもの）と、そのタブの組を分ける向き、新しい組（左・上の子）の取り分。向きと取り分は `app::default_dock_for` と同じ定数。
/// - 3D ビュー: キャンバスの組の左へ。
/// - キャンバス: 3D ビューの組の右へ（取り分は左の子の 3D ビューのもの）。
/// - サブツール: ツールプロパティの組の上へ（無ければブラシサイズ、それも無ければカラーの上へ）。
pub fn default_split(dock: &DockState<Tab>, tab: Tab) -> Option<(Tab, Split, f32)> {
    use crate::app::{
        CENTER_VIEW3D, LEFT_BRUSH_SIZE, LEFT_COLOR, LEFT_SUB_TOOLS, LEFT_TOOL_PROPERTIES,
    };
    let candidates: Vec<(Tab, Split, f32)> = match tab {
        Tab::View3d => vec![(Tab::Canvas, Split::Left, CENTER_VIEW3D)],
        Tab::Canvas => vec![(Tab::View3d, Split::Right, CENTER_VIEW3D)],
        Tab::SubTools => [
            (Tab::ToolProperties, LEFT_TOOL_PROPERTIES),
            (Tab::BrushSize, LEFT_BRUSH_SIZE),
            (Tab::Color, LEFT_COLOR),
        ]
        .into_iter()
        .map(|(neighbor, share)| {
            (
                neighbor,
                Split::Above,
                LEFT_SUB_TOOLS / (LEFT_SUB_TOOLS + share),
            )
        })
        .collect(),
        _ => Vec::new(),
    };
    candidates
        .into_iter()
        .find(|(neighbor, _, _)| dock.find_main_surface_tab(neighbor).is_some())
}

/// タブを `dock` の主の面へ入れて前にする: `at` の下の組 → 戻る先 `home` のタブの組 → 既定の並びで同じ組だったタブの組 → 既定の並びで隣にいたタブの組を
/// 分けて自分の組を作り直す（`default_split`）→ 右の組（最後に描いた組の矩形で右端。まだ描いていなければ最初の組）。主の面が空なら、新しい組を作る。
pub fn dock_into(dock: &mut DockState<Tab>, tab: Tab, home: &[Tab], at: Option<Pos2>) {
    let surface = SurfaceIndex::main();
    let target = at.and_then(|p| leaf_at(dock, p)).or_else(|| {
        home.iter()
            .chain(default_mates(tab).iter())
            .find_map(|t| dock.find_main_surface_tab(t).map(|(node, _)| node))
    });
    let target = match target {
        Some(node) => Some(node),
        None => {
            if let Some((neighbor, split, share)) = default_split(dock, tab) {
                if let Some((node, _)) = dock.find_main_surface_tab(&neighbor) {
                    dock.main_surface_mut()
                        .split_tabs(node, split, share, vec![tab]);
                    return;
                }
            }
            rightmost_leaf(dock.main_surface())
        }
    };
    match target {
        Some(node) => {
            if let Ok(leaf) = dock.leaf_mut(egui_dock::NodePath { surface, node }) {
                leaf.append_tab(tab);
                return;
            }
            dock.main_surface_mut().push_to_first_leaf(tab);
        }
        None => dock.main_surface_mut().push_to_first_leaf(tab),
    }
}

/// 右端の組（最後に描いた矩形の右の端が一番右。まだ描いていなければ最初の組）。
fn rightmost_leaf(tree: &Tree<Tab>) -> Option<NodeIndex> {
    let leaves: Vec<(usize, Rect)> = tree
        .iter()
        .enumerate()
        .filter_map(|(i, node)| match node {
            Node::Leaf(leaf) => Some((i, leaf.rect)),
            _ => None,
        })
        .collect();
    let drawn = leaves
        .iter()
        .filter(|(_, r)| r.is_finite() && r.width() > 0.0)
        .max_by(|a, b| {
            (a.1.right(), -a.1.top())
                .partial_cmp(&(b.1.right(), -b.1.top()))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(i, _)| NodeIndex(*i));
    drawn.or_else(|| leaves.first().map(|(i, _)| NodeIndex(*i)))
}

/// 主の面の、タブごとの同じ組のタブ（自分を除く）。フレームの頭に控え、egui_dock がタブを浮いたウィンドウへ動かしたあとで戻る先に使う。
pub fn leaf_mates(dock: &DockState<Tab>) -> HashMap<Tab, Vec<Tab>> {
    let mut out = HashMap::new();
    for node in dock.main_surface().iter() {
        if let Node::Leaf(leaf) = node {
            for tab in &leaf.tabs {
                out.insert(
                    *tab,
                    leaf.tabs.iter().copied().filter(|t| t != tab).collect(),
                );
            }
        }
    }
    out
}

/// egui_dock の浮いたウィンドウ 1 つ（別ウィンドウへ替える前）。
#[derive(Debug)]
pub struct Float {
    pub dock: DockState<Tab>,
    /// そのウィンドウの矩形（描いた viewport の点。まだ描いていなければ、最初に描くときの位置と大きさ）。
    pub rect: Rect,
}

/// `dock` の浮いたウィンドウの面を全部外し、別ウィンドウの中身にする形で返す（面の番号の順）。`drawn` は、描いたウィンドウの矩形（egui が覚えている
/// ウィンドウの矩形。egui_dock は最初に描くときに位置と大きさの頼みを使い切り、ウィンドウの矩形を自分では持たないので、描いた後はこちらから読む）。
pub fn take_floats(
    dock: &mut DockState<Tab>,
    drawn: impl Fn(SurfaceIndex) -> Option<Rect>,
) -> Vec<Float> {
    let indices: Vec<SurfaceIndex> = dock
        .iter_surfaces_indexed()
        .filter(|(_, s)| matches!(s, egui_dock::Surface::Window(..)))
        .map(|(i, _)| i)
        .collect();
    let mut out = Vec::new();
    for index in indices.into_iter().rev() {
        let Some(egui_dock::Surface::Window(tree, state)) = dock.remove_surface(index) else {
            continue;
        };
        let rect = drawn(index)
            .filter(|r| r.is_finite() && r.width() > 0.0 && r.height() > 0.0)
            .unwrap_or_else(|| window_rect(&state));
        let mut inner = DockState::new(Vec::new());
        *inner.main_surface_mut() = tree;
        clear_focus(&mut inner);
        if inner.main_surface().num_tabs() > 0 {
            out.push(Float { dock: inner, rect });
        }
    }
    out.reverse();
    // 主の面のフォーカスが、消した面を指したまま残らないように
    dock.set_focused_node_and_surface(egui_dock::NodePath {
        surface: SurfaceIndex(usize::MAX),
        node: NodeIndex::root(),
    });
    out
}

/// egui_dock が浮いたウィンドウに付ける egui のウィンドウの名前（`window {面の番号}`）。
pub fn float_area_id(index: SurfaceIndex) -> egui::Id {
    egui::Id::new(format!("window {index:?}"))
}

/// 別ウィンドウの中身のドックのフォーカスを外す（ウィンドウの木を移したとき、元の木のフォーカスの番号が残らないように）。
fn clear_focus(dock: &mut DockState<Tab>) {
    dock.main_surface_mut()
        .set_focused_node(NodeIndex(usize::MAX));
}

/// egui_dock の浮いたウィンドウの、最初に描くときの位置と大きさ（読み口が無いので、書き出した形から）。
fn window_rect(state: &egui_dock::WindowState) -> Rect {
    let value = serde_json::to_value(state).unwrap_or(serde_json::Value::Null);
    let pair = |key: &str| -> Option<[f32; 2]> {
        let v = value.get(key)?;
        let (x, y) = match v {
            serde_json::Value::Object(o) => (o.get("x")?.as_f64()?, o.get("y")?.as_f64()?),
            serde_json::Value::Array(a) => (a.first()?.as_f64()?, a.get(1)?.as_f64()?),
            _ => return None,
        };
        Some([x as f32, y as f32])
    };
    let position = pair("next_position").unwrap_or([120.0, 120.0]);
    let size = pair("next_size").unwrap_or(DEFAULT_SIZE);
    Rect::from_min_size(position.into(), size.into())
}

/// 新しく出すウィンドウの内側の大きさ（点）: 出す元の組の大きさを、最小と上限の間へ。分からなければ既定。
pub fn new_window_size(leaf: Option<Rect>) -> [f32; 2] {
    match leaf.filter(|r| r.is_finite() && r.width() > 1.0 && r.height() > 1.0) {
        Some(r) => [
            r.width().clamp(MIN_SIZE[0], MAX_NEW_SIZE[0]),
            r.height().clamp(MIN_SIZE[1], MAX_NEW_SIZE[1]),
        ],
        None => DEFAULT_SIZE,
    }
}

/// タブがいる組の矩形（主の面。描いた viewport の点）。
pub fn leaf_rect(dock: &DockState<Tab>, tab: Tab) -> Option<Rect> {
    let (node, _) = dock.find_main_surface_tab(&tab)?;
    match &dock.main_surface()[node] {
        Node::Leaf(leaf) => Some(leaf.rect),
        _ => None,
    }
}

/// タブを前にする（そのタブの組の中で）。
pub fn activate(dock: &mut DockState<Tab>, tab: Tab) {
    if let Some((node, index)) = dock.find_main_surface_tab(&tab) {
        let _ = dock.set_active_tab(egui_dock::TabPath::new(
            SurfaceIndex::main(),
            node,
            TabIndex(index.0),
        ));
    }
}

/// 別ウィンドウのアイコン（メインウィンドウと同じロゴ。1 度だけ読む）。
pub fn app_icon() -> std::sync::Arc<egui::IconData> {
    static ICON: std::sync::OnceLock<std::sync::Arc<egui::IconData>> = std::sync::OnceLock::new();
    ICON.get_or_init(|| {
        let png = include_bytes!("../../assets/logo/yolupainter-256.png");
        let image = image::load_from_memory(png)
            .map(|i| i.into_rgba8())
            .unwrap_or_default();
        let (width, height) = image.dimensions();
        std::sync::Arc::new(egui::IconData {
            rgba: image.into_raw(),
            width,
            height,
        })
    })
    .clone()
}
