//! ツールの登録の表。ツールごとの名前・アイコン・キー・ツールの帯の区切り・種類（範囲・押して終わる・描く・消す・大きさを持つ）・サブツールの一覧・
//! ツールプロパティの欄・オプションバーの項目・キャンバスの入力の受け口（`input::CanvasKind`）・3D ビューで面を押したときの行き先（`input::Surface`）・
//! ポインタの形（`input::Cursor`）を、1 つの表 `TOOLS` で持つ。`Tool` の `id`・`name_in`・`key`・`is_*` は、この表を引く薄い関数。
//! ツールを足すときは `Tool` の列挙に 1 行と、この表に 1 行（と、そのツールの欄・入力の関数）を足す。表の並びは `Tool::ALL`（ツールの帯の並び）と同じで、
//! 試験が `Tool` の番号と表の添字が合っていることを確かめる。キャンバスと 3D ビューの入力・オプションバー・プロパティの欄は、ツールの名前で
//! 振り分けず、この表を引く。
pub mod input;

use egui::{Rect, Ui};

use self::input::{CanvasKind, Cursor, Surface};
use crate::lang::Lang;
use crate::panels::{brushes, path_props, region_props};
use crate::state::{AppState, Tool};
use crate::ui::widgets::Rows;
use crate::{drafting, eyedrop, gradient, rulers, selection, textlayer, transform};

/// サブツールの一覧の種類（左のドックのサブツールの上の部分に何を出すか）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubTools {
    /// ブラシの一覧（グループのタブ。消しゴムのグループは出さない）。
    Brushes,
    /// 消しゴムの一覧（ブラシの一覧の消しゴムのグループだけ）。
    Erasers,
    /// このツールの設定の組のプリセット（`subtool`）。
    Presets,
    /// 同じ並びのツール（押すとツールが替わる。選択のツール）。
    Tools(&'static [Tool]),
    /// 1 つだけ。
    Single,
}

/// 選択のツールの並び（ツールの帯の選択の組と同じ）。
pub const SELECTION_TOOLS: [Tool; 7] = [
    Tool::SelectRect,
    Tool::SelectEllipse,
    Tool::Lasso,
    Tool::Polygon,
    Tool::Wand,
    Tool::IdSelect,
    Tool::SelectPen,
];

/// ツールプロパティの欄の中身を出す関数（欄の行は `Rows` の compact）。
pub type PropsFn = fn(&mut Ui, &mut AppState, &mut Rows, &egui::Context);
/// オプションバーの中身を出す関数（`x` はツールのアイコンと区切りの右の左端。使える右端は `r.right()`）。
pub type OptionsFn = fn(&mut Ui, &mut AppState, Rect, f32);

pub struct ToolDef {
    pub tool: Tool,
    /// アイコン（`tools/<id>`）と、保存するファイルの名前。
    pub id: &'static str,
    pub ja: &'static str,
    pub en: &'static str,
    pub key: &'static str,
    /// ツールの帯で、このツールの前に区切りを入れる。
    pub starts_group: bool,
    /// 範囲を塗る・選ぶツール（バケツ・ポリゴン塗りつぶし・ID の色で選択。入力は `region`）。
    pub region: bool,
    /// 押した瞬間に終わるツール（ストロークもドラッグも持たない）。
    pub one_shot: bool,
    /// 画素にブラシで描くツール（3D ビューの入力が描き始めてよい）。
    pub paints: bool,
    /// 消すツール（描くツールのうち、ペンの消しゴムの端と同じく消す側で描く）。
    pub erases: bool,
    /// 選択のツール（選択範囲を作る・変える。ID の色で選択は範囲のツールなので含めない）。
    pub select: bool,
    pub path: bool,
    /// 大きさ（直径）を持つツール。「ブラシサイズ」のパネルに丸を出す。
    pub sized: bool,
    /// 「塗るチャンネル」（複数のチャンネルを一度に塗る設定。`AppState::mat`）が効くツール。ツールプロパティの終わりに、その区分を出す（表示だけの印）。選択のツールは、
    /// 選択範囲の塗りつぶし・消去が入れてある組を使うので、これを持つ（ツールの動きは変わらない）。
    pub paint_channels: bool,
    pub subtools: SubTools,
    /// キャンバスの入力の受け口（ドラッグの札を持つツール）。ブラシ・消しゴム・範囲のツール・スポイトはストロークで描くので持たない。
    pub canvas: Option<CanvasKind>,
    /// 3D ビューで面を押したときの行き先。
    pub surface: Surface,
    /// キャンバスの上のポインタの形。
    pub cursor: Cursor,
    /// ツールプロパティ（左のドックのツールプロパティのパネルの欄）。
    pub properties: PropsFn,
    /// オプションバー（そのツールでよく使う 2〜3 個。ツールプロパティと同じ値を見せる）。
    pub options: OptionsFn,
}

fn no_props(_: &mut Ui, _: &mut AppState, _: &mut Rows, _: &egui::Context) {}
fn no_options(_: &mut Ui, _: &mut AppState, _: Rect, _: f32) {}

