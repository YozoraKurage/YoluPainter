//! Live Link のスタンドアロンの側（yolu-link-demo の作りを画面に載せたもの）。Unity のエディタの YoluPainter（Live Link の受け口）が
//! つなぎ、シーンのモデルを送ってくる。描いたテクスチャは共有メモリで返し、Unity が本物のマテリアルで見せる。
//!
//! - 起動時（設定で切れる）・--livelink・ファイル ▸ Live Link で待ち受ける（名前は既定で `yolupainter-livelink`、環境変数 `YOLUPAINTER_LINK_NAME` で替えられる）。Unity の
//!   ブリッジが挨拶すると、読める版を取り決めて返す。重ならなければ断り、状態の帯に版の不一致を出す。つなげる Unity は 1 つで、
//!   2 つ目は Busy で断る。
//! - モデル（Model）を受けたら、マテリアルごとにテクスチャセットを結び付け・作り（`sets`）、目を開いていて Unity が Color を見せられる
//!   （流し込み先のある）マテリアルのセットを共有メモリに出して知らせる。描いたら、変わったタイルだけを合成して共有メモリへ書き、
//!   知らせる（チャンネルは今は Color だけ。core の M1 が Color だけなので）。
//! - ポーズ（Pose）・マテリアルの更新（Materials）・モデルを閉じた（ModelClosed）は `AppState.model` に当てる（3D ビューが読む）。
//!
//! スレッド: 待ち受け（来たつながりを受けるだけ。止める合図を 50 ms ごとに見る）と、つながりごとの挨拶と読み・書き。画面のスレッドは
//! フレームの頭で知らせを読み（`poll`）、フレームの終わりに変わったタイルを出す（`publish`）。画面のスレッドはパイプに書かない（書く
//! スレッドへ渡すだけ）ので、Unity が遅くても描く手は止まらない。共有メモリへの書き込みも待たない（タイルごとの seqlock）。

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use yolu_protocol::host::PublishedSet;
use yolu_protocol::compat::{accepts_with, refusal_from_reject};
use yolu_protocol::link::{
    self, accept_as, error_message, negotiate, wrong_direction, LinkError,
};
use yolu_protocol::{
    channel, feature, shm::valid_tile_size, AppVersion, Connection, ErrorCode, Hello, Identity,
    Kind, LinkInfo, MaterialRequest, MaterialWant, Message, Product, Received,
    Reject, RejectCode, ServerKey, SkewReport, Tile, VersionRefusal, DEFAULT_LINK_NAME,
    MAX_REQUEST_ITEMS, MAX_TEXTURE_SIZE,
};

use crate::engine::{Channel, Document, RowOrder, TileCoord};
use crate::livelink_pose::PoseSlot;
use crate::livelink_queue::{message_bytes, pose_bytes, Backlog};
use crate::model::{ModelSource, SceneModel};
use crate::state::AppState;
use crate::lang::Lang;
use crate::view3d::model::ViewError;

/// 挨拶で名乗る名前。
pub const AGENT: &str = concat!("YoluPainter ", env!("CARGO_PKG_VERSION"));

/// このスタンドアロンが挨拶で出す機能の印（`yolu_protocol::feature`。双方の共通部分がそのつながりで使える機能）。
/// 印を立てる機能を足すときは、ここに `feature` のビットを足す（ビットの割り当ては `yolu_protocol::feature`）。
/// マテリアルの値（MATERIAL_VALUES）: Unity の本物の lilToon のマテリアルの値と描いていないスロットの絵を受けて描く（`look::link`）。
/// 元のテクスチャ（ORIGINAL_TEXTURES）: Unity が送る元の絵を、新しく作ったセット・何も触っていない最初のセットの一番下のレイヤーに入れる
/// （絵の無いマテリアルは白）（`livelink_base`）。
/// マテリアルの頼み（MATERIAL_REQUEST）: 元の絵を待たせたセットのマテリアルと、値が来ない・絵が揃わないマテリアルを、Unity に頼む。
/// 印が双方にあれば、Unity は元の絵を自分から押し出さず、頼まれたものだけを送る（手元の絵と印が同じなら画素なし）。
pub const FEATURES: u64 =
    feature::MATERIAL_VALUES | feature::ORIGINAL_TEXTURES | feature::MATERIAL_REQUEST;

/// Unity に出すチャンネル（セットの共有メモリ。今は Color だけ）。Unity はここにあるチャンネルの流し込み先だけを描いた絵で見せ、ほかの
/// 流し込み先は元のテクスチャのまま見せる（マテリアルの値で描くときも同じ決まり。`look::link`）。
pub const PUBLISHED_CHANNELS: &[u8] = &[channel::COLOR];

/// 挨拶の名乗り（名乗りの文字列・Cargo の版・出す機能の印）。
pub fn identity() -> Identity {
    Identity::standalone(AGENT)
        .with_version(AppVersion::parse(env!("CARGO_PKG_VERSION")))
        .with_features(FEATURES)
}

/// 2 つ目の Unity を断る理由。
const BUSY_TEXT: &str =
    "スタンドアロンの YoluPainter はほかの Unity とつながっています（つなげるのは 1 つ）。";

/// つながりの様子。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum LinkStatus {
    #[default]
    Off,
    Listening,
    Connected {
        agent: String,
        version: u16,
        session: u64,
    },
    /// 待ち受けられない（同じ名前で別のスタンドアロンが待ち受けている など）。
    Failed(String),
}

/// 知らせの重さ（状態の帯の色）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeLevel {
    Info,
    Warning,
    Error,
}

/// 画面が読む Live Link の様子の写し。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkView {
    pub name: String,
    pub status: LinkStatus,
    /// 最後の知らせ（つながった・切れた・断った・版の不一致など）。
    pub notice: Option<(NoticeLevel, String)>,
    /// Unity に出しているテクスチャセット（uid）。
    pub published: Vec<u32>,
    /// これまでに知らせたタイルの数（作り直しの全部を含む）。
    pub tiles_sent: u64,
    /// 最後に版が合わずに断った理由（つながる・やめるまで出す。プロトコルの版が重ならないときだけ）。
    pub mismatch: Option<String>,
    /// 断った理由の構造（どちらを何版以上に上げるか。画面の言語で文を作る）。
    pub refusal: Option<VersionRefusal>,
    /// つながっているあいだの、両側の名乗りと決まった版（版のずれ・使える機能を調べる）。
    pub link: Option<LinkInfo>,
}

impl Default for LinkView {
    fn default() -> Self {
        LinkView {
            name: DEFAULT_LINK_NAME.into(),
            status: LinkStatus::Off,
            notice: None,
            published: Vec::new(),
            tiles_sent: 0,
            mismatch: None,
            refusal: None,
            link: None,
        }
    }
}

impl LinkView {
    /// 待ち受けているか、つながっている。
    pub fn is_on(&self) -> bool {
        matches!(
            self.status,
            LinkStatus::Listening | LinkStatus::Connected { .. }
        )
    }

    /// 入口のアイコンのツールチップに出す短い文。
    pub fn summary(&self) -> String {
        self.summary_in(Lang::Ja)
    }

