//! Live Link のファイルの受け渡し: Unity が頼みの JSON を `inbox/` に置き、スタンドアロンが拾う。スタンドアロンが書き出したら返事の
//! JSON を `outbox/` に置き、Unity が拾う。送るのはファイルの道と小さな値だけ（メッシュ・絵の画素は送らない。スタンドアロンが FBX と
//! 絵のファイルを自分で読む）。
//!
//! - フォルダ（`Folder`）: Windows は `%LOCALAPPDATA%\YoluPainter\LiveLink`、macOS は `~/Library/Application Support/YoluPainter/LiveLink`、
//!   ほかは `${XDG_DATA_HOME:-~/.local/share}/YoluPainter/LiveLink`（`XDG_DATA_HOME` が絶対の道でなければ既定）。環境変数
//!   [`ENV_DIR`] で差し替えられる（試験・2 つのアプリを並べるとき）。中に `inbox/`・`claimed/`・`outbox/` と、起きている印 `presence.json`。
//!   フォルダは自分だけのもの（`private`。Unix は 0700 で持ち主が自分、Windows は自分だけの DACL）で、そうでないフォルダは読まない。
//! - 書くときは必ず `<名前>.tmp` に書いて閉じてから最終の名前へ置き換える（読み手は書きかけを見ない）。読み手は `.tmp` を見ない。
//! - 拾うのは [`Folder::claim`]: まず `claimed/<名前>.lock`（拾っている印）を「同じ名前があれば必ず失敗する」作り方（`create_new`）で作り、作れた
//!   方だけが `inbox/` から `claimed/` へ頼みを移す。2 つのスタンドアロンが同時に拾おうとしても、印を作れた 1 つだけが受ける（名前の変更の
//!   成否では決めない。Windows では、同時の名前の変更が両方成功することがある）。読み終えたら頼みを `claimed/` から消し、その後で印を消す
//!   （[`Claimed::finish`]）。印は拾った頼みを持っている間だけ残り、落とせば消える。
//! - 頼みの読み（[`read_request`]）: 大きさの上限（[`MAX_REQUEST_BYTES`]・数の上限）、知らない `format`・`kind`、有限でない数、
//!   決まりに合わない形（番号が範囲の外・必須の欄が無い）を、理由（[`Reason`]）つきで断る。知らないキーは読み飛ばす（Unity の新しい版が
//!   足した欄で壊れない）。

use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::private::{check_private_dir, create_private_file, ensure_private_dir};

/// 頼み・返事・起きている印の形式の版。
pub const FORMAT: u32 = 1;
/// 頼みの種類（今は「開く」だけ。同じ `target.key` の文書を開いていれば送り直し）。
pub const KIND_OPEN: &str = "open";
/// フォルダを差し替える環境変数。
pub const ENV_DIR: &str = "YOLUPAINTER_LIVELINK_DIR";
/// 頼みのファイルの大きさの上限。
pub const MAX_REQUEST_BYTES: u64 = 16 << 20;
/// レンダラー（と、送れなかったレンダラー・モデル）の数の上限。
pub const MAX_RENDERERS: usize = 1024;
/// 骨の値の数の上限。
pub const MAX_BONES: usize = 16_384;
/// マテリアルの数の上限。
pub const MAX_MATERIALS: usize = 1024;
/// 1 つのマテリアルのテクスチャの数の上限。
pub const MAX_TEXTURES: usize = 256;
/// 1 つのレンダラーの BlendShape・1 つのマテリアルの値の種類ごとの数・キーワードの数の上限。
pub const MAX_ENTRIES: usize = 4096;
/// 文字列の長さの上限（バイト）。
pub const MAX_TEXT_BYTES: usize = 8192;
/// 起きている印を書き直す間隔。
pub const PRESENCE_EVERY: Duration = Duration::from_secs(2);
/// 起きている印がこれより古ければ、スタンドアロンは起きていないと見る（Unity の側）。
pub const PRESENCE_FRESH: Duration = Duration::from_secs(6);
/// `inbox/` を見る間隔。
pub const INBOX_EVERY: Duration = Duration::from_millis(500);
/// 起きている印の名前。
pub const PRESENCE: &str = "presence.json";
/// 書きかけの印（この末尾の名前は読まない）。
pub const TMP_SUFFIX: &str = ".tmp";
/// `claimed/` の拾っている印の名前の末尾（`<頼みのファイルの名前>.lock`。`.json` で終わらないので、頼みとして読まれない）。
pub const LOCK_SUFFIX: &str = ".lock";

// ───────── 理由 ─────────

