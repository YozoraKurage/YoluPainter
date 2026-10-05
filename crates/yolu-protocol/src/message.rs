//! 命令の種類と中身。
//!
//! 版の決まり:
//! - つないだら Unity 側（ブリッジ）が `Hello` で読める版の範囲を言い、スタンドアロンが両方の読める一番新しい版を `Welcome` で返す。
//!   重ならなければ `Reject`（理由 `VersionMismatch`）を返して閉じる。
//! - プロトコルの版が重なる相手とは、アプリの版や機能の印がずれていてもつなぐ（つないだまま警告する。`compat`）。挨拶（`Hello`・`Welcome`）の
//!   後ろに足した版の欄・機能の印の共通部分で、新しい命令を送ってよいかを決める（`Kind::required_feature`）。
//! - 同じ版の中で変えてよいのは、命令の中身の**後ろに欄を足す**ことだけ（古い読み手は残りを読み飛ばす）。欄の意味を変える・途中に
//!   足す・消すときは、新しい種類の命令にするか版を上げる。
//! - 知らない種類の命令は、受けた側が `Error`（`UnknownCommand`、その種類の番号）を返して捨て、つながりは保つ。読めない中身
//!   （短い・上限を超える・決まりに合わない）は `Error`（`Malformed`）を返して捨てる。向きの違う命令（Unity 側に `Model` が来た
//!   など）は `Error`（`UnexpectedCommand`）。
//!
//! 数はリトルエンディアン、文字列は長さ（u32）と UTF-8。位置は Unity の座標（左手系、メートル）で、読み込んだモデルの根の
//! ゲームオブジェクトのローカルの空間。三角形の巻きは、鏡に映した（行列式が負の）レンダラーでも表を同じ向きにそろえて送る。

use crate::compat::{AppVersion, RejectDetail, VersionInfo};
use crate::wire::{DecodeError, Reader, Writer};

/// この版のプロトコル。
pub const PROTOCOL_VERSION: u16 = 1;
/// 読める一番古い版。
pub const MIN_PROTOCOL_VERSION: u16 = 1;

/// 名前・鍵などの文字列の上限（バイト）。
pub const MAX_NAME_BYTES: usize = 1024;
/// 共有メモリのファイルのパスの上限（バイト）。
pub const MAX_PATH_BYTES: usize = 4096;
/// 1 つのモデルのマテリアルの数の上限。
pub const MAX_MATERIALS: usize = 1024;
/// 1 つのモデルのメッシュ（レンダラー）の数の上限。
pub const MAX_MESHES: usize = 4096;
/// 1 つのメッシュの頂点の数の上限。
pub const MAX_VERTICES: usize = 1 << 24;
/// 1 つのサブメッシュの添字の数の上限。
pub const MAX_INDICES: usize = 3 << 24;
/// 1 つのメッシュのサブメッシュの数の上限。
pub const MAX_SUBMESHES: usize = 256;
/// 1 つのマテリアルのテクスチャのプロパティの数の上限。
pub const MAX_TEXTURE_PROPERTIES: usize = 256;
/// 1 つの TilesChanged のタイルの数の上限。
pub const MAX_TILES_PER_MESSAGE: usize = 1 << 16;
/// テクスチャセットの幅・高さの上限（Unity のテクスチャの上限）。
pub const MAX_TEXTURE_SIZE: u32 = 16384;
/// 1 つのマテリアルの値（MaterialValues）のプロパティの数の上限（lilToon 2.3 は約 600。スタンドアロンの見た目の設定の上限と同じ）。
pub const MAX_VALUE_PROPERTIES: usize = 2048;
/// MaterialValues のキーワードの数の上限。
pub const MAX_VALUE_KEYWORDS: usize = 256;
/// MaterialValues のスロット（テクスチャのプロパティ）の数の上限。
pub const MAX_VALUE_SLOTS: usize = 256;
/// MaterialValues のプロパティ・キーワード・スロットの名前の長さの上限（バイト。UTF-16 の 128 文字が収まる）。
pub const MAX_VALUE_NAME_BYTES: usize = 512;
/// 描いていないスロットの絵（MaterialTexture）の辺の上限（送る側が縮めてから送る）。
pub const MAX_SLOT_TEXTURE_SIZE: u32 = 2048;
/// 元の絵（MaterialOriginal）の辺の上限（Unity 版の画像の上限 `ImageContent.MaxSide` と同じ。送る側はこれを超える絵を縮めずに断る）。
pub const MAX_ORIGINAL_SIZE: u32 = 8192;

/// 命令の種類（枠の頭に入る番号）。番号は変えない。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(u16)]
pub enum Kind {
    Hello = 0x0001,
    Bye = 0x0002,
    Model = 0x0010,
    Pose = 0x0011,
    Materials = 0x0012,
    ModelClosed = 0x0013,
    MaterialValues = 0x0014,
    MaterialTexture = 0x0015,
    MaterialOriginal = 0x0016,
    Welcome = 0x0101,
    Reject = 0x0102,
    TextureSet = 0x0110,
    TextureSetRemoved = 0x0111,
    TilesChanged = 0x0112,
    Error = 0x0200,
}

