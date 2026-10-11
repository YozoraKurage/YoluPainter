//! 投影の塗り（3D のストロークの元の側と、見える面だけの対称の写し）。
//!
//! ストロークの始めに、カメラの前の全ての三角形を画面へ写し（近い面で切る）、画面を区画に分けて、区画ごとに重なる三角形の一覧を作る。
//! ブラシの円が区画に初めて触れたときだけ、その区画の「投影の画素」を作って覚える: 塗るテクスチャセットの三角形の UV の中のテクセルの
//! 中心を、UV の重心座標で 3D の点へ戻して画面へ写し（遠近が正しい）、区画の中に落ちたものを残す。ダブは、円に重なる区画の投影の画素
//! だけを回して覆いを出す。
//!
//! - 遮蔽: 区画の中の三角形（向き・テクスチャセット・レンダラーによらず、カメラの前の全部）に対する、画面の点と三角形の 2D の判定と、
//!   その点での奥行き（三角形の平面とカメラのレイの交わり）の比べ。区画を小さな升に分け、升ごとに重なる三角形を手前から並べて、
//!   テクセルより奥の三角形に来たら打ち切る。BVH のレイは使わない。
//! - 面の向き: 裏の面（法線がカメラを向かない）は、切り替えが無ければ塗らない。法線と視線の cos で弱める（始まりの角度で 1、
//!   終わりの角度で 0、その間は cos に線形）。裏の面を塗るときは裏から見た向き。
//! - 継ぎ目のにじみ: UV アイランドの縁の辺（隣の三角形と UV が続かない辺・隣の無い辺）から外へ N テクセルの距離の中の、塗るセットのどの
//!   三角形の UV にも入らないテクセルを、辺の上のいちばん近い点で塗る（画面の位置・遮蔽・向き・覆いはその点のもの）。
//! - 対称の写し: 写したカメラ（鏡映・回転したカメラ）から同じ画面の円で投影すると、元のカメラから元の側を見たのと同じ写しの側が
//!   出る。そのうち、元のカメラからも見えるテクセルだけを残す（元のカメラの区画の遮るもので確かめる）。
//! - 決定的: 区画の投影の画素は、区画と三角形と設定だけで決まる（作る順・並列の度合い・捨てて作り直したかによらない）。ダブでは同じ
//!   テクセルを、覆いの大きいほう（同じなら三角形の番号の小さいほう、続けて辺の番号）の 1 つにし、下の行から並べる。
//! - 覚え: 区画の投影の画素は、呼び手の決めたバイトに収める（長く使っていない区画から捨て、要るときに同じものを作り直す）。元の側と
//!   写しの投影の塗りは 1 つの予算を分け、どれの区画でも古いものから捨てる（[`evict_least_recent`]）。カメラによらない一覧（塗るセットの
//!   UV が覆うテクセル・アイランドの縁の辺）はスナップショットに覚えて、次のストロークでも使う。
//! - 計算は倍精度（頂点はモデルの単精度の値から）の四則と平方根だけ（画角の tan と弱めの cos も四則で作る）。例外は放射状の写しの
//!   カメラの回転で、[`RadialSymmetry`] の単精度の sin・cos（機械の数学の関数）を使う。
//! - 正投影のカメラ（[`super::camera::Projection::Orthographic`]）: 画面への写しは奥行きで割らず、視線は前の向きに平行。面の表裏は法線と
//!   前の向き、面の向きの弱めは法線と前の向きの cos、辺の上の割合は画面の割合のまま（奥行きで歪まない）。透視の式は変えない。

use std::cell::RefCell;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use glam::{DVec2, DVec3, Vec2, Vec3};
use rayon::prelude::*;

use super::build::{position_key, FastMap};
use super::camera::CameraView;
use super::dab::{DabRefusal, SurfaceDabResult, SurfacePixel};
use super::query::coverage;
use super::symmetry::{MirrorPlane, RadialSymmetry};
use super::unity::{clamp01, finite2};
use super::{SurfaceGeometry, SurfaceHit};
use crate::brush::edge::{self, TexelMetric};
use crate::brush::profile;
use crate::brush::AntiAlias;

/// 投影の塗りの切り替え（ストロークの始めに固める）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProjectionSettings {
    /// 手前の面に隠れた所も塗る（遮蔽を見ない）。
    pub paint_hidden: bool,
    /// 裏の面（法線がカメラを向かない面）も塗る。
    pub paint_backfaces: bool,
    /// 面の向き（法線と視線の角度）で弱める。
    pub angle_falloff: bool,
    /// 弱め始める角度（度。0〜90）。
    pub angle_start: f32,
    /// 塗らなくなる角度（度。始まり〜90）。
    pub angle_end: f32,
    /// 継ぎ目のにじみ（UV アイランドの縁から外へのテクセルの数。0〜[`MAX_SEAM_BLEED`]）。
    pub seam_bleed: u32,
}

/// 継ぎ目のにじみの上限（テクセル）。
pub const MAX_SEAM_BLEED: u32 = 16;

impl Default for ProjectionSettings {
    fn default() -> Self {
        ProjectionSettings {
            paint_hidden: false,
            paint_backfaces: false,
            angle_falloff: true,
            angle_start: 80.0,
            angle_end: 85.0,
            seam_bleed: 2,
        }
    }
}

impl ProjectionSettings {
    /// 範囲の外の値を丸めたもの（有限でない角度は既定の値。終わりは始まり以上）。
    pub fn sanitized(self) -> ProjectionSettings {
        let d = ProjectionSettings::default();
        let angle = |v: f32, fallback: f32| {
            if v.is_finite() {
                v.clamp(0.0, 90.0)
            } else {
                fallback
            }
        };
        let start = angle(self.angle_start, d.angle_start);
        let end = angle(self.angle_end, d.angle_end).max(start);
        ProjectionSettings {
            angle_start: start,
            angle_end: end,
            seam_bleed: self.seam_bleed.min(MAX_SEAM_BLEED),
            ..self
        }
    }
}

/// 覆いの集めと、候補の並べ替えを、ワーカーで行う下限（区画の投影の画素の数 / 覆いのある候補の数）。合成の球（202,800 三角形）・文書 2048²・
/// 大きさ 256（候補が約 6.4 万、区画の投影の画素が約 11 万）でダブ 1 つの中央値を測ると、直列は 8 スレッドの分け合いと同じか速く
/// （8 スレッド 4.0 → 3.7 ms）、32 スレッドでは分け合いが遅かった（4.9 → 3.6 ms）。分け合いが負ける理由は測っていない（ワーカーを起こす遅れが
/// 疑わしい）。直列のほうが速い範囲より十分大きいダブだけをワーカーで行う。結果は変わらない。
const PARALLEL_CANDIDATES_DEFAULT: usize = 1 << 18;
static PARALLEL_CANDIDATES: AtomicUsize = AtomicUsize::new(PARALLEL_CANDIDATES_DEFAULT);

/// 試験のための口: 覆いの集めと候補の並べ替えをワーカーで行う下限を変え、前の値を返す（小さなダブでもワーカーの経路を通すために使う。
/// どの値でも結果は同じで、経路の選び方だけが変わる）。
#[doc(hidden)]
pub fn set_parallel_projection_candidates(candidates: usize) -> usize {
    PARALLEL_CANDIDATES.swap(candidates, Ordering::Relaxed)
}

/// 区画の大きさの範囲（画面の単位）。ブラシの直径の 1/4 を、この範囲に収める。
pub const MIN_BUCKET: f32 = 8.0;
pub const MAX_BUCKET: f32 = 256.0;
/// 遮蔽の升の大きさ（画面の単位）。
const CELL: f64 = 8.0;
/// UV の内外の判定の許し（重心座標）。
const INSIDE_EPSILON: f64 = 1e-7;
/// 1 つの区画の名目の固定のバイト（一覧の項目と管理）。
const BUCKET_OVERHEAD: u64 = 64;

/// 画面の上のダブの形: 投影の画素ごとの覆い（面の向きの弱めの前）。投影の画素はワーカーからも読むので Sync。
pub(crate) trait ScreenCover: Sync {
    /// 区画を選ぶ円の半径（画面の単位）。この円の外の投影の画素の覆いは 0 でなければならない。
    fn reach(&self) -> f64;
    /// 中心からの画面のずれ (dx, dy)（y は下向き）の所にある、テクセル (x, y) の覆い（0 以下は塗らない）。`metric` はそのテクセルの
    /// 1 つ分が画面でどれだけか（[`TexelMetric`] の xx・xy・yy。y は下向き）。
    fn cover(&self, dx: f64, dy: f64, x: u16, y: u16, metric: [f32; 3]) -> f32;
}

/// 画面の形の覆い（[`SurfaceProjector::sweep`]）: 中心からの画面のずれ (dx, dy)（y は下向き）の所にある投影の画素の覆い（0 以下は
/// 塗らない）。`columns` はそのテクセルの x・y の 1 つ分の画面の動き（[jx.x, jx.y, jy.x, jy.y]。持たない投影の塗りでは 0）。
pub(crate) trait AreaCover: Sync {
    fn cover(&self, dx: f64, dy: f64, columns: [f32; 4]) -> f32;
}

/// 丸い筆先（3D のストロークの今までの式: 中心からの距離を半径で割った値の硬さの smoothstep。円の上と外は 0）。アンチエイリアスの
/// 段があれば、縁の帯（テクセル）を投影の画素ごとのテクセルの画面の大きさで画面の距離へ直す（[`crate::brush::AntiAlias`]）。
#[derive(Clone, Copy, Debug)]
pub(crate) struct RoundCover {
    radius: f64,
    radius2: f64,
    hardness: f32,
    aa: Option<RoundAa>,
}

/// 丸い筆先のアンチエイリアスの、ダブで 1 つの値。
#[derive(Clone, Copy, Debug)]
struct RoundAa {
    /// 段の帯の幅（テクセル）。
    band: f64,
    /// 届く半径（画面）と、その中に収まる帯の上限（規格化した距離の単位）。
    reach: f64,
    cap: f64,
}

impl RoundCover {
    pub(crate) fn hardness(&self) -> f32 {
        self.hardness
    }

    pub(crate) fn new(radius: f32, hardness: f32) -> RoundCover {
        let r = radius as f64;
        RoundCover {
            radius: r,
            radius2: r * r,
            hardness: clamp01(hardness),
            aa: None,
        }
    }

    /// アンチエイリアスの段 level を付ける。`center` はダブの中心のテクセルの画面の大きさ（届く半径の見積もり: 帯がその 2 倍の
    /// 大きさのテクセルでも収まるように取り、それより大きく写るテクセルでは帯をその半径に収まる幅で止める）。中心の値が無い・
    /// 使えないなら今の式のまま。
    pub(crate) fn with_anti_alias(
        mut self,
        level: AntiAlias,
        center: Option<TexelMetric>,
    ) -> RoundCover {
        let w = level.band();
        let Some(m) = center.filter(|m| w > 0.0 && m.usable() && self.radius > 0.0) else {
            return self;
        };
        let h = self.hardness as f64;
        let longest = m.eigen().0.sqrt();
        let estimate = 2.0 * w.max(1.0) * longest / self.radius;
        let (_, outer) = edge::bounds(h, estimate);
        let reach = self.radius * outer;
        self.aa = Some(RoundAa {
            band: w,
            reach,
            cap: edge::band_for_outer(h, outer),
        });
        self
    }
}