/// 断り・送れなかった理由（`problems[].reason`・`refused[].reason` の言葉）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Reason {
    /// メッシュが FBX から来ていない（Unity が送れなかった）。
    MeshNotFromFbx,
    /// 骨・メッシュのノードが FBX の中に見つからない。
    BoneNotFound,
    /// 同じ名前の兄弟があり、骨が 1 つに決まらない。
    AmbiguousBone,
    /// 取り込みの設定に合わせられない（`bake_axis_conversion`）。
    UnsupportedImport,
    /// FBX を読めない。
    FbxUnreadable,
    /// 絵のファイルを読めない。
    TextureUnreadable,
    /// 上限を超えた。
    TooLarge,
    /// 形式の版・種類が違う・読めない頼み。
    FormatUnknown,
    /// 描いている最中・保存の途中（スタンドアロンは待って当てるので、今は使わない。Unity が知っている言葉として残す）。
    Busy,
    /// 違う相手の頼みで、利用者が保存していない変更を捨てずに、開くのをやめた。
    Declined,
}

impl Reason {
    pub const ALL: [Reason; 10] = [
        Reason::MeshNotFromFbx,
        Reason::BoneNotFound,
        Reason::AmbiguousBone,
        Reason::UnsupportedImport,
        Reason::FbxUnreadable,
        Reason::TextureUnreadable,
        Reason::TooLarge,
        Reason::FormatUnknown,
        Reason::Busy,
        Reason::Declined,
    ];

    /// JSON の言葉。
    pub fn as_str(self) -> &'static str {
        match self {
            Reason::MeshNotFromFbx => "mesh_not_from_fbx",
            Reason::BoneNotFound => "bone_not_found",
            Reason::AmbiguousBone => "ambiguous_bone",
            Reason::UnsupportedImport => "unsupported_import",
            Reason::FbxUnreadable => "fbx_unreadable",
            Reason::TextureUnreadable => "texture_unreadable",
            Reason::TooLarge => "too_large",
            Reason::FormatUnknown => "format_unknown",
            Reason::Busy => "busy",
            Reason::Declined => "declined",
        }
    }

    /// JSON の言葉から（知らない言葉は None）。
    pub fn parse(word: &str) -> Option<Reason> {
        Reason::ALL.into_iter().find(|r| r.as_str() == word)
    }
}

// ───────── 頼み ─────────

/// 頼み `inbox/<id>.json`（Unity → スタンドアロン）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub format: u32,
    pub kind: String,
    /// 頼みごとに新しい ID（返事の名前と `request` に使う）。
    pub id: String,
    #[serde(default)]
    pub bridge: Bridge,
    #[serde(default)]
    pub project: UnityProject,
    pub target: Target,
    #[serde(default)]
    pub root: Option<Root>,
    pub models: Vec<ModelRef>,
    pub renderers: Vec<Renderer>,
    #[serde(default)]
    pub bones: Vec<BoneValue>,
    #[serde(default)]
    pub materials: Vec<Material>,
    /// Unity が送れなかったレンダラー。
    #[serde(default)]
    pub refused: Vec<Problem>,
}

/// 頼んだ Unity のパッケージ。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Bridge {
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub unity: String,
}

/// Unity のプロジェクト。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct UnityProject {
    #[serde(default)]
    pub root: String,
    #[serde(default)]
    pub name: String,
}

/// 頼みの相手（シーンのオブジェクト）。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Target {
    /// シーンのオブジェクトの身元（同じ相手の送り直しを見分ける）。
    pub key: String,
    /// 表示用の名前。
    #[serde(default)]
    pub name: String,
    /// 書き出しの既定の置き場（絶対の道）。
    #[serde(default)]
    pub export_dir: String,
}

/// Unity のワールド → 相手の根（列優先の 4×4）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Root {
    pub world: Vec<f32>,
}

/// 相手が使う FBX 1 つ。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelRef {
    /// `renderers[].model`・`bones[].model` が指す番号。
    pub id: u32,
    /// FBX の絶対の道（区切りは `/`）。
    pub fbx: String,
    /// Unity のアセットの GUID（同じ道でも別のアセットになったら読み直す）。
    #[serde(default)]
    pub guid: String,
    #[serde(default)]
    pub import: ImportSettings,
}

/// FBX の取り込みの設定（Unity の ModelImporter）。
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImportSettings {
    #[serde(default = "one")]
    pub global_scale: f64,
    #[serde(default = "yes")]
    pub use_file_scale: bool,
    #[serde(default)]
    pub bake_axis_conversion: bool,
    #[serde(default = "yes")]
    pub import_blend_shapes: bool,
    /// 根の子が 1 つだけのとき、その子をプレハブの根にしない（Unity の既定は false: 畳む）。
    #[serde(default)]
    pub preserve_hierarchy: bool,
}

