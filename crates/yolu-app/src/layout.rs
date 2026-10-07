//! 画面の並びの保存: ドックの並び（タブの組・分け方・大きさ・どのタブが前か・浮かせた窓）と、窓の大きさ・位置・最大化を、設定のフォルダの
//! `layout.json` へ書き、次の起動で戻す。ドックは egui_dock が持つ serde の形（`DockState` をそのまま）、タブは安定した名前（`Tab::key`）で
//! 書く。書くのは、並びが変わったとき（1 秒おき・ドラッグの最中でない）と終わるとき。書き方は一時ファイルから置き換える 1 回の操作。
//! 浮かせた窓の位置と大きさは、egui が覚えている窓の矩形を保存のときに入れ（egui_dock は自分では更新しない）、読み戻すと、egui_dock が
//! 最初に描くときの位置と大きさとして使う。
//!
//! 読めない・古い版・知らないタブ・タブが足りない／重なる・大きすぎる・egui_dock が添字で引いて落ちる値（前のタブの番号・木の子・空の組・
//! 分け方・窓の位置）のどれでも、そのファイルのドックは捨てて既定の並び（`app::default_dock`）で始める（理由は診断のログだけで、画面には
//! 出さない）。窓の大きさ・位置は、ドックとは別に確かめる（ドックを捨てても窓は戻す）。

use std::io;
use std::path::{Path, PathBuf};

use egui_dock::DockState;
use serde_json::{json, Value};

use crate::Tab;

/// 設定のフォルダの中のファイル名。
pub const FILE_NAME: &str = "layout.json";
/// ファイルの形の版（形を変えたら上げる。知らない版は読まずに既定の並び）。
pub const FORMAT: u64 = 1;
/// 読む大きさの上限（これを超えるファイルは壊れているとして読まない）。
const MAX_FILE_BYTES: u64 = 1024 * 1024;
/// 窓の最小の内側の大きさ（点。`main` の最小の大きさと同じ）。
pub const MIN_SIZE: [f32; 2] = [960.0, 640.0];
/// 窓の大きさの上限（点。これより大きい値は壊れた値）。
const MAX_SIZE: f32 = 16384.0;
/// 窓の位置の範囲（点。これより外は壊れた値）。
const MAX_POSITION: f32 = 32000.0;

/// 設定のフォルダの `layout.json`（設定のフォルダが分からなければ None）。
pub fn path() -> Option<PathBuf> {
    crate::settings::path().and_then(|p| path_for(&p))
}

/// 設定のファイル（`settings.conf`）と同じフォルダの `layout.json`。
pub fn path_for(settings: &Path) -> Option<PathBuf> {
    Some(settings.parent()?.join(FILE_NAME))
}

/// 窓の大きさと位置（最大化していない状態のもの）と、最大化していたか。
///
/// 位置と大きさは点で、点 = 画素 / `pixels_per_point`（書いたときに窓がいた画面の拡大率。アプリは egui の拡大を使わないので OS の論理の点と
/// 同じ）。画素 = 点 × `pixels_per_point` は仮想スクリーンの物理画素で、拡大率の違う画面をまたぐときは、点の座標を別の画面の拡大率で
/// 読み替えない（`windowpos::plan` が画素に直してから、今の画面と突き合わせる）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowRecord {
    /// 外枠の左上（点）。
    pub position: [f32; 2],
    /// 内側の大きさ（点）。
    pub size: [f32; 2],
    /// 書いたときの 1 点あたりの画素（窓がいた画面の拡大率）。
    pub pixels_per_point: f32,
    pub maximized: bool,
}

impl WindowRecord {
    /// 値が正しいか（有限・範囲の中）。大きさは最小の大きさまで引き上げる。正しくなければ None。
    pub fn sanitized(self) -> Option<WindowRecord> {
        let finite = self
            .position
            .iter()
            .chain(&self.size)
            .chain([&self.pixels_per_point])
            .all(|v| v.is_finite());
        let range = self.position.iter().all(|v| v.abs() <= MAX_POSITION)
            && self.size.iter().all(|v| (1.0..=MAX_SIZE).contains(v))
            && (0.25..=8.0).contains(&self.pixels_per_point);
        (finite && range).then_some(WindowRecord {
            size: [self.size[0].max(MIN_SIZE[0]), self.size[1].max(MIN_SIZE[1])],
            ..self
        })
    }

