//! 定規（レイヤーとグループに付く、描くときの寄せ先と対称）。
//!
//! 定規はレイヤー（グループもレイヤー）が持ち（[`Layer::rulers`](crate::Layer::rulers)）、文書と一緒に保存される（.ylp は正本の版 35）。
//! 2D の定規は文書の画素（左下が原点。パスの点・[`CanvasSymmetry`] の中心と同じ空間）、3D の定規はモデルの空間（投影の箱・デカール・
//! 3D の対称と同じ空間）に置く。ID は文書の中で重ならない（[`Document::new_ruler_id`](crate::Document::new_ruler_id)）。
//!
//! - 種類は直線定規・平行線・同心円・パース・対称。直線定規以外は「特殊定規」で、描く所（2D と 3D は別に数える）ごとに、効く特殊定規は
//!   `snap` の印のある 1 つだけ（[`Document::snapping_special_rulers`](crate::Document::snapping_special_rulers)）。
//! - どの描くレイヤーから見えるかは、表示の範囲（[`RulerScope`]）・表示の入り切り・持ち主（とその親）の目で決まる
//!   （[`Document::rulers_seen_from`](crate::Document::rulers_seen_from)）。
//! - 対称定規は、2D は [`CanvasSymmetry`]（[`Ruler::canvas_symmetry`]）、3D は [`SurfaceSymmetrySetup`]（[`Ruler::surface_symmetry`]。
//!   鏡 1 枚＋軸のまわりの回転）へ変える。写しの集合は、線対称 N 本が二面体群の N 個、回転対称 N 本が回転の N 個。
//! - 編集（作る・動かす・消す・移す・設定を変える）は、それぞれ 1 回の取り消し（[`Document::set_rulers`](crate::Document::set_rulers)・
//!   [`Document::move_rulers`](crate::Document::move_rulers)・[`Document::set_snap_ruler`](crate::Document::set_snap_ruler)）。ドラッグと
//!   スライダーは `coalesce` で 1 回にまとめる。画素も合成も変えない。

use std::fmt;

use glam::{DVec2, DVec3, Vec3};

use crate::error::CoreError;
use crate::geometry::{MirrorPlane, RadialSymmetry, SurfaceSymmetrySetup};
use crate::layer::LayerId;
use crate::symmetry::{CanvasSymmetry, SymmetryMode};

/// 1 つのレイヤーに付けられる定規の数の上限。
pub const MAX_RULERS_PER_LAYER: usize = 64;
/// 対称定規の線の本数の範囲。
pub const LINES: std::ops::RangeInclusive<u8> = 2..=16;
/// 2D の点の座標の範囲（文書の画素。[`CanvasSymmetry`] の中心と同じ）。
pub const MAX_CANVAS_COORD: f64 = 1e7;
/// 3D の点の座標の範囲（モデルの空間。パスの鏡の面と同じ）。
pub const MAX_MODEL_COORD: f64 = 1e6;
/// 2D の `a` と `b` の最小の間隔（画素）。
pub const MIN_CANVAS_SEPARATION: f64 = 0.01;
/// 3D の `a` と `b` の最小の間隔（モデルの単位）。
pub const MIN_MODEL_SEPARATION: f64 = 1e-6;

/// 定規の ID（128 ビット。0 は使わない。文書の中で重ならない）。
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RulerId(pub u128);

impl fmt::Debug for RulerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RulerId({:032x})", self.0)
    }
}
impl fmt::Display for RulerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:032x}", self.0)
    }
}

/// 定規の種類（番号は .ylp の `kind`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RulerKind {
    /// 直線定規（`a`〜`b` の線）。
    Line = 0,
    /// 平行線（`a` を通る、`a`→`b` の向きの線）。
    Parallel = 1,
    /// 同心円（中心 `a`、表示の半径 `a`〜`b`）。
    Concentric = 2,
    /// パース（消失点 `a`、2 点のとき消失点 `b` も）。
    Perspective = 3,
    /// 対称（中心 `a`、最初の線の向き `a`→`b`）。
    Symmetry = 4,
}

impl RulerKind {
    pub const ALL: [RulerKind; 5] = [
        RulerKind::Line,
        RulerKind::Parallel,
        RulerKind::Concentric,
        RulerKind::Perspective,
        RulerKind::Symmetry,
    ];

    /// .ylp の `kind` の値（0〜4）。
    pub fn index(self) -> u8 {
        self as u8
    }

    pub fn from_index(index: u8) -> Option<RulerKind> {
        RulerKind::ALL.get(index as usize).copied()
    }

    /// 特殊定規（直線定規以外）か。描く所ごとに、`snap` の印のある 1 つだけが効く。
    pub fn is_special(self) -> bool {
        self != RulerKind::Line
    }
}