impl Kind {
    pub fn from_u16(v: u16) -> Option<Kind> {
        Some(match v {
            0x0001 => Kind::Hello,
            0x0002 => Kind::Bye,
            0x0010 => Kind::Model,
            0x0011 => Kind::Pose,
            0x0012 => Kind::Materials,
            0x0013 => Kind::ModelClosed,
            0x0014 => Kind::MaterialValues,
            0x0015 => Kind::MaterialTexture,
            0x0016 => Kind::MaterialOriginal,
            0x0101 => Kind::Welcome,
            0x0102 => Kind::Reject,
            0x0110 => Kind::TextureSet,
            0x0111 => Kind::TextureSetRemoved,
            0x0112 => Kind::TilesChanged,
            0x0200 => Kind::Error,
            _ => return None,
        })
    }
    /// この命令を送るのに要る機能の印（相手にも立っているときだけ送る。要らなければ 0）。新しい命令を足すときは、ここに行を足す。
    pub fn required_feature(self) -> u64 {
        match self {
            Kind::Hello
            | Kind::Bye
            | Kind::Model
            | Kind::Pose
            | Kind::Materials
            | Kind::ModelClosed
            | Kind::Welcome
            | Kind::Reject
            | Kind::TextureSet
            | Kind::TextureSetRemoved
            | Kind::TilesChanged
            | Kind::Error => 0,
            Kind::MaterialValues | Kind::MaterialTexture => crate::compat::feature::MATERIAL_VALUES,
            Kind::MaterialOriginal => crate::compat::feature::ORIGINAL_TEXTURES,
        }
    }
    /// 誰が送る命令か。
    pub fn direction(self) -> Direction {
        match self {
            Kind::Hello
            | Kind::Model
            | Kind::Pose
            | Kind::Materials
            | Kind::ModelClosed
            | Kind::MaterialValues
            | Kind::MaterialTexture
            | Kind::MaterialOriginal => Direction::ToStandalone,
            Kind::Welcome
            | Kind::Reject
            | Kind::TextureSet
            | Kind::TextureSetRemoved
            | Kind::TilesChanged => Direction::ToUnity,
            Kind::Bye | Kind::Error => Direction::Both,
        }
    }
}

/// 命令の向き。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Direction {
    /// Unity（ブリッジ）→ スタンドアロン。
    ToStandalone,
    /// スタンドアロン → Unity。
    ToUnity,
    Both,
}

/// Unity 版のチャンネル（C# の PaintChannel・yolu-core の Channel と同じ番号）。
pub mod channel {
    pub const COLOR: u8 = 0;
    pub const ROUGHNESS: u8 = 1;
    pub const METALLIC: u8 = 2;
    pub const HEIGHT: u8 = 3;
    /// 合成ではなく Normal の出力（接空間・OpenGL の Y+・不透明。R = x・G = y・B = z・A = 255）。
    pub const NORMAL: u8 = 4;
    pub const EMISSION: u8 = 5;
    pub const COUNT: u8 = 6;
}

/// 挨拶の鍵の欄（`auth` を見よ）: 乱数の nonce と、鍵で作った証し。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HelloAuth {
    pub nonce: [u8; crate::auth::NONCE_BYTES],
    pub proof: [u8; crate::auth::PROOF_BYTES],
}

/// つないだ直後の挨拶（Unity → スタンドアロン）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hello {
    /// 読める版の範囲。
    pub min_version: u16,
    pub max_version: u16,
    /// 送り手の名前と版（ログ用。例: "YoluPainter Unity bridge abi 1"）。
    pub agent: String,
    /// 機能の印（`compat::feature`。双方の共通部分がそのつながりで使える機能）。
    pub features: u64,
    /// 鍵を知っている証し（後ろに足した欄。無いのは鍵を知らない古いブリッジで、スタンドアロンは断る）。
    pub auth: Option<HelloAuth>,
    /// 自分のアプリの版と、求める相手の版（鍵の欄のさらに後ろに足した欄。無いのは版を名乗らない古いブリッジ。鍵の欄が無ければ書かない）。
    pub versions: Option<VersionInfo>,
}

/// 挨拶への返事（スタンドアロン → Unity）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Welcome {
    /// これから使う版。
    pub version: u16,
    pub agent: String,
    /// このつながりの番号（共有メモリのファイルの名前に入る）。
    pub session: u64,
    pub features: u64,
    /// スタンドアロンも鍵を知っている証し（後ろに足した欄。無いのは鍵を確かめない古いスタンドアロンで、ブリッジは使わない）。
    pub proof: Option<[u8; crate::auth::PROOF_BYTES]>,
    /// 自分のアプリの版と、求める相手の版（証しの欄のさらに後ろに足した欄。無いのは版を名乗らない古いスタンドアロン。証しが無ければ書かない）。
    pub versions: Option<VersionInfo>,
}

/// 断りの理由。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u16)]
pub enum RejectCode {
    /// 読める版が重ならない。
    VersionMismatch = 1,
    /// ほかの Unity とつながっている。
    Busy = 2,
    /// 鍵が無い・合わない（古いブリッジ、別の待ち受けの鍵、ほかのユーザー）。
    Unauthorized = 3,
    /// それ以外。
    Other = 0xffff,
}

impl RejectCode {
    pub fn from_u16(v: u16) -> RejectCode {
        match v {
            1 => RejectCode::VersionMismatch,
            2 => RejectCode::Busy,
            3 => RejectCode::Unauthorized,
            _ => RejectCode::Other,
        }
    }
}

/// つながりを断る（スタンドアロン → Unity。送った側は閉じる）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reject {
    pub code: RejectCode,
    pub text: String,
    /// 版の範囲が重ならない断り（`VersionMismatch`）の構造。理由の文の後ろに足した欄で、相手が自分の言語で文を作る材料
    /// （断る側の読める範囲と、断る側が求める相手の版）。無いのは古い相手か、版の断りでないもの。
    pub detail: Option<RejectDetail>,
}

impl Reject {
    /// 版の断り以外の断り（詳しい欄なし）。
    pub fn plain(code: RejectCode, text: impl Into<String>) -> Reject {
        Reject {
            code,
            text: text.into(),
            detail: None,
        }
    }
}

/// モデルのマテリアルの組を指す鍵（Unity 版の .ylp の形式 7 の `material` と同じ考え方）。
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum MaterialKey {
    /// マテリアルの無いスロットの全部。
    Unassigned,
    /// マテリアル。アセットなら GUID（小文字の 16 進 32 文字）と localFileId。
    Material {
        name: String,
        asset: Option<(String, i64)>,
    },
}

/// シェーダーの 2D テクスチャのプロパティと、マテリアルに今入っているテクスチャの大きさ（無ければ 0）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextureProperty {
    pub name: String,
    pub width: u32,
    pub height: u32,
}

/// Unity 側が見せられるチャンネルと、その流し込み先のプロパティ（YoluPainter の流し込みの決まりで決めたもの）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelRoute {
    pub channel: u8,
    pub property: String,
}

