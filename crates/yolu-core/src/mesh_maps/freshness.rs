//! 保存してあるメッシュマップが今の条件で焼いたものかの照合（C# の `MeshMapProvenance.Check` と `MeshMapExpectation`）。
//! 古い・照合できないマップは黙って使わない。使う側はこの結果が `Current` のときだけマップを使う。
use super::{
    bake::{material_identity, ENGINE_VERSION, POSE, SPACE},
    MeshBakeInput, MeshBakeSettings, MeshMapProvenance,
};
use std::fmt;

/// マップを使う側の今の条件。`mesh_hash` が `None` なら照合できない（モデルが読み込まれていない）。
#[derive(Clone, Debug)]
pub struct MeshMapExpectation {
    pub mesh_hash: Option<String>,
    pub topology_hash: Option<String>,
    /// 高ポリの指紋。高ポリを使わないなら `None`。
    pub reference_hash: Option<String>,
    pub width: i32,
    pub height: i32,
    pub target_slot: i32,
    /// テクスチャセットが受け持つスロットの並び（昇順）。空なら `target_slot` 1 つ
    /// （-1 は全部、-2 以下は「モデルに無い」でどのマップとも合わない）。
    pub target_slots: Vec<i32>,
    pub uv_channel: i32,
    /// 渡すと、その種類の設定・余白・アンチエイリアス・高ポリの投影の設定が違うマップも古いとみなす。
    pub settings: Option<MeshBakeSettings>,
    /// `settings` を渡すとき、MaterialAsset の ID マップの素材の識別（`material_identity_key`）。
    pub material_identity: String,
}
impl MeshMapExpectation {
    /// 入力と設定から、焼き直したら同じになる条件を作る。
    pub fn for_bake(
        input: &MeshBakeInput,
        settings: &MeshBakeSettings,
        reference: Option<&MeshBakeInput>,
    ) -> Self {
        Self {
            mesh_hash: Some(input.hash.clone()),
            topology_hash: Some(input.topology_hash.clone()),
            reference_hash: reference.map(|r| r.hash.clone()),
            width: settings.width,
            height: settings.height,
            target_slot: settings.target_slot,
            target_slots: settings.target_slots.clone(),
            uv_channel: input.uv_channel,
            settings: Some(settings.clone()),
            material_identity: material_identity(input, reference),
        }
    }
    fn targets(&self) -> Vec<i32> {
        if !self.target_slots.is_empty() {
            self.target_slots.clone()
        } else if self.target_slot == -1 {
            vec![]
        } else {
            vec![self.target_slot]
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeshMapState {
    /// 今の条件で焼いたもの。
    Current,
    /// 条件が変わった（理由は `MeshMapCheck::reasons`）。使わずに焼き直す。
    Stale,
    /// 照合できない（モデルが無い）。使わない。
    Unverified,
}
/// 古い理由。並びは C# の照合と同じ。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MeshMapStaleReason {
    EngineVersion {
        baked: i32,
        current: i32,
    },
    SpaceOrPose {
        space: String,
        pose: String,
    },
    /// 高ポリ無しで焼いたが、今は高ポリがある。
    ReferenceAdded,
    /// 高ポリで焼いたが、今は高ポリが無い。
    ReferenceRemoved,
    /// 高ポリか投影の設定が変わった。
    ReferenceChanged,
    Size {
        baked: (i32, i32),
        now: (i32, i32),
    },
    /// テクスチャセットのマテリアルが読み込んだモデルに無い。
    MaterialNotInModel,
    Slots {
        baked: Vec<i32>,
        now: Vec<i32>,
    },
    UvChannel {
        baked: i32,
        now: i32,
    },
    Padding {
        baked: i32,
        now: i32,
    },
    Antialiasing {
        baked: i32,
        now: i32,
    },
    /// その種類の設定（`MeshBakeSettings::kind_key`）が変わった。
    Settings {
        baked: String,
        now: String,
    },
    /// 三角形・UV は同じで、頂点か法線が動いた（ポーズ・BlendShape・編集）。静的な基準の姿勢のマップは自動では焼き直さない。
    ShapeChanged,
    /// 三角形・UV・スロットが変わった。
    ModelChanged,
}
impl MeshMapStaleReason {
    /// 機械が読む短い記号（C# の照合との突き合わせに使う）。
    pub fn code(&self) -> &'static str {
        match self {
            Self::EngineVersion { .. } => "engine",
            Self::SpaceOrPose { .. } => "space_pose",
            Self::ReferenceAdded => "reference_added",
            Self::ReferenceRemoved => "reference_removed",
            Self::ReferenceChanged => "reference_changed",
            Self::Size { .. } => "size",
            Self::MaterialNotInModel => "material_not_in_model",
            Self::Slots { .. } => "slots",
            Self::UvChannel { .. } => "uv_channel",
            Self::Padding { .. } => "padding",
            Self::Antialiasing { .. } => "antialiasing",
            Self::Settings { .. } => "settings",
            Self::ShapeChanged => "shape_changed",
            Self::ModelChanged => "model_changed",
        }
    }
}
fn slots_text(slots: &[i32]) -> String {
    if slots.is_empty() {
        "全部".into()
    } else {
        slots
            .iter()
            .map(i32::to_string)
            .collect::<Vec<_>>()
            .join("、")
    }
}
impl fmt::Display for MeshMapStaleReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EngineVersion { baked, current } => {
                write!(f, "メッシュマップのエンジンの版 {baked}（今は {current}）")
            }
            Self::SpaceOrPose { space, pose } => {
                write!(f, "空間「{space}」・姿勢「{pose}」で焼いたもの（この版は作らない）")
            }
            Self::ReferenceAdded => f.write_str("高ポリ無しで焼いたが、今は高ポリを選んでいる"),
            Self::ReferenceRemoved => f.write_str("高ポリで焼いたが、今は高ポリを選んでいない"),
            Self::ReferenceChanged => f.write_str("高ポリか投影の設定が変わった"),
            Self::Size { baked, now } => write!(
                f,
                "焼いた大きさ {}×{}、キャンバスは {}×{}",
                baked.0, baked.1, now.0, now.1
            ),
            Self::MaterialNotInModel => {
                f.write_str("テクスチャセットのマテリアルが読み込んだモデルに無い")
            }
            Self::Slots { baked, now } => write!(
                f,
                "焼いたスロット {}、テクスチャセットは {}",
                slots_text(baked),
                slots_text(now)
            ),
            Self::UvChannel { baked, now } => write!(f, "UV{baked} で焼いた（プロジェクトは UV{now}）"),
            Self::Padding { baked, now } => write!(f, "余白 {baked}（設定は {now}）"),
            Self::Antialiasing { baked, now } => {
                write!(f, "アンチエイリアス {baked}×{baked}（設定は {now}×{now}）")
            }
            Self::Settings { baked, now } => write!(f, "ベイクの設定が変わった（{baked} → {now}）"),
            Self::ShapeChanged => f.write_str(
                "焼いた後にモデルの形が変わった（三角形と UV は同じ。静的な基準の姿勢で焼くので自動では焼き直さない）",
            ),
            Self::ModelChanged => {
                f.write_str("焼いた後にモデルが変わった（三角形・UV・スロットが違う）")
            }
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MeshMapCheck {
    pub state: MeshMapState,
    pub reasons: Vec<MeshMapStaleReason>,
}
impl MeshMapProvenance {
    /// 今の条件に対して古い理由を返す（空なら今の条件で焼いたもの）。モデルの指紋が無ければ照合できない。
    pub fn check(&self, expected: &MeshMapExpectation) -> MeshMapCheck {
        use MeshMapStaleReason as R;
        let mut reasons = vec![];
        if self.engine_version != ENGINE_VERSION {
            reasons.push(R::EngineVersion {
                baked: self.engine_version,
                current: ENGINE_VERSION,
            });
        }
        if self.space != SPACE || self.pose != POSE {
            reasons.push(R::SpaceOrPose {
                space: self.space.clone(),
                pose: self.pose.clone(),
            });
        }
        let reference = expected.reference_hash.as_deref();
        let source_differs = match &expected.settings {
            Some(s) => self.source != s.source_key(reference),
            None => match reference {
                None => self.source != "Self",
                Some(hash) => !self.source.contains(hash),
            },
        };
        if source_differs {
            reasons.push(if self.source == "Self" {
                R::ReferenceAdded
            } else if reference.is_none() {
                R::ReferenceRemoved
            } else {
                R::ReferenceChanged
            });
        }
        if self.width != expected.width || self.height != expected.height {
            reasons.push(R::Size {
                baked: (self.width, self.height),
                now: (expected.width, expected.height),
            });
        }
        let wanted = expected.targets();
        // モデルが無ければスロットは比べない（照合できないので Unverified）
        if expected.mesh_hash.is_none() {
        } else if expected.target_slot < -1 && expected.target_slots.is_empty() {
            reasons.push(R::MaterialNotInModel);
        } else if self.target_slots != wanted {
            reasons.push(R::Slots {
                baked: self.target_slots.clone(),
                now: wanted,
            });
        }
        if self.uv_channel != expected.uv_channel {
            reasons.push(R::UvChannel {
                baked: self.uv_channel,
                now: expected.uv_channel,
            });
        }
        if let Some(s) = &expected.settings {
            if self.padding != s.padding {
                reasons.push(R::Padding {
                    baked: self.padding,
                    now: s.padding,
                });
            }
            if self.antialiasing != s.antialiasing {
                reasons.push(R::Antialiasing {
                    baked: self.antialiasing,
                    now: s.antialiasing,
                });
            }
            let now = s.kind_key(self.kind, &expected.material_identity);
            if now != self.settings_key {
                reasons.push(R::Settings {
                    baked: self.settings_key.clone(),
                    now,
                });
            }
        }
        if let Some(hash) = &expected.mesh_hash {
            if *hash != self.mesh_hash {
                reasons.push(
                    if expected.topology_hash.as_deref() == Some(self.topology_hash.as_str()) {
                        R::ShapeChanged
                    } else {
                        R::ModelChanged
                    },
                );
            }
        }
        let state = if !reasons.is_empty() {
            MeshMapState::Stale
        } else if expected.mesh_hash.is_none() {
            MeshMapState::Unverified
        } else {
            MeshMapState::Current
        };
        MeshMapCheck { state, reasons }
    }
}
