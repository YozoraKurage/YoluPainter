//! core の型と io の分類から、画面用の短い理由を作る。
use super::Lang;
use crate::stencil::StencilError;
use crate::view3d::model::ViewError;
use yolu_core::generator::{self, anchor};
use yolu_core::geometry::GeometryError;
use yolu_core::skin::RigError;
use yolu_core::{CoreError, FallbackEffect, InactiveEffect, InactiveReason, InactiveTarget};
use yolu_model::ModelError;

use crate::library::{REFUSAL_PNG, REFUSAL_PNG_SIZE, REFUSAL_UNSUPPORTED};
use crate::shelf::Block;
use crate::update::Failure as UpdateFailure;
use crate::view3d::pose::hide::store::StoreError as HideStoreError;
use crate::view3d::pose::presets::store::StoreError as PoseStoreError;
use yolu_core::paths::{self, RebindError};
use yolu_io::brushes::BrushImportError;
use yolu_io::library as files;
use yolu_io::psd::CopyRefusal;
use yolu_io::shelf::{REFUSAL_ARCHIVE_BUDGET, REFUSAL_MEMORY_BUDGET, REFUSAL_RESOURCE_COUNT};
use yolu_io::smart::{
    REFUSAL_GENERATORS, REFUSAL_IMAGES, REFUSAL_NEW_FILTERS, REFUSAL_PATH_LISTS,
    REFUSAL_POINT_GRADIENTS, REFUSAL_RUST_ADJUSTMENTS, REFUSAL_RUST_GENERATORS,
    REFUSAL_USER_CHANNELS,
};

/// 効いているロックの名前（「すべて」が付いていればそれだけ。複数なら「、」でつなぐ）。名前の表は `layerops::lock_name` の 1 つだけで、
/// レイヤーの欄の錠の印・プロパティ・ロックの付け外しの状態の文と、断りの文が同じ言い方になる。
fn lock_list(lang: Lang, lock: yolu_core::LayerLocks) -> String {
    use yolu_core::LayerLocks as L;
    if lock.contains(L::ALL) {
        return crate::layerops::lock_name(lang, L::ALL).into();
    }
    crate::layerops::lock_names(lang, lock).join(lang.pick("、", ", "))
}

/// 設定の予算で断った理由（yolu-io の `OVER_LAYER_PIXELS_*`）の英語。どの予算かだけを言う（数は出さない）。
pub(crate) fn budget_text(text: &str) -> Option<&'static str> {
    if text.contains(yolu_io::OVER_LAYER_PIXELS_DOCUMENT) {
        Some("A project exceeds the Layer memory budget")
    } else if text.contains(yolu_io::OVER_LAYER_PIXELS_TOTAL) {
        Some("The whole exceeds the Layer memory budget")
    } else if text.contains("グループの入れ子の上限") {
        Some("Groups are nested too deeply")
    } else if text.contains("キャンバスの辺の上限") {
        Some("The canvas edge exceeds the limit (8192)")
    } else if text.contains("レイヤーの数の上限") {
        Some("Too many layers (limit 2048)")
    } else {
        None
    }
}

impl Lang {
    pub fn core_error(self, error: &CoreError) -> String {
        crate::crash::problem(self.core_error_text(error))
    }
    /// core の誤りの文。同じ誤りはどの操作でも同じ文にする（前は、塗るツール・棚・覚えた選択範囲がそれぞれ別の文を持っていた）。
    /// 下の個別の文の無い誤りは、日本語は core の文、英語は種類ごとの短い文。
    fn core_error_text(self, error: &CoreError) -> String {
        match error {
            CoreError::LayerLocked {
                layer,
                holder,
                lock,
            } => {
                let locks = self.quote(&lock_list(self, *lock));
                match (self, layer == holder) {
                    (Lang::Ja, true) => format!("レイヤーの{locks}がロックされています"),
                    (Lang::Ja, false) => format!("親グループの{locks}がロックされています"),
                    (Lang::En, true) => format!("The layer has {locks} locked"),
                    (Lang::En, false) => format!("A parent group has {locks} locked"),
                }
            }
            CoreError::Cancelled => self.pick("取り消しました", "Cancelled").into(),
            CoreError::StrokeActive => super::refusals::during_stroke(self).into(),
            CoreError::NoActiveStroke => self
                .pick("ストロークは終わっています", "Stroke already ended")
                .into(),
            CoreError::LayerNotFound
            | CoreError::InvalidArgument("保存するレイヤーがありません") => {
                self.pick("レイヤーがありません", "Layer not found").into()
            }
            CoreError::ChannelNotFound => self
                .pick("チャンネルがありません", "Channel not found")
                .into(),
            // 予算は設定のウィンドウの名前（レイヤーのメモリ・1 回の操作）で言う
            CoreError::SourceBudgetExceeded => self
                .pick(
                    "レイヤーのメモリの予算を超えます",
                    "Over the Layer memory budget",
                )
                .into(),
            CoreError::StrokeBudgetExceeded => self
                .pick(
                    "1 回の操作のメモリの予算を超えます（取り消しました）",
                    "Over the memory budget of one operation (cancelled)",
                )
                .into(),
            CoreError::WorkingBudgetExceeded => self
                .pick(
                    "作業のメモリの上限を超えます",
                    "Working memory budget exceeded",
                )
                .into(),
            CoreError::Unsupported("ユーザーチャンネルの対応が一致しません") => {
                self.pick("チャンネルが合いません", "Channels do not match")
                    .into()
            }
            CoreError::InvalidArgument("レイヤーは2048個までです") => {
                self.pick("レイヤーが多すぎます", "Too many layers").into()
            }
            CoreError::InvalidArgument("保存するマスクがありません") => {
                self.pick("マスクがありません", "No mask").into()
            }
            CoreError::InvalidArgument("スマート素材の名前") => {
                self.pick("名前が使えません", "Name not allowed").into()
            }
            CoreError::InvalidArgument("配置先がグループではありません") => self
                .pick("置き先がグループではありません", "Target is not a group")
                .into(),
            CoreError::InvalidArgument("選択範囲の名前が長すぎる") => self.pick(
                format!(
                    "名前が長すぎます（{} 文字まで）。",
                    yolu_core::MAX_SAVED_NAME_CHARS
                ),
                format!(
                    "The name is too long (up to {} characters).",
                    yolu_core::MAX_SAVED_NAME_CHARS
                ),
            ),
            CoreError::InvalidArgument("同じ名前の選択範囲がある") => self
                .pick(
                    "同じ名前の選択範囲があります。",
                    "A saved selection with that name exists.",
                )
                .into(),
            // core の文（「できない: 〜」のようにコロンでつないだ形）は、何が（なぜ）の 1 つの文に組み直す
            CoreError::MergeRefused(reason) => self.with_reason(
                self.pick("結合できません", "Cannot merge"),
                self.pick(reason.to_string(), merge_refusal(*reason).to_owned()),
            ),
            CoreError::MergeAppearance(report) => self.with_reason(
                self.pick(
                    "結合で見た目が許容差を超えて変わります",
                    "Merge changes the appearance beyond the tolerance",
                ),
                report.max_visible_difference.to_string(),
            ),
            CoreError::InactiveEffect { reason, .. } => self.with_reason(
                self.pick(
                    "効いていない効果は焼き込めません",
                    "Cannot bake an inactive effect",
                ),
                self.inactive_reason(reason),
            ),
            CoreError::InvalidArgument(what) => self.with_reason(
                self.pick("値が範囲外です", "Invalid value"),
                self.pick(*what, core_reason(what)),
            ),
            CoreError::Unsupported(what) => {
                self.with_reason(self.pick(*what, core_reason(what)), "")
            }
            _ if self == Self::Ja => error.to_string(),
            CoreError::Clipboard(reason) => clipboard_refusal(*reason).into(),
            CoreError::BatchActive => "Not allowed inside a batch of edits".into(),
            CoreError::TileUnreadable => "Cannot read a tile back from the disk cache".into(),
        }
    }

    /// .ylp・ファイルの失敗の文。日本語は診断（どの項目か）をそのまま出し、英語は種類ごとの短い文にする
    /// （診断の本文は日本語なので、英語のウィンドウには出さない）。OS のエラーは番号を添える。
    pub fn io_error(self, error: &yolu_io::Error) -> String {
        crate::crash::problem(self.io_error_text(error))
    }
    fn io_error_text(self, error: &yolu_io::Error) -> String {
        use yolu_io::{Error, Unwritable};
        match error {
            Error::Core(e) => self.core_error(e),
            Error::Io(e) => self.file_error(e),
            Error::Json(e) => self.pick(error.to_string(), format!("Invalid JSON: {e}")),
            Error::InvalidData(text) => {
                self.pick(text.clone(), "Invalid or unsupported project data".into())
            }
            // 設定の予算で断ったものは、どの予算かを英語でも言う（yolu-io の理由の文で見分ける）
            Error::Budget(text) => self.pick(
                text.clone(),
                budget_text(text)
                    .unwrap_or("Size, count or memory limit exceeded")
                    .into(),
            ),
            Error::Unwritable(what) => self.pick(
                what.to_string(),
                match what {
                    Unwritable::GeneratorRampMixing => {
                        "Color mixing in a fill gradient cannot be saved to .ylp".into()
                    }
                },
            ),
            // 保存の衝突のうち、保存先が外で変わったのではない理由（yolu-io の store.rs）は言い分ける。
            // 別の保存がロックを持っているのは待ち、前の版の置き場とロックの場所の不具合は保存先の周りの事情
            Error::SaveConflict(text) if text.contains("進行中") => {
                self.pick(text.clone(), "Another save is in progress".into())
            }
            Error::SaveConflict(text)
                if text.contains("バックアップ先がフォルダーではありません") =>
            {
                self.pick(text.clone(), "Backup location is not a folder".into())
            }
            Error::SaveConflict(text) if text.contains("バックアップ先がシンボリックリンク") => {
                self.pick(text.clone(), "Backup location is a link".into())
            }
            Error::SaveConflict(text) if text.contains("ロックのファイル") => {
                self.pick(text.clone(), "Save lock file is a link".into())
            }
            // 保存先の名前（.ylp で終わらない）と、保存の直前に外から作られた新規の保存先
            Error::SaveConflict(text) if text.contains(".ylp で終わっていません") => {
                self.pick(text.clone(), "The file name must end with .ylp".into())
            }
            Error::SaveConflict(text) if text.contains("外部で作られました") => self.pick(
                text.clone(),
                "A file appeared at the save target; not overwritten".into(),
            ),
            // 保存先が、開いたあとで外で消された・動かされた（変わったのとは言い分ける）
            Error::SaveConflict(text) if text.contains("外部で消されています") => self
                .pick(
                    text.clone(),
                    "The save target was deleted or moved outside; not overwritten".into(),
                ),
            // 開いた .ylp の古い版の写し（置換の規則が POSIX でないファイルシステムで、保存が置き換えるために手放した）
            Error::SaveConflict(text) if text.contains(yolu_io::SOURCE_RELEASED) => self.pick(
                text.clone(),
                "The opened .ylp was replaced by a save; this copy can no longer be read".into(),
            ),
            Error::SaveConflict(text) => {
                self.pick(text.clone(), "Save target or backup changed".into())
            }
            Error::UnsupportedFormat {
                format,
                app,
                version,
            } => self.pick(
                format!(
                    "未対応の .ylp の形式 {format} です（{app} {version} で保存。上限 {}）",
                    yolu_io::MAX_FORMAT
                ),
                format!(
                    "The .ylp format {format} is not supported (saved by {app} {version}; maximum {})",
                    yolu_io::MAX_FORMAT
                ),
            ),
        }
    }

    /// 読み書きの失敗の文。種類で言い分け、OS のエラー番号（共有違反・空き不足などの手掛かり）を添える。
    pub fn file_error(self, error: &std::io::Error) -> String {
        crate::crash::problem(self.file_error_text(error))
    }
    fn file_error_text(self, error: &std::io::Error) -> String {
        use std::io::ErrorKind::*;
        let reason = match error.kind() {
            NotFound => self.pick(
                "ファイルまたはフォルダーがありません",
                "File or folder not found",
            ),
            PermissionDenied => self.pick("アクセスが拒否されました", "Access denied"),
            AlreadyExists => self.pick("ファイルが既にあります", "File already exists"),
            InvalidData => self.pick("ファイルのデータが不正です", "Invalid file data"),
            StorageFull => self.pick("ディスクの空きがありません", "Disk full"),
            QuotaExceeded => self.pick("容量の割り当てを超えました", "Disk quota exceeded"),
            ReadOnlyFilesystem => self.pick("読み取り専用の場所です", "Read-only location"),
            ResourceBusy => self.pick("ファイルが使用中です", "File in use"),
            FileTooLarge => self.pick("ファイルが大きすぎます", "File too large"),
            _ => self.pick("ファイルの読み書きに失敗しました", "File I/O failed"),
        };
        match error.raw_os_error() {
            Some(code) => self.pick(
                format!("{reason}（OS エラー {code}）"),
                format!("{reason} (OS error {code})"),
            ),
            None => reason.into(),
        }
    }

    /// メッシュマップの誤りの文（core は日本語の文だけを持つので、英語は知っている理由の英語か、一般の文）。
    pub fn mesh_map_error(self, error: &yolu_core::mesh_maps::MeshMapError) -> String {
        crate::crash::problem(self.pick(error.0.clone(), core_reason(&error.0).to_owned()))
    }

    /// 別のスレッドを起こせなかった理由（OS の英語の文をそのまま出さず、種類で言い分けて、OS のエラー番号を添える）。
    /// 何ができなかったかは呼ぶ側が `with_reason` の「何が」で言う。
    pub fn thread_error(self, error: &std::io::Error) -> String {
        use std::io::ErrorKind::*;
        let reason = match error.kind() {
            WouldBlock | OutOfMemory => {
                self.pick("システムの資源が足りません", "Not enough system resources")
            }
            _ => self.pick(
                "システムが処理の開始を断りました",
                "The system refused to start the task",
            ),
        };
        crate::crash::problem(match error.raw_os_error() {
            Some(code) => self.pick(
                format!("{reason}（OS エラー {code}）"),
                format!("{reason} (OS error {code})"),
            ),
            None => reason.into(),
        })
    }