const fn def(
    tool: Tool,
    id: &'static str,
    ja: &'static str,
    en: &'static str,
    key: &'static str,
) -> ToolDef {
    ToolDef {
        tool,
        id,
        ja,
        en,
        key,
        starts_group: false,
        region: false,
        one_shot: false,
        paints: false,
        erases: false,
        select: false,
        path: false,
        sized: false,
        paint_channels: false,
        subtools: SubTools::Single,
        canvas: None,
        surface: Surface::Unsupported,
        cursor: Cursor::Crosshair,
        properties: no_props,
        options: no_options,
    }
}

impl ToolDef {
    const fn group(mut self) -> Self {
        self.starts_group = true;
        self
    }
    const fn region(mut self) -> Self {
        self.region = true;
        self
    }
    const fn one_shot(mut self) -> Self {
        self.one_shot = true;
        self
    }
    const fn paints(mut self) -> Self {
        self.paints = true;
        self.surface = Surface::Paint;
        self.cursor = Cursor::Brush;
        self
    }
    const fn erases(mut self) -> Self {
        self.erases = true;
        self
    }
    const fn canvas(mut self, kind: CanvasKind) -> Self {
        self.canvas = Some(kind);
        self
    }
    const fn surface(mut self, surface: Surface) -> Self {
        self.surface = surface;
        self
    }
    const fn cursor(mut self, cursor: Cursor) -> Self {
        self.cursor = cursor;
        self
    }
    const fn select(mut self) -> Self {
        self.select = true;
        self
    }
    const fn path(mut self) -> Self {
        self.path = true;
        self
    }
    const fn sized(mut self) -> Self {
        self.sized = true;
        self
    }
    const fn paint_channels(mut self) -> Self {
        self.paint_channels = true;
        self
    }
    const fn sub(mut self, subtools: SubTools) -> Self {
        self.subtools = subtools;
        self
    }
    const fn ui(mut self, properties: PropsFn, options: OptionsFn) -> Self {
        self.properties = properties;
        self.options = options;
        self
    }

    pub fn name(&self, lang: Lang) -> &'static str {
        lang.pick(self.ja, self.en)
    }
}

/// 全部のツール（`Tool::ALL` の並び）。
pub static TOOLS: [ToolDef; 19] = [
    def(Tool::Brush, "brush", "ブラシ", "Brush", "B")
        .paints()
        .sized()
        .paint_channels()
        .sub(SubTools::Brushes)
        .ui(brushes::props, brushes::options),
    def(Tool::Eraser, "eraser", "消しゴム", "Eraser", "E")
        .paints()
        .erases()
        .sized()
        .paint_channels()
        .sub(SubTools::Erasers)
        .ui(brushes::props, brushes::options),
    def(Tool::Fill, "fill", "バケツ", "Fill", "G")
        .region()
        .one_shot()
        .paint_channels()
        .surface(Surface::Region)
        .sub(SubTools::Presets)
        .ui(region_props::fill_props, region_props::options),
    def(
        Tool::Gradient,
        "gradient",
        "グラデーション",
        "Gradient",
        "Shift+G",
    )
    .paint_channels()
    .canvas(CanvasKind::Gradient)
    .surface(Surface::Screen)
    .sub(SubTools::Presets)
    .ui(gradient::props::body, gradient::props::options),
    def(Tool::Shape, "shape", "図形", "Shape", "U")
        .paint_channels()
        .canvas(CanvasKind::Drafting)
        .surface(Surface::Screen)
        .sub(SubTools::Presets)
        .ui(drafting::props::shape_props, drafting::props::options),
    def(Tool::Ruler, "ruler", "定規", "Ruler", "Shift+U")
        .canvas(CanvasKind::Drafting)
        .surface(Surface::Screen)
        .sub(SubTools::Presets)
        .ui(rulers::tool::props, rulers::tool::options),
    def(
        Tool::PolygonFill,
        "polygon-fill",
        "ポリゴン塗りつぶし",
        "Polygon Fill",
        "4",
    )
    .region()
    .paint_channels()
    .surface(Surface::Region)
    .sub(SubTools::Presets)
    .ui(region_props::polygon_props, region_props::options),
    def(
        Tool::Eyedropper,
        "eyedropper",
        "スポイト",
        "Eyedropper",
        "I",
    )
    .one_shot()
    .paint_channels()
    .surface(Surface::Pick)
    .sub(SubTools::Presets)
    .ui(eyedrop::props, eyedrop::options),
    def(
        Tool::SelectRect,
        "select-rectangle",
        "長方形選択",
        "Rectangle Select",
        "M",
    )
    .group()
    .select()
    .paint_channels()
    .canvas(CanvasKind::Selection)
    .surface(Surface::Screen)
    .sub(SubTools::Tools(&SELECTION_TOOLS))
    .ui(selection::props::body, selection::props::select_options),
    def(
        Tool::SelectEllipse,
        "select-ellipse",
        "楕円形選択",
        "Ellipse Select",
        "Shift+M",
    )
    .select()
    .paint_channels()
    .canvas(CanvasKind::Selection)
    .surface(Surface::Screen)
    .sub(SubTools::Tools(&SELECTION_TOOLS))
    .ui(selection::props::body, selection::props::select_options),
    def(Tool::Lasso, "lasso", "なげなわ", "Lasso", "L")
        .select()
        .paint_channels()
        .canvas(CanvasKind::Selection)
        .surface(Surface::Screen)
        .sub(SubTools::Tools(&SELECTION_TOOLS))
        .ui(selection::props::body, selection::props::select_options),
    def(
        Tool::Polygon,
        "select-polygon",
        "多角形選択",
        "Polygon Select",
        "Shift+L",
    )
    .select()
    .paint_channels()
    .canvas(CanvasKind::Selection)
    .surface(Surface::Screen)
    .sub(SubTools::Tools(&SELECTION_TOOLS))
    .ui(selection::props::body, selection::props::select_options),
    def(Tool::Wand, "magic-wand", "自動選択", "Magic Wand", "W")
        .select()
        .paint_channels()
        .canvas(CanvasKind::Selection)
        .surface(Surface::Screen)
        .sub(SubTools::Tools(&SELECTION_TOOLS))
        .ui(selection::props::body, selection::props::select_options),
    def(
        Tool::IdSelect,
        "id-select",
        "ID の色で選択",
        "ID Color Select",
        "Shift+W",
    )
    .region()
    .one_shot()
    .surface(Surface::Region)
    .sub(SubTools::Tools(&SELECTION_TOOLS))
    .ui(region_props::id_props, region_props::options),
    def(
        Tool::SelectPen,
        "select-pen",
        "選択ペン",
        "Selection Pen",
        "S",
    )
    .select()
    .paint_channels()
    .sized()
    .canvas(CanvasKind::Selection)
    .surface(Surface::Cover)
    .sub(SubTools::Tools(&SELECTION_TOOLS))
    .ui(selection::props::body, selection::props::select_options),
    def(Tool::Move, "move", "移動・変形", "Move / Transform", "V")
        .group()
        .canvas(CanvasKind::Transform)
        .cursor(Cursor::Transform)
        .sub(SubTools::Presets)
        .ui(transform::props::body, transform::props::options),
    def(Tool::Liquify, "liquify", "ゆがみ", "Liquify", "")
        .canvas(CanvasKind::Transform)
        .cursor(Cursor::Transform)
        .sub(SubTools::Presets)
        .ui(
            transform::props::liquify_body,
            transform::props::liquify_options,
        ),
    def(Tool::Path, "path", "パス", "Path", "P")
        .path()
        .paint_channels()
        .canvas(CanvasKind::Path)
        .surface(Surface::Path)
        .cursor(Cursor::Path)
        .sub(SubTools::Single)
        .ui(path_props::body, path_props::options),
    def(Tool::Text, "text", "テキスト", "Text", "T")
        .canvas(CanvasKind::Text)
        .cursor(Cursor::Text)
        .sub(SubTools::Single)
        .ui(textlayer::props::props, textlayer::props::options),
];

