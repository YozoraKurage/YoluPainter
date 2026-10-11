//! 画面の並びの保存: ドックの並び（タブの組・分け方・大きさ・どのタブが前か・浮かせたウィンドウ）と、ウィンドウの大きさ・位置・最大化を、設定のフォルダの
//! `layout.json` へ書き、次の起動で戻す。ドックは egui_dock が持つ serde の形（`DockState` をそのまま）、タブは安定した名前（`Tab::key`）で
//! 書く。書くのは、並びが変わったとき（1 秒おき・ドラッグの最中でない）と終わるとき。書き方は一時ファイルから置き換える 1 回の操作。
//! 外へ出したウィンドウ（`detach`）は `detached` に、ウィンドウごとの中のドック・外枠の位置と内側の大きさ・戻る先のタブを書く（無ければ書かない。別ウィンドウの
//! 無いファイルは前の版と同じ中身）。前の版の、アプリの中の浮いたウィンドウ（egui_dock のウィンドウの面）は、読んだあとアプリが別ウィンドウへ替える。
//!
//! 読めない・古い版・知らないタブ・タブが足りない／重なる・大きすぎる・egui_dock が添字で引いて落ちる値（前のタブの番号・木の子・空の組・
//! 分け方・ウィンドウの位置）のどれでも、そのファイルのドックは捨てて既定の並び（`app::default_dock`）で始める（理由は診断のログだけで、画面には
//! 出さない）。ウィンドウの大きさ・位置は、ドックとは別に確かめる（ドックを捨ててもウィンドウは戻す）。

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
/// ウィンドウの最小の内側の大きさ（点。`main` の最小の大きさと同じ）。
pub const MIN_SIZE: [f32; 2] = [960.0, 640.0];
/// ウィンドウの大きさの上限（点。これより大きい値は壊れた値）。
const MAX_SIZE: f32 = 16384.0;
/// ウィンドウの位置の範囲（点。これより外は壊れた値）。
const MAX_POSITION: f32 = 32000.0;

/// 設定のフォルダの `layout.json`（設定のフォルダが分からなければ None）。
pub fn path() -> Option<PathBuf> {
    crate::settings::path().and_then(|p| path_for(&p))
}

/// 設定のファイル（`settings.conf`）と同じフォルダの `layout.json`。
pub fn path_for(settings: &Path) -> Option<PathBuf> {
    Some(settings.parent()?.join(FILE_NAME))
}

/// ウィンドウの大きさと位置（最大化していない状態のもの）と、最大化していたか。
///
/// 位置と大きさは点で、点 = 画素 / `pixels_per_point`（書いたときにウィンドウがいた画面の拡大率。アプリは egui の拡大を使わないので OS の論理の点と
/// 同じ）。画素 = 点 × `pixels_per_point` は仮想スクリーンの物理画素で、拡大率の違う画面をまたぐときは、点の座標を別の画面の拡大率で
/// 読み替えない（`windowpos::plan` が画素に直してから、今の画面と突き合わせる）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowRecord {
    /// 外枠の左上（点）。
    pub position: [f32; 2],
    /// 内側の大きさ（点）。
    pub size: [f32; 2],
    /// 書いたときの 1 点あたりの画素（ウィンドウがいた画面の拡大率）。
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
    /// 読めたウィンドウの大きさと位置。
    pub window: Option<WindowRecord>,
    /// 外へ出したウィンドウ（ドックの並びが使えるときだけ）。
    pub detached: Vec<DetachedRecord>,
    /// 書いたときの並びが、利用者が仕切りもタブも動かしていない既定のままだったか（ファイルの `auto_fit`。無ければ false。ドックの並びが使えるときだけ true になりうる）。
    pub auto_fit: bool,
    /// 捨てた理由（診断のログに書く文。画面には出さない）。
    pub problems: Vec<String>,
}