/// 表示の範囲（どの描くレイヤーから見えるか。下の `visible` が偽なら、どの範囲でも見えない）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RulerScope {
    /// すべてのレイヤー。
    All = 0,
    /// 同じグループの中（持ち主がグループならその中、そうでなければ持ち主の親の中。入れ子の下まで全部。一番上のレイヤーの定規は
    /// 文書の全部）。既定。
    Group = 1,
    /// 選んでいるときだけ（描くレイヤーが持ち主、持ち主がグループならその中のレイヤー）。
    Selected = 2,
}

impl RulerScope {
    pub fn index(self) -> u8 {
        self as u8
    }

    pub fn from_index(index: u8) -> Option<RulerScope> {
        match index {
            0 => Some(RulerScope::All),
            1 => Some(RulerScope::Group),
            2 => Some(RulerScope::Selected),
            _ => None,
        }
    }
}

/// 定規を置く空間。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RulerSpace {
    /// 2D: 文書の画素。
    Canvas,
    /// 3D: モデルの空間。
    Model,
}

/// 定規の置き場（点の意味は種類ごと。[`RulerKind`]）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RulerPlace {
    /// 2D: 文書の画素（左下が原点）。
    Canvas { a: DVec2, b: DVec2 },
    /// 3D: モデルの空間。`up` は向き（直線・平行線・パースは作った面の法線、同心円は円の面の法線、対称は回転の軸。最初の鏡の面は
    /// `up` と `b − a` を含む）。ストアには単位にして持つ（[`Ruler::normalized`]）。
    Model { a: DVec3, b: DVec3, up: DVec3 },
}

/// 定規。
#[derive(Clone, Debug, PartialEq)]
pub struct Ruler {
    pub id: RulerId,
    pub kind: RulerKind,
    pub place: RulerPlace,
    /// パースだけ: 消失点が 2 つ。
    pub two_points: bool,
    /// 対称だけ: 線の本数（2〜16）。ほかの種類は 0。
    pub lines: u8,
    /// 対称だけ: 線対称（`lines` が偶数のときだけ）。偽は回転対称。
    pub line_symmetry: bool,
    /// 3D の対称だけ: 写しは見えない面にも塗る。
    pub see_through: bool,
    /// 表示（偽なら、どの表示の範囲でも見えず、寄せも写しもしない）。
    pub visible: bool,
    pub scope: RulerScope,
    /// スナップする特殊定規の印（直線定規は偽）。
    pub snap: bool,
}

/// 定規とその持ち主（[`Document::rulers_seen_from`](crate::Document::rulers_seen_from) の要素）。
#[derive(Clone, Copy, Debug)]
pub struct RulerRef<'a> {
    pub owner: LayerId,
    pub ruler: &'a Ruler,
}

/// 描く所ごとに効く特殊定規（[`Document::snapping_special_rulers`](crate::Document::snapping_special_rulers)）。2D と 3D は別に数える。
#[derive(Clone, Copy, Debug, Default)]
pub struct SpecialRulers<'a> {
    /// 2D の特殊定規（文書の画素の空間のもの）。
    pub canvas: Option<RulerRef<'a>>,
    /// 3D の特殊定規（モデルの空間のもの）。
    pub model: Option<RulerRef<'a>>,
}

/// 単位の向き。すでに単位（誤差 1e-12 以内）ならそのまま返す（何度通しても同じ値）。長さが 0 か有限でなければ None。
fn unit(v: DVec3) -> Option<DVec3> {
    let len = v.length();
    if !len.is_finite() || len < 1e-12 {
        return None;
    }
    if (len - 1.0).abs() <= 1e-12 {
        Some(v)
    } else {
        Some(v / len)
    }
}

impl Ruler {
    /// 2D の定規。表示、表示の範囲は同じグループの中、スナップの印なし。対称は線対称 2 本（鏡 1 枚）。
    pub fn canvas(id: RulerId, kind: RulerKind, a: DVec2, b: DVec2) -> Ruler {
        Ruler::with_place(id, kind, RulerPlace::Canvas { a, b })
    }

    /// 3D の定規（`up` は [`RulerPlace::Model`]）。ほかは [`Ruler::canvas`] と同じ。
    pub fn model(id: RulerId, kind: RulerKind, a: DVec3, b: DVec3, up: DVec3) -> Ruler {
        Ruler::with_place(id, kind, RulerPlace::Model { a, b, up })
    }

    fn with_place(id: RulerId, kind: RulerKind, place: RulerPlace) -> Ruler {
        let symmetry = kind == RulerKind::Symmetry;
        Ruler {
            id,
            kind,
            place,
            two_points: false,
            lines: if symmetry { 2 } else { 0 },
            line_symmetry: symmetry,
            see_through: false,
            visible: true,
            scope: RulerScope::Group,
            snap: false,
        }
    }