    pub fn summary_in(&self, lang: Lang) -> String {
        match &self.status {
            LinkStatus::Off => lang.pick("Live Link: 切っています", "Live Link: Off").into(),
            LinkStatus::Listening if self.mismatch.is_some() => lang
                .pick(
                    "Live Link: 版の合わない Unity を断りました",
                    "Live Link: Version mismatch",
                )
                .into(),
            LinkStatus::Listening => lang.pick(
                format!("Live Link: Unity を待っています（{}）", self.name),
                format!("Live Link: Waiting for Unity ({})", self.name),
            ),
            LinkStatus::Connected { version, .. } => lang.pick(
                format!(
                    "Live Link: Unity とつながっています（版 {version}・セット {}）",
                    self.published.len()
                ),
                format!(
                    "Live Link: Connected (v{version} · {} sets)",
                    self.published.len()
                ),
            ),
            LinkStatus::Failed(_) => lang
                .pick("Live Link: 待ち受けられません", "Live Link: Unavailable")
                .into(),
        }
    }

    /// 窓の先頭に出す状態の名前（名前だけ）。
    pub fn state_label(&self, lang: Lang) -> &'static str {
        match &self.status {
            LinkStatus::Off => lang.pick("切断", "Off"),
            LinkStatus::Listening if self.mismatch.is_some() => {
                lang.pick("版が合いません", "Version mismatch")
            }
            LinkStatus::Listening => lang.pick("待機中", "Waiting"),
            LinkStatus::Connected { .. } => lang.pick("接続中", "Connected"),
            LinkStatus::Failed(_) => lang.pick("待ち受けられません", "Unavailable"),
        }
    }

    /// 入口のアイコンの印の様子。
    pub fn indicator(&self) -> LinkIndicator {
        match &self.status {
            LinkStatus::Off => LinkIndicator::Off,
            LinkStatus::Failed(_) => LinkIndicator::Failed,
            LinkStatus::Listening if self.mismatch.is_some() => LinkIndicator::Mismatch,
            LinkStatus::Listening => LinkIndicator::Waiting,
            LinkStatus::Connected { .. } => match self.notice {
                Some((NoticeLevel::Error, _)) => LinkIndicator::Mismatch,
                _ if self.skew().is_some() => LinkIndicator::Skewed,
                _ => LinkIndicator::Connected,
            },
        }
    }

    /// つながっているあいだの版のずれ（警告に値するずれがあるときだけ）。
    pub fn skew(&self) -> Option<SkewReport> {
        if !matches!(self.status, LinkStatus::Connected { .. }) {
            return None;
        }
        self.link
            .as_ref()
            .map(LinkInfo::skew)
            .filter(SkewReport::is_skewed)
    }

    /// このつながりで使える機能の印（つながっていなければ 0）。
    pub fn common_features(&self) -> u64 {
        match self.status {
            LinkStatus::Connected { .. } => self.link.as_ref().map_or(0, LinkInfo::common_features),
            _ => 0,
        }
    }

    /// つながっている Unity の名前（挨拶の名乗りから。「(Unity 2022.3.22f1)」の形なら版の名前だけ）。つながっていなければ None。
    pub fn unity_name(&self) -> Option<String> {
        let LinkStatus::Connected { agent, .. } = &self.status else {
            return None;
        };
        let version = agent
            .find("(Unity ")
            .and_then(|at| {
                let rest = &agent[at + 1..];
                rest.find(')').map(|end| rest[..end].to_owned())
            })
            .filter(|name| !name.trim().is_empty());
        Some(version.unwrap_or_else(|| agent.clone()))
    }

    /// アイコンのツールチップ: 状態の文と、理由があれば（待ち受けられない・版が合わない・最後の知らせが誤り）その理由。
    pub fn tooltip(&self, lang: Lang) -> String {
        let summary = self.summary_in(lang);
        match (&self.status, &self.mismatch, &self.notice) {
            (LinkStatus::Failed(e), _, _) => format!("{summary}\n{e}"),
            (_, Some(m), _) => match &self.refusal {
                Some(refusal) => format!("{summary}\n{}", refusal_tooltip(lang, refusal)),
                None => format!("{summary}\n{m}"),
            },
            (_, _, Some((NoticeLevel::Error, n))) => format!("{summary}\n{n}"),
            _ => match self.skew() {
                Some(skew) => format!("{summary}\n{}", skew_tooltip(lang, &skew)),
                None => summary,
            },
        }
    }
}

/// 入口のアイコンの印の様子（切断は灰・待機中は薄い色・接続は緑・版の不一致と、つないだままの版のずれは警告の色・待ち受けられないは赤）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkIndicator {
    Off,
    Waiting,
    Connected,
    Mismatch,
    /// つながっているが、版か機能の印がずれている（警告の色。ツールチップに両方の版と、どちらを上げるか）。
    Skewed,
    Failed,
}

fn product_name(lang: Lang, product: Product) -> &'static str {
    match product {
        Product::Unity => lang.pick("Unity のパッケージ", "Unity package"),
        Product::Standalone => lang.pick("スタンドアロン", "standalone"),
    }
}

/// 機能の印の名前（名前を知らない印は「新しい機能」にまとめる）。
fn feature_names(lang: Lang, mask: u64) -> String {
    let mut names: Vec<&str> = feature::known_bits(mask)
        .into_iter()
        .map(|bit| match bit {
            feature::MATERIAL_VALUES => lang.pick("マテリアルの値", "Material values"),
            feature::ASSETS => lang.pick("アセット", "Assets"),
            feature::PROJECT_TRANSFER => lang.pick("プロジェクトの転送", "Project transfer"),
            feature::ORIGINAL_TEXTURES => lang.pick("元のテクスチャ", "Original textures"),
            feature::MATERIAL_REQUEST => lang.pick("マテリアルの頼み", "Material requests"),
            _ => lang.pick("アニメーション", "Animation"),
        })
        .collect();
    if mask & !feature::KNOWN != 0 {
        names.push(lang.pick("新しい機能", "Newer features"));
    }
    names.join(lang.pick("・", ", "))
}

/// 上げる製品と求める版の 1 行（「○○を 0.4.0 以上に上げる必要があります」。版の指定が無ければ「○○を更新する必要があります」）。
fn update_line(lang: Lang, product: Product, to: Option<AppVersion>) -> String {
    let what = product_name(lang, product);
    match to.filter(|v| !v.is_zero()) {
        Some(v) => lang.pick(
            format!("{what}を {v} 以上に上げる必要があります"),
            format!("The {what} must be {v} or newer"),
        ),
        None => lang.pick(
            format!("{what}を更新する必要があります"),
            format!("The {what} must be updated"),
        ),
    }
}

/// プロトコルの版が重ならず断ったときの、ツールチップの文（範囲と、どちらを何版以上に上げるか）。
fn refusal_tooltip(lang: Lang, refusal: &VersionRefusal) -> String {
    let (u, s) = (refusal.unity_range, refusal.standalone_range);
    format!(
        "{}\n{}",
        lang.pick(
            format!(
                "プロトコルの版が合いません（Unity 側 {}〜{}、スタンドアロン {}〜{}）",
                u.0, u.1, s.0, s.1
            ),
            format!(
                "Protocol versions do not match (Unity {}–{}, standalone {}–{})",
                u.0, u.1, s.0, s.1
            ),
        ),
        update_line(lang, refusal.update, refusal.to)
    )
}