/// マテリアルの組 1 つの情報。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaterialInfo {
    pub key: MaterialKey,
    /// シェーダーの名前（マテリアルが無ければ空）。
    pub shader: String,
    pub textures: Vec<TextureProperty>,
    /// Unity 側が見せられるチャンネル（ここに無いチャンネルを描いても Unity には見えない）。
    pub routes: Vec<ChannelRoute>,
}

/// 三角形の組 1 つ（1 つのマテリアルで描く）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Submesh {
    /// モデルのマテリアルの並びの番号。
    pub material: u32,
    /// 三角形の頂点の添字（3 つずつ）。
    pub indices: Vec<u32>,
}

/// レンダラー 1 つのメッシュ（スキンメッシュは送った時のポーズで焼いた形）。
#[derive(Clone, Debug, PartialEq)]
pub struct MeshData {
    /// モデルの中で安定した鍵（根からの子の番号の道。名前が重なっても違う）。
    pub key: String,
    /// レンダラーのゲームオブジェクトの名前（表示用）。
    pub name: String,
    pub skinned: bool,
    pub positions: Vec<[f32; 3]>,
    /// 空か、positions と同じ数。
    pub normals: Vec<[f32; 3]>,
    /// 空か、positions と同じ数。
    pub uv0: Vec<[f32; 2]>,
    pub submeshes: Vec<Submesh>,
}

/// モデルの全体（Unity → スタンドアロン）。前のモデルを置き換える。
#[derive(Clone, Debug, PartialEq)]
pub struct Model {
    /// モデルの世代（送り直すたびに増やす）。Pose・TextureSet はこの番号で同じモデルを指す。
    pub generation: u32,
    pub name: String,
    pub materials: Vec<MaterialInfo>,
    pub meshes: Vec<MeshData>,
}

/// 1 つのメッシュの新しい形。
#[derive(Clone, Debug, PartialEq)]
pub struct MeshPose {
    /// Model の meshes の番号。
    pub mesh: u32,
    /// そのメッシュの頂点と同じ数。
    pub positions: Vec<[f32; 3]>,
    /// 空か、positions と同じ数。
    pub normals: Vec<[f32; 3]>,
}

/// ポーズの変化（Unity → スタンドアロン。変わったメッシュだけ）。
#[derive(Clone, Debug, PartialEq)]
pub struct Pose {
    pub generation: u32,
    pub meshes: Vec<MeshPose>,
}

/// マテリアルの情報の更新（シェーダーの差し替えなど。数と並びは Model と同じ）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaterialsUpdate {
    pub generation: u32,
    pub materials: Vec<MaterialInfo>,
}

/// テクスチャセットの 1 チャンネルの共有メモリ。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelImage {
    pub channel: u8,
    /// 共有メモリのファイル（`shm` の形）。
    pub path: String,
}

/// テクスチャセットを知らせる（スタンドアロン → Unity。同じ番号なら置き換え）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextureSet {
    pub set: u32,
    /// どのモデルの世代のマテリアルか。
    pub generation: u32,
    /// モデルのマテリアルの並びの番号。
    pub material: u32,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub tile_size: u32,
    pub channels: Vec<ChannelImage>,
}

/// タイルの座標（左下が (0, 0)）。
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct Tile {
    pub x: u16,
    pub y: u16,
}

/// 共有メモリの中で変わったタイル（スタンドアロン → Unity）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TilesChanged {
    pub set: u32,
    pub channel: u8,
    /// 書き終えた時刻（UNIX 時刻のマイクロ秒。遅れを測るためだけ）。
    pub stamp_us: u64,
    pub tiles: Vec<Tile>,
}

/// マテリアルの値の種類（スタンドアロンがどの見た目で描くか）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum ValuesKind {
    /// 値なし（描ける見た目のシェーダーでない・確かめられない）。前に送った値を捨てる合図。
    None = 0,
    /// 版・バリアント・プロパティを確かめた lilToon。
    LilToon = 1,
}

impl ValuesKind {
    /// 知らない番号（新しい送り手が足した見た目）は値なしとして読む（描けない値を lilToon として描かない）。
    pub fn from_u8(v: u8) -> ValuesKind {
        match v {
            1 => ValuesKind::LilToon,
            _ => ValuesKind::None,
        }
    }
}

/// マテリアルのプロパティの値（Unity の型ごと。色はマテリアルに入っているままの値＝ガンマの空間、`[HDR]` の色はリニア）。
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum PropertyValue {
    /// Float・Range（Unity の古い Int も）。
    Float(f32),
    /// Integer。
    Int(i32),
    Color([f32; 4]),
    /// Vector と、テクスチャのタイリング・オフセット（`<名前>_ST`）。
    Vector([f32; 4]),
}

/// プロパティ 1 つ。
#[derive(Clone, PartialEq, Debug)]
pub struct PropertyEntry {
    pub name: String,
    pub value: PropertyValue,
}

/// 描いていないスロット（YoluPainter の流し込み先でないテクスチャのプロパティ）の絵の様子。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum SlotState {
    /// テクスチャが入っていない（シェーダーの既定で描く）。
    Empty = 0,
    /// 絵を送る（この命令の後ろに MaterialTexture が来る）。
    Follows = 1,
    /// 前に送った絵と同じ（受け手は持っている絵を使い続ける。モデルを送り直した後は使わない）。
    Unchanged = 2,
    /// 送る絵の予算を超えたので送らない。
    OverBudget = 3,
    /// 読めない（送る側の理由。描けない形式など）。
    Unreadable = 4,
}

impl SlotState {
    /// 知らない番号は「読めない」として読む（絵は来ない）。
    pub fn from_u8(v: u8) -> SlotState {
        match v {
            0 => SlotState::Empty,
            1 => SlotState::Follows,
            2 => SlotState::Unchanged,
            3 => SlotState::OverBudget,
            _ => SlotState::Unreadable,
        }
    }
}

/// スロット 1 つの様子と、元のテクスチャの大きさ（入っていなければ 0）。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SlotTexture {
    pub name: String,
    pub state: SlotState,
    pub width: u32,
    pub height: u32,
}