fn one() -> f64 {
    1.0
}

fn yes() -> bool {
    true
}

impl Default for ImportSettings {
    fn default() -> Self {
        ImportSettings {
            global_scale: 1.0,
            use_file_scale: true,
            bake_axis_conversion: false,
            import_blend_shapes: true,
            preserve_hierarchy: false,
        }
    }
}

/// レンダラー 1 つ。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Renderer {
    /// 相手の根からの道（名前を `/` でつなぐ。表示と知らせ用）。
    pub path: String,
    /// `models[].id`。
    pub model: u32,
    /// FBX の中のメッシュのノードの道（Unity の根からの名前を `/` で）。
    pub node: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub skinned: bool,
    /// BlendShape の名前 → 重み（0〜100）。
    #[serde(default)]
    pub blend_shapes: BTreeMap<String, f32>,
    /// サブメッシュの順に、`materials` の番号。
    #[serde(default)]
    pub materials: Vec<u32>,
}

/// 骨 1 つのローカル（FBX の親の骨に対する値）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BoneValue {
    pub model: u32,
    /// FBX の中の骨のノードの道（空はその FBX の根に当たるノード）。
    pub node: String,
    pub local: Local,
}

/// T・R（x, y, z, w）・S。
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Local {
    pub t: [f32; 3],
    pub r: [f32; 4],
    pub s: [f32; 3],
}

/// マテリアルの無いサブメッシュの鍵（`materials[].key`）。
pub const KEY_NONE: &str = "none";

/// マテリアル 1 つ。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Material {
    /// Unity のマテリアルの身元（セットを結ぶ鍵）: アセットは `guid:<32 桁>/fileid:<数>`、シーンの中のマテリアルは
    /// `object:<GlobalObjectId>`、GlobalObjectId が空の物は `instance:<InstanceID>`、マテリアルの無いサブメッシュは [`KEY_NONE`]。
    pub key: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub shader: Shader,
    #[serde(default)]
    pub values: Values,
    #[serde(default)]
    pub textures: Vec<TextureRef>,
}

/// シェーダーの身元。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Shader {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub guid: String,
    /// シェーダーのアセットが入っている UPM パッケージの名前（Assets の中・組み込みは空）。
    #[serde(default)]
    pub package: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub render_queue: Option<i32>,
}

/// マテリアルの値（プロパティの名前 → 値）。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Values {
    #[serde(default)]
    pub floats: BTreeMap<String, f32>,
    #[serde(default)]
    pub colors: BTreeMap<String, [f32; 4]>,
    #[serde(default)]
    pub vectors: BTreeMap<String, [f32; 4]>,
    #[serde(default)]
    pub ints: BTreeMap<String, i32>,
}

/// テクスチャのプロパティ 1 つ。`path` が無い（null）のは、Unity の中にしかない絵（ファイルが無い）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextureRef {
    pub property: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub guid: String,
    #[serde(default = "yes")]
    pub srgb: bool,
    #[serde(default)]
    pub normal_map: bool,
    #[serde(default = "unit_scale")]
    pub scale: [f32; 2],
    #[serde(default)]
    pub offset: [f32; 2],
}

fn unit_scale() -> [f32; 2] {
    [1.0, 1.0]
}

/// 送れなかった・合わなかった物 1 つ（`path` はレンダラーの道か、ファイルの道）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Problem {
    #[serde(default)]
    pub path: String,
    pub reason: String,
}

impl Problem {
    pub fn new(path: impl Into<String>, reason: Reason) -> Problem {
        Problem {
            path: path.into(),
            reason: reason.as_str().to_owned(),
        }
    }

    /// 知っている理由（知らない言葉は None）。
    pub fn known_reason(&self) -> Option<Reason> {
        Reason::parse(&self.reason)
    }
}

impl Request {
    /// `models[].id` から、`models` の並びの番号。
    pub fn model_index(&self, id: u32) -> Option<usize> {
        self.models.iter().position(|m| m.id == id)
    }
}

// ───────── 返事 ─────────

/// 返事の種類。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplyKind {
    /// 受けて開いた・送り直しを当てた。
    Opened,
    /// 受けなかった（`problems` に理由）。
    Refused,
    /// 利用者が書き出した（`files` に書いた PNG）。
    Exported,
}

/// 返事 `outbox/<id>-<n>.json`（スタンドアロン → Unity）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Reply {
    pub format: u32,
    /// 頼みの `id`。
    pub request: String,
    pub kind: ReplyKind,
    pub app: AppInfo,
    #[serde(default)]
    pub problems: Vec<Problem>,
    #[serde(default)]
    pub files: Vec<ExportedFile>,
}