    fn to_json(self) -> Value {
        json!({
            "x": self.position[0], "y": self.position[1],
            "width": self.size[0], "height": self.size[1],
            "pixels_per_point": self.pixels_per_point,
            "maximized": self.maximized,
        })
    }

    fn from_json(value: &Value) -> Option<WindowRecord> {
        let number = |key: &str| value.get(key)?.as_f64().map(|v| v as f32);
        WindowRecord {
            position: [number("x")?, number("y")?],
            size: [number("width")?, number("height")?],
            pixels_per_point: number("pixels_per_point")?,
            maximized: value
                .get("maximized")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        }
        .sanitized()
    }
}

/// 読んだ結果。
#[derive(Debug, Default)]
pub struct Loaded {
    /// 読めて、正しいドックの並び（無ければ既定の並び）。
    pub dock: Option<DockState<Tab>>,
    /// 読めた窓の大きさと位置。
    pub window: Option<WindowRecord>,
    /// 捨てた理由（診断のログに書く文。画面には出さない）。
    pub problems: Vec<String>,
}

/// ファイルを読む。無いときは何も無い結果（理由も無い）。読めないものは理由つきで捨てる。
pub fn load(path: &Path) -> Loaded {
    let text = match std::fs::metadata(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Loaded::default(),
        Err(e) => {
            return problem(format!(
                "画面の並びのファイルを調べられません（{e}）。既定の並びで始めます。"
            ))
        }
        Ok(meta) if meta.len() > MAX_FILE_BYTES => {
            return problem("画面の並びのファイルが大きすぎます。既定の並びで始めます。".into());
        }
        Ok(_) => match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) => {
                return problem(format!(
                    "画面の並びのファイルを読めません（{e}）。既定の並びで始めます。"
                ))
            }
        },
    };
    parse(&text)
}

fn problem(text: String) -> Loaded {
    Loaded {
        problems: vec![text],
        ..Loaded::default()
    }
}

/// ファイルの中身を読む。
pub fn parse(text: &str) -> Loaded {
    let Ok(root) = serde_json::from_str::<Value>(text) else {
        return problem(
            "画面の並びのファイルが JSON として読めません。既定の並びで始めます。".into(),
        );
    };
    let mut out = Loaded::default();
    let Some(format) = root.get("format").and_then(Value::as_u64) else {
        return problem("画面の並びのファイルに版がありません。既定の並びで始めます。".into());
    };
    if format != FORMAT {
        return problem(format!("画面の並びのファイルの版 {format} は読めません（この版は {FORMAT}）。既定の並びで始めます。"));
    }
    // 窓の大きさと位置は、ドックとは別に確かめる（ドックを捨てても窓は戻す）
    match root.get("window") {
        None | Some(Value::Null) => {}
        Some(value) => match WindowRecord::from_json(value) {
            Some(window) => out.window = Some(window),
            None => out.problems.push(
                "窓の大きさと位置の値が正しくありません。窓は既定の大きさで始めます。".into(),
            ),
        },
    }
    let Some(dock) = root.get("dock") else {
        out.problems
            .push("画面の並びのファイルにドックの並びがありません。既定の並びで始めます。".into());
        return out;
    };
    match serde_json::from_value::<DockState<Tab>>(dock.clone()) {
        Ok(mut dock) => {
            forget_focus(&mut dock);
            match validate(&dock) {
                Ok(()) => {
                    add_missing_tabs(&mut dock);
                    out.dock = Some(dock);
                }
                Err(reason) => out.problems.push(format!(
                    "ドックの並びを使えません（{reason}）。既定の並びで始めます。"
                )),
            }
        }
        Err(e) => out.problems.push(format!(
            "ドックの並びを読めません（{e}）。既定の並びで始めます。"
        )),
    }
    out
}

/// 読んだドックの「フォーカスしている面・組」を外す。ファイルの値のままだと、無い面や組を指していても確かめられず（面のほうは読み口が無い）、
/// egui_dock が描くときに添字で引いて落ちる。egui_dock 自身も、メインの組を全部浮かせたあとなどに、もう無い組を指したまま残すことがある。
/// フォーカスは、クリックしたとき egui_dock が付け直す（既定の並びで始めたときと同じ、フォーカスなし）。
fn forget_focus(dock: &mut DockState<Tab>) {
    // 無い場所を指すと、egui_dock は「ドック全体のフォーカスしている面」を外す
    dock.set_focused_node_and_surface(egui_dock::NodePath {
        surface: egui_dock::SurfaceIndex(usize::MAX),
        node: egui_dock::NodeIndex::root(),
    });
    for surface in dock.iter_surfaces_mut() {
        if let Some(tree) = surface.node_tree_mut() {
            // 無い組を指すと、その木のフォーカスが外れる
            tree.set_focused_node(egui_dock::NodeIndex(usize::MAX));
        }
    }
}

