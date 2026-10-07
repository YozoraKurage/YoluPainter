//! 対称の設定。文書に入れない画面の設定で、ストロークを始めるときに core へ写す。
//! - 2D（縦・横・両方・放射状 2〜16）: 中心は文書の大きさに対する 0〜1 の割合で持つ（セットを替えて大きさが違っても、中央は中央の
//!   まま）。軸の線分も、ここで求める。
//! - 3D（[`Symmetry3d`]）: モデルの空間でのミラー（X・Y・Z に直交する面。中心はモデルの原点からのずれ）と、軸のまわりの放射状
//!   （2〜16）。2D とは独立に切り替える（3D のビューと 2D のキャンバスを並べて見ていても、それぞれ別に効く）。

use yolu_core::geometry::{MirrorPlane, RadialSymmetry, SurfaceSymmetrySetup, SymmetryAxis};
use yolu_core::glam::{Quat, Vec3};

use crate::engine::{CanvasSymmetry, DVec2, SymmetryMode};
use crate::lang::Lang;

/// 放射状の写しの数の範囲。
pub const MIN_COUNT: u32 = 2;
pub const MAX_COUNT: u32 = 16;

/// 対称のモードの並び（メニューとボタンの順）。
pub const MODES: [SymmetryMode; 5] = [
    SymmetryMode::None,
    SymmetryMode::Vertical,
    SymmetryMode::Horizontal,
    SymmetryMode::Both,
    SymmetryMode::Radial,
];

pub fn mode_name(lang: Lang, mode: SymmetryMode) -> &'static str {
    match mode {
        SymmetryMode::None => lang.pick("なし", "Off"),
        SymmetryMode::Vertical => lang.pick("縦", "Vertical"),
        SymmetryMode::Horizontal => lang.pick("横", "Horizontal"),
        SymmetryMode::Both => lang.pick("両方", "Both"),
        SymmetryMode::Radial => lang.pick("放射状", "Radial"),
    }
}

/// モードのツールチップ（軸の向きと写し方）。
pub fn mode_tooltip(lang: Lang, mode: SymmetryMode) -> &'static str {
    match mode {
        SymmetryMode::None => lang.pick("対称を使わない", "No symmetry"),
        SymmetryMode::Vertical => lang.pick(
            "縦の軸で左右に写す",
            "Mirror left and right across the vertical axis",
        ),
        SymmetryMode::Horizontal => lang.pick(
            "横の軸で上下に写す",
            "Mirror up and down across the horizontal axis",
        ),
        SymmetryMode::Both => lang.pick(
            "縦と横の両方で 4 つに写す",
            "Mirror across both axes (4 copies)",
        ),
        SymmetryMode::Radial => lang.pick(
            "中心のまわりに回して写す",
            "Rotate copies around the center",
        ),
    }
}

/// 3D の面の対称の面（ミラー）のずれの範囲（モデルの単位。C# と同じ ±1000）。
pub const MAX_OFFSET_3D: f32 = 1000.0;

pub const AXES_3D: [SymmetryAxis; 3] = [SymmetryAxis::X, SymmetryAxis::Y, SymmetryAxis::Z];

pub fn axis_name(axis: SymmetryAxis) -> &'static str {
    match axis {
        SymmetryAxis::X => "X",
        SymmetryAxis::Y => "Y",
        SymmetryAxis::Z => "Z",
    }
}

/// 3D の対称の画面の設定（2D の [`SymmetryState`] とは独立）。モデルの空間のまま持ち、ストロークを始めるときに固める。
#[derive(Clone, Debug, PartialEq)]
pub struct Symmetry3d {
    /// ミラー（軸に直交する面で左右に写す）。
    pub mirror: bool,
    pub axis: SymmetryAxis,
    /// 面のずれ（モデルの原点から、軸の向きに。モデルの単位）。
    pub offset: f32,
    /// 放射状（軸のまわりに回して写す）。ミラーと一緒に使える（鏡映を先に、回転を後に当てる）。
    pub radial: bool,
    pub radial_axis: SymmetryAxis,
    /// 放射状の写しの数（2〜16）。
    pub radial_count: u32,
    /// 写しの側は、カメラから見えない面にも塗る（元の側はいつも見える面だけ）。
    pub ignore_visibility: bool,
    /// 3D ビューに対称の面と軸を出す。
    pub show_plane: bool,
}

