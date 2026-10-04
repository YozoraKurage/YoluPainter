//! core の型と io の分類から、画面用の短い理由を作る。
use super::Lang;
use crate::view3d::model::ViewError;
use yolu_core::geometry::GeometryError;
use yolu_core::skin::RigError;
use yolu_core::CoreError;
use yolu_model::ModelError;

impl Lang {
    pub fn core_error(self, error: &CoreError) -> String {
        if self == Self::Ja { return error.to_string(); }
        match error {
            CoreError::MergeRefused(reason) => format!("Cannot merge: {}", merge_refusal(*reason)),
            CoreError::MergeAppearance(report) => format!(
                "Merge changes the appearance beyond the tolerance ({})",
                report.max_visible_difference
            ),
            CoreError::Cancelled => "Cancelled".into(),
            CoreError::LayerLocked { .. } => "Layer or parent group is locked".into(),
            CoreError::InvalidArgument(what) => format!("Invalid value: {}", core_reason(what)),
            CoreError::Unsupported(what) => format!("Unsupported: {}", core_reason(what)),
            CoreError::LayerNotFound => "Layer not found".into(),
            CoreError::ChannelNotFound => "Channel not found".into(),
            CoreError::StrokeActive => "Stroke in progress".into(),
            CoreError::NoActiveStroke => "Stroke already ended".into(),
            CoreError::SourceBudgetExceeded => "Pixel budget exceeded (cancelled)".into(),
            CoreError::StrokeBudgetExceeded => "Stroke budget exceeded (cancelled)".into(),
            CoreError::WorkingBudgetExceeded => "Working memory budget exceeded".into(),
        }
    }

    /// .ylp・ファイルの失敗の文。日本語は診断（どの項目か）をそのまま出し、英語は種類ごとの短い文にする
    /// （診断の本文は日本語なので、英語の窓には出さない）。OS のエラーは番号を添える。
    pub fn io_error(self, error: &yolu_io::Error) -> String {
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
                    Unwritable::LayerLocks => "Layer locks cannot be saved to .ylp yet".into(),
                },
            ),
            Error::SaveConflict(text) => self.pick(text.clone(), "Save target or backup changed".into()),
            Error::UnsupportedFormat { format, app, version } => self.pick(
                format!("未対応の .ylp 形式: {format}（{app} {version} で保存。上限 7）"),
                format!("Unsupported .ylp format: {format} (saved by {app} {version}; maximum 7)"),
            ),
        }
    }

    /// 読み書きの失敗の文。種類で言い分け、OS のエラー番号（共有違反・空き不足などの手掛かり）を添える。
    pub fn file_error(self, error: &std::io::Error) -> String {
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
        _ => return None,
    })
}

impl Lang {
    pub fn surface_error(self, error: &yolu_core::geometry::SurfaceStrokeError) -> String {
        use yolu_core::geometry::SurfaceStrokeError;
        match error {
            SurfaceStrokeError::Core(e) => self.core_error(e),
            SurfaceStrokeError::Dab(e) => self.dab_refusal(*e).into(),
            SurfaceStrokeError::TooManyDabs => self.pick("ダブの上限を超えました（取り消した）", "Dab limit exceeded (cancelled)").into(),
        }
    }
    pub fn dab_refusal(self, error: yolu_core::geometry::DabRefusal) -> &'static str {
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
        match error {
            GeometryError::NonFinite => self.pick("メッシュの位置か UV に有限でない値があります", "Non-finite position or UV in the mesh"),
            GeometryError::BoundsOverflow => self.pick("メッシュの大きさが扱える範囲を超えています", "Mesh bounds exceed the supported range"),
            GeometryError::InvalidTolerance => self.pick("溶接の許しは正の値にしてください", "Weld tolerance must be positive"),
            GeometryError::TooManyTriangles => self.pick("三角形が多すぎます", "Too many triangles"),
            GeometryError::Canceled => self.pick("取り消しました", "Cancelled"),
            GeometryError::Mismatch => self.pick("三角形の並びが元のスナップショットと違います", "Triangle order differs from the snapshot"),
        }
    }

    pub fn model_error(self, error: &ModelError) -> String {
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
        "骨" => "bones",
        "メッシュ" => "meshes",
        "頂点" => "vertices",
        "三角形" => "triangles",
        "ウェイト" => "weights",
        "BlendShape" => "BlendShapes",
        "BlendShape の差分" => "BlendShape offsets",
        "1 つの頂点のウェイト" => "weights per vertex",
        "骨の変換" => "bone transforms",
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
        let rig = RigError::TooLarge { what: "骨", value: 2, limit: 1 };
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
        assert_eq!(Lang::En.view_error(&ViewError::Model(ModelError::Rig(RigError::TooLarge { what: "骨", value: 2, limit: 1 }))), "Too many bones (2, maximum 1)");
        let mut dabs = vec![SurfaceStrokeError::TooManyDabs, SurfaceStrokeError::Core(CoreError::StrokeActive)];
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
            Error::Unwritable(Unwritable::LayerLocks),
            Error::SaveConflict("保存先が外部で変更されています".into()),
            Error::UnsupportedFormat { format: 99, app: "FuturePainter".into(), version: "9.0".into() },
            Error::from(std::io::Error::from(std::io::ErrorKind::PermissionDenied)),
            Error::from(CoreError::StrokeActive),
            Error::from(CoreError::SourceBudgetExceeded),
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
        assert!(Lang::Ja.io_error(&errors[3]).contains("ロック"));
        assert!(Lang::En.io_error(&errors[1]).contains("limit exceeded"));
        assert!(Lang::En.io_error(&errors[2]).contains("Manual ID colors"));
        assert!(Lang::En.io_error(&errors[3]).contains("Layer locks"));
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
            Note::SetNotConvertible { set: "Skin".into(), issue: "layers[2].filters（フィルター・Generator）".into() },
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
        assert!(english[4].contains("Skin") && english[4].contains("layers[2].filters") && !english[4].contains("Generator"));

        let issues: Vec<String> = ["layers[0].locks（ロック）", "manual_id_colors（手動の ID 色）", "layers[1].filters（フィルター・Generator）", "layers[2].anchor（Anchor）", "layers[3].anchor（Anchor）"]
            .into_iter()
            .map(String::from)
            .collect();
        let ja = Lang::Ja.unsupported_features(&issues);
        assert!(ja.contains("ロック") && ja.contains("ほか 2 件"), "{ja}");
        let en = Lang::En.unsupported_features(&issues);
        assert_eq!(en, "Unsupported document features (layers[0].locks, manual_id_colors, layers[1].filters and 2 more)");
    }
}