/// 保存した並びに無くてよいタブ。ポーズはスキンのあるモデルを読むと足される。ログは後の版で足したタブで、それより前の版が保存した
/// 並びには無い（無いだけで並び全部を捨てないよう、読んだときに `add_missing_tabs` が足す）。アクションは既定の並びに無く、開いたときだけある。
pub const OPTIONAL_TABS: [Tab; 3] = [Tab::Pose, Tab::Log, Tab::Actions];

/// 後の版で足したタブが、読んだ並びに無ければ足す: ログは既定の並びと同じく、レイヤーと同じ組の後ろ（レイヤーが無ければ最初の組。
/// 前へは出さない）。
pub fn add_missing_tabs(dock: &mut DockState<Tab>) {
    if dock.find_tab(&Tab::Log).is_some() {
        return;
    }
    push_beside_layers(dock, Tab::Log);
}

/// タブを、レイヤーと同じ組の後ろへ入れる（レイヤーが無ければ最初の組）。
fn push_beside_layers(dock: &mut DockState<Tab>, tab: Tab) {
    let target = dock.find_tab(&Tab::Layers).map(|p| p.node_path());
    match target.and_then(|path| dock.leaf_mut(path).ok()) {
        Some(leaf) => leaf.tabs.push(tab),
        None => dock.push_to_first_leaf(tab),
    }
}

/// パネルを開いて前に出す（`Action::ShowPanel`）: ドックのどこか（浮かせた窓も）にあればそのタブを選び、無ければレイヤーと同じ組の後ろへ
/// 入れて選ぶ（既定の並びに無いパネル。アクション）。
pub fn show_tab(dock: &mut DockState<Tab>, tab: Tab) {
    if dock.find_tab(&tab).is_none() {
        push_beside_layers(dock, tab);
    }
    if let Some(path) = dock.find_tab(&tab) {
        let _ = dock.set_active_tab(path);
    }
}

/// 読んだドックが使えるか。egui_dock は読んだ値をそのまま添字で引くので、描く前に次を確かめる（外れていれば理由。外れたまま渡すと、
/// 起動のたびに落ちて、並びのファイルを手で消すまで起動できなくなる）。
/// - 面: 先頭が主の面で、それ以外に主の面が無い。浮かせた窓の面は、組を 1 つ以上持ち、窓の位置と大きさが有限で範囲の中。
/// - 木: 分けた所の両側が木の中にあり、根からつながらない節が無い（見えないタブができる）。分け方は有限で 0 と 1 の間。
/// - 組: 空でなく、前のタブの番号がタブの数の中。浮かせた窓の木のフォーカスは、木の中の組（読むときは `forget_focus` で外してある）。
/// - タブ: どのタブも 1 つずつ（ポーズ・ログ・アクションは、無くてよい。ポーズはモデルを読むと足され、ログは読んだときに
///   `add_missing_tabs` が足し、アクションは開いたときだけある）。
///   足りない・重なるなら理由。
pub fn validate(dock: &DockState<Tab>) -> Result<(), String> {
    use egui_dock::Surface;
    if !matches!(dock.iter_surfaces().next(), Some(Surface::Main(_))) {
        return Err("先頭の面が主の面でない".into());
    }
    for (index, surface) in dock.iter_surfaces_indexed() {
        match surface {
            Surface::Empty => {}
            Surface::Main(tree) => {
                if index.0 != 0 {
                    return Err("主の面が 2 つある".into());
                }
                validate_tree(tree)?;
            }
            Surface::Window(tree, state) => {
                validate_tree(tree)?;
                // 浮かせた窓の木のフォーカスは、描くときに組として引く（メインの木は、組を全部浮かせたあとに、もう無い組を指したままでよい）
                if let Some(focus) = tree.focused_leaf() {
                    if !tree.iter().nth(focus.0).is_some_and(|node| node.is_leaf()) {
                        return Err(format!(
                            "浮かせた窓 {} のフォーカスが組を指していない",
                            index.0
                        ));
                    }
                }
                if !tree.iter().any(|node| node.is_leaf()) {
                    return Err(format!("浮かせた窓 {} に組が無い", index.0));
                }
                // 窓の状態の値（最初に描くときの位置と大きさ）。中身は読み口が無いので、書き出した形で確かめる
                let value = serde_json::to_value(state).unwrap_or(Value::Null);
                let position = (-f64::from(MAX_POSITION), f64::from(MAX_POSITION));
                let size = (1.0, f64::from(MAX_SIZE));
                for (key, (low, high)) in [
                    ("screen_rect", position),
                    ("next_position", position),
                    ("next_size", size),
                ] {
                    if !value
                        .get(key)
                        .is_none_or(|v| v.is_null() || bounded(v, low, high))
                    {
                        return Err(format!("浮かせた窓 {} の {key} が範囲の外", index.0));
                    }
                }
            }
        }
    }
    let mut count = std::collections::HashMap::new();
    for (_, tab) in dock.iter_all_tabs() {
        *count.entry(*tab).or_insert(0usize) += 1;
    }
    for tab in Tab::ALL {
        let n = count.get(&tab).copied().unwrap_or(0);
        let optional = OPTIONAL_TABS.contains(&tab);
        if n > 1 {
            return Err(format!("タブ {} が {n} つある", tab.key()));
        }
        if n == 0 && !optional {
            return Err(format!("タブ {} が無い", tab.key()));
        }
    }
    Ok(())
}

