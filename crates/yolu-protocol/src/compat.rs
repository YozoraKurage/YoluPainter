//! 互いの版と機能の印の取り決め（挨拶で聞き合う）。つないだまま、版のずれを知らせるための材料。
//!
//! - **アプリの版**（`AppVersion`）: スタンドアロンと Unity のパッケージは別の製品で、版の番号も別（スタンドアロン 0.1.0 と
//!   Unity のパッケージ 0.3.0 は互いに新しくも古くもない）。そこで版は直接比べず、**各側が「これ以上の相手」と宣言する版**
//!   （`min_peer`）と比べる。互換を壊す変更を入れるとき、その変更が入る版へ宣言を上げれば、古い相手には警告が出る。
//!   プロトコルの版（`PROTOCOL_VERSION`）が重なるかどうかとは別の話で、重ならなければ今までどおり断る。
//! - **機能の印**（`feature`）: 双方が出す印の共通部分が、このつながりで使える機能。新しい命令は、相手の印が立っているときだけ送る
//!   （`Kind::required_feature`・`Connection::send_gated`）。印は後ろに足すだけで、知らない印は読み飛ばす。
//! - 挨拶の後ろの欄（`VersionInfo`）は、鍵の欄と同じく「無い相手（古い版）」とも今までどおりつながる。無い相手の版は不明として、
//!   上げるのを勧める。
//!
//! 守れないこと: 挨拶の証し（`auth`）はこれらの欄を覆わない。同じユーザーのほかのプログラムは鍵を読めるので、版や印を偽れても
//! 区別できない（偽って得るのは警告を出さないことだけ）。

use std::fmt;

use crate::message::{Kind, Message};

/// アプリの版（major.minor.patch。プレリリースの識別子は読み捨てる）。
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct AppVersion {
    pub major: u16,
    pub minor: u16,
    pub patch: u16,
}

impl AppVersion {
    /// 「要求なし」（一番古い版）。
    pub const ZERO: AppVersion = AppVersion::new(0, 0, 0);

    pub const fn new(major: u16, minor: u16, patch: u16) -> AppVersion {
        AppVersion {
            major,
            minor,
            patch,
        }
    }

    /// 「1.2.3」「1.2.3-rc.1」「1.2」を読む（各部分は 0〜65535 の数）。
    pub fn parse(text: &str) -> Option<AppVersion> {
        let core = text.trim().split(['-', '+']).next()?;
        let mut parts = core.split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next()?.parse().ok()?;
        let patch = match parts.next() {
            Some(p) => p.parse().ok()?,
            None => 0,
        };
        if parts.next().is_some() {
            return None;
        }
        Some(AppVersion::new(major, minor, patch))
    }

    pub fn is_zero(self) -> bool {
        self == AppVersion::ZERO
    }

    /// 1 つの数に詰める（C の関数の値。`unpack` で戻す）。
    pub fn pack(self) -> u64 {
        ((self.major as u64) << 32) | ((self.minor as u64) << 16) | self.patch as u64
    }

    /// `pack` の逆。`NONE`（不明）は None。
    pub fn unpack(packed: u64) -> Option<AppVersion> {
        (packed != NONE_PACKED).then_some(AppVersion::new(
            (packed >> 32) as u16,
            (packed >> 16) as u16,
            packed as u16,
        ))
    }

    /// 不明な版を C の関数で返す値。
    pub const NONE_PACKED: u64 = NONE_PACKED;
}

const NONE_PACKED: u64 = u64::MAX;

impl fmt::Display for AppVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// 版の欄の値（`Option<AppVersion>` を詰める）。
pub fn pack_version(v: Option<AppVersion>) -> u64 {
    v.map_or(NONE_PACKED, AppVersion::pack)
}