/// 外へ出したウィンドウの位置と大きさ。外枠の左上と内側の大きさは点で、点 = 画素 / `pixels_per_point`（書いたときにウィンドウがいた画面の拡大率。
/// メインウィンドウの `WindowRecord` と同じ決め方）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FloatRecord {
    /// 外枠の左上（点）。
    pub position: [f32; 2],
    /// 内側の大きさ（点）。
    pub size: [f32; 2],
    /// 書いたときの 1 点あたりの画素。
    pub pixels_per_point: f32,
}

impl FloatRecord {
    /// 値が正しいか（有限・範囲の中）。大きさは別ウィンドウの最小の大きさまで引き上げる。正しくなければ None。
    pub fn sanitized(self) -> Option<FloatRecord> {
        let finite = self
            .position
            .iter()
            .chain(&self.size)
            .chain([&self.pixels_per_point])
            .all(|v| v.is_finite());
        let range = self.position.iter().all(|v| v.abs() <= MAX_POSITION)
            && self.size.iter().all(|v| (1.0..=MAX_SIZE).contains(v))
            && (0.25..=8.0).contains(&self.pixels_per_point);
        let min = crate::detach::MIN_SIZE;
        (finite && range).then_some(FloatRecord {
            size: [self.size[0].max(min[0]), self.size[1].max(min[1])],
            ..self
        })
    }

    fn to_json(self) -> Value {
        json!({
            "x": self.position[0], "y": self.position[1],
            "width": self.size[0], "height": self.size[1],
            "pixels_per_point": self.pixels_per_point,
        })
    }

    fn from_json(value: &Value) -> Option<FloatRecord> {
        let number = |key: &str| value.get(key)?.as_f64().map(|v| v as f32);
        FloatRecord {
            position: [number("x")?, number("y")?],
            size: [number("width")?, number("height")?],
            pixels_per_point: number("pixels_per_point")?,
        }
        .sanitized()
    }
}

/// 外へ出したウィンドウ 1 つの記録。
#[derive(Clone, Debug)]
pub struct DetachedRecord {
    /// 中のドック（主の面だけ）。
    pub dock: DockState<Tab>,
    /// 外枠の位置と内側の大きさ（無い・正しくなければ None: メインウィンドウの上の既定の場所に開く）。
    pub window: Option<FloatRecord>,
    /// 戻る先（ウィンドウを閉じたとき、中のタブを入れる組のタブ）。
    pub home: Vec<Tab>,
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
    // ウィンドウの大きさと位置は、ドックとは別に確かめる（ドックを捨ててもウィンドウは戻す）
    match root.get("window") {
        None | Some(Value::Null) => {}
        Some(value) => match WindowRecord::from_json(value) {
            Some(window) => out.window = Some(window),
            None => out.problems.push(
                "ウィンドウの大きさと位置の値が正しくありません。ウィンドウは既定の大きさで始めます。".into(),
            ),
        },
    }
    let Some(dock) = root.get("dock") else {
        out.problems
            .push("画面の並びのファイルにドックの並びがありません。既定の並びで始めます。".into());
        return out;
    };
    let mut dock = match serde_json::from_value::<DockState<Tab>>(dock.clone()) {
        Ok(dock) => dock,
        Err(e) => {
            out.problems.push(format!(
                "ドックの並びを読めません（{e}）。既定の並びで始めます。"
            ));
            return out;
        }
    };
    forget_focus(&mut dock);
    // 外へ出したウィンドウ（無ければ空。中のドックが読めない・使えないなら、主のドックと一緒に捨てる）
    let mut detached = match parse_detached(root.get("detached"), &mut out.problems) {
        Ok(detached) => detached,
        Err(reason) => {
            out.problems.push(format!(
                "別ウィンドウの並びを使えません（{reason}）。既定の並びで始めます。"
            ));
            return out;
        }
    };
    let docks: Vec<&DockState<Tab>> = detached.iter().map(|d| &d.dock).collect();
    match validate_all(&dock, &docks) {
        Ok(()) => {
            add_missing_tabs(&mut dock, &mut detached);
            // （足したときに egui_dock が付けた、フォーカスしている組は外す）
            forget_focus(&mut dock);
            for d in &mut detached {
                forget_focus(&mut d.dock);
            }
            out.dock = Some(dock);
            out.detached = detached;
            out.auto_fit = root
                .get("auto_fit")
                .and_then(Value::as_bool)
                .unwrap_or(false);
        }
        Err(reason) => out.problems.push(format!(
            "ドックの並びを使えません（{reason}）。既定の並びで始めます。"
        )),
    }
    out
}