/// 返事を書いたアプリ。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AppInfo {
    pub version: String,
}

/// 書き出した PNG 1 つ（どのマテリアルのどのプロパティか）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExportedFile {
    /// マテリアルの `key`。
    pub material: String,
    pub property: String,
    /// 書いた PNG の絶対の道（区切りは `/`）。
    pub path: String,
    pub srgb: bool,
    pub normal_map: bool,
}

impl Reply {
    pub fn new(request: &str, kind: ReplyKind, version: &str) -> Reply {
        Reply {
            format: FORMAT,
            request: request.to_owned(),
            kind,
            app: AppInfo {
                version: version.to_owned(),
            },
            problems: Vec::new(),
            files: Vec::new(),
        }
    }
}

// ───────── 起きている印 ─────────

/// 起きている印 `presence.json`（スタンドアロンが受け付けている間、[`PRESENCE_EVERY`] ごとに書き直す）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Presence {
    pub format: u32,
    pub app: String,
    pub version: String,
    pub pid: u32,
    /// 書いた時刻（UTC の RFC 3339。例 `2026-10-07T12:00:00Z`）。
    pub updated: String,
}

impl Presence {
    /// このプロセスの、今の時刻の印。
    pub fn now(version: &str) -> Presence {
        Presence {
            format: FORMAT,
            app: "YoluPainter".into(),
            version: version.to_owned(),
            pid: std::process::id(),
            updated: utc_text(SystemTime::now()),
        }
    }
}

/// UTC の RFC 3339（秒まで、`Z`）。1970 年より前は 1970-01-01T00:00:00Z。
pub fn utc_text(at: SystemTime) -> String {
    let secs = at.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()) as i64;
    let (days, rest) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // 日数 → 暦（Howard Hinnant の civil_from_days）
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

/// RFC 3339 の UTC（`utc_text` の形。秒の小数とタイムゾーンの `+00:00` も受ける）から時刻。読めなければ None。
pub fn parse_utc(text: &str) -> Option<SystemTime> {
    let b = text.as_bytes();
    if b.len() < 20 || b[4] != b'-' || b[7] != b'-' || !matches!(b[10], b'T' | b't') {
        return None;
    }
    let num = |r: std::ops::Range<usize>| text.get(r)?.parse::<i64>().ok();
    let (year, month, day) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (hour, minute, second) = (num(11..13)?, num(14..16)?, num(17..19)?);
    let mut rest = &text[19..];
    if let Some(frac) = rest.strip_prefix('.') {
        let digits = frac.bytes().take_while(u8::is_ascii_digit).count();
        rest = &frac[digits..];
    }
    if !matches!(rest, "Z" | "z" | "+00:00") {
        return None;
    }
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }
    // 暦 → 日数（days_from_civil）
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let secs = days * 86_400 + hour * 3600 + minute * 60 + second;
    u64::try_from(secs)
        .ok()
        .map(|s| UNIX_EPOCH + Duration::from_secs(s))
}

// ───────── 読み ─────────

/// 頼みを受けなかった理由（返事の `problems` にする）。
#[derive(Clone, Debug, PartialEq)]
pub struct Refusal {
    pub reason: Reason,
    /// 人に見せる詳しい理由（日本語。知らせに添える。返事には書かない）。
    pub detail: String,
}

impl Refusal {
    fn new(reason: Reason, detail: impl Into<String>) -> Refusal {
        Refusal {
            reason,
            detail: detail.into(),
        }
    }
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}（{}）", self.detail, self.reason.as_str())
    }
}

/// ID が返事のファイルの名前にできる形か（1〜64 文字の英数字と `-`・`_`）。
pub fn valid_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// 頼みのバイト列を読む（大きさ・版・種類・形・数の上限・有限の数を確かめる）。
pub fn read_request(bytes: &[u8]) -> Result<Request, Refusal> {
    if bytes.len() as u64 > MAX_REQUEST_BYTES {
        return Err(Refusal::new(
            Reason::TooLarge,
            format!("頼みが大きすぎます（{} バイト）", bytes.len()),
        ));
    }
    let value: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|e| Refusal::new(Reason::FormatUnknown, format!("JSON として読めません: {e}")))?;
    // 版と種類は、形を読む前に見る（新しい版の頼みを「形が違う」ではなく「版が違う」で断る）
    let format = value.get("format").and_then(serde_json::Value::as_u64);
    if format != Some(FORMAT as u64) {
        return Err(Refusal::new(
            Reason::FormatUnknown,
            format!(
                "形式の版が違います（{}）",
                value.get("format").map_or("無し".into(), |v| v.to_string())
            ),
        ));
    }
    let kind = value.get("kind").and_then(serde_json::Value::as_str);
    if kind != Some(KIND_OPEN) {
        return Err(Refusal::new(
            Reason::FormatUnknown,
            format!("知らない種類です（{}）", kind.unwrap_or("無し")),
        ));
    }
    let request: Request = serde_json::from_value(value)
        .map_err(|e| Refusal::new(Reason::FormatUnknown, format!("頼みの形が違います: {e}")))?;
    check(&request)?;
    Ok(request)
}