/// 値（数・入れ子の数）が全部、`low` から `high` の中か（無限大は JSON で null になるので、null も外れ）。
fn bounded(value: &Value, low: f64, high: f64) -> bool {
    match value {
        Value::Number(n) => n.as_f64().is_some_and(|v| (low..=high).contains(&v)),
        Value::Array(items) => items.iter().all(|v| bounded(v, low, high)),
        Value::Object(fields) => fields.values().all(|v| bounded(v, low, high)),
        _ => false,
    }
}

/// 木 1 本の形（`validate` の木と組の項目）。木は完全二分木の並びで、添字 i の子は 2i+1 と 2i+2。
fn validate_tree(tree: &egui_dock::Tree<Tab>) -> Result<(), String> {
    use egui_dock::Node;
    let nodes: Vec<&Node<Tab>> = tree.iter().collect();
    let mut reachable = vec![false; nodes.len()];
    let mut pending = if nodes.is_empty() {
        Vec::new()
    } else {
        vec![0usize]
    };
    while let Some(i) = pending.pop() {
        reachable[i] = true;
        match nodes[i] {
            Node::Empty => {}
            Node::Leaf(leaf) => {
                if leaf.tabs.is_empty() {
                    return Err("タブの無い組がある".into());
                }
                if leaf.active.0 >= leaf.tabs.len() {
                    return Err(format!(
                        "前のタブの番号 {} がタブの数 {} の外",
                        leaf.active.0,
                        leaf.tabs.len()
                    ));
                }
                if !leaf.scroll.is_finite() {
                    return Err("タブの帯のスクロールが有限でない".into());
                }
            }
            Node::Vertical(split) | Node::Horizontal(split) => {
                if !(split.fraction.is_finite() && split.fraction > 0.0 && split.fraction < 1.0) {
                    return Err(format!("分け方 {} が 0 と 1 の間でない", split.fraction));
                }
                let (left, right) = (2 * i + 1, 2 * i + 2);
                if right >= nodes.len() {
                    return Err("分けた所の片側が木の外".into());
                }
                if matches!(nodes[left], Node::Empty) || matches!(nodes[right], Node::Empty) {
                    return Err("分けた所の片側が空".into());
                }
                pending.push(left);
                pending.push(right);
            }
        }
    }
    if nodes
        .iter()
        .zip(&reachable)
        .any(|(node, seen)| !seen && !matches!(node, Node::Empty))
    {
        return Err("根からつながらない節がある".into());
    }
    Ok(())
}

/// 浮かせた窓の、今の位置と大きさ（面の番号つき。egui が覚えている窓の矩形）。egui_dock 0.21 は窓の矩形を自分では更新しないので、
/// 保存のときにここから渡し、読み戻すときは egui_dock が「最初に描くときの位置と大きさ」として使う。
pub type FloatRect = (egui_dock::SurfaceIndex, egui::Rect);