    /// ステンシルの画像を読めない理由（core の断り・ファイルの失敗は他のウィンドウと同じ文を通す）。
    pub fn stencil_error(self, error: &StencilError) -> String {
        crate::crash::problem(self.stencil_error_text(error))
    }
    fn stencil_error_text(self, error: &StencilError) -> String {
        match error {
            StencilError::Core(e) => self.core_error(e),
            StencilError::File(e) => self.file_error(e),
            StencilError::NotPng => self
                .pick("PNG として読めません", "Not a readable PNG")
                .into(),
            StencilError::TooLarge {
                width,
                height,
                side,
            } => self.pick(
                format!("画像が大きすぎます（{width} × {height}、1 辺は {side} まで）"),
                format!("Image too large ({width} × {height}; maximum side {side})"),
            ),
            StencilError::Limits => self
                .pick(
                    "画像が大きすぎます（メモリの上限）",
                    "Image too large (memory limit)",
                )
                .into(),
        }
    }

    /// 開いたときの知らせの短い文（日本語は診断をそのまま、英語は種類ごとに）。
    pub fn project_note(self, note: &yolu_io::Note) -> String {
        use yolu_io::Note;
        if self == Self::Ja {
            return note.to_string();
        }
        match note {
            Note::ViewSlotUnreadable(_) => "Unreadable material slot in view.json; using 0".into(),
            Note::Migrated { format } => {
                format!("Migrated format {format} to the format 7 layout in memory")
            }
            Note::MaterialRefsMigrated { format } => {
                format!("Migrated format {format} material references to format 7 in memory")
            }
            Note::UnknownEntryKept(name) => format!("Unsupported entry kept as is: {name}"),
            Note::SetNotConvertible { set, issue } => {
                format!(
                    "Texture set “{set}” cannot be converted: {}",
                    issue_key(issue)
                )
            }
            Note::SmartResourceKept(name) => {
                format!("Smart resource “{name}” kept as a file (no preview expansion)")
            }
            Note::BrushKept => {
                "Brush settings and images kept as is (not validated or executed)".into()
            }
        }
    }

    /// 効果が入力のまま通している理由（日本語は core の文、英語は種類ごとの短い文。core が理由の文だけを持つ設定の不備は一般の文）。
    pub fn inactive_reason(self, reason: &InactiveReason) -> String {
        use generator::Inactive as I;
        if self == Self::Ja {
            return reason.to_string();
        }
        match reason {
            InactiveReason::Generator(I::MissingMap(k)) => format!("No {k:?} map"),
            InactiveReason::Generator(I::StaleMap(k)) => {
                format!("The {k:?} map was baked with other settings")
            }
            InactiveReason::Generator(I::UnverifiedMap(k)) => {
                format!("The {k:?} map cannot be verified")
            }
            InactiveReason::Generator(I::MapSize(k)) => {
                format!("The {k:?} map size differs from the texture set")
            }
            InactiveReason::Generator(I::PinMismatch(k)) => {
                format!("The {k:?} map differs from the pinned bake")
            }
            InactiveReason::Generator(I::MissingFrame) => "Model root position is unknown".into(),
            InactiveReason::Generator(I::EmptyBounds) => "Position bounds have zero size".into(),
            InactiveReason::Generator(I::NoIdColors) => "No ID colors selected".into(),
            InactiveReason::Generator(I::Anchor(anchor::Issue::NotChosen)) => {
                "No anchor chosen".into()
            }
            InactiveReason::Generator(I::Anchor(anchor::Issue::Missing)) => {
                "The anchor to read is gone".into()
            }
            InactiveReason::Generator(I::Anchor(anchor::Issue::NotBelow)) => {
                "The anchor is not below its layer".into()
            }
            InactiveReason::Generator(I::NoImage) => "No image chosen".into(),
            InactiveReason::Generator(I::MissingImage) => {
                "The image is not in the project or cannot be read".into()
            }
            InactiveReason::Generator(I::NoModel) => "No model".into(),
            InactiveReason::Generator(I::IslandMap) => {
                "The UV island map does not fit in the memory budget".into()
            }
            InactiveReason::Rejected(why) if why.is_ascii() => why.clone(),
            InactiveReason::Rejected(_) => "Settings cannot be used".into(),
        }
    }

    /// 効かない効果 1 件の 1 文（レイヤーの名前・種類が効いていないことと、その理由）。
    pub fn inactive_effect(self, effect: &InactiveEffect) -> String {
        let name = self.quote(&effect.layer_name);
        let what = match (self, effect.target) {
            (Lang::Ja, InactiveTarget::Generator { mask, kind }) => format!(
                "{name}{}の {} は効いていません",
                if mask { "（マスク）" } else { "" },
                yolu_core::effects::generator_kind_name(kind)
            ),
            (Lang::Ja, InactiveTarget::FillGradient(c)) => {
                format!("{name}（{c:?}）のグラデーションは値を見せています")
            }
            (Lang::Ja, InactiveTarget::Decal) => format!("{name}のデカールは出ていません"),
            (Lang::Ja, InactiveTarget::FillImage(c)) => {
                format!("{name}（{c:?}）は画像を投影していません")
            }
            (Lang::Ja, InactiveTarget::FillPoints(c)) => {
                format!("{name}（{c:?}）の点のグラデーションは値を見せています")
            }
            (Lang::En, target) => {
                let what = match target {
                    InactiveTarget::Generator { mask, kind } => format!(
                        "{}{}",
                        generator_kind_name(kind),
                        if mask { " (mask)" } else { "" }
                    ),
                    InactiveTarget::FillGradient(c) => format!("gradient ({c:?})"),
                    InactiveTarget::Decal => "decal".into(),
                    InactiveTarget::FillImage(c) => format!("image ({c:?})"),
                    InactiveTarget::FillPoints(c) => format!("point gradient ({c:?})"),
                };
                format!("{name} {what} has no effect")
            }
        };
        self.with_reason(what, self.inactive_reason(&effect.reason))
    }

    /// 位置のマップが使えなくて UV の空間で評価しているノイズ・グランジの段 1 件の 1 行（日本語は core の文、英語はレイヤーの名前・種類・理由）。
    pub fn fallback_effect(self, effect: &FallbackEffect) -> String {
        if self == Self::Ja {
            return effect.to_string();
        }
        format!(
            "\"{}\" {}{}: evaluated in UV space. {}",
            effect.layer_name,
            generator_kind_name(effect.kind),
            if effect.mask { " (mask)" } else { "" },
            self.inactive_reason(&effect.reason)
        )
    }

    /// 保存で書き直したセットの効かない効果が、合成の PNG に入っていないことの知らせ（正本には設定が残る）。初めの 1 件の理由を添える。
    pub fn inactive_effects_not_in_composite(
        self,
        set: &str,
        effects: &[InactiveEffect],
    ) -> String {
        let first = effects
            .first()
            .map(|e| self.inactive_effect(e))
            .unwrap_or_default();
        let what = match self {
            Self::Ja => format!(
                "「{set}」の効いていない効果 {} 件は合成の PNG に入っていません",
                effects.len()
            ),
            Self::En => format!(
                "{} inactive effect(s) in \"{set}\" are not in the composite PNG",
                effects.len()
            ),
        };
        format!(" {}", self.with_reason(what, first))
    }

    /// 文書を core へ変換できない項目の一覧の文（初めの 3 つと数）。項目のキーは英数字なので、英語のウィンドウにもそのまま出す。
    pub fn unsupported_features(self, issues: &[String]) -> String {
        let shown = issues
            .iter()
            .take(3)
            .map(|i| self.pick(i.as_str(), issue_key(i)))
            .collect::<Vec<_>>();
        let more = issues.len().saturating_sub(3);
        match self {
            Self::Ja => {
                let mut text = shown.join("、");
                if more > 0 {
                    text += &format!(" ほか {more} 件");
                }
                format!("編集に対応していない中身（{text}）")
            }
            Self::En => {
                let mut text = shown.join(", ");
                if more > 0 {
                    text += &format!(" and {more} more");
                }
                format!("Unsupported project features ({text})")
            }
        }
    }
}

/// 項目の説明（`layers[2].filters（フィルター・Generator）`）から、英数字のキーだけ。
fn issue_key(issue: &str) -> &str {
    issue.split('（').next().unwrap_or(issue)
}

/// Generator の種類の英語の名前（日本語は core の `generator_kind_name`）。
fn generator_kind_name(kind: generator::Kind) -> &'static str {
    match kind {
        generator::Kind::EdgeWear => "Edge wear",
        generator::Kind::Dirt => "Dirt",
        generator::Kind::PositionGradient => "Position gradient",
        generator::Kind::Thickness => "Thickness",
        generator::Kind::Direction => "Direction",
        generator::Kind::ShapeGradient => "Shape gradient",
        generator::Kind::IdColor => "ID color",
        generator::Kind::Anchor => "Anchor",
        generator::Kind::Noise => "Noise",
        generator::Kind::Grunge => "Grunge",
        generator::Kind::Image => "Image",
        generator::Kind::Pattern => "Pattern",
        generator::Kind::Light => "Light",
        generator::Kind::MaskBuilder => "Mask builder",
        generator::Kind::UvIslandVariation => "UV island variation",
    }
}

fn clipboard_refusal(reason: yolu_core::ClipboardRefusal) -> &'static str {
    use yolu_core::ClipboardRefusal::*;
    match reason {
        NoPixels => "This layer has no pixels",
        NothingToCopy { selection: true } => "Nothing inside the selection",
        NothingToCopy { selection: false } => "The layer is empty",
        TooLarge { .. } => "The area to copy is too large",
        OperationBudget { .. } => "The pasted layer exceeds the operation budget",
        NotPaintLayer => "A fill layer cannot be cut",
    }
}

fn merge_refusal(reason: yolu_core::MergeRefusal) -> &'static str {
    use yolu_core::MergeRefusal::*;
    match reason {
        NoLayerBelow => "No layer below",
        LayerBelowIsGroup => "The layer below is a group",
        LayerBelowIsAdjustment => "The layer below is an adjustment layer",
        HiddenLayer => "A layer is hidden",
        IsGroup => "The target is a group",
        NotGroup => "Not a group",
        EmptyGroup => "The group is empty",
        NothingVisible => "No visible layers",
        DifferentGroups => "Layers have different parent groups",
        TooManyRulers => "The merged layer would have too many rulers",
    }
}

/// core の理由の文（`&'static str`）の英語。無ければ、英語の文はそのまま、日本語の文は一般の文に落とす。
fn core_reason(reason: &str) -> &str {
    known_core_reason(reason).unwrap_or(if reason.is_ascii() {
        reason
    } else {
        "Unsupported value or operation"
    })
}

