//! core の型と io の分類から、画面用の短い理由を作る。
use super::Lang;
use crate::stencil::StencilError;
use crate::view3d::model::ViewError;
use yolu_core::generator::{self, anchor};
use yolu_core::geometry::GeometryError;
use yolu_core::skin::RigError;
use yolu_core::{
    CoreError, FallbackEffect, InactiveEffect, InactiveReason, InactiveTarget, Reason,
};
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
    crate::layerops::lock_names(lang, lock).join(lang.t("lang-text"))
}

impl Lang {
    /// 設定の予算で断った理由のうち、どの予算かを言う文（yolu-io の理由の文で見分ける。数は出さない）。日本語は yolu-io の文、
    /// 英語は短い文。どの予算か分からなければ None。
    pub(crate) fn budget_reason(self, text: &str) -> Option<String> {
        let key = if text.contains(yolu_io::OVER_LAYER_PIXELS_DOCUMENT) {
            self.f("lang-a-project-exceeds-the-layer", &[("text", &text)])
        } else if text.contains(yolu_io::OVER_LAYER_PIXELS_TOTAL) {
            self.f("lang-the-whole-exceeds-the-layer", &[("text", &text)])
        } else if text.contains("グループの入れ子の上限") {
            self.f("lang-groups-are-nested-too-deeply-2", &[("text", &text)])
        } else if text.contains("キャンバスの辺の上限") {
            self.f("lang-the-canvas-edge-exceeds-the", &[("text", &text)])
        } else if text.contains("レイヤーの数の上限") {
            self.f("lang-too-many-layers-limit-2048", &[("text", &text)])
        } else {
            return None;
        };
        Some(key)
    }

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
            CoreError::Cancelled => self.t("lang-cancelled").into(),
            CoreError::StrokeActive => super::refusals::during_stroke(self).into(),
            CoreError::NoActiveStroke => self.t("lang-stroke-already-ended").into(),
            CoreError::LayerNotFound | CoreError::InvalidArgument(Reason::NoLayersToSave) => {
                self.t("lang-layer-not-found").into()
            }
            CoreError::ChannelNotFound => self.t("lang-channel-not-found").into(),
            // 予算は設定のウィンドウの名前（レイヤーのメモリ・1 回の操作）で言う
            CoreError::SourceBudgetExceeded => self.t("lang-over-the-layer-memory-budget").into(),
            CoreError::StrokeBudgetExceeded => self.t("lang-over-the-memory-budget-of").into(),
            CoreError::WorkingBudgetExceeded => {
                self.t("lang-working-memory-budget-exceeded").into()
            }
            CoreError::Unsupported(Reason::UserChannelMappingDoesNotMatch) => {
                self.t("lang-channels-do-not-match").into()
            }
            CoreError::InvalidArgument(Reason::TooManyLayers) => {
                self.t("lang-too-many-layers").into()
            }
            CoreError::InvalidArgument(Reason::NoMaskToSave) => self.t("lang-no-mask").into(),
            CoreError::InvalidArgument(Reason::SmartMaterialName) => {
                self.t("lang-name-not-allowed").into()
            }
            CoreError::InvalidArgument(Reason::TargetNotGroup) => {
                self.t("lang-target-is-not-a-group").into()
            }
            CoreError::InvalidArgument(Reason::SelectionNameTooLong) => self.f(
                "lang-the-name-is-too-long",
                &[("max_saved_name_chars", &yolu_core::MAX_SAVED_NAME_CHARS)],
            ),
            CoreError::InvalidArgument(Reason::SavedSelectionNameExists) => {
                self.t("lang-a-saved-selection-with-that").into()
            }
            // core の文（「できない: 〜」のようにコロンでつないだ形）は、何が（なぜ）の 1 つの文に組み直す
            CoreError::MergeRefused(reason) => {
                self.with_reason(self.t("lang-cannot-merge"), self.merge_refusal(*reason))
            }
            CoreError::MergeAppearance(report) => self.with_reason(
                self.t("lang-merge-changes-the-appearance-beyond"),
                report.max_visible_difference.to_string(),
            ),
            CoreError::InactiveEffect { reason, .. } => self.with_reason(
                self.t("lang-cannot-bake-an-inactive-effect"),
                self.inactive_reason(reason),
            ),
            CoreError::InvalidArgument(what) => {
                self.with_reason(self.t("lang-invalid-value"), self.reason(*what))
            }
            CoreError::Unsupported(what) => self.with_reason(self.reason(*what), ""),
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
            // 日本語は yolu-io が作った診断をそのまま出し（.ftl は `{ $text }`）、英語は種類ごとの短い文（.ftl は
            // `{ $text -> *[other] 英語の文 }`。`text` は日英で引数の組を揃えるための印で、英語の文には出ない）
            Error::Json(e) => self.f("lang-invalid-json", &[("e", e)]),
            Error::InvalidData(text) => self.f(
                "lang-invalid-or-unsupported-project-data",
                &[("text", text)],
            ),
            // 設定の予算で断ったものは、どの予算かを英語でも言う（yolu-io の理由の文で見分ける）
            Error::Budget(text) => self
                .budget_reason(text)
                .unwrap_or_else(|| self.f("lang-size-count-or-memory-limit", &[("text", text)])),
            Error::Unwritable(what) => match what {
                Unwritable::GeneratorRampMixing => self.f(
                    "lang-color-mixing-in-a-fill",
                    &[("text", &what.to_string())],
                ),
            },
            // 保存の衝突のうち、保存先が外で変わったのではない理由（yolu-io の store.rs）は言い分ける。
            // 別の保存がロックを持っているのは待ち、前の版の置き場とロックの場所の不具合は保存先の周りの事情
            Error::SaveConflict(text) if text.contains("進行中") => {
                self.f("lang-another-save-is-in-progress", &[("text", text)])
            }
            Error::SaveConflict(text)
                if text.contains("バックアップ先がフォルダーではありません") =>
            {
                self.f("lang-backup-location-is-not-a", &[("text", text)])
            }
            Error::SaveConflict(text) if text.contains("バックアップ先がシンボリックリンク") => {
                self.f("lang-backup-location-is-a-link", &[("text", text)])
            }
            Error::SaveConflict(text) if text.contains("ロックのファイル") => {
                self.f("lang-save-lock-file-is-a", &[("text", text)])
            }
            // 保存先の名前（.ylp で終わらない）と、保存の直前に外から作られた新規の保存先
            Error::SaveConflict(text) if text.contains(".ylp で終わっていません") => {
                self.f("lang-the-file-name-must-end", &[("text", text)])
            }
            Error::SaveConflict(text) if text.contains("外部で作られました") => {
                self.f("lang-a-file-appeared-at-the", &[("text", text)])
            }
            // 保存先が、開いたあとで外で消された・動かされた（変わったのとは言い分ける）
            Error::SaveConflict(text) if text.contains("外部で消されています") => {
                self.f("lang-the-save-target-was-deleted", &[("text", text)])
            }
            // 開いた .ylp の古い版の写し（置換の規則が POSIX でないファイルシステムで、保存が置き換えるために手放した）
            Error::SaveConflict(text) if text.contains(yolu_io::SOURCE_RELEASED) => {
                self.f("lang-the-opened-ylp-was-replaced", &[("text", text)])
            }
            Error::SaveConflict(text) => {
                self.f("lang-save-target-or-backup-changed", &[("text", text)])
            }
            Error::UnsupportedFormat {
                format,
                app,
                version,
            } => self.f(
                "lang-the-ylp-format-is-not",
                &[
                    ("value", &format),
                    ("app", &app),
                    ("version", &version),
                    ("max_format", &yolu_io::MAX_FORMAT),
                ],
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
            NotFound => self.t("lang-file-or-folder-not-found"),
            PermissionDenied => self.t("lang-access-denied"),
            AlreadyExists => self.t("lang-file-already-exists"),
            InvalidData => self.t("lang-invalid-file-data"),
            StorageFull => self.t("lang-disk-full"),
            QuotaExceeded => self.t("lang-disk-quota-exceeded"),
            ReadOnlyFilesystem => self.t("lang-read-only-location"),
            ResourceBusy => self.t("lang-file-in-use"),
            FileTooLarge => self.t("lang-file-too-large"),
            _ => self.t("lang-file-i-o-failed"),
        };
        match error.raw_os_error() {
            Some(code) => self.f("lang-os-error", &[("reason", &reason), ("code", &code)]),
            None => reason.into(),
        }
    }

    /// メッシュマップの誤りの文（英語は理由の種類から引く。スレッドの束の失敗は OS・rayon の文）。
    pub fn mesh_map_error(self, error: &yolu_core::mesh_maps::MeshMapError) -> String {
        use yolu_core::mesh_maps::MeshMapError;
        crate::crash::problem(match error {
            MeshMapError::Refused(why) => self.reason(*why).into(),
            // スレッドの束の失敗は OS・rayon の文（英語ならそのまま）
            MeshMapError::Threads(why) if self == Lang::Ja || why.is_ascii() => why.clone(),
            MeshMapError::Threads(why) => {
                self.f("lang-unsupported-value-or-operation", &[("text", why)])
            }
        })
    }

    /// 別のスレッドを起こせなかった理由（OS の英語の文をそのまま出さず、種類で言い分けて、OS のエラー番号を添える）。
    /// 何ができなかったかは呼ぶ側が `with_reason` の「何が」で言う。
    pub fn thread_error(self, error: &std::io::Error) -> String {
        use std::io::ErrorKind::*;
        let reason = match error.kind() {
            WouldBlock | OutOfMemory => self.t("lang-not-enough-system-resources"),
            _ => self.t("lang-the-system-refused-to-start"),
        };
        crate::crash::problem(match error.raw_os_error() {
            Some(code) => self.f("lang-os-error", &[("reason", &reason), ("code", &code)]),
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
            StencilError::NotPng => self.t("lang-not-a-readable-png").into(),
            StencilError::TooLarge {
                width,
                height,
                side,
            } => self.f(
                "lang-image-too-large-maximum-side",
                &[("width", &width), ("height", &height), ("side", &side)],
            ),
            StencilError::Limits => self.t("lang-image-too-large-memory-limit").into(),
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

impl Lang {
    /// 統合を断った理由。日本語は core の文（`MergeRefusal` の `Display`）、英語は種類ごとの短い文。
    fn merge_refusal(self, reason: yolu_core::MergeRefusal) -> String {
        use yolu_core::MergeRefusal::*;
        let text = reason.to_string();
        match reason {
            NoLayerBelow => self.f("lang-no-layer-below", &[("text", &text)]),
            LayerBelowIsGroup => self.f("lang-the-layer-below-is-a", &[("text", &text)]),
            LayerBelowIsAdjustment => self.f("lang-the-layer-below-is-an", &[("text", &text)]),
            HiddenLayer => self.f("lang-a-layer-is-hidden", &[("text", &text)]),
            IsGroup => self.f("lang-the-target-is-a-group", &[("text", &text)]),
            NotGroup => self.f("lang-not-a-group", &[("text", &text)]),
            EmptyGroup => self.f("lang-the-group-is-empty", &[("text", &text)]),
            NothingVisible => self.f("lang-no-visible-layers", &[("text", &text)]),
            DifferentGroups => self.f(
                "lang-layers-have-different-parent-groups",
                &[("text", &text)],
            ),
            TooManyRulers => self.f("lang-the-merged-layer-would-have", &[("text", &text)]),
        }
    }

    pub fn surface_error(self, error: &yolu_core::geometry::SurfaceStrokeError) -> String {
        crate::crash::problem(self.surface_error_text(error))
    }
    fn surface_error_text(self, error: &yolu_core::geometry::SurfaceStrokeError) -> String {
        use yolu_core::geometry::SurfaceStrokeError;
        match error {
            SurfaceStrokeError::Core(e) => self.core_error(e),
            SurfaceStrokeError::Dab(e) => self.dab_refusal(*e).into(),
            SurfaceStrokeError::TooManyDabs => self.t("lang-cancelled-the-stroke-is-too").into(),
            SurfaceStrokeError::Sampling(e) => self.sampling_error(*e).into(),
            SurfaceStrokeError::EffectWithSymmetry => self.t("lang-smudge-and-clone-do-not").into(),
            SurfaceStrokeError::CloneSource => self.t("lang-the-clone-source-is-not").into(),
        }
    }
    /// 指先・クローンの読み元を決められなかった理由。どれもストロークごと取り消すので、「取り消し」を言う。内部の言葉（スナップショット・予算・図）は使わない。
    pub fn sampling_error(self, error: yolu_core::geometry::SamplingError) -> &'static str {
        crate::crash::problem(self.sampling_error_text(error))
    }
    fn sampling_error_text(self, error: yolu_core::geometry::SamplingError) -> &'static str {
        use yolu_core::geometry::SamplingError::*;
        match error {
            SnapshotChanged => self.t("lang-cancelled-the-model-changed"),
            BindingMismatch => self.t("lang-cancelled-the-surface-does-not"),
            InvalidArguments => self.t("lang-cancelled-brush-size-or-position"),
            ChartBudget => self.t("lang-cancelled-too-many-faces-to"),
            LookupBudget => self.t("lang-cancelled-the-area-to-read"),
            Unreachable => self.t("lang-cancelled-some-pixels-cannot-reach"),
        }
    }
    /// 対称の写しが塗られなかった理由（短い状態）。
    pub fn mirror_note(self, outcome: yolu_core::geometry::MirrorOutcome) -> &'static str {
        use yolu_core::geometry::MirrorOutcome::*;
        match outcome {
            NoSurface => self.t("lang-symmetry-copies-with-no-surface"),
            OtherSlot => self.t("lang-symmetry-copies-on-another-texture"),
            Hidden => self.t("lang-symmetry-copies-that-cannot-be"),
            UvMismatch => self.t("lang-symmetry-copies-with-flat-or"),
            Painted | OnPlane => "",
        }
    }
    /// 指先が、つながらない面で拾い直したときの状態。
    pub fn smudge_lost(self) -> &'static str {
        self.t("lang-smudge-picked-up-again-on")
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
            SnapshotChanged => self.t("lang-not-painted-because-the-model"),
            InvalidArguments => self.t("lang-not-painted-brush-size-out"),
            BindingMismatch => self.t("lang-not-painted-the-surface-does"),
            TriangleBudget => self.t("lang-not-painted-the-brush-covers"),
            PixelBudget => self.t("lang-not-painted-the-brush-area"),
            VisibilityBudget => self.t("lang-not-painted-too-many-points"),
            BvhBudget => self.t("lang-not-painted-too-many-overlapping"),
            MemoryBudget => self.t("lang-not-painted-not-enough-memory"),
        }
    }
    /// 3D のブラシのストロークで、打ち（ブラシの 1 回分。対称の写し・デュアルブラシの 2 つ目の打ちも）を塗れなかった理由。2D の
    /// ストロークと同じく、どれもストロークごと取り消す（塗り残しを作らない）ので、「取り消し」を言う。1 回の操作のメモリが足りない
    /// ときは、2D の予算を超えたときと同じ文。パスの評価の文は `path_dab_refusal`。内部の言葉（BVH・予算・レイ・スナップショット）は
    /// 使わず、原因を使う人の言葉で短く言う。
    fn dab_refusal_text(self, error: yolu_core::geometry::DabRefusal) -> &'static str {
        use yolu_core::geometry::DabRefusal::*;
        match error {
            SnapshotChanged => self.t("lang-cancelled-the-model-changed"),
            InvalidArguments => self.t("lang-cancelled-brush-size-or-camera"),
            BindingMismatch => self.t("lang-cancelled-the-surface-does-not"),
            TriangleBudget => self.t("lang-cancelled-the-brush-covers-too"),
            PixelBudget => self.t("lang-cancelled-the-brush-area-is"),
            VisibilityBudget => self.t("lang-cancelled-too-many-points-to"),
            BvhBudget => self.t("lang-cancelled-too-many-overlapping-faces"),
            // 2D の `CoreError::StrokeBudgetExceeded` と同じ文
            MemoryBudget => self.t("lang-over-the-memory-budget-of"),
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
            GeometryError::NonFinite => self.t("lang-non-finite-position-or-uv"),
            GeometryError::BoundsOverflow => self.t("lang-mesh-bounds-exceed-the-supported"),
            GeometryError::InvalidTolerance => self.t("lang-weld-tolerance-must-be-positive"),
            GeometryError::TooManyTriangles => self.t("lang-too-many-triangles"),
            GeometryError::Canceled => self.t("lang-cancelled"),
            GeometryError::Mismatch => self.t("lang-triangle-order-differs-from-the"),
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
            ModelError::Io(e) => self.with_reason(self.t("lang-cannot-read-the-file"), e),
            ModelError::Parse(e) => self.with_reason(self.t("lang-invalid-fbx"), e),
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
        lang.t("lang-over-the-asset-budget").into()
    } else if m.contains(REFUSAL_ARCHIVE_BUDGET) {
        lang.t("lang-over-the-file-size-limit").into()
    } else if m.contains(REFUSAL_USER_CHANNELS) {
        lang.t("lang-it-uses-user-channels").into()
    } else if m.contains(REFUSAL_RUST_GENERATORS) {
        lang.t("lang-it-uses-noise-and-grunge").into()
    } else if m.contains(REFUSAL_RUST_ADJUSTMENTS) {
        lang.t("lang-it-uses-colour-adjustments").into()
    } else if m.contains(REFUSAL_NEW_FILTERS) {
        lang.t("lang-it-uses-filters-or-generators").into()
    } else if m.contains(REFUSAL_PATH_LISTS) {
        lang.t("lang-it-uses-a-path-list").into()
    } else if m.contains(REFUSAL_POINT_GRADIENTS) {
        lang.t("lang-it-uses-a-point-gradient").into()
    } else {
        lang.io_error(e)
    }
}