    pub fn space(&self) -> RulerSpace {
        match self.place {
            RulerPlace::Canvas { .. } => RulerSpace::Canvas,
            RulerPlace::Model { .. } => RulerSpace::Model,
        }
    }

    pub fn is_special(&self) -> bool {
        self.kind.is_special()
    }

    /// `up` を単位にした定規（2D は変わらない）。単位のものは値を変えない（保存→読み込みで同じ値）。`up` の長さが 0 か有限でなければ
    /// そのまま返す（[`Ruler::validate`] が断る）。
    pub fn normalized(mut self) -> Ruler {
        if let RulerPlace::Model { up, .. } = &mut self.place {
            if let Some(u) = unit(*up) {
                *up = u;
            }
        }
        self
    }

    /// 値の確かめ（core と .ylp の読み手で同じ）: ID は 0 でない、座標は有限で 2D は ±1e7・3D は ±1e6、`a` と `b` は離れている（2D は
    /// 0.01 画素、3D は 1e-6）、`up` は長さが 0 でない、`lines` は対称が 2〜16（ほかは 0）、線対称は偶数だけ、種類に無い印
    /// （`two_points` はパース、`line_symmetry` は対称、`see_through` は 3D の対称だけ、直線定規の `snap`）は既定の値、
    /// 3D の対称は `b − a` が `up` と平行でない。
    pub fn validate(&self) -> Result<(), CoreError> {
        if self.id.0 == 0 {
            return Err(CoreError::InvalidArgument("定規の ID（0 は使わない）"));
        }
        match self.place {
            RulerPlace::Canvas { a, b } => {
                if !(a.is_finite() && b.is_finite())
                    || a.abs().max_element() > MAX_CANVAS_COORD
                    || b.abs().max_element() > MAX_CANVAS_COORD
                {
                    return Err(CoreError::InvalidArgument("定規の点（有限・±1e7）"));
                }
                if a.distance(b) < MIN_CANVAS_SEPARATION {
                    return Err(CoreError::InvalidArgument(
                        "定規の 2 点が近すぎる（0.01 画素以上）",
                    ));
                }
                if self.see_through {
                    return Err(CoreError::InvalidArgument(
                        "見えない面にも写すのは 3D の対称だけ",
                    ));
                }
            }
            RulerPlace::Model { a, b, up } => {
                if !(a.is_finite() && b.is_finite() && up.is_finite())
                    || a.abs().max_element() > MAX_MODEL_COORD
                    || b.abs().max_element() > MAX_MODEL_COORD
                    || up.abs().max_element() > MAX_MODEL_COORD
                {
                    return Err(CoreError::InvalidArgument("定規の点（有限・±1e6）"));
                }
                if a.distance(b) < MIN_MODEL_SEPARATION {
                    return Err(CoreError::InvalidArgument("定規の 2 点が近すぎる（1e-6）"));
                }
                let Some(u) = unit(up) else {
                    return Err(CoreError::InvalidArgument("定規の向きが 0"));
                };
                if self.kind == RulerKind::Symmetry {
                    let d = b - a;
                    if (d - u * d.dot(u)).length() < MIN_MODEL_SEPARATION {
                        return Err(CoreError::InvalidArgument(
                            "対称定規の最初の線が回転の軸と平行",
                        ));
                    }
                } else if self.see_through {
                    return Err(CoreError::InvalidArgument(
                        "見えない面にも写すのは 3D の対称だけ",
                    ));
                }
            }
        }
        if self.two_points && self.kind != RulerKind::Perspective {
            return Err(CoreError::InvalidArgument("2 点はパースだけ"));
        }
        if self.kind == RulerKind::Symmetry {
            if !LINES.contains(&self.lines) {
                return Err(CoreError::InvalidArgument("対称定規の線の本数（2〜16）"));
            }
            if self.line_symmetry && !self.lines.is_multiple_of(2) {
                return Err(CoreError::InvalidArgument("線対称の線の本数（偶数）"));
            }
        } else if self.lines != 0 || self.line_symmetry {
            return Err(CoreError::InvalidArgument("線の本数と線対称は対称定規だけ"));
        }
        if self.kind == RulerKind::Line && self.snap {
            return Err(CoreError::InvalidArgument(
                "直線定規はスナップする特殊定規にならない",
            ));
        }
        Ok(())
    }