fn known_core_reason(reason: &str) -> Option<&'static str> {
    Some(match reason {
        "手動ID色の数・番号・色が範囲外です" => "Manual ID colors are out of range",
        "リボンの画像が無い" => "The ribbon image is missing",
        "パスレイヤーにはパスが 1 本以上要る" => "A path layer needs at least one path",
        "パスで描けるのはラスターと塗りつぶしレイヤーだけ" => {
            "Paths draw only on raster and fill layers"
        }
        "パスを付けられるのはパスの無いラスターか塗りつぶしレイヤーだけ" => {
            "Paths can be attached only to a raster or fill layer without paths"
        }
        "塗りつぶしレイヤーのパスは画素にできない" => {
            "The paths of a fill layer cannot be rasterized"
        }
        "塗りつぶしレイヤーのパスの画素は塗りつぶしレイヤーだけ" => {
            "Fill-layer path pixels belong to a fill layer"
        }
        "3D のパスは呼び手が形で描いて set_paths へ渡す" => {
            "Paths on a model are drawn with the model and set with set_paths"
        }
        "塗りのパスが 1 つの UV アイランドに収まらない" => {
            "The fill path does not stay on one UV island"
        }
        "手動ID色のモデル指紋が不正です" => {
            "The model fingerprint of the manual ID colors is invalid"
        }
        "手動ID色が別のモデルに属しています" => {
            "The manual ID colors belong to another model"
        }
        "手で選んだアイランドが別のモデルに属しています" => {
            "The chosen islands belong to another model"
        }
        "手で選んだアイランドの番号がモデルにありません" => {
            "A chosen island is not in the model"
        }
        "手で選んだアイランドの数か番号が範囲外です" => {
            "Too many chosen islands, or an island out of range"
        }
        "同じアイランドが「焼かない」と「優先する」の両方にあります" => {
            "The same island is both not baked and preferred"
        }
        "手で選んだアイランドのモデル指紋が不正です" => {
            "The model fingerprint of the chosen islands is invalid"
        }
        "ベイクの優先の値が範囲外です" => {
            "The overlapping UV priority is out of range"
        }
        "UVが0〜1の外です。繰り返し・UDIMのUVはベイクできません" => {
            "UVs outside 0–1 (tiled or UDIM UVs) cannot be baked"
        }
        "マスクへのストロークはチャンネルの合成を読めない" => {
            "A mask stroke cannot read the channel composite"
        }
        "合成の参照元はクローンか色の混ぜの最初のダブの前にだけ決められる" => {
            "The composite source can only be set before the first dab of a clone or color mixing"
        }
        "写像されたダブはクローンか指先だけ" => "Mapped dabs need clone or smudge",
        "写像されたダブはクローン・指先・ぼかし・色の混ぜの伸ばすだけ" => {
            "Mapped dabs need clone, smudge, blur or the smear of color mixing"
        }
        "混ぜるブラシは画素ごとには塗れない（apply_dab で下地を凍結する）" => {
            "A mixing brush cannot paint pixel by pixel"
        }
        "絵の具の量（0〜1）" => "Paint amount (0–1)",
        "絵の具の濃さ（0〜1）" => "Paint density (0–1)",
        "色延び（0〜1）" => "Color stretch (0–1)",
        "写像されたダブの画素" => "Mapped dab pixel",
        "写像されたダブの参照" => "Mapped dab source",
        "写像された画素に参照が無い" => "A mapped pixel needs a source",
        "写像されたダブの画素が重複" => "Duplicate mapped dab pixel",
        "1 区間のダブが百万を超える" => "More than one million dabs per segment",
        "1 区間のデュアルブラシのダブが百万を超える" => {
            "More than one million dual brush dabs per segment"
        }
        "Height → Normal が読むのは Height のチャンネルだけ" => {
            "Height to Normal requires the Height channel"
        }
        "Height → Normal の強さ（±256）" => "Height to Normal strength (±256)",
        "Height の合成は幅 × 高さ × 4 バイト" => {
            "Height buffer size must be width × height × 4 bytes"
        }
        "Normal の合成は幅 × 高さ × 4 バイト" => {
            "Normal buffer size must be width × height × 4 bytes"
        }
        "PassThrough はグループだけ" => "Pass Through requires a group",
        "jitter（0〜1）" => "Jitter (0–1)",
        "purity（−1〜1）" => "Purity (−1–1)",
        "texture depth（0〜1）" => "Texture depth (0–1)",
        "texture scale（0.05〜64）" => "Texture scale (0.05–64)",
        "その番号のチャンネルはもうある" => "Channel ID already exists",
        "ぼかしの半径（1〜64）" => "Blur radius (1–64)",
        "ラスターと塗りつぶし以外には、レイヤーの出力の画素が無い" => {
            "Only raster and fill layers have layer output pixels"
        }
        "グループ以外には、グループの出力が無い" => {
            "Only groups have a group output"
        }
        "調整レイヤーではない" => "Not an adjustment layer",
        "筆圧の曲線（点 2〜16・両端は 0 と 1・間隔 0.02 以上・値 0〜1）" => {
            "Pen pressure curve (2–16 points, ends at 0 and 1, at least 0.02 apart, values 0–1)"
        }
        "筆圧の最小値（0〜1）" => "Pen pressure minimum (0–1)",
        "まとめるレイヤーが無い" => "No layers to merge",
        "ガンマ（0.1〜9.99）" => "Gamma (0.1–9.99)",
        "クローンの位置（±1e7）" => "Clone position (±1e7)",
        "グループでないレイヤーの中に入っている" => "Parent is not a group",
        "グループではない" => "Not a group",
        "グループにだけ入れられる" => "Parent must be a group",
        "グループには描けない" => "Cannot paint a group",
        "グループの中身が続いていない" => "Group children are not contiguous",
        "グループの入れ子が輪になっている" => "Cyclic group hierarchy",
        "グループの入れ子が深すぎる" => "Group nesting is too deep",
        "グループを自分の中へは入れられない" => "Cannot move a group into itself",
        "ステンシルにキャンバスからの写しが無いので、2D のダブは読めない" => {
            "Missing canvas-to-stencil transform for 2D dabs"
        }
        "ステンシルにキャンバスからの写しが無い（画素ごとにステンシルの上の点を渡す）" => {
            "Missing canvas-to-stencil transform"
        }
        "ステンシルの footprint" => "Stencil footprint",
        "ステンシルのミップマップが予算を超える（小さい画像にする）" => {
            "Stencil mipmap budget exceeded"
        }
        "ステンシルの点は画素ごとに 1 つ" => {
            "Stencil point count must match pixel count"
        }
        "ステンシルの画像の画素 / 点" => "Stencil image pixels / points",
        "大きさの変更の準備のあとにプロジェクトが変わった" => {
            "The project changed after the resize was prepared"
        }
        "ステンシルの点（±1e9）" => "Stencil point (±1e9)",
        "ステンシルの画像のバイト数が幅 × 高さ × 4 でない" => {
            "Stencil buffer size must be width × height × 4 bytes"
        }
        "ステンシルの画像の大きさ（1〜8192）" => "Stencil image size (1–8192)",
        "タイルがキャンバスの外" => "Tile outside canvas",
        "タイルのバイト数が違う" => "Invalid tile byte count",
        "歩幅がタイルの一辺の約数でない" => "The step does not divide the tile edge",
        "タイルの座標がキャンバスの外" => "Tile coordinates outside canvas",
        "タイルの長さ" => "Tile length",
        "チャンネルの名前" => "Channel name",
        "チャンネルは 64 まで" => "Maximum 64 channels",
        "デュアルブラシの半径（0.5〜65536）" => "Dual brush radius (0.5–65536)",
        "デュアルブラシの散布" => "Dual brush scatter",
        "デュアルブラシの数" => "Dual brush count",
        "デュアルブラシの真円率" => "Dual brush roundness",
        "デュアルブラシの硬さ" => "Dual brush hardness",
        "デュアルブラシの間隔" => "Dual brush spacing",
        "フェード（0〜10000）" => "Fade (0–10000)",
        "マスクの画素の RGB は 0" => "Mask RGB must be zero",
        "レベル補正の入力の範囲" => "Levels input range",
        "レベル補正の出力の範囲" => "Levels output range",
        "予算が今の画素より小さい" => "Budget is smaller than existing pixels",
        "写した画素のバイト数が幅 × 高さ × 4 でない" => {
            "Copied pixels must be width × height × 4 bytes"
        }
        "写したキャンバスの大きさ" => "Size of the source canvas",
        "写した矩形がキャンバスの外" => "Copied rectangle lies outside the canvas",
        "写した矩形が空" => "Copied rectangle is empty",
        "画素が矩形の外" => "Pixel outside the rectangle",
        "画素を置き換えられるのはラスターレイヤーだけ" => {
            "Only paint layers have pixels to replace"
        }
        "画像の大きさがキャンバスと違う" => "Image size differs from the canvas",
        "無効のチャンネルは切り取れない" => "Cannot cut a disabled channel",
        "無効のチャンネルは置き換えられない" => {
            "Cannot replace a disabled channel"
        }
        "入れ子が輪になっている" => "Cyclic hierarchy",
        "入力の座標が範囲外" => "Input coordinates out of range",
        "入力の時刻が戻った" => "Input time moved backwards",
        "出力の大きさが矩形と違う" => "Output size does not match rectangle",
        "効果のブラシは消しゴムにできない" => "Effect brushes cannot erase",
        "効果のブラシは画素ごとには塗れない（apply_dab で読み元を凍結する）" => {
            "Effect brushes require dab painting"
        }
        "半径（0〜200）" => "Radius (0–200)",
        "同じグループの中のレイヤーだけをまとめられる" => {
            "Merge requires layers in the same group"
        }
        "同じ名前のチャンネルがある" => "Channel name already exists",
        "塗りつぶしレイヤーだけが値を持つ" => "Values require a fill layer",
        "塗りつぶしレイヤーではありません" => "Not a fill layer",
        "塗りつぶしレイヤーには描けない" => "Cannot paint a fill layer",
        "塗りつぶしはラスターレイヤーだけ" => "Fill requires a raster layer",
        "多角形の点が多すぎる（100000 まで）" => {
            "Too many polygon points (maximum 100000)"
        }
        "大きさ" => "Size",
        "対称の中心（±1e7）" => "Symmetry center (±1e7)",
        "レイヤーにマスクが無い" => "Layer has no mask",
        "レイヤーはもうマスクを持っている" => "Layer already has a mask",
        "レイヤーはグループの下に並ぶ" => "Layers must follow their group",
        "手ぶれ補正・入り抜き（0〜10000）" => "Stabilizer and taper (0–10000)",
        "指先の強さ（0〜1）" => "Smudge strength (0–1)",
        "指先・クローンは対称と組めない（写しごとに読み元と動きが要る）" => {
            "Smudge and clone do not support symmetry"
        }
        "放射状の写しの数（2〜16）" => "Radial symmetry count (2–16)",
        "有効なチャンネルに使えない調整" => {
            "Adjustment not supported by enabled channels"
        }
        "標準のチャンネルは変えられない" => "Cannot change a built-in channel",
        "標準のチャンネルは消せない" => "Cannot remove a built-in channel",
        "無いグループに入っている" => "Parent group not found",
        "無効のチャンネルには塗れない" => "Cannot fill a disabled channel",
        "無効のチャンネルには描けない" => "Cannot paint a disabled channel",
        "キャンバスの外の余白は 0 でなければならない" => {
            "Padding outside canvas must be zero"
        }
        "画素がキャンバスの外" => "Pixel outside canvas",
        "矩形がキャンバスの外" => "Rectangle outside canvas",
        "種がキャンバスの外" => "Seed outside canvas",
        "筆先の並び（1〜256 枚）" => "Brush tip sequence (1–256)",
        "筆先の大きさ（1〜2048）" => "Brush tip size (1–2048)",
        "筆先の覆いの長さが幅 × 高さでない" => {
            "Brush tip coverage size must be width × height"
        }
        "範囲の大きさがキャンバスと違う" => "Region size does not match canvas",
        "色のゆらぎ（0〜1）" => "Color dynamics (0–1)",
        "色相/彩度のレイヤーが有効" => "Hue/Saturation layer enabled",
        "色相/彩度は色のチャンネルだけ" => "Hue/Saturation requires a color channel",
        "色相・彩度・明度" => "Hue, saturation and value",
        "この種類は 8 つの値では組み立てられない（種類ごとの組み立てを使う）" => {
            "This adjustment kind cannot be built from the eight values"
        }
        "調整の種類と値が合わない" => "The adjustment kind and its values do not match",
        "カラーバランス（−100〜100）" => "Color balance (−100–100)",
        "明るさ（−150〜150）・コントラスト（−50〜100）" => {
            "Brightness (−150–150) and contrast (−50–100)"
        }
        "しきい値（1〜255）" => "Threshold (1–255)",
        "階調（2〜255）" => "Posterize levels (2–255)",
        "グラデーションマップとカラーバランスは色のチャンネルだけに適用できます" => {
            "Gradient Map and Color Balance apply only to color channels"
        }
        "接空間法線には再正規化するぼかしだけを適用できます" => {
            "Only the renormalizing blur applies to tangent-space normals"
        }
        "値の切り出し・値の幅・太らせる・細らせる・輪郭の検出はスカラーとマスクだけに適用できます" => {
            "Histogram Scan, Histogram Range, Dilate / Erode and Edge Detect apply only to scalar channels and masks"
        }
        "グローは色のチャンネルだけに適用できます" => {
            "Glow applies only to color channels"
        }
        "親の数がレイヤーの数と違う" => "Parent count does not match layer count",
        "調整レイヤーだけが調整の設定を持つ" => {
            "Adjustment settings require an adjustment layer"
        }
        "調整レイヤーには描けない" => "Cannot paint an adjustment layer",
        "速さの上限（0 より大きい）" => "Maximum speed (greater than zero)",
        "選択範囲のタイルがキャンバスの外" => "Selection tile outside canvas",
        "選択範囲のタイルが空" => "Empty selection tile",
        "選択範囲のタイルが重なっている" => "Overlapping selection tiles",
        "選択範囲のタイルの余白が 0 でない" => "Selection tile padding must be zero",
        "選択範囲のタイルの大きさ" => "Selection tile size",
        "選択範囲のタイルの長さ" => "Selection tile length",
        "選択範囲の大きさ" => "Selection size",
        "選択範囲の大きさがキャンバスと違う" => "Selection size does not match canvas",
        "選択範囲の名前が空" => "The selection name is empty",
        "選択範囲の名前が長すぎる" => "The selection name is too long",
        "選択範囲の名前に制御文字がある" => {
            "The selection name has control characters"
        }
        "選択範囲の名前が整っていない" => {
            "The selection name has leading or trailing spaces"
        }
        "選択範囲が無い" => "There is no selection",
        "残せる選択範囲の数の上限" => "Too many saved selections",
        "残した選択範囲の番号" => "No such saved selection",
        "同じ名前の選択範囲がある" => "A saved selection with that name exists",
        "残した選択範囲を戻せるのは読み込みの直後だけ" => {
            "Saved selections can only be restored right after loading"
        }
        "選択範囲の大きさが違う" => "Selection size mismatch",
        "選択範囲を戻せるのは読み込みの直後だけ" => {
            "Selection restore requires a freshly loaded project"
        }
        "面が無い" => "Surface not found",
        "UV 比較の解像度" => "UV comparison resolution",
        "グラデーション" => "Gradient",
        "三角形の座標" => "Triangle coordinates",
        "三角形番号" => "Triangle index",
        "空のマテリアル" => "Empty material",
        "重複したチャンネル" => "Duplicate channel",
        "ID 色の復元は読み込み直後だけ" => {
            "ID colors can only be restored right after loading"
        }
        "グラデーション両端のチャンネル" => "Channels at both ends of the gradient",
        "保存するレイヤーがありません" => "No layers to save",
        "保存するマスクがありません" => "No mask to save",
        "空のスマート素材" => "Empty smart material",
        "スマートマスクの断片が不正" => "Invalid smart mask fragment",
        "スマートマスクはレイヤーに置けません" => {
            "A smart mask cannot be placed on a layer"
        }
        "配置先がグループではありません" => "The target is not a group",
        "レイヤーは2048個までです" => "Maximum 2048 layers",
        "スマートマテリアルはマスクに置けません" => {
            "A smart material cannot be placed on a mask"
        }
        "ユーザーチャンネルの対応が一致しません" => {
            "User channel mapping does not match"
        }
        "ID マップが必要" => "An ID map is required",
        "ID マップの大きさ" => "ID map size",
        "ID の色" => "ID color",
        "スマート素材の名前" => "Smart material name",
        "変形は有限値" => "Transform must be finite",
        "変形が潰れる、または範囲外" => "Transform is degenerate or out of range",
        "動かすラスターレイヤーが無い" => "No raster layer to move",
        "画像の辺は 1〜8192" => "Image side (1–8192)",
        "移動先のグループ" => "Destination group",
        "未知のロック" => "Unknown lock",
        "結合は 2 レイヤー以上" => "Merging requires at least two layers",
        "1 つのスタックの段は 32 まで" => "Maximum 32 effects per stack",
        "Anchor のジェネレーターではない" => "Not an anchor generator",
        "Anchor の ID が空" => "Anchor ID is empty",
        "Anchor の ID が空か重なっている" => "Anchor ID is empty or duplicated",
        "Anchor の ID が重なっている" => "Anchor ID is duplicated",
        "Anchor の名前が空" => "Anchor name is empty",
        "Anchor の名前が長すぎる" => "Anchor name is too long",
        "Anchor は Normal を読めない" => "An anchor cannot read Normal",
        "ジェネレーターではない" => "Not a generator",
        "ジェネレーターの設定" => "Generator settings",
        "ジェネレーターは 1 画素に 1 つの値を作るので、接空間の法線には置けない" => {
            "A generator makes one value per pixel and cannot be placed on a tangent-space normal"
        }
        "ジェネレーターはジェネレーターの設定で置く" => {
            "A generator is placed with generator settings"
        }
        "グラデーションの設定" => "Gradient settings",
        "グループの合成へのフィルターは無い" => {
            "Groups have no filters on their composite"
        }
        "このマスクにはもう Anchor がある" => "This mask already has an anchor",
        "このレイヤーにはもう Anchor がある" => "This layer already has an anchor",
        "スタックの到達半径の合計が 512 画素を超える" => {
            "Total reach of the stack exceeds 512 pixels"
        }
        "その Anchor が無い" => "Anchor not found",
        "そのチャンネルにグラデーションが無い" => "The channel has no gradient",
        "そのレイヤーにそのフィルターが無い" => "The layer has no such filter",
        "パスが参照する三角形が無い" => "The path refers to a missing triangle",
        "パスで描かれたチャンネルは無効にできない" => {
            "Cannot disable a channel drawn by a path"
        }
        "パスで描かれたレイヤーには手で描けない" => {
            "Cannot paint by hand on a path layer"
        }
        "パスのチャンネルがレイヤーで有効でない" => {
            "The path channel is not enabled on the layer"
        }
        "パスのチャンネルの面がレイヤーに無い" => {
            "The layer has no surface for the path channel"
        }
        "フィルターの ID" => "Filter ID",
        "フィルターの ID が空" => "Filter ID is empty",
        "フィルターの ID が空か重なっている" => "Filter ID is empty or duplicated",
        "フィルターの ID が重なっている" => "Filter ID is duplicated",
        "フィルターのチャンネル" => "Filter channels",
        "フィルターのチャンネルが選ばれていない" => "No filter channel selected",
        "フィルターのチャンネルが重なっている" => {
            "Filter channels are duplicated"
        }
        "フィルターの設定" => "Filter settings",
        "ブラシの間隔に対してパスが長すぎる" => {
            "The path is too long for the brush spacing"
        }
        "プロジェクトにその画像が無い" => "The project has no such image",
        "マスクが無い" => "No mask",
        "マスクのフィルターのチャンネル" => "Mask filter channels",
        "マスクのフィルターはチャンネルを持たない" => {
            "Mask filters have no channels"
        }
        "マスクのフィルターはチャンネルを持たない（マスクは全チャンネルで共有）" => {
            "Mask filters have no channels (a mask is shared by all channels)"
        }
        "マスクの無いレイヤーのマスクには Anchor を置けない" => {
            "Cannot place a mask anchor on a layer without a mask"
        }
        "マップの境界箱" => "Map bounding box",
        "マップの大きさと長さが合わない" => "Map size and length do not match",
        "マップの幅" => "Map width",
        "マップの条件の鍵" => "Map condition key",
        "マップの高さ" => "Map height",
        "メッシュマップの種類" => "Mesh map kind",
        "モデルの位置・回転" => "Model position and rotation",
        "モデルの指紋がパスを作ったときと違う" => {
            "The model fingerprint differs from when the path was made"
        }
        "予算が今のフィルターの要る量より小さい" => {
            "Budget is smaller than the current filters need"
        }
        "効果（フィルター・画像・グラデーション）は標準のチャンネルだけに置ける" => {
            "Effects (filters, images, gradients) can only be placed on standard channels"
        }
        "塗りつぶしのグラデーションのチャンネル" => "Fill gradient channel",
        "ベイクの優先の復元は読み込み直後だけ" => {
            "Bake priority can only be restored right after loading"
        }
        "画像の無いチャンネル" => "Channel without an image",
        "異方性を切る塗りつぶしの画像のチャンネル" => {
            "Fill image channel for turning anisotropic filtering off"
        }
        "塗りつぶしの点のグラデーションのチャンネル" => "Fill point gradient channel",
        "そのチャンネルに点のグラデーションが無い" => "The channel has no point gradient",
        "点のグラデーションは色かスカラーで、法線ではない" => {
            "A point gradient is for colour or scalar channels, not the normal"
        }
        "塗りつぶしのグラデーションはランプ付きの形のグラデーション" => {
            "A fill gradient is a shape gradient with a ramp"
        }
        "塗りつぶしのグラデーションは置き換え" => "A fill gradient replaces",
        "塗りつぶしの入力" => "Fill input",
        "塗りつぶしレイヤーだけが持つ" => "Only fill layers have this",
        "塗りつぶしの画像のチャンネルか ID" => "Fill image channel or ID",
        "レイヤーのパスはチャンネルを変えない" => {
            "A layer path does not change its channel"
        }
        "レイヤーのパスは種類（モデルの上かキャンバスの上か）を変えない" => {
            "A layer path does not change its kind (model or canvas)"
        }
        "名前を替えるパスがレイヤーに無い" => "The layer has no such path to rename",
        "形のグラデーションは色かスカラーで、法線ではない" => {
            "A shape gradient is a color or scalar, not a normal"
        }
        "投影の値" => "Projection values",
        "描いた面のチャンネルが重なっている" => {
            "Painted surface channels are duplicated"
        }
        "描いた面は、パスのチャンネルごとに、キャンバスと同じ大きさで 1 つ" => {
            "One painted surface per path channel, at the canvas size"
        }
        "画像の ID が空" => "Image ID is empty",
        "画像の大きさ" => "Image size",
        "画像の大きさと画素の長さ" => "Image size and pixel length do not match",
        "自分のレイヤーの Anchor を読むジェネレーター（値が自分に戻る）" => {
            "A generator reading an anchor on its own layer (the value would feed back)"
        }
        "調整レイヤーには画素が無い" => "Adjustment layers have no pixels",
        "面のダブを拒否した" => "The surface dab was refused",
        "見た目の設定の復元は読み込み直後だけ" => {
            "Look settings can only be restored right after loading"
        }
        "見た目のシェーダーの名前" => "Look shader name",
        "見た目のシェーダーの身元" => "Look shader identity",
        "見た目のプロパティの数" => "Number of look properties",
        "見た目のテクスチャの数" => "Number of look textures",
        "見た目のキーワードの数" => "Number of look keywords",
        "受けた見た目の出どころ" => "Source of the received look",
        "受けた見た目の絵の数" => "Number of received look textures",
        "受けた見た目の絵" => "Received look texture",
        "受けた見た目のスロットの名前" => "Received look slot name",
        "見た目のプロパティの名前" => "Look property name",
        "見た目のプロパティの値" => "Look property value",
        "見た目のテクスチャの名前" => "Look texture name",
        "見た目の詰め合わせの成分" => "Look packed texture component",
        "見た目の画像の ID" => "Look image ID",
        "見た目のキーワード" => "Look keyword",
        "テキストを塗る面は空であること" => "The text surface must be empty",
        "フォントのファイルを読めない" => "Cannot read the font file",
        "フォントに輪郭が無い" => "The font has no outlines",
        "同梱のフォントの名前" => "Bundled font name",
        "フォントのファイルの道" => "Font file path",
        "フォントの名前" => "Font name",
        "文の長さ" => "Text length",
        "文の制御文字" => "Control characters in the text",
        "文字のサイズ" => "Text size",
        "行間" => "Line spacing",
        "字間" => "Tracking",
        "文字の位置" => "Text position",
        "文字の回転" => "Text rotation",
        "折り返しの幅" => "Wrap width",
        "テキストレイヤーではない" => "Not a text layer",
        "テキストレイヤーには手で描けない" => "Cannot paint by hand on a text layer",
        "テキストレイヤーにはパスを付けられない" => "A text layer cannot have a path",
        "テキストレイヤーの Color は無効にできない" => "Cannot disable Color on a text layer",
        "テキストの値を付けられるのはパスとテキストの無いラスターのレイヤーだけ" => {
            "Text needs a raster layer without a path or text"
        }
        "テキストレイヤーの Color が無い" => "The text layer has no Color",
        "まとめるテキストの段が無い" => "No text step to merge into",
        "線対称の線の本数（偶数）" => "Line symmetry needs an even number of lines",
        "対称の角度（±360 度）" => "The symmetry angle must be within ±360 degrees",
        "レイヤーに付けられる定規は 64 個まで" => "A layer can have at most 64 rulers",
        "定規の ID が重なっている" => "Ruler IDs overlap",
        "定規の ID がほかのレイヤーの定規と重なっている" => {
            "A ruler ID overlaps a ruler on another layer"
        }
        "移し先が同じレイヤー" => "The target layer is the same layer",
        "移す定規がそのレイヤーに無い" => "The ruler to move is not on that layer",
        "その定規がそのレイヤーに無い" => "That ruler is not on that layer",
        "直線定規はスナップする特殊定規にならない" => {
            "A straight ruler cannot be the snapping special ruler"
        }
        "定規の ID（0 は使わない）" => "A ruler ID cannot be 0",
        "定規の点（有限・±1e7）" => "Ruler points must be finite and within ±1e7",
        "定規の 2 点が近すぎる（0.01 画素以上）" => {
            "The two ruler points are too close (at least 0.01 px apart)"
        }
        "見えない面にも写すのは 3D の対称だけ" => {
            "Mirroring onto hidden faces is for 3D symmetry only"
        }
        "定規の点（有限・±1e6）" => "Ruler points must be finite and within ±1e6",
        "定規の 2 点が近すぎる（1e-6）" => "The two ruler points are too close (at least 1e-6 apart)",
        "定規の向きが 0" => "The ruler direction is zero",
        "対称定規の最初の線が回転の軸と平行" => {
            "The first line of a symmetry ruler is parallel to its rotation axis"
        }
        "2 点はパースだけ" => "Two points are for a perspective ruler only",
        "対称定規の線の本数（2〜16）" => "A symmetry ruler has 2 to 16 lines",
        "線の本数と線対称は対称定規だけ" => "Line count and line symmetry are for symmetry rulers only",
        _ => return None,
    })
}