/// ライブラリのフォルダ・画像の断りの短い文（そうでない断りは None）。
pub fn library_known_error(lang: Lang, e: &yolu_io::Error) -> Option<String> {
    let m = e.to_string();
    let has = |text: &str| m.contains(text);
    let short = if has(files::REFUSAL_ROOT_LINK) {
        lang.t("lang-the-library-folder-is-a")
    } else if has(files::REFUSAL_ROOT_NOT_FOLDER) {
        lang.t("lang-the-library-location-is-not")
    } else if has(files::REFUSAL_LINK) {
        lang.t("lang-links-are-not-followed")
    } else if has(files::REFUSAL_PATH) {
        lang.t("lang-the-name-has-disallowed-characters")
    } else if has(files::REFUSAL_NOT_FILE) {
        lang.t("lang-not-a-file")
    } else if has(files::REFUSAL_TOO_LARGE) {
        lang.t("lang-too-large")
    } else if has(files::REFUSAL_EMPTY) {
        lang.t("lang-empty-file")
    } else if has(files::REFUSAL_NO_NAME) {
        lang.t("lang-cannot-choose-a-name")
    } else if has(REFUSAL_UNSUPPORTED) {
        lang.t("lang-png-and-ylsmart-only")
    } else if has(REFUSAL_PNG_SIZE) {
        lang.t("lang-image-sides-must-be-1")
    } else if has(REFUSAL_PNG) {
        lang.t("lang-not-a-readable-png")
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
        Error::Invalid(why) => crate::crash::problem(lang.reason(*why)).into(),
        Error::ModelMismatch => lang.t("lang-the-model-differs-from-the").into(),
        Error::MissingTriangle => lang.t("lang-the-path-refers-to-a").into(),
        Error::TooManySamples => lang.t("lang-the-path-is-too-long").into(),
        Error::Canceled => lang.t("lang-cancelled").into(),
        Error::Core(e) => lang.core_error(e),
        Error::Dab(d) => lang.path_dab_refusal(*d).into(),
        Error::MissingImage => lang.t("lang-the-ribbon-image-is-not").into(),
        Error::FillIslands => lang.t("lang-the-fill-path-does-not").into(),
    }
}