/// 機能の印（挨拶の `features` の各ビット。割り当ては変えない）。
pub mod feature {
    /// マテリアルの値（lilToon などのパラメーター）の受け渡し。
    pub const MATERIAL_VALUES: u64 = 1 << 0;
    /// アセットの受け渡し。
    pub const ASSETS: u64 = 1 << 1;
    /// プロジェクトの転送。
    pub const PROJECT_TRANSFER: u64 = 1 << 2;
    /// アニメーション。
    pub const ANIMATION: u64 = 1 << 3;
    /// 元のテクスチャの受け渡し（Unity が元の絵を送り、スタンドアロンが新しく作ったテクスチャセットの一番下に入れる）。
    pub const ORIGINAL_TEXTURES: u64 = 1 << 4;
    /// マテリアルの値・元の絵の頼み（スタンドアロンが Unity に「このマテリアルの値・元の絵がほしい」と頼み、Unity は頼まれた元の絵だけを送る。
    /// 印が双方に立つと、Unity は元の絵を自分から押し出さない）。頼みを受ける（Unity）側も、頼む（スタンドアロン）側も、同じ印を立てる。
    pub const MATERIAL_REQUEST: u64 = 1 << 5;
    /// この表にある印の全部（これ以外のビットは、新しい相手が足した、名前を知らない機能）。
    pub const KNOWN: u64 = MATERIAL_VALUES
        | ASSETS
        | PROJECT_TRANSFER
        | ANIMATION
        | ORIGINAL_TEXTURES
        | MATERIAL_REQUEST;
    /// 名前を知っている印を、ビットの小さい順に取り出す。
    pub fn known_bits(mask: u64) -> Vec<u64> {
        (0..64)
            .map(|i| 1u64 << i)
            .filter(|bit| mask & bit != 0 && KNOWN & bit != 0)
            .collect()
    }
}

/// 頼み（`feature::MATERIAL_REQUEST`）を出す・受けるようになった版（スタンドアロンも Unity のパッケージも同じ番号）。印の無い相手を
/// 「この版以上に上げると頼める」と案内する。互換を壊す変更ではない（印の無い相手とは、頼まずに今までどおりつながる）ので、`MIN_*` は上げない。
pub const REQUEST_SINCE: AppVersion = AppVersion::new(0, 4, 0);

/// この版のスタンドアロンが求める Unity のパッケージの一番古い版（今は要求なし）。互換を壊す変更を入れるとき、その変更が入る版へ上げる。
pub const MIN_UNITY_PACKAGE: AppVersion = AppVersion::ZERO;
/// この版の Unity のパッケージ（ブリッジ）が求めるスタンドアロンの一番古い版（今は要求なし）。
pub const MIN_STANDALONE: AppVersion = AppVersion::ZERO;

/// この版の読めるプロトコルの版の範囲。
const DEFAULT_PROTOCOL_RANGE: (u16, u16) = (
    crate::message::MIN_PROTOCOL_VERSION,
    crate::message::PROTOCOL_VERSION,
);

/// 製品（版の番号が別）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Product {
    /// Unity のパッケージ（ブリッジの側）。
    Unity,
    Standalone,
}

/// 挨拶の後ろの欄: 自分の版と、「これ以上の相手」と宣言する版。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct VersionInfo {
    pub app: AppVersion,
    pub min_peer: AppVersion,
}

/// 版の範囲の断りの詳しい中身（`Reject` の後ろの欄）。断った側の読める範囲と、断った側が求める相手の版、
/// 断られた側が挨拶で言った範囲と求める版（言っていなければ `ZERO`）。この欄だけで、どちらを何版以上に上げるかを組み立てられる。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RejectDetail {
    pub min_version: u16,
    pub max_version: u16,
    pub min_peer: AppVersion,
    pub peer_min_version: u16,
    pub peer_max_version: u16,
    pub peer_min_peer: AppVersion,
}

/// 自分の名乗り。挨拶に載せる。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub product: Product,
    /// 名乗りの文字列（ログ用）。
    pub agent: String,
    /// 自分のアプリの版（分からなければ None。版の欄は送らない）。
    pub app_version: Option<AppVersion>,
    /// 「これ以上の相手」と宣言する版。
    pub min_peer: AppVersion,
    /// 自分が出す機能の印。
    pub features: u64,
    /// 自分が読めるプロトコルの版の範囲（既定はこの版の範囲。試験が重ならない範囲の相手を作るために選べる）。
    pub protocol: (u16, u16),
}

impl Identity {
    /// スタンドアロンの名乗り（版は分かるなら `with_version`、機能の印は `with_features`）。
    pub fn standalone(agent: &str) -> Identity {
        Identity {
            product: Product::Standalone,
            agent: agent.to_owned(),
            app_version: None,
            min_peer: MIN_UNITY_PACKAGE,
            features: 0,
            protocol: DEFAULT_PROTOCOL_RANGE,
        }
    }

