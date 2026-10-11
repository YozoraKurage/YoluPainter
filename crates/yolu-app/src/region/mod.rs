//! 範囲のツール（バケツ・ポリゴン塗りつぶし・ID の色で選択）を画面につなぐ。範囲は、2D キャンバスなら UV の点の下の三角形、3D ビューなら
//! 当たった面の三角形から、三角形・メッシュの塊・UV アイランド・マテリアルのどれかに辿る（`index`。core の `region` と同じ範囲）。
//! バケツは範囲を 1 回で塗り（今の選択範囲の内側だけ。マテリアルがオンなら組の全部のチャンネルを 1 回の Undo で）、ポリゴン塗りつぶしは
//! 押したまま通った範囲を足していき、離して 1 回の Undo にする（`tools`）。ID の色で選択は焼いた ID マップの色から選択範囲を作る
//! （`idcolor`。手動の ID の色の編集もここ）。ポインタの下の範囲は 3D では薄い面、2D では UV の輪郭で強調する（`overlay`）。
//!
//! 入力は `canvas`・`view3d::input` が、始める・動く・終える・Esc・フォーカスを失うの同じ道（ストロークと同じ）から呼ぶ。ドラッグは
//! `AppState::region.drag` の札で持ち、離す・Esc（捨てる）・ウィンドウのフォーカスを失う（そこまでを確定）・離したのを取りこぼす で必ず終える。

pub mod bucket;
pub mod color;
pub mod idcolor;
pub mod index;
pub mod overlay;
pub mod tools;

use std::sync::Arc;

use yolu_core::geometry::SurfaceRegionKind;

use self::index::{RegionIndex, UvGrid};
use crate::lang::Lang;
use crate::notice::Source;
use crate::state::AppState;

pub use self::idcolor::IdColorOp;
pub use self::tools::{Cycle, CycleKind, Hover, PolygonDrag};

/// 範囲のツールの設定と途中の状態。
pub struct RegionState {
    /// クリックした三角形から辿る範囲（バケツとポリゴン塗りつぶしで共有。Unity 版の「3D Pick」の既定は UV アイランド）。
    pub kind: SurfaceRegionKind,
    /// バケツ: 範囲をモデルの三角形からでなく、押した画素に近い色で決める（3D は押した面の UV が指す画素から）。
    pub by_color: bool,
    /// 近い色の許し幅（各成分の差。0〜255）。
    pub tolerance: u8,
    /// 近い色: 押した画素から 4 近傍でつながる所だけ。
    pub contiguous: bool,
    /// 2D のキャンバスで、効いている対称定規の写しの全部の点から塗る（3D ビューでは効かない）。
    pub snap_symmetry: bool,
    /// 近い色: レイヤーでなくチャンネルの合成を見る。
    pub sample_all: bool,
    pub color: color::Options,
    pub references: std::collections::HashSet<(u128, yolu_core::LayerId)>,
    pub leftover_drag: Option<bucket::Drag>,
    pub job: Option<bucket::Job>,
    /// 消す（画素は透明に。マスクは黒 = 隠す）か、塗る（描画色と不透明度。マスクは白 = 見せる）か。
    pub erase: bool,
    /// ID の色で選ぶときの許し幅（8 bit のチャンネルの差の最大）。
    pub id_tolerance: u8,
    /// 入力のフレームごとの修飾キー（ID の色で選択の足す・引く・重ねる）。
    pub modifiers: egui::Modifiers,
    /// ポリゴン塗りつぶしのドラッグ。
    pub drag: Option<PolygonDrag>,
    /// ポインタの下の範囲（強調）。
    pub hover: Option<Hover>,
    /// 2D で重なった UV の同じ所を続けて押したときの選び替え（ポリゴン塗りつぶしとベイクのアイランドを選ぶ）。
    pub cycle: Option<tools::Cycle>,
    index: Option<Arc<RegionIndex>>,
    grid: Option<Arc<UvGrid>>,
    /// 手動の ID の色を直している部品と、焼いた ID マップの口。
    pub id: idcolor::IdState,
}

impl Default for RegionState {
    fn default() -> Self {
        RegionState {
            kind: SurfaceRegionKind::UvIsland,
            by_color: false,
            tolerance: 32,
            contiguous: true,
            snap_symmetry: true,
            sample_all: false,
            color: Default::default(),
            references: Default::default(),
            leftover_drag: None,
            job: None,
            erase: false,
            id_tolerance: yolu_core::id_colors::DEFAULT_TOLERANCE,
            modifiers: egui::Modifiers::NONE,
            drag: None,
            hover: None,
            cycle: None,
            index: None,
            grid: None,
            id: Default::default(),
        }
    }
}

/// 範囲のツールの操作。
#[derive(Clone, Debug, PartialEq)]
pub enum RegionAction {
    Kind(SurfaceRegionKind),
    /// バケツの範囲を、モデルの範囲の種類にする（近い色をやめる）。
    FillRange(SurfaceRegionKind),
    ByColor(bool),
    Tolerance(u8),
    Contiguous(bool),
    SampleAll(bool),
    /// バケツの「対称定規にスナップ」。
    SnapSymmetry(bool),
    Reference(color::Reference),
    ReferenceLayer(yolu_core::LayerId),
    Distance(color::Distance),
    Gap(u8),
    Margin(i16),
    Leftovers(bool),
    MaxArea(u32),
    Erase(bool),
    IdTolerance(u8),
    /// 手動の ID の色を直す部品（部品の並びの番号）。
    IdPart(usize),
    /// 手動の ID の色の変更（文書を変える。1 回の Undo）。
    IdColor(IdColorOp),
}