impl ScreenCover for RoundCover {
    fn reach(&self) -> f64 {
        match &self.aa {
            Some(aa) => aa.reach,
            None => self.radius,
        }
    }
    #[inline]
    fn cover(&self, dx: f64, dy: f64, _x: u16, _y: u16, metric: [f32; 3]) -> f32 {
        let d2 = dx * dx + dy * dy;
        if let Some(aa) = &self.aa {
            if d2 >= aa.reach * aa.reach {
                return 0.0;
            }
            let m = TexelMetric {
                xx: metric[0] as f64,
                xy: metric[1] as f64,
                yy: metric[2] as f64,
            };
            if m.usable() {
                let h = self.hardness as f64;
                let dist = d2.sqrt();
                let (big, small) = m.eigen();
                // 画面の円のテクセルの空間での半径（テクセルが画面で短く写る向きほど、テクセルの数は多い）
                let e = edge::band_and_density(
                    aa.band,
                    h,
                    self.radius / small.sqrt(),
                    self.radius / big.sqrt(),
                );
                // 規格化した距離のテクセルあたりの勾配（中心は、テクセルがいちばん長く写る向きの値）
                let g = if dist > 0.0 {
                    m.gradient(dx / dist, dy / dist) / self.radius
                } else {
                    big.sqrt() / self.radius
                };
                let band = (e.band * g).min(aa.cap);
                if e.density != 1.0 || band > 1.0 - h {
                    return edge::cover64(dist / self.radius, h, band, e.density) as f32;
                }
            }
        }
        if d2 >= self.radius2 {
            return 0.0;
        }
        coverage((d2.sqrt() / self.radius) as f32, self.hardness)
    }
}

/// 投影の画素 1 つ（区画に覚える）。
#[derive(Clone, Copy, Debug)]
struct ProjPixel {
    x: u16,
    y: u16,
    face: u32,
    /// 画面の位置。
    sx: f32,
    sy: f32,
    /// 面の向きの弱め（0〜1）。
    weight: f32,
    /// テクセルの点（にじみは辺の上の点。モデルの空間）。
    position: Vec3,
    /// 0 は三角形の中のテクセル、1〜3 はにじみの辺（同じ覆い・同じ三角形のときの並びに使う）。
    rank: u8,
}

/// 区画 1 つの投影の画素と、アンチエイリアスのある投影の塗りでは、投影の画素ごとのテクセル 1 つ分の画面の大きさ
/// （[`TexelMetric`] の xx・xy・yy。y は下向き。並びは `pixels` と同じ。無い投影の塗りでは空）。画面の形を集める投影の塗りでは、
/// 投影の画素ごとのテクセルの x・y の 1 つ分の画面の動き（テクセル → 画面の写しの列。y は下向き。無ければ空）。
#[derive(Debug, Default)]
struct Bucket {
    pixels: Vec<ProjPixel>,
    metrics: Vec<[f32; 3]>,
    columns: Vec<[f32; 4]>,
}

impl Bucket {
    /// 下の行から（同じ位置は三角形の番号・辺の番号の順に）並べ替えたもの。テクセルの位置・三角形・辺で順が決まるので、作る順や
    /// 並列の度合いによらない。ダブが複数の区画の候補を位置の順に併合できるようにする。
    fn sorted(self) -> Bucket {
        let n = self.pixels.len();
        let mut order: Vec<u32> = (0..n as u32).collect();
        order.sort_unstable_by_key(|&i| {
            let p = &self.pixels[i as usize];
            (p.y, p.x, p.face, p.rank)
        });
        if order.iter().enumerate().all(|(k, &i)| k == i as usize) {
            return self;
        }
        Bucket {
            pixels: order.iter().map(|&i| self.pixels[i as usize]).collect(),
            metrics: if self.metrics.is_empty() {
                Vec::new()
            } else {
                order.iter().map(|&i| self.metrics[i as usize]).collect()
            },
            columns: if self.columns.is_empty() {
                Vec::new()
            } else {
                order.iter().map(|&i| self.columns[i as usize]).collect()
            },
        }
    }
}

/// 画面の形の覆いを測った投影の画素 1 つ（[`SurfaceProjector::sweep`] が渡す。覆いは 0 より大きい）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SweepPixel {
    pub x: u16,
    pub y: u16,
    pub coverage: f32,
    /// 画面の位置（にじみのテクセルは、塗る元の辺の上の点の位置）。
    pub screen: Vec2,
}

/// [`SweepPixel`] 1 つの名目のバイト。
pub(crate) const SWEEP_PIXEL_BYTES: u64 = std::mem::size_of::<SweepPixel>() as u64;

/// 投影の画素 1 つの名目のバイト。
const PIXEL_BYTES: u64 = std::mem::size_of::<ProjPixel>() as u64;
/// アンチエイリアスのある投影の塗りで、投影の画素 1 つに加わるバイト（テクセルの画面の大きさ）。
const METRIC_BYTES: u64 = std::mem::size_of::<[f32; 3]>() as u64;
/// テクセルの画面の動きを持つ投影の塗りで、投影の画素 1 つに加わるバイト。
const COLUMN_BYTES: u64 = std::mem::size_of::<[f32; 4]>() as u64;

/// 覚えの数（試験と計測用）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProjectionStats {
    /// 作った区画の延べ数（捨てて作り直したものも数える）。
    pub buckets_built: u64,
    /// 覚えていた区画を使った延べ数。
    pub bucket_reuses: u64,
    /// 予算のために捨てた区画の延べ数。
    pub evictions: u64,
    /// 区画が予算に入らずに飛ばしたダブの数。
    pub skipped_dabs: u64,
    /// 今覚えている区画の数とバイト。
    pub cached_buckets: usize,
    pub cached_bytes: u64,
    /// 覚えたバイトのいちばん大きかった値（ダブの後）。
    pub peak_bytes: u64,
    /// 1 つのダブに要った区画のバイトのいちばん大きかった値。
    pub largest_dab_bytes: u64,
}

/// 対称の写しの側を塗るときの、カメラの写し方（鏡映を先に、回転を後に。写しの側の点 P は、元の側の点を写したもの）。
/// 写したカメラから写しの側を見ると、元のカメラから元の側を見たのと同じに見えるので、同じ画面の円で塗る。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CopyTransform {
    pub mirror: Option<MirrorPlane>,
    /// 放射状と、その何番目の写しか。
    pub radial: Option<(RadialSymmetry, u32)>,
}

impl CopyTransform {
    fn point(&self, p: DVec3) -> DVec3 {
        let mut p = p;
        if let Some(m) = &self.mirror {
            let n = m.normal.as_dvec3();
            p -= n * (2.0 * (p - m.point.as_dvec3()).dot(n));
        }
        if let Some((r, copy)) = &self.radial {
            let o = r.origin.as_dvec3();
            p = o + r.rotation(*copy).as_dquat() * (p - o);
        }
        p
    }

    /// 写しの側の点を元の側へ戻す（回転を戻してから鏡映）。
    pub(crate) fn inverse_point(&self, p: DVec3) -> DVec3 {
        let mut p = p;
        if let Some((r, copy)) = &self.radial {
            let o = r.origin.as_dvec3();
            p = o + r.rotation(*copy).as_dquat().inverse() * (p - o);
        }
        if let Some(m) = &self.mirror {
            let n = m.normal.as_dvec3();
            p -= n * (2.0 * (p - m.point.as_dvec3()).dot(n));
        }
        p
    }

    fn direction(&self, d: DVec3) -> DVec3 {
        let mut d = d;
        if let Some(m) = &self.mirror {
            let n = m.normal.as_dvec3();
            d -= n * (2.0 * d.dot(n));
        }
        if let Some((r, copy)) = &self.radial {
            d = r.rotation(*copy).as_dquat() * d;
        }
        d
    }
}

/// カメラ（倍精度。ビューの空間は x が右・y が上・z が前、カメラが原点）。
#[derive(Clone, Copy, Debug)]
struct Frame {
    eye: DVec3,
    right: DVec3,
    up: DVec3,
    forward: DVec3,
    /// 鏡映したカメラ（軸が左右逆の組）。三角形の巻きの向きが逆になるので、表裏の判定を裏返す。
    flip: bool,
    /// tan(画角/2) × 縦横比、tan(画角/2)。
    tx: f64,
    ty: f64,
    near: f64,
    sw: f64,
    sh: f64,
    /// 正投影の、横と縦の見える長さの半分（ビューの空間）。透視なら None。
    ortho: Option<(f64, f64)>,
}

impl Frame {
    fn new(view: &CameraView, transform: &CopyTransform) -> Frame {
        let q = view.rotation.as_dquat();
        // 縦の画角 30° の tan(15°) = 2 − √3（平方根は丸めまで決まっているので、機械の数学の関数によらない）
        let ty = if super::camera::FIELD_OF_VIEW == 30.0 {
            2.0 - 3.0f64.sqrt()
        } else {
            (super::camera::FIELD_OF_VIEW as f64 * 0.5)
                .to_radians()
                .tan()
        };
        let (sw, sh) = (view.width as f64, view.height as f64);
        Frame {
            eye: transform.point(view.position.as_dvec3()),
            right: transform.direction(q * DVec3::X),
            up: transform.direction(q * DVec3::Y),
            forward: transform.direction(q * DVec3::Z),
            flip: transform.mirror.is_some(),
            tx: ty * (sw / sh),
            ty,
            near: view.near as f64,
            sw,
            sh,
            ortho: match view.projection {
                super::camera::Projection::Perspective => None,
                super::camera::Projection::Orthographic { height } => {
                    let hy = height as f64 * 0.5;
                    Some((hy * (sw / sh), hy))
                }
            },
        }
    }

    #[inline]
    fn view_of(&self, p: Vec3) -> DVec3 {
        let d = p.as_dvec3() - self.eye;
        DVec3::new(d.dot(self.right), d.dot(self.up), d.dot(self.forward))
    }

    #[inline]
    fn screen_of(&self, v: DVec3) -> DVec2 {
        if let Some((hx, hy)) = self.ortho {
            return DVec2::new(
                (v.x / hx + 1.0) * 0.5 * self.sw,
                (1.0 - v.y / hy) * 0.5 * self.sh,
            );
        }
        DVec2::new(
            (v.x / (v.z * self.tx) + 1.0) * 0.5 * self.sw,
            (1.0 - v.y / (v.z * self.ty)) * 0.5 * self.sh,
        )
    }

    /// 画面の点を通る視線と、ビューの空間の平面 normal · p = k の交わり（平面が視線に沿うなら None）。透視の視線は原点から
    /// 向き (x·tx, y·ty, 1)、正投影の視線は (x·hx, y·hy, 0) から前の向き。
    #[inline]
    fn plane_point(&self, s: DVec2, normal: DVec3, k: f64) -> Option<DVec3> {
        if let Some((hx, hy)) = self.ortho {
            let o = DVec3::new(
                (2.0 * s.x / self.sw - 1.0) * hx,
                (1.0 - 2.0 * s.y / self.sh) * hy,
                0.0,
            );
            if normal.z == 0.0 {
                return None;
            }
            return Some(DVec3::new(o.x, o.y, (k - normal.dot(o)) / normal.z));
        }
        let d = DVec3::new(
            (2.0 * s.x / self.sw - 1.0) * self.tx,
            (1.0 - 2.0 * s.y / self.sh) * self.ty,
            1.0,
        );
        let denom = normal.dot(d);
        if denom == 0.0 {
            return None;
        }
        Some(d * (k / denom))
    }

    /// ビューの空間の平面 normal · p = k の三角形が、見る人の側を向くか（鏡映した組の裏返しの前の判定。透視は原点が平面の表の側、
    /// 正投影は法線が前の向きに逆らう）。
    #[inline]
    fn faces_viewer(&self, normal: DVec3, k: f64) -> bool {
        match self.ortho {
            None => k < 0.0,
            Some(_) => normal.z < 0.0,
        }
    }

    /// 単位の法線 unit と、ビューの空間の点 p から見る人への向きの cos の絶対値（点が原点なら None）。
    #[inline]
    fn view_cos(&self, unit: DVec3, p: DVec3) -> Option<f64> {
        if self.ortho.is_some() {
            return Some(unit.z.abs());
        }
        let length = p.length();
        if length.is_nan() || length <= 0.0 {
            return None;
        }
        Some(unit.dot(p).abs() / length)
    }
}

/// 画面へ写した三角形（近い面で切った凸多角形）。
#[derive(Clone, Copy, Debug)]
struct ScreenFace {
    face: u32,
    poly: [DVec2; 4],
    n: usize,
    /// 多角形の向き（辺の外積の符号をそろえる）。
    sign: f64,
    min: DVec2,
    max: DVec2,
    /// ビューの空間の平面 normal · p = k。
    normal: DVec3,
    k: f64,
    /// 近い面で切った頂点のいちばん近い奥行き。
    min_depth: f64,
}