/// つないだままの版のずれのツールチップの文: 両方の版・どちらを上げればよいか・使えない機能の名前。
fn skew_tooltip(lang: Lang, skew: &SkewReport) -> String {
    let own = skew
        .own_version
        .map_or_else(|| lang.pick("不明", "unknown").to_owned(), |v| v.to_string());
    let peer = skew
        .peer_version
        .map_or_else(|| lang.pick("不明", "unknown").to_owned(), |v| v.to_string());
    let mut lines = vec![lang.pick(
        format!("スタンドアロン {own}・Unity のパッケージ {peer}"),
        format!("Standalone {own} · Unity package {peer}"),
    )];
    // 自分はスタンドアロン、相手は Unity のパッケージ
    if skew.peer_should_update() {
        lines.push(update_line(lang, Product::Unity, skew.update_peer));
    }
    if skew.own_should_update() {
        lines.push(update_line(lang, Product::Standalone, skew.update_self));
    }
    let apart = skew.missing_on_peer | skew.missing_here;
    if apart != 0 {
        lines.push(format!(
            "{}: {}",
            lang.pick("使えない機能", "Unavailable"),
            feature_names(lang, apart)
        ));
    }
    lines.join("\n")
}

/// 始める・やめるの頼み（メニューから。`YoluApp` が当てる）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkRequest {
    Start,
    Stop,
}

/// 裏のスレッドからの知らせ。
enum Event {
    Connected {
        session: u64,
        hello: Hello,
        version: u16,
        out: Sender<Out>,
        /// 両側の名乗りと決まった版（使える機能・版のずれ）。
        link: Option<LinkInfo>,
        /// 読むスレッドが溜めるポーズ（メッシュごとに最新だけ）。
        pose: Arc<PoseSlot>,
    },
    /// 版が合わないので断った。
    Refused {
        text: String,
        refusal: Option<VersionRefusal>,
    },
    /// ほかの Unity とつながっているので断った。
    Busy {
        agent: String,
    },
    /// 鍵が無い・合わない挨拶を断った（古いブリッジ・別の鍵・別のユーザーのつなぎ）。
    Unauthorized {
        text: String,
    },
    HandshakeFailed(String),
    Message {
        session: u64,
        message: Message,
        /// 列に積んでいる間の大きさ（`livelink_queue::message_bytes`。取り出したときに帳簿から引く）。
        bytes: usize,
    },
    /// 読むスレッドがポーズを溜め始めた（束 `batch`。`PoseSlot::put`）。この知らせを列の順に読んだときに、その束のポーズを取り出して当てる。
    PoseReady {
        session: u64,
        batch: u64,
    },
    Unknown {
        session: u64,
        kind: u16,
    },
    Malformed {
        session: u64,
        kind: u16,
        text: String,
    },
    /// つながりが終わった（None は Unity の Bye）。
    Closed {
        session: u64,
        reason: Option<String>,
    },
}

/// 書くスレッドへ渡すもの。
enum Out {
    Message(Message),
    /// Bye を送って閉じる。
    Bye,
}

struct Listening {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for Listening {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

struct Active {
    session: u64,
    out: Sender<Out>,
    /// このつながりで使える機能の印（双方の共通部分）。
    common_features: u64,
    /// 読むスレッドが溜めたポーズ（画面のスレッドが、受けた順を守って取り出す）。
    pose: Arc<PoseSlot>,
}

impl Active {
    /// 送る。印の要る命令（`Kind::required_feature`）は、相手に印があるときだけ。
    fn send(&self, message: Message) {
        self.send_with(message, Kind::required_feature);
    }

    /// `send` の、命令の種類ごとに要る印の決め方を選べる形（試験が印の要る表を差し込む）。送ったら true。
    fn send_with(&self, message: Message, need_of: impl Fn(Kind) -> u64) -> bool {
        if !accepts_with(self.common_features, &message, need_of) {
            return false;
        }
        self.out.send(Out::Message(message)).is_ok()
    }
}

/// Unity に出しているテクスチャセット 1 つ。
struct Published {
    set: PublishedSet,
    /// 書いた文書（開き直しで替われば全部を書き直す）。
    doc_id: u128,
    /// 最後に書いた後の文書の変化の通し番号。
    since: u64,
}

/// Live Link（`YoluApp` が 1 つ持つ）。
pub struct LiveLink {
    name: String,
    status: LinkStatus,
    notice: Option<(NoticeLevel, String)>,
    mismatch: Option<String>,
    refusal: Option<VersionRefusal>,
    /// つながっているあいだの、両側の名乗りと決まった版。
    link_info: Option<LinkInfo>,
    tx: Sender<Event>,
    rx: Receiver<Event>,
    listening: Option<Listening>,
    /// つながっている Unity のつながりの番号（0 は無し）。待ち受けのスレッドが 2 つ目を断るのに使う。
    active_session: Arc<AtomicU64>,
    sessions: Arc<AtomicU64>,
    active: Option<Active>,
    published: BTreeMap<u32, Published>,
    /// 共有メモリを作れなかったセット（次のモデルまで作り直さない。毎フレーム試さない）。
    failed: BTreeSet<u32>,
    failed_for_model: u64,
    tiles_sent: u64,
    /// 受けたマテリアルの値（Unity の lilToon。`look::link`）。
    values: crate::look::link::LinkValues,
    /// 元の絵を待たせているセットと、受けた元の絵（`livelink_base`）。
    base: crate::livelink_base::LiveBase,
    /// 待ちの時間切れを見るため、待っているあいだ描き直しを頼む窓口（`start` で受け取る）。
    ctx: Option<egui::Context>,
    /// 読むスレッドが列へ積んだ命令の量（上限に達したら、読むスレッドは画面のスレッドが取り出すまで読まない）。
    backlog: Arc<Backlog>,
    /// Unity に出した頼みの数（試験・診断用）。
    requests_sent: u64,
    /// 頼めない理由を知らせた（このつながりで 1 度だけ）。
    request_unavailable_told: bool,
}

impl Default for LiveLink {
    fn default() -> Self {
        LiveLink::new()
    }
}

impl Drop for LiveLink {
    fn drop(&mut self) {
        if let Some(a) = self.active.take() {
            let _ = a.out.send(Out::Bye);
        }
        // 上限で待っている読むスレッドを終わらせる（列の受け手はもう無い）
        self.backlog.close();
    }
}

impl LiveLink {
    pub fn new() -> LiveLink {
        let (tx, rx) = mpsc::channel();
        let name = std::env::var("YOLUPAINTER_LINK_NAME")
            .ok()
            .filter(|n| link::valid_link_name(n))
            .unwrap_or_else(|| DEFAULT_LINK_NAME.to_owned());
        LiveLink {
            name,
            status: LinkStatus::Off,
            notice: None,
            mismatch: None,
            refusal: None,
            link_info: None,
            tx,
            rx,
            listening: None,
            active_session: Arc::new(AtomicU64::new(0)),
            sessions: Arc::new(AtomicU64::new(0)),
            active: None,
            published: BTreeMap::new(),
            failed: BTreeSet::new(),
            failed_for_model: 0,
            tiles_sent: 0,
            values: Default::default(),
            base: Default::default(),
            ctx: None,
            backlog: Backlog::new(),
            requests_sent: 0,
            request_unavailable_told: false,
        }
    }

    /// つなぎ先の名前（待ち受けていないときだけ替えられる。試験で重ならない名前にする）。
    pub fn set_name(&mut self, name: &str) -> Result<(), String> {
        if self.listening.is_some() {
            return Err("待ち受けている間は名前を替えません。".into());
        }
        if !link::valid_link_name(name) {
            return Err("つなぎ先の名前は 1〜64 文字の英数字と . _ - です。".into());
        }
        self.name = name.to_owned();
        Ok(())
    }

    pub fn status(&self) -> &LinkStatus {
        &self.status
    }

    /// 元の絵が入るまで Unity に出さずに待たせているセットの数（試験・診断用）。
    pub fn originals_waiting(&self) -> usize {
        self.base.waiting_count()
    }

