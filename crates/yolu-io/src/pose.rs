//! モデルの今のポーズ（根の `pose.json`）。スタンドアロン版が .ylp の状態として残す。
//!
//! - 持つのは、休みの形からの差だけ（骨ごとの平行移動の差・回転の差・大きさの比と、BlendShape ごとの重み）。骨は名前の道（根から
//!   骨までの名前の並び）、BlendShape はメッシュの名前と BlendShape の名前の組で持つので、名前の組が同じモデルへ戻せる（合わないものは
//!   戻すときに理由つきで飛ばす。ここでは持つだけで、モデルは見ない）。ポーズのプリセット（`.ylpose`）と同じ表し方。
//! - 正本の版も .ylp の形式（7・8）も変えない状態のエントリ（`view.json`・`look.json` と同じ扱い）。Unity 版・スタンドアロン版 0.3.x
//!   は知らないエントリとして読み飛ばし、スタンドアロン版 0.3.x は開いて保存し直してもバイト列のまま残す（Unity 版で保存すると
//!   落ちる。失うのはポーズだけ）。
//! - 形（UTF-8 の JSON のオブジェクト、8 MiB まで）:
//!   ```json
//!   {
//!     "format": 1,
//!     "bones": [ { "path": ["Hips", "Spine"], "translation": [0.0, 0.01, 0.0], "rotation": [0.0, 0.0, 0.1, 0.99], "scale": [1.0, 1.0, 1.0] } ],
//!     "shapes": [ { "mesh": "Face", "name": "Smile", "weight": 40.0 } ]
//!   }
//!   ```
//!   `translation` は親の空間での足し算の差、`rotation` は骨のローカルの差（単位クォータニオン x,y,z,w。今の回転 = 休みの回転 × 差）、
//!   `scale` は休みの大きさとの比。`weight` は Unity と同じ 0〜100 の目盛り。知らないキーは読み飛ばし、`format` が 1 でないもの
//!   （新しい版）は読まずに断る。
//! - 読み手は範囲の外・数でない値・単位でない回転・骨や BlendShape の重なり・数の上限を超えたものを、エントリごと断る（一部だけを
//!   読まない）。断ったエントリはファイルにバイト列のまま残り、ポーズを書き換えるまで保つ。

use serde_json::Value;

use crate::{check, check_budget, Result};

/// エントリの名前（根）。
pub const ENTRY: &str = "pose.json";
/// 版。
pub const FORMAT: i64 = 1;
/// エントリの大きさの上限。
pub const MAX_BYTES: usize = 8 * 1024 * 1024;
/// 骨の項目の数の上限（ポーズのプリセットと同じ）。
pub const MAX_BONES: usize = 4096;
/// BlendShape の項目の数の上限。
pub const MAX_SHAPES: usize = 4096;
/// 名前の道の深さの上限。
pub const MAX_PATH_DEPTH: usize = 256;
/// 名前 1 つの長さの上限（文字数）。
pub const MAX_NAME_CHARS: usize = 256;
/// 平行移動の差・大きさの比の絶対値の上限（桁の外れた値でモデルを壊さない）。
pub const MAX_MAGNITUDE: f32 = 1.0e6;
/// BlendShape の重みの絶対値の上限（Unity の目盛りで 100 が普通。外れた値は断る）。
pub const MAX_WEIGHT: f32 = 1.0e4;
/// 回転の差の長さの許す幅（単位クォータニオン。読んだあとで正規化する）。
const QUAT_TOLERANCE: f32 = 0.01;

/// 骨 1 つの休みの形からの差。
#[derive(Clone, Debug, PartialEq)]
pub struct StoredBone {
    /// 骨の名前の道（根から）。
    pub path: Vec<String>,
    pub translation: [f32; 3],
    /// x, y, z, w。
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
}

/// BlendShape 1 つの重み。
#[derive(Clone, Debug, PartialEq)]
pub struct StoredShape {
    pub mesh: String,
    pub name: String,
    pub weight: f32,
}