impl ScreenFace {
    /// 三角形を画面へ写す（カメラの前に無い・画面で潰れる三角形は None）。
    fn new(frame: &Frame, face: u32, v: [DVec3; 3]) -> Option<ScreenFace> {
        let normal = (v[1] - v[0]).cross(v[2] - v[0]);
        if normal.length_squared().is_nan() || normal.length_squared() <= 0.0 {
            return None;
        }
        let k = normal.dot(v[0]);
        // 近い面（z = near）で切る
        let mut clipped = [DVec3::ZERO; 4];
        let mut n = 0;
        for i in 0..3 {
            let a = v[i];
            let b = v[(i + 1) % 3];
            let a_in = a.z >= frame.near;
            let b_in = b.z >= frame.near;
            if a_in {
                clipped[n] = a;
                n += 1;
            }
            if a_in != b_in {
                let t = (frame.near - a.z) / (b.z - a.z);
                let mut p = a + (b - a) * t;
                p.z = frame.near;
                clipped[n] = p;
                n += 1;
            }
        }
        if n < 3 {
            return None;
        }
        let mut poly = [DVec2::ZERO; 4];
        let mut min = DVec2::splat(f64::INFINITY);
        let mut max = DVec2::splat(f64::NEG_INFINITY);
        let mut min_depth = f64::INFINITY;
        for i in 0..n {
            let s = frame.screen_of(clipped[i]);
            if !s.is_finite() {
                return None;
            }
            poly[i] = s;
            min = min.min(s);
            max = max.max(s);
            min_depth = min_depth.min(clipped[i].z);
        }
        let mut area = 0.0;
        for i in 0..n {
            let (a, b) = (poly[i], poly[(i + 1) % n]);
            area += a.x * b.y - b.x * a.y;
        }
        if area.is_nan() || area.abs() <= 1e-18 {
            return None;
        }
        Some(ScreenFace {
            face,
            poly,
            n,
            sign: area.signum(),
            min,
            max,
            normal,
            k,
            min_depth,
        })
    }

    /// 画面の点が多角形の中（辺の上を含む）か。
    #[inline]
    fn contains(&self, s: DVec2) -> bool {
        if s.x < self.min.x || s.x > self.max.x || s.y < self.min.y || s.y > self.max.y {
            return false;
        }
        for i in 0..self.n {
            let a = self.poly[i];
            let b = self.poly[(i + 1) % self.n];
            let e = (b.x - a.x) * (s.y - a.y) - (b.y - a.y) * (s.x - a.x);
            if e * self.sign < 0.0 {
                return false;
            }
        }
        true
    }

    /// 画面の点での奥行き（その点のレイと平面の交わり。平面がレイに沿うなら None）。
    #[inline]
    fn depth_at(&self, frame: &Frame, s: DVec2) -> Option<f64> {
        let z = frame.plane_point(s, self.normal, self.k)?.z;
        z.is_finite().then_some(z)
    }

    /// 多角形を閉じた長方形で切った多角形（空なら長さ 0）。
    fn clip_to_rect(&self, lo: DVec2, hi: DVec2, out: &mut Vec<DVec2>) {
        out.clear();
        out.extend_from_slice(&self.poly[..self.n]);
        let mut scratch: Vec<DVec2> = Vec::with_capacity(8);
        // 4 本の半平面: x >= lo.x、x <= hi.x、y >= lo.y、y <= hi.y
        for side in 0..4 {
            scratch.clear();
            let value = |p: DVec2| -> f64 {
                match side {
                    0 => p.x - lo.x,
                    1 => hi.x - p.x,
                    2 => p.y - lo.y,
                    _ => hi.y - p.y,
                }
            };
            let m = out.len();
            for i in 0..m {
                let a = out[i];
                let b = out[(i + 1) % m];
                let (va, vb) = (value(a), value(b));
                if va >= 0.0 {
                    scratch.push(a);
                }
                if (va >= 0.0) != (vb >= 0.0) {
                    let t = va / (va - vb);
                    scratch.push(a + (b - a) * t);
                }
            }
            std::mem::swap(out, &mut scratch);
            if out.is_empty() {
                return;
            }
        }
    }
}

/// UV のテクセルの空間（画素の単位）での三角形と、重心座標への係数。
#[derive(Clone, Copy, Debug)]
struct UvFace {
    p0: DVec2,
    p1: DVec2,
    p2: DVec2,
    c11: f64,
    c12: f64,
    c21: f64,
    c22: f64,
}

impl UvFace {
    fn new(t: &super::SurfaceTriangle, width: i32, height: i32) -> Option<UvFace> {
        let scale = DVec2::new(width as f64, height as f64);
        let p0 = t.uv_a.as_dvec2() * scale;
        let p1 = t.uv_b.as_dvec2() * scale;
        let p2 = t.uv_c.as_dvec2() * scale;
        if !(p0.is_finite() && p1.is_finite() && p2.is_finite()) {
            return None;
        }
        let (e1, e2) = (p1 - p0, p2 - p0);
        let det = e1.x * e2.y - e1.y * e2.x;
        if det.is_nan() || det.abs() <= 1e-12 {
            return None;
        }
        Some(UvFace {
            p0,
            p1,
            p2,
            c11: e2.y / det,
            c12: -e2.x / det,
            c21: -e1.y / det,
            c22: e1.x / det,
        })
    }

    /// テクセル (x, y) の中心の重心座標（A・B・C）。
    #[inline]
    fn bary(&self, x: i32, y: i32) -> DVec3 {
        let dx = (x as f64 + 0.5) - self.p0.x;
        let dy = (y as f64 + 0.5) - self.p0.y;
        let b1 = self.c11 * dx + self.c12 * dy;
        let b2 = self.c21 * dx + self.c22 * dy;
        DVec3::new(1.0 - b1 - b2, b1, b2)
    }

    #[inline]
    fn inside(b: DVec3) -> bool {
        b.x >= -INSIDE_EPSILON && b.y >= -INSIDE_EPSILON && b.z >= -INSIDE_EPSILON
    }

    fn point(&self, b: DVec3) -> DVec2 {
        self.p0 * b.x + self.p1 * b.y + self.p2 * b.z
    }

    fn corner(&self, i: usize) -> DVec2 {
        match i {
            0 => self.p0,
            1 => self.p1,
            _ => self.p2,
        }
    }

    /// 行 y の中で、三角形の中のテクセルの x の範囲（無ければ None）。範囲の端は、テクセルごとの判定と同じ式で確かめる。
    fn span(&self, y: i32, width: i32) -> Option<(i32, i32)> {
        let row = (y as f64 + 0.5) - self.p0.y;
        // b(X) = alpha·dx + beta、dx = x + 0.5 − p0.x
        let lines = [
            (-(self.c11 + self.c21), 1.0 - (self.c12 + self.c22) * row),
            (self.c11, self.c12 * row),
            (self.c21, self.c22 * row),
        ];
        let (mut lo, mut hi) = (f64::NEG_INFINITY, f64::INFINITY);
        for (alpha, beta) in lines {
            if alpha > 0.0 {
                lo = lo.max((-INSIDE_EPSILON - beta) / alpha);
            } else if alpha < 0.0 {
                hi = hi.min((-INSIDE_EPSILON - beta) / alpha);
            } else if beta < -INSIDE_EPSILON {
                return None;
            }
        }
        if lo.is_nan() || hi.is_nan() || lo > hi {
            return None;
        }
        // dx → x（dx = x + 0.5 − p0.x）
        let to_x = |dx: f64| dx + self.p0.x - 0.5;
        let mut x0 = (to_x(lo).ceil().max(-1.0).min(width as f64)) as i32;
        let mut x1 = (to_x(hi).floor().max(-1.0).min(width as f64)) as i32;
        x0 = x0.clamp(0, width - 1);
        x1 = x1.clamp(0, width - 1);
        let inside = |x: i32| UvFace::inside(self.bary(x, y));
        // 判定の式と端をそろえる（割り算の丸めで 1 つずれても、テクセルごとの判定と同じ範囲にする）
        while x0 > 0 && inside(x0 - 1) {
            x0 -= 1;
        }
        while x0 <= x1 && !inside(x0) {
            x0 += 1;
        }
        while x1 < width - 1 && inside(x1 + 1) {
            x1 += 1;
        }
        while x1 >= x0 && !inside(x1) {
            x1 -= 1;
        }
        (x0 <= x1).then_some((x0, x1))
    }
}

/// 三角形ごとの印。
const FRONT: u8 = 1;
const TARGET: u8 = 2;
/// 辺 i（頂点 i → i+1）が UV アイランドの縁（にじみを出す）。
const SEAM0: u8 = 4;

/// 塗るセットの UV が覆うテクセル（にじみを、ほかの三角形のテクセルへ出さないため）。行ごとに語の境目から始める（行の組ごとに
/// 並列に埋められるように）。
struct UvMask {
    stride: usize,
    words: Vec<u64>,
}

impl UvMask {
    fn new(width: i32, height: i32) -> UvMask {
        let stride = (width as usize).div_ceil(64);
        UvMask {
            stride,
            words: vec![0; stride * height as usize],
        }
    }
    fn bytes(&self) -> u64 {
        self.words.len() as u64 * 8
    }
    #[inline]
    fn get(&self, x: i32, y: i32) -> bool {
        let x = x as usize;
        self.words[y as usize * self.stride + x / 64] >> (x % 64) & 1 == 1
    }
    /// 1 行の語（row）の x0..=x1 を立てる。
    fn fill(row: &mut [u64], x0: i32, x1: i32) {
        let (mut i, end) = (x0 as usize, x1 as usize + 1);
        while i < end {
            let bit = i % 64;
            let n = (64 - bit).min(end - i);
            let m = if n == 64 {
                u64::MAX
            } else {
                ((1u64 << n) - 1) << bit
            };
            row[i / 64] |= m;
            i += n;
        }
    }
}

/// ダブの候補 1 つ（同じテクセルを 1 つにする前）。
#[derive(Clone, Copy)]
struct Candidate {
    key: i64,
    coverage: f32,
    face: u32,
    rank: u8,
    position: Vec3,
}

/// カメラによらない、ストロークの間の一覧（対称の写しの投影の塗りと共有し、スナップショットに覚えて次のストロークでも使う）:
/// 塗るセットの UV が覆うテクセルと、三角形ごとの UV アイランドの縁の辺。
pub(crate) struct Shared {
    material: Option<i32>,
    width: i32,
    height: i32,
    mask: Option<UvMask>,
    /// 三角形ごとの `SEAM0 << 辺`（塗るセットの三角形だけ）。
    seams: Vec<u8>,
}

/// UV の覆いを並列に埋める行の組の高さ。
const MASK_ROWS: usize = 32;

impl Shared {
    /// スナップショットに覚えたものがあればそれを、無ければ作って覚える（継ぎ目のにじみと、ぼかしがアイランドの縁で継ぎ目をまたいで読む
    /// 所を決めるのに使う）。
    fn get(
        geometry: &SurfaceGeometry,
        material: Option<i32>,
        width: i32,
        height: i32,
    ) -> Arc<Shared> {
        let same = |s: &Shared| s.material == material && s.width == width && s.height == height;
        if let Ok(cache) = geometry.projection_cache.lock() {
            if let Some(s) = cache.iter().find(|s| same(s)) {
                return s.clone();
            }
        }
        let shared = Arc::new(Shared::new(geometry, material, width, height));
        if let Ok(mut cache) = geometry.projection_cache.lock() {
            cache.retain(|s| !same(s));
            cache.push(shared.clone());
            if cache.len() > 2 {
                cache.remove(0);
            }
        }
        shared
    }