impl Default for Symmetry3d {
    fn default() -> Self {
        Symmetry3d {
            mirror: false,
            axis: SymmetryAxis::X,
            offset: 0.0,
            radial: false,
            radial_axis: SymmetryAxis::Y,
            radial_count: 2,
            ignore_visibility: false,
            show_plane: true,
        }
    }
}

impl Symmetry3d {
    /// 3D の写しが効くか（ミラーか放射状のどちらか）。
    pub fn enabled(&self) -> bool {
        self.mirror || self.radial
    }

    /// 面のずれを ±1000 に丸めて置く（有限でない値は無視する）。
    pub fn set_offset(&mut self, offset: f32) {
        if offset.is_finite() {
            self.offset = offset.clamp(-MAX_OFFSET_3D, MAX_OFFSET_3D);
        }
    }

    pub fn set_radial_count(&mut self, count: u32) {
        self.radial_count = count.clamp(MIN_COUNT, MAX_COUNT);
    }

    /// 今の設定のミラーの面（モデルの空間。ルートは原点・回転なし）。
    pub fn plane(&self) -> MirrorPlane {
        MirrorPlane::from_model(Vec3::ZERO, Quat::IDENTITY, self.axis, self.offset)
    }

    /// 今の設定の放射状（モデルの原点を通る軸のまわり）。
    pub fn radial_symmetry(&self) -> Option<RadialSymmetry> {
        RadialSymmetry::from_model(
            Vec3::ZERO,
            Quat::IDENTITY,
            self.radial_axis,
            self.radial_count,
        )
        .ok()
    }

    /// core に渡す設定（どちらも切っていれば None）。
    pub fn setup(&self) -> Option<SurfaceSymmetrySetup> {
        self.enabled().then(|| SurfaceSymmetrySetup {
            mirror: self.mirror.then(|| self.plane()),
            radial: if self.radial {
                self.radial_symmetry()
            } else {
                None
            },
            ignore_visibility: self.ignore_visibility,
        })
    }

    /// 面の中心をモデルの境界の中央にするずれ（今の軸で）。
    pub fn bounds_center_offset(&self, center: Vec3) -> f32 {
        MirrorPlane::from_model(Vec3::ZERO, Quat::IDENTITY, self.axis, 0.0).signed_distance(center)
    }
}

/// 対称の画面の設定。
#[derive(Clone, Debug, PartialEq)]
pub struct SymmetryState {
    pub mode: SymmetryMode,
    /// 中心（文書の幅・高さに対する割合。左下が原点）。
    pub center: (f64, f64),
    /// 放射状の写しの数（2〜16）。
    pub count: u32,
    /// キャンバスの上に軸を出す。
    pub show_axes: bool,
    /// 切る前のモード（切り替えで入れ直す。覚えが無ければ縦）。
    pub last_mode: SymmetryMode,
    /// 3D の面の対称。
    pub surface: Symmetry3d,
}

impl Default for SymmetryState {
    fn default() -> Self {
        SymmetryState {
            mode: SymmetryMode::None,
            center: (0.5, 0.5),
            count: 2,
            show_axes: true,
            last_mode: SymmetryMode::Vertical,
            surface: Symmetry3d::default(),
        }
    }
}

impl SymmetryState {
    /// 対称が効くか。
    pub fn enabled(&self) -> bool {
        self.mode != SymmetryMode::None
    }

    pub fn set_mode(&mut self, mode: SymmetryMode) {
        if mode != SymmetryMode::None {
            self.last_mode = mode;
        }
        self.mode = mode;
    }

    /// 入っているなら切り、切っているなら最後のモードを入れ直す。
    pub fn toggle(&mut self) {
        if self.enabled() {
            self.mode = SymmetryMode::None;
        } else {
            self.mode = self.last_mode;
        }
    }

    pub fn set_count(&mut self, count: u32) {
        self.count = count.clamp(MIN_COUNT, MAX_COUNT);
    }