    /// Unity のブリッジの名乗り。
    pub fn unity(agent: &str) -> Identity {
        Identity {
            product: Product::Unity,
            agent: agent.to_owned(),
            app_version: None,
            min_peer: MIN_STANDALONE,
            features: 0,
            protocol: DEFAULT_PROTOCOL_RANGE,
        }
    }

    pub fn with_version(mut self, version: Option<AppVersion>) -> Identity {
        self.app_version = version;
        self
    }

    pub fn with_features(mut self, features: u64) -> Identity {
        self.features = features;
        self
    }

    pub fn with_min_peer(mut self, min_peer: AppVersion) -> Identity {
        self.min_peer = min_peer;
        self
    }

    /// 読めるプロトコルの版の範囲を選ぶ（`min <= max`）。実際のアプリは既定のまま。
    pub fn with_protocol_range(mut self, min: u16, max: u16) -> Identity {
        self.protocol = (min.min(max), max);
        self
    }

    /// 挨拶の後ろの欄（版を知らなければ送らない）。
    pub fn version_info(&self) -> Option<VersionInfo> {
        self.app_version.map(|app| VersionInfo {
            app,
            min_peer: self.min_peer,
        })
    }

    /// 自分が読める版の範囲。
    pub fn protocol_range(&self) -> (u16, u16) {
        self.protocol
    }
}

/// 相手の名乗り（挨拶から）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeerInfo {
    pub agent: String,
    /// None は版の欄を送らない古い相手。
    pub versions: Option<VersionInfo>,
    pub features: u64,
}

impl PeerInfo {
    pub fn app_version(&self) -> Option<AppVersion> {
        self.versions.map(|v| v.app)
    }
}

/// 挨拶が済んだつながりについての、両側の名乗りと決まった版。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkInfo {
    /// これから使うプロトコルの版。
    pub protocol: u16,
    pub own: Identity,
    pub peer: PeerInfo,
}

impl LinkInfo {
    /// このつながりで使える機能（双方の印の共通部分）。
    pub fn common_features(&self) -> u64 {
        self.own.features & self.peer.features
    }

    pub fn has_feature(&self, feature: u64) -> bool {
        feature != 0 && self.common_features() & feature == feature
    }

    /// 命令を送ってよいか（命令が要る印が、相手にも立っているか）。
    pub fn accepts(&self, message: &Message) -> bool {
        accepts(self.common_features(), message)
    }

    /// このつながりで頼み（`Kind::MaterialRequest`）を出せるか。出せないなら理由（相手の印が無い。相手を何版以上に上げると頼めるか）。
    /// 印の無い古い相手には頼まない（相手は知らない命令を `Error` で断るので、先に控える）。
    pub fn request_support(&self) -> Result<(), RequestUnavailable> {
        if self.has_feature(feature::MATERIAL_REQUEST) {
            return Ok(());
        }
        Err(RequestUnavailable {
            peer_version: self.peer.app_version(),
            update_to: REQUEST_SINCE,
            own_has_mark: self.own.features & feature::MATERIAL_REQUEST != 0,
        })
    }

    /// 版のずれ。
    pub fn skew(&self) -> SkewReport {
        let peer_version = self.peer.app_version();
        let update_peer = match peer_version {
            // 版の欄を送らない相手は、この仕組みより古い。上げるのを勧める（求める版があれば添える）
            None => Some(self.own.min_peer),
            Some(v) if v < self.own.min_peer => Some(self.own.min_peer),
            Some(_) => None,
        };
        let update_self = match (self.own.app_version, self.peer.versions) {
            (Some(own), Some(peer)) if own < peer.min_peer => Some(peer.min_peer),
            _ => None,
        };
        SkewReport {
            own_version: self.own.app_version,
            peer_version,
            update_peer,
            update_self,
            missing_on_peer: self.own.features & !self.peer.features,
            missing_here: self.peer.features & !self.own.features,
        }
    }
}