    /// 画面に写す様子。
    pub fn view(&self) -> LinkView {
        LinkView {
            name: self.name.clone(),
            status: self.status.clone(),
            notice: self.notice.clone(),
            published: self.published.keys().copied().collect(),
            tiles_sent: self.tiles_sent,
            mismatch: self.mismatch.clone(),
            refusal: self.refusal,
            link: self.link_info.clone(),
        }
    }

    fn notify(&mut self, level: NoticeLevel, text: String, state: &mut AppState) {
        state.message = text.clone();
        self.notice = Some((level, text));
    }

    /// 頼みを当てる。
    pub fn request(&mut self, request: LinkRequest, ctx: &egui::Context, state: &mut AppState) {
        match request {
            LinkRequest::Start => self.start(ctx, state),
            LinkRequest::Stop => self.stop(state),
        }
    }

    /// 待ち受けを始める。
    pub fn start(&mut self, ctx: &egui::Context, state: &mut AppState) {
        if self.listening.is_some() {
            return;
        }
        let listener = match link::Server::bind(&self.name, true) {
            Ok(l) => l,
            Err(e) => {
                let text = state.lang.pick(format!("Live Link を「{}」で待ち受けられません: {e}", self.name), format!("Live Link unavailable at “{}”: {e}", self.name));
                self.status = LinkStatus::Failed(text.clone());
                self.notify(NoticeLevel::Error, text, state);
                return;
            }
        };
        self.ctx = Some(ctx.clone());
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let stop = stop.clone();
            let tx = self.tx.clone();
            let ctx = ctx.clone();
            let active = self.active_session.clone();
            let sessions = self.sessions.clone();
            let backlog = self.backlog.clone();
            let key = listener.key();
            thread::Builder::new()
                .name("yolu-livelink-listen".into())
                .spawn(move || {
                    while !stop.load(Ordering::Relaxed) {
                        match listener.accept() {
                            Ok(stream) => {
                                let session = sessions.fetch_add(1, Ordering::Relaxed) + 1;
                                let (tx, ctx, active) = (tx.clone(), ctx.clone(), active.clone());
                                let (key, backlog) = (key.clone(), backlog.clone());
                                let _ = thread::Builder::new()
                                    .name(format!("yolu-livelink-{session}"))
                                    .spawn(move || serve(stream, session, tx, ctx, active, key, backlog));
                            }
                            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                                thread::sleep(Duration::from_millis(50))
                            }
                            Err(_) => thread::sleep(Duration::from_millis(200)),
                        }
                    }
                })
                .expect("スレッドを作れる")
        };
        self.listening = Some(Listening {
            stop,
            thread: Some(thread),
        });
        self.status = LinkStatus::Listening;
        let text = state.lang.pick(format!("Live Link: Unity を待っています（{}）。", self.name), format!("Live Link: Waiting for Unity ({}).", self.name));
        self.notify(NoticeLevel::Info, text, state);
    }

    /// 待ち受けをやめ、つながっていれば切る（Bye を送る）。出したテクスチャセットの共有メモリは片付ける。
    pub fn stop(&mut self, state: &mut AppState) {
        self.listening = None; // 落とすと待ち受けのスレッドが止まる
        let was_connected = self.active.is_some();
        self.disconnect(state);
        self.status = LinkStatus::Off;
        self.mismatch = None;
        self.refusal = None;
        let text = if was_connected {
            state.lang.pick("Live Link を切りました。", "Live Link disconnected.")
        } else {
            state.lang.pick("Live Link の待ち受けをやめました。", "Live Link stopped.")
        };
        self.notify(NoticeLevel::Info, text.into(), state);
    }

    fn disconnect(&mut self, state: &mut AppState) {
        // 受けたばかりの値（同じフレームの、切れる前の命令）を、捨てる前にセットへ当てる（切っても最後の値が残る）
        if let Some(session) = self.current_session() {
            self.values.apply(state, session);
        }
        if let Some(a) = self.active.take() {
            let _ = a.out.send(Out::Bye);
            let _ = self.active_session.compare_exchange(
                a.session,
                0,
                Ordering::AcqRel,
                Ordering::Relaxed,
            );
            if let Some(m) = state.model.as_mut() {
                if m.source == (ModelSource::LiveLink { session: a.session }) {
                    m.live = false;
                }
            }
        }
        self.published.clear();
        self.failed.clear();
        self.link_info = None;
        self.request_unavailable_told = false;
        // 受けた値は捨てる（セットの文書に当てた受けた見た目は、最後の値として残る）
        self.values.clear();
        // 元の絵を待たせていたセットは出す（入れた元の絵の層は文書に残る）
        self.base.clear();
    }

    fn current_session(&self) -> Option<u64> {
        self.active.as_ref().map(|a| a.session)
    }