    fn new(geometry: &SurfaceGeometry, material: Option<i32>, width: i32, height: i32) -> Shared {
        let triangles = geometry.triangles();
        let tolerance = geometry.weld_tolerance() as f64;
        let seams: Vec<u8> = (0..triangles.len())
            .into_par_iter()
            .with_min_len(1024)
            .map(|i| {
                if material.is_some_and(|m| m != triangles[i].material) {
                    0
                } else {
                    seam_edges(geometry, i, tolerance)
                }
            })
            .collect();
        // 塗るセットの UV が覆うテクセル（向き・見え方によらない）。三角形を行の組へ振り分け、組ごとに並列に埋める
        let mut mask = UvMask::new(width, height);
        let groups = (height as usize).div_ceil(MASK_ROWS);
        let faces: Vec<Option<(UvFace, i32, i32)>> = triangles
            .par_iter()
            .with_min_len(1024)
            .map(|t| {
                if material.is_some_and(|m| m != t.material) {
                    return None;
                }
                let uv = UvFace::new(t, width, height)?;
                let lo = uv.p0.y.min(uv.p1.y).min(uv.p2.y);
                let hi = uv.p0.y.max(uv.p1.y).max(uv.p2.y);
                if lo - 0.5 > height as f64 || hi - 0.5 < -1.0 {
                    return None;
                }
                let y0 = ((lo - 0.5).floor().max(0.0) as i64).min(height as i64 - 1) as i32;
                let y1 = ((hi - 0.5).ceil().max(0.0) as i64).min(height as i64 - 1) as i32;
                Some((uv, y0, y1))
            })
            .collect();
        let mut lists: Vec<Vec<u32>> = vec![Vec::new(); groups];
        for (i, f) in faces.iter().enumerate() {
            if let Some((_, y0, y1)) = f {
                for list in &mut lists[*y0 as usize / MASK_ROWS..=*y1 as usize / MASK_ROWS] {
                    list.push(i as u32);
                }
            }
        }
        let stride = mask.stride;
        mask.words
            .par_chunks_mut(stride * MASK_ROWS)
            .zip(lists.par_iter())
            .enumerate()
            .for_each(|(g, (words, list))| {
                let first = (g * MASK_ROWS) as i32;
                for &i in list {
                    let Some((uv, y0, y1)) = &faces[i as usize] else {
                        continue;
                    };
                    for y in (*y0).max(first)..=(*y1).min(first + MASK_ROWS as i32 - 1) {
                        if let Some((x0, x1)) = uv.span(y, width) {
                            let row = (y - first) as usize * stride;
                            UvMask::fill(&mut words[row..row + stride], x0, x1);
                        }
                    }
                }
            });
        Shared {
            material,
            width,
            height,
            mask: Some(mask),
            seams,
        }
    }

    fn bytes(&self) -> u64 {
        self.mask.as_ref().map_or(0, |m| m.bytes()) + self.seams.len() as u64
    }
}

/// カメラ 1 つ分の区画の振り分け（ストロークの間は変えない。対称の写しの投影の塗りは、元のカメラの振り分けも読む）。
struct Layout {
    frame: Frame,
    bucket: f64,
    columns: usize,
    rows: usize,
    /// 区画 b の三角形は `faces[offsets[b]..offsets[b + 1]]`（番号の順）。
    offsets: Vec<u32>,
    faces: Vec<u32>,
    /// 三角形ごとの `FRONT`・`TARGET`・`SEAM0 << 辺`。
    flags: Vec<u8>,
}

impl Layout {
    #[allow(clippy::too_many_arguments)]
    fn new(
        geometry: &SurfaceGeometry,
        view: &CameraView,
        material: Option<i32>,
        transform: &CopyTransform,
        bucket: f64,
        settings: &ProjectionSettings,
        shared: &Shared,
    ) -> Layout {
        let frame = Frame::new(view, transform);
        let columns = (frame.sw / bucket).ceil().max(1.0) as usize;
        let rows = (frame.sh / bucket).ceil().max(1.0) as usize;
        let triangles = geometry.triangles();
        // 三角形ごとに画面の箱を出す（並列。結果は番号の順）
        let ranges: Vec<(u8, Option<[u32; 4]>)> = triangles
            .par_iter()
            .enumerate()
            .with_min_len(1024)
            .map(|(i, t)| {
                let v = [frame.view_of(t.a), frame.view_of(t.b), frame.view_of(t.c)];
                let Some(f) = ScreenFace::new(&frame, i as u32, v) else {
                    return (0, None);
                };
                let mut flag = 0u8;
                if frame.faces_viewer(f.normal, f.k) != frame.flip {
                    flag |= FRONT;
                }
                if material.is_none_or(|m| m == t.material)
                    && (flag & FRONT != 0 || settings.paint_backfaces)
                {
                    flag |= TARGET;
                }
                let lo = f.min.max(DVec2::ZERO);
                let hi = f.max.min(DVec2::new(frame.sw, frame.sh));
                if lo.x > hi.x || lo.y > hi.y {
                    return (flag, None);
                }
                let bx0 = cell_index(lo.x, bucket, columns);
                let bx1 = cell_index(hi.x, bucket, columns);
                let by0 = cell_index(lo.y, bucket, rows);
                let by1 = cell_index(hi.y, bucket, rows);
                (flag, Some([bx0 as u32, bx1 as u32, by0 as u32, by1 as u32]))
            })
            .collect();
        let mut counts = vec![0u32; columns * rows];
        for (_, r) in &ranges {
            if let Some([x0, x1, y0, y1]) = r {
                for by in *y0..=*y1 {
                    for bx in *x0..=*x1 {
                        counts[by as usize * columns + bx as usize] += 1;
                    }
                }
            }
        }
        let mut offsets = vec![0u32; columns * rows + 1];
        for b in 0..columns * rows {
            offsets[b + 1] = offsets[b] + counts[b];
        }
        let mut faces = vec![0u32; offsets[columns * rows] as usize];
        let mut cursor: Vec<u32> = offsets[..columns * rows].to_vec();
        for (i, (_, r)) in ranges.iter().enumerate() {
            if let Some([x0, x1, y0, y1]) = r {
                for by in *y0..=*y1 {
                    for bx in *x0..=*x1 {
                        let b = by as usize * columns + bx as usize;
                        faces[cursor[b] as usize] = i as u32;
                        cursor[b] += 1;
                    }
                }
            }
        }
        let mut flags: Vec<u8> = ranges.iter().map(|(f, _)| *f).collect();
        drop(ranges);
        for (f, seams) in flags.iter_mut().zip(shared.seams.iter()) {
            *f |= seams;
        }
        Layout {
            frame,
            bucket,
            columns,
            rows,
            offsets,
            faces,
            flags,
        }
    }

    fn bytes(&self) -> u64 {
        self.offsets.len() as u64 * 4 + self.faces.len() as u64 * 4 + self.flags.len() as u64
    }

    /// 区画の長方形（画面の外は切る。切る・選ぶための目安で、画素がどの区画かは `cell_index` で決める）。
    fn rect(&self, bx: usize, by: usize) -> (DVec2, DVec2) {
        let b = self.bucket;
        (
            DVec2::new(bx as f64 * b, by as f64 * b),
            DVec2::new(
                ((bx + 1) as f64 * b).min(self.frame.sw),
                ((by + 1) as f64 * b).min(self.frame.sh),
            ),
        )
    }

    /// 画面の点の区画（表示域の外なら None）。
    #[inline]
    fn bucket_of(&self, s: DVec2) -> Option<(usize, usize)> {
        if !(s.x >= 0.0 && s.x < self.frame.sw && s.y >= 0.0 && s.y < self.frame.sh) {
            return None;
        }
        Some((
            cell_index(s.x, self.bucket, self.columns),
            cell_index(s.y, self.bucket, self.rows),
        ))
    }

    /// 区画の三角形を画面へ写したものと、そのビューの空間の頂点（画面で潰れる三角形は除く）。
    fn screen_faces(
        &self,
        geometry: &SurfaceGeometry,
        b: usize,
    ) -> (Vec<ScreenFace>, Vec<[DVec3; 3]>) {
        let triangles = geometry.triangles();
        let list = &self.faces[self.offsets[b] as usize..self.offsets[b + 1] as usize];
        let mut screen = Vec::with_capacity(list.len());
        let mut views = Vec::with_capacity(list.len());
        for &f in list {
            let t = &triangles[f as usize];
            let v = [
                self.frame.view_of(t.a),
                self.frame.view_of(t.b),
                self.frame.view_of(t.c),
            ];
            if let Some(s) = ScreenFace::new(&self.frame, f, v) {
                screen.push(s);
                views.push(v);
            }
        }
        (screen, views)
    }

    /// 区画の遮るもの。
    fn occluders(&self, geometry: &SurfaceGeometry, b: usize) -> Occluders {
        let (screen, _) = self.screen_faces(geometry, b);
        let (lo, hi) = self.rect(b % self.columns, b / self.columns);
        Occluders::new(screen, lo, hi)
    }
}

/// 1 回のストロークの投影の塗り（カメラ・スナップショット・設定はストロークの間は変えない）。
pub struct SurfaceProjector {
    geometry: Arc<SurfaceGeometry>,
    view: CameraView,
    material: Option<i32>,
    settings: ProjectionSettings,
    width: i32,
    height: i32,
    layout: Arc<Layout>,
    /// 対称の写しの投影の塗りのとき、元のカメラの振り分け（写しのテクセルが元のカメラから見えるかを確かめる）。
    real: Option<Arc<Layout>>,
    shared: Arc<Shared>,
    /// cos の閾値（弱め始め・0 になる所）。弱めないなら None。
    cos_range: Option<(f64, f64)>,
    tolerance: f64,
    cache: FastMap<u32, (Arc<Bucket>, u64)>,
    /// 投影の画素ごとのテクセルの画面の大きさを持つか（ストロークにアンチエイリアスの段があるときだけ）。
    metrics: bool,
    /// 投影の画素ごとのテクセルの画面の動きを持つか（画面の形の塗りの、1 テクセルを 4 × 4 の点で見る覆いだけ）。
    columns: bool,
    clock: u64,
    /// 最後に区画に時刻を付けたダブの時刻と、そのダブの区画のバイト。
    stamp: (u64, u64),
    stats: ProjectionStats,
}

impl SurfaceProjector {
    /// ストロークの始めの準備。brush_screen_radius はブラシの画面の半径（区画の大きさを決める）。material は塗るテクスチャセット
    /// （None なら全部）。width・height は文書の大きさ。
    pub fn new(
        geometry: Arc<SurfaceGeometry>,
        view: &CameraView,
        material: Option<i32>,
        width: i32,
        height: i32,
        brush_screen_radius: f32,
        settings: ProjectionSettings,
    ) -> Result<SurfaceProjector, DabRefusal> {
        if width <= 0
            || height <= 0
            || width > 32768
            || height > 32768
            || !(view.width.is_finite() && view.height.is_finite())
            || view.width < 1.0
            || view.height < 1.0
            || !view.position.is_finite()
            || !brush_screen_radius.is_finite()
            || matches!(view.projection, super::camera::Projection::Orthographic { height }
                if !(height.is_finite() && height > 0.0))
        {
            return Err(DabRefusal::InvalidArguments);
        }
        let settings = settings.sanitized();
        let shared = Shared::get(&geometry, material, width, height);
        let bucket = ((brush_screen_radius * 2.0) / 4.0).clamp(MIN_BUCKET, MAX_BUCKET) as f64;
        let layout = Arc::new(Layout::new(
            &geometry,
            view,
            material,
            &CopyTransform::default(),
            bucket,
            &settings,
            &shared,
        ));
        Ok(SurfaceProjector::with_layout(
            geometry, *view, material, width, height, settings, layout, None, shared,
        ))
    }

    /// 対称の写しの側を塗る投影の塗り: 写したカメラから同じ画面の円で見える写しの側のテクセルのうち、元のカメラからも見えるもの
    /// （隠れた所も塗る設定なら見え方は確かめない。裏の面も塗る設定なら、元のカメラから見た向きは確かめない）。区画の大きさ・設定・
    /// カメラによらない一覧は、この投影の塗りと同じ。
    pub fn copy(&self, transform: CopyTransform) -> SurfaceProjector {
        let layout = Arc::new(Layout::new(
            &self.geometry,
            &self.view,
            self.material,
            &transform,
            self.layout.bucket,
            &self.settings,
            &self.shared,
        ));
        let real = self.real.clone().unwrap_or_else(|| self.layout.clone());
        SurfaceProjector::with_layout(
            self.geometry.clone(),
            self.view,
            self.material,
            self.width,
            self.height,
            self.settings,
            layout,
            Some(real),
            self.shared.clone(),
        )
        .with_texel_metrics(self.metrics)
        .with_texel_columns(self.columns)
    }