/// 頼みを出せない理由（`LinkInfo::request_support`）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RequestUnavailable {
    /// 相手の版（版を名乗らない古い相手は None）。
    pub peer_version: Option<AppVersion>,
    /// 相手をこの版以上に上げると頼める。
    pub update_to: AppVersion,
    /// こちらに印があるか（無ければ、こちらが頼みを知らない）。
    pub own_has_mark: bool,
}

/// 要る印 `need` が `common`（双方の印の共通部分）に全部立っているか。印の要らない（0）ときは常に true。
/// 印の要る命令を送ってよいかの確かめは、全部ここを通る（`accepts`・`Connection::send_requiring`・ブリッジの送り口・アプリの送り口）。
pub fn satisfies(common: u64, need: u64) -> bool {
    common & need == need
}

/// 命令が要る印が `common` に立っているか。印の要らない命令は常に送ってよい。
pub fn accepts(common: u64, message: &Message) -> bool {
    accepts_with(common, message, Kind::required_feature)
}

/// `accepts` の、命令の種類ごとに要る印の決め方を選べる形（`need_of`）。試験が印の要る表を差し込んで、「どの命令がどの印を要るか」から
/// 送らない決めまでを確かめる。実際の送り口は `Kind::required_feature` を渡す（今は MaterialValues・MaterialTexture が MATERIAL_VALUES を、MaterialOriginal が ORIGINAL_TEXTURES を、MaterialRequest が MATERIAL_REQUEST を要る）。
pub fn accepts_with(common: u64, message: &Message, need_of: impl Fn(Kind) -> u64) -> bool {
    satisfies(common, need_of(message.kind()))
}

/// つないだまま見せる、版のずれ（警告の材料）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SkewReport {
    pub own_version: Option<AppVersion>,
    pub peer_version: Option<AppVersion>,
    /// 相手を上げるべき: 相手の版が自分の求める版に満たない（版を名乗らない古い相手を含む）。値は求める版（`ZERO` は指定なし）。
    pub update_peer: Option<AppVersion>,
    /// 自分を上げるべき: 自分の版が、相手の求める版に満たない。値は求める版。
    pub update_self: Option<AppVersion>,
    /// 自分にあって相手に無い機能（相手を上げれば使える）。
    pub missing_on_peer: u64,
    /// 相手にあって自分に無い機能（自分を上げれば使える。名前を知らない印を含む）。
    pub missing_here: u64,
}

impl SkewReport {
    /// 警告を出すほどのずれがあるか。
    pub fn is_skewed(&self) -> bool {
        self.update_peer.is_some()
            || self.update_self.is_some()
            || self.missing_on_peer != 0
            || self.missing_here != 0
    }

    /// 相手を上げると解けるずれがあるか。
    pub fn peer_should_update(&self) -> bool {
        self.update_peer.is_some() || self.missing_on_peer != 0
    }

    /// 自分を上げると解けるずれがあるか。
    pub fn own_should_update(&self) -> bool {
        self.update_self.is_some() || self.missing_here != 0
    }
}

/// プロトコルの版が重ならないときの断り（どちらを、どの版以上に）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct VersionRefusal {
    /// 上げるべき製品。
    pub update: Product,
    /// その製品の求める版（分からない・要求なしは None）。
    pub to: Option<AppVersion>,
    /// 読める版の範囲（Unity 側・スタンドアロン）。
    pub unity_range: (u16, u16),
    pub standalone_range: (u16, u16),
}

impl VersionRefusal {
    /// 断りの文（日本語と英語を並べる。古い相手がそのまま見せても読める。画面は構造から自分の言語で作る）。
    /// 上げる製品と版は「○○を X 以上に上げる必要があります / The ○○ must be X or newer」の理由の形で書く（指示の文にしない）。
    pub fn text(&self) -> String {
        let (ja_what, en_what) = match self.update {
            Product::Unity => ("Unity のパッケージ", "Unity package"),
            Product::Standalone => ("スタンドアロン", "standalone"),
        };
        let (ja_need, en_need) = match self.to.filter(|v| !v.is_zero()) {
            Some(v) => (
                format!("{ja_what}を {v} 以上に上げる必要があります。"),
                format!("The {en_what} must be {v} or newer."),
            ),
            None => (
                format!("{ja_what}を更新する必要があります。"),
                format!("The {en_what} must be updated."),
            ),
        };
        format!(
            "プロトコルの版が合いません（Unity 側 {}〜{}、スタンドアロン {}〜{}）。{ja_need} / \
             Protocol versions do not match (Unity {}–{}, standalone {}–{}). {en_need}",
            self.unity_range.0,
            self.unity_range.1,
            self.standalone_range.0,
            self.standalone_range.1,
            self.unity_range.0,
            self.unity_range.1,
            self.standalone_range.0,
            self.standalone_range.1,
        )
    }
}

