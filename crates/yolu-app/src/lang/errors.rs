//! core の型と io の分類から、画面用の短い理由を作る。
use super::Lang;
use crate::stencil::StencilError;
use crate::view3d::model::ViewError;
use yolu_core::generator::{self, anchor};
use yolu_core::geometry::GeometryError;
use yolu_core::skin::RigError;
use yolu_core::{CoreError, FallbackEffect, InactiveEffect, InactiveReason, InactiveTarget};
use yolu_model::ModelError;

impl Lang {
    pub fn core_error(self, error: &CoreError) -> String {
        crate::crash::problem(self.core_error_text(error))
    }
    fn core_error_text(self, error: &CoreError) -> String {
        if self == Self::Ja { return error.to_string(); }
        match error {
            CoreError::MergeRefused(reason) => format!("Cannot merge: {}", merge_refusal(*reason)),
            CoreError::MergeAppearance(report) => format!(
                "Merge changes the appearance beyond the tolerance ({})",
                report.max_visible_difference
            ),
            CoreError::Cancelled => "Cancelled".into(),
            CoreError::LayerLocked { .. } => "Layer or parent group is locked".into(),
            CoreError::InactiveEffect { reason, .. } => {
                format!("Cannot bake an inactive effect: {}", self.inactive_reason(reason))
            }
            CoreError::InvalidArgument(what) => format!("Invalid value: {}", core_reason(what)),
            CoreError::Unsupported(what) => format!("Unsupported: {}", core_reason(what)),
            CoreError::LayerNotFound => "Layer not found".into(),
            CoreError::ChannelNotFound => "Channel not found".into(),
            CoreError::StrokeActive => "Stroke in progress".into(),
            CoreError::NoActiveStroke => "Stroke already ended".into(),
            CoreError::SourceBudgetExceeded => "Pixel budget exceeded (cancelled)".into(),
            CoreError::StrokeBudgetExceeded => "Stroke budget exceeded (cancelled)".into(),
            CoreError::WorkingBudgetExceeded => "Working memory budget exceeded".into(),
            CoreError::Clipboard(reason) => clipboard_refusal(*reason).into(),
            CoreError::BatchActive => "Not allowed inside a batch of edits".into(),
        }
    }