    #[allow(clippy::too_many_arguments)]
    fn with_layout(
        geometry: Arc<SurfaceGeometry>,
        view: CameraView,
        material: Option<i32>,
        width: i32,
        height: i32,
        settings: ProjectionSettings,
        layout: Arc<Layout>,
        real: Option<Arc<Layout>>,
        shared: Arc<Shared>,
    ) -> SurfaceProjector {
        let cos_range = settings.angle_falloff.then(|| {
            (
                deterministic_cos_degrees(settings.angle_start as f64),
                deterministic_cos_degrees(settings.angle_end as f64),
            )
        });
        let tolerance = geometry.visibility_epsilon() as f64;
        SurfaceProjector {
            geometry,
            view,
            material,
            settings,
            width,
            height,
            layout,
            real,
            shared,
            cos_range,
            tolerance,
            cache: FastMap::default(),
            metrics: false,
            columns: false,
            clock: 0,
            stamp: (0, 0),
            stats: ProjectionStats::default(),
        }
    }

    pub fn settings(&self) -> ProjectionSettings {
        self.settings
    }

    /// 投影の画素ごとに、テクセル 1 つ分の画面の大きさも求めて持つ（アンチエイリアスの帯に使う。投影の画素 1 つが
    /// `METRIC_BYTES` 増える）。区画を作る前に決める（作った区画は作り直さない）。写しの投影の塗りは、この設定を受け継ぐ。
    pub(crate) fn with_texel_metrics(mut self, on: bool) -> SurfaceProjector {
        debug_assert!(self.cache.is_empty());
        self.metrics = on;
        self
    }

    /// 投影の画素ごとに、テクセルの x・y の 1 つ分の画面の動きも求めて持つ（画面の形の塗りの覆いに使う。投影の画素 1 つが
    /// `COLUMN_BYTES` 増える）。区画を作る前に決める。
    pub(crate) fn with_texel_columns(mut self, on: bool) -> SurfaceProjector {
        debug_assert!(self.cache.is_empty());
        self.columns = on;
        self
    }

    /// 区画 1 つの名目のバイト。
    fn bucket_bytes(&self, len: usize) -> u64 {
        let extra = if self.metrics { METRIC_BYTES } else { 0 }
            + if self.columns { COLUMN_BYTES } else { 0 };
        bucket_bytes(len, extra)
    }

    /// 区画の大きさ（画面の単位）。
    pub fn bucket_size(&self) -> f32 {
        self.layout.bucket as f32
    }

    /// ストロークの間ずっと持つ一覧（このカメラの区画の三角形・印）のバイト。写しと共有する一覧は [`SurfaceProjector::shared_bytes`]。
    pub fn fixed_bytes(&self) -> u64 {
        self.layout.bytes()
    }

    /// 写しと共有する、カメラによらない一覧（UV の覆い・アイランドの縁）のバイト。
    pub fn shared_bytes(&self) -> u64 {
        self.shared.bytes()
    }

    /// テクセルが塗るセットのどれかの三角形の UV に入るか（文書の外は入らない）。
    pub fn covered(&self, x: i32, y: i32) -> bool {
        x >= 0
            && y >= 0
            && x < self.width
            && y < self.height
            && self.shared.mask.as_ref().is_none_or(|m| m.get(x, y))
    }

    /// 今覚えている投影の画素のバイト。
    pub fn cached_bytes(&self) -> u64 {
        self.stats.cached_bytes
    }

    pub fn stats(&self) -> ProjectionStats {
        self.stats
    }

    /// 画面の円（center・radius は画面の単位）に入る、見えるテクセルと覆い（行優先・下の行から）。limit は覚えておける投影の画素の
    /// バイト。このダブに要る区画だけで limit を超えるときは、画素を返さずに `MemoryBudget` で断る（覚えはそのまま）。超えないときは、
    /// 覚えが limit を超えた分を、長く使っていない区画から捨てる。
    pub fn dab(
        &mut self,
        center: Vec2,
        radius: f32,
        hardness: f32,
        limit: u64,
    ) -> SurfaceDabResult {
        let clock = self.clock + 1;
        let result = self.dab_at(center, radius, hardness, limit, clock);
        if result.refusal.is_none() {
            evict_least_recent(&mut [self], limit, clock);
        }
        result
    }

    /// [`SurfaceProjector::dab`] の、捨てない形（いくつかの投影の塗りで 1 つの予算を分けるときに、呼び手が [`evict_least_recent`] で
    /// まとめて捨てる）。room はこのダブの区画に使ってよいバイト、clock は使った区画に付ける時刻（呼び手のダブごとに増やす）。
    pub(crate) fn dab_at(
        &mut self,
        center: Vec2,
        radius: f32,
        hardness: f32,
        room: u64,
        clock: u64,
    ) -> SurfaceDabResult {
        if !radius.is_finite() || radius <= 0.0 {
            return SurfaceDabResult {
                refusal: Some(DabRefusal::InvalidArguments),
                ..SurfaceDabResult::default()
            };
        }
        self.dab_with(
            center,
            &RoundCover::new(radius, hardness),
            true,
            room,
            clock,
        )
    }

    /// [`SurfaceProjector::dab_at`] の、ダブの形を渡す形: 円（`cover.reach()`）に重なる区画の投影の画素の覆いを `cover` で出す。
    /// `weighted` なら面の向きの弱め（裏の面・傾いた面）を掛ける（デュアルブラシの 2 つ目のダブの溜まりは掛けない）。
    pub(crate) fn dab_with<C: ScreenCover>(
        &mut self,
        center: Vec2,
        cover: &C,
        weighted: bool,
        room: u64,
        clock: u64,
    ) -> SurfaceDabResult {
        let mut result = SurfaceDabResult::default();
        let r = cover.reach();
        if !finite2(center) || !r.is_finite() || r <= 0.0 {
            result.refusal = Some(DabRefusal::InvalidArguments);
            return result;
        }
        let c = center.as_dvec2();
        let needed = self.buckets_in_circle(c, r);
        self.clock = clock;
        let missing: Vec<u32> = needed
            .iter()
            .copied()
            .filter(|b| !self.cache.contains_key(b))
            .collect();
        let built: Vec<(u32, Bucket)> = {
            let _profile =
                (!missing.is_empty()).then(|| profile::scope(profile::Stage::SurfaceBuckets));
            if missing.len() > 1 && rayon::current_num_threads() > 1 {
                missing
                    .par_iter()
                    .with_max_len(1)
                    .map(|&b| (b, self.build_bucket(b as usize)))
                    .collect()
            } else {
                missing
                    .iter()
                    .map(|&b| (b, self.build_bucket(b as usize)))
                    .collect()
            }
        };
        let need: u64 = needed
            .iter()
            .map(|b| match self.cache.get(b) {
                Some((p, _)) => self.bucket_bytes(p.pixels.len()),
                None => 0,
            })
            .sum::<u64>()
            + built
                .iter()
                .map(|(_, p)| self.bucket_bytes(p.pixels.len()))
                .sum::<u64>();
        self.stats.buckets_built += built.len() as u64;
        self.stats.bucket_reuses += (needed.len() - built.len()) as u64;
        self.stats.largest_dab_bytes = self.stats.largest_dab_bytes.max(need);
        if need > room {
            self.stats.skipped_dabs += 1;
            result.refusal = Some(DabRefusal::MemoryBudget);
            return result;
        }
        for (b, bucket) in built {
            self.stats.cached_bytes += self.bucket_bytes(bucket.pixels.len());
            self.cache.insert(b, (Arc::new(bucket), 0));
        }
        for b in &needed {
            if let Some(entry) = self.cache.get_mut(b) {
                entry.1 = clock;
            }
        }
        self.stamp = (clock, need);
        self.stats.cached_buckets = self.cache.len();
        // 円の中の投影の画素の覆い
        let _gather_profile = profile::scope(profile::Stage::SurfaceGather);
        let lists: Vec<Arc<Bucket>> = needed.iter().map(|b| self.cache[b].0.clone()).collect();
        let width = self.width as i64;
        let gather = |bucket: &Arc<Bucket>| -> Vec<Candidate> {
            let mut out = Vec::new();
            for (i, p) in bucket.pixels.iter().enumerate() {
                let dx = p.sx as f64 - c.x;
                let dy = p.sy as f64 - c.y;
                // テクセルの画面の大きさを持たない投影の塗りは 0（使えない値。丸い筆先は今の式）
                let metric = bucket.metrics.get(i).copied().unwrap_or([0.0; 3]);
                let cov = cover.cover(dx, dy, p.x, p.y, metric);
                if cov <= 0.0 {
                    continue;
                }
                let cov = if weighted { cov * p.weight } else { cov };
                if cov > 0.0 {
                    out.push(Candidate {
                        key: p.y as i64 * width + p.x as i64,
                        coverage: cov,
                        face: p.face,
                        rank: p.rank,
                        position: p.position,
                    });
                }
            }
            out
        };
        let total: usize = lists.iter().map(|l| l.pixels.len()).sum();
        result.candidate_pixels = total.min(i32::MAX as usize) as i32;
        let parallel_min = PARALLEL_CANDIDATES.load(Ordering::Relaxed);
        let mut candidates: Vec<Candidate> = if lists.len() > 1 && total > parallel_min {
            lists.par_iter().map(gather).collect::<Vec<_>>().concat()
        } else {
            lists.iter().flat_map(gather).collect()
        };
        // 区画ごとに下の行から並んでいる（区画を作るときに並べてある）ので、区画の並びのまま足した候補は、整った列がいくつか連なった形。
        // 位置だけで安定に並べる（整った列を併合するだけなので、全部の候補を比べて並べるより速い）と、同じテクセルは区画の並びの順に隣り合い、
        // 覆いの大きいほう（同じなら三角形の番号の小さいほう、続けて辺の番号）の 1 つだけを残す
        if candidates.len() > parallel_min {
            candidates.par_sort_by_key(|c| c.key);
        } else {
            candidates.sort_by_key(|c| c.key);
        }
        candidates.dedup_by(|next, kept| {
            if next.key != kept.key {
                return false;
            }
            if next
                .coverage
                .total_cmp(&kept.coverage)
                .reverse()
                .then(next.face.cmp(&kept.face))
                .then(next.rank.cmp(&kept.rank))
                .is_lt()
            {
                *kept = *next;
            }
            true
        });
        result.pixels = candidates
            .into_iter()
            .map(|c| SurfacePixel {
                x: (c.key % width) as i32,
                y: (c.key / width) as i32,
                coverage: c.coverage,
                triangle: Some(c.face),
                position: c.position,
            })
            .collect();
        result
    }

    /// 時刻 clock のダブで使った区画のバイト（そのダブの間は捨てられない分）。
    pub(crate) fn bytes_used_at(&self, clock: u64) -> u64 {
        if self.stamp.0 == clock {
            self.stamp.1
        } else {
            0
        }
    }

