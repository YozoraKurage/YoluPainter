//! 編集できる 2D/3D の筆跡。評価は独立した面を返し、文書・選択・履歴を変更しない。
pub mod bezier;
#[cfg(test)]
mod cancellation_tests;
mod fill;
mod list;
mod rebind;
mod render;
mod ribbon;
mod surface;
use crate::geometry::DabRefusal;
use crate::{BrushSettings, Channel, CoreError, Rgba8};
pub use list::{
    list_channels, list_images, render_list, validate_list, LayerPathEntry, MAX_LAYER_PATHS,
    MAX_PATH_NAME,
};
pub use rebind::{
    point_of, point_position, rebind_surface_path, rebind_tolerance, RebindError,
    REBIND_MAX_NODE_VISITS, REBIND_TOLERANCE_FRACTION,
};
pub use render::{render_canvas, Options, Rendered};
pub use surface::{fingerprint, render_surface};

pub const ALGORITHM_VERSION: u32 = 1;
pub const MAX_POINTS: usize = 4096;
pub const MAX_CANVAS_SAMPLES: usize = 4_000_000;

#[derive(Clone, Debug, PartialEq)]
pub enum Error {
    Invalid(&'static str),
    ModelMismatch,
    MissingTriangle,
    TooManySamples,
    Canceled,
    Core(CoreError),
    Dab(DabRefusal),
    /// リボンの画像が [`Options::images`] に無い（アセットに無い・まだ読んでいない）。
    MissingImage,
    /// 3D の塗りのパスの点・曲線が、1 つの UV の島に収まらない。
    FillIslands,
}
impl From<CoreError> for Error {
    fn from(e: CoreError) -> Self {
        Self::Core(e)
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(s) => f.write_str(s),
            Self::ModelMismatch => {
                f.write_str("モデルの三角形・UV・スロットの指紋が、パスを作ったときと違います")
            }
            Self::MissingTriangle => f.write_str("パスが参照する三角形がありません"),
            Self::TooManySamples => f.write_str("ブラシの間隔に対してパスが長すぎます"),
            Self::Canceled => f.write_str("パスの評価を取り消しました"),
            Self::Core(e) => e.fmt(f),
            Self::Dab(e) => e.fmt(f),
            Self::MissingImage => f.write_str("リボンの画像がありません"),
            Self::FillIslands => f.write_str("塗りのパスが 1 つの UV の島に収まっていません"),
        }
    }
}
impl std::error::Error for Error {}

/// 点の接線（曲線がその点をどう通るか）。取っ手は点からの向き（3D は休みの形のモデルの空間、2D は画素）。
///
/// 区間（点 s → s + 1）の両端が `Smooth` なら、曲線は今までどおり点を順に通る centripetal Catmull–Rom（同じ式・同じバイト）。
/// どちらかの端が `Corner` か `Handles` なら 3 次のベジェで、`Smooth` の端の制御点は Catmull–Rom と同じ向き・速さ、`Corner` の端は
/// 点そのもの（そこで折れる）、`Handles` の端は点 + 取っ手。
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Tangent<V> {
    #[default]
    Smooth,
    Corner,
    Handles {
        /// 前の区間からその点へ入る側の取っ手。
        incoming: V,
        /// その点から次の区間へ出る側の取っ手。
        outgoing: V,
    },
}

impl<V> Tangent<V> {
    pub fn is_smooth(&self) -> bool {
        matches!(self, Tangent::Smooth)
    }
}