    /// 裏のスレッドからの知らせを読み、状態に当てる（フレームの頭で）。
    pub fn poll(&mut self, state: &mut AppState) {
        while let Ok(event) = self.rx.try_recv() {
            match event {
                Event::Connected {
                    session,
                    hello,
                    version,
                    out,
                    link,
                    pose,
                } => {
                    if self.listening.is_none() || self.active.is_some() {
                        // やめた後・つながっている間に来た（待ち受けのスレッドは 2 つ目を断るので、普通は来ない）
                        let _ = out.send(Out::Bye);
                        continue;
                    }
                    self.active = Some(Active {
                        session,
                        out,
                        common_features: link.as_ref().map_or(0, LinkInfo::common_features),
                        pose,
                    });
                    self.link_info = link;
                    self.status = LinkStatus::Connected {
                        agent: hello.agent.clone(),
                        version,
                        session,
                    };
                    self.mismatch = None;
                    self.refusal = None;
                    let text = state.lang.pick(format!(
                        "Live Link: Unity とつながりました（{}・プロトコルの版 {version}）。",
                        hello.agent
                    ), format!(
                        "Live Link: Connected ({} · protocol v{version}).",
                        hello.agent
                    ));
                    self.notify(NoticeLevel::Info, text, state);
                }
                Event::Refused { text, refusal } => {
                    // 理由の全文（版の範囲と、どちらを上げるか）は入口の印のツールチップに出す
                    self.mismatch = Some(text);
                    self.refusal = refusal;
                    self.notify(
                        NoticeLevel::Error,
                        state.lang.pick("Live Link: 版の合わない Unity を断りました。", "Live Link: Version mismatch.").into(),
                        state,
                    );
                }
                Event::Busy { agent } => {
                    let text = state.lang.pick(format!("Live Link: 2 つ目の Unity（{agent}）を断りました。"), format!("Live Link: Second Unity connection refused ({agent})."));
                    self.notify(NoticeLevel::Warning, text, state);
                }
                Event::Unauthorized { text } => {
                    // 理由は鍵の断りの文（古いブリッジ・別の鍵など）。つながっている Unity には影響しない
                    let text = state.lang.pick(
                        format!("Live Link: 鍵の合わない接続を断りました（{text}）。"),
                        format!("Live Link: Connection refused, key mismatch ({text})."),
                    );
                    self.notify(NoticeLevel::Warning, text, state);
                }
                Event::HandshakeFailed(e) => {
                    let text = state.lang.pick(format!("Live Link: つなぎ始めで失敗しました: {e}"), format!("Live Link: Handshake failed: {e}"));
                    self.notify(NoticeLevel::Warning, text, state);
                }
                Event::Message { session, message, bytes } => {
                    // 列から出したので、帳簿から引く（読むスレッドが上限で待っていれば、ここで再開する）
                    self.backlog.release(bytes);
                    if Some(session) == self.current_session() {
                        self.handle(message, state);
                    }
                }
                Event::PoseReady { session, batch } => {
                    // 読むスレッドが溜めたポーズ（メッシュごとの最新）を、この知らせの位置で当てる。列でこの知らせより前の命令（モデルなど）は、
                    // もう当てた。後ろの命令が来ていれば、束は列へ流れていて、ここでは取り出せない（`PoseSlot` の「順番」）
                    if Some(session) == self.current_session() {
                        if let Some(taken) = self.active.as_ref().and_then(|a| a.pose.take_batch(batch)) {
                            if taken.dropped > 0 {
                                let text = state.lang.pick(
                                    format!("Live Link: Unity からのポーズを {} 件捨てました（溜められる量を超えました）。", taken.dropped),
                                    format!("Live Link: Dropped {} poses from Unity (over the amount kept).", taken.dropped),
                                );
                                self.notify(NoticeLevel::Warning, text, state);
                            }
                            if !taken.pose.meshes.is_empty() {
                                self.handle(Message::Pose(taken.pose), state);
                            }
                        }
                    }
                }
                Event::Unknown { session, kind } => {
                    if Some(session) == self.current_session() {
                        let text = state.lang.pick(format!(
                            "Live Link: Unity からの知らない命令（種類 0x{kind:04x}）を断りました。"
                        ), format!(
                            "Live Link: Unknown Unity command (0x{kind:04x})."
                        ));
                        self.notify(NoticeLevel::Warning, text, state);
                    }
                }
                Event::Malformed {
                    session,
                    kind,
                    text,
                } => {
                    if Some(session) == self.current_session() {
                        // 元の絵が読めなかったら、どの絵が欠けたか分からない: 待たせているセットを全部出す
                        if kind == Kind::MaterialOriginal as u16 {
                            self.base.release_all();
                        }
                        let text = state.lang.pick(format!(
                            "Live Link: Unity からの命令（{}）を読めません: {text}",
                            link::kind_name(kind)
                        ), format!(
                            "Live Link: Invalid Unity command ({}): {text}",
                            link::kind_name(kind)
                        ));
                        self.notify(NoticeLevel::Warning, text, state);
                    }
                }
                Event::Closed { session, reason } => {
                    if Some(session) != self.current_session() {
                        continue;
                    }
                    self.disconnect(state);
                    self.status = if self.listening.is_some() {
                        LinkStatus::Listening
                    } else {
                        LinkStatus::Off
                    };
                    let (level, text) = match reason {
                        None => (
                            NoticeLevel::Info,
                            state.lang.pick("Live Link: Unity が切りました。", "Live Link: Unity disconnected.").to_owned(),
                        ),
                        Some(e) => (
                            NoticeLevel::Warning,
                            state.lang.pick(format!("Live Link: Unity とのつながりが切れました: {e}"), format!("Live Link: Connection lost: {e}")),
                        ),
                    };
                    self.notify(level, text, state);
                }
            }
        }
        // 受けたマテリアルの値を、付いたテクスチャセットへ当てる（変わったセット・文書が替わったセットだけ）
        if let Some(session) = self.current_session() {
            self.values.apply(state, session);
            // 揃った元の絵をセットの一番下へ入れる。入れられなかった理由は知らせる
            if let Some(text) = self.base.poll(state, session, Instant::now()) {
                self.notify(NoticeLevel::Warning, text, state);
            }
            // 元の絵を待たせたセット・値が来ないマテリアルを、Unity に頼む
            self.ask(state, Instant::now());
            // 届くのを待っているあいだは、時間切れを見るために描き直す
            if self.base.waiting() {
                if let Some(ctx) = &self.ctx {
                    ctx.request_repaint_after(Duration::from_millis(500));
                }
            }
        }
    }

    /// Unity に頼む: 元の絵を待たせているセットのマテリアルと、値が来ない・絵が揃わないマテリアル（頼みを出せるつながりのとき）。
    /// 頼みは今のモデルの世代に付ける。つながりに印（マテリアルの頼み）が無い古い Unity には頼まない: 元の絵は Unity が自分から送り、
    /// 値が来ないまま頼めないときだけ、理由（Unity のパッケージの版）を 1 度知らせる。
    fn ask(&mut self, state: &mut AppState, now: Instant) {
        let Some(session) = self.current_session() else {
            return;
        };
        let Some(model) = state
            .model
            .as_ref()
            .filter(|m| m.live && m.source == (ModelSource::LiveLink { session }))
        else {
            return;
        };
        let generation = model.generation;
        let common = self.active.as_ref().map_or(0, |a| a.common_features);
        let support = self.link_info.as_ref().map(LinkInfo::request_support);
        let can_ask = matches!(support, Some(Ok(())));
        let mut items: Vec<MaterialWant> = Vec::new();
        if can_ask {
            items.extend(self.base.take_requests());
        }
        if common & feature::MATERIAL_VALUES != 0 {
            // 値が来るはずのマテリアル: テクスチャセットが付いていて、マテリアルがあるもの（マテリアルの無い組は、値が無い）
            let bound: Vec<u32> = state
                .sets
                .iter()
                .filter_map(|s| s.bound)
                .filter(|m| {
                    model
                        .materials
                        .get(*m as usize)
                        .is_some_and(|info| info.key != yolu_protocol::MaterialKey::Unassigned)
                })
                .collect();
            let wanted = self.values.wanted(bound, now);
            if can_ask {
                items.extend(wanted.into_iter().map(MaterialWant::values));
            } else if !wanted.is_empty() && !self.request_unavailable_told {
                self.request_unavailable_told = true;
                if let Some(Err(reason)) = support {
                    let line = update_line(state.lang, Product::Unity, Some(reason.update_to));
                    let text = state.lang.pick(
                        format!("Live Link: Unity から値を取り直せません（{line}）。"),
                        format!("Live Link: Cannot ask Unity for the values again ({line})."),
                    );
                    self.notify(NoticeLevel::Info, text, state);
                }
            }
        }
        if items.is_empty() {
            return;
        }
        let Some(active) = &self.active else {
            return;
        };
        for chunk in items.chunks(MAX_REQUEST_ITEMS) {
            active.send(Message::MaterialRequest(MaterialRequest {
                generation,
                items: chunk.to_vec(),
            }));
            self.requests_sent += 1;
        }
    }

    /// Unity に出した頼みの数（試験・診断用）。
    pub fn requests_sent(&self) -> u64 {
        self.requests_sent
    }

    /// 読むスレッドが命令の列へ積んでいて、画面のスレッドがまだ取り出していない量（バイト。試験・診断用）。
    pub fn queued_bytes(&self) -> usize {
        self.backlog.queued()
    }

    /// 命令の列に積んでおく量の上限を決める（試験用。既定は `livelink_queue::MAX_QUEUED_BYTES`）。
    pub fn set_queue_limit(&mut self, bytes: usize) {
        self.backlog.set_limit(bytes);
    }

    /// 読むスレッドが溜めている、まだ当てていないポーズの量（位置と法線のバイト。試験・診断用）。
    pub fn pending_pose_bytes(&self) -> usize {
        self.active.as_ref().map_or(0, |a| a.pose.pending_bytes())
    }

    /// 値を頼むまでの猶予と頼み直す間隔を決める（試験用。既定は `look::link` の `ASK_AFTER`・`ASK_AGAIN`）。
    pub fn set_ask_timing(&mut self, after: Duration, again: Duration) {
        self.values.set_ask_timing(after, again);
    }

    /// 手元に残した元の絵（試験・診断用）。
    pub fn original_cache(&self) -> &crate::livelink_base::OriginalCache {
        self.base.cache()
    }