    /// 画面の箱 [lo, hi]（画面の単位）に重なる全部の区画の投影の画素の、中心 center からのずれで測った `cover` の覆い（`weighted` なら
    /// 面の向きの弱めも掛ける）を、区画の組ごとに作って、0 より大きいものを sink へ渡す（区画は覚えずに捨てる。1 回きりの大きな形の
    /// 集め。箱の外の投影の画素の覆いは 0 でなければならない）。同じテクセルは区画をまたいで何度も来うる（重なった UV・にじみ）ので、
    /// まとめるのは sink。sink は今溜めているバイトを返す。room は一覧（共有の一覧・区画の一覧）・作った区画・候補・溜めの合計に
    /// 使ってよいバイトで、超えたら `MemoryBudget`（sink に渡した分は呼び手が捨てる）。
    pub(crate) fn sweep<C: AreaCover>(
        &mut self,
        center: DVec2,
        cover: &C,
        weighted: bool,
        (lo, hi): (DVec2, DVec2),
        room: u64,
        mut sink: impl FnMut(&[SweepPixel]) -> u64,
    ) -> Result<(), DabRefusal> {
        if !center.is_finite() || !lo.is_finite() || !hi.is_finite() {
            return Err(DabRefusal::InvalidArguments);
        }
        let c = center;
        let needed = self.buckets_in_box(lo, hi);
        let fixed = self.shared_bytes() + self.fixed_bytes();
        let threads = rayon::current_num_threads().max(1);
        let gather = |bucket: &Bucket| -> Vec<SweepPixel> {
            let mut out = Vec::new();
            for (i, p) in bucket.pixels.iter().enumerate() {
                let columns = bucket.columns.get(i).copied().unwrap_or([0.0; 4]);
                let cov = cover.cover(p.sx as f64 - c.x, p.sy as f64 - c.y, columns);
                if cov <= 0.0 {
                    continue;
                }
                let cov = if weighted { cov * p.weight } else { cov };
                if cov > 0.0 {
                    out.push(SweepPixel {
                        x: p.x,
                        y: p.y,
                        coverage: cov,
                        screen: Vec2::new(p.sx, p.sy),
                    });
                }
            }
            out
        };
        let mut held = 0u64;
        for batch in needed.chunks(threads * 2) {
            let built: Vec<Bucket> = if batch.len() > 1 && threads > 1 {
                batch
                    .par_iter()
                    .with_max_len(1)
                    .map(|&b| self.build_bucket(b as usize))
                    .collect()
            } else {
                batch
                    .iter()
                    .map(|&b| self.build_bucket(b as usize))
                    .collect()
            };
            self.stats.buckets_built += built.len() as u64;
            let bytes: u64 = built
                .iter()
                .map(|b| self.bucket_bytes(b.pixels.len()))
                .sum();
            self.stats.largest_dab_bytes = self.stats.largest_dab_bytes.max(bytes);
            if fixed + held + bytes > room {
                self.stats.skipped_dabs += 1;
                return Err(DabRefusal::MemoryBudget);
            }
            let lists: Vec<Vec<SweepPixel>> = if built.len() > 1 && threads > 1 {
                built.par_iter().with_max_len(1).map(gather).collect()
            } else {
                built.iter().map(gather).collect()
            };
            let candidates: u64 = lists
                .iter()
                .map(|l| l.len() as u64 * SWEEP_PIXEL_BYTES)
                .sum();
            if fixed + held + bytes + candidates > room {
                self.stats.skipped_dabs += 1;
                return Err(DabRefusal::MemoryBudget);
            }
            drop(built);
            for list in &lists {
                held = sink(list);
            }
            if fixed + held > room {
                self.stats.skipped_dabs += 1;
                return Err(DabRefusal::MemoryBudget);
            }
        }
        Ok(())
    }

    /// 画面の箱 [lo, hi] に重なる区画（番号の順）。
    fn buckets_in_box(&self, lo: DVec2, hi: DVec2) -> Vec<u32> {
        let layout = &self.layout;
        let b = layout.bucket;
        let mut out = Vec::new();
        if !(lo.x <= hi.x && lo.y <= hi.y)
            || hi.x < 0.0
            || hi.y < 0.0
            || lo.x > layout.frame.sw
            || lo.y > layout.frame.sh
        {
            return out;
        }
        let x0 = cell_index(lo.x, b, layout.columns);
        let x1 = cell_index(hi.x, b, layout.columns);
        let y0 = cell_index(lo.y, b, layout.rows);
        let y1 = cell_index(hi.y, b, layout.rows);
        for by in y0..=y1 {
            for bx in x0..=x1 {
                out.push((by * layout.columns + bx) as u32);
            }
        }
        out
    }

    /// 円に重なる区画（番号の順）。
    fn buckets_in_circle(&self, c: DVec2, r: f64) -> Vec<u32> {
        let layout = &self.layout;
        let b = layout.bucket;
        let mut out = Vec::new();
        if c.x + r < 0.0 || c.y + r < 0.0 || c.x - r > layout.frame.sw || c.y - r > layout.frame.sh
        {
            return out;
        }
        // 範囲は 1 つずつ広げ、区画ごとに円と長方形で確かめる（丸めで区画を落とさないように、半径も少し広げる）
        let x0 = cell_index(c.x - r, b, layout.columns).saturating_sub(1);
        let x1 = (cell_index(c.x + r, b, layout.columns) + 1).min(layout.columns - 1);
        let y0 = cell_index(c.y - r, b, layout.rows).saturating_sub(1);
        let y1 = (cell_index(c.y + r, b, layout.rows) + 1).min(layout.rows - 1);
        let reach = r * (1.0 + 1e-9) + 1e-9;
        for by in y0..=y1 {
            for bx in x0..=x1 {
                let (lo, hi) = layout.rect(bx, by);
                let nearest = c.clamp(lo, hi);
                if (nearest - c).length_squared() < reach * reach {
                    out.push((by * layout.columns + bx) as u32);
                }
            }
        }
        out
    }

    /// 区画 1 つの投影の画素（区画・三角形・設定だけで決まる）。
    fn build_bucket(&self, b: usize) -> Bucket {
        let layout = &*self.layout;
        let frame = &layout.frame;
        let (bx, by) = (b % layout.columns, b / layout.columns);
        let (lo, hi) = layout.rect(bx, by);
        let triangles = self.geometry.triangles();
        let (screen, views) = layout.screen_faces(&self.geometry, b);
        let occluders =
            (!self.settings.paint_hidden).then(|| Occluders::new(screen.clone(), lo, hi));
        // 写しは、元のカメラからも見えるテクセルだけ（隠れた所も塗る設定なら確かめない）
        let real = match (&self.real, self.settings.paint_hidden) {
            (Some(l), false) => Some(RealCheck {
                layout: l,
                geometry: &self.geometry,
                backfaces: self.settings.paint_backfaces,
                tolerance: self.tolerance,
                occluders: RefCell::new(FastMap::default()),
            }),
            _ => None,
        };
        let mut out = Bucket::default();
        let mut clipped = Vec::with_capacity(8);
        for (i, sf) in screen.iter().enumerate() {
            let f = sf.face as usize;
            if layout.flags[f] & TARGET == 0 {
                continue;
            }
            let t = &triangles[f];
            let Some(uv) = UvFace::new(t, self.width, self.height) else {
                continue;
            };
            sf.clip_to_rect(lo, hi, &mut clipped);
            if clipped.len() < 3 {
                continue;
            }
            let v = views[i];
            let ctx = FaceContext {
                layout,
                face: sf.face,
                v,
                model: [t.a.as_dvec3(), t.b.as_dvec3(), t.c.as_dvec3()],
                unit: sf.normal.normalize(),
                cell: (bx, by),
                occluders: occluders.as_ref(),
                tolerance: self.tolerance,
                cos_range: self.cos_range,
                real: real.as_ref().map(|r| {
                    (
                        r,
                        [
                            r.layout.frame.view_of(t.a),
                            r.layout.frame.view_of(t.b),
                            r.layout.frame.view_of(t.c),
                        ],
                    )
                }),
                texel_steps: texel_steps(&v, &uv),
                metrics: self.metrics,
                columns: self.columns,
            };
            // 区画の中の部分の UV の箱（切った多角形の頂点をレイで平面へ戻す）
            let mut uv_lo = DVec2::splat(f64::INFINITY);
            let mut uv_hi = DVec2::splat(f64::NEG_INFINITY);
            for &q in &clipped {
                let Some(p) = frame.plane_point(q, sf.normal, sf.k) else {
                    continue;
                };
                let u = uv.point(bary3(v, p));
                if u.is_finite() {
                    uv_lo = uv_lo.min(u);
                    uv_hi = uv_hi.max(u);
                }
            }
            if uv_lo.x <= uv_hi.x && uv_lo.y <= uv_hi.y {
                let x0 = (uv_lo.x - 1.5).floor().max(0.0) as i64;
                let x1 = (uv_hi.x + 0.5).ceil().min(self.width as f64 - 1.0) as i64;
                let y0 = (uv_lo.y - 1.5).floor().max(0.0) as i64;
                let y1 = (uv_hi.y + 0.5).ceil().min(self.height as f64 - 1.0) as i64;
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        let w = uv.bary(x as i32, y as i32);
                        if !UvFace::inside(w) {
                            continue;
                        }
                        ctx.push(w, x as u16, y as u16, 0, &mut out);
                    }
                }
            }
            // 継ぎ目のにじみ
            if let Some(mask) = self
                .shared
                .mask
                .as_ref()
                .filter(|_| self.settings.seam_bleed > 0)
            {
                for edge in 0..3 {
                    if layout.flags[f] & (SEAM0 << edge) == 0 {
                        continue;
                    }
                    self.bleed_edge(&ctx, &uv, mask, edge, lo, hi, &mut out);
                }
            }
        }
        out.sorted()
    }

    /// 辺 edge の外のにじみのテクセルのうち、辺の上の点が区画に落ちるもの。
    #[allow(clippy::too_many_arguments)]
    fn bleed_edge(
        &self,
        ctx: &FaceContext<'_>,
        uv: &UvFace,
        mask: &UvMask,
        edge: usize,
        lo: DVec2,
        hi: DVec2,
        out: &mut Bucket,
    ) {
        let n = self.settings.seam_bleed as f64;
        let (i, j) = (edge, (edge + 1) % 3);
        let (ua, ub) = (uv.corner(i), uv.corner(j));
        let e = ub - ua;
        let len2 = e.length_squared();
        if len2.is_nan() || len2 <= 0.0 {
            return;
        }
        // 辺のうち区画にかかる部分（画面の線分を閉じた長方形で切る → 3D の辺の上の割合へ）
        let Some((u0, u1)) = edge_interval(&ctx.layout.frame, ctx.v[i], ctx.v[j], lo, hi) else {
            return;
        };
        let pa = ua + e * u0;
        let pb = ua + e * u1;
        let box_lo = pa.min(pb) - DVec2::splat(n + 1.0);
        let box_hi = pa.max(pb) + DVec2::splat(n + 1.0);
        let x0 = (box_lo.x - 0.5).floor().max(0.0) as i64;
        let x1 = (box_hi.x - 0.5).ceil().min(self.width as f64 - 1.0) as i64;
        let y0 = (box_lo.y - 0.5).floor().max(0.0) as i64;
        let y1 = (box_hi.y - 0.5).ceil().min(self.height as f64 - 1.0) as i64;
        for y in y0..=y1 {
            for x in x0..=x1 {
                if mask.get(x as i32, y as i32) {
                    continue;
                }
                let texel = DVec2::new(x as f64 + 0.5, y as f64 + 0.5);
                let u = ((texel - ua).dot(e) / len2).clamp(0.0, 1.0);
                if (texel - (ua + e * u)).length_squared() > n * n {
                    continue;
                }
                let mut w = DVec3::ZERO;
                w[i] = 1.0 - u;
                w[j] = u;
                ctx.push(w, x as u16, y as u16, edge as u8 + 1, out);
            }
        }
    }
}

/// 覚えた区画 1 つの名目のバイト。
fn bucket_bytes(len: usize, extra: u64) -> u64 {
    len as u64 * (PIXEL_BYTES + extra) + BUCKET_OVERHEAD
}

/// いくつかの投影の塗り（元の側と対称の写し）の覚えを、合わせて limit バイトまでに減らす。どの投影の塗りの区画でも、長く使って
/// いないものから捨てる（時刻 clock のダブで使った区画は捨てない。時刻が同じなら並びの前の投影の塗り・番号の小さい区画から）。
/// 捨てた区画は要るときに同じものを作り直すので、塗る結果は変わらない。
pub(crate) fn evict_least_recent(projectors: &mut [&mut SurfaceProjector], limit: u64, clock: u64) {
    let mut total: u64 = projectors.iter().map(|p| p.stats.cached_bytes).sum();
    if total > limit {
        let mut old: Vec<(u64, usize, u32, u64)> = projectors
            .iter()
            .enumerate()
            .flat_map(|(i, p)| {
                p.cache
                    .iter()
                    .filter(|(_, (_, used))| *used != clock)
                    .map(move |(b, (list, used))| (*used, i, *b, p.bucket_bytes(list.pixels.len())))
            })
            .collect();
        old.sort_unstable();
        for (_, i, b, bytes) in old {
            if total <= limit {
                break;
            }
            let p = &mut projectors[i];
            p.cache.remove(&b);
            p.stats.cached_bytes -= bytes;
            p.stats.evictions += 1;
            total -= bytes;
        }
    }
    for p in projectors.iter_mut() {
        p.stats.cached_buckets = p.cache.len();
        p.stats.peak_bytes = p.stats.peak_bytes.max(p.stats.cached_bytes);
    }
}