/// `detached` の値（無ければ空）。ウィンドウの位置の値だけが壊れていれば、そのウィンドウは位置なし（理由を `problems` へ）。中のドックが読めなければ Err。
fn parse_detached(
    value: Option<&Value>,
    problems: &mut Vec<String>,
) -> Result<Vec<DetachedRecord>, String> {
    let items = match value {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::Array(items)) => items,
        Some(_) => return Err("配列でない".into()),
    };
    let mut out = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let dock = item
            .get("dock")
            .ok_or_else(|| format!("別ウィンドウ {i} にドックが無い"))?;
        let dock = serde_json::from_value::<DockState<Tab>>(dock.clone())
            .map_err(|e| format!("別ウィンドウ {i} のドックを読めない（{e}）"))?;
        let window = match item.get("window") {
            None | Some(Value::Null) => None,
            Some(v) => {
                let record = FloatRecord::from_json(v);
                if record.is_none() {
                    problems.push(format!(
                        "別ウィンドウ {i} の位置と大きさの値が正しくありません。メインウィンドウの上に開きます。"
                    ));
                }
                record
            }
        };
        let home = item
            .get("home")
            .and_then(Value::as_array)
            .map(|keys| {
                keys.iter()
                    .filter_map(Value::as_str)
                    .filter_map(Tab::from_key)
                    .collect()
            })
            .unwrap_or_default();
        out.push(DetachedRecord { dock, window, home });
    }
    Ok(out)
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

/// 保存した並びに無くてよいタブ。ポーズはスキンのあるモデルを読むと足される。ログ・ツールプロパティ・ブラシサイズ・マテリアルは後の版で足したタブで、
/// それより前の版が保存した並びには無い（無いだけで並び全部を捨てないよう、読んだときに `add_missing_tabs` が足す）。アクションとナビゲーターは既定の並びに無く、
/// 開いたときだけある（ナビゲーターは前の版の並びにはあり、その並びでは今までの組にそのまま残る。足さない）。
pub const OPTIONAL_TABS: [Tab; 7] = [
    Tab::Pose,
    Tab::Log,
    Tab::Actions,
    Tab::Navigator,
    Tab::ToolProperties,
    Tab::BrushSize,
    Tab::Material,
];

/// 「ウィンドウ」のメニューで開いたり閉じたりできるタブ。`OPTIONAL_TABS` のうち既定の並びに無いもの（ほかのパネルは、押すと前に出すだけ）。
pub const HIDEABLE: [Tab; 2] = [Tab::Navigator, Tab::Actions];

/// 前の版の並びに足すときの値: 「サブツール」の組の下にツールプロパティを足すとき、サブツールの組が持つ高さの取り分と、その下にブラシサイズを足すとき、
/// ツールプロパティの組が持つ取り分。今の既定の並び（`app::default_dock_for`）の同じ取り分（0.294・0.729）とは別の値で、既定の並びを作り直すものではない。
const TOOL_PROPERTIES_SHARE: f32 = 0.38;
const BRUSH_SIZE_SPLIT: f32 = 0.8;