    /// 対称定規の 2D の写し（2D の対称定規だけ）。回転対称は [`SymmetryMode::Radial`]（角度は写しによらないので 0）。線対称は向き
    /// （`b − a`。度、+X から反時計回り。軸と 45 度の向きは正確な値）が、今の縦・横・両方と同じ写しになる場合だけそのモード
    /// （2 本で縦の向きなら [`SymmetryMode::Vertical`]、2 本で横なら [`SymmetryMode::Horizontal`]、4 本で軸の向きなら [`SymmetryMode::Both`]）を返し、
    /// ほかは [`SymmetryMode::Lines`]。写しの集合だけでなく並びまで今と同じになるので、ブラシ・3D のストロークの 2D の写し・パスの描く順の
    /// どれでも、今のモードとバイトまで同じ。確かめに通らない定規は None。
    pub fn canvas_symmetry(&self) -> Option<CanvasSymmetry> {
        if self.kind != RulerKind::Symmetry || self.validate().is_err() {
            return None;
        }
        let RulerPlace::Canvas { a, b } = self.place else {
            return None;
        };
        let count = u32::from(self.lines);
        if !self.line_symmetry {
            return CanvasSymmetry::new(SymmetryMode::Radial, a, count).ok();
        }
        let angle = direction_degrees(b - a);
        let along_x = angle == 0.0 || angle == 180.0;
        let along_y = angle == 90.0 || angle == -90.0;
        let mode = match self.lines {
            2 if along_y => Some(SymmetryMode::Vertical),
            2 if along_x => Some(SymmetryMode::Horizontal),
            4 if along_x || along_y => Some(SymmetryMode::Both),
            _ => None,
        };
        match mode {
            Some(mode) => CanvasSymmetry::new(mode, a, 2).ok(),
            None => CanvasSymmetry::lines(a, count, angle).ok(),
        }
    }

    /// 対称定規の 3D の写し（3D の対称定規だけ）。鏡の面は `a` を通り `up` と `b − a` を含む面（線対称のとき）、回転は `a` を通る `up` の
    /// まわり（線対称 N 本は N/2 個で、2 本なら回転なし。回転対称 N 本は N 個）。鏡映を先に、回転を後に当てる。確かめに通らない
    /// 定規は None。
    pub fn surface_symmetry(&self) -> Option<SurfaceSymmetrySetup> {
        if self.kind != RulerKind::Symmetry || self.validate().is_err() {
            return None;
        }
        let RulerPlace::Model { a, b, up } = self.place else {
            return None;
        };
        let up = unit(up)?;
        let origin = a.as_vec3();
        let axis = up.as_vec3();
        let count = u32::from(self.lines);
        let (mirror, radial) = if self.line_symmetry {
            // d は b − a から up の成分を除いた単位の向き。鏡の法線は up × d（up と d の両方に直交）
            let d = unit((b - a) - up * (b - a).dot(up))?;
            let mirror =
                MirrorPlane::new(origin, axis.cross(d.as_vec3()), axis, d.as_vec3()).ok()?;
            let turns = count / 2;
            let radial = if turns >= 2 {
                Some(RadialSymmetry::new(origin, axis, turns).ok()?)
            } else {
                None
            };
            (Some(mirror), radial)
        } else {
            (None, Some(RadialSymmetry::new(origin, axis, count).ok()?))
        };
        Some(SurfaceSymmetrySetup {
            mirror,
            radial,
            ignore_visibility: self.see_through,
        })
    }

    /// 定規の中心のモデルの空間の点（3D の定規）。
    pub fn model_origin(&self) -> Option<Vec3> {
        match self.place {
            RulerPlace::Model { a, .. } => Some(a.as_vec3()),
            RulerPlace::Canvas { .. } => None,
        }
    }
}