impl Tool {
    /// このツールの表の行。
    pub fn def(self) -> &'static ToolDef {
        &TOOLS[self as usize]
    }

    /// 選択のツールか（ID の色で選択は含めない）。
    pub fn is_select(self) -> bool {
        self.def().select
    }

    /// 画素にブラシで描くツールか（3D ビューの入力が描き始めてよいか）。
    pub fn paints(self) -> bool {
        self.def().paints
    }

    /// パスのツールか。
    pub fn is_path(self) -> bool {
        self.def().path
    }

    /// 大きさ（直径）を持つツールか（ブラシサイズの節を出す）。
    pub fn is_sized(self) -> bool {
        self.def().sized
    }

    /// 消すツールか（消しゴム）。
    pub fn erases(self) -> bool {
        self.def().erases
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_in_the_order_of_the_strip_and_indexed_by_the_tool() {
        for (i, def) in TOOLS.iter().enumerate() {
            assert_eq!(def.tool as usize, i, "{}", def.id);
            assert_eq!(Tool::ALL[i], def.tool, "{}", def.id);
        }
        assert_eq!(TOOLS.len(), Tool::ALL.len());
    }

    #[test]
    fn ids_are_unique_and_every_tool_has_both_names() {
        let mut ids: Vec<&str> = TOOLS.iter().map(|d| d.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), TOOLS.len());
        for def in &TOOLS {
            assert!(!def.ja.is_empty() && !def.en.is_empty(), "{}", def.id);
            assert!(
                !def.en
                    .chars()
                    .any(|c| ('\u{3000}'..='\u{9fff}').contains(&c)),
                "{}",
                def.id
            );
        }
    }

    #[test]
    fn selection_tools_list_each_other_in_the_strip_order() {
        let strip: Vec<Tool> = Tool::ALL
            .iter()
            .copied()
            .filter(|t| matches!(t.def().subtools, SubTools::Tools(_)))
            .collect();
        assert_eq!(strip, SELECTION_TOOLS);
        for tool in strip {
            assert_eq!(tool.def().subtools, SubTools::Tools(&SELECTION_TOOLS));
        }
    }
}