/// 後の版で足したタブが、読んだ並びにどこにも（メインのドックにも別ウィンドウにも）無ければ足す。
/// - ログ: 既定の並びと同じく、レイヤーと同じ組のレイヤーのすぐ後ろ（レイヤーが別ウィンドウにあればそのウィンドウの同じ組の後ろ、どこにも無ければメインの最初の組。前へは出さない）。
/// - ツールプロパティ・ブラシサイズ: 前は「サブツール」の中に入っていた 3 つを、既定の並びと同じく、サブツールの組の下に縦に並べる
///   （サブツールの組を分けて、下にツールプロパティ、その下にブラシサイズ）。サブツールが別ウィンドウにあるなら、そのウィンドウの組の後ろへ。
///   サブツールが無ければ最初の組。
/// - マテリアル: 既定の並びと同じく、プロパティと同じ組のプロパティのすぐ後ろ（プロパティが別ウィンドウにあればそのウィンドウの同じ組の後ろ、どこにも無ければメインの最初の組。前へは出さない）。
///
/// 足した場所は、並びの形に応じて変わるだけで、ほかのタブの並び・大きさ・前のタブは変えない。
pub fn add_missing_tabs(dock: &mut DockState<Tab>, detached: &mut [DetachedRecord]) {
    let present = |dock: &DockState<Tab>, detached: &[DetachedRecord], tab: Tab| {
        dock.find_tab(&tab).is_some() || detached.iter().any(|d| d.dock.find_tab(&tab).is_some())
    };
    if !present(dock, detached, Tab::Log) {
        push_beside_anywhere(dock, detached, Tab::Layers, Tab::Log);
    }
    if !present(dock, detached, Tab::ToolProperties) {
        if dock.find_tab(&Tab::SubTools).is_some() {
            split_below(
                dock,
                Tab::SubTools,
                Tab::ToolProperties,
                TOOL_PROPERTIES_SHARE,
            );
        } else if let Some(window) = detached
            .iter_mut()
            .find(|d| d.dock.find_tab(&Tab::SubTools).is_some())
        {
            push_beside(&mut window.dock, Tab::SubTools, Tab::ToolProperties);
        } else {
            dock.push_to_first_leaf(Tab::ToolProperties);
        }
    }
    if !present(dock, detached, Tab::BrushSize) {
        if dock.find_tab(&Tab::ToolProperties).is_some() {
            split_below(dock, Tab::ToolProperties, Tab::BrushSize, BRUSH_SIZE_SPLIT);
        } else if let Some(window) = detached
            .iter_mut()
            .find(|d| d.dock.find_tab(&Tab::ToolProperties).is_some())
        {
            push_beside(&mut window.dock, Tab::ToolProperties, Tab::BrushSize);
        } else {
            dock.push_to_first_leaf(Tab::BrushSize);
        }
    }
    if !present(dock, detached, Tab::Material) {
        push_beside_anywhere(dock, detached, Tab::Properties, Tab::Material);
    }
}

/// `push_beside` の、`anchor` が別ウィンドウにあるときは、そのウィンドウの `anchor` と同じ組の後ろへ入れる形（どこにも無ければメインの最初の組）。
fn push_beside_anywhere(
    dock: &mut DockState<Tab>,
    detached: &mut [DetachedRecord],
    anchor: Tab,
    tab: Tab,
) {
    if dock.find_tab(&anchor).is_none() {
        if let Some(window) = detached
            .iter_mut()
            .find(|d| d.dock.find_tab(&anchor).is_some())
        {
            push_beside(&mut window.dock, anchor, tab);
            return;
        }
    }
    push_beside(dock, anchor, tab);
}

/// タブを、`anchor` と同じ組の、`anchor` のすぐ後ろへ入れる（前のタブは変えない。`anchor` が無ければ最初の組）。
fn push_beside(dock: &mut DockState<Tab>, anchor: Tab, tab: Tab) {
    let at = dock.find_tab(&anchor);
    let leaf = at.and_then(|p| {
        dock.leaf_mut(p.node_path())
            .ok()
            .map(|leaf| (leaf, p.tab.0))
    });
    match leaf {
        Some((leaf, index)) => {
            leaf.tabs.insert(index + 1, tab);
            // 前のタブが、入れた所より後ろなら、1 つずらして同じタブのままにする
            if leaf.active.0 > index {
                leaf.active.0 += 1;
            }
        }
        None => dock.push_to_first_leaf(tab),
    }
}