/// 写しのテクセルが、元のカメラから見えるか（表の面か・手前に別の面が無いか）を確かめる。元のカメラの区画の遮るものは、
/// 写しの区画を作る間だけ覚える。
struct RealCheck<'a> {
    layout: &'a Layout,
    geometry: &'a SurfaceGeometry,
    backfaces: bool,
    tolerance: f64,
    occluders: RefCell<FastMap<usize, Occluders>>,
}

impl RealCheck<'_> {
    /// 三角形 face の上の重み w の点（元のカメラのビューの空間の頂点 rv）が見えるか。
    fn visible(&self, face: u32, rv: &[DVec3; 3], w: DVec3) -> bool {
        if !self.backfaces && self.layout.flags[face as usize] & FRONT == 0 {
            return false;
        }
        let frame = &self.layout.frame;
        let p = rv[0] * w.x + rv[1] * w.y + rv[2] * w.z;
        if p.z.is_nan() || p.z < frame.near {
            return false;
        }
        let s = frame.screen_of(p);
        let Some((bx, by)) = self.layout.bucket_of(s) else {
            return false;
        };
        let b = by * self.layout.columns + bx;
        let mut map = self.occluders.borrow_mut();
        let occluders = map
            .entry(b)
            .or_insert_with(|| self.layout.occluders(self.geometry, b));
        !occluders.occluded(frame, s, p.z, face, self.tolerance)
    }
}

/// 塗る三角形 1 つの、区画の中での判定に使うもの。
struct FaceContext<'a> {
    layout: &'a Layout,
    face: u32,
    /// ビューの空間の頂点・モデルの空間の頂点。
    v: [DVec3; 3],
    model: [DVec3; 3],
    /// 単位の法線（ビューの空間）。
    unit: DVec3,
    /// この区画の列と行。
    cell: (usize, usize),
    occluders: Option<&'a Occluders>,
    tolerance: f64,
    cos_range: Option<(f64, f64)>,
    /// 写しのとき、元のカメラの確かめと、その三角形の元のカメラのビューの空間の頂点。
    real: Option<(&'a RealCheck<'a>, [DVec3; 3])>,
    /// テクセルの x・y の 1 つ分の、ビューの空間での動き（三角形の上で一定）。
    texel_steps: [DVec3; 2],
    /// 投影の画素ごとにテクセルの画面の大きさを求めるか。
    metrics: bool,
    /// 投影の画素ごとにテクセルの画面の動きを求めるか。
    columns: bool,
}

/// 三角形の上で、テクセルの x・y の 1 つ分がビューの空間でどれだけ動くか（UV → 重心座標の係数と頂点から）。
fn texel_steps(v: &[DVec3; 3], uv: &UvFace) -> [DVec3; 2] {
    let (e1, e2) = (v[1] - v[0], v[2] - v[0]);
    [e1 * uv.c11 + e2 * uv.c21, e1 * uv.c12 + e2 * uv.c22]
}

/// ビューの空間の点 p で、テクセルの動き steps が画面でどれだけか（テクセル → 画面の写しの J·Jᵀ）。
fn texel_metric(frame: &Frame, p: DVec3, steps: &[DVec3; 2]) -> TexelMetric {
    let (jx, jy) = texel_columns(frame, p, steps);
    TexelMetric::from_columns(jx, jy)
}

/// ビューの空間の点 p で、テクセルの x・y の 1 つ分の動き steps が画面でどれだけ動くか（テクセル → 画面の写し J の列。y は下向き）。
fn texel_columns(frame: &Frame, p: DVec3, steps: &[DVec3; 2]) -> ((f64, f64), (f64, f64)) {
    if let Some((hx, hy)) = frame.ortho {
        // 正投影の写しは線形（奥行きによらない）
        let (ax, ay) = (0.5 * frame.sw / hx, -0.5 * frame.sh / hy);
        let column = |d: DVec3| (ax * d.x, ay * d.y);
        return (column(steps[0]), column(steps[1]));
    }
    let iz = 1.0 / p.z;
    let (ax, ay) = (0.5 * frame.sw / frame.tx, -0.5 * frame.sh / frame.ty);
    let column = |d: DVec3| {
        (
            ax * (d.x - p.x * iz * d.z) * iz,
            ay * (d.y - p.y * iz * d.z) * iz,
        )
    };
    (column(steps[0]), column(steps[1]))
}

impl SurfaceProjector {
    /// 面の点 hit のテクセル 1 つ分が、この投影の画面でどれだけか（y は下向き。カメラの前に無い・UV が潰れていれば None）。
    pub(crate) fn metric_at(&self, hit: &SurfaceHit) -> Option<TexelMetric> {
        let t = self.geometry.triangles().get(hit.triangle as usize)?;
        let frame = &self.layout.frame;
        let v = [frame.view_of(t.a), frame.view_of(t.b), frame.view_of(t.c)];
        let uv = UvFace::new(t, self.width, self.height)?;
        let w = hit.barycentric.as_dvec3();
        let p = v[0] * w.x + v[1] * w.y + v[2] * w.z;
        if p.z.is_nan() || p.z < frame.near {
            return None;
        }
        Some(texel_metric(frame, p, &texel_steps(&v, &uv))).filter(|m| m.usable())
    }
}

impl FaceContext<'_> {
    /// 三角形の上の重み w の点が、この区画の中に見えるなら、その投影の画素（と、持つならテクセルの画面の大きさ）を out に足す。
    #[inline]
    fn push(&self, w: DVec3, x: u16, y: u16, rank: u8, out: &mut Bucket) {
        if let Some((pixel, p)) = self.pixel(w, x, y, rank) {
            out.pixels.push(pixel);
            if self.metrics {
                let m = texel_metric(&self.layout.frame, p, &self.texel_steps);
                out.metrics.push([m.xx as f32, m.xy as f32, m.yy as f32]);
            }
            if self.columns {
                let (jx, jy) = texel_columns(&self.layout.frame, p, &self.texel_steps);
                out.columns
                    .push([jx.0 as f32, jx.1 as f32, jy.0 as f32, jy.1 as f32]);
            }
        }
    }

    /// 三角形の上の重み w の点が、この区画の中に見えるなら、その投影の画素と、ビューの空間の点。
    #[inline]
    fn pixel(&self, w: DVec3, x: u16, y: u16, rank: u8) -> Option<(ProjPixel, DVec3)> {
        let frame = &self.layout.frame;
        let p = self.v[0] * w.x + self.v[1] * w.y + self.v[2] * w.z;
        if p.z.is_nan() || p.z < frame.near {
            return None;
        }
        let s = frame.screen_of(p);
        if self.layout.bucket_of(s) != Some(self.cell) {
            return None;
        }
        let weight = match self.cos_range {
            None => 1.0,
            Some((start, end)) => {
                // 法線と、点からカメラへの向きの cos。平面の上の点では unit · p が同じ符号（表の面は負・裏の面と鏡映のカメラでは
                // 逆）なので、絶対値が見ている側から測った角度になる（正投影は前の向きとの cos）
                let cos = frame.view_cos(self.unit, p)?;
                if cos >= start {
                    1.0
                } else if cos <= end {
                    return None;
                } else {
                    ((cos - end) / (start - end)) as f32
                }
            }
        };
        if weight.is_nan() || weight <= 0.0 {
            return None;
        }
        if let Some(o) = self.occluders {
            if o.occluded(frame, s, p.z, self.face, self.tolerance) {
                return None;
            }
        }
        if let Some((real, rv)) = &self.real {
            if !real.visible(self.face, rv, w) {
                return None;
            }
        }
        let m = self.model[0] * w.x + self.model[1] * w.y + self.model[2] * w.z;
        Some((
            ProjPixel {
                x,
                y,
                face: self.face,
                sx: s.x as f32,
                sy: s.y as f32,
                weight,
                position: m.as_vec3(),
                rank,
            },
            p,
        ))
    }
}

/// 区画の遮るもの: 升ごとに、重なる三角形を手前（近い奥行き）から。
struct Occluders {
    faces: Vec<ScreenFace>,
    lo: DVec2,
    columns: usize,
    rows: usize,
    offsets: Vec<u32>,
    items: Vec<u32>,
}

impl Occluders {
    fn new(screen: Vec<ScreenFace>, lo: DVec2, hi: DVec2) -> Occluders {
        let columns = ((hi.x - lo.x) / CELL).ceil().max(1.0) as usize;
        let rows = ((hi.y - lo.y) / CELL).ceil().max(1.0) as usize;
        let range = |f: &ScreenFace| -> Option<(usize, usize, usize, usize)> {
            let a = f.min.max(lo);
            let b = f.max.min(hi);
            if a.x > b.x || a.y > b.y {
                return None;
            }
            Some((
                cell_index(a.x - lo.x, CELL, columns),
                cell_index(b.x - lo.x, CELL, columns),
                cell_index(a.y - lo.y, CELL, rows),
                cell_index(b.y - lo.y, CELL, rows),
            ))
        };
        let mut counts = vec![0u32; columns * rows];
        for f in &screen {
            if let Some((x0, x1, y0, y1)) = range(f) {
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        counts[y * columns + x] += 1;
                    }
                }
            }
        }
        let mut offsets = vec![0u32; columns * rows + 1];
        for c in 0..columns * rows {
            offsets[c + 1] = offsets[c] + counts[c];
        }
        let mut items = vec![0u32; offsets[columns * rows] as usize];
        let mut cursor: Vec<u32> = offsets[..columns * rows].to_vec();
        for (i, f) in screen.iter().enumerate() {
            if let Some((x0, x1, y0, y1)) = range(f) {
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        let c = y * columns + x;
                        items[cursor[c] as usize] = i as u32;
                        cursor[c] += 1;
                    }
                }
            }
        }
        for c in 0..columns * rows {
            let slice = &mut items[offsets[c] as usize..offsets[c + 1] as usize];
            slice.sort_unstable_by(|&a, &b| {
                let (fa, fb) = (&screen[a as usize], &screen[b as usize]);
                fa.min_depth
                    .total_cmp(&fb.min_depth)
                    .then(fa.face.cmp(&fb.face))
            });
        }
        Occluders {
            faces: screen,
            lo,
            columns,
            rows,
            offsets,
            items,
        }
    }

    /// 画面の点 s（奥行き depth、三角形 face の上）の手前に、ほかの三角形があるか。
    #[inline]
    fn occluded(&self, frame: &Frame, s: DVec2, depth: f64, face: u32, tolerance: f64) -> bool {
        let cx = cell_index(s.x - self.lo.x, CELL, self.columns);
        let cy = cell_index(s.y - self.lo.y, CELL, self.rows);
        let c = cy * self.columns + cx;
        let limit = depth - tolerance.max(depth * 1e-6);
        for &i in &self.items[self.offsets[c] as usize..self.offsets[c + 1] as usize] {
            let o = &self.faces[i as usize];
            if o.min_depth >= limit {
                break;
            }
            if o.face == face || !o.contains(s) {
                continue;
            }
            if let Some(z) = o.depth_at(frame, s) {
                if z < limit {
                    return true;
                }
            }
        }
        false
    }
}

/// x の入る升（floor(x / size) を 0〜count − 1 に収める）。区画への三角形の振り分け・画素の区画・円の区画の選びが同じ式を使うので、
/// 三角形の中の点の区画は、いつもその三角形を振り分けた区画のどれか（式が x について単調なので）。
#[inline]
fn cell_index(x: f64, size: f64, count: usize) -> usize {
    let i = (x / size).floor();
    if i > 0.0 {
        (i as usize).min(count - 1)
    } else {
        0
    }
}