    /// Unity へ返す誤りの返事。表示の言語に依らず、プロトコルの診断として日本語の文に固定する
    /// （画面に出る知らせは `state.lang` で作る。返事を受ける Unity の側が、自分の言語で扱う）。
    fn reply_error(&self, code: ErrorCode, kind: u16, text: String) {
        if let Some(a) = &self.active {
            a.send(error_message(code, kind, text));
        }
    }

    fn handle(&mut self, message: Message, state: &mut AppState) {
        let Some(session) = self.current_session() else {
            return;
        };
        if let Some(reply) = wrong_direction(&message, true) {
            if let Some(a) = &self.active {
                a.send(reply);
            }
            return;
        }
        let ours = |m: &SceneModel| m.source == ModelSource::LiveLink { session };
        match message {
            Message::Model(model) => {
                // 同じつながりの 2 つ目以降のモデル（Unity が送り直した）は、3D ビューを前へ出し直さない
                let first = !state.model.as_ref().is_some_and(ours);
                // 何も触っていない最初のプロジェクトの最初のセットにも、元の絵を入れてよい（結び付ける前に見ておく）
                let untouched = state.is_pristine().then(|| state.sets.current().uid);
                let (report, shape) = state.receive_link_model(&model, session);
                self.failed.clear();
                self.values
                    .model(model.generation, model.materials.len() as u32);
                // 元の絵を送る Unity なら、入れてよいセット（今回作った・何も触っていない）を、元の絵が入るまで Unity に出さない
                // （前のモデルから待たせているセットのうち、文書が変わっていないものは持ち越す）
                let common = self.active.as_ref().map_or(0, |a| a.common_features);
                if common & feature::ORIGINAL_TEXTURES != 0 {
                    let mut fresh = report.created_sets.clone();
                    fresh.extend(untouched);
                    self.base.model(state, &model, &fresh, untouched, Instant::now());
                }
                let mut text = state.lang.pick(
                    format!("Live Link: モデル「{}」を受けました。", model.name),
                    format!("Live Link: Received the model “{}”.", model.name),
                );
                if shape.is_ok() && first {
                    // 届いたモデルは 3D ビューに出す（キャンバスが前にあれば 3D ビューのタブを前へ）
                    state.view3d.pose.focus = true;
                }
                if let Err(e) = shape {
                    let e = state.lang.view_error(&e);
                    text += &state.lang.pick(format!(" 3D ビューには出せません: {e}。"), format!(" Unavailable in 3D View: {e}."));
                }
                if !report.created.is_empty() {
                    text += &state.lang.pick(format!(" 新しいテクスチャセット: {}。", report.created.join("・")), format!(" New texture sets: {}.", report.created.join(", ")));
                }
                if !report.unmatched.is_empty() {
                    text += &state.lang.pick(format!(" モデルに無いセット: {}。", report.unmatched.join("・")), format!(" Sets not in this model: {}.", report.unmatched.join(", ")));
                }
                if let Some(t) = report.limit_text(state.lang) {
                    text += &format!(" {t}");
                }
                self.notify(NoticeLevel::Info, text, state);
            }
            Message::Pose(pose) => {
                // 3D ビューの形に当てる（描いている最中なら、終わってから）。合わないポーズは何も変えずに断る
                let result = if state.model.as_ref().is_some_and(ours) {
                    state.receive_link_pose(&pose)
                } else {
                    Err(ViewError::NoLinkModel)
                };
                if let Err(e) = result {
                    self.reply_error(ErrorCode::Refused, 0x0011, Lang::Ja.view_error(&e));
                }
            }
            Message::Materials(update) => match state.model.as_mut().filter(|m| ours(m)) {
                Some(m) => match m.apply_materials(&update) {
                    Ok(keys_changed) => {
                        if keys_changed {
                            state.bind_model();
                        }
                    }
                    Err(e) => self.reply_error(ErrorCode::Refused, 0x0012, e),
                },
                None => self.reply_error(
                    ErrorCode::Refused,
                    0x0012,
                    "モデルを受ける前のマテリアルの更新は使えません".into(),
                ),
            },
            Message::MaterialValues(values) => {
                // 値は今のつながりのモデルの世代のもの。合わない値は何も変えずに断る
                let generation = state
                    .model
                    .as_ref()
                    .filter(|m| ours(m))
                    .map(|m| m.generation);
                let result = match generation {
                    Some(_) => self.values.receive_values(values),
                    None => Err("モデルを受ける前のマテリアルの値は使えません".into()),
                };
                if let Err(e) = result {
                    self.reply_error(ErrorCode::Refused, Kind::MaterialValues as u16, e);
                }
            }
            Message::MaterialTexture(texture) => {
                let lang = state.lang;
                match self.values.receive_texture(texture, lang) {
                    Ok(()) => {}
                    // 命令の食い違いは Unity への返事だけ（開発の診断の文で、画面には出さない）
                    Err(crate::look::link::TextureRefused::Protocol(e)) => {
                        self.reply_error(ErrorCode::Refused, Kind::MaterialTexture as u16, e)
                    }
                    Err(crate::look::link::TextureRefused::OverBudget(e)) => {
                        self.notify(NoticeLevel::Warning, format!("Live Link: {e}"), state)
                    }
                }
            }
            Message::MaterialOriginal(original) => {
                // 元の絵は今のモデルの世代のもの。待たせていないマテリアルの絵は要らない。合わない命令は Unity への返事だけ
                if let Err(e) = self.base.receive(original, Instant::now()) {
                    self.reply_error(ErrorCode::Refused, Kind::MaterialOriginal as u16, e);
                }
            }
            Message::ModelClosed { generation } => {
                if state.model.as_ref().is_some_and(ours) && state.close_link_model(generation) {
                    self.notify(
                        NoticeLevel::Info,
                        state.lang.pick("Live Link: Unity がモデルを閉じました。", "Live Link: Unity closed the model.").into(),
                        state,
                    );
                }
            }
            Message::Error(e) => {
                let text = state.lang.pick(format!(
                    "Live Link: Unity からの誤りの知らせ（{}）: {}",
                    link::kind_name(e.kind),
                    e.text
                ), format!(
                    "Live Link: Unity error ({}): {}",
                    link::kind_name(e.kind),
                    e.text
                ));
                self.notify(NoticeLevel::Warning, text, state);
            }
            Message::Hello(_) => self.reply_error(
                ErrorCode::UnexpectedCommand,
                0x0001,
                "挨拶はつないだときだけです".into(),
            ),
            // Bye は読むスレッドが Closed にする。向きの違う命令は上で断った
            _ => {}
        }
    }