    /// 中心を 0〜1 に丸めて置く（有限でない値は無視する）。
    pub fn set_center(&mut self, x: f64, y: f64) {
        if x.is_finite() && y.is_finite() {
            self.center = (x.clamp(0.0, 1.0), y.clamp(0.0, 1.0));
        }
    }

    /// core に渡す設定（文書の大きさに写した中心）。
    pub fn canvas(&self, width: u32, height: u32) -> CanvasSymmetry {
        CanvasSymmetry {
            mode: self.mode,
            center: DVec2::new(self.center.0 * width as f64, self.center.1 * height as f64),
            count: self.count.clamp(MIN_COUNT, MAX_COUNT),
        }
    }
}

/// 軸の線分（キャンバスの座標。縦・横はキャンバスの端から端まで、放射状は中心からキャンバスの端までの放線）。
pub fn axis_lines(s: &CanvasSymmetry, width: u32, height: u32) -> Vec<((f64, f64), (f64, f64))> {
    let (w, h) = (width as f64, height as f64);
    let (cx, cy) = (s.center.x, s.center.y);
    let mut lines = Vec::new();
    if matches!(s.mode, SymmetryMode::Vertical | SymmetryMode::Both) {
        lines.push(((cx, 0.0), (cx, h)));
    }
    if matches!(s.mode, SymmetryMode::Horizontal | SymmetryMode::Both) {
        lines.push(((0.0, cy), (w, cy)));
    }
    if s.mode == SymmetryMode::Radial {
        for t in s.transforms().unwrap_or_default() {
            // 中心から右へ 1 だけ進んだ点の写り先の向きへ、キャンバスの端まで
            let (x, y) = t.map(cx + 1.0, cy);
            let (dx, dy) = (x - cx, y - cy);
            let mut length = f64::INFINITY;
            if dx > 1e-12 {
                length = length.min((w - cx) / dx);
            } else if dx < -1e-12 {
                length = length.min(-cx / dx);
            }
            if dy > 1e-12 {
                length = length.min((h - cy) / dy);
            } else if dy < -1e-12 {
                length = length.min(-cy / dy);
            }
            if length.is_finite() {
                lines.push(((cx, cy), (cx + dx * length, cy + dy * length)));
            }
        }
    }
    lines
}