/// マテリアルの値（Unity → スタンドアロン。機能の印 MATERIAL_VALUES）。モデルの 1 つのマテリアルの、シェーダーの名前・プロパティの値・
/// キーワード・描いていないスロットの絵の様子。同じマテリアルの前の値を置き換える。
#[derive(Clone, PartialEq, Debug)]
pub struct MaterialValues {
    pub generation: u32,
    /// モデルのマテリアルの並びの番号。
    pub material: u32,
    pub kind: ValuesKind,
    pub shader: String,
    /// 何の対応と確かめたか（例: "lilToon 2.3.4 · Standard/Opaque"。人に見せるだけ）。
    pub source: String,
    pub properties: Vec<PropertyEntry>,
    pub keywords: Vec<String>,
    pub slots: Vec<SlotTexture>,
}

/// 描いていないスロットの絵（Unity → スタンドアロン。機能の印 MATERIAL_VALUES）。直前の MaterialValues で `Follows` と言ったスロットのもの。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MaterialTexture {
    pub generation: u32,
    pub material: u32,
    pub slot: String,
    pub width: u32,
    pub height: u32,
    /// Unity がこの絵を sRGB として読む（RGB をリニアへ直してから使う）。偽はリニアのまま。
    pub srgb: bool,
    /// RGBA8（straight）、行は下から（Unity の並び）。幅 × 高さ × 4 バイト。
    pub pixels: Vec<u8>,
}

/// 元の絵（MaterialOriginal）の様子。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum OriginalState {
    /// 絵が付いている（この命令の画素）。
    Image = 0,
    /// 読めない（送る側の理由。2D の絵でない・HDR・GPU が使えないなど）。
    Unreadable = 1,
    /// 辺が上限（`MAX_ORIGINAL_SIZE`）を超えるので送らない。
    TooLarge = 2,
    /// この送りの全部の絵の予算を超えたので送らない。
    OverBudget = 3,
}

impl OriginalState {
    /// 知らない番号は「読めない」として読む（絵は来ない）。
    pub fn from_u8(v: u8) -> OriginalState {
        match v {
            0 => OriginalState::Image,
            2 => OriginalState::TooLarge,
            3 => OriginalState::OverBudget,
            _ => OriginalState::Unreadable,
        }
    }
}

/// 元の絵を Unity がどう読んだか。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum OriginalRead {
    /// 原本のファイル（PNG・TGA・JPG）から。圧縮が無く、透明な画素の RGB も原本のまま。
    File = 0,
    /// Unity が取り込んだ絵（CPU が読める値）から。
    Imported = 1,
    /// 取り込んだ絵を GPU に描いて読み戻した（読める形でない・取り込み設定が絵を変える・アセットでない）。
    Gpu = 2,
}

impl OriginalRead {
    /// 知らない番号は「GPU を通して」として読む（原本の確かな値と言わない）。
    pub fn from_u8(v: u8) -> OriginalRead {
        match v {
            0 => OriginalRead::File,
            1 => OriginalRead::Imported,
            _ => OriginalRead::Gpu,
        }
    }
}

/// 元の絵の `flags` の bit: 圧縮されたテクスチャから読んだ（値は圧縮を解いたもので、原本のファイルのものではない）。
pub const ORIGINAL_COMPRESSED: u8 = 1;

/// 元の絵（Unity → スタンドアロン。機能の印 ORIGINAL_TEXTURES）。モデルのマテリアルの、YoluPainter が描くスロット（Color の流し込み先）に
/// 入っている元のテクスチャ。スタンドアロンは新しく作ったテクスチャセットの一番下に入れる。モデルのマテリアルの情報（`TextureProperty`）に
/// 絵の入っている、Color の流し込み先ごとに 1 つ送る（絵が付かないときも様子だけ送る。受け手は全部が揃うまで、そのセットを Unity に出さない）。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MaterialOriginal {
    pub generation: u32,
    /// モデルのマテリアルの並びの番号。
    pub material: u32,
    /// スロット（シェーダーのプロパティの名前。Color の流し込み先）。
    pub slot: String,
    pub state: OriginalState,
    pub read: OriginalRead,
    /// 圧縮されたテクスチャから読んだ（`ORIGINAL_COMPRESSED`）。
    pub compressed: bool,
    /// 絵の大きさ（絵が付かない様子では、Unity のテクスチャの大きさ）。
    pub width: u32,
    pub height: u32,
    /// Unity がこの絵を sRGB として読む（偽はリニアのデータ）。ガンマの色空間のプロジェクトは、画素をそのまま使うので真で送る。
    pub srgb: bool,
    /// RGBA8（straight）、行は下から。幅 × 高さ × 4 バイト（`Image` のときだけ。ほかは空）。
    pub pixels: Vec<u8>,
}

/// 誤りの種類。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u16)]
pub enum ErrorCode {
    /// 知らない種類の命令。
    UnknownCommand = 1,
    /// 中身が読めない。
    Malformed = 2,
    /// この向き・この時には受けない命令（挨拶の前の Model など）。
    UnexpectedCommand = 3,
    /// 読めたが受け付けられない（古い世代・範囲外のメッシュなど）。
    Refused = 4,
    Other = 0xffff,
}

impl ErrorCode {
    pub fn from_u16(v: u16) -> ErrorCode {
        match v {
            1 => ErrorCode::UnknownCommand,
            2 => ErrorCode::Malformed,
            3 => ErrorCode::UnexpectedCommand,
            4 => ErrorCode::Refused,
            _ => ErrorCode::Other,
        }
    }
}

/// 誤りの知らせ（どちらの向きにも）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ErrorMessage {
    pub code: ErrorCode,
    /// 原因の命令の種類（無ければ 0）。
    pub kind: u16,
    pub text: String,
}

/// 命令。
#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    Hello(Hello),
    Bye,
    Model(Model),
    Pose(Pose),
    Materials(MaterialsUpdate),
    ModelClosed { generation: u32 },
    MaterialValues(MaterialValues),
    MaterialTexture(MaterialTexture),
    MaterialOriginal(MaterialOriginal),
    Welcome(Welcome),
    Reject(Reject),
    TextureSet(TextureSet),
    TextureSetRemoved { set: u32 },
    TilesChanged(TilesChanged),
    Error(ErrorMessage),
}