/// タブを、`anchor` の組の下に新しい組として足す（`anchor` の組は、分ける前の高さの `keep` の取り分を持つ）。
fn split_below(dock: &mut DockState<Tab>, anchor: Tab, tab: Tab, keep: f32) {
    match dock.find_tab(&anchor) {
        Some(path) => {
            dock.split(
                path.node_path(),
                egui_dock::Split::Below,
                keep,
                egui_dock::Node::leaf_with(vec![tab]),
            );
        }
        None => dock.push_to_first_leaf(tab),
    }
}

/// 読んだドックが使えるか。egui_dock は読んだ値をそのまま添字で引くので、描く前に次を確かめる（外れていれば理由。外れたまま渡すと、
/// 起動のたびに落ちて、並びのファイルを手で消すまで起動できなくなる）。
/// - 面: 先頭が主の面で、それ以外に主の面が無い。浮かせたウィンドウの面は、組を 1 つ以上持ち、ウィンドウの位置と大きさが有限で範囲の中。
/// - 木: 分けた所の両側が木の中にあり、根からつながらない節が無い（見えないタブができる）。分け方は有限で 0 と 1 の間。
/// - 組: 空でなく、前のタブの番号がタブの数の中。浮かせたウィンドウの木のフォーカスは、木の中の組（読むときは `forget_focus` で外してある）。
/// - タブ: どのタブも 1 つずつ（ポーズ・アクションと、後の版で足したタブ（`OPTIONAL_TABS`）は、無くてよい。ポーズはモデルを読むと足され、
///   後の版で足したタブは読んだときに `add_missing_tabs` が足し、アクションは開いたときだけある）。足りない・重なるなら理由。
pub fn validate(dock: &DockState<Tab>) -> Result<(), String> {
    validate_all(dock, &[])
}

/// 主のドックと外へ出したウィンドウのドックを合わせて確かめる（`validate` の項目に加えて: 別ウィンドウのドックは主の面だけで、タブが 1 つ以上ある。
/// タブの重なり・不足は、主のドックと別ウィンドウを合わせて数える）。
pub fn validate_all(dock: &DockState<Tab>, detached: &[&DockState<Tab>]) -> Result<(), String> {
    validate_structure(dock)?;
    for (i, inner) in detached.iter().enumerate() {
        validate_structure(inner).map_err(|e| format!("別ウィンドウ {i}: {e}"))?;
        if inner
            .iter_surfaces()
            .any(|s| matches!(s, egui_dock::Surface::Window(..)))
        {
            return Err(format!("別ウィンドウ {i} に浮いたウィンドウがある"));
        }
        if inner.main_surface().num_tabs() == 0 {
            return Err(format!("別ウィンドウ {i} にタブが無い"));
        }
    }
    let mut count = std::collections::HashMap::new();
    for d in std::iter::once(dock).chain(detached.iter().copied()) {
        for (_, tab) in d.iter_all_tabs() {
            *count.entry(*tab).or_insert(0usize) += 1;
        }
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

/// ドック 1 つの形（面・木・組・浮いたウィンドウの値）。
fn validate_structure(dock: &DockState<Tab>) -> Result<(), String> {
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
                // 浮かせたウィンドウの木のフォーカスは、描くときに組として引く（メインの木は、組を全部浮かせたあとに、もう無い組を指したままでよい）
                if let Some(focus) = tree.focused_leaf() {
                    if !tree.iter().nth(focus.0).is_some_and(|node| node.is_leaf()) {
                        return Err(format!(
                            "浮かせたウィンドウ {} のフォーカスが組を指していない",
                            index.0
                        ));
                    }
                }
                if !tree.iter().any(|node| node.is_leaf()) {
                    return Err(format!("浮かせたウィンドウ {} に組が無い", index.0));
                }
                // ウィンドウの状態の値（最初に描くときの位置と大きさ）。中身は読み口が無いので、書き出した形で確かめる
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
                        return Err(format!(
                            "浮かせたウィンドウ {} の {key} が範囲の外",
                            index.0
                        ));
                    }
                }
            }
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