    /// .ylp・ファイルの失敗の文。日本語は診断（どの項目か）をそのまま出し、英語は種類ごとの短い文にする
    /// （診断の本文は日本語なので、英語の窓には出さない）。OS のエラーは番号を添える。
    pub fn io_error(self, error: &yolu_io::Error) -> String {
        crate::crash::problem(self.io_error_text(error))
    }
    fn io_error_text(self, error: &yolu_io::Error) -> String {
        use yolu_io::{Error, Unwritable};
        match error {
            Error::Core(e) => self.core_error(e),
            Error::Io(e) => self.file_error(e),
            Error::Json(e) => self.pick(error.to_string(), format!("Invalid JSON: {e}")),
            Error::InvalidData(text) => self.pick(text.clone(), "Invalid or unsupported project data".into()),
            Error::Budget(text) => self.pick(text.clone(), "Size, count or memory limit exceeded".into()),
            Error::Unwritable(what) => self.pick(
                what.to_string(),
                match what {
                    Unwritable::ManualIdColors => "Manual ID colors cannot be saved to .ylp yet".into(),
                },
            ),
            // 保存の衝突のうち、保存先が外で変わったのではない理由（yolu-io の store.rs）は言い分ける。
            // 別の保存がロックを持っているのは待ち、前の版の置き場とロックの場所の不具合は保存先の周りの事情
            Error::SaveConflict(text) if text.contains("進行中") => self.pick(text.clone(), "Another save is in progress".into()),
            Error::SaveConflict(text) if text.contains("バックアップ先がフォルダーではありません") => {
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
            Error::SaveConflict(text) if text.contains("外部で作られました") => {
                self.pick(text.clone(), "A file appeared at the save target; not overwritten".into())
            }
            Error::SaveConflict(text) => self.pick(text.clone(), "Save target or backup changed".into()),
            Error::UnsupportedFormat { format, app, version } => self.pick(
                format!("未対応の .ylp 形式: {format}（{app} {version} で保存。上限 7）"),
                format!("Unsupported .ylp format: {format} (saved by {app} {version}; maximum 7)"),
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
            NotFound => self.pick("ファイルまたはフォルダーがありません", "File or folder not found"),
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
            Some(code) => self.pick(format!("{reason}（OS エラー {code}）"), format!("{reason} (OS error {code})")),
            None => reason.into(),
        }
    }

    /// ステンシルの画像を読めない理由（core の断り・ファイルの失敗は他の窓と同じ文を通す）。
    pub fn stencil_error(self, error: &StencilError) -> String {
        crate::crash::problem(self.stencil_error_text(error))
    }
    fn stencil_error_text(self, error: &StencilError) -> String {
        match error {
            StencilError::Core(e) => self.core_error(e),
            StencilError::File(e) => self.file_error(e),
            StencilError::NotPng => self.pick("PNG として読めません", "Not a readable PNG").into(),
            StencilError::TooLarge { width, height, side } => self.pick(
                format!("画像が大きすぎます（{width} × {height}、1 辺は {side} まで）"),
                format!("Image too large ({width} × {height}; maximum side {side})"),
            ),
            StencilError::Limits => self.pick("画像が大きすぎます（メモリの上限）", "Image too large (memory limit)").into(),
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
            Note::Migrated { format } => format!("Migrated format {format} to the format 7 layout in memory"),
            Note::MaterialRefsMigrated { format } => format!("Migrated format {format} material references to format 7 in memory"),
            Note::UnknownEntryKept(name) => format!("Unsupported entry kept as is: {name}"),
            Note::SetNotConvertible { set, issue } => {
                format!("Texture set “{set}” cannot be converted: {}", issue_key(issue))
            }
            Note::SmartResourceKept(name) => format!("Smart resource “{name}” kept as a file (no preview expansion)"),
            Note::BrushKept => "Brush settings and images kept as is (not validated or executed)".into(),
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
            InactiveReason::Generator(I::StaleMap(k)) => format!("The {k:?} map was baked with other settings"),
            InactiveReason::Generator(I::UnverifiedMap(k)) => format!("The {k:?} map cannot be verified"),
            InactiveReason::Generator(I::MapSize(k)) => format!("The {k:?} map size differs from the texture set"),
            InactiveReason::Generator(I::PinMismatch(k)) => format!("The {k:?} map differs from the pinned bake"),
            InactiveReason::Generator(I::MissingFrame) => "Model root position is unknown".into(),
            InactiveReason::Generator(I::EmptyBounds) => "Position bounds have zero size".into(),
            InactiveReason::Generator(I::NoIdColors) => "No ID colors selected".into(),
            InactiveReason::Generator(I::Anchor(anchor::Issue::NotChosen)) => "No anchor chosen".into(),
            InactiveReason::Generator(I::Anchor(anchor::Issue::Missing)) => "The anchor to read is gone".into(),
            InactiveReason::Generator(I::Anchor(anchor::Issue::NotBelow)) => "The anchor is not below its layer".into(),
            InactiveReason::Rejected(why) if why.is_ascii() => why.clone(),
            InactiveReason::Rejected(_) => "Settings cannot be used".into(),
        }
    }

    /// 効かない効果 1 件の 1 行（日本語は core の文、英語は層の名前・種類・理由）。
    pub fn inactive_effect(self, effect: &InactiveEffect) -> String {
        if self == Self::Ja {
            return effect.to_string();
        }
        let what = match effect.target {
            InactiveTarget::Generator { mask, kind } => {
                format!("{}{}", generator_kind_name(kind), if mask { " (mask)" } else { "" })
            }
            InactiveTarget::FillGradient(c) => format!("gradient ({c:?})"),
            InactiveTarget::Decal => "decal".into(),
            InactiveTarget::FillImage(c) => format!("image ({c:?})"),
        };
        format!("\"{}\" {what}: {}", effect.layer_name, self.inactive_reason(&effect.reason))
    }

    /// 位置のマップが使えなくて UV の空間で評価しているノイズ・グランジの段 1 件の 1 行（日本語は core の文、英語は層の名前・種類・理由）。
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
    pub fn inactive_effects_not_in_composite(self, set: &str, effects: &[InactiveEffect]) -> String {
        let first = effects.first().map(|e| self.inactive_effect(e)).unwrap_or_default();
        match self {
            Self::Ja => format!(
                " 「{set}」の効いていない効果 {} 件は合成の PNG に入っていません: {first}。",
                effects.len()
            ),
            Self::En => format!(
                " {} inactive effect(s) in \"{set}\" are not in the composite PNG: {first}.",
                effects.len()
            ),
        }
    }

    /// 文書を core へ変換できない項目の一覧の文（初めの 3 つと数）。項目のキーは英数字なので、英語の窓にもそのまま出す。
    pub fn unsupported_features(self, issues: &[String]) -> String {
        let shown = issues.iter().take(3).map(|i| self.pick(i.as_str(), issue_key(i))).collect::<Vec<_>>();
        let more = issues.len().saturating_sub(3);
        match self {
            Self::Ja => {
                let mut text = shown.join("、");
                if more > 0 {
                    text += &format!(" ほか {more} 件");
                }
                format!("core で扱えない中身（{text}）")
            }
            Self::En => {
                let mut text = shown.join(", ");
                if more > 0 {
                    text += &format!(" and {more} more");
                }
                format!("Unsupported document features ({text})")
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
    }
}

/// core の理由の文（`&'static str`）の英語。無ければ、英語の文はそのまま、日本語の文は一般の文に落とす。
fn core_reason(reason: &str) -> &str {
    known_core_reason(reason).unwrap_or(if reason.is_ascii() { reason } else { "Unsupported value or operation" })
}

fn known_core_reason(reason: &str) -> Option<&'static str> {
    Some(match reason {
        "マスクへのストロークはチャンネルの合成を読めない" => {
            "A mask stroke cannot read the channel composite"
        }
        "合成の参照元はクローンか色の混ぜの最初のダブの前にだけ決められる" => {
            "The composite source can only be set before the first dab of a clone or color mixing"
        }
        "写像されたダブはクローンか指先だけ" => "Mapped dabs need clone or smudge",
        "写像されたダブはクローン・指先・色の混ぜの伸ばすだけ" => {
            "Mapped dabs need clone, smudge or the smear of color mixing"
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
        "1 区間のデュアルブラシのダブが百万を超える" => "More than one million dual brush dabs per segment",
        "Height → Normal が読むのは Height のチャンネルだけ" => "Height to Normal requires the Height channel",
        "Height → Normal の強さ（±256）" => "Height to Normal strength (±256)",
        "Height の合成は幅 × 高さ × 4 バイト" => "Height buffer size must be width × height × 4 bytes",
        "Normal の合成は幅 × 高さ × 4 バイト" => "Normal buffer size must be width × height × 4 bytes",
        "PassThrough はグループだけ" => "Pass Through requires a group",
        "jitter（0〜1）" => "Jitter (0–1)",
        "purity（−1〜1）" => "Purity (−1–1)",
        "texture depth（0〜1）" => "Texture depth (0–1)",
        "texture scale（0.05〜64）" => "Texture scale (0.05–64)",
        "その番号のチャンネルはもうある" => "Channel ID already exists",
        "ぼかしの半径（1〜64）" => "Blur radius (1–64)",
        "ラスターと塗りつぶし以外には、層の出力の画素が無い" => "Only raster and fill layers have layer output pixels",
        "グループ以外には、グループの出力が無い" => "Only groups have a group output",
        "調整の層ではない" => "Not an adjustment layer",
        "筆圧の曲線（点 2〜16・両端は 0 と 1・間隔 0.02 以上・値 0〜1）" => {
            "Pen pressure curve (2–16 points, ends at 0 and 1, at least 0.02 apart, values 0–1)"
        }
        "筆圧の最小値（0〜1）" => "Pen pressure minimum (0–1)",
        "まとめる層が無い" => "No layers to merge",
        "ガンマ（0.1〜9.99）" => "Gamma (0.1–9.99)",
        "クローンの位置（±1e7）" => "Clone position (±1e7)",
        "グループでない層の中に入っている" => "Parent is not a group",
        "グループではない" => "Not a group",
        "グループにだけ入れられる" => "Parent must be a group",
        "グループには描けない" => "Cannot paint a group",
        "グループの中身が続いていない" => "Group children are not contiguous",
        "グループの入れ子が輪になっている" => "Cyclic group hierarchy",
        "グループを自分の中へは入れられない" => "Cannot move a group into itself",
        "ステンシルに画布からの写しが無いので、2D のダブは読めない" => "Missing canvas-to-stencil transform for 2D dabs",
        "ステンシルに画布からの写しが無い（画素ごとにステンシルの上の点を渡す）" => "Missing canvas-to-stencil transform",
        "ステンシルの footprint" => "Stencil footprint",
        "ステンシルのミップマップが予算を超える（小さい画像にする）" => "Stencil mipmap budget exceeded",
        "ステンシルの点は画素ごとに 1 つ" => "Stencil point count must match pixel count",
        "ステンシルの画像の画素 / 点" => "Stencil image pixels / points",
        "大きさの変更の準備のあとに文書が変わった" => "The document changed after the resize was prepared",
        "ステンシルの点（±1e9）" => "Stencil point (±1e9)",
        "ステンシルの画像のバイト数が幅 × 高さ × 4 でない" => "Stencil buffer size must be width × height × 4 bytes",
        "ステンシルの画像の大きさ（1〜8192）" => "Stencil image size (1–8192)",
        "タイルが画布の外" => "Tile outside canvas",
        "タイルのバイト数が違う" => "Invalid tile byte count",
        "タイルの座標が画布の外" => "Tile coordinates outside canvas",
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
        "写した画素のバイト数が幅 × 高さ × 4 でない" => "Copied pixels must be width × height × 4 bytes",
        "写した文書の大きさ" => "Size of the source document",
        "写した矩形が文書の外" => "Copied rectangle lies outside the document",
        "写した矩形が空" => "Copied rectangle is empty",
        "画素が矩形の外" => "Pixel outside the rectangle",
        "画素を置き換えられるのはラスターの層だけ" => "Only paint layers have pixels to replace",
        "画像の大きさが文書と違う" => "Image size differs from the document",
        "無効のチャンネルは切り取れない" => "Cannot cut a disabled channel",
        "無効のチャンネルは置き換えられない" => "Cannot replace a disabled channel",
        "入れ子が輪になっている" => "Cyclic hierarchy",
        "入力の座標が範囲外" => "Input coordinates out of range",
        "入力の時刻が戻った" => "Input time moved backwards",
        "出力の大きさが矩形と違う" => "Output size does not match rectangle",
        "効果のブラシは消しゴムにできない" => "Effect brushes cannot erase",
        "効果のブラシは画素ごとには塗れない（apply_dab で読み元を凍結する）" => "Effect brushes require dab painting",
        "半径（0〜200）" => "Radius (0–200)",
        "同じグループの中の層だけをまとめられる" => "Merge requires layers in the same group",
        "同じ名前のチャンネルがある" => "Channel name already exists",
        "塗りつぶしの層だけが値を持つ" => "Values require a fill layer",
        "塗りつぶしの層には描けない" => "Cannot paint a fill layer",
        "塗りつぶしはラスターの層だけ" => "Fill requires a raster layer",
        "多角形の点が多すぎる（100000 まで）" => "Too many polygon points (maximum 100000)",
        "大きさ" => "Size",
        "対称の中心（±1e7）" => "Symmetry center (±1e7)",
        "層にマスクが無い" => "Layer has no mask",
        "層はもうマスクを持っている" => "Layer already has a mask",
        "層はグループの下に並ぶ" => "Layers must follow their group",
        "手ぶれ補正・入り抜き（0〜10000）" => "Stabilizer and taper (0–10000)",
        "指先の強さ（0〜1）" => "Smudge strength (0–1)",
        "指先・クローンは対称と組めない（写しごとに読み元と動きが要る）" => "Smudge and clone do not support symmetry",
        "放射状の写しの数（2〜16）" => "Radial symmetry count (2–16)",
        "有効なチャンネルに使えない調整" => "Adjustment not supported by enabled channels",
        "標準のチャンネルは変えられない" => "Cannot change a built-in channel",
        "標準のチャンネルは消せない" => "Cannot remove a built-in channel",
        "無いグループに入っている" => "Parent group not found",
        "無効のチャンネルには塗れない" => "Cannot fill a disabled channel",
        "無効のチャンネルには描けない" => "Cannot paint a disabled channel",
        "画布の外の余白は 0 でなければならない" => "Padding outside canvas must be zero",
        "画素が画布の外" => "Pixel outside canvas",
        "矩形が画布の外" => "Rectangle outside canvas",
        "種が画布の外" => "Seed outside canvas",
        "筆先の並び（1〜256 枚）" => "Brush tip sequence (1–256)",
        "筆先の大きさ（1〜2048）" => "Brush tip size (1–2048)",
        "筆先の覆いの長さが幅 × 高さでない" => "Brush tip coverage size must be width × height",
        "範囲の大きさが文書と違う" => "Region size does not match document",
        "色のゆらぎ（0〜1）" => "Color dynamics (0–1)",
        "色相/彩度の層が有効" => "Hue/Saturation layer enabled",
        "色相/彩度は色のチャンネルだけ" => "Hue/Saturation requires a color channel",
        "色相・彩度・明度" => "Hue, saturation and value",
        "この種類は 8 つの値では組み立てられない（種類ごとの組み立てを使う）" => "This adjustment kind cannot be built from the eight values",
        "調整の種類と値が合わない" => "The adjustment kind and its values do not match",
        "カラーバランス（−100〜100）" => "Color balance (−100–100)",
        "明るさ（−150〜150）・コントラスト（−50〜100）" => "Brightness (−150–150) and contrast (−50–100)",
        "しきい値（1〜255）" => "Threshold (1–255)",
        "階調（2〜255）" => "Posterize levels (2–255)",
        "グラデーションマップとカラーバランスは色のチャンネルだけに適用できます" => "Gradient Map and Color Balance apply only to color channels",
        "接空間法線には再正規化するぼかしだけを適用できます" => "Only the renormalizing blur applies to tangent-space normals",
        "親の数が層の数と違う" => "Parent count does not match layer count",
        "調整の層だけが調整の設定を持つ" => "Adjustment settings require an adjustment layer",
        "調整の層には描けない" => "Cannot paint an adjustment layer",
        "速さの上限（0 より大きい）" => "Maximum speed (greater than zero)",
        "選択範囲のタイルが文書の外" => "Selection tile outside document",
        "選択範囲のタイルが空" => "Empty selection tile",
        "選択範囲のタイルが重なっている" => "Overlapping selection tiles",
        "選択範囲のタイルの余白が 0 でない" => "Selection tile padding must be zero",
        "選択範囲のタイルの大きさ" => "Selection tile size",
        "選択範囲のタイルの長さ" => "Selection tile length",
        "選択範囲の大きさ" => "Selection size",
        "選択範囲の大きさが文書と違う" => "Selection size does not match document",
        "選択範囲の大きさが違う" => "Selection size mismatch",
        "選択範囲を戻せるのは読み込みの直後だけ" => "Selection restore requires a freshly loaded document",
        "面が無い" => "Surface not found",
        "UV 比較の解像度" => "UV comparison resolution",
        "グラデーション" => "Gradient",
        "三角形の座標" => "Triangle coordinates",
        "三角形番号" => "Triangle index",
        "空のマテリアル" => "Empty material",
        "重複したチャンネル" => "Duplicate channel",
        "ID 色の復元は読み込み直後だけ" => "ID colors can only be restored right after loading",
        "グラデーション両端のチャンネル" => "Channels at both ends of the gradient",
        "保存する層がありません" => "No layers to save",
        "保存するマスクがありません" => "No mask to save",
        "空のスマート素材" => "Empty smart material",
        "スマートマスクの断片が不正" => "Invalid smart mask fragment",
        "スマートマスクは層に置けません" => "A smart mask cannot be placed on a layer",
        "配置先がグループではありません" => "The target is not a group",
        "層は2048個までです" => "Maximum 2048 layers",
        "スマートマテリアルはマスクに置けません" => "A smart material cannot be placed on a mask",
        "ユーザーチャンネルの対応が一致しません" => "User channel mapping does not match",
        "ID マップが必要" => "An ID map is required",
        "ID マップの大きさ" => "ID map size",
        "ID の色" => "ID color",
        "スマート素材の名前" => "Smart material name",
        "変形は有限値" => "Transform must be finite",
        "変形が潰れる、または範囲外" => "Transform is degenerate or out of range",
        "動かすラスター層が無い" => "No raster layer to move",
        "画像の辺は 1〜8192" => "Image side (1–8192)",
        "移動先のグループ" => "Destination group",
        "未知のロック" => "Unknown lock",
        "結合は 2 層以上" => "Merging requires at least two layers",
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
        "ジェネレーターは 1 画素に 1 つの値を作るので、接空間の法線には置けない" => "A generator makes one value per pixel and cannot be placed on a tangent-space normal",
        "ジェネレーターはジェネレーターの設定で置く" => "A generator is placed with generator settings",
        "グラデーションの設定" => "Gradient settings",
        "グループの合成へのフィルターは無い" => "Groups have no filters on their composite",
        "このマスクにはもう Anchor がある" => "This mask already has an anchor",
        "この層にはもう Anchor がある" => "This layer already has an anchor",
        "スタックの到達半径の合計が 512 画素を超える" => "Total reach of the stack exceeds 512 pixels",
        "その Anchor が無い" => "Anchor not found",
        "そのチャンネルにグラデーションが無い" => "The channel has no gradient",
        "その層にそのフィルターが無い" => "The layer has no such filter",
        "パスが参照する三角形が無い" => "The path refers to a missing triangle",
        "パスで描かれたチャンネルは無効にできない" => "Cannot disable a channel drawn by a path",
        "パスで描かれた層には手で描けない" => "Cannot paint by hand on a path layer",
        "パスで描けるのはラスターの層だけ" => "Paths require a raster layer",
        "パスのチャンネルが層で有効でない" => "The path channel is not enabled on the layer",
        "パスのチャンネルの面が層に無い" => "The layer has no surface for the path channel",
        "パスを付けられるのはパスの無いラスターの層だけ" => "A path needs a raster layer without a path",
        "フィルターの ID" => "Filter ID",
        "フィルターの ID が空" => "Filter ID is empty",
        "フィルターの ID が空か重なっている" => "Filter ID is empty or duplicated",
        "フィルターの ID が重なっている" => "Filter ID is duplicated",
        "フィルターのチャンネル" => "Filter channels",
        "フィルターのチャンネルが選ばれていない" => "No filter channel selected",
        "フィルターのチャンネルが重なっている" => "Filter channels are duplicated",
        "フィルターの設定" => "Filter settings",
        "ブラシの間隔に対してパスが長すぎる" => "The path is too long for the brush spacing",
        "プロジェクトにその画像が無い" => "The project has no such image",
        "マスクが無い" => "No mask",
        "マスクのフィルターのチャンネル" => "Mask filter channels",
        "マスクのフィルターはチャンネルを持たない" => "Mask filters have no channels",
        "マスクのフィルターはチャンネルを持たない（マスクは全チャンネルで共有）" => "Mask filters have no channels (a mask is shared by all channels)",
        "マスクの無い層のマスクには Anchor を置けない" => "Cannot place a mask anchor on a layer without a mask",
        "マップの境界箱" => "Map bounding box",
        "マップの大きさと長さが合わない" => "Map size and length do not match",
        "マップの幅" => "Map width",
        "マップの条件の鍵" => "Map condition key",
        "マップの高さ" => "Map height",
        "メッシュマップの種類" => "Mesh map kind",
        "モデルの位置・回転" => "Model position and rotation",
        "モデルの指紋がパスを作ったときと違う" => "The model fingerprint differs from when the path was made",
        "予算が今のフィルターの要る量より小さい" => "Budget is smaller than the current filters need",
        "効果（フィルター・画像・グラデーション）は標準のチャンネルだけに置ける" => "Effects (filters, images, gradients) can only be placed on standard channels",
        "塗りつぶしのグラデーションのチャンネル" => "Fill gradient channel",
        "塗りつぶしのグラデーションはランプ付きの形のグラデーション" => "A fill gradient is a shape gradient with a ramp",
        "塗りつぶしのグラデーションは置き換え" => "A fill gradient replaces",
        "塗りつぶしの入力" => "Fill input",
        "塗りつぶしの層だけが持つ" => "Only fill layers have this",
        "塗りつぶしの画像のチャンネルか ID" => "Fill image channel or ID",
        "層のパスはチャンネルを変えない" => "A layer path does not change its channel",
        "層のパスは種類（モデルの上かキャンバスの上か）を変えない" => "A layer path does not change its kind (model or canvas)",
        "形のグラデーションは色かスカラーで、法線ではない" => "A shape gradient is a color or scalar, not a normal",
        "投影の値" => "Projection values",
        "描いた面のチャンネルが重なっている" => "Painted surface channels are duplicated",
        "描いた面は、パスのチャンネルごとに、文書と同じ大きさで 1 つ" => "One painted surface per path channel, at the document size",
        "画像の ID が空" => "Image ID is empty",
        "画像の大きさ" => "Image size",
        "画像の大きさと画素の長さ" => "Image size and pixel length do not match",
        "自分の層の Anchor を読むジェネレーター（値が自分に戻る）" => "A generator reading an anchor on its own layer (the value would feed back)",
        "調整の層には画素が無い" => "Adjustment layers have no pixels",
        "面のダブを拒否した" => "The surface dab was refused",
        "見た目の設定の復元は読み込み直後だけ" => "Look settings can only be restored right after loading",
        "見た目のシェーダーの名前" => "Look shader name",
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
            SurfaceStrokeError::TooManyDabs => self.pick("ダブの上限を超えました（取り消した）", "Dab limit exceeded (cancelled)").into(),
            SurfaceStrokeError::Sampling(e) => self.sampling_error(*e).into(),
            SurfaceStrokeError::EffectWithSymmetry => self
                .pick("指先・クローンでは対称を使えません", "Smudge and clone do not work with symmetry")
                .into(),
            SurfaceStrokeError::CloneSource => self
                .pick("クローンの元が今のモデルの面ではありません", "The clone source is not on this model")
                .into(),
        }
    }
    /// 指先・クローンの読み元を決められなかった理由。
    pub fn sampling_error(self, error: yolu_core::geometry::SamplingError) -> &'static str {
        crate::crash::problem(self.sampling_error_text(error))
    }
    fn sampling_error_text(self, error: yolu_core::geometry::SamplingError) -> &'static str {
        use yolu_core::geometry::SamplingError::*;
        match error {
            SnapshotChanged => self.pick(
                "モデルのスナップショットが変わりました",
                "Model snapshot changed",
            ),
            BindingMismatch => self.pick(
                "面がスナップショットと合いません",
                "Surface binding mismatch",
            ),
            InvalidArguments => self.pick(
                "参照の半径か位置が範囲外です",
                "Invalid sampling radius or position",
            ),
            ChartBudget => self.pick(
                "参照を読む面の予算を超えました（取り消した）",
                "Sampling surface budget exceeded (cancelled)",
            ),
            LookupBudget => self.pick(
                "参照を探す回数の予算を超えました（取り消した）",
                "Sampling lookup budget exceeded (cancelled)",
            ),
            Unreachable => self.pick(
                "このダブの画素が参照の図に入りません（取り消した）",
                "A dab pixel is outside the sampling chart (cancelled)",
            ),
        }
    }
    /// 対称の写しが塗られなかった理由（短い状態）。
    pub fn mirror_note(self, outcome: yolu_core::geometry::MirrorOutcome) -> &'static str {
        use yolu_core::geometry::MirrorOutcome::*;
        match outcome {
            NoSurface => self.pick(
                "対称: 近くに面が無い写しは飛ばしました",
                "Symmetry: copies with no surface nearby were skipped",
            ),
            OtherSlot => self.pick(
                "対称: 別のテクスチャセットの写しは飛ばしました",
                "Symmetry: copies on another texture set were skipped",
            ),
            Hidden => self.pick(
                "対称: 見えない写しは飛ばしました",
                "Symmetry: copies that cannot be seen were skipped",
            ),
            Painted | OnPlane => "",
        }
    }
    /// 指先が、つながらない面で拾い直したときの状態。
    pub fn smudge_lost(self) -> &'static str {
        self.pick(
            "指先: つながらない面で拾い直しました",
            "Smudge picked up again on a disconnected surface",
        )
    }
    pub fn dab_refusal(self, error: yolu_core::geometry::DabRefusal) -> &'static str {
        crate::crash::problem(self.dab_refusal_text(error))
    }
    fn dab_refusal_text(self, error: yolu_core::geometry::DabRefusal) -> &'static str {
        use yolu_core::geometry::DabRefusal::*;
        match error {
            SnapshotChanged => self.pick("モデルのスナップショットが変わりました", "Model snapshot changed"),
            InvalidArguments => self.pick("ブラシの大きさ・解像度・カメラが範囲外です", "Invalid brush size, resolution or camera"),
            BindingMismatch => self.pick("面がスナップショットと合いません", "Surface binding mismatch"),
            TriangleBudget => self.pick("三角形の予算を超えました（取り消した）", "Triangle budget exceeded (cancelled)"),
            PixelBudget => self.pick("画素の予算を超えました（取り消した）", "Pixel budget exceeded (cancelled)"),
            VisibilityBudget => self.pick("遮蔽のレイの予算を超えました（取り消した）", "Visibility ray budget exceeded (cancelled)"),
            BvhBudget => self.pick("BVH の予算を超えました（取り消した）", "BVH work budget exceeded (cancelled)"),
        }
    }
}

