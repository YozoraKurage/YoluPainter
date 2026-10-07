//! レイヤーのパスの一覧（1 つのレイヤーに何本ものパス）。レイヤーの対象チャンネルの画素は、一覧の見せるパスを下から順に同じ作業面へ描いた結果
//! （後のパスが前のパスの上に重なる。消しゴムのパスは前のパスの画素を消す）。隠したパスは描かないが、点と設定は残る。
//!
//! 一覧のパスはどれも同じ側（2D のキャンバスか 3D のモデルの上）で、基準のチャンネルが同じ。3D のパスは同じモデル（指紋）に
//! 結び付く（描くとき 1 つの形を使う）。ID は一覧の中で重ならない（画面が選んだパスを ID で覚える）。

use super::render::Painter;
use super::{Error, Options, Rendered};
use crate::effects::LayerPath;
use crate::geometry::SurfaceGeometry;
use crate::Channel;

/// 1 つのレイヤーのパスの数の上限。
pub const MAX_LAYER_PATHS: usize = 256;
/// パスの名前の長さの上限（UTF-16 の単位）。
pub const MAX_PATH_NAME: usize = 128;

/// 一覧の 1 本（名前・表示・パス）。ID はパスの ID（[`LayerPathEntry::id`]）。
#[derive(Clone, Debug, PartialEq)]
pub struct LayerPathEntry {
    /// 一覧に見せる名前。空なら画面が並びの番号から名前を作る（古い文書の 1 本のパスは空で読む）。
    pub name: String,
    /// 描くか（隠したパスは画素に入らない）。
    pub visible: bool,
    pub path: LayerPath,
}

impl LayerPathEntry {
    /// 名前の無い、見せるパス。
    pub fn new(path: LayerPath) -> LayerPathEntry {
        LayerPathEntry {
            name: String::new(),
            visible: true,
            path,
        }
    }
    pub fn id(&self) -> u128 {
        self.path.id()
    }
    /// 名前（128 文字（UTF-16）まで、制御文字なし）とパスを確かめる。
    pub fn validate(&self) -> Result<(), Error> {
        if self.name.encode_utf16().count() > MAX_PATH_NAME
            || self.name.chars().any(char::is_control)
        {
            return Err(Error::Invalid(
                "パスの名前は 128 文字（UTF-16）まで、制御文字なしです",
            ));
        }
        match &self.path {
            LayerPath::Canvas(p) => p.validate(),
            LayerPath::Surface(p) => p.validate(),
        }
    }
    /// 履歴に積む大きさ（パスの状態と名前の UTF-16）。名前の無い 1 本は C# の SetPath と同じ大きさ。
    pub(crate) fn state_cost(&self) -> u64 {
        self.path.state_cost() + self.name.encode_utf16().count() as u64 * 2
    }
}

/// 一覧を確かめる: 256 本まで、どれも正しい、同じ側・同じ基準のチャンネル・3D は同じモデルの指紋、ID が重ならない。
pub fn validate_list(entries: &[LayerPathEntry]) -> Result<(), Error> {
    if entries.len() > MAX_LAYER_PATHS {
        return Err(Error::Invalid("1 つのレイヤーのパスは 256 本までです"));
    }
    for e in entries {
        e.validate()?;
    }
    let Some(first) = entries.first() else {
        return Ok(());
    };
    for (i, e) in entries.iter().enumerate() {
        if e.path.is_canvas() != first.path.is_canvas() {
            return Err(Error::Invalid(
                "1 つのレイヤーのパスは、どれもキャンバスの上か、どれもモデルの上です",
            ));
        }
        if e.path.channel() != first.path.channel() {
            return Err(Error::Invalid(
                "1 つのレイヤーのパスは、同じ基準のチャンネルを使います",
            ));
        }
        if let (LayerPath::Surface(a), LayerPath::Surface(b)) = (&e.path, &first.path) {
            if a.model_fingerprint != b.model_fingerprint {
                return Err(Error::Invalid(
                    "1 つのレイヤーの 3D のパスは、同じモデルに結び付きます",
                ));
            }
        }
        if entries[..i].iter().any(|o| o.id() == e.id()) {
            return Err(Error::Invalid("1 つのレイヤーのパスの ID が重なっています"));
        }
    }
    Ok(())
}

/// 一覧のリボンが読むアセットの画像（隠したパスのものも。初めて出た順）。アセットから消す前の確かめと、画像を読んでおくのに使う。
pub fn list_images(entries: &[LayerPathEntry]) -> Vec<crate::ImageId> {
    let mut out = Vec::new();
    for e in entries {
        if let super::PathKind::Ribbon(r) = e.path.style().kind {
            if !out.contains(&r.image) {
                out.push(r.image);
            }
        }
    }
    out
}

/// 一覧が描くチャンネル（見せないパスのものも。初めて出た順）。レイヤーの画素はこのチャンネルを描き直す。
pub fn list_channels(entries: &[LayerPathEntry]) -> Vec<Channel> {
    let mut out: Vec<Channel> = Vec::new();
    for e in entries {
        for c in e.path.channels() {
            if !out.contains(&c) {
                out.push(c);
            }
        }
    }
    out
}

/// 一覧を描く: 見せるパスを下から順に同じ作業面へ描き、[`list_channels`] のチャンネルごとに 1 つの面を返す（何も描かない
/// チャンネルは空の面）。3D のパスがあるなら `geometry`（休みの形）が要る。予算は一覧の全体。失敗・取消では何も返さない。
/// サンプル・ダブ・欠落の数は全部のパスの和。
pub fn render_list(
    entries: &[LayerPathEntry],
    geometry: Option<&SurfaceGeometry>,
    options: &Options<'_>,
) -> Result<Rendered, Error> {
    validate_list(entries)?;
    options.check()?;
    let mut painter = Painter::new(options)?;
    painter.ensure_channels(&list_channels(entries))?;
    let (mut samples, mut dabs, mut gaps) = (0, 0, 0);
    for e in entries {
        if !e.visible {
            continue;
        }
        match &e.path {
            LayerPath::Canvas(p) => {
                samples += super::render::draw_canvas(&mut painter, p)?;
            }
            LayerPath::Surface(p) => {
                let g = geometry.ok_or(Error::Invalid("3D のパスを描く形がありません"))?;
                super::surface::check_model(p, g)?;
                let (s, d, k) = super::surface::draw_surface(&mut painter, p, g, options)?;
                samples += s;
                dabs += d;
                gaps += k;
            }
        }
    }
    painter.finish(samples, dabs, gaps)
}