/// 保存するポーズ。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StoredPose {
    pub bones: Vec<StoredBone>,
    pub shapes: Vec<StoredShape>,
}

impl StoredPose {
    /// 休みの形と同じ（何も持たない）か。
    pub fn is_rest(&self) -> bool {
        self.bones.is_empty() && self.shapes.is_empty()
    }
}

/// 名前（骨・メッシュ・BlendShape）が決まりに合うか（1〜上限の文字数、制御文字なし）。
pub fn name_ok(name: &str) -> bool {
    !name.is_empty()
        && name.chars().count() <= MAX_NAME_CHARS
        && !name.chars().any(char::is_control)
}

/// 決まりを確かめる（読むときも書くときも同じ。回転は読んだあとの正規化は呼ばない）。
fn validate(pose: &StoredPose) -> Result<()> {
    check_budget(
        pose.bones.len() <= MAX_BONES && pose.shapes.len() <= MAX_SHAPES,
        "ポーズの項目の数が上限を超えています",
    )?;
    let finite = |v: &[f32], limit: f32| v.iter().all(|x| x.is_finite() && x.abs() <= limit);
    let mut seen = std::collections::HashSet::new();
    for b in &pose.bones {
        check(
            !b.path.is_empty()
                && b.path.len() <= MAX_PATH_DEPTH
                && b.path.iter().all(|n| name_ok(n)),
            "ポーズの骨の名前の道が不正です",
        )?;
        check(
            seen.insert(b.path.clone()),
            format!("ポーズの骨が重なっています: {}", b.path.join("/")),
        )?;
        check(
            finite(&b.translation, MAX_MAGNITUDE) && finite(&b.scale, MAX_MAGNITUDE),
            "ポーズの平行移動か大きさが範囲外です",
        )?;
        let len = b.rotation.iter().map(|x| x * x).sum::<f32>().sqrt();
        check(
            b.rotation.iter().all(|x| x.is_finite()) && (len - 1.0).abs() <= QUAT_TOLERANCE,
            "ポーズの回転が単位クォータニオンではありません",
        )?;
    }
    let mut seen = std::collections::HashSet::new();
    for s in &pose.shapes {
        check(
            name_ok(&s.mesh) && name_ok(&s.name),
            "ポーズの BlendShape の名前が不正です",
        )?;
        check(
            seen.insert((s.mesh.clone(), s.name.clone())),
            format!(
                "ポーズの BlendShape が重なっています: {}/{}",
                s.mesh, s.name
            ),
        )?;
        check(
            s.weight.is_finite() && s.weight.abs() <= MAX_WEIGHT,
            "ポーズの BlendShape の重みが範囲外です",
        )?;
    }
    Ok(())
}

/// 数 `n` 個の配列を読む。
fn floats<const N: usize>(v: &Value, what: &str) -> Result<[f32; N]> {
    let list = v.as_array().filter(|a| a.len() == N).ok_or_else(|| {
        crate::Error::InvalidData(format!("pose.json の {what} が {N} 個の数ではありません"))
    })?;
    let mut out = [0f32; N];
    for (slot, x) in out.iter_mut().zip(list) {
        let x = x.as_f64().ok_or_else(|| {
            crate::Error::InvalidData(format!("pose.json の {what} に数でない値があります"))
        })?;
        check(
            x.is_finite() && x.abs() <= f64::from(f32::MAX),
            format!("pose.json の {what} が範囲外です"),
        )?;
        *slot = x as f32;
    }
    Ok(out)
}

fn text<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    v.get(key).and_then(Value::as_str).ok_or_else(|| {
        crate::Error::InvalidData(format!("pose.json の {key} が文字列ではありません"))
    })
}