impl Message {
    pub fn kind(&self) -> Kind {
        match self {
            Message::Hello(_) => Kind::Hello,
            Message::Bye => Kind::Bye,
            Message::Model(_) => Kind::Model,
            Message::Pose(_) => Kind::Pose,
            Message::Materials(_) => Kind::Materials,
            Message::ModelClosed { .. } => Kind::ModelClosed,
            Message::MaterialValues(_) => Kind::MaterialValues,
            Message::MaterialTexture(_) => Kind::MaterialTexture,
            Message::MaterialOriginal(_) => Kind::MaterialOriginal,
            Message::Welcome(_) => Kind::Welcome,
            Message::Reject(_) => Kind::Reject,
            Message::TextureSet(_) => Kind::TextureSet,
            Message::TextureSetRemoved { .. } => Kind::TextureSetRemoved,
            Message::TilesChanged(_) => Kind::TilesChanged,
            Message::Error(_) => Kind::Error,
        }
    }

    /// 中身（枠の頭を除く）を書く。
    pub fn encode_payload(&self) -> Vec<u8> {
        let mut w = Writer::new();
        match self {
            Message::Hello(h) => {
                w.u16(h.min_version);
                w.u16(h.max_version);
                w.str(&h.agent);
                w.u64(h.features);
                if let Some(a) = &h.auth {
                    w.raw(&a.nonce);
                    w.raw(&a.proof);
                    // 版の欄は鍵の欄の後ろの位置で決まる。鍵の欄が無ければ書かない（古い読み手が版の欄を鍵の欄と読み違えないように）
                    if let Some(v) = &h.versions {
                        write_versions(&mut w, v);
                    }
                }
            }
            Message::Bye => {}
            Message::Model(m) => {
                let vertices: usize = m.meshes.iter().map(|x| x.positions.len()).sum();
                w = Writer::with_capacity(64 + vertices * 32);
                w.u32(m.generation);
                w.str(&m.name);
                write_materials(&mut w, &m.materials);
                w.u32(m.meshes.len() as u32);
                for mesh in &m.meshes {
                    w.str(&mesh.key);
                    w.str(&mesh.name);
                    w.bool(mesh.skinned);
                    w.f32_groups(&mesh.positions);
                    w.f32_groups(&mesh.normals);
                    w.f32_groups(&mesh.uv0);
                    w.u32(mesh.submeshes.len() as u32);
                    for sub in &mesh.submeshes {
                        w.u32(sub.material);
                        w.u32_slice(&sub.indices);
                    }
                }
            }
            Message::Pose(p) => {
                w.u32(p.generation);
                w.u32(p.meshes.len() as u32);
                for mesh in &p.meshes {
                    w.u32(mesh.mesh);
                    w.f32_groups(&mesh.positions);
                    w.f32_groups(&mesh.normals);
                }
            }
            Message::Materials(m) => {
                w.u32(m.generation);
                write_materials(&mut w, &m.materials);
            }
            Message::ModelClosed { generation } => w.u32(*generation),
            Message::MaterialValues(v) => {
                w = Writer::with_capacity(64 + v.properties.len() * 40);
                w.u32(v.generation);
                w.u32(v.material);
                w.u8(v.kind as u8);
                w.str(&v.shader);
                w.str(&v.source);
                w.u32(v.properties.len() as u32);
                for p in &v.properties {
                    w.str(&p.name);
                    match p.value {
                        PropertyValue::Float(x) => {
                            w.u8(0);
                            w.f32(x);
                        }
                        PropertyValue::Int(x) => {
                            w.u8(1);
                            w.i32(x);
                        }
                        PropertyValue::Color(c) => {
                            w.u8(2);
                            c.iter().for_each(|x| w.f32(*x));
                        }
                        PropertyValue::Vector(c) => {
                            w.u8(3);
                            c.iter().for_each(|x| w.f32(*x));
                        }
                    }
                }
                w.u32(v.keywords.len() as u32);
                for k in &v.keywords {
                    w.str(k);
                }
                w.u32(v.slots.len() as u32);
                for slot in &v.slots {
                    w.str(&slot.name);
                    w.u8(slot.state as u8);
                    w.u32(slot.width);
                    w.u32(slot.height);
                }
            }
            Message::MaterialTexture(t) => {
                w = Writer::with_capacity(48 + t.slot.len() + t.pixels.len());
                w.u32(t.generation);
                w.u32(t.material);
                w.str(&t.slot);
                w.u32(t.width);
                w.u32(t.height);
                w.bool(t.srgb);
                w.bytes(&t.pixels);
            }
            Message::MaterialOriginal(o) => {
                w = Writer::with_capacity(64 + o.slot.len() + o.pixels.len());
                w.u32(o.generation);
                w.u32(o.material);
                w.str(&o.slot);
                w.u8(o.state as u8);
                w.u8(o.read as u8);
                w.u8(if o.compressed { ORIGINAL_COMPRESSED } else { 0 });
                w.u32(o.width);
                w.u32(o.height);
                w.bool(o.srgb);
                w.bytes(&o.pixels);
            }
            Message::Welcome(x) => {
                w.u16(x.version);
                w.str(&x.agent);
                w.u64(x.session);
                w.u64(x.features);
                if let Some(p) = &x.proof {
                    w.raw(p);
                    if let Some(v) = &x.versions {
                        write_versions(&mut w, v);
                    }
                }
            }
            Message::Reject(x) => {
                w.u16(x.code as u16);
                w.str(&x.text);
                if let Some(d) = &x.detail {
                    w.u16(d.min_version);
                    w.u16(d.max_version);
                    write_version(&mut w, d.min_peer);
                    w.u16(d.peer_min_version);
                    w.u16(d.peer_max_version);
                    write_version(&mut w, d.peer_min_peer);
                }
            }
            Message::TextureSet(s) => {
                w.u32(s.set);
                w.u32(s.generation);
                w.u32(s.material);
                w.str(&s.name);
                w.u32(s.width);
                w.u32(s.height);
                w.u32(s.tile_size);
                w.u32(s.channels.len() as u32);
                for c in &s.channels {
                    w.u8(c.channel);
                    w.str(&c.path);
                }
            }
            Message::TextureSetRemoved { set } => w.u32(*set),
            Message::TilesChanged(t) => {
                w = Writer::with_capacity(24 + t.tiles.len() * 4);
                w.u32(t.set);
                w.u8(t.channel);
                w.u64(t.stamp_us);
                w.u32(t.tiles.len() as u32);
                for tile in &t.tiles {
                    w.u16(tile.x);
                    w.u16(tile.y);
                }
            }
            Message::Error(e) => {
                w.u16(e.code as u16);
                w.u16(e.kind);
                w.str(&e.text);
            }
        }
        w.into_inner()
    }