    /// 変わったタイルを共有メモリに書いて知らせる（フレームの終わりに）。出すセットの増減・世代の変化もここで合わせる。
    pub fn publish(&mut self, state: &mut AppState) {
        let Some(session) = self.current_session() else {
            return;
        };
        let model = state
            .model
            .as_ref()
            .filter(|m| m.live && m.source == ModelSource::LiveLink { session });
        let Some(model) = model else {
            let gone: Vec<u32> = std::mem::take(&mut self.published).into_keys().collect();
            if let Some(a) = &self.active {
                for uid in gone {
                    a.send(Message::TextureSetRemoved { set: uid });
                }
            }
            return;
        };
        if self.failed_for_model != model.revision {
            self.failed.clear();
            self.failed_for_model = model.revision;
        }
        let generation = model.generation;
        // 出すセット: 目が開いていて、マテリアルに付いていて、Unity が Color を見せられる
        let wanted: Vec<(usize, u32, u32)> = state
            .sets
            .iter()
            .enumerate()
            .filter_map(|(i, s)| {
                let m = s.bound?;
                let info = model.materials.get(m as usize)?;
                // 元の絵が入るまで待たせているセットは、まだ Unity に出さない（出してあるものは、そのまま）
                let held = self.base.holds(s.uid) && !self.published.contains_key(&s.uid);
                (s.visible
                    && !held
                    && info.routes.iter().any(|r| r.channel == channel::COLOR))
                .then_some((i, s.uid, m))
            })
            .collect();
        let gone: Vec<u32> = self
            .published
            .keys()
            .copied()
            .filter(|uid| !wanted.iter().any(|w| w.1 == *uid))
            .collect();
        let mut out = Vec::new();
        for uid in gone {
            self.published.remove(&uid);
            out.push(Message::TextureSetRemoved { set: uid });
        }
        let mut notes = Vec::new();
        for (index, uid, material) in wanted {
            let doc = state.set_doc(index);
            let name = state
                .sets
                .get(index)
                .map(|s| s.name.clone())
                .unwrap_or_default();
            let fits = self.published.get(&uid).is_some_and(|p| {
                p.set.width() == doc.width()
                    && p.set.height() == doc.height()
                    && p.set.tile_size() == doc.tile_size()
            });
            if !fits {
                self.published.remove(&uid);
                if self.failed.contains(&uid) {
                    continue;
                }
                match create(session, uid, generation, material, &name, doc, state.lang) {
                    Ok((p, tiles)) => {
                        out.push(p.set.announce());
                        self.tiles_sent += tiles as u64;
                        self.published.insert(uid, p);
                    }
                    Err(e) => {
                        self.failed.insert(uid);
                        notes.push(state.lang.pick(format!(
                            "Live Link: テクスチャセット「{name}」を Unity に出せません: {e}"
                        ), format!(
                            "Live Link: Cannot publish texture set “{name}”: {e}"
                        )));
                    }
                }
                continue;
            }
            let p = self.published.get_mut(&uid).expect("上で確かめた");
            match write_changes(p, doc, state.lang) {
                Ok(tiles) if !tiles.is_empty() => {
                    self.tiles_sent += tiles.len() as u64;
                    out.extend(p.set.tiles_changed(channel::COLOR, &tiles));
                }
                Ok(_) => {}
                Err(e) => notes.push(state.lang.pick(format!(
                    "Live Link: テクスチャセット「{name}」の共有メモリに書けません: {e}"
                ), format!(
                    "Live Link: Cannot update texture set “{name}”: {e}"
                ))),
            }
            // 世代・マテリアルの番号・名前が変わったら知らせ直す（ブリッジは知らせを受けると全部のタイルを読み直すので、中身を書いた後に）
            if p.set.generation != generation || p.set.material != material || p.set.name != name {
                p.set.generation = generation;
                p.set.material = material;
                p.set.name = name;
                out.push(p.set.announce());
            }
        }
        if let Some(active) = &self.active {
            for m in out {
                active.send(m);
            }
        }
        for n in notes {
            self.notify(NoticeLevel::Error, n, state);
        }
    }
}

/// 文書の全部を合成して共有メモリに書いた、新しい出しもの。返すのは書いたタイルの数。
#[allow(clippy::too_many_arguments)]
fn create(
    session: u64,
    uid: u32,
    generation: u32,
    material: u32,
    name: &str,
    doc: &Document,
    lang: Lang,
) -> Result<(Published, usize), String> {
    if doc.width() > MAX_TEXTURE_SIZE || doc.height() > MAX_TEXTURE_SIZE {
        return Err(lang.pick(format!(
            "大きさ {}×{} は Unity のテクスチャの上限 {MAX_TEXTURE_SIZE} を超えます",
            doc.width(),
            doc.height()
        ), format!(
            "Texture size {}×{} exceeds the Unity limit ({MAX_TEXTURE_SIZE})",
            doc.width(),
            doc.height()
        )));
    }
    if !valid_tile_size(doc.tile_size()) {
        return Err(lang.pick(format!(
            "タイルの大きさ {} は共有メモリで使えません（16〜1024 の 2 の冪）",
            doc.tile_size()
        ), format!(
            "Invalid shared tile size {} (power of two, 16–1024)",
            doc.tile_size()
        )));
    }
    let set = PublishedSet::create(
        session,
        uid,
        generation,
        material,
        name,
        doc.width(),
        doc.height(),
        doc.tile_size(),
        PUBLISHED_CHANNELS,
    )
    .map_err(|e| e.to_string())?;
    let mut p = Published {
        set,
        doc_id: doc.id(),
        since: doc.change_serial(),
    };
    let ts = doc.tile_size();
    let all: Vec<TileCoord> = (0..doc.height().div_ceil(ts))
        .flat_map(|y| (0..doc.width().div_ceil(ts)).map(move |x| TileCoord::new(x, y)))
        .collect();
    let n = write_tiles(&mut p, doc, &all, lang)?.len();
    Ok((p, n))
}

/// 前に書いた後に変わったタイルを書く（文書が替わっていれば全部）。返すのは書いたタイル。
fn write_changes(p: &mut Published, doc: &Document, lang: Lang) -> Result<Vec<Tile>, String> {
    let ts = doc.tile_size();
    let coords = if p.doc_id != doc.id() {
        None
    } else {
        doc.changed_tiles(Channel::Color, p.since)
    };
    let coords = coords.unwrap_or_else(|| {
        (0..doc.height().div_ceil(ts))
            .flat_map(|y| (0..doc.width().div_ceil(ts)).map(move |x| TileCoord::new(x, y)))
            .collect()
    });
    p.doc_id = doc.id();
    p.since = doc.change_serial();
    write_tiles(p, doc, &coords, lang)
}

fn write_tiles(
    p: &mut Published,
    doc: &Document,
    coords: &[TileCoord],
    lang: Lang,
) -> Result<Vec<Tile>, String> {
    let ts = doc.tile_size() as usize;
    let mut buf = Vec::new();
    let mut tiles = Vec::with_capacity(coords.len());
    let img = p
        .set
        .image_mut(channel::COLOR)
        .ok_or(lang.pick("Color の共有メモリがありません", "Color shared memory not found"))?;
    for c in coords {
        let Some(rect) = doc.tile_rect(*c) else {
            continue;
        };
        if rect.width == 0 || rect.height == 0 {
            continue;
        }
        buf.resize(rect.width as usize * rect.height as usize * 4, 0);
        doc.composite_into(Channel::Color, rect, &mut buf, RowOrder::BottomUp)
            .map_err(|e| lang.core_error(&e))?;
        let row = rect.width as usize * 4;
        img.write_tile(c.x, c.y, |slot| {
            for r in 0..rect.height as usize {
                slot[r * ts * 4..r * ts * 4 + row].copy_from_slice(&buf[r * row..(r + 1) * row]);
            }
        })
        .map_err(|e| e.to_string())?;
        tiles.push(Tile {
            x: c.x as u16,
            y: c.y as u16,
        });
    }
    Ok(tiles)
}