/// 取っ手の長さの上限（2D は画素、3D はモデルの単位）。
pub const MAX_HANDLE: f64 = 1e6;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanvasPoint {
    pub x: f64,
    pub y: f64,
    pub pressure: f64,
    pub tangent: Tangent<glam::DVec2>,
}
impl CanvasPoint {
    /// 滑らかな点。
    pub fn new(x: f64, y: f64, pressure: f64) -> Result<Self, Error> {
        let p = Self {
            x,
            y,
            pressure,
            tangent: Tangent::Smooth,
        };
        p.validate()?;
        Ok(p)
    }
    pub(crate) fn validate(self) -> Result<(), Error> {
        if !self.x.is_finite()
            || !self.y.is_finite()
            || self.x.abs() > 1e6
            || self.y.abs() > 1e6
            || !(0.0..=1.0).contains(&self.pressure)
        {
            return Err(Error::Invalid(
                "2D の点は有限の ±1000000 画素、筆圧は 0..1 です",
            ));
        }
        if let Tangent::Handles { incoming, outgoing } = self.tangent {
            if [incoming, outgoing]
                .iter()
                .any(|h| !h.is_finite() || h.x.abs() > MAX_HANDLE || h.y.abs() > MAX_HANDLE)
            {
                return Err(Error::Invalid("取っ手は有限の ±1000000 です"));
            }
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PathPoint {
    pub triangle: u32,
    pub u: f64,
    pub v: f64,
    pub pressure: f64,
    pub tangent: Tangent<glam::Vec3>,
}
impl PathPoint {
    /// 滑らかな点。重心座標の `-1e-9` 以上 `0` 未満は 0 に丸める。直に組んだ値は丸められず、`SurfacePath::validate` が断る。
    pub fn new(triangle: u32, u: f64, v: f64, pressure: f64) -> Result<Self, Error> {
        if triangle > i32::MAX as u32
            || !u.is_finite()
            || !v.is_finite()
            || u < -1e-9
            || v < -1e-9
            || u + v > 1.0 + 1e-9
            || !(0.0..=1.0).contains(&pressure)
        {
            return Err(Error::Invalid("3D の点は三角形の内側、筆圧は 0..1 です"));
        }
        Ok(Self {
            triangle,
            u: u.max(0.0),
            v: v.max(0.0),
            pressure,
            tangent: Tangent::Smooth,
        })
    }
    /// 接線を替えた点。
    pub fn with_tangent(self, tangent: Tangent<glam::Vec3>) -> Self {
        Self { tangent, ..self }
    }
}
impl CanvasPoint {
    /// 接線を替えた点。
    pub fn with_tangent(self, tangent: Tangent<glam::DVec2>) -> Self {
        Self { tangent, ..self }
    }
}

impl<V: Copy> Tangent<V> {
    /// 向きを逆にしたパスでの接線（入る側と出る側を入れ替える）。
    pub fn reversed(self) -> Tangent<V> {
        match self {
            Tangent::Handles { incoming, outgoing } => Tangent::Handles {
                incoming: outgoing,
                outgoing: incoming,
            },
            other => other,
        }
    }
}

impl CanvasPath {
    /// 点の並びを逆にしたパス（取っ手の入る側と出る側も入れ替える。曲線の形は同じで、描く向きが逆）。
    pub fn reversed(&self) -> CanvasPath {
        CanvasPath {
            points: self
                .points
                .iter()
                .rev()
                .map(|p| p.with_tangent(p.tangent.reversed()))
                .collect(),
            ..self.clone()
        }
    }
}

impl SurfacePath {
    /// 点の並びを逆にしたパス（取っ手の入る側と出る側も入れ替える）。
    pub fn reversed(&self) -> SurfacePath {
        SurfacePath {
            points: self
                .points
                .iter()
                .rev()
                .map(|p| p.with_tangent(p.tangent.reversed()))
                .collect(),
            ..self.clone()
        }
    }
}
/// 半径は 2D では画素、3D ではモデルの空間。ほかの欄は通常の丸いブラシと共通。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PathBrush(pub BrushSettings);
impl Default for PathBrush {
    fn default() -> Self {
        Self(BrushSettings {
            radius: 0.01,
            ..Default::default()
        })
    }
}
impl PathBrush {
    fn validate(self, canvas: bool) -> Result<(), Error> {
        let b = self.0;
        if !b.radius.is_finite() || b.radius <= 0.0 || b.radius > if canvas { 4096.0 } else { 1e6 }
        {
            return Err(Error::Invalid("パスのブラシの半径が範囲外です"));
        }
        BrushSettings { radius: 1.0, ..b }.validate()?;
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChannelPaint {
    pub channel: Channel,
    pub color: Rgba8,
}
/// パスの種類（描き方）。
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum PathKind {
    /// 丸いブラシ（筆先）のストローク。
    #[default]
    Stroke,
    /// アセットの画像を、パスの向きに回した画像のダブとして並べる。
    Ribbon(Ribbon),
    /// 閉じたパスの内側を塗る（開いたパスは終わりから始めへ閉じて塗る）。3D は点が全部 1 つの UV の島にあるときだけ。
    Fill,
    /// 指先: 前の画素をパスに沿って引きずる（強さ 0〜1）。
    Smudge { strength: f64 },
    /// 消しゴム: 前の画素を消す（ブラシの `erase` と同じ）。
    Erase,
}

impl PathKind {
    /// 指先の既定（ブラシの指先と同じ 0.5）。
    pub const SMUDGE: PathKind = PathKind::Smudge { strength: 0.5 };
}

/// リボンの画像の並べ方。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum RibbonMode {
    /// 1 つのダブに画像の全部（画像の縦横の比のまま、幅をブラシの直径に合わせる）。
    #[default]
    Tile,
    /// パス全体に 1 枚を伸ばす: ダブ i に画像の [i/n, (i+1)/n] の区間。
    Stretch,
}

/// リボンの設定。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ribbon {
    /// アセットの画像（[`Options::images`] から読む）。
    pub image: crate::ImageId,
    pub mode: RibbonMode,
    /// 並べる間隔（ダブの長さに対する割合、0.1〜4。1 で隙間なく並ぶ。Tile だけに効く）。
    pub spacing: f64,
}

/// リボンの間隔の範囲。
pub const RIBBON_SPACING: std::ops::RangeInclusive<f64> = 0.1..=4.0;

/// 筆先の角度の範囲（度）。
pub const TIP_ANGLE: std::ops::RangeInclusive<f64> = -360.0..=360.0;
/// 投影の深さの範囲（ブラシの半径の倍数）。
pub const DEPTH: std::ops::RangeInclusive<f64> = 0.05..=64.0;

/// パスの対称。映した側は持たず、描くときに作って、元のパスのすぐ後に同じ作業面へ描く。
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum PathSymmetry {
    #[default]
    None,
    /// 2D: 画布の対称（鏡映・放射状。最初の恒等の写しは元のパス）。
    Canvas(crate::CanvasSymmetry),
    /// 3D: 休みの形のモデルの空間の鏡の面（面の上の点と法線）。映した点は、映した位置のいちばん近い面（同じマテリアル、
    /// 向きの合う面、許す距離の内側）へ置く。
    Mirror {
        point: glam::Vec3,
        normal: glam::Vec3,
    },
}

/// 1 本のパスの描き方の設定（種類・筆先・投影の深さ・対称）。既定は今までのパス（丸いブラシのストローク、自動の深さ）。
#[derive(Clone, Debug, PartialEq, Default)]
pub struct PathStyle {
    pub kind: PathKind,
    /// 筆先の画像（None は丸。ストローク・消しゴム・指先に効く）。画像の長い辺が直径にかかる。
    pub tip: Option<std::sync::Arc<crate::BrushTip>>,
    /// 筆先の角度（度、反時計回り）。`follow` ならパスの進む向きからの角度、そうでなければ 2D は画布の向き、3D はモデルの上（+Y を
    /// 面に落とした向き）からの角度。
    pub angle: f64,
    /// 筆先をパスの進む向きに回す。
    pub follow: bool,
    /// 3D の投影の深さ（曲線から法線の向きにどこまで面を探すか。ブラシの半径の倍数）。None は自動（ダブの間隔の 4 倍・区間の長さの
    /// 1/4・半径の 4 倍の大きい方）。
    pub depth: Option<f64>,
    /// 対称（映したパスも描く）。
    pub symmetry: PathSymmetry,
}

impl PathStyle {
    /// 既定（今までのパス）か。1 本のパスの並び（版 8・10・18）で表せる設定だけか（消しゴムはブラシの `erase` で表せる）。
    pub fn is_plain(&self) -> bool {
        matches!(self.kind, PathKind::Stroke | PathKind::Erase)
            && self.tip.is_none()
            && self.angle == 0.0
            && !self.follow
            && self.depth.is_none()
            && self.symmetry == PathSymmetry::None
    }
    fn validate(&self) -> Result<(), Error> {
        if !self.angle.is_finite() || !TIP_ANGLE.contains(&self.angle) {
            return Err(Error::Invalid("筆先の角度は -360〜360 度です"));
        }
        if self
            .depth
            .is_some_and(|d| !d.is_finite() || !DEPTH.contains(&d))
        {
            return Err(Error::Invalid("投影の深さは半径の 0.05〜64 倍です"));
        }
        match self.symmetry {
            PathSymmetry::None => {}
            PathSymmetry::Canvas(c) => {
                if !c.enabled() || c.validate().is_err() {
                    return Err(Error::Invalid("パスの対称の種類・中心・数が範囲外です"));
                }
            }
            PathSymmetry::Mirror { point, normal } => {
                if !point.is_finite()
                    || !normal.is_finite()
                    || normal.length_squared() < 1e-12
                    || point.abs().max_element() > 1e6
                {
                    return Err(Error::Invalid("パスの鏡の面が有限でないか、法線が 0 です"));
                }
            }
        }
        match self.kind {
            PathKind::Ribbon(r) => {
                if r.image.0 == 0 || !r.spacing.is_finite() || !RIBBON_SPACING.contains(&r.spacing)
                {
                    return Err(Error::Invalid(
                        "リボンの画像と間隔（0.1〜4）を確かめてください",
                    ));
                }
            }
            PathKind::Smudge { strength } => {
                if !(0.0..=1.0).contains(&strength) {
                    return Err(Error::Invalid("指先の強さは 0..1 です"));
                }
            }
            PathKind::Stroke | PathKind::Fill | PathKind::Erase => {}
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CanvasPath {
    pub id: u128,
    pub channel: Channel,
    pub brush: PathBrush,
    pub points: Vec<CanvasPoint>,
    pub material: Option<Vec<ChannelPaint>>,
    pub style: PathStyle,
}
#[derive(Clone, Debug, PartialEq)]
pub struct SurfacePath {
    pub id: u128,
    pub channel: Channel,
    pub brush: PathBrush,
    pub points: Vec<PathPoint>,
    pub model_fingerprint: String,
    pub material: Option<Vec<ChannelPaint>>,
    pub style: PathStyle,
}
fn paints(
    channel: Channel,
    brush: PathBrush,
    material: &Option<Vec<ChannelPaint>>,
) -> Result<Vec<ChannelPaint>, Error> {
    if !channel.is_standard() {
        return Err(Error::Invalid("パスは標準の 6 チャンネルを使います"));
    }
    let fallback = [ChannelPaint {
        channel,
        color: brush.0.color,
    }];
    let p = material.as_deref().unwrap_or(&fallback);
    if p.is_empty() || p.len() > 6 {
        return Err(Error::Invalid("パスの組は 1..6 チャンネルです"));
    }
    let mut seen = 0u64;
    for m in p {
        if !m.channel.is_standard() || seen & (1 << m.channel.index()) != 0 {
            return Err(Error::Invalid("パスのチャンネルが不正か重複しています"));
        }
        seen |= 1 << m.channel.index();
    }
    Ok(p.to_vec())
}
impl CanvasPath {
    pub fn validate(&self) -> Result<(), Error> {
        self.brush.validate(true)?;
        self.style.validate()?;
        if matches!(self.style.symmetry, PathSymmetry::Mirror { .. }) {
            return Err(Error::Invalid("2D のパスの対称は画布の対称です"));
        }
        paints(self.channel, self.brush, &self.material)?;
        if self.points.len() > MAX_POINTS {
            return Err(Error::Invalid("パスの点は 4096 個までです"));
        }
        for p in &self.points {
            p.validate()?;
        }
        Ok(())
    }
}
impl SurfacePath {
    pub fn validate(&self) -> Result<(), Error> {
        self.brush.validate(false)?;
        self.style.validate()?;
        if matches!(self.style.symmetry, PathSymmetry::Canvas(_)) {
            return Err(Error::Invalid("3D のパスの対称はモデルの鏡の面です"));
        }
        paints(self.channel, self.brush, &self.material)?;
        if self.points.len() > MAX_POINTS {
            return Err(Error::Invalid("パスの点は 4096 個までです"));
        }
        if self.model_fingerprint.is_empty() || self.model_fingerprint.encode_utf16().count() > 128
        {
            return Err(Error::Invalid("モデルの指紋は 1..128 文字です"));
        }
        for p in &self.points {
            let normalized = PathPoint::new(p.triangle, p.u, p.v, p.pressure)?;
            if normalized.with_tangent(p.tangent) != *p {
                return Err(Error::Invalid(
                    "重心座標が未正規化です（-1e-9 以上 0 未満は 0 に丸める）",
                ));
            }
            if let Tangent::Handles { incoming, outgoing } = p.tangent {
                if [incoming, outgoing]
                    .iter()
                    .any(|h| !h.is_finite() || h.abs().max_element() as f64 > MAX_HANDLE)
                {
                    return Err(Error::Invalid("取っ手は有限の ±1000000 です"));
                }
            }
        }
        Ok(())
    }
}