/// 点（キャンバスの座標）の、対称の写し（元の点を除く）。映した側のカーソルに使う。
pub fn mirrored_points(s: &CanvasSymmetry, x: f64, y: f64) -> Vec<(f64, f64)> {
    s.transforms()
        .map(|ts| ts.iter().skip(1).map(|t| t.map(x, y)).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn center_follows_the_document_size() {
        let s = SymmetryState::default();
        let a = s.canvas(100, 60);
        let b = s.canvas(2048, 512);
        assert_eq!((a.center.x, a.center.y), (50.0, 30.0));
        assert_eq!((b.center.x, b.center.y), (1024.0, 256.0));
    }

    #[test]
    fn toggle_remembers_the_last_mode_and_values_are_clamped() {
        let mut s = SymmetryState::default();
        s.toggle();
        assert_eq!(s.mode, SymmetryMode::Vertical, "覚えが無ければ縦");
        s.set_mode(SymmetryMode::Radial);
        s.toggle();
        assert_eq!(s.mode, SymmetryMode::None);
        s.toggle();
        assert_eq!(s.mode, SymmetryMode::Radial, "最後のモードへ戻る");
        s.set_mode(SymmetryMode::None);
        assert_eq!(s.last_mode, SymmetryMode::Radial, "なしは覚えに入れない");
        s.set_count(99);
        assert_eq!(s.count, MAX_COUNT);
        s.set_count(0);
        assert_eq!(s.count, MIN_COUNT);
        s.set_center(-1.0, 3.0);
        assert_eq!(s.center, (0.0, 1.0));
        s.set_center(f64::NAN, 0.5);
        assert_eq!(s.center, (0.0, 1.0), "有限でない値は無視する");
    }

    #[test]
    fn axes_cover_the_canvas_and_radial_rays_stop_at_the_edge() {
        let s = |mode| CanvasSymmetry {
            mode,
            center: DVec2::new(50.0, 20.0),
            count: 4,
        };
        assert!(axis_lines(&s(SymmetryMode::None), 100, 40).is_empty());
        assert_eq!(
            axis_lines(&s(SymmetryMode::Vertical), 100, 40),
            vec![((50.0, 0.0), (50.0, 40.0))]
        );
        assert_eq!(
            axis_lines(&s(SymmetryMode::Horizontal), 100, 40),
            vec![((0.0, 20.0), (100.0, 20.0))]
        );
        assert_eq!(axis_lines(&s(SymmetryMode::Both), 100, 40).len(), 2);
        let rays = axis_lines(&s(SymmetryMode::Radial), 100, 40);
        assert_eq!(rays.len(), 4);
        // 右・上・左・下の放線がキャンバスの端で終わる
        for (a, b) in &rays {
            assert_eq!(*a, (50.0, 20.0));
            let on_edge = b.0.abs() < 1e-9
                || (b.0 - 100.0).abs() < 1e-9
                || b.1.abs() < 1e-9
                || (b.1 - 40.0).abs() < 1e-9;
            assert!(on_edge, "{b:?}");
        }
    }

    #[test]
    fn the_3d_symmetry_is_off_until_mirror_or_radial_and_builds_the_core_setup() {
        let mut s = Symmetry3d::default();
        assert!(!s.enabled() && s.setup().is_none());
        s.mirror = true;
        s.axis = SymmetryAxis::Y;
        s.set_offset(0.25);
        let setup = s.setup().expect("ミラーが入った");
        let plane = setup.mirror.expect("面");
        assert_eq!(plane.normal, Vec3::Y);
        assert!((plane.signed_distance(Vec3::new(5.0, 0.25, -3.0))).abs() < 1e-6);
        assert!(setup.radial.is_none() && !setup.ignore_visibility);
        s.radial = true;
        s.radial_axis = SymmetryAxis::Z;
        s.set_radial_count(6);
        s.ignore_visibility = true;
        let setup = s.setup().unwrap();
        let radial = setup.radial.expect("放射状");
        assert_eq!((radial.axis, radial.count), (Vec3::Z, 6));
        assert!(setup.ignore_visibility && setup.mirror.is_some());
        s.mirror = false;
        assert!(s.setup().unwrap().mirror.is_none(), "放射状だけ");
    }

    #[test]
    fn the_3d_offset_and_count_are_clamped_and_bounds_center_follows_the_axis() {
        let mut s = Symmetry3d::default();
        s.set_offset(5000.0);
        assert_eq!(s.offset, MAX_OFFSET_3D);
        s.set_offset(-5000.0);
        assert_eq!(s.offset, -MAX_OFFSET_3D);
        s.set_offset(f32::NAN);
        assert_eq!(s.offset, -MAX_OFFSET_3D, "有限でない値は無視する");
        s.set_radial_count(99);
        assert_eq!(s.radial_count, MAX_COUNT);
        s.set_radial_count(0);
        assert_eq!(s.radial_count, MIN_COUNT);
        let center = Vec3::new(1.0, 2.0, 3.0);
        for (axis, expected) in [
            (SymmetryAxis::X, 1.0),
            (SymmetryAxis::Y, 2.0),
            (SymmetryAxis::Z, 3.0),
        ] {
            s.axis = axis;
            assert_eq!(s.bounds_center_offset(center), expected);
        }
        // 置いた面は境界の中央を通る
        s.axis = SymmetryAxis::Y;
        s.set_offset(s.bounds_center_offset(center));
        assert!(s.plane().signed_distance(center).abs() < 1e-6);
    }

    #[test]
    fn mirrored_points_skip_the_original() {
        let both = CanvasSymmetry {
            mode: SymmetryMode::Both,
            center: DVec2::new(50.0, 50.0),
            count: 2,
        };
        let p = mirrored_points(&both, 10.0, 20.0);
        assert_eq!(p.len(), 3);
        assert!(
            p.contains(&(90.0, 20.0)) && p.contains(&(10.0, 80.0)) && p.contains(&(90.0, 80.0))
        );
        assert!(mirrored_points(&CanvasSymmetry::default(), 1.0, 2.0).is_empty());
    }
}