    /// 種類の番号と中身から読む。知らない種類は `UnknownKind`。中身の後ろの読まなかったバイトは、新しい版が足した欄として読み飛ばす。
    pub fn decode(kind: u16, payload: &[u8]) -> Result<Message, DecodeError> {
        let k = Kind::from_u16(kind).ok_or(DecodeError::UnknownKind(kind))?;
        let mut r = Reader::new(payload);
        let r = &mut r;
        Ok(match k {
            Kind::Hello => {
                let min_version = r.u16()?;
                let max_version = r.u16()?;
                if min_version > max_version {
                    return Err(DecodeError::Invalid("版の範囲"));
                }
                let agent = r.str(MAX_NAME_BYTES, "送り手の名前")?;
                let features = r.u64()?;
                // 鍵の欄は後ろに足したもの。無い（欄の分に足りない）のは古いブリッジで、鍵が無いものとして扱う（受け手が断る）。
                // 欄の後ろはさらに新しい版の欄として読み飛ばす
                let auth = if r.remaining() >= crate::auth::NONCE_BYTES + crate::auth::PROOF_BYTES {
                    Some(HelloAuth {
                        nonce: r.array()?,
                        proof: r.array()?,
                    })
                } else {
                    None
                };
                // 版の欄は鍵の欄の後ろ。足りなければ版を名乗らない古い相手（欄の後ろは、さらに新しい版の欄として読み飛ばす）
                let versions = if auth.is_some() {
                    read_versions(r)?
                } else {
                    None
                };
                Message::Hello(Hello {
                    min_version,
                    max_version,
                    agent,
                    features,
                    auth,
                    versions,
                })
            }
            Kind::Bye => Message::Bye,
            Kind::Model => {
                let generation = r.u32()?;
                let name = r.str(MAX_NAME_BYTES, "モデルの名前")?;
                let materials = read_materials(r)?;
                let mesh_count = r.count(MAX_MESHES, 1, "メッシュの数")?;
                let mut meshes = Vec::with_capacity(mesh_count);
                for _ in 0..mesh_count {
                    let key = r.str(MAX_NAME_BYTES, "メッシュの鍵")?;
                    let name = r.str(MAX_NAME_BYTES, "メッシュの名前")?;
                    let skinned = r.bool()?;
                    let positions = r.f32_groups::<3>(MAX_VERTICES, "頂点の位置")?;
                    let normals = r.f32_groups::<3>(MAX_VERTICES, "法線")?;
                    let uv0 = r.f32_groups::<2>(MAX_VERTICES, "UV0")?;
                    if !normals.is_empty() && normals.len() != positions.len() {
                        return Err(DecodeError::Invalid("法線の数"));
                    }
                    if !uv0.is_empty() && uv0.len() != positions.len() {
                        return Err(DecodeError::Invalid("UV0 の数"));
                    }
                    let sub_count = r.count(MAX_SUBMESHES, 8, "サブメッシュの数")?;
                    let mut submeshes = Vec::with_capacity(sub_count);
                    for _ in 0..sub_count {
                        let material = r.u32()?;
                        if material as usize >= materials.len() {
                            return Err(DecodeError::Invalid("サブメッシュのマテリアル"));
                        }
                        let indices = r.u32_vec(MAX_INDICES, "三角形の添字")?;
                        if indices.len() % 3 != 0 {
                            return Err(DecodeError::Invalid("三角形の添字の数"));
                        }
                        if indices.iter().any(|&i| i as usize >= positions.len()) {
                            return Err(DecodeError::Invalid("三角形の添字"));
                        }
                        submeshes.push(Submesh { material, indices });
                    }
                    meshes.push(MeshData {
                        key,
                        name,
                        skinned,
                        positions,
                        normals,
                        uv0,
                        submeshes,
                    });
                }
                Message::Model(Model {
                    generation,
                    name,
                    materials,
                    meshes,
                })
            }
            Kind::Pose => {
                let generation = r.u32()?;
                let count = r.count(MAX_MESHES, 12, "ポーズのメッシュの数")?;
                let mut meshes = Vec::with_capacity(count);
                for _ in 0..count {
                    let mesh = r.u32()?;
                    let positions = r.f32_groups::<3>(MAX_VERTICES, "頂点の位置")?;
                    let normals = r.f32_groups::<3>(MAX_VERTICES, "法線")?;
                    if !normals.is_empty() && normals.len() != positions.len() {
                        return Err(DecodeError::Invalid("法線の数"));
                    }
                    meshes.push(MeshPose {
                        mesh,
                        positions,
                        normals,
                    });
                }
                Message::Pose(Pose { generation, meshes })
            }
            Kind::Materials => Message::Materials(MaterialsUpdate {
                generation: r.u32()?,
                materials: read_materials(r)?,
            }),
            Kind::ModelClosed => Message::ModelClosed {
                generation: r.u32()?,
            },
            Kind::MaterialValues => {
                let generation = r.u32()?;
                let material = r.u32()?;
                let kind = ValuesKind::from_u8(r.u8()?);
                let shader = r.str(MAX_NAME_BYTES, "シェーダーの名前")?;
                let source = r.str(MAX_NAME_BYTES, "値の出どころ")?;
                // 名前（4 バイト以上）・型（1）・値（4 以上）
                let count = r.count(MAX_VALUE_PROPERTIES, 9, "プロパティの数")?;
                let mut properties = Vec::with_capacity(count);
                for _ in 0..count {
                    let name = r.str(MAX_VALUE_NAME_BYTES, "プロパティの名前")?;
                    let value = match r.u8()? {
                        0 => PropertyValue::Float(r.finite_f32("プロパティの値")?),
                        1 => PropertyValue::Int(r.i32()?),
                        t @ (2 | 3) => {
                            let mut c = [0f32; 4];
                            for x in &mut c {
                                *x = r.finite_f32("プロパティの値")?;
                            }
                            if t == 2 {
                                PropertyValue::Color(c)
                            } else {
                                PropertyValue::Vector(c)
                            }
                        }
                        _ => return Err(DecodeError::Invalid("プロパティの型")),
                    };
                    properties.push(PropertyEntry { name, value });
                }
                let count = r.count(MAX_VALUE_KEYWORDS, 4, "キーワードの数")?;
                let mut keywords = Vec::with_capacity(count);
                for _ in 0..count {
                    keywords.push(r.str(MAX_VALUE_NAME_BYTES, "キーワード")?);
                }
                let count = r.count(MAX_VALUE_SLOTS, 13, "スロットの数")?;
                let mut slots = Vec::with_capacity(count);
                for _ in 0..count {
                    slots.push(SlotTexture {
                        name: r.str(MAX_VALUE_NAME_BYTES, "スロットの名前")?,
                        state: SlotState::from_u8(r.u8()?),
                        width: r.u32()?,
                        height: r.u32()?,
                    });
                }
                Message::MaterialValues(MaterialValues {
                    generation,
                    material,
                    kind,
                    shader,
                    source,
                    properties,
                    keywords,
                    slots,
                })
            }
            Kind::MaterialTexture => {
                let generation = r.u32()?;
                let material = r.u32()?;
                let slot = r.str(MAX_VALUE_NAME_BYTES, "スロットの名前")?;
                let width = r.u32()?;
                let height = r.u32()?;
                if width == 0
                    || height == 0
                    || width > MAX_SLOT_TEXTURE_SIZE
                    || height > MAX_SLOT_TEXTURE_SIZE
                {
                    return Err(DecodeError::Invalid("スロットの絵の大きさ"));
                }
                let srgb = r.bool()?;
                let max = (MAX_SLOT_TEXTURE_SIZE as usize).pow(2) * 4;
                let pixels = r.bytes(max, "スロットの絵の画素")?;
                if pixels.len() != width as usize * height as usize * 4 {
                    return Err(DecodeError::Invalid("スロットの絵の画素の数"));
                }
                Message::MaterialTexture(MaterialTexture {
                    generation,
                    material,
                    slot,
                    width,
                    height,
                    srgb,
                    pixels: pixels.to_vec(),
                })
            }
            Kind::MaterialOriginal => {
                let generation = r.u32()?;
                let material = r.u32()?;
                let slot = r.str(MAX_VALUE_NAME_BYTES, "スロットの名前")?;
                let state = OriginalState::from_u8(r.u8()?);
                let read = OriginalRead::from_u8(r.u8()?);
                // 知らない bit は読み飛ばす（新しい送り手が足した印）
                let compressed = r.u8()? & ORIGINAL_COMPRESSED != 0;
                let width = r.u32()?;
                let height = r.u32()?;
                let srgb = r.bool()?;
                let max = (MAX_ORIGINAL_SIZE as usize).pow(2) * 4;
                let pixels = r.bytes(max, "元の絵の画素")?;
                if state == OriginalState::Image {
                    if width == 0
                        || height == 0
                        || width > MAX_ORIGINAL_SIZE
                        || height > MAX_ORIGINAL_SIZE
                    {
                        return Err(DecodeError::Invalid("元の絵の大きさ"));
                    }
                    if pixels.len() != width as usize * height as usize * 4 {
                        return Err(DecodeError::Invalid("元の絵の画素の数"));
                    }
                } else if !pixels.is_empty() {
                    return Err(DecodeError::Invalid("絵の付かない元の絵の画素"));
                }
                Message::MaterialOriginal(MaterialOriginal {
                    generation,
                    material,
                    slot,
                    state,
                    read,
                    compressed,
                    width,
                    height,
                    srgb,
                    pixels: pixels.to_vec(),
                })
            }
            Kind::Welcome => {
                let version = r.u16()?;
                let agent = r.str(MAX_NAME_BYTES, "送り手の名前")?;
                let session = r.u64()?;
                let features = r.u64()?;
                // 証しの欄は後ろに足したもの（足りなければ鍵を確かめない古いスタンドアロン）
                let proof = if r.remaining() >= crate::auth::PROOF_BYTES {
                    Some(r.array()?)
                } else {
                    None
                };
                let versions = if proof.is_some() {
                    read_versions(r)?
                } else {
                    None
                };
                Message::Welcome(Welcome {
                    version,
                    agent,
                    session,
                    features,
                    proof,
                    versions,
                })
            }
            Kind::Reject => {
                let code = RejectCode::from_u16(r.u16()?);
                let text = r.str(MAX_PATH_BYTES, "理由")?;
                // 詳しい欄は後ろに足したもの（足りなければ古い相手の断り）
                let detail = if r.remaining() >= (4 + VERSION_BYTES) * 2 {
                    let min_version = r.u16()?;
                    let max_version = r.u16()?;
                    let min_peer = read_version(r)?;
                    let peer_min_version = r.u16()?;
                    let peer_max_version = r.u16()?;
                    let peer_min_peer = read_version(r)?;
                    (min_version <= max_version && peer_min_version <= peer_max_version).then_some(
                        RejectDetail {
                            min_version,
                            max_version,
                            min_peer,
                            peer_min_version,
                            peer_max_version,
                            peer_min_peer,
                        },
                    )
                } else {
                    None
                };
                Message::Reject(Reject { code, text, detail })
            }
            Kind::TextureSet => {
                let set = r.u32()?;
                let generation = r.u32()?;
                let material = r.u32()?;
                let name = r.str(MAX_NAME_BYTES, "セットの名前")?;
                let width = r.u32()?;
                let height = r.u32()?;
                let tile_size = r.u32()?;
                if width == 0
                    || height == 0
                    || width > MAX_TEXTURE_SIZE
                    || height > MAX_TEXTURE_SIZE
                {
                    return Err(DecodeError::Invalid("テクスチャセットの大きさ"));
                }
                if !crate::shm::valid_tile_size(tile_size) {
                    return Err(DecodeError::Invalid("タイルの大きさ"));
                }
                let count = r.count(channel::COUNT as usize, 5, "チャンネルの数")?;
                let mut channels: Vec<ChannelImage> = Vec::with_capacity(count);
                for _ in 0..count {
                    let ch = r.u8()?;
                    if ch >= channel::COUNT || channels.iter().any(|c| c.channel == ch) {
                        return Err(DecodeError::Invalid("チャンネル"));
                    }
                    channels.push(ChannelImage {
                        channel: ch,
                        path: r.str(MAX_PATH_BYTES, "共有メモリのパス")?,
                    });
                }
                Message::TextureSet(TextureSet {
                    set,
                    generation,
                    material,
                    name,
                    width,
                    height,
                    tile_size,
                    channels,
                })
            }
            Kind::TextureSetRemoved => Message::TextureSetRemoved { set: r.u32()? },
            Kind::TilesChanged => {
                let set = r.u32()?;
                let ch = r.u8()?;
                if ch >= channel::COUNT {
                    return Err(DecodeError::Invalid("チャンネル"));
                }
                let stamp_us = r.u64()?;
                let count = r.count(MAX_TILES_PER_MESSAGE, 4, "タイルの数")?;
                let mut tiles = Vec::with_capacity(count);
                for _ in 0..count {
                    tiles.push(Tile {
                        x: r.u16()?,
                        y: r.u16()?,
                    });
                }
                Message::TilesChanged(TilesChanged {
                    set,
                    channel: ch,
                    stamp_us,
                    tiles,
                })
            }
            Kind::Error => Message::Error(ErrorMessage {
                code: ErrorCode::from_u16(r.u16()?),
                kind: r.u16()?,
                text: r.str(MAX_PATH_BYTES, "誤りの説明")?,
            }),
        })
    }
}