/// 付け直せなかった理由の文（画面の言語で）。
pub fn rebind_error(lang: Lang, error: RebindError) -> String {
    match error {
        RebindError::OtherModel => lang.t("lang-the-path-was-drawn-on").into(),
        RebindError::NoMaterial => lang.t("lang-its-material-is-not-in").into(),
        RebindError::MissingTriangle { point } => lang.f(
            "lang-point-refers-to-a-triangle",
            &[("point", &(point + 1))],
        ),
        RebindError::SearchBudget { point } => {
            lang.f("lang-finding-point-on-the-new", &[("point", &(point + 1))])
        }
        RebindError::NoSurface { point, tolerance } => lang.f(
            "lang-point-has-no-surface-of",
            &[
                ("point", &(point + 1)),
                ("tolerance", &format!("{:.4}", tolerance)),
            ],
        ),
    }
}

/// 取り込めない理由（画面の言語。レイヤーの名前は利用者の名前なのでそのまま）。
pub fn psd_copy_refusal(lang: Lang, why: &CopyRefusal) -> String {
    match why {
        CopyRefusal::Malformed(text) => {
            lang.f("lang-not-a-readable-psd-damaged", &[("text", text)])
        }
        CopyRefusal::LargeDocument => lang.t("lang-psb-files-cannot-be-imported").into(),
        CopyRefusal::ColorFormat { depth, mode } => {
            let name = crate::psd_import::color_mode_name(lang, *mode);
            lang.f(
                "lang-only-rgb-8-bit-psds",
                &[("name", &name), ("depth", &depth)],
            )
        }
        CopyRefusal::NoLayers => lang.t("lang-no-layers-a-flattened-image").into(),
        CopyRefusal::TooManyLayers { count, limit } => lang.f(
            "lang-too-many-layers-limit",
            &[("value", &count), ("limit", &limit)],
        ),
        CopyRefusal::CanvasTooLarge { width, height } => lang.f(
            "lang-canvas-too-large",
            &[("width", &width), ("height", &height)],
        ),
        CopyRefusal::LayerTooLarge { layer } => {
            lang.f("lang-has-too-many-pixels", &[("layer", &layer)])
        }
        CopyRefusal::BudgetExceeded { layer } => {
            lang.f("lang-layer-memory-budget-exceeded-at", &[("layer", &layer)])
        }
        CopyRefusal::LayerDataTooLarge { layer } => {
            lang.f("lang-has-too-much-extra-data", &[("layer", &layer)])
        }
        CopyRefusal::EdgeOverLimit {
            width,
            height,
            limit,
        } => lang.f(
            "lang-canvas-too-large-limit",
            &[("width", &width), ("height", &height), ("limit", &limit)],
        ),
        CopyRefusal::LayerCountOverLimit { count, limit } => lang.f(
            "lang-too-many-layers-limit",
            &[("value", &count), ("limit", &limit)],
        ),
        CopyRefusal::NestingTooDeep { limit } => {
            lang.f("lang-groups-are-nested-too-deeply", &[("limit", &limit)])
        }
    }
}