impl Lang {
    /// 3D ビュー・ポーズの失敗の文。
    pub fn view_error(self, error: &ViewError) -> String {
        crate::crash::problem(self.view_error_text(error))
    }
    fn view_error_text(self, error: &ViewError) -> String {
        if self == Self::Ja {
            return error.to_string();
        }
        match error {
            ViewError::Stroking => "Cannot change the pose during a stroke.".into(),
            ViewError::NoPoseModel => "No model to pose".into(),
            ViewError::NoPoseEdit => "Pose edit not started".into(),
            ViewError::NoLinkModel => "Pose received before model".into(),
            ViewError::NoPoseBase => "No model to apply the pose to".into(),
            ViewError::BadMeshIndex => "Mesh index exceeds the vertex count".into(),
            ViewError::NoTriangles => "No triangles".into(),
            ViewError::Cancelled => "Cancelled".into(),
            ViewError::LoadStopped => "Loading stopped".into(),
            ViewError::PoseGeneration { pose, model: Some(model) } => {
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
            GeometryError::NonFinite => self.pick("メッシュの位置か UV に有限でない値があります", "Non-finite position or UV in the mesh"),
            GeometryError::BoundsOverflow => self.pick("メッシュの大きさが扱える範囲を超えています", "Mesh bounds exceed the supported range"),
            GeometryError::InvalidTolerance => self.pick("溶接の許しは正の値でなければなりません", "Weld tolerance must be positive"),
            GeometryError::TooManyTriangles => self.pick("三角形が多すぎます", "Too many triangles"),
            GeometryError::Canceled => self.pick("取り消しました", "Cancelled"),
            GeometryError::Mismatch => self.pick("三角形の並びが元のスナップショットと違います", "Triangle order differs from the snapshot"),
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
        if self == Self::Ja {
            return error.to_string();
        }
        match error {
            ModelError::Io(e) => format!("Cannot read the file: {e}"),
            ModelError::FileTooLarge { bytes, limit } => format!(
                "File too large ({:.1} MiB, maximum {:.0} MiB)",
                *bytes as f64 / 1048576.0,
                *limit as f64 / 1048576.0
            ),
            ModelError::Parse(e) => format!("Invalid FBX: {e}"),
            ModelError::NoMesh => "No triangle mesh".into(),
            ModelError::Rig(e) => self.rig_error(e),
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
        "スキンの行列・ウェイト（負のウェイトを含む）" => "skin matrices and weights",
        _ if what.is_ascii() => what,
        _ => "values",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn merge_and_lock_errors_use_the_selected_language() {
        use yolu_core::MergeRefusal::*;
        use yolu_core::{LayerId, LayerLocks, LayerMergeReport, MergeMethod};
        let mut errors: Vec<CoreError> = [NoLayerBelow, LayerBelowIsGroup, LayerBelowIsAdjustment, HiddenLayer, IsGroup, NotGroup, EmptyGroup, NothingVisible, DifferentGroups]
            .into_iter()
            .map(CoreError::MergeRefused)
            .collect();
        errors.push(CoreError::Cancelled);
        errors.push(CoreError::LayerLocked { layer: LayerId(1), holder: LayerId(2), lock: LayerLocks::ALL });
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
            assert_eq!(Lang::Ja.core_error(error), error.to_string());
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
            InactiveReason::Rejected("ASCII reason".into()),
            InactiveReason::Rejected("範囲外の値".into()),
        ];
        let english: Vec<String> = reasons.iter().map(|r| Lang::En.inactive_reason(r)).collect();
        for (i, (reason, en)) in reasons.iter().zip(&english).enumerate() {
            assert_eq!(Lang::Ja.inactive_reason(reason), reason.to_string());
            assert!(en.is_ascii() && !en.is_empty(), "{en}");
            assert!(english[i + 1..].iter().all(|other| other != en), "{en}");
        }
        assert!(english[0].contains("Thickness") && english[4].contains("pinned"));
        // 結合の断りの文は、断った理由を言語ごとに運ぶ
        let refused = CoreError::InactiveEffect { layer: LayerId(1), mask: false, reason: Box::new(reasons[0].clone()) };
        assert_eq!(Lang::Ja.core_error(&refused), refused.to_string());
        assert_eq!(Lang::En.core_error(&refused), "Cannot bake an inactive effect: No Thickness map");
        // 1 行の文: 層の名前は利用者の文字列なのでそのまま、種類と理由は英語
        let effects = [
            InactiveEffect {
                layer: LayerId(1),
                layer_name: "Top".into(),
                target: InactiveTarget::Generator { mask: false, kind: Kind::EdgeWear },
                reason: reasons[0].clone(),
            },
            InactiveEffect {
                layer: LayerId(1),
                layer_name: "Top".into(),
                target: InactiveTarget::Generator { mask: true, kind: Kind::Anchor },
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
        let lines: Vec<String> = effects.iter().map(|e| Lang::En.inactive_effect(e)).collect();
        for (i, (effect, line)) in effects.iter().zip(&lines).enumerate() {
            assert_eq!(Lang::Ja.inactive_effect(effect), effect.to_string());
            assert!(line.is_ascii() && line.contains(&effect.layer_name), "{line}");
            assert!(lines[i + 1..].iter().all(|other| other != line), "{line}");
        }
        assert_eq!(lines[0], "\"Top\" Edge wear: No Thickness map");
        assert_eq!(lines[1], "\"Top\" Anchor (mask): No anchor chosen");
        assert!(lines[2].contains("gradient (Color)") && lines[3].contains("decal") && lines[4].contains("image (Roughness)"));
        // 保存の知らせは、書き直したセットの名前と件数と初めの理由
        let ja = Lang::Ja.inactive_effects_not_in_composite("Skin", &effects);
        assert!(ja.contains("「Skin」") && ja.contains("5 件") && ja.contains("合成の PNG に入っていません") && ja.contains(&effects[0].to_string()), "{ja}");
        let en = Lang::En.inactive_effects_not_in_composite("Skin", &effects);
        assert_eq!(en, format!(" 5 inactive effect(s) in \"Skin\" are not in the composite PNG: {}.", lines[0]));
    }

    /// 位置のマップが使えなくて UV の空間で評価しているノイズ・グランジの 1 行が、画面の言語で出る（層の名前はそのまま）。
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
        let lines: Vec<String> = effects.iter().map(|e| Lang::En.fallback_effect(e)).collect();
        for (i, (effect, line)) in effects.iter().zip(&lines).enumerate() {
            let ja = Lang::Ja.fallback_effect(effect);
            assert_eq!(ja, effect.to_string());
            assert!(ja.contains("UV の空間") && ja.contains("「Top」"), "{ja}");
            assert!(line.is_ascii() && line.contains("\"Top\"") && line.contains("UV space"), "{line}");
            assert!(lines[i + 1..].iter().all(|other| other != line), "{line}");
        }
        assert!(lines[0].contains("Noise") && !lines[0].contains("(mask)") && lines[0].contains("No Position map"), "{}", lines[0]);
        assert!(lines[1].contains("Grunge (mask)") && lines[1].contains("baked with other settings"), "{}", lines[1]);
        assert!(Lang::Ja.fallback_effect(&effects[1]).contains("（マスク）"));
    }

    #[test]
    fn editing_errors_use_the_selected_language() {
        for error in [CoreError::LayerNotFound, CoreError::ChannelNotFound, CoreError::StrokeActive, CoreError::NoActiveStroke, CoreError::SourceBudgetExceeded, CoreError::StrokeBudgetExceeded, CoreError::WorkingBudgetExceeded, CoreError::Unsupported("塗りつぶしの層には描けない")] {
            assert_eq!(Lang::Ja.core_error(&error), error.to_string());
            assert!(Lang::En.core_error(&error).is_ascii());
        }
        let error = CoreError::Unsupported("塗りつぶしの層には描けない");
        assert_eq!(Lang::En.core_error(&error), "Unsupported: Cannot paint a fill layer");
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
        .chain([CoreError::BatchActive])
        .collect();
        let english: Vec<String> = errors.iter().map(|e| Lang::En.core_error(e)).collect();
        for (i, (error, en)) in errors.iter().zip(&english).enumerate() {
            assert_eq!(Lang::Ja.core_error(error), error.to_string());
            assert!(en.is_ascii() && !en.is_empty(), "{en}");
            assert!(english[i + 1..].iter().all(|other| other != en), "{en}");
            // 画面に出す理由に、開発用の数（バイト・MiB）は入れない
            assert!(!error.to_string().chars().any(|c| c.is_ascii_digit()), "{error}");
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
        sources(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../yolu-core/src"), &mut files);
        assert!(files.len() > 10);
        let (mut found, mut missing) = (0, Vec::new());
        for file in files {
            let text = std::fs::read_to_string(&file).unwrap();
            for head in ["CoreError::InvalidArgument(", "CoreError::Unsupported("] {
                for (at, _) in text.match_indices(head) {
                    let rest = text[at + head.len()..].trim_start();
                    let Some(rest) = rest.strip_prefix('"') else { continue };
                    let reason = &rest[..rest.find('"').unwrap()];
                    found += 1;
                    if known_core_reason(reason).is_none() && !reason.is_ascii() {
                        missing.push(format!("{}: {reason}", file.file_name().unwrap().to_string_lossy()));
                    }
                }
            }
        }
        assert!(found > 100, "{found}");
        assert!(missing.is_empty(), "英語の文が無い理由: {missing:#?}");
    }

    #[test]
    fn view_and_surface_errors_are_translated_by_kind() {
        use yolu_core::geometry::{DabRefusal, SurfaceStrokeError};
        let rig = RigError::TooLarge { what: "ボーン", value: 2, limit: 1 };
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
            ViewError::PoseGeneration { pose: 2, model: Some(1) },
            ViewError::PoseGeneration { pose: 2, model: None },
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
            ViewError::Model(ModelError::FileTooLarge { bytes: 3 << 20, limit: 1 << 20 }),
            ViewError::Model(ModelError::Parse("bad".into())),
            ViewError::Model(ModelError::NoMesh),
            ViewError::Model(ModelError::Rig(rig)),
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
            assert_eq!(ja, e.to_string());
            assert!(en.is_ascii() && !en.is_empty(), "{en}");
            assert_ne!(ja, en);
        }
        assert_eq!(Lang::En.view_error(&ViewError::Model(ModelError::Rig(RigError::TooLarge { what: "ボーン", value: 2, limit: 1 }))), "Too many bones (2, maximum 1)");
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
        for d in [
            DabRefusal::SnapshotChanged,
            DabRefusal::InvalidArguments,
            DabRefusal::BindingMismatch,
            DabRefusal::TriangleBudget,
            DabRefusal::PixelBudget,
            DabRefusal::VisibilityBudget,
            DabRefusal::BvhBudget,
        ] {
            dabs.push(SurfaceStrokeError::Dab(d));
        }
        for e in &dabs {
            let (ja, en) = (Lang::Ja.surface_error(e), Lang::En.surface_error(e));
            assert!(en.is_ascii() && !en.is_empty() && ja != en, "{en}");
        }
    }

    /// 種類の違う失敗は、日英どちらでも別々の文になる（予算超過・壊れたデータ・まだ書けない中身・衝突・ファイルの失敗）。
    #[test]
    fn io_errors_are_told_apart_by_kind_in_both_languages() {
        use yolu_io::{Error, Unwritable};
        let errors = [
            Error::InvalidData("正本が不正".into()),
            Error::Budget("アーカイブの予算超過です".into()),
            Error::Unwritable(Unwritable::ManualIdColors),
            Error::SaveConflict("保存先が外部で変更されています".into()),
            Error::UnsupportedFormat { format: 99, app: "FuturePainter".into(), version: "9.0".into() },
            Error::from(std::io::Error::from(std::io::ErrorKind::PermissionDenied)),
            Error::from(CoreError::StrokeActive),
            Error::from(CoreError::SourceBudgetExceeded),
            Error::SaveConflict("別の保存が進行中です".into()),
            Error::SaveConflict("バックアップ先がフォルダーではありません".into()),
            Error::SaveConflict("バックアップ先がシンボリックリンクです".into()),
            Error::SaveConflict("ロックのファイルがシンボリックリンクです".into()),
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
        assert!(Lang::Ja.io_error(&errors[1]).contains("アーカイブの予算超過"));
        assert!(Lang::Ja.io_error(&errors[2]).contains("ID の色"));
        assert!(Lang::En.io_error(&errors[1]).contains("limit exceeded"));
        assert!(Lang::En.io_error(&errors[2]).contains("Manual ID colors"));
        // 別の保存が進行中の衝突は、外で変わった衝突と日英どちらでも言い分ける
        assert_eq!(Lang::En.io_error(&errors[8]), "Another save is in progress");
        assert!(Lang::En.io_error(&errors[3]).contains("changed"));
        assert!(Lang::Ja.io_error(&errors[8]).contains("進行中"));
        // 保存先の周りの不具合は、データの不正（InvalidData の汎用文）にも「外で変わった」にも見せず、場所が理由だと言う
        for (i, key) in [(9, "Backup location is not a folder"), (10, "link"), (11, "lock file")] {
            let en = Lang::En.io_error(&errors[i]);
            assert!(en.contains(key) && !en.contains("changed") && !en.contains("Invalid"), "{en}");
        }
    }

    /// 保存が実際に返す「置き場の不具合」の理由が、英語の窓でも言い分けられる（理由の文を書き換えて、表の対応が外れても気づく）。
    #[test]
    fn real_save_refusals_about_the_place_are_told_in_english() {
        use yolu_io::{Error, SaveTarget};
        let dir = std::env::temp_dir().join(format!("yolu-app-save-places-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let sample = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../yolu-io/tests/fixtures/format1.ylp"));
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
    fn real_save_refusals_about_the_name_and_a_target_created_elsewhere_are_told_in_both_languages() {
        use yolu_io::{Error, Project, SaveTarget};
        let dir = std::env::temp_dir().join(format!("yolu-app-save-names-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let sample = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../yolu-io/tests/fixtures/format1.ylp"));
        let project = Project::read(sample).unwrap();
        let error = SaveTarget::create(dir.join("a.txt")).unwrap_err();
        assert!(matches!(error, Error::SaveConflict(_)), "{error:?}");
        assert!(Lang::Ja.io_error(&error).contains(".ylp") && !Lang::Ja.io_error(&error).is_ascii());
        assert_eq!(Lang::En.io_error(&error), "The file name must end with .ylp");
        let mut target = SaveTarget::create(dir.join("b.ylp")).unwrap();
        std::fs::write(dir.join("b.ylp"), b"made elsewhere").unwrap();
        let error = target.save(&project).unwrap_err();
        assert!(Lang::Ja.io_error(&error).contains("外部で作られました"), "{}", Lang::Ja.io_error(&error));
        assert_eq!(Lang::En.io_error(&error), "A file appeared at the save target; not overwritten");
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
            let texts: Vec<String> = kinds.iter().map(|k| lang.file_error(&Error::from(*k))).collect();
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
            Note::SetNotConvertible { set: "Skin".into(), issue: "layers[2].filters（フィルター・ジェネレーター）".into() },
            Note::SmartResourceKept("Rust".into()),
            Note::BrushKept,
        ];
        let english: Vec<String> = notes.iter().map(|n| Lang::En.project_note(n)).collect();
        for (i, (note, en)) in notes.iter().zip(&english).enumerate() {
            assert_eq!(Lang::Ja.project_note(note), note.to_string());
            assert!(en.chars().all(|c| (c as u32) < 0x3000) && !en.is_empty(), "{en}");
            assert!(english[i + 1..].iter().all(|other| other != en));
        }
        // 件数だけでなく、名前・形式・項目のキーが読める
        assert!(english[1].contains('2') && english[2].contains('3'));
        assert!(english[3].contains("future.bin") && english[5].contains("Rust"));
        assert!(english[4].contains("Skin") && english[4].contains("layers[2].filters") && !english[4].contains("ジェネレーター"));

        let issues: Vec<String> = ["layers[0].locks（ロック）", "manual_id_colors（手動の ID 色）", "layers[1].filters（フィルター・ジェネレーター）", "layers[2].anchor（Anchor）", "layers[3].anchor（Anchor）"]
            .into_iter()
            .map(String::from)
            .collect();
        let ja = Lang::Ja.unsupported_features(&issues);
        assert!(ja.contains("ロック") && ja.contains("ほか 2 件"), "{ja}");
        let en = Lang::En.unsupported_features(&issues);
        assert_eq!(en, "Unsupported document features (layers[0].locks, manual_id_colors, layers[1].filters and 2 more)");
    }
}