impl Lang {
    pub fn surface_error(self, error: &yolu_core::geometry::SurfaceStrokeError) -> String {
        crate::crash::problem(self.surface_error_text(error))
    }
    fn surface_error_text(self, error: &yolu_core::geometry::SurfaceStrokeError) -> String {
        use yolu_core::geometry::SurfaceStrokeError;
        match error {
            SurfaceStrokeError::Core(e) => self.core_error(e),
            SurfaceStrokeError::Dab(e) => self.dab_refusal(*e).into(),
            SurfaceStrokeError::TooManyDabs => self
                .pick(
                    "ストロークが長すぎるため、取り消しました",
                    "Cancelled: the stroke is too long",
                )
                .into(),
            SurfaceStrokeError::Sampling(e) => self.sampling_error(*e).into(),
            SurfaceStrokeError::EffectWithSymmetry => self
                .pick(
                    "指先・クローンでは対称を使えません",
                    "Smudge and clone do not work with symmetry",
                )
                .into(),
            SurfaceStrokeError::CloneSource => self
                .pick(
                    "クローンの元が今のモデルの面ではありません",
                    "The clone source is not on this model",
                )
                .into(),
        }
    }
    /// 指先・クローンの読み元を決められなかった理由。どれもストロークごと取り消すので、「取り消し」を言う。内部の言葉（スナップショット・予算・図）は使わない。
    pub fn sampling_error(self, error: yolu_core::geometry::SamplingError) -> &'static str {
        crate::crash::problem(self.sampling_error_text(error))
    }
    fn sampling_error_text(self, error: yolu_core::geometry::SamplingError) -> &'static str {
        use yolu_core::geometry::SamplingError::*;
        match error {
            SnapshotChanged => self.pick(
                "モデルが変わったため、取り消しました",
                "Cancelled: the model changed",
            ),
            BindingMismatch => self.pick(
                "面がモデルと合わないため、取り消しました",
                "Cancelled: the surface does not match the model",
            ),
            InvalidArguments => self.pick(
                "ブラシの大きさか位置が範囲外のため、取り消しました",
                "Cancelled: brush size or position out of range",
            ),
            ChartBudget => self.pick(
                "読み取る面が多すぎるため、取り消しました",
                "Cancelled: too many faces to read from",
            ),
            LookupBudget => self.pick(
                "読み取る範囲が広すぎるため、取り消しました",
                "Cancelled: the area to read from is too large",
            ),
            Unreachable => self.pick(
                "読み取り先に届かない画素があるため、取り消しました",
                "Cancelled: some pixels cannot reach the source",
            ),
        }
    }
    /// 対称の写しが塗られなかった理由（短い状態）。
    pub fn mirror_note(self, outcome: yolu_core::geometry::MirrorOutcome) -> &'static str {
        use yolu_core::geometry::MirrorOutcome::*;
        match outcome {
            NoSurface => self.pick(
                "近くに面が無い対称の写しは飛ばしました",
                "Symmetry copies with no surface nearby were skipped",
            ),
            OtherSlot => self.pick(
                "別のテクスチャセットにある対称の写しは飛ばしました",
                "Symmetry copies on another texture set were skipped",
            ),
            Hidden => self.pick(
                "見えない対称の写しは飛ばしました",
                "Symmetry copies that cannot be seen were skipped",
            ),
            UvMismatch => self.pick(
                "UV が潰れているか大きさが合わない対称の写しは飛ばしました",
                "Symmetry copies with flat or mismatched UVs were skipped",
            ),
            Painted | OnPlane => "",
        }
    }
    /// 指先が、つながらない面で拾い直したときの状態。
    pub fn smudge_lost(self) -> &'static str {
        self.pick(
            "指先はつながらない面で拾い直しました",
            "Smudge picked up again on a disconnected surface",
        )
    }
    pub fn dab_refusal(self, error: yolu_core::geometry::DabRefusal) -> &'static str {
        crate::crash::problem(self.dab_refusal_text(error))
    }
    /// パスの評価が 3D の打ち（ブラシの 1 回分）を塗れなかった理由。パスの評価は、どの理由でも評価ごと失敗して何も塗られない
    /// （`yolu_core::paths` は断りがあれば種類を問わず `Error::Dab` を返す）ので、飛ばして続ける 3 つに「一部」とは言わない。
    pub fn path_dab_refusal(self, error: yolu_core::geometry::DabRefusal) -> &'static str {
        crate::crash::problem(self.path_dab_refusal_text(error))
    }
    fn path_dab_refusal_text(self, error: yolu_core::geometry::DabRefusal) -> &'static str {
        use yolu_core::geometry::DabRefusal::*;
        match error {
            SnapshotChanged => self.pick(
                "モデルが変わったため、塗れませんでした",
                "Not painted because the model changed",
            ),
            InvalidArguments => self.pick(
                "ブラシの大きさが範囲外のため、塗れませんでした",
                "Not painted: brush size out of range",
            ),
            BindingMismatch => self.pick(
                "面がモデルと合わないため、塗れませんでした",
                "Not painted: the surface does not match the model",
            ),
            TriangleBudget => self.pick(
                "ブラシがまたがる面が多すぎるため、塗れませんでした",
                "Not painted: the brush covers too many faces",
            ),
            PixelBudget => self.pick(
                "ブラシの範囲が広すぎるため、塗れませんでした",
                "Not painted: the brush area is too large",
            ),
            VisibilityBudget => self.pick(
                "見える面の判定が多すぎるため、塗れませんでした",
                "Not painted: too many points to check for visibility",
            ),
            BvhBudget => self.pick(
                "重なった面が多すぎるため、塗れませんでした",
                "Not painted: too many overlapping faces under the brush",
            ),
            MemoryBudget => self.pick(
                "1 回の操作のメモリが足りないため、塗れませんでした",
                "Not painted: not enough memory for one operation",
            ),
        }
    }
    /// 3D のブラシのストロークで、打ち（ブラシの 1 回分。対称の写し・デュアルブラシの 2 つ目の打ちも）を塗れなかった理由。2D の
    /// ストロークと同じく、どれもストロークごと取り消す（塗り残しを作らない）ので、「取り消し」を言う。1 回の操作のメモリが足りない
    /// ときは、2D の予算を超えたときと同じ文。パスの評価の文は `path_dab_refusal`。内部の言葉（BVH・予算・レイ・スナップショット）は
    /// 使わず、原因を使う人の言葉で短く言う。
    fn dab_refusal_text(self, error: yolu_core::geometry::DabRefusal) -> &'static str {
        use yolu_core::geometry::DabRefusal::*;
        match error {
            SnapshotChanged => self.pick(
                "モデルが変わったため、取り消しました",
                "Cancelled: the model changed",
            ),
            InvalidArguments => self.pick(
                "ブラシの大きさかカメラが範囲外のため、取り消しました",
                "Cancelled: brush size or camera out of range",
            ),
            BindingMismatch => self.pick(
                "面がモデルと合わないため、取り消しました",
                "Cancelled: the surface does not match the model",
            ),
            TriangleBudget => self.pick(
                "ブラシがまたがる面が多すぎるため、取り消しました",
                "Cancelled: the brush covers too many faces",
            ),
            PixelBudget => self.pick(
                "ブラシの範囲が広すぎるため、取り消しました",
                "Cancelled: the brush area is too large",
            ),
            VisibilityBudget => self.pick(
                "見える面の判定が多すぎるため、取り消しました",
                "Cancelled: too many points to check for visibility",
            ),
            BvhBudget => self.pick(
                "重なった面が多すぎるため、取り消しました",
                "Cancelled: too many overlapping faces under the brush",
            ),
            // 2D の `CoreError::StrokeBudgetExceeded` と同じ文
            MemoryBudget => self.pick(
                "1 回の操作のメモリの予算を超えます（取り消しました）",
                "Over the memory budget of one operation (cancelled)",
            ),
        }
    }
}