fn check(r: &Request) -> Result<(), Refusal> {
    let bad = |what: String| Err(Refusal::new(Reason::FormatUnknown, what));
    let large = |what: &str, n: usize, max: usize| {
        if n > max {
            Err(Refusal::new(
                Reason::TooLarge,
                format!("{what}が多すぎます（{n}、上限 {max}）"),
            ))
        } else {
            Ok(())
        }
    };
    large("モデル", r.models.len(), MAX_RENDERERS)?;
    large("レンダラー", r.renderers.len(), MAX_RENDERERS)?;
    large("送れなかったレンダラー", r.refused.len(), MAX_RENDERERS)?;
    large("骨", r.bones.len(), MAX_BONES)?;
    large("マテリアル", r.materials.len(), MAX_MATERIALS)?;
    if !valid_id(&r.id) {
        return bad("頼みの id が決まりに合いません".into());
    }
    if r.target.key.trim().is_empty() {
        return bad("target.key がありません".into());
    }
    let mut texts: Vec<&str> = vec![
        &r.id,
        &r.target.key,
        &r.target.name,
        &r.target.export_dir,
        &r.project.root,
        &r.project.name,
        &r.bridge.version,
        &r.bridge.unity,
    ];
    if let Some(root) = &r.root {
        if root.world.len() != 16 || root.world.iter().any(|x| !x.is_finite()) {
            return bad("root.world は有限の数 16 個です".into());
        }
    }
    let mut ids = std::collections::BTreeSet::new();
    for m in &r.models {
        if !ids.insert(m.id) {
            return bad(format!("models の id {} が重なっています", m.id));
        }
        if m.fbx.is_empty() {
            return bad("models[].fbx がありません".into());
        }
        let s = m.import.global_scale;
        if !s.is_finite() || s <= 0.0 {
            return bad("import.global_scale は正の有限の数です".into());
        }
        texts.extend([m.fbx.as_str(), m.guid.as_str()]);
    }
    for x in &r.renderers {
        if !ids.contains(&x.model) {
            return bad(format!(
                "レンダラー「{}」のモデル {} がありません",
                x.path, x.model
            ));
        }
        large("BlendShape", x.blend_shapes.len(), MAX_ENTRIES)?;
        if x.blend_shapes.values().any(|w| !w.is_finite()) {
            return bad(format!(
                "レンダラー「{}」の BlendShape の重みが有限でありません",
                x.path
            ));
        }
        large("サブメッシュ", x.materials.len(), MAX_TEXTURES)?;
        if x.materials.iter().any(|&m| m as usize >= r.materials.len()) {
            return bad(format!(
                "レンダラー「{}」のマテリアルの番号が範囲の外です",
                x.path
            ));
        }
        texts.extend([x.path.as_str(), x.node.as_str()]);
        texts.extend(x.blend_shapes.keys().map(String::as_str));
    }
    for b in &r.bones {
        if !ids.contains(&b.model) {
            return bad(format!("骨「{}」のモデル {} がありません", b.node, b.model));
        }
        let l = &b.local;
        if l.t.iter().chain(&l.r).chain(&l.s).any(|x| !x.is_finite()) {
            return bad(format!("骨「{}」の値が有限でありません", b.node));
        }
        texts.push(&b.node);
    }
    for m in &r.materials {
        if m.key.trim().is_empty() {
            return bad("materials[].key がありません".into());
        }
        large("テクスチャ", m.textures.len(), MAX_TEXTURES)?;
        let v = &m.values;
        for n in [
            v.floats.len(),
            v.colors.len(),
            v.vectors.len(),
            v.ints.len(),
            m.shader.keywords.len(),
        ] {
            large("マテリアルの値", n, MAX_ENTRIES)?;
        }
        let finite = v.floats.values().all(|x| x.is_finite())
            && v.colors.values().flatten().all(|x| x.is_finite())
            && v.vectors.values().flatten().all(|x| x.is_finite())
            && m.textures
                .iter()
                .all(|t| t.scale.iter().chain(&t.offset).all(|x| x.is_finite()));
        if !finite {
            return bad(format!("マテリアル「{}」の値が有限でありません", m.name));
        }
        texts.extend([
            m.key.as_str(),
            m.name.as_str(),
            m.shader.name.as_str(),
            m.shader.guid.as_str(),
            m.shader.package.as_str(),
            m.shader.version.as_str(),
        ]);
        texts.extend(m.shader.keywords.iter().map(String::as_str));
        texts.extend(
            v.floats
                .keys()
                .chain(v.colors.keys())
                .chain(v.vectors.keys())
                .chain(v.ints.keys())
                .map(String::as_str),
        );
        for t in &m.textures {
            texts.push(&t.property);
            texts.push(&t.guid);
            if let Some(p) = &t.path {
                texts.push(p);
            }
        }
    }
    for p in &r.refused {
        texts.extend([p.path.as_str(), p.reason.as_str()]);
    }
    if texts.iter().any(|t| t.len() > MAX_TEXT_BYTES) {
        return Err(Refusal::new(Reason::TooLarge, "文字列が長すぎます"));
    }
    Ok(())
}