/// 読む（形の違い・新しい版・範囲外・重なり・上限超えは断る。回転は読んだあとに長さを 1 に直す）。
pub fn read(bytes: &[u8]) -> Result<StoredPose> {
    check_budget(bytes.len() <= MAX_BYTES, "pose.json が大きすぎます")?;
    let root: Value = serde_json::from_slice(bytes)?;
    check(root.is_object(), "pose.json がオブジェクトではありません")?;
    check(
        root.get("format").and_then(Value::as_i64) == Some(FORMAT),
        "pose.json の版が未対応です",
    )?;
    let array = |key: &str| -> Result<Vec<Value>> {
        match root.get(key) {
            None | Some(Value::Null) => Ok(Vec::new()),
            Some(Value::Array(a)) => Ok(a.clone()),
            Some(_) => Err(crate::Error::InvalidData(format!(
                "pose.json の {key} が配列ではありません"
            ))),
        }
    };
    let (bones, shapes) = (array("bones")?, array("shapes")?);
    // 項目を作る前に数の上限を見る（細工した巨大な配列で、変換の作業を増やさない）
    check_budget(
        bones.len() <= MAX_BONES && shapes.len() <= MAX_SHAPES,
        "ポーズの項目の数が上限を超えています",
    )?;
    let mut pose = StoredPose::default();
    for b in &bones {
        let path = b
            .get("path")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                crate::Error::InvalidData("pose.json の path が配列ではありません".into())
            })?
            .iter()
            .map(|n| {
                n.as_str().map(str::to_owned).ok_or_else(|| {
                    crate::Error::InvalidData(
                        "pose.json の path に文字列でない名前があります".into(),
                    )
                })
            })
            .collect::<Result<Vec<_>>>()?;
        pose.bones.push(StoredBone {
            path,
            translation: floats(&b["translation"], "translation")?,
            rotation: floats(&b["rotation"], "rotation")?,
            scale: floats(&b["scale"], "scale")?,
        });
    }
    for s in &shapes {
        let [weight] = floats::<1>(
            &serde_json::json!([s.get("weight").cloned().unwrap_or(Value::Null)]),
            "weight",
        )?;
        pose.shapes.push(StoredShape {
            mesh: text(s, "mesh")?.to_owned(),
            name: text(s, "name")?.to_owned(),
            weight,
        });
    }
    validate(&pose)?;
    for b in &mut pose.bones {
        let len = b.rotation.iter().map(|x| x * x).sum::<f32>().sqrt();
        if (len - 1.0).abs() > 1.0e-6 {
            for x in &mut b.rotation {
                *x /= len;
            }
        }
        if b.rotation[3] < 0.0 {
            // q と -q は同じ回転（1 通りに）
            for x in &mut b.rotation {
                *x = -*x;
            }
        }
    }
    Ok(pose)
}

/// f32 を、読み戻すと同じ値になる最短の JSON の数で書く（`Display` は指数を使わず、読み戻して同じになる最短の桁）。
fn number(x: f32) -> String {
    let text = format!("{x}");
    if text == "-0" {
        "0".into()
    } else {
        text
    }
}

fn array<const N: usize>(v: &[f32; N]) -> String {
    format!(
        "[{}]",
        v.iter().map(|x| number(*x)).collect::<Vec<_>>().join(",")
    )
}

/// 書く（決まりに合わなければ断る）。同じポーズはいつも同じバイト列になる。
pub fn write(pose: &StoredPose) -> Result<Vec<u8>> {
    validate(pose)?;
    let quote = |s: &str| serde_json::to_string(s).expect("文字列は書ける");
    let mut out = format!("{{\"format\":{FORMAT},\"bones\":[");
    for (i, b) in pose.bones.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let path: Vec<String> = b.path.iter().map(|n| quote(n)).collect();
        out += &format!(
            "{{\"path\":[{}],\"translation\":{},\"rotation\":{},\"scale\":{}}}",
            path.join(","),
            array(&b.translation),
            array(&b.rotation),
            array(&b.scale)
        );
    }
    out += "],\"shapes\":[";
    for (i, s) in pose.shapes.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out += &format!(
            "{{\"mesh\":{},\"name\":{},\"weight\":{}}}",
            quote(&s.mesh),
            quote(&s.name),
            number(s.weight)
        );
    }
    out += "]}";
    check_budget(out.len() <= MAX_BYTES, "pose.json が大きすぎます")?;
    Ok(out.into_bytes())
}