/// 保存用の写し（各部品の矩形は、フレームごとに計算し直す値なので 0 にそろえる。窓の大きさを変えても中身が変わらず、無限大の値（まだ
/// 描いていない部品の矩形）を JSON に書かずに済む）。浮かせた窓は、位置と大きさを「最初に描くときの値」として入れる。
fn normalized(dock: &DockState<Tab>, floats: &[FloatRect]) -> DockState<Tab> {
    use egui::Rect;
    use egui_dock::Node;
    let mut copy = dock.clone();
    for (surface, rect) in floats {
        if rect.min.is_finite() && rect.max.is_finite() && rect.width() > 0.0 && rect.height() > 0.0
        {
            // （`get_window_state_mut` は範囲の外の番号で落ちるので、面を取ってから見る）
            if let Some(egui_dock::Surface::Window(_, state)) = copy.get_surface_mut(*surface) {
                state.set_position(rect.min).set_size(rect.size());
            }
        }
    }
    for (_, node) in copy.iter_all_nodes_mut() {
        match node {
            Node::Leaf(leaf) => {
                leaf.rect = Rect::ZERO;
                leaf.viewport = Rect::ZERO;
                leaf.scroll = 0.0;
            }
            Node::Vertical(split) | Node::Horizontal(split) => split.rect = Rect::ZERO,
            Node::Empty => {}
        }
    }
    copy
}

/// ファイルの中身を作る（浮かせた窓の位置と大きさは入れない形。`render_with` が入れる）。
pub fn render(dock: &DockState<Tab>, window: Option<&WindowRecord>) -> String {
    render_with(dock, window, &[])
}

/// ファイルの中身を作る。`floats` は浮かせた窓の今の位置と大きさ。
pub fn render_with(
    dock: &DockState<Tab>,
    window: Option<&WindowRecord>,
    floats: &[FloatRect],
) -> String {
    let dock = serde_json::to_value(normalized(dock, floats)).unwrap_or(Value::Null);
    let mut root = json!({ "format": FORMAT, "dock": dock });
    if let Some(window) = window {
        root["window"] = window.to_json();
    }
    serde_json::to_string_pretty(&root).unwrap_or_default()
}

/// 書く（一時ファイルへ書いて同期し、最後の 1 回の置き換えで確定する。途中で止まっても前のファイルは壊れない）。
pub fn save(path: &Path, text: &str) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "layout directory missing"))?;
    std::fs::create_dir_all(parent)?;
    yolu_io::atomic::replace_bytes(path, text.as_bytes())
}

/// 起動のときに窓へ戻す大きさと位置（設定のファイルから。無い・正しくないなら None）。置き場所は `windowpos::startup` が、画面ごとの
/// 拡大率・作業領域と突き合わせて決める（画面を列挙できない OS だけ、位置が今の画面に見えているかを `is_visible_on_a_monitor` で
/// 確かめてから、記録をそのまま使う）。
pub fn saved_window() -> Option<WindowRecord> {
    saved_window_at(&path()?)
}

/// `saved_window` の、ファイルの場所を渡せる形。
pub fn saved_window_at(path: &Path) -> Option<WindowRecord> {
    load(path).window
}

/// 窓の位置（外枠の左上・点）が、今つながっている画面のどれかに見えているか。外枠の上の帯（つかんで動かす所）の真ん中が画面の中に
/// あれば見えているとする。確かめられない OS（Windows 以外）は常に true。
pub fn is_visible_on_a_monitor(window: &WindowRecord) -> bool {
    platform::title_bar_on_a_monitor(window)
}

#[cfg(windows)]
mod platform {
    use super::WindowRecord;
    use windows::Win32::Foundation::RECT;
    use windows::Win32::Graphics::Gdi::{MonitorFromRect, MONITOR_DEFAULTTONULL};

    pub fn title_bar_on_a_monitor(window: &WindowRecord) -> bool {
        let scale = window.pixels_per_point;
        let center_x = (window.position[0] + window.size[0] / 2.0) * scale;
        let top = window.position[1] * scale;
        // 上の帯の真ん中 200 × 32 点
        let rect = RECT {
            left: (center_x - 100.0 * scale) as i32,
            right: (center_x + 100.0 * scale) as i32,
            top: top as i32,
            bottom: (top + 32.0 * scale) as i32,
        };
        // SAFETY: 初期化した RECT を渡す（Win32 の呼び方どおり）。
        let monitor = unsafe { MonitorFromRect(&rect, MONITOR_DEFAULTTONULL) };
        !monitor.is_invalid()
    }
}

#[cfg(not(windows))]
mod platform {
    use super::WindowRecord;

    pub fn title_bar_on_a_monitor(_: &WindowRecord) -> bool {
        true
    }
}