/// 向き（2D）の角度。度、+X から反時計回り（−180 より大きく 180 以下）。軸と 45 度の向きは正確な値。
pub(crate) fn direction_degrees(d: DVec2) -> f64 {
    if d.y == 0.0 {
        return if d.x >= 0.0 { 0.0 } else { 180.0 };
    }
    if d.x == 0.0 {
        // y は 0 でない（上で返している）ので、符号の判定は `> 0.0` と同じ
        return if d.y.is_sign_positive() { 90.0 } else { -90.0 };
    }
    if d.x.abs() == d.y.abs() {
        // ここへ来る時点で x・y とも 0 でも NaN でもないので、符号の判定は `> 0.0` と同じ
        return match (d.x.is_sign_positive(), d.y.is_sign_positive()) {
            (true, true) => 45.0,
            (false, true) => 135.0,
            (false, false) => -135.0,
            (true, false) => -45.0,
        };
    }
    d.y.atan2(d.x).to_degrees()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u128) -> RulerId {
        RulerId(n)
    }

    fn mirror(a: DVec2, b: DVec2) -> Ruler {
        Ruler::canvas(id(1), RulerKind::Symmetry, a, b)
    }

    #[test]
    fn directions_on_the_axes_and_diagonals_are_exact_degrees() {
        for (d, want) in [
            (DVec2::new(3.0, 0.0), 0.0),
            (DVec2::new(-3.0, 0.0), 180.0),
            (DVec2::new(0.0, 2.0), 90.0),
            (DVec2::new(0.0, -2.0), -90.0),
            (DVec2::new(5.0, 5.0), 45.0),
            (DVec2::new(-5.0, 5.0), 135.0),
            (DVec2::new(-5.0, -5.0), -135.0),
            (DVec2::new(5.0, -5.0), -45.0),
        ] {
            assert_eq!(direction_degrees(d), want, "{d:?}");
        }
        assert!((direction_degrees(DVec2::new(1.0, 2.0)) - 63.434_948_822_922_01).abs() < 1e-12);
    }

    #[test]
    fn kind_and_scope_numbers_round_trip_for_every_value() {
        for (k, kind) in RulerKind::ALL.into_iter().enumerate() {
            assert_eq!(usize::from(kind.index()), k, "{kind:?} の番号");
            assert_eq!(RulerKind::from_index(kind.index()), Some(kind));
        }
        assert_eq!(RulerKind::from_index(5), None);
        for (k, scope) in [RulerScope::All, RulerScope::Group, RulerScope::Selected]
            .into_iter()
            .enumerate()
        {
            assert_eq!(usize::from(scope.index()), k, "{scope:?} の番号");
            assert_eq!(RulerScope::from_index(scope.index()), Some(scope));
        }
        assert_eq!(RulerScope::from_index(3), None);
    }

    #[test]
    fn unit_is_idempotent_and_refuses_zero() {
        let u = unit(DVec2::new(3.0, 4.0).extend(12.0)).unwrap();
        assert!((u.length() - 1.0).abs() < 1e-15);
        assert_eq!(unit(u), Some(u), "単位のものはそのまま");
        assert_eq!(unit(DVec3::ZERO), None);
        assert_eq!(unit(DVec3::new(f64::NAN, 0.0, 1.0)), None);
    }

    #[test]
    fn validate_refuses_values_a_kind_cannot_have() {
        let line = Ruler::canvas(id(1), RulerKind::Line, DVec2::ZERO, DVec2::new(10.0, 0.0));
        assert!(line.validate().is_ok());
        let mut bad = line.clone();
        bad.snap = true;
        assert!(bad.validate().is_err(), "直線定規の snap");
        let mut bad = line.clone();
        bad.two_points = true;
        assert!(bad.validate().is_err(), "パース以外の 2 点");
        let mut bad = line.clone();
        bad.lines = 4;
        assert!(bad.validate().is_err(), "対称以外の線の本数");
        let mut bad = line.clone();
        bad.see_through = true;
        assert!(bad.validate().is_err(), "2D の見えない面にも写す");
        let mut bad = line.clone();
        bad.id = id(0);
        assert!(bad.validate().is_err(), "空の ID");
        let near = Ruler::canvas(id(1), RulerKind::Line, DVec2::ZERO, DVec2::new(0.005, 0.0));
        assert!(near.validate().is_err(), "2 点が近すぎる");
        let far = Ruler::canvas(id(1), RulerKind::Line, DVec2::ZERO, DVec2::new(2e7, 0.0));
        assert!(far.validate().is_err(), "座標が範囲の外");
        let nan = Ruler::canvas(
            id(1),
            RulerKind::Line,
            DVec2::new(f64::NAN, 0.0),
            DVec2::new(1.0, 0.0),
        );
        assert!(nan.validate().is_err(), "有限でない");

        let mut s = mirror(DVec2::ZERO, DVec2::new(0.0, 1.0));
        assert!(s.validate().is_ok());
        for lines in [0u8, 1, 17, 255] {
            s.lines = lines;
            assert!(s.validate().is_err(), "線の本数 {lines}");
        }
        s.lines = 3;
        assert!(s.validate().is_err(), "線対称は偶数だけ");
        s.line_symmetry = false;
        assert!(s.validate().is_ok(), "回転対称は奇数もよい");
        s.lines = 16;
        s.line_symmetry = true;
        assert!(s.validate().is_ok());
    }

    #[test]
    fn model_validation_checks_up_and_the_first_line() {
        let a = DVec3::new(0.0, 0.0, 0.0);
        let sym = |b: DVec3, up: DVec3| Ruler::model(id(7), RulerKind::Symmetry, a, b, up);
        assert!(sym(DVec3::X, DVec3::Y).validate().is_ok());
        assert!(sym(DVec3::X, DVec3::ZERO).validate().is_err(), "up が 0");
        assert!(
            sym(DVec3::Y * 3.0, DVec3::Y).validate().is_err(),
            "最初の線が軸と平行"
        );
        assert!(
            sym(DVec3::Y * 3.0, -DVec3::Y).validate().is_err(),
            "逆向きでも平行"
        );
        assert!(
            sym(DVec3::splat(2e6), DVec3::Y).validate().is_err(),
            "3D の座標は ±1e6"
        );
        let mut line = Ruler::model(id(7), RulerKind::Line, a, DVec3::X, DVec3::Z);
        line.see_through = true;
        assert!(line.validate().is_err(), "見えない面にも写すのは対称だけ");
    }

    /// 3D の点は a・b・up のどれが範囲（±1e6）の外でも断られ（有限でも）、a と b の間隔はちょうど最小の値なら通る。
    #[test]
    fn model_validation_checks_each_of_a_b_and_up_and_the_exact_minimum_separation() {
        let line = |a: DVec3, b: DVec3, up: DVec3| Ruler::model(id(7), RulerKind::Line, a, b, up);
        assert!(line(DVec3::ZERO, DVec3::X, DVec3::Y).validate().is_ok());
        let far = 2.0 * MAX_MODEL_COORD;
        let (fx, fy, fz) = (DVec3::X * far, DVec3::Y * far, DVec3::Z * far);
        assert!(
            line(fx, DVec3::X, DVec3::Y).validate().is_err(),
            "a だけ範囲の外"
        );
        assert!(
            line(DVec3::ZERO, fy, DVec3::Y).validate().is_err(),
            "b だけ範囲の外"
        );
        assert!(
            line(DVec3::ZERO, DVec3::X, fz).validate().is_err(),
            "up だけ範囲の外"
        );
        // 範囲の端ちょうどは通る
        let edge = MAX_MODEL_COORD;
        assert!(line(-DVec3::X * edge, DVec3::X * edge, DVec3::Y * edge)
            .validate()
            .is_ok());
        // 間隔: 最小の値ちょうどは通り、それより近いと断られる
        let exact = DVec3::X * MIN_MODEL_SEPARATION;
        assert_eq!(
            DVec3::ZERO.distance(exact),
            MIN_MODEL_SEPARATION,
            "試験の前提"
        );
        assert!(
            line(DVec3::ZERO, exact, DVec3::Y).validate().is_ok(),
            "ちょうど最小の間隔"
        );
        assert!(
            line(DVec3::ZERO, exact * 0.9, DVec3::Y).validate().is_err(),
            "最小より近い"
        );
        // 有限でない値は、a・b・up のどれか 1 つだけでも断られる
        let nan = DVec3::new(f64::NAN, 0.0, 0.0);
        let bad = |a: DVec3, b: DVec3, up: DVec3| line(a, b, up).validate().is_err();
        assert!(bad(nan, DVec3::X, DVec3::Y), "a だけ有限でない");
        assert!(bad(DVec3::ZERO, nan, DVec3::Y), "b だけ有限でない");
        assert!(bad(DVec3::ZERO, DVec3::X, nan), "up だけ有限でない");
    }

    /// 3D の対称定規の最初の線が軸と平行かは、`a` が原点でなくても `b − a` で決まる。軸に直交する成分がちょうど最小の値なら通る。
    #[test]
    fn a_3d_symmetry_first_line_is_judged_by_b_minus_a_with_an_exact_minimum() {
        let a = DVec3::new(1.0, 2.0, 3.0);
        let sym = |offset: DVec3| Ruler::model(id(7), RulerKind::Symmetry, a, a + offset, DVec3::Y);
        assert!(sym(DVec3::Y * 3.0).validate().is_err(), "軸と平行");
        assert!(sym(DVec3::X + DVec3::Y * 3.0).validate().is_ok());
        let tiny = DVec3::X * MIN_MODEL_SEPARATION + DVec3::Y * 5.0;
        let ruler = Ruler::model(id(7), RulerKind::Symmetry, DVec3::ZERO, tiny, DVec3::Y);
        assert_eq!(
            (tiny - DVec3::Y * tiny.dot(DVec3::Y)).length(),
            MIN_MODEL_SEPARATION,
            "試験の前提"
        );
        assert!(ruler.validate().is_ok(), "直交する成分がちょうど最小");
    }

    /// 2D の `a` と `b` の間隔も、最小の値ちょうどは通り、それより近いと断られる。
    #[test]
    fn canvas_validation_accepts_the_exact_minimum_separation() {
        let line = |b: DVec2| Ruler::canvas(id(7), RulerKind::Line, DVec2::ZERO, b);
        let exact = DVec2::X * MIN_CANVAS_SEPARATION;
        assert_eq!(
            DVec2::ZERO.distance(exact),
            MIN_CANVAS_SEPARATION,
            "試験の前提"
        );
        assert!(line(exact).validate().is_ok(), "ちょうど最小の間隔");
        assert!(line(exact * 0.9).validate().is_err(), "最小より近い");
    }

    #[test]
    fn normalizing_up_keeps_a_unit_vector_and_is_stable() {
        let r = Ruler::model(
            id(1),
            RulerKind::Line,
            DVec3::ZERO,
            DVec3::X,
            DVec3::new(0.0, 5.0, 0.0),
        );
        let n = r.clone().normalized();
        assert!(matches!(n.place, RulerPlace::Model { up, .. } if up == DVec3::Y));
        assert_eq!(n.clone().normalized(), n, "二度通しても同じ");
        let odd = Ruler::model(
            id(1),
            RulerKind::Line,
            DVec3::ZERO,
            DVec3::X,
            DVec3::new(1.0, 2.0, 3.0),
        )
        .normalized();
        assert_eq!(odd.clone().normalized(), odd);
    }

    #[test]
    fn a_2d_symmetry_ruler_becomes_a_canvas_symmetry() {
        let c = DVec2::new(40.0, 50.0);
        let sym = |lines: u8, toward: DVec2| {
            let mut r = mirror(c, c + toward);
            r.lines = lines;
            r.canvas_symmetry().unwrap()
        };
        // 今の縦・横・両方と同じ写しになる向きは、そのモードで返す（写しの並びまで今と同じ）
        for (lines, toward, old) in [
            (2u8, DVec2::new(0.0, 7.0), SymmetryMode::Vertical),
            (2, DVec2::new(0.0, -7.0), SymmetryMode::Vertical),
            (2, DVec2::new(5.0, 0.0), SymmetryMode::Horizontal),
            (2, DVec2::new(-5.0, 0.0), SymmetryMode::Horizontal),
            (4, DVec2::new(5.0, 0.0), SymmetryMode::Both),
            (4, DVec2::new(-5.0, 0.0), SymmetryMode::Both),
            (4, DVec2::new(0.0, 3.0), SymmetryMode::Both),
            (4, DVec2::new(0.0, -3.0), SymmetryMode::Both),
        ] {
            let got = sym(lines, toward);
            let want = CanvasSymmetry::new(old, c, 2).unwrap();
            assert_eq!(got, want, "{lines} 本 {toward:?}");
            assert_eq!(got.transforms().unwrap(), want.transforms().unwrap());
        }
        // そうでない向きと本数は線対称（最初の線の角度は度）
        let diagonal = sym(2, DVec2::new(4.0, 4.0));
        assert_eq!(diagonal.mode, SymmetryMode::Lines);
        assert_eq!(
            (diagonal.center, diagonal.count, diagonal.angle),
            (c, 2, 45.0)
        );
        for (lines, toward, angle) in [
            (4u8, DVec2::new(4.0, 4.0), 45.0),
            (6, DVec2::new(5.0, 0.0), 0.0),
            (6, DVec2::new(0.0, 5.0), 90.0),
            (8, DVec2::new(0.0, -5.0), -90.0),
            (16, DVec2::new(-5.0, 0.0), 180.0),
        ] {
            let got = sym(lines, toward);
            assert_eq!(got.mode, SymmetryMode::Lines, "{lines} 本 {toward:?}");
            assert_eq!(
                (got.center, got.count, got.angle),
                (c, u32::from(lines), angle)
            );
        }
        let oblique = sym(2, DVec2::new(1.0, 2.0));
        assert_eq!(oblique.mode, SymmetryMode::Lines);
        assert!((oblique.angle - 63.434_948_822_922_01).abs() < 1e-12);
        // 回転対称は放射状（角度は 0）
        let mut r = mirror(c, c + DVec2::new(3.0, 4.0));
        r.line_symmetry = false;
        r.lines = 6;
        let radial = r.canvas_symmetry().unwrap();
        assert_eq!(radial.mode, SymmetryMode::Radial);
        assert_eq!((radial.center, radial.count, radial.angle), (c, 6, 0.0));
        // 3D の定規と、対称でない定規は None
        assert!(Ruler::canvas(id(1), RulerKind::Line, c, c + DVec2::X)
            .canvas_symmetry()
            .is_none());
        assert!(
            Ruler::model(id(1), RulerKind::Symmetry, DVec3::ZERO, DVec3::X, DVec3::Y)
                .canvas_symmetry()
                .is_none()
        );
    }

    /// 3D の写し（鏡映を先に、軸のまわりの回転を後）の全部の像（元の点を含む）。
    fn images(setup: &SurfaceSymmetrySetup, p: Vec3) -> Vec<Vec3> {
        let mirrored: Vec<Vec3> = match &setup.mirror {
            Some(m) => vec![p, m.reflect(p)],
            None => vec![p],
        };
        let turns = setup.radial.map_or(1, |r| r.count);
        let mut out = Vec::new();
        for q in mirrored {
            for k in 0..turns {
                out.push(match &setup.radial {
                    Some(r) => r.rotate_point(q, k),
                    None => q,
                });
            }
        }
        out
    }

    fn same_points(a: &[Vec3], b: &[Vec3]) -> bool {
        a.len() == b.len()
            && a.iter()
                .all(|p| b.iter().any(|q| (*p - *q).length() < 1e-4))
    }

    /// 設計の確かめ（dihedral.py）を Rust で: 3D の対称定規（鏡 1 枚＋軸のまわりの回転）の像は、同じ定規を 2D の線対称・回転対称
    /// 〔軸に直交する面の座標（最初の線の向き d、up × d）で、角度 0 度〕で写した像と、点の集合として同じ（線対称 N 本は二面体群の N 個）。
    #[test]
    fn a_3d_symmetry_ruler_images_equal_the_2d_dihedral_group_in_the_plane_around_the_axis() {
        let a = DVec3::new(1.0, 2.0, 3.0);
        // (軸の向き, b − a)。軸が Y で最初の線が軸に直交する例と、軸が傾いて b − a が軸の向きの成分も持つ例
        let mut cases: Vec<(DVec3, DVec3)> = [0.0f64, 0.4, 1.9, -2.5]
            .into_iter()
            .map(|tilt| (DVec3::Y, DVec3::new(tilt.cos(), 0.0, tilt.sin()) * 2.0))
            .collect();
        cases.push((DVec3::new(0.3, 0.9, -0.2), DVec3::new(1.0, 0.4, 0.7)));
        cases.push((DVec3::new(-1.0, 0.5, 2.0), DVec3::new(0.2, -1.5, 0.9)));
        for (up_input, offset) in cases {
            let up = up_input.normalize();
            // 軸に直交する面の座標: e1 = b − a から軸の成分を除いた単位の向き、e2 = up × e1
            let e1 = (offset - up * offset.dot(up)).normalize();
            let e2 = up.cross(e1);
            for lines in 2..=16u8 {
                for line_symmetry in [true, false] {
                    if line_symmetry && lines % 2 != 0 {
                        continue;
                    }
                    let mut r = Ruler::model(id(5), RulerKind::Symmetry, a, a + offset, up_input);
                    r.lines = lines;
                    r.line_symmetry = line_symmetry;
                    let setup = r.surface_symmetry().unwrap();
                    let planar = if line_symmetry {
                        CanvasSymmetry::lines(DVec2::ZERO, u32::from(lines), 0.0).unwrap()
                    } else {
                        CanvasSymmetry::new(SymmetryMode::Radial, DVec2::ZERO, u32::from(lines))
                            .unwrap()
                    };
                    // 軸の上の高さ 0.7、軸からの (u, v) = (1.3, 0.4) の点
                    let (u, v, h) = (1.3f64, 0.4f64, 0.7f64);
                    let p = a + e1 * u + e2 * v + up * h;
                    let got = images(&setup, p.as_vec3());
                    let want: Vec<Vec3> = planar
                        .transforms()
                        .unwrap()
                        .iter()
                        .map(|t| {
                            let (u2, v2) = t.map(u, v);
                            (a + e1 * u2 + e2 * v2 + up * h).as_vec3()
                        })
                        .collect();
                    assert_eq!(got.len(), usize::from(lines), "写しは線の本数と同じ数");
                    assert!(
                        same_points(&got, &want),
                        "{lines} 本 線対称={line_symmetry} 軸 {up_input:?} b−a {offset:?}: {got:?} と {want:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_3d_symmetry_ruler_becomes_a_mirror_and_a_rotation() {
        let make = |lines: u8, line_symmetry: bool| {
            let mut r = Ruler::model(
                id(3),
                RulerKind::Symmetry,
                DVec3::new(1.0, 2.0, 3.0),
                DVec3::new(2.0, 2.0, 3.0),
                DVec3::Y,
            );
            r.lines = lines;
            r.line_symmetry = line_symmetry;
            r.see_through = true;
            r.surface_symmetry().unwrap()
        };
        // 線対称 2 本: 鏡 1 枚（軸 Y と向き X を含む面 = 法線 Y × X = −Z）、回転なし
        let two = make(2, true);
        let plane = two.mirror.unwrap();
        assert_eq!(plane.point, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(plane.normal, -Vec3::Z);
        assert!(two.radial.is_none() && two.ignore_visibility);
        // 線対称 6 本: 鏡 1 枚と回転 3 個
        let six = make(6, true);
        assert!(six.mirror.is_some());
        assert_eq!(six.radial.unwrap().count, 3);
        assert_eq!(six.radial.unwrap().axis, Vec3::Y);
        // 回転対称 5 本: 回転 5 個だけ
        let five = make(5, false);
        assert!(five.mirror.is_none());
        assert_eq!(five.radial.unwrap().count, 5);
        // 2D の定規は 3D の写しを持たない
        assert!(mirror(DVec2::ZERO, DVec2::X).surface_symmetry().is_none());
    }
}