impl Lang {
    /// 3D ビュー・ポーズの失敗の文。
    pub fn view_error(self, error: &ViewError) -> String {
        crate::crash::problem(self.view_error_text(error))
    }
    fn view_error_text(self, error: &ViewError) -> String {
        // モデルの読み込みの誤りは、日本語も「何が（なぜ）」の 1 文に組み直した形で
        if let ViewError::Model(e) = error {
            return self.model_error_text(e);
        }
        if self == Self::Ja {
            return error.to_string();
        }
        match error {
            ViewError::Stroking => super::refusals::during_stroke(self).into(),
            ViewError::NoPoseModel => "No model to pose".into(),
            ViewError::NoPoseEdit => "Pose edit not started".into(),
            ViewError::NoLinkModel => "Pose received before model".into(),
            ViewError::NoPoseBase => "No model to apply the pose to".into(),
            ViewError::BadMeshIndex => "Mesh index exceeds the vertex count".into(),
            ViewError::NoTriangles => "No triangles".into(),
            ViewError::Cancelled => "Cancelled".into(),
            ViewError::LoadStopped => "Loading stopped".into(),
            ViewError::PoseGeneration {
                pose,
                model: Some(model),
            } => {
                format!("Pose generation {pose} differs from model generation {model}")
            }
            ViewError::PoseGeneration { pose, model: None } => {
                format!("No model for pose generation {pose}")
            }
            ViewError::PoseMesh => "Pose mesh index out of range".into(),
            ViewError::PoseVertices => "Pose vertex count differs from the mesh".into(),
            ViewError::Rig(e) => self.rig_error(e),
            ViewError::Geometry(e) => self.geometry_error(*e).into(),
            ViewError::Model(e) => self.model_error(e),
        }
    }

    pub fn rig_error(self, error: &RigError) -> String {
        crate::crash::problem(self.rig_error_text(error))
    }
    fn rig_error_text(self, error: &RigError) -> String {
        if self == Self::Ja {
            return error.to_string();
        }
        match error {
            RigError::TooLarge { what, value, limit } => {
                format!("Too many {} ({value}, maximum {limit})", rig_what(what))
            }
            RigError::BadParent { bone } => format!("Invalid parent of bone {bone}"),
            RigError::BadMesh { mesh } => format!("Invalid indices or vertex count in mesh {mesh}"),
            RigError::BadSkin { mesh } => format!("Invalid skin in mesh {mesh}"),
            RigError::BadBlendShape { mesh, shape } => {
                format!("Invalid BlendShape {shape} in mesh {mesh}")
            }
            RigError::NonFinite { what } => format!("Non-finite value in {}", rig_what(what)),
            RigError::PoseMismatch => "Pose does not match the model".into(),
        }
    }

    pub fn geometry_error(self, error: GeometryError) -> &'static str {
        crate::crash::problem(self.geometry_error_text(error))
    }
    fn geometry_error_text(self, error: GeometryError) -> &'static str {
        match error {
            GeometryError::NonFinite => self.pick(
                "メッシュの位置か UV に有限でない値があります",
                "Non-finite position or UV in the mesh",
            ),
            GeometryError::BoundsOverflow => self.pick(
                "メッシュの大きさが扱える範囲を超えています",
                "Mesh bounds exceed the supported range",
            ),
            GeometryError::InvalidTolerance => self.pick(
                "溶接の許しは正の値でなければなりません",
                "Weld tolerance must be positive",
            ),
            GeometryError::TooManyTriangles => {
                self.pick("三角形が多すぎます", "Too many triangles")
            }
            GeometryError::Canceled => self.pick("取り消しました", "Cancelled"),
            GeometryError::Mismatch => self.pick(
                "三角形の並びが元のモデルと違います",
                "Triangle order differs from the original model",
            ),
        }
    }

    pub fn model_error(self, error: &ModelError) -> String {
        let text = self.model_error_text(error);
        // ufbx の文には制作物の中の名前が入りうるので、失敗の文として覚えない
        if matches!(error, ModelError::Parse(_)) {
            text
        } else {
            crate::crash::problem(text)
        }
    }
    fn model_error_text(self, error: &ModelError) -> String {
        match error {
            // 「〜できません: 理由」のコロンでつないだ形にしない
            ModelError::Io(e) => {
                self.with_reason(self.pick("ファイルを読めません", "Cannot read the file"), e)
            }
            ModelError::Parse(e) => {
                self.with_reason(self.pick("FBX として読めません", "Invalid FBX"), e)
            }
            _ if self == Self::Ja => error.to_string(),
            ModelError::FileTooLarge { bytes, limit } => format!(
                "File too large ({:.1} MiB, maximum {:.0} MiB)",
                *bytes as f64 / 1048576.0,
                *limit as f64 / 1048576.0
            ),
            ModelError::NoMesh => "No triangle mesh".into(),
            ModelError::Cancelled => "Cancelled".into(),
            ModelError::Rig(e) => self.rig_error(e),
            ModelError::Changed => "The file has changed since it was loaded".into(),
            ModelError::NoTake => "The take is not in the file".into(),
        }
    }
}

fn rig_what(what: &str) -> &str {
    match what {
        "ボーン" => "bones",
        "メッシュ" => "meshes",
        "頂点" => "vertices",
        "三角形" => "triangles",
        "ウェイト" => "weights",
        "BlendShape" => "BlendShapes",
        "BlendShape の差分" => "BlendShape offsets",
        "1 つの頂点のウェイト" => "weights per vertex",
        "ボーンの変換" => "bone transforms",
        "メッシュの位置・法線・UV" => "mesh positions, normals and UVs",
        "ポーズ" => "the pose",
        "スキンの行列・ウェイト（負のウェイトを含む）" => {
            "skin matrices and weights"
        }
        _ if what.is_ascii() => what,
        _ => "values",
    }
}

/// yolu-io の断りの短い理由（読み込み・棚の予算・保存）。
pub fn shelf_io_error(lang: Lang, e: &yolu_io::Error) -> String {
    if let Some(text) = library_known_error(lang, e) {
        return text;
    }
    let m = e.to_string();
    if m.contains(REFUSAL_RESOURCE_COUNT) {
        crate::lang::refusals::shelf_full(lang).into()
    } else if m.contains(REFUSAL_IMAGES) {
        Block::Images.reason(lang)
    } else if m.contains(REFUSAL_GENERATORS) {
        Block::Generators.reason(lang)
    } else if m.contains(REFUSAL_MEMORY_BUDGET) {
        lang.pick("アセットの予算を超えます", "Over the asset budget")
            .into()
    } else if m.contains(REFUSAL_ARCHIVE_BUDGET) {
        lang.pick(
            "ファイルの大きさの上限を超えます",
            "Over the file size limit",
        )
        .into()
    } else if m.contains(REFUSAL_USER_CHANNELS) {
        lang.pick("ユーザーチャンネルを使っています", "It uses user channels")
            .into()
    } else if m.contains(REFUSAL_RUST_GENERATORS) {
        lang.pick("ノイズ・グランジを使っています", "It uses Noise and Grunge")
            .into()
    } else if m.contains(REFUSAL_RUST_ADJUSTMENTS) {
        lang.pick("色調補正を使っています", "It uses colour adjustments")
            .into()
    } else if m.contains(REFUSAL_NEW_FILTERS) {
        lang.pick(
            "0.5.0 で追加したフィルター・ジェネレーターを使っています",
            "It uses filters or generators added in 0.5.0",
        )
        .into()
    } else if m.contains(REFUSAL_PATH_LISTS) {
        lang.pick(
            "パスの一覧か新しいパスの設定を使っています",
            "It uses a path list or new path settings",
        )
        .into()
    } else if m.contains(REFUSAL_POINT_GRADIENTS) {
        lang.pick(
            "点のグラデーションか、異方性フィルターを切った画像を使っています",
            "It uses a point gradient or an image with anisotropic filtering off",
        )
        .into()
    } else {
        lang.io_error(e)
    }
}

/// ライブラリのフォルダ・画像の断りの短い文（そうでない断りは None）。
pub fn library_known_error(lang: Lang, e: &yolu_io::Error) -> Option<String> {
    let m = e.to_string();
    let has = |text: &str| m.contains(text);
    let short = if has(files::REFUSAL_ROOT_LINK) {
        lang.pick(
            "ライブラリの場所がリンクです",
            "The library folder is a link",
        )
    } else if has(files::REFUSAL_ROOT_NOT_FOLDER) {
        lang.pick(
            "ライブラリの場所がフォルダではありません",
            "The library location is not a folder",
        )
    } else if has(files::REFUSAL_LINK) {
        lang.pick("リンクはたどりません", "Links are not followed")
    } else if has(files::REFUSAL_PATH) {
        lang.pick(
            "名前に使えない文字があります",
            "The name has disallowed characters",
        )
    } else if has(files::REFUSAL_NOT_FILE) {
        lang.pick("ファイルではありません", "Not a file")
    } else if has(files::REFUSAL_TOO_LARGE) {
        lang.pick("大きすぎます", "Too large")
    } else if has(files::REFUSAL_EMPTY) {
        lang.pick("空のファイルです", "Empty file")
    } else if has(files::REFUSAL_NO_NAME) {
        lang.pick("名前を決められません", "Cannot choose a name")
    } else if has(REFUSAL_UNSUPPORTED) {
        lang.pick("PNG と .ylsmart だけです", "PNG and .ylsmart only")
    } else if has(REFUSAL_PNG_SIZE) {
        lang.pick(REFUSAL_PNG_SIZE, "Image sides must be 1 to 8192")
    } else if has(REFUSAL_PNG) {
        lang.pick(REFUSAL_PNG, "Not a readable PNG")
    } else {
        return None;
    };
    Some(short.to_owned())
}

/// 断りの理由の短い文（ライブラリのフォルダの断り・画像の断りは言語ごとに、ほかは棚の言い方）。
pub fn library_io_error(lang: Lang, e: &yolu_io::Error) -> String {
    library_known_error(lang, e).unwrap_or_else(|| shelf_io_error(lang, e))
}

/// 取り込めない理由の文（言語ごと）。
pub fn brush_import_error(lang: Lang, error: &BrushImportError) -> String {
    match error {
        BrushImportError::Io(e) => lang.file_error(e),
        other => lang.pick(other.to_string(), other.english()),
    }
}

/// パスの評価の失敗の文。
pub fn path_error(lang: Lang, error: &paths::Error) -> String {
    use paths::Error;
    match error {
        // 点の検査の理由は、そのまま短い理由（「値が範囲外です（…）」で包まない）
        Error::Invalid(why) => crate::crash::problem(lang.pick(*why, core_reason(why))).into(),
        Error::ModelMismatch => lang
            .pick(
                "モデルが、パスを描いたときと違います",
                "The model differs from the one the path was drawn on",
            )
            .into(),
        Error::MissingTriangle => lang
            .pick(
                "パスが指す三角形がモデルにありません",
                "The path refers to a triangle the model does not have",
            )
            .into(),
        Error::TooManySamples => lang
            .pick(
                "ブラシの間隔に対してパスが長すぎます",
                "The path is too long for the brush spacing",
            )
            .into(),
        Error::Canceled => lang.pick("取り消しました", "Cancelled").into(),
        Error::Core(e) => lang.core_error(e),
        Error::Dab(d) => lang.path_dab_refusal(*d).into(),
        Error::MissingImage => lang
            .pick(
                "リボンの画像がアセットにありません",
                "The ribbon image is not in the assets",
            )
            .into(),
        Error::FillIslands => lang
            .pick(
                "塗りのパスが 1 つの UV アイランドに収まっていません",
                "The fill path does not stay on one UV island",
            )
            .into(),
    }
}