/// 理由のツールチップ（予算で断ったものは、設定で上げられること）。
pub fn psd_copy_refusal_tooltip(lang: Lang, why: &CopyRefusal) -> Option<String> {
    why.raised_by_budget()
        .then(|| lang.t("lang-the-limit-follows-layer-memory-tip").into())
}

/// 更新の失敗の「何が」（`what` は "check" か "download"）。
fn update_head(lang: Lang, what: &str) -> &'static str {
    match what {
        "check" => lang.t("lang-cannot-check-for-updates"),
        _ => lang.t("lang-cannot-download-the-update"),
    }
}

/// 更新の仕事のスレッドを起こせなかった文。
pub(crate) fn update_start_failure(lang: Lang, what: &str, error: &std::io::Error) -> String {
    lang.with_reason(update_head(lang, what), lang.thread_error(error))
}

pub(crate) fn update_failure(lang: Lang, what: &'static str, failure: UpdateFailure) -> String {
    let head = update_head(lang, what);
    let reason = match failure {
        UpdateFailure::Network => lang.t("lang-connection-failed"),
        UpdateFailure::Verify => lang.t("lang-verification-failed"),
        UpdateFailure::NoAsset => lang.t("lang-no-download-for-this-system"),
        UpdateFailure::Disk => lang.t("lang-cannot-save-the-file"),
        UpdateFailure::Stopped => lang.t("lang-the-job-stopped"),
        UpdateFailure::Canceled => return lang.t("lang-download-canceled").into(),
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
            return lang.f("lang-cannot-read-the-image", &[("why", &why)]);
        }
        if why.contains("その画像が無い") {
            return lang.t("lang-the-image-is-not-in").into();
        }
    }
    lang.inactive_reason(&effect.reason)
}

pub(crate) fn hide_preset_save_error(lang: Lang, e: &HideStoreError) -> String {
    lang.with_reason(lang.t("lang-cannot-save-the-hide-set"), e.describe(lang))
}

pub(crate) fn pose_preset_save_error(lang: Lang, e: &PoseStoreError) -> String {
    lang.with_reason(lang.t("lang-cannot-save-the-pose"), e.describe(lang))
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
            CoreError::Unsupported(Reason::CannotPaintFillLayer),
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
            Lang::Ja.core_error(&CoreError::Unsupported(Reason::CannotPaintFillLayer)),
            "塗りつぶしレイヤーには描けない。"
        );
        let error = CoreError::Unsupported(Reason::CannotPaintFillLayer);
        assert_eq!(Lang::En.core_error(&error), "Cannot paint a fill layer.");
        assert_eq!(
            Lang::Ja.core_error(&CoreError::InvalidArgument(Reason::PaintAmountRange)),
            "値が範囲外です（絵の具の量（0〜1））。"
        );
        assert_eq!(
            Lang::En.core_error(&CoreError::InvalidArgument(Reason::PaintAmountRange)),
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