/// アプリの版の欄の長さ（major・minor・patch の u16 が 3 つ）。
const VERSION_BYTES: usize = 6;

fn write_version(w: &mut Writer, v: AppVersion) {
    w.u16(v.major);
    w.u16(v.minor);
    w.u16(v.patch);
}

fn read_version(r: &mut Reader<'_>) -> Result<AppVersion, DecodeError> {
    Ok(AppVersion::new(r.u16()?, r.u16()?, r.u16()?))
}

fn write_versions(w: &mut Writer, v: &VersionInfo) {
    write_version(w, v.app);
    write_version(w, v.min_peer);
}

/// 版の欄（自分の版と求める相手の版）。足りなければ None（版を名乗らない古い相手）。
fn read_versions(r: &mut Reader<'_>) -> Result<Option<VersionInfo>, DecodeError> {
    if r.remaining() < VERSION_BYTES * 2 {
        return Ok(None);
    }
    Ok(Some(VersionInfo {
        app: read_version(r)?,
        min_peer: read_version(r)?,
    }))
}

fn write_materials(w: &mut Writer, materials: &[MaterialInfo]) {
    w.u32(materials.len() as u32);
    for m in materials {
        match &m.key {
            MaterialKey::Unassigned => w.u8(0),
            MaterialKey::Material { name, asset } => {
                w.u8(1);
                w.str(name);
                match asset {
                    None => w.bool(false),
                    Some((guid, file_id)) => {
                        w.bool(true);
                        w.str(guid);
                        w.i64(*file_id);
                    }
                }
            }
        }
        w.str(&m.shader);
        w.u32(m.textures.len() as u32);
        for t in &m.textures {
            w.str(&t.name);
            w.u32(t.width);
            w.u32(t.height);
        }
        w.u32(m.routes.len() as u32);
        for route in &m.routes {
            w.u8(route.channel);
            w.str(&route.property);
        }
    }
}