/// 範囲の種類の名前。
pub fn kind_name(lang: Lang, kind: SurfaceRegionKind) -> &'static str {
    match kind {
        SurfaceRegionKind::Triangle => lang.pick("三角形", "Triangle"),
        SurfaceRegionKind::MeshPart => lang.pick("メッシュの塊", "Mesh Part"),
        SurfaceRegionKind::UvIsland => lang.pick("UV アイランド", "UV Island"),
        SurfaceRegionKind::Material => lang.pick("マテリアル", "Material"),
    }
}

impl AppState {
    /// 範囲のツールの操作を当てる（描いている間は断る）。
    pub fn region_apply(&mut self, action: RegionAction) {
        if self.is_stroking() {
            self.refuse(
                Source::Fill,
                crate::lang::refusals::during_stroke(self.lang),
            );
            return;
        }
        let r = &mut self.region;
        match action {
            RegionAction::Kind(kind) => {
                r.kind = kind;
                r.hover = None;
            }
            RegionAction::FillRange(kind) => {
                r.kind = kind;
                r.by_color = false;
                r.hover = None;
            }
            RegionAction::ByColor(on) => {
                r.by_color = on;
                r.hover = None;
            }
            RegionAction::Tolerance(v) => r.tolerance = v,
            RegionAction::Contiguous(v) => r.contiguous = v,
            RegionAction::SnapSymmetry(v) => r.snap_symmetry = v,
            RegionAction::SampleAll(v) => {
                r.sample_all = v;
                r.color.reference = if v {
                    color::Reference::Visible
                } else {
                    color::Reference::Editing
                };
            }
            RegionAction::Reference(v) => {
                r.color.reference = v;
                r.sample_all = v == color::Reference::Visible;
            }
            RegionAction::ReferenceLayer(id) => {
                let key = (self.doc.id(), id);
                if self.doc.layer(id).is_some() && !r.references.remove(&key) {
                    r.references.insert(key);
                }
            }
            RegionAction::Distance(v) => r.color.distance = v,
            RegionAction::Gap(v) => r.color.gap = v.min(32),
            RegionAction::Margin(v) => r.color.margin = v.clamp(-200, 200),
            RegionAction::Leftovers(v) => r.color.leftovers = v,
            RegionAction::MaxArea(v) => r.color.max_area = v.clamp(1, 65536),
            RegionAction::Erase(v) => r.erase = v,
            RegionAction::IdTolerance(v) => {
                r.id_tolerance = v;
                r.hover = None;
            }
            RegionAction::IdPart(i) => r.id.part = i,
            RegionAction::IdColor(op) => self.id_color_edit(op),
        }
    }

    /// 今のモデル（見せる形）と、描くテクスチャセットのマテリアルの組。無ければ None（モデルが無い・今のセットがモデルに無い）。
    pub fn region_model(&self) -> Option<(Arc<crate::view3d::model::ViewModel>, i32)> {
        let model = self.view3d.model.clone()?;
        (self.view3d.material >= 0).then_some((model, self.view3d.material))
    }

    /// `region_model` が None の理由（モデルが無い、またはモデルはあるが今のテクスチャセットが付いていない）。3D のブラシと同じ文。
    pub fn region_missing_reason(&self) -> String {
        let lang = self.lang;
        if self.view3d.model.is_none() {
            return lang.pick("モデルがありません", "No model").into();
        }
        let name = &self.sets.current().name;
        lang.pick(
            format!("今のテクスチャセット「{name}」はこのモデルにありません。"),
            format!("Texture set “{name}” is not in this model."),
        )
    }

    /// 今のモデルの範囲の索引（モデルが替わるまで作り直さない）。
    pub fn region_index(&mut self) -> Option<Arc<RegionIndex>> {
        let (model, _) = self.region_model()?;
        let r = &mut self.region;
        if !r.index.as_ref().is_some_and(|i| i.is_for(&model.geometry)) {
            r.index = Some(Arc::new(RegionIndex::new(&model.geometry)));
            r.hover = None;
        }
        r.index.clone()
    }

    /// 今のモデルとセットの UV の格子（作ってあるときだけ。作らない。3D ビューのカーソルが読む）。
    pub fn cached_region_grid(&self) -> Option<Arc<UvGrid>> {
        let (model, material) = self.region_model()?;
        self.region
            .grid
            .clone()
            .filter(|g| g.is_for(&model.geometry, material))
    }

    /// 今のセットの UV の格子（モデルかセットが替わるまで作り直さない）。
    pub fn region_grid(&mut self) -> Option<Arc<UvGrid>> {
        let (model, material) = self.region_model()?;
        let r = &mut self.region;
        if !r
            .grid
            .as_ref()
            .is_some_and(|g| g.is_for(&model.geometry, material))
        {
            r.grid = Some(Arc::new(UvGrid::new(&model.geometry, material)));
        }
        r.grid.clone()
    }
}