/// 重ならない版の範囲をどう直すか。`own` が自分、`peer_*` が相手の挨拶から。重なるなら None。
pub fn judge_ranges(
    own: &Identity,
    peer_range: (u16, u16),
    peer_min_peer: Option<AppVersion>,
) -> Option<VersionRefusal> {
    judge(
        own.product,
        own.protocol_range(),
        own.min_peer,
        peer_range,
        peer_min_peer,
    )
}

/// `judge_ranges` の部品（断った側 `sender` の製品・範囲・求める版と、断られた側の範囲・求める版）。
pub fn judge(
    sender: Product,
    sender_range: (u16, u16),
    sender_min_peer: AppVersion,
    receiver_range: (u16, u16),
    receiver_min_peer: Option<AppVersion>,
) -> Option<VersionRefusal> {
    if sender_range.0.max(receiver_range.0) <= sender_range.1.min(receiver_range.1) {
        return None;
    }
    let (receiver_product, sender_product) = match sender {
        Product::Unity => (Product::Standalone, Product::Unity),
        Product::Standalone => (Product::Unity, Product::Standalone),
    };
    // 断った側の一番新しい版が、相手の一番古い版より古い → 断った側を上げる（相手の求める版へ）。そうでなければ相手が古い（断った側の求める版へ）
    let (update, to) = if sender_range.1 < receiver_range.0 {
        (sender_product, receiver_min_peer)
    } else {
        (receiver_product, Some(sender_min_peer))
    };
    let (unity_range, standalone_range) = match sender {
        Product::Unity => (sender_range, receiver_range),
        Product::Standalone => (receiver_range, sender_range),
    };
    Some(VersionRefusal {
        update,
        to: to.filter(|v| !v.is_zero()),
        unity_range,
        standalone_range,
    })
}

/// 受けた（送った）断りの欄から、どちらを何版以上に上げるか。`sender` は断った側の製品。版の範囲の断りで、詳しい欄があるときだけ
/// （古い相手の断り・版の断りでないものは None）。
pub fn refusal_from_reject(
    sender: Product,
    reject: &crate::message::Reject,
) -> Option<VersionRefusal> {
    if reject.code != crate::message::RejectCode::VersionMismatch {
        return None;
    }
    let d = reject.detail?;
    judge(
        sender,
        (d.min_version, d.max_version),
        d.min_peer,
        (d.peer_min_version, d.peer_max_version),
        Some(d.peer_min_peer),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_parse_pack_and_order() {
        assert_eq!(AppVersion::parse("0.3.0"), Some(AppVersion::new(0, 3, 0)));
        assert_eq!(
            AppVersion::parse("1.2.3-rc.1+build5"),
            Some(AppVersion::new(1, 2, 3))
        );
        assert_eq!(AppVersion::parse("2.10"), Some(AppVersion::new(2, 10, 0)));
        for bad in ["", "1", "a.b.c", "1.2.3.4", "1.2.x", "70000.0.0", "-1.0.0"] {
            assert_eq!(AppVersion::parse(bad), None, "{bad:?}");
        }
        let v = AppVersion::new(1, 20, 300);
        assert_eq!(AppVersion::unpack(v.pack()), Some(v));
        assert_eq!(AppVersion::unpack(AppVersion::NONE_PACKED), None);
        assert_eq!(pack_version(None), AppVersion::NONE_PACKED);
        assert!(AppVersion::new(0, 3, 0) < AppVersion::new(0, 10, 0));
        assert_eq!(v.to_string(), "1.20.300");
    }

    #[test]
    fn known_feature_bits_are_listed_in_order() {
        assert_eq!(
            feature::known_bits(feature::ANIMATION | feature::MATERIAL_VALUES | 1 << 40),
            vec![feature::MATERIAL_VALUES, feature::ANIMATION]
        );
    }
}