/// GUID は小文字の 16 進 32 文字（Unity の AssetDatabase の形）。
pub fn is_asset_guid(s: &str) -> bool {
    s.len() == 32
        && s.bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

fn read_materials(r: &mut Reader<'_>) -> Result<Vec<MaterialInfo>, DecodeError> {
    let count = r.count(MAX_MATERIALS, 1, "マテリアルの数")?;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let key = match r.u8()? {
            0 => MaterialKey::Unassigned,
            1 => {
                let name = r.str(MAX_NAME_BYTES, "マテリアルの名前")?;
                let asset = if r.bool()? {
                    let guid = r.str(32, "GUID")?;
                    if !is_asset_guid(&guid) {
                        return Err(DecodeError::Invalid("GUID"));
                    }
                    Some((guid, r.i64()?))
                } else {
                    None
                };
                MaterialKey::Material { name, asset }
            }
            _ => return Err(DecodeError::Invalid("マテリアルの鍵の形")),
        };
        let shader = r.str(MAX_NAME_BYTES, "シェーダーの名前")?;
        let tex_count = r.count(MAX_TEXTURE_PROPERTIES, 12, "テクスチャのプロパティの数")?;
        let mut textures = Vec::with_capacity(tex_count);
        for _ in 0..tex_count {
            textures.push(TextureProperty {
                name: r.str(MAX_NAME_BYTES, "プロパティの名前")?,
                width: r.u32()?,
                height: r.u32()?,
            });
        }
        let route_count = r.count(channel::COUNT as usize, 5, "流し込み先の数")?;
        let mut routes: Vec<ChannelRoute> = Vec::with_capacity(route_count);
        for _ in 0..route_count {
            let ch = r.u8()?;
            if ch >= channel::COUNT || routes.iter().any(|x| x.channel == ch) {
                return Err(DecodeError::Invalid("流し込み先のチャンネル"));
            }
            routes.push(ChannelRoute {
                channel: ch,
                property: r.str(MAX_NAME_BYTES, "流し込み先のプロパティ")?,
            });
        }
        out.push(MaterialInfo {
            key,
            shader,
            textures,
            routes,
        });
    }
    Ok(out)
}