/// 浮かせたウィンドウの、今の位置と大きさ（面の番号つき。egui が覚えているウィンドウの矩形）。egui_dock 0.21 はウィンドウの矩形を自分では更新しないので、
/// 保存のときにここから渡し、読み戻すときは egui_dock が「最初に描くときの位置と大きさ」として使う。
pub type FloatRect = (egui_dock::SurfaceIndex, egui::Rect);

/// 保存用の写し（各部品の矩形は、フレームごとに計算し直す値なので 0 にそろえる。ウィンドウの大きさを変えても中身が変わらず、無限大の値（まだ
/// 描いていない部品の矩形）を JSON に書かずに済む）。浮かせたウィンドウは、位置と大きさを「最初に描くときの値」として入れる。
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

/// ファイルの中身を作る（浮かせたウィンドウの位置と大きさは入れない形。`render_with` が入れる）。
pub fn render(dock: &DockState<Tab>, window: Option<&WindowRecord>) -> String {
    render_with(dock, window, &[])
}

/// ファイルの中身を作る。`floats` は浮かせたウィンドウの今の位置と大きさ。
pub fn render_with(
    dock: &DockState<Tab>,
    window: Option<&WindowRecord>,
    floats: &[FloatRect],
) -> String {
    render_all(dock, window, floats, &[])
}

/// ファイルの中身を作る（外へ出したウィンドウつき。別ウィンドウが無ければ `detached` は書かない）。
pub fn render_all(
    dock: &DockState<Tab>,
    window: Option<&WindowRecord>,
    floats: &[FloatRect],
    detached: &[DetachedRecord],
) -> String {
    render_full(dock, window, floats, detached, false)
}

/// ファイルの中身を作る。`auto_fit` は、並びが利用者の動かしていない既定のまま（ウィンドウの幅に合わせて右の列の割合を直している）か。true のときだけ
/// `"auto_fit": true` を書く（前の版のファイルと同じく、無ければ false）。
pub fn render_full(
    dock: &DockState<Tab>,
    window: Option<&WindowRecord>,
    floats: &[FloatRect],
    detached: &[DetachedRecord],
    auto_fit: bool,
) -> String {
    let dock = serde_json::to_value(normalized(dock, floats)).unwrap_or(Value::Null);
    let mut root = json!({ "format": FORMAT, "dock": dock });
    if auto_fit {
        root["auto_fit"] = Value::Bool(true);
    }
    if let Some(window) = window {
        root["window"] = window.to_json();
    }
    if !detached.is_empty() {
        root["detached"] = Value::Array(
            detached
                .iter()
                .map(|d| {
                    let mut item = json!({
                        "dock": serde_json::to_value(normalized(&d.dock, &[])).unwrap_or(Value::Null),
                        "home": d.home.iter().map(|t| t.key()).collect::<Vec<_>>(),
                    });
                    if let Some(w) = d.window.and_then(FloatRecord::sanitized) {
                        item["window"] = w.to_json();
                    }
                    item
                })
                .collect(),
        );
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

/// 起動のときにウィンドウへ戻す大きさと位置（設定のファイルから。無い・正しくないなら None）。置き場所は `windowpos::startup` が、画面ごとの
/// 拡大率・作業領域と突き合わせて決める（画面を列挙できない OS だけ、位置が今の画面に見えているかを `is_visible_on_a_monitor` で
/// 確かめてから、記録をそのまま使う）。
pub fn saved_window() -> Option<WindowRecord> {
    saved_window_at(&path()?)
}

/// `saved_window` の、ファイルの場所を渡せる形。
pub fn saved_window_at(path: &Path) -> Option<WindowRecord> {
    load(path).window
}

/// ウィンドウの位置（外枠の左上・点）が、今つながっている画面のどれかに見えているか。外枠の上の帯（つかんで動かす所）の真ん中が画面の中に
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