// ───────── フォルダ ─────────

/// 受け渡しのフォルダ。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Folder {
    root: PathBuf,
}

/// 拾った頼み（`claimed/` に移したファイル）。拾った受け手だけが持つ印 `claimed/<名前>.lock` も持ち、[`Claimed::finish`] か落としたときに消す
/// （頼みが `claimed/` にある間、同じ名前の頼みを別の受け手に拾わせない）。複製はできない（印を持つのは 1 つだけ）。
#[derive(Debug)]
pub struct Claimed {
    pub path: PathBuf,
    /// ファイルの名前の拡張子の前（頼みが読めないときの返事の名前）。
    pub stem: String,
    /// 持っている印の道（手放したら None）。
    mark: Mutex<Option<PathBuf>>,
}

/// `os`（`std::env::consts::OS` の言葉）の決まりで絶対の道か。走っている OS の `Path::is_absolute` ではなく引数の OS で見る（ほかの OS の
/// 決まりを、どの OS でも試験できるように）。windows は、ドライブ文字つき（`X:\`・`X:/`）か、区切り 2 つと名前で始まる道（UNC・`\\?\`。
/// 区切りは `\` と `/` のどちらでも。UNC のサーバー・共有の中身までは確かめない）。ほかは `/` で始まる道。
fn absolute_for(os: &str, path: &Path) -> bool {
    let bytes = path.as_os_str().as_encoded_bytes();
    if os == "windows" {
        let separator = |b: u8| b == b'\\' || b == b'/';
        matches!(bytes, [drive, b':', next, ..] if drive.is_ascii_alphabetic() && separator(*next))
            || matches!(bytes, [a, b, name, ..] if separator(*a) && separator(*b) && !separator(*name))
    } else {
        bytes.first() == Some(&b'/')
    }
}

/// OS ごとのフォルダ（環境の読み方を引数にして、試験できるようにする）。決まらなければ None。
pub fn folder_for(os: &str, env: impl Fn(&str) -> Option<PathBuf>) -> Option<PathBuf> {
    if let Some(dir) = env(ENV_DIR).filter(|p| absolute_for(os, p)) {
        return Some(dir);
    }
    let absolute = |key| env(key).filter(|p| absolute_for(os, p));
    let base = match os {
        "windows" => absolute("LOCALAPPDATA")?,
        "macos" => absolute("HOME")?
            .join("Library")
            .join("Application Support"),
        _ => absolute("XDG_DATA_HOME")
            .or_else(|| absolute("HOME").map(|h| h.join(".local").join("share")))?,
    };
    Some(base.join("YoluPainter").join("LiveLink"))
}

/// このプロセスの環境のフォルダ。
pub fn default_folder() -> Option<PathBuf> {
    folder_for(std::env::consts::OS, |key| {
        std::env::var_os(key).map(PathBuf::from)
    })
}

/// 書いて閉じてから置き換える（`<名前>.tmp` に書き、フラッシュして閉じ、最終の名前へ）。前の `.tmp` が残っていれば消してから書く。
pub fn write_replacing(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(TMP_SUFFIX);
    let tmp = PathBuf::from(tmp);
    match std::fs::remove_file(&tmp) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let result = (|| {
        let mut file = create_private_file(&tmp, false)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// 読む（名前がシンボリックリンクなら辿らない。普通のファイルで、Unix では持ち主が自分のときだけ。`max` バイトを超えたら断る）。
pub fn read_own_file(path: &Path, max: u64) -> io::Result<Vec<u8>> {
    let file = crate::private::open_private_file(path)?;
    let meta = file.metadata()?;
    if !meta.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "普通のファイルではありません",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.uid() != unsafe { libc::geteuid() } {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "持ち主が別のユーザーのファイルです",
            ));
        }
    }
    if meta.len() > max {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "大きすぎます"));
    }
    let mut out = Vec::with_capacity(meta.len() as usize);
    file.take(max + 1).read_to_end(&mut out)?;
    if out.len() as u64 > max {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "大きすぎます"));
    }
    Ok(out)
}