/// 付け直せなかった理由の文（画面の言語で）。
pub fn rebind_error(lang: Lang, error: RebindError) -> String {
    match error {
        RebindError::OtherModel => lang
            .pick("別のモデルで描かれたパスです", "The path was drawn on another model")
            .into(),
        RebindError::NoMaterial => lang
            .pick("マテリアルが新しいモデルにありません", "Its material is not in the new model")
            .into(),
        RebindError::MissingTriangle { point } => lang.pick(
            format!("点 {} はモデルに無い三角形を指しています", point + 1),
            format!("Point {} refers to a triangle the model does not have", point + 1),
        ),
        RebindError::SearchBudget { point } => lang.pick(
            format!("点 {} を新しいメッシュで探すのに時間がかかりすぎました", point + 1),
            format!("Finding point {} on the new mesh took too long", point + 1),
        ),
        RebindError::NoSurface { point, tolerance } => lang.pick(
            format!("点 {} の近く（{tolerance:.4} 以内）に、同じマテリアルの面が新しいメッシュにありません", point + 1),
            format!("Point {} has no surface of its material within {tolerance:.4} on the new mesh", point + 1),
        ),
    }
}

/// 取り込めない理由（画面の言語。レイヤーの名前は利用者の名前なのでそのまま）。
pub fn psd_copy_refusal(lang: Lang, why: &CopyRefusal) -> String {
    match why {
        CopyRefusal::Malformed(text) => lang.pick(
            text.clone(),
            "Not a readable PSD (damaged or unsupported)".into(),
        ),
        CopyRefusal::LargeDocument => lang.pick(
            "PSB 形式は取り込めません".into(),
            "PSB files cannot be imported".into(),
        ),
        CopyRefusal::ColorFormat { depth, mode } => {
            let name = crate::psd_import::color_mode_name(lang, *mode);
            lang.pick(
                format!("RGB 8 bit 以外は取り込めません（{name}・{depth} bit）"),
                format!("Only RGB 8-bit PSDs can be imported ({name}, {depth}-bit)"),
            )
        }
        CopyRefusal::NoLayers => lang.pick(
            "レイヤーがありません（統合画像だけの PSD）".into(),
            "No layers (a flattened image only)".into(),
        ),
        CopyRefusal::TooManyLayers { count, limit } => lang.pick(
            format!("レイヤーが多すぎます（{count} 枚・上限 {limit} 枚）"),
            format!("Too many layers ({count}; limit {limit})"),
        ),
        CopyRefusal::CanvasTooLarge { width, height } => lang.pick(
            format!("キャンバスが大きすぎます（{width}×{height}）"),
            format!("Canvas too large ({width}×{height})"),
        ),
        CopyRefusal::LayerTooLarge { layer } => lang.pick(
            format!("「{layer}」の画素が大きすぎます"),
            format!("\"{layer}\" has too many pixels"),
        ),
        CopyRefusal::BudgetExceeded { layer } => lang.pick(
            format!("「{layer}」でレイヤーのメモリの予算を超えました"),
            format!("Layer memory budget exceeded at \"{layer}\""),
        ),
        CopyRefusal::LayerDataTooLarge { layer } => lang.pick(
            format!("「{layer}」の付加情報が大きすぎます"),
            format!("\"{layer}\" has too much extra data"),
        ),
        CopyRefusal::EdgeOverLimit {
            width,
            height,
            limit,
        } => lang.pick(
            format!("キャンバスが大きすぎます（{width}×{height}・上限 {limit}）"),
            format!("Canvas too large ({width}×{height}; limit {limit})"),
        ),
        CopyRefusal::LayerCountOverLimit { count, limit } => lang.pick(
            format!("レイヤーが多すぎます（{count} 枚・上限 {limit} 枚）"),
            format!("Too many layers ({count}; limit {limit})"),
        ),
        CopyRefusal::NestingTooDeep { limit } => lang.pick(
            format!("グループの入れ子が深すぎます（上限 {limit} 段）"),
            format!("Groups are nested too deeply (limit {limit})"),
        ),
    }
}

/// 理由のツールチップ（予算で断ったものは、設定で上げられること）。
pub fn psd_copy_refusal_tooltip(lang: Lang, why: &CopyRefusal) -> Option<String> {
    why.raised_by_budget().then(|| {
        lang.pick(
            "上限は設定の「レイヤーのメモリ」から決まります。上げると取り込めることがあります",
            "The limit follows Layer memory in Settings. Raising it may let this import",
        )
        .into()
    })
}

/// 更新の失敗の「何が」（`what` は "check" か "download"）。
fn update_head(lang: Lang, what: &str) -> &'static str {
    match what {
        "check" => lang.pick("更新を確かめられません", "Cannot check for updates"),
        _ => lang.pick("更新をダウンロードできません", "Cannot download the update"),
    }
}

/// 更新の仕事のスレッドを起こせなかった文。
pub(crate) fn update_start_failure(lang: Lang, what: &str, error: &std::io::Error) -> String {
    lang.with_reason(update_head(lang, what), lang.thread_error(error))
}

pub(crate) fn update_failure(lang: Lang, what: &'static str, failure: UpdateFailure) -> String {
    let head = update_head(lang, what);
    let reason = match failure {
        UpdateFailure::Network => lang.pick("通信できません", "connection failed"),
        UpdateFailure::Verify => lang.pick("検証を通りません", "verification failed"),
        UpdateFailure::NoAsset => lang.pick(
            "この環境向けの配布物がありません",
            "no download for this system",
        ),
        UpdateFailure::Disk => lang.pick("ファイルを保存できません", "cannot save the file"),
        UpdateFailure::Stopped => lang.pick("処理が止まりました", "the job stopped"),
        UpdateFailure::Canceled => {
            return lang
                .pick("ダウンロードを取り消しました。", "Download canceled.")
                .into()
        }
    };
    lang.with_reason(head, reason)
}

/// 効果 1 件の理由。画像は、棚の画像を復号できなかったならその理由（壊れている・予算を超える）を言い、そうでなく棚に無いなら
/// 設定の不備ではなく画像が無いことを言う。`failure` は、その画像を復号できなかった理由。
pub(crate) fn inactive_effect_reason(
    lang: Lang,
    effect: &InactiveEffect,
    failure: Option<&str>,
) -> String {
    if let (InactiveTarget::FillImage(_), InactiveReason::Rejected(why)) =
        (&effect.target, &effect.reason)
    {
        if let Some(why) = failure {
            return lang.pick(
                format!("画像を読めません（{why}）"),
                format!("Cannot read the image ({why})"),
            );
        }
        if why.contains("その画像が無い") {
            return lang
                .pick(
                    "プロジェクトに画像が無い",
                    "The image is not in the project",
                )
                .into();
        }
    }
    lang.inactive_reason(&effect.reason)
}

pub(crate) fn hide_preset_save_error(lang: Lang, e: &HideStoreError) -> String {
    lang.with_reason(
        lang.pick("隠し方を保存できません", "Cannot save the hide set"),
        e.describe(lang),
    )
}