/// 3D の点 p の、三角形 v に対する重心座標。
fn bary3(v: [DVec3; 3], p: DVec3) -> DVec3 {
    let (e0, e1, e2) = (v[1] - v[0], v[2] - v[0], p - v[0]);
    let (d00, d01, d11) = (e0.dot(e0), e0.dot(e1), e1.dot(e1));
    let (d20, d21) = (e2.dot(e0), e2.dot(e1));
    let denom = d00 * d11 - d01 * d01;
    if denom == 0.0 {
        return DVec3::new(1.0, 0.0, 0.0);
    }
    let b1 = (d11 * d20 - d01 * d21) / denom;
    let b2 = (d00 * d21 - d01 * d20) / denom;
    DVec3::new(1.0 - b1 - b2, b1, b2)
}

/// ビューの空間の辺 a → b のうち、画面の閉じた長方形 [lo, hi] に写る部分の、辺の上の割合の範囲（少し広げる）。
fn edge_interval(frame: &Frame, a: DVec3, b: DVec3, lo: DVec2, hi: DVec2) -> Option<(f64, f64)> {
    // 近い面で切る
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    let near = frame.near;
    if a.z < near && b.z < near {
        return None;
    }
    if a.z < near {
        t0 = (near - a.z) / (b.z - a.z);
    } else if b.z < near {
        t1 = (near - a.z) / (b.z - a.z);
    }
    let p0 = a + (b - a) * t0;
    let p1 = a + (b - a) * t1;
    let (s0, s1) = (frame.screen_of(p0), frame.screen_of(p1));
    // 画面の線分を長方形で切る（Liang–Barsky）
    let d = s1 - s0;
    let (mut e0, mut e1) = (0.0f64, 1.0f64);
    for (p, q) in [
        (-d.x, s0.x - lo.x),
        (d.x, hi.x - s0.x),
        (-d.y, s0.y - lo.y),
        (d.y, hi.y - s0.y),
    ] {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
        } else {
            let r = q / p;
            if p < 0.0 {
                e0 = e0.max(r);
            } else {
                e1 = e1.min(r);
            }
        }
    }
    if e0 > e1 {
        return None;
    }
    // 画面の割合 → 3D の割合（透視は 1/z が画面で線形、正投影は画面の割合のまま）
    let (z0, z1) = (p0.z, p1.z);
    let ortho = frame.ortho.is_some();
    let to3d = |e: f64| -> f64 {
        if ortho {
            return t0 + (t1 - t0) * e;
        }
        let w = e / z1;
        let w0 = (1.0 - e) / z0;
        let u = w / (w + w0);
        t0 + (t1 - t0) * u
    };
    let (u0, u1) = (to3d(e0), to3d(e1));
    let margin = 1e-6;
    Some((
        (u0.min(u1) - margin).max(0.0),
        (u0.max(u1) + margin).min(1.0),
    ))
}

/// 三角形 i の辺のうち、UV アイランドの縁のもの（`SEAM0 << 辺`）。隣の三角形が同じ 2 頂点（溶接の鍵）で、その頂点の UV も同じなら縁ではない。
fn seam_edges(geometry: &SurfaceGeometry, i: usize, tolerance: f64) -> u8 {
    let t = &geometry.triangles()[i];
    let keys = [
        position_key(t.a, tolerance),
        position_key(t.b, tolerance),
        position_key(t.c, tolerance),
    ];
    let uvs = [t.uv_a, t.uv_b, t.uv_c];
    let mut flags = 0u8;
    for edge in 0..3 {
        let (a, b) = (edge, (edge + 1) % 3);
        let continues = geometry.neighbors(i).iter().any(|&n| {
            let o = &geometry.triangles()[n as usize];
            let okeys = [
                position_key(o.a, tolerance),
                position_key(o.b, tolerance),
                position_key(o.c, tolerance),
            ];
            let ouvs = [o.uv_a, o.uv_b, o.uv_c];
            let ia = okeys.iter().position(|k| *k == keys[a]);
            let ib = okeys.iter().position(|k| *k == keys[b]);
            match (ia, ib) {
                (Some(ia), Some(ib)) if ia != ib => {
                    same_uv(ouvs[ia], uvs[a]) && same_uv(ouvs[ib], uvs[b])
                }
                _ => false,
            }
        });
        if !continues {
            flags |= SEAM0 << edge;
        }
    }
    flags
}

fn same_uv(a: Vec2, b: Vec2) -> bool {
    (a.x - b.x).abs() <= 1e-6 && (a.y - b.y).abs() <= 1e-6
}

/// 度の cos（四則だけの多項式。機械の数学の関数によらず同じ値）。0〜90 度。
fn deterministic_cos_degrees(degrees: f64) -> f64 {
    let d = degrees.clamp(0.0, 90.0);
    // cos(x) = sin(90° − x)。小さいほうの角で級数を使う
    let to_radians = std::f64::consts::PI / 180.0;
    if d <= 45.0 {
        taylor_cos(d * to_radians)
    } else {
        taylor_sin((90.0 - d) * to_radians)
    }
}

fn taylor_cos(x: f64) -> f64 {
    let x2 = x * x;
    let mut term = 1.0;
    let mut sum = 1.0;
    for k in 1..12 {
        term *= -x2 / ((2 * k - 1) as f64 * (2 * k) as f64);
        sum += term;
    }
    sum
}

fn taylor_sin(x: f64) -> f64 {
    let x2 = x * x;
    let mut term = x;
    let mut sum = x;
    for k in 1..12 {
        term *= -x2 / ((2 * k) as f64 * (2 * k + 1) as f64);
        sum += term;
    }
    sum
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 区画の投影の画素は、下の行から（同じ位置は三角形・辺の番号の順に）並び、テクセルの画面の大きさも同じ順に付いてくる。
    #[test]
    fn a_bucket_holds_its_pixels_in_row_order_with_their_metrics_alongside() {
        let pixel = |x: u16, y: u16, face: u32, rank: u8| ProjPixel {
            x,
            y,
            face,
            sx: x as f32,
            sy: y as f32,
            weight: 1.0,
            position: Vec3::new(x as f32, y as f32, face as f32),
            rank,
        };
        let given = [
            (pixel(5, 2, 0, 0), 1.0),
            (pixel(3, 1, 7, 0), 2.0),
            (pixel(3, 1, 4, 2), 3.0),
            (pixel(9, 0, 1, 0), 4.0),
            (pixel(3, 1, 4, 1), 5.0),
        ];
        let sorted = Bucket {
            pixels: given.iter().map(|p| p.0).collect(),
            metrics: given.iter().map(|p| [p.1, 0.0, 0.0]).collect(),
            columns: given.iter().map(|p| [p.1, 0.0, 0.0, 0.0]).collect(),
        }
        .sorted();
        let order: Vec<(u16, u16, u32, u8)> = sorted
            .pixels
            .iter()
            .map(|p| (p.x, p.y, p.face, p.rank))
            .collect();
        assert_eq!(
            order,
            [
                (9, 0, 1, 0),
                (3, 1, 4, 1),
                (3, 1, 4, 2),
                (3, 1, 7, 0),
                (5, 2, 0, 0)
            ]
        );
        let metrics: Vec<f32> = sorted.metrics.iter().map(|m| m[0]).collect();
        assert_eq!(metrics, [4.0, 5.0, 3.0, 2.0, 1.0], "画面の大きさも同じ順");
        let columns: Vec<f32> = sorted.columns.iter().map(|m| m[0]).collect();
        assert_eq!(columns, metrics, "画面の動きも同じ順");
        // もう並んでいれば、そのまま
        let again = Bucket {
            pixels: sorted.pixels.clone(),
            metrics: sorted.metrics.clone(),
            columns: sorted.columns.clone(),
        }
        .sorted();
        assert_eq!(again.metrics, sorted.metrics);
        assert_eq!(again.columns, sorted.columns);
        // 画面の大きさを持たない投影の塗りは、空のまま
        let plain = Bucket {
            pixels: given.iter().map(|p| p.0).collect(),
            metrics: Vec::new(),
            columns: Vec::new(),
        }
        .sorted();
        assert!(plain.metrics.is_empty() && plain.columns.is_empty());
        assert_eq!(plain.pixels.len(), 5);
    }

    #[test]
    fn deterministic_cos_matches_the_library_closely() {
        for d in [0.0f64, 10.0, 45.0, 60.0, 80.0, 85.0, 90.0] {
            let want = d.to_radians().cos();
            assert!((deterministic_cos_degrees(d) - want).abs() < 1e-13, "{d}");
        }
    }

    #[test]
    fn spans_match_the_per_texel_test() {
        let t = super::super::SurfaceTriangle::new(
            Vec3::ZERO,
            Vec3::X,
            Vec3::Y,
            Vec2::new(0.1, 0.13),
            Vec2::new(0.83, 0.27),
            Vec2::new(0.4, 0.9),
        );
        let uv = UvFace::new(&t, 64, 48).unwrap();
        for y in 0..48 {
            let expected: Vec<i32> = (0..64).filter(|&x| UvFace::inside(uv.bary(x, y))).collect();
            let got: Vec<i32> = uv
                .span(y, 64)
                .map_or(Vec::new(), |(a, b)| (a..=b).collect());
            assert_eq!(got, expected, "row {y}");
        }
    }

    #[test]
    fn the_mask_fills_whole_words_and_tails() {
        let mut m = UvMask::new(200, 2);
        let stride = m.stride;
        UvMask::fill(&mut m.words[stride..2 * stride], 3, 150);
        for x in 0..200 {
            assert_eq!(m.get(x, 1), (3..=150).contains(&x), "{x}");
            assert!(!m.get(x, 0));
        }
    }

    /// 丸い筆先のアンチエイリアス: 帯は投影の画素ごとのテクセルの画面の大きさで直し（向きでも違う）、届く半径の外は 0。中心の 2 倍より
    /// 大きく写るテクセルでは、帯を届く半径に収まる幅で止める。テクセルの大きさを持たない画素は今の式。
    #[test]
    fn the_round_band_follows_each_pixels_texel_and_stops_at_the_reach() {
        use crate::brush::AntiAlias;
        let h = 1.0;
        let center = TexelMetric::from_columns((1.0, 0.0), (0.0, 1.0));
        let round = RoundCover::new(10.0, h).with_anti_alias(AntiAlias::Strong, Some(center));
        let reach = round.reach();
        // 中心の 2 倍まで帯が収まる: 半径 10 + 帯 2 × 2 / 2 + …
        assert!(reach > 11.9 && reach < 12.1, "{reach}");
        // テクセルが画面で 1 点（中心と同じ）: 帯は画面の 2 点。縁の 9 点と 11 点の間を smoothstep で
        let metric = |k: f32| [k * k, 0.0, k * k];
        let at = |x: f64, k: f32| round.cover(x, 0.0, 0, 0, metric(k));
        assert_eq!(at(8.9, 1.0), 1.0);
        assert!((at(10.0, 1.0) - 0.5).abs() < 1e-6);
        assert_eq!(at(11.0, 1.0), 0.0);
        // テクセルが画面で 0.5 点: 帯は画面の 1 点（縁の 9.5〜10.5）
        assert_eq!(at(9.45, 0.5), 1.0);
        assert!(at(10.25, 0.5) > 0.0 && at(10.55, 0.5) == 0.0);
        // 横だけ 0.5 点に縮むテクセル: 横の向きは帯 1 点、縦の向きは 2 点
        let squashed = [0.25f32, 0.0, 1.0];
        assert_eq!(round.cover(10.55, 0.0, 0, 0, squashed), 0.0);
        assert!(round.cover(0.0, 10.55, 0, 0, squashed) > 0.0);
        // 中心の 4 倍に写るテクセル: 帯は届く半径に収まる幅で止まり、届く半径の外は 0
        assert!(at(11.5, 4.0) > 0.0);
        assert_eq!(at(reach, 4.0), 0.0);
        assert_eq!(round.cover(0.0, reach + 0.01, 0, 0, metric(8.0)), 0.0);
        // テクセルの大きさを持たない画素（なし のストロークの投影の塗り）は今の式
        let old = RoundCover::new(10.0, h);
        for x in [0.0, 9.0, 9.99, 10.0, 10.5] {
            assert_eq!(
                round.cover(x, 0.0, 0, 0, [0.0; 3]),
                old.cover(x, 0.0, 0, 0, [0.0; 3])
            );
        }
    }
}