impl Folder {
    /// フォルダを使えるようにする（無ければ作る。根と `inbox`・`claimed`・`outbox` が自分だけのものでなければ断る）。
    pub fn open(root: impl Into<PathBuf>) -> io::Result<Folder> {
        let root = root.into();
        if let Some(parent) = root.parent() {
            std::fs::create_dir_all(parent)?;
        }
        ensure_private_dir(&root)?;
        let folder = Folder { root };
        for dir in [folder.inbox(), folder.claimed(), folder.outbox()] {
            ensure_private_dir(&dir)?;
        }
        Ok(folder)
    }

    /// 作らずに、今ある自分だけのフォルダとして使う（無い・自分だけのものでなければ断る）。
    pub fn existing(root: impl Into<PathBuf>) -> io::Result<Folder> {
        let folder = Folder { root: root.into() };
        check_private_dir(&folder.root)?;
        Ok(folder)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn inbox(&self) -> PathBuf {
        self.root.join("inbox")
    }
    pub fn claimed(&self) -> PathBuf {
        self.root.join("claimed")
    }
    pub fn outbox(&self) -> PathBuf {
        self.root.join("outbox")
    }
    pub fn presence_path(&self) -> PathBuf {
        self.root.join(PRESENCE)
    }

    /// 起きている印を書き直す。
    pub fn write_presence(&self, presence: &Presence) -> io::Result<()> {
        let bytes = serde_json::to_vec(presence).map_err(io::Error::other)?;
        write_replacing(&self.presence_path(), &bytes)
    }

    /// 起きている印を消す（無ければ何もしない）。
    pub fn remove_presence(&self) -> io::Result<()> {
        match std::fs::remove_file(self.presence_path()) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }

    /// 起きている印を読む（無ければ None）。
    pub fn read_presence(&self) -> io::Result<Option<Presence>> {
        match read_own_file(&self.presence_path(), 64 << 10) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// `inbox/` の頼み（`.json` で終わる名前。`.tmp` は見ない）。古い順（更新の時刻、同じなら名前）。
    pub fn waiting(&self) -> io::Result<Vec<PathBuf>> {
        json_files(&self.inbox())
    }

    /// `claimed/` に残っている頼み（拾った後に終わらなかった物）。
    pub fn claimed_files(&self) -> io::Result<Vec<PathBuf>> {
        json_files(&self.claimed())
    }

    /// `outbox/` の返事（試験・Unity の側の読み）。
    pub fn replies(&self) -> io::Result<Vec<PathBuf>> {
        json_files(&self.outbox())
    }

    /// 頼みを拾う。`claimed/<名前>.lock` を `create_new` で作れた受け手だけが、`inbox/` から `claimed/` へ頼みを移して受ける。印が既にある
    /// （ほかの受け手が拾っている最中か、拾った頼みをまだ持っている）なら、頼みには触れず None。印を作れても頼みがもう無い
    /// （先に拾われて終わった）なら、印を消して None。印を作れなかった理由が「既にある」でなければ（権限・ディスクなど）、None にせず断る。
    pub fn claim(&self, inbox_file: &Path) -> io::Result<Option<Claimed>> {
        let Some(name) = inbox_file.file_name() else {
            return Ok(None);
        };
        let mark = self.mark_for(name);
        match create_private_file(&mark, false) {
            Ok(file) => drop(file),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => return Ok(None),
            Err(e) => return Err(e),
        }
        // ここから先は、勝てずに出る道（早い return・panic も）でも `claimed` を落とせば印が消える
        let to = self.claimed().join(name);
        let stem = to
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let claimed = Claimed {
            path: to,
            stem,
            mark: Mutex::new(Some(mark)),
        };
        match std::fs::rename(inbox_file, &claimed.path) {
            Ok(()) => Ok(Some(claimed)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// 頼みのファイルの名前から、拾っている印の道。
    fn mark_for(&self, name: &std::ffi::OsStr) -> PathBuf {
        let mut mark = name.to_owned();
        mark.push(LOCK_SUFFIX);
        self.claimed().join(mark)
    }

    /// 返事を書く（`outbox/<頼みの id>-<n>.json`）。id が名前にできなければ断る。
    pub fn write_reply(&self, reply: &Reply, n: u32) -> io::Result<PathBuf> {
        if !valid_id(&reply.request) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "返事の id が決まりに合いません",
            ));
        }
        let path = self.outbox().join(format!("{}-{n}.json", reply.request));
        let bytes = serde_json::to_vec_pretty(reply).map_err(io::Error::other)?;
        write_replacing(&path, &bytes)?;
        Ok(path)
    }

    /// `claimed/` に残った、`age` より古い頼みと拾っている印を消して、消したファイルの数を返す（落ちたプロセスが残した物。新しい物は
    /// ほかの受け手が当てている最中かもしれない）。古い印は、頼みがまだ `inbox/` にあれば、次の受け手が拾えるようになる（印を作った
    /// 直後に落ちた受け手が残した形）。頼みを先に、印を後に消す（[`Claimed::finish`] と同じ順）。
    pub fn sweep_claimed(&self, age: Duration) -> io::Result<usize> {
        let now = SystemTime::now();
        let mut removed = 0;
        let leftovers = self.claimed_files()?.into_iter().chain(self.marks()?);
        for path in leftovers {
            let old = std::fs::symlink_metadata(&path)
                .and_then(|m| m.modified())
                .is_ok_and(|t| now.duration_since(t).is_ok_and(|d| d > age));
            if old && std::fs::remove_file(&path).is_ok() {
                removed += 1;
            }
        }
        Ok(removed)
    }

    /// `claimed/` の拾っている印（`.lock` で終わる名前）。
    fn marks(&self) -> io::Result<Vec<PathBuf>> {
        files_ending(&self.claimed(), LOCK_SUFFIX)
    }
}

impl Claimed {
    /// 拾った頼みを読む（ファイルが読めない・大きすぎるときも断り）。
    pub fn read(&self) -> Result<Request, Refusal> {
        let bytes = read_own_file(&self.path, MAX_REQUEST_BYTES).map_err(|e| {
            if e.kind() == io::ErrorKind::InvalidData && e.to_string().contains("大きすぎ") {
                Refusal::new(Reason::TooLarge, "頼みが大きすぎます")
            } else {
                Refusal::new(Reason::FormatUnknown, format!("頼みを読めません: {e}"))
            }
        })?;
        read_request(&bytes)
    }

    /// 終えた頼みを消し、その後で拾っている印を消す。印を持っている間だけ消す（2 回呼んでもよい。2 回目は何もしない。印を手放した後に、
    /// 同じ名前の頼みをほかの受け手が拾っていても、その頼みを消さない）。頼みを消せなくても印は手放す。
    pub fn finish(&self) -> io::Result<()> {
        let Some(mark) = self.take_mark() else {
            return Ok(());
        };
        let request = remove_if_present(&self.path);
        let released = remove_if_present(&mark);
        request.and(released)
    }

    fn take_mark(&self) -> Option<PathBuf> {
        self.mark
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }

    /// 返事の名前にする ID（頼みが読めればその id、読めなければファイルの名前。どちらも名前にできなければ None）。
    pub fn reply_id(&self, request: Option<&Request>) -> Option<String> {
        request
            .map(|r| r.id.clone())
            .filter(|id| valid_id(id))
            .or_else(|| valid_id(&self.stem).then(|| self.stem.clone()))
    }
}

/// 消す（無ければ何もしない）。
fn remove_if_present(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// 拾った頼みを落としたら、持っている印を消す（頼みのファイルは `claimed/` に残す。古くなれば `sweep_claimed` が消す）。
impl Drop for Claimed {
    fn drop(&mut self) {
        if let Some(mark) = self.take_mark() {
            let _ = remove_if_present(&mark);
        }
    }
}

/// フォルダの中の `.json` のファイル（シンボリックリンク・フォルダ・`.tmp`・`.lock` は除く）。古い順。
fn json_files(dir: &Path) -> io::Result<Vec<PathBuf>> {
    files_ending(dir, ".json")
}

/// フォルダの中の、名前が `suffix` で終わるファイル（`.` で始まる名前・シンボリックリンク・フォルダは除く）。古い順。
fn files_ending(dir: &Path, suffix: &str) -> io::Result<Vec<PathBuf>> {
    let mut out: Vec<(SystemTime, PathBuf)> = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.ends_with(suffix) || name.starts_with('.') {
            continue;
        }
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if !meta.file_type().is_file() {
            continue;
        }
        out.push((meta.modified().unwrap_or(UNIX_EPOCH), path));
    }
    out.sort();
    Ok(out.into_iter().map(|(_, p)| p).collect())
}

#[cfg(test)]
mod tests;