/// つながり 1 つ: 挨拶（版の取り決め。2 つ目の Unity は断る）、書くスレッドを立て、このスレッドで読み続ける。
fn serve(
    stream: interprocess::local_socket::Stream,
    session: u64,
    tx: Sender<Event>,
    ctx: egui::Context,
    active: Arc<AtomicU64>,
    key: Arc<ServerKey>,
    backlog: Arc<Backlog>,
) {
    // 読むスレッドが溜めるポーズ（メッシュごとに最新だけ。画面のスレッドが止まっていても膨らまない）。ポーズはほかの命令より先に適用される
    // 必要があるので、別の命令を列へ積む前に、溜めたポーズを先に列へ流す
    let pose_slot = Arc::new(PoseSlot::new());
    let wake = |e: Event| {
        if let Some(taken) = pose_slot.take() {
            if !taken.pose.meshes.is_empty() {
                let bytes = pose_bytes(&taken.pose);
                backlog.add(bytes);
                let _ = tx.send(Event::Message {
                    session,
                    message: Message::Pose(taken.pose),
                    bytes,
                });
            }
        }
        let _ = tx.send(e);
        ctx.request_repaint();
    };
    let release = || {
        let _ = active.compare_exchange(session, 0, Ordering::AcqRel, Ordering::Relaxed);
    };
    // 「つなげる Unity は 1 つ」の枠は、挨拶（鍵と版）が済んでから取る。挨拶を送らない接続や鍵の合わない接続が枠を塞がず、
    // つながっていることも、鍵を知っている相手にしか教えない。
    let busy_agent = std::cell::RefCell::new(String::new());
    let claim = |hello: &Hello| {
        active
            .compare_exchange(0, session, Ordering::AcqRel, Ordering::Relaxed)
            .map(|_| ())
            .map_err(|_| {
                *busy_agent.borrow_mut() = hello.agent.clone();
                Reject::plain(RejectCode::Busy, BUSY_TEXT)
            })
    };
    let (conn, mut reader, hello) = match accept_as(
        stream,
        &identity(),
        session,
        &key,
        link::HANDSHAKE_TIMEOUT,
        &claim,
    ) {
        Ok(x) => x,
        Err(LinkError::Rejected(r)) if r.code == RejectCode::Busy => {
            wake(Event::Busy {
                agent: busy_agent.take(),
            });
            return;
        }
        Err(LinkError::Rejected(r)) if r.code == RejectCode::Unauthorized => {
            release();
            wake(Event::Unauthorized { text: r.text });
            return;
        }
        Err(LinkError::Rejected(r)) => {
            release();
            let refusal = refusal_from_reject(Product::Standalone, &r);
            wake(Event::Refused {
                text: r.text,
                refusal,
            });
            return;
        }
        Err(e) => {
            release();
            wake(Event::HandshakeFailed(e.to_string()));
            return;
        }
    };
    let version = negotiate(&hello).unwrap_or(yolu_protocol::PROTOCOL_VERSION);
    let (out_tx, out_rx) = mpsc::channel::<Out>();
    let writer = conn.clone();
    let _ = thread::Builder::new()
        .name(format!("yolu-livelink-{session}-write"))
        .spawn(move || write_loop(writer, out_rx));
    let link = conn.link_info().cloned();
    wake(Event::Connected {
        session,
        hello,
        version,
        out: out_tx,
        link,
        pose: pose_slot.clone(),
    });
    loop {
        // 積んだ量が上限に達していたら、画面のスレッドが取り出すまで次の命令を読まない（ソケットとパイプが詰まる。溜まりはブリッジの送り待ちへ移る。`livelink_queue` の保証の射程）。今のつながりでなくなれば終える
        if !backlog.wait_for_room(|| active.load(Ordering::Acquire) == session) {
            release();
            break;
        }
        match reader.next(&conn) {
            Ok(Received::Message(Message::Bye)) => {
                // 自分から抜けて閉じる（Windows は受けの時間切れが無いので、待ち合わない）
                release();
                wake(Event::Closed {
                    session,
                    reason: None,
                });
                break;
            }
            Ok(Received::Message(Message::Pose(pose))) => {
                // 溜め場へ（最新だけ）。溜め場が空だったとき（新しい束）だけ、束の知らせを列へ積んで画面のスレッドを起こす
                if let Some(batch) = pose_slot.put(pose) {
                    let _ = tx.send(Event::PoseReady { session, batch });
                    ctx.request_repaint();
                }
            }
            Ok(Received::Message(message)) => {
                let bytes = message_bytes(&message);
                backlog.add(bytes);
                wake(Event::Message { session, message, bytes })
            }
            Ok(Received::Unknown(kind)) => wake(Event::Unknown { session, kind }),
            Ok(Received::Malformed(kind, e)) => wake(Event::Malformed {
                session,
                kind,
                text: e.to_string(),
            }),
            Ok(Received::Idle) => {}
            Err(e) => {
                release();
                wake(Event::Closed {
                    session,
                    reason: Some(e.to_string()),
                });
                break;
            }
        }
    }
}

fn write_loop(conn: Connection, rx: Receiver<Out>) {
    while let Ok(out) = rx.recv() {
        match out {
            Out::Message(m) => match conn.send(&m) {
                Ok(()) => {}
                // 枠の上限を超える命令は送らずに捨てる（つながりは保つ。書くスレッドを止めない）
                Err(e) if e.kind() == std::io::ErrorKind::InvalidInput => {}
                Err(_) => break,
            },
            Out::Bye => {
                let _ = conn.send(&Message::Bye);
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MARK_A: u64 = 1 << 40;
    const MARK_B: u64 = 1 << 41;

    /// 試験用の印の要る表（今の命令は印を要らないので、試験が差し込む）: テクスチャセットを消すのは A、モデルを閉じるのは A と B。
    fn need_of(kind: Kind) -> u64 {
        match kind {
            Kind::TextureSetRemoved => MARK_A,
            Kind::ModelClosed => MARK_A | MARK_B,
            _ => 0,
        }
    }

    fn active(common_features: u64) -> (Active, Receiver<Out>) {
        let (out, rx) = mpsc::channel();
        (
            Active {
                session: 1,
                out,
                common_features,
                pose: Arc::new(PoseSlot::new()),
            },
            rx,
        )
    }

    fn sent(rx: &Receiver<Out>) -> Vec<Kind> {
        rx.try_iter()
            .filter_map(|o| match o {
                Out::Message(m) => Some(m.kind()),
                Out::Bye => None,
            })
            .collect()
    }

    #[test]
    fn a_message_that_needs_a_mark_is_sent_only_when_the_link_has_it() {
        let removed = || Message::TextureSetRemoved { set: 1 };
        let closed = || Message::ModelClosed { generation: 1 };
        let bye = || Message::Bye;
        // 共通が A だけ: A を要るものは送る。A と B を要るものは送らない。印の要らないものは送る
        let (a, rx) = active(MARK_A);
        assert!(a.send_with(removed(), need_of));
        assert!(!a.send_with(closed(), need_of));
        assert!(a.send_with(bye(), need_of));
        assert_eq!(sent(&rx), vec![Kind::TextureSetRemoved, Kind::Bye]);
        // 共通が空: 印の要らないものだけ
        let (a, rx) = active(0);
        assert!(!a.send_with(removed(), need_of));
        assert!(!a.send_with(closed(), need_of));
        assert!(a.send_with(bye(), need_of));
        assert_eq!(sent(&rx), vec![Kind::Bye]);
        // 共通が A と B: 全部送る
        let (a, rx) = active(MARK_A | MARK_B);
        assert!(a.send_with(removed(), need_of) && a.send_with(closed(), need_of));
        assert_eq!(sent(&rx), vec![Kind::TextureSetRemoved, Kind::ModelClosed]);
        // 今の表（印の要らない）では、共通が空でも送る（今までどおり）
        let (a, rx) = active(0);
        a.send(closed());
        assert_eq!(sent(&rx), vec![Kind::ModelClosed]);
    }
}