pub(crate) fn pose_preset_save_error(lang: Lang, e: &PoseStoreError) -> String {
    lang.with_reason(
        lang.pick("ポーズを保存できません", "Cannot save the pose"),
        e.describe(lang),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    /// 処理を始められなかった理由は、OS の英語の文をそのまま出さず、どちらの言語でも言う（OS のエラー番号は手掛かりとして添える）。
    #[test]
    fn a_failed_start_is_told_in_both_languages_without_the_os_text() {
        use std::io::{Error, ErrorKind};
        // 資源が足りない誤りの番号は OS ごとに違う（Linux の EAGAIN 11・macOS の EAGAIN 35・Windows の WSAEWOULDBLOCK 10035。
        // Windows の 11 は ERROR_BAD_FORMAT で、資源の不足ではない）
        let code = if cfg!(windows) {
            10035
        } else if cfg!(target_os = "macos") {
            35
        } else {
            11
        };
        let busy = Error::from_raw_os_error(code);
        assert_eq!(busy.kind(), ErrorKind::WouldBlock, "{busy:?}");
        let (ja, en) = (Lang::Ja.thread_error(&busy), Lang::En.thread_error(&busy));
        assert_eq!(
            ja,
            format!("システムの資源が足りません（OS エラー {code}）")
        );
        assert_eq!(en, format!("Not enough system resources (OS error {code})"));
        let other = Error::from(ErrorKind::InvalidInput);
        assert_eq!(
            Lang::En.thread_error(&other),
            "The system refused to start the task"
        );
        assert!(!Lang::Ja.thread_error(&other).is_ascii());
        for text in [&ja, &en] {
            assert!(!text.contains("temporarily"), "{text}");
        }
        // 「何が」と合わせて 1 つの文
        assert_eq!(
            Lang::En.with_reason("Cannot export", &en),
            format!("Cannot export (Not enough system resources (OS error {code})).")
        );
    }

    /// 開いた .ylp の古い版の写し（保存が置き換えるために手放した）を読もうとした理由は、どちらの言語でも言う。
    #[test]
    fn a_released_open_file_is_told_in_both_languages() {
        let error = yolu_io::Error::SaveConflict(yolu_io::SOURCE_RELEASED.into());
        let (ja, en) = (Lang::Ja.io_error(&error), Lang::En.io_error(&error));
        assert!(ja.contains("置き換えられた"), "{ja}");
        assert!(en.is_ascii() && en.contains("replaced"), "{en}");
    }
    #[test]
    fn merge_and_lock_errors_use_the_selected_language() {
        use yolu_core::MergeRefusal::*;
        use yolu_core::{LayerId, LayerLocks, LayerMergeReport, MergeMethod};
        let mut errors: Vec<CoreError> = [
            NoLayerBelow,
            LayerBelowIsGroup,
            LayerBelowIsAdjustment,
            HiddenLayer,
            IsGroup,
            NotGroup,
            EmptyGroup,
            NothingVisible,
            DifferentGroups,
            TooManyRulers,
        ]
        .into_iter()
        .map(CoreError::MergeRefused)
        .collect();
        errors.push(CoreError::Cancelled);
        errors.push(CoreError::LayerLocked {
            layer: LayerId(1),
            holder: LayerId(2),
            lock: LayerLocks::ALL,
        });
        errors.push(CoreError::MergeAppearance(Box::new(LayerMergeReport {
            result_id: LayerId(3),
            method: MergeMethod::Layers,
            notes: 0,
            compared_pixels: 4,
            changed_pixels: 1,
            max_difference: 9,
            max_visible_difference: 7,
            changed_by_channel: Default::default(),
        })));
        let english: Vec<String> = errors.iter().map(|e| Lang::En.core_error(e)).collect();
        for (i, (error, en)) in errors.iter().zip(&english).enumerate() {
            // 取り消しとロックは、両方の言語で決めた文（ロックはどのロックか・親のグループか）。結合の断りは「何が（なぜ）」の
            // 1 文に組み直す（core の「結合できない: 理由」のコロンでつないだ形にしない）。ほかは core の文
            match error {
                CoreError::Cancelled | CoreError::LayerLocked { .. } => {}
                CoreError::MergeRefused(reason) => assert_eq!(
                    Lang::Ja.core_error(error),
                    format!("結合できません（{reason}）。")
                ),
                CoreError::MergeAppearance(_) => assert_eq!(
                    Lang::Ja.core_error(error),
                    "結合で見た目が許容差を超えて変わります（7）。"
                ),
                _ => assert_eq!(Lang::Ja.core_error(error), error.to_string()),
            }
            assert!(!Lang::Ja.core_error(error).contains(": "), "{error:?}");
            assert!(en.is_ascii() && !en.is_empty(), "{en}");
            assert!(english[i + 1..].iter().all(|other| other != en), "{en}");
        }
        assert!(english.last().unwrap().contains('7'));
    }

    /// 効かない効果の理由（どの種類のマップ・Anchor が使えないか）と 1 行の文・知らせが、画面の言語で出る。
    #[test]
    fn inactive_effects_are_told_in_both_languages() {
        use yolu_core::generator::{Inactive as I, Kind, MapKind};
        use yolu_core::{Channel, LayerId};
        let reasons = [
            InactiveReason::Generator(I::MissingMap(MapKind::Thickness)),
            InactiveReason::Generator(I::StaleMap(MapKind::Position)),
            InactiveReason::Generator(I::UnverifiedMap(MapKind::WorldNormal)),
            InactiveReason::Generator(I::MapSize(MapKind::Curvature)),
            InactiveReason::Generator(I::PinMismatch(MapKind::Curvature)),
            InactiveReason::Generator(I::MissingFrame),
            InactiveReason::Generator(I::EmptyBounds),
            InactiveReason::Generator(I::NoIdColors),
            InactiveReason::Generator(I::Anchor(anchor::Issue::NotChosen)),
            InactiveReason::Generator(I::Anchor(anchor::Issue::Missing)),
            InactiveReason::Generator(I::Anchor(anchor::Issue::NotBelow)),
            InactiveReason::Generator(I::NoModel),
            InactiveReason::Generator(I::IslandMap),
            InactiveReason::Rejected("ASCII reason".into()),
            InactiveReason::Rejected("範囲外の値".into()),
        ];
        let english: Vec<String> = reasons
            .iter()
            .map(|r| Lang::En.inactive_reason(r))
            .collect();
        for (i, (reason, en)) in reasons.iter().zip(&english).enumerate() {
            assert_eq!(Lang::Ja.inactive_reason(reason), reason.to_string());
            assert!(en.is_ascii() && !en.is_empty(), "{en}");
            assert!(english[i + 1..].iter().all(|other| other != en), "{en}");
        }
        assert!(english[0].contains("Thickness") && english[4].contains("pinned"));
        // 結合の断りの文は、断った理由を言語ごとに運ぶ
        let refused = CoreError::InactiveEffect {
            layer: LayerId(1),
            mask: false,
            reason: Box::new(reasons[0].clone()),
        };
        assert_eq!(
            Lang::Ja.core_error(&refused),
            "効いていない効果は焼き込めません（Thickness のマップがありません）。"
        );
        assert_eq!(
            Lang::En.core_error(&refused),
            "Cannot bake an inactive effect (No Thickness map)."
        );
        // 1 行の文: レイヤーの名前は利用者の文字列なのでそのまま、種類と理由は英語
        let effects = [
            InactiveEffect {
                layer: LayerId(1),
                layer_name: "Top".into(),
                target: InactiveTarget::Generator {
                    mask: false,
                    kind: Kind::EdgeWear,
                },
                reason: reasons[0].clone(),
            },
            InactiveEffect {
                layer: LayerId(1),
                layer_name: "Top".into(),
                target: InactiveTarget::Generator {
                    mask: true,
                    kind: Kind::Anchor,
                },
                reason: reasons[8].clone(),
            },
            InactiveEffect {
                layer: LayerId(2),
                layer_name: "Fill".into(),
                target: InactiveTarget::FillGradient(Channel::Color),
                reason: reasons[1].clone(),
            },
            InactiveEffect {
                layer: LayerId(2),
                layer_name: "Fill".into(),
                target: InactiveTarget::Decal,
                reason: reasons[5].clone(),
            },
            InactiveEffect {
                layer: LayerId(2),
                layer_name: "Fill".into(),
                target: InactiveTarget::FillImage(Channel::Roughness),
                reason: reasons[3].clone(),
            },
        ];
        let lines: Vec<String> = effects
            .iter()
            .map(|e| Lang::En.inactive_effect(e))
            .collect();
        for (i, (effect, line)) in effects.iter().zip(&lines).enumerate() {
            // 1 つの文（「〜: 〜」のコロンでつながない）
            let ja = Lang::Ja.inactive_effect(effect);
            assert!(
                ja.contains(&effect.layer_name) && !ja.contains(": ") && !line.contains(": "),
                "{ja} / {line}"
            );
            assert!(
                line.is_ascii() && line.contains(&effect.layer_name),
                "{line}"
            );
            assert!(lines[i + 1..].iter().all(|other| other != line), "{line}");
        }
        assert_eq!(
            lines[0],
            "\"Top\" Edge wear has no effect (No Thickness map)."
        );
        assert_eq!(
            lines[1],
            "\"Top\" Anchor (mask) has no effect (No anchor chosen)."
        );
        assert!(
            lines[2].contains("gradient (Color)")
                && lines[3].contains("decal")
                && lines[4].contains("image (Roughness)")
        );
        // 保存の知らせは、書き直したセットの名前と件数と初めの理由
        let ja = Lang::Ja.inactive_effects_not_in_composite("Skin", &effects);
        assert!(
            ja.contains("「Skin」")
                && ja.contains("5 件")
                && ja.contains("合成の PNG に入っていません")
                && ja.contains(Lang::Ja.inactive_effect(&effects[0]).trim_end_matches('。')),
            "{ja}"
        );
        let en = Lang::En.inactive_effects_not_in_composite("Skin", &effects);
        assert_eq!(
            en,
            format!(
                " 5 inactive effect(s) in \"Skin\" are not in the composite PNG ({}).",
                lines[0].trim_end_matches('.')
            )
        );
    }

    /// 位置のマップが使えなくて UV の空間で評価しているノイズ・グランジの 1 行が、画面の言語で出る（レイヤーの名前はそのまま）。
    #[test]
    fn fallback_effects_are_told_in_both_languages() {
        use yolu_core::generator::{Inactive as I, Kind, MapKind};
        use yolu_core::LayerId;
        let effects = [
            FallbackEffect {
                layer: LayerId(1),
                layer_name: "Top".into(),
                mask: false,
                kind: Kind::Noise,
                reason: InactiveReason::Generator(I::MissingMap(MapKind::Position)),
            },
            FallbackEffect {
                layer: LayerId(1),
                layer_name: "Top".into(),
                mask: true,
                kind: Kind::Grunge,
                reason: InactiveReason::Generator(I::StaleMap(MapKind::Position)),
            },
        ];
        let lines: Vec<String> = effects
            .iter()
            .map(|e| Lang::En.fallback_effect(e))
            .collect();
        for (i, (effect, line)) in effects.iter().zip(&lines).enumerate() {
            let ja = Lang::Ja.fallback_effect(effect);
            assert_eq!(ja, effect.to_string());
            assert!(ja.contains("UV の空間") && ja.contains("「Top」"), "{ja}");
            assert!(
                line.is_ascii() && line.contains("\"Top\"") && line.contains("UV space"),
                "{line}"
            );
            assert!(lines[i + 1..].iter().all(|other| other != line), "{line}");
        }
        assert!(
            lines[0].contains("Noise")
                && !lines[0].contains("(mask)")
                && lines[0].contains("No Position map"),
            "{}",
            lines[0]
        );
        assert!(
            lines[1].contains("Grunge (mask)") && lines[1].contains("baked with other settings"),
            "{}",
            lines[1]
        );
        assert!(Lang::Ja.fallback_effect(&effects[1]).contains("（マスク）"));
    }

    #[test]
    fn editing_errors_use_the_selected_language() {
        for error in [
            CoreError::LayerNotFound,
            CoreError::ChannelNotFound,
            CoreError::StrokeActive,
            CoreError::NoActiveStroke,
            CoreError::SourceBudgetExceeded,
            CoreError::StrokeBudgetExceeded,
            CoreError::WorkingBudgetExceeded,
            CoreError::Unsupported("塗りつぶしレイヤーには描けない"),
        ] {
            assert!(!Lang::Ja.core_error(&error).is_empty());
            assert!(Lang::En.core_error(&error).is_ascii());
        }
        // 同じ誤りは、どの操作でも同じ文（塗るツール・棚・覚えた選択範囲の別の表は無くした）
        assert_eq!(
            Lang::Ja.core_error(&CoreError::LayerNotFound),
            "レイヤーがありません"
        );
        assert_eq!(
            Lang::En.core_error(&CoreError::LayerNotFound),
            "Layer not found"
        );
        for lang in Lang::ALL {
            assert_eq!(
                lang.core_error(&CoreError::StrokeActive),
                super::super::refusals::during_stroke(lang)
            );
        }
        // できない操作は、core の理由の文をそのまま 1 文に（「できない: 理由」のコロンでつないだ形にしない）
        assert_eq!(
            Lang::Ja.core_error(&CoreError::Unsupported("塗りつぶしレイヤーには描けない")),
            "塗りつぶしレイヤーには描けない。"
        );
        let error = CoreError::Unsupported("塗りつぶしレイヤーには描けない");
        assert_eq!(Lang::En.core_error(&error), "Cannot paint a fill layer.");
        assert_eq!(
            Lang::Ja.core_error(&CoreError::InvalidArgument("絵の具の量（0〜1）")),
            "値が範囲外です（絵の具の量（0〜1））。"
        );
        assert_eq!(
            Lang::En.core_error(&CoreError::InvalidArgument("絵の具の量（0〜1）")),
            "Invalid value (Paint amount (0–1))."
        );
    }
    #[test]
    fn clipboard_refusals_use_the_selected_language() {
        use yolu_core::ClipboardRefusal::*;
        let errors: Vec<CoreError> = [
            NoPixels,
            NothingToCopy { selection: true },
            NothingToCopy { selection: false },
            TooLarge { bytes: 2, limit: 1 },
            OperationBudget { bytes: 2, limit: 1 },
            NotPaintLayer,
        ]
        .into_iter()
        .map(CoreError::Clipboard)
        .chain([CoreError::BatchActive, CoreError::TileUnreadable])
        .collect();
        let english: Vec<String> = errors.iter().map(|e| Lang::En.core_error(e)).collect();
        for (i, (error, en)) in errors.iter().zip(&english).enumerate() {
            // 取り消しとロックは、両方の言語で決めた文（ロックはどのロックか・親のグループか）。ほかは core の文
            if !matches!(error, CoreError::Cancelled | CoreError::LayerLocked { .. }) {
                assert_eq!(Lang::Ja.core_error(error), error.to_string());
            }
            assert!(en.is_ascii() && !en.is_empty(), "{en}");
            assert!(english[i + 1..].iter().all(|other| other != en), "{en}");
            // 画面に出す理由に、開発用の数（バイト・MiB）は入れない
            assert!(
                !error.to_string().chars().any(|c| c.is_ascii_digit()),
                "{error}"
            );
            assert!(!en.chars().any(|c| c.is_ascii_digit()), "{en}");
        }
    }
    #[test]
    fn every_core_reason_has_an_english_text() {
        // core が CoreError に渡す理由の文は全部、英語の文を持つ（新しい理由を足したらここで落ちる。core_reason に足す）
        fn sources(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    sources(&path, out);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    out.push(path);
                }
            }
        }
        let mut files = Vec::new();
        sources(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../yolu-core/src"),
            &mut files,
        );
        assert!(files.len() > 10);
        let (mut found, mut missing) = (0, Vec::new());
        for file in files {
            let text = std::fs::read_to_string(&file).unwrap();
            for head in ["CoreError::InvalidArgument(", "CoreError::Unsupported("] {
                for (at, _) in text.match_indices(head) {
                    let rest = text[at + head.len()..].trim_start();
                    let Some(rest) = rest.strip_prefix('"') else {
                        continue;
                    };
                    let reason = &rest[..rest.find('"').unwrap()];
                    found += 1;
                    if known_core_reason(reason).is_none() && !reason.is_ascii() {
                        missing.push(format!(
                            "{}: {reason}",
                            file.file_name().unwrap().to_string_lossy()
                        ));
                    }
                }
            }
        }
        assert!(found > 100, "{found}");
        assert!(missing.is_empty(), "英語の文が無い理由: {missing:#?}");
    }

    #[test]
    fn view_and_surface_errors_are_translated_by_kind() {
        use yolu_core::geometry::SurfaceStrokeError;
        let rig = RigError::TooLarge {
            what: "ボーン",
            value: 2,
            limit: 1,
        };
        let mut errors = vec![
            ViewError::Stroking,
            ViewError::NoPoseModel,
            ViewError::NoPoseEdit,
            ViewError::NoLinkModel,
            ViewError::NoPoseBase,
            ViewError::BadMeshIndex,
            ViewError::NoTriangles,
            ViewError::Cancelled,
            ViewError::LoadStopped,
            ViewError::PoseGeneration {
                pose: 2,
                model: Some(1),
            },
            ViewError::PoseGeneration {
                pose: 2,
                model: None,
            },
            ViewError::PoseMesh,
            ViewError::PoseVertices,
            ViewError::Rig(rig.clone()),
            ViewError::Rig(RigError::NonFinite { what: "ポーズ" }),
            ViewError::Rig(RigError::PoseMismatch),
            ViewError::Rig(RigError::BadParent { bone: 1 }),
            ViewError::Rig(RigError::BadMesh { mesh: 1 }),
            ViewError::Rig(RigError::BadSkin { mesh: 1 }),
            ViewError::Rig(RigError::BadBlendShape { mesh: 1, shape: 2 }),
            ViewError::Model(ModelError::Io("denied".into())),
            ViewError::Model(ModelError::FileTooLarge {
                bytes: 3 << 20,
                limit: 1 << 20,
            }),
            ViewError::Model(ModelError::Parse("bad".into())),
            ViewError::Model(ModelError::NoMesh),
            ViewError::Model(ModelError::Cancelled),
            ViewError::Model(ModelError::Rig(rig)),
            ViewError::Model(ModelError::Changed),
            ViewError::Model(ModelError::NoTake),
        ];
        for e in [
            GeometryError::NonFinite,
            GeometryError::BoundsOverflow,
            GeometryError::InvalidTolerance,
            GeometryError::TooManyTriangles,
            GeometryError::Canceled,
            GeometryError::Mismatch,
        ] {
            errors.push(ViewError::Geometry(e));
        }
        for e in &errors {
            let (ja, en) = (Lang::Ja.view_error(e), Lang::En.view_error(e));
            // 読めない FBX・読めないファイルは「何が（なぜ）」の 1 文に組み直す。ほかは core・モデルの文のまま
            match e {
                ViewError::Model(m @ (ModelError::Io(_) | ModelError::Parse(_))) => {
                    assert!(!ja.contains(": ") && ja.ends_with("）。"), "{ja}");
                    assert_ne!(ja, m.to_string());
                }
                _ => assert_eq!(ja, e.to_string()),
            }
            assert!(en.is_ascii() && !en.is_empty(), "{en}");
            assert_ne!(ja, en);
        }
        assert_eq!(
            Lang::En.view_error(&ViewError::Model(ModelError::Rig(RigError::TooLarge {
                what: "ボーン",
                value: 2,
                limit: 1
            }))),
            "Too many bones (2, maximum 1)"
        );
        use yolu_core::geometry::SamplingError;
        let mut dabs = vec![
            SurfaceStrokeError::TooManyDabs,
            SurfaceStrokeError::Core(CoreError::StrokeActive),
            SurfaceStrokeError::EffectWithSymmetry,
            SurfaceStrokeError::CloneSource,
        ];
        for e in [
            SamplingError::SnapshotChanged,
            SamplingError::BindingMismatch,
            SamplingError::InvalidArguments,
            SamplingError::ChartBudget,
            SamplingError::LookupBudget,
            SamplingError::Unreachable,
        ] {
            dabs.push(SurfaceStrokeError::Sampling(e));
        }
        for d in DAB_REFUSALS {
            dabs.push(SurfaceStrokeError::Dab(d));
        }
        for e in &dabs {
            let (ja, en) = (Lang::Ja.surface_error(e), Lang::En.surface_error(e));
            assert!(en.is_ascii() && !en.is_empty() && ja != en, "{en}");
        }
    }

    const DAB_REFUSALS: [yolu_core::geometry::DabRefusal; 8] = {
        use yolu_core::geometry::DabRefusal::*;
        [
            SnapshotChanged,
            InvalidArguments,
            BindingMismatch,
            TriangleBudget,
            PixelBudget,
            VisibilityBudget,
            BvhBudget,
            MemoryBudget,
        ]
    };

    /// 3D の塗りの断りの文に、内部の言葉を使わない・短い・日英で別々の文であることを確かめる（共通の見張り）。
    fn assert_users_words(
        seen: &mut std::collections::BTreeSet<String>,
        label: &str,
        ja: &str,
        en: &str,
    ) {
        assert!(
            ja.chars().count() <= 40 && !ja.ends_with('。'),
            "{label}: {ja}"
        );
        assert!(
            en.is_ascii() && en.len() <= 70 && !en.ends_with('.'),
            "{label}: {en}"
        );
        assert!(
            seen.insert(ja.into()) && seen.insert(en.into()),
            "{label}: 別々の文"
        );
        let lower = en.to_ascii_lowercase();
        for word in [
            "bvh", "budget", "ray", "snapshot", "triangle", "mesh", "dab", "chart", "lookup",
            "sampling", "binding",
        ] {
            assert!(!lower.contains(word), "{label}: {en}");
        }
        for word in [
            "BVH",
            "予算",
            "レイ",
            "スナップショット",
            "三角形",
            "メッシュ",
            "ダブ",
            "参照",
            "図",
        ] {
            assert!(!ja.contains(word), "{label}: {ja}");
        }
    }

    /// 3D のブラシのストロークで塗れなかった理由の 8 つの文。2D のストロークと同じく、どれもストロークごと取り消すので「取り消し」を言う。
    /// 1 回の操作のメモリが足りないときは、2D の予算を超えたとき（`CoreError::StrokeBudgetExceeded`。設定のウィンドウの名前で言う）と
    /// 同じ文。ほかの 7 つは内部の言葉（BVH・予算・レイ・スナップショット）を使わず、短く、日英で別々の文になる。
    #[test]
    fn dab_refusals_cancel_the_stroke_and_the_memory_one_is_the_2d_sentence() {
        use yolu_core::geometry::{DabRefusal, SurfaceStrokeError};
        let mut seen = std::collections::BTreeSet::new();
        for refusal in DAB_REFUSALS {
            let (ja, en) = (Lang::Ja.dab_refusal(refusal), Lang::En.dab_refusal(refusal));
            assert!(
                ja.contains("取り消") && en.to_ascii_lowercase().contains("cancelled"),
                "{refusal:?}: {ja} / {en}"
            );
            for lang in [Lang::Ja, Lang::En] {
                assert_eq!(
                    lang.surface_error(&SurfaceStrokeError::Dab(refusal)),
                    lang.dab_refusal(refusal),
                    "{refusal:?}"
                );
            }
            if refusal == DabRefusal::MemoryBudget {
                assert_eq!(ja, Lang::Ja.core_error(&CoreError::StrokeBudgetExceeded));
                assert_eq!(en, Lang::En.core_error(&CoreError::StrokeBudgetExceeded));
                continue;
            }
            assert_users_words(&mut seen, &format!("{refusal:?}"), ja, en);
        }
    }

    /// パスの評価では 8 つの断りのどれも評価ごと失敗して何も塗られない（`yolu_core::paths` は種類を問わず `Error::Dab` を返す）。
    /// 「一部が塗れなかった」とは言わず、どれも「塗れませんでした」を言う。パスの画面の文（`pathtool::path_error_text`）がこの文を
    /// 使っていることもここで押さえる。
    #[test]
    fn path_evaluation_refusals_never_claim_a_partial_result() {
        use yolu_core::paths::Error;
        let mut seen = std::collections::BTreeSet::new();
        for refusal in DAB_REFUSALS {
            let (ja, en) = (
                Lang::Ja.path_dab_refusal(refusal),
                Lang::En.path_dab_refusal(refusal),
            );
            assert_users_words(&mut seen, &format!("{refusal:?}"), ja, en);
            for (lang, text) in [(Lang::Ja, ja), (Lang::En, en)] {
                assert_eq!(path_error(lang, &Error::Dab(refusal)), text, "{refusal:?}");
            }
            for part in ["一部", "所があります", "Some parts"] {
                assert!(
                    !ja.contains(part) && !en.contains(part),
                    "{refusal:?}: {ja} / {en}"
                );
            }
            assert!(
                ja.ends_with("塗れませんでした") && en.starts_with("Not painted"),
                "{refusal:?}: {ja} / {en}"
            );
        }
    }

    /// 指先・クローンの読み元の断りと、1 回の入力のダブの上限の文も、3D の塗りの流れで同じ種類の上限として出るので、内部の言葉を使わない。
    /// どれもストロークごと取り消す。
    #[test]
    fn sampling_and_dab_limit_errors_are_told_in_the_users_words() {
        use yolu_core::geometry::{SamplingError, SurfaceStrokeError};
        let mut errors = vec![SurfaceStrokeError::TooManyDabs];
        for e in [
            SamplingError::SnapshotChanged,
            SamplingError::BindingMismatch,
            SamplingError::InvalidArguments,
            SamplingError::ChartBudget,
            SamplingError::LookupBudget,
            SamplingError::Unreachable,
        ] {
            errors.push(SurfaceStrokeError::Sampling(e));
        }
        let mut seen = std::collections::BTreeSet::new();
        for e in &errors {
            let (ja, en) = (Lang::Ja.surface_error(e), Lang::En.surface_error(e));
            assert_users_words(&mut seen, &format!("{e:?}"), &ja, &en);
            assert!(
                ja.contains("取り消") && en.starts_with("Cancelled"),
                "{e:?}: {ja} / {en}"
            );
        }
    }

    /// 種類の違う失敗は、日英どちらでも別々の文になる（予算超過・壊れたデータ・まだ書けない中身・衝突・ファイルの失敗）。
    #[test]
    fn io_errors_are_told_apart_by_kind_in_both_languages() {
        use yolu_io::{Error, Unwritable};
        let errors = [
            Error::InvalidData("正本が不正".into()),
            Error::Budget("アーカイブの予算超過です".into()),
            Error::Unwritable(Unwritable::GeneratorRampMixing),
            Error::SaveConflict("保存先が外部で変更されています".into()),
            Error::UnsupportedFormat {
                format: 99,
                app: "FuturePainter".into(),
                version: "9.0".into(),
            },
            Error::from(std::io::Error::from(std::io::ErrorKind::PermissionDenied)),
            Error::from(CoreError::StrokeActive),
            Error::from(CoreError::SourceBudgetExceeded),
            Error::SaveConflict("別の保存が進行中です".into()),
            Error::SaveConflict("バックアップ先がフォルダーではありません".into()),
            Error::SaveConflict("バックアップ先がシンボリックリンクです".into()),
            Error::SaveConflict("ロックのファイルがシンボリックリンクです".into()),
            Error::SaveConflict("保存先が外部で消されています。上書きしません".into()),
        ];
        for lang in Lang::ALL {
            let texts: Vec<String> = errors.iter().map(|e| lang.io_error(e)).collect();
            for (i, a) in texts.iter().enumerate() {
                assert!(!a.is_empty());
                assert_eq!(a.is_ascii(), lang == Lang::En, "{lang:?} {a}");
                for b in &texts[i + 1..] {
                    assert_ne!(a, b, "{lang:?} {i}");
                }
            }
        }
        // 日本語は診断（どの項目か・どの予算か）を保つ。英語は種類を言い、診断の日本語は出さない
        assert!(Lang::Ja.io_error(&errors[0]).contains("正本が不正"));
        assert!(Lang::Ja
            .io_error(&errors[1])
            .contains("アーカイブの予算超過"));
        assert!(Lang::Ja.io_error(&errors[2]).contains("混色"));
        assert!(Lang::En.io_error(&errors[1]).contains("limit exceeded"));
        assert!(Lang::En.io_error(&errors[2]).contains("Color mixing"));
        // 別の保存が進行中の衝突は、外で変わった衝突と日英どちらでも言い分ける
        assert_eq!(Lang::En.io_error(&errors[8]), "Another save is in progress");
        assert!(Lang::En.io_error(&errors[3]).contains("changed"));
        assert!(Lang::Ja.io_error(&errors[8]).contains("進行中"));
        // 保存先が外で消された衝突は、変わった衝突とも言い分ける
        assert!(Lang::En.io_error(&errors[12]).contains("deleted or moved"));
        assert!(Lang::Ja.io_error(&errors[12]).contains("消されています"));
        // 保存先の周りの不具合は、データの不正（InvalidData の汎用文）にも「外で変わった」にも見せず、場所が理由だと言う
        for (i, key) in [
            (9, "Backup location is not a folder"),
            (10, "link"),
            (11, "lock file"),
        ] {
            let en = Lang::En.io_error(&errors[i]);
            assert!(
                en.contains(key) && !en.contains("changed") && !en.contains("Invalid"),
                "{en}"
            );
        }
    }

    /// 保存が実際に返す「置き場の不具合」の理由が、英語のウィンドウでも言い分けられる（理由の文を書き換えて、表の対応が外れても気づく）。
    #[test]
    fn real_save_refusals_about_the_place_are_told_in_english() {
        use yolu_io::{Error, SaveTarget};
        let dir = std::env::temp_dir().join(format!("yolu-app-save-places-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let sample = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../yolu-io/tests/fixtures/format1.ylp"
        ));
        let open = |name: &str| {
            let path = dir.join(name);
            std::fs::write(&path, sample).unwrap();
            SaveTarget::open(&path).unwrap()
        };
        let refusal = |name: &str| {
            let (project, mut target) = open(name);
            let error = target.save(&project).unwrap_err();
            assert!(matches!(error, Error::SaveConflict(_)), "{error:?}");
            (Lang::Ja.io_error(&error), Lang::En.io_error(&error))
        };
        // 前の版の置き場が普通のファイルで塞がれている
        std::fs::write(dir.join("a.ylp-backups~"), b"a file").unwrap();
        let (ja, en) = refusal("a.ylp");
        assert!(ja.contains("バックアップ先") && !ja.is_ascii(), "{ja}");
        assert_eq!(en, "Backup location is not a folder");
        #[cfg(unix)]
        {
            // 前の版の置き場・ロックの場所がシンボリックリンク
            std::os::unix::fs::symlink(&dir, dir.join("b.ylp-backups~")).unwrap();
            assert_eq!(refusal("b.ylp").1, "Backup location is a link");
            std::os::unix::fs::symlink(&dir, dir.join(".c.ylp.save.lock~")).unwrap();
            assert_eq!(refusal("c.ylp").1, "Save lock file is a link");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 保存先の名前と、保存の直前に外から作られた新規の保存先の拒否も、日英どちらでも言い分けられる。
    #[test]
    fn real_save_refusals_about_the_name_and_a_target_created_elsewhere_are_told_in_both_languages()
    {
        use yolu_io::{Error, Project, SaveTarget};
        let dir = std::env::temp_dir().join(format!("yolu-app-save-names-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let sample = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../yolu-io/tests/fixtures/format1.ylp"
        ));
        let project = Project::read(sample).unwrap();
        let error = SaveTarget::create(dir.join("a.txt")).unwrap_err();
        assert!(matches!(error, Error::SaveConflict(_)), "{error:?}");
        assert!(
            Lang::Ja.io_error(&error).contains(".ylp") && !Lang::Ja.io_error(&error).is_ascii()
        );
        assert_eq!(
            Lang::En.io_error(&error),
            "The file name must end with .ylp"
        );
        let mut target = SaveTarget::create(dir.join("b.ylp")).unwrap();
        std::fs::write(dir.join("b.ylp"), b"made elsewhere").unwrap();
        let error = target.save(&project).unwrap_err();
        assert!(
            Lang::Ja.io_error(&error).contains("外部で作られました"),
            "{}",
            Lang::Ja.io_error(&error)
        );
        assert_eq!(
            Lang::En.io_error(&error),
            "A file appeared at the save target; not overwritten"
        );
        assert_eq!(std::fs::read(dir.join("b.ylp")).unwrap(), b"made elsewhere");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_errors_name_the_kind_and_keep_the_os_error_number() {
        use std::io::{Error, ErrorKind};
        let kinds = [
            ErrorKind::NotFound,
            ErrorKind::PermissionDenied,
            ErrorKind::AlreadyExists,
            ErrorKind::InvalidData,
            ErrorKind::StorageFull,
            ErrorKind::QuotaExceeded,
            ErrorKind::ReadOnlyFilesystem,
            ErrorKind::ResourceBusy,
            ErrorKind::FileTooLarge,
            ErrorKind::Other,
        ];
        for lang in Lang::ALL {
            let texts: Vec<String> = kinds
                .iter()
                .map(|k| lang.file_error(&Error::from(*k)))
                .collect();
            for (i, a) in texts.iter().enumerate() {
                assert_eq!(a.is_ascii(), lang == Lang::En, "{lang:?} {a}");
                assert!(!a.contains("OS"), "{a}");
                for b in &texts[i + 1..] {
                    assert_ne!(a, b);
                }
            }
            // OS の番号（共有違反など、種類に落ちない失敗の手掛かり）は、どの言語にも残る
            let os = lang.file_error(&Error::from_raw_os_error(32));
            assert!(os.contains("32") && os.contains("OS"), "{os}");
            assert_eq!(os.is_ascii(), lang == Lang::En, "{os}");
        }
    }

    #[test]
    fn project_notes_and_unsupported_features_keep_their_contents_in_english() {
        use yolu_io::Note;
        let notes = [
            Note::ViewSlotUnreadable("x".into()),
            Note::Migrated { format: 2 },
            Note::MaterialRefsMigrated { format: 3 },
            Note::UnknownEntryKept("future.bin".into()),
            Note::SetNotConvertible {
                set: "Skin".into(),
                issue: "layers[2].filters（フィルター・ジェネレーター）".into(),
            },
            Note::SmartResourceKept("Rust".into()),
            Note::BrushKept,
        ];
        let english: Vec<String> = notes.iter().map(|n| Lang::En.project_note(n)).collect();
        for (i, (note, en)) in notes.iter().zip(&english).enumerate() {
            assert_eq!(Lang::Ja.project_note(note), note.to_string());
            assert!(
                en.chars().all(|c| (c as u32) < 0x3000) && !en.is_empty(),
                "{en}"
            );
            assert!(english[i + 1..].iter().all(|other| other != en));
        }
        // 件数だけでなく、名前・形式・項目のキーが読める
        assert!(english[1].contains('2') && english[2].contains('3'));
        assert!(english[3].contains("future.bin") && english[5].contains("Rust"));
        assert!(
            english[4].contains("Skin")
                && english[4].contains("layers[2].filters")
                && !english[4].contains("ジェネレーター")
        );

        let issues: Vec<String> = [
            "layers[0].locks（ロック）",
            "manual_id_colors（手動の ID 色）",
            "layers[1].filters（フィルター・ジェネレーター）",
            "layers[2].anchor（Anchor）",
            "layers[3].anchor（Anchor）",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        let ja = Lang::Ja.unsupported_features(&issues);
        assert!(ja.contains("ロック") && ja.contains("ほか 2 件"), "{ja}");
        let en = Lang::En.unsupported_features(&issues);
        assert_eq!(en, "Unsupported project features (layers[0].locks, manual_id_colors, layers[1].filters and 2 more)");
    }
}
