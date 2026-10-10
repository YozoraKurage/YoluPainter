//! パスのプリセット（設定のフォルダの `path_presets/`）: 種類・ブラシ・組・筆先・投影の深さ・対称の入り切りを名前つきで保存する
//! （点・リボンの画像は持たない。画像はプロジェクトのアセットなので、当てるときは今のパスの画像か、アセットの最初の画像を使う）。
//!
//! 1 つが 1 ファイル（`path-<番号>.json`。UTF-8 の JSON、16 MiB まで）: `format` 1、`name`、`kind`（stroke・ribbon・fill・smudge・
//! erase）、`smudge_strength`・`ribbon_mode`（tile・stretch）・`ribbon_spacing`、`diameter`（画素）・`hardness`・`spacing`・
//! `opacity`・`flow`・`color`（RGBA8）・`pressure_size`・`pressure_opacity`・`pressure_flow`、`anti_alias`（weak・medium・strong。
//! なし は書かず、無ければ なし）、`material`（null か [チャンネルの番号,
//! R, G, B, A] の並び）、`tip`（null か `name`・`width`・`height`・`alpha`（覆いの 16 進））、`angle`・`follow`・`depth`（null は
//! 自動）・`symmetry`。読めないファイル・新しい形式は読み飛ばして理由を残す（ほかのファイルは読む）。書き込みは一時ファイルへ
//! 書いて読み戻して確かめてから 1 回の置換（`userfiles::write_text`）。

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{json, Value};
use yolu_core::paths::{ChannelPaint, PathKind, Ribbon, RibbonMode};
use yolu_core::{AntiAlias, BrushSettings, BrushTip, Channel, ImageId, Rgba8};

use super::{with_brush, with_material, with_style, PathAction};
use crate::lang::Lang;
use crate::notice::Source;
use crate::state::AppState;
use crate::userfiles;
use yolu_core::paths::{PathBrush, PathStyle, PathSymmetry};

const FORMAT: i64 = 1;
const PREFIX: &str = "path-";
const EXTENSION: &str = "json";
/// 1 ファイルの大きさの上限（2048² の筆先の 16 進が入る）。
const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;
/// プリセットの数の上限。
pub const MAX_PRESETS: usize = 256;
/// 名前の長さの上限（文字数）。
pub const MAX_NAME_CHARS: usize = 64;

/// プリセットの中身（パスの点は持たない）。
#[derive(Clone, Debug, PartialEq)]
pub struct PresetData {
    pub kind: PathKind,
    /// ブラシ（半径は画素。3D のパスへ当てるときはモデルの大きさに換算する）。
    pub brush: BrushSettings,
    pub material: Option<Vec<ChannelPaint>>,
    pub tip: Option<Arc<BrushTip>>,
    pub angle: f64,
    pub follow: bool,
    pub depth: Option<f64>,
    pub symmetry: bool,
}

/// 名前つきのプリセット。
#[derive(Clone, Debug, PartialEq)]
pub struct Preset {
    pub id: u32,
    pub name: String,
    pub data: PresetData,
}

/// プリセットの置き場。
#[derive(Default)]
pub struct PathPresets {
    dir: Option<PathBuf>,
    list: Vec<Preset>,
    /// 読めなかったファイルの理由（最初の 1 件）。
    pub problem: Option<String>,
}

fn kind_name(kind: &PathKind) -> &'static str {
    match kind {
        PathKind::Stroke => "stroke",
        PathKind::Ribbon(_) => "ribbon",
        PathKind::Fill => "fill",
        PathKind::Smudge { .. } => "smudge",
        PathKind::Erase => "erase",
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

/// JSON にする。
pub fn encode(name: &str, d: &PresetData) -> String {
    let (strength, mode, spacing) = match d.kind {
        PathKind::Smudge { strength } => (strength, "tile", 1.0),
        PathKind::Ribbon(r) => (
            0.5,
            match r.mode {
                RibbonMode::Tile => "tile",
                RibbonMode::Stretch => "stretch",
            },
            r.spacing,
        ),
        _ => (0.5, "tile", 1.0),
    };
    let b = d.brush;
    let rgba = |c: Rgba8| json!([c.r, c.g, c.b, c.a]);
    let mut value = json!({
        "format": FORMAT,
        "name": name,
        "kind": kind_name(&d.kind),
        "smudge_strength": strength,
        "ribbon_mode": mode,
        "ribbon_spacing": spacing,
        "diameter": b.radius * 2.0,
        "hardness": b.hardness,
        "spacing": b.spacing,
        "opacity": b.opacity,
        "flow": b.flow,
        "color": rgba(b.color),
        "pressure_size": b.pressure_size,
        "pressure_opacity": b.pressure_opacity,
        "pressure_flow": b.pressure_flow,
        "material": d.material.as_ref().map(|m| {
            m.iter()
                .map(|p| json!([p.channel.index(), p.color.r, p.color.g, p.color.b, p.color.a]))
                .collect::<Vec<_>>()
        }),
        "tip": d.tip.as_ref().map(|t| json!({
            "name": t.name(),
            "width": t.width(),
            "height": t.height(),
            "alpha": hex(t.alpha()),
        })),
        "angle": d.angle,
        "follow": d.follow,
        "depth": d.depth,
        "symmetry": d.symmetry,
    });
    // 縁のアンチエイリアスは なし でないときだけ書く（なし のプリセットは今までと同じ文）
    if b.anti_alias != AntiAlias::None {
        value["anti_alias"] = json!(b.anti_alias.id());
    }
    serde_json::to_string_pretty(&value).unwrap_or_default()
}

/// JSON から読む（名前と中身）。読めなければ理由の短い文。
pub fn decode(text: &str) -> Result<(String, PresetData), String> {
    let v: Value = serde_json::from_str(text).map_err(|_| "JSON".to_owned())?;
    match v["format"].as_i64() {
        Some(FORMAT) => {}
        Some(n) => return Err(format!("format {n}")),
        None => return Err("format".into()),
    }
    let num = |k: &str, lo: f64, hi: f64| -> Result<f64, String> {
        v[k].as_f64()
            .filter(|x| x.is_finite() && (lo..=hi).contains(x))
            .ok_or_else(|| k.to_owned())
    };
    let flag = |k: &str| v[k].as_bool().ok_or_else(|| k.to_owned());
    let byte = |x: &Value| x.as_u64().filter(|b| *b <= 255).map(|b| b as u8);
    let rgba = |x: &Value| -> Option<Rgba8> {
        let a = x.as_array()?;
        (a.len() == 4).then(|| {
            Some(Rgba8::new(
                byte(&a[0])?,
                byte(&a[1])?,
                byte(&a[2])?,
                byte(&a[3])?,
            ))
        })?
    };
    let name: String = v["name"]
        .as_str()
        .ok_or("name")?
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_NAME_CHARS)
        .collect();
    let kind = match v["kind"].as_str().ok_or("kind")? {
        "stroke" => PathKind::Stroke,
        "fill" => PathKind::Fill,
        "erase" => PathKind::Erase,
        "smudge" => PathKind::Smudge {
            strength: num("smudge_strength", 0.0, 1.0)?,
        },
        "ribbon" => PathKind::Ribbon(Ribbon {
            // 画像はプリセットに持たない（当てるときに決める）
            image: ImageId(0),
            mode: match v["ribbon_mode"].as_str() {
                Some("stretch") => RibbonMode::Stretch,
                Some("tile") => RibbonMode::Tile,
                _ => return Err("ribbon_mode".into()),
            },
            spacing: num("ribbon_spacing", 0.1, 4.0)?,
        }),
        other => return Err(format!("kind {other}")),
    };
    let brush = BrushSettings {
        radius: num("diameter", 0.01, 8192.0)? / 2.0,
        hardness: num("hardness", 0.0, 1.0)?,
        spacing: num("spacing", 0.01, 4.0)?,
        opacity: num("opacity", 0.0, 1.0)?,
        flow: num("flow", 0.0, 1.0)?,
        color: rgba(&v["color"]).ok_or("color")?,
        pressure_size: flag("pressure_size")?,
        pressure_opacity: flag("pressure_opacity")?,
        pressure_flow: flag("pressure_flow")?,
        erase: false,
        anti_alias: match &v["anti_alias"] {
            Value::Null => AntiAlias::None,
            x => x
                .as_str()
                .and_then(AntiAlias::from_id)
                .ok_or("anti_alias")?,
        },
    };
    let material = match &v["material"] {
        Value::Null => None,
        Value::Array(items) if (1..=6).contains(&items.len()) => Some(
            items
                .iter()
                .map(|item| {
                    let a = item.as_array().filter(|a| a.len() == 5)?;
                    let channel =
                        Channel::from_index(a[0].as_u64()? as usize).filter(|c| c.is_standard())?;
                    let color = Rgba8::new(byte(&a[1])?, byte(&a[2])?, byte(&a[3])?, byte(&a[4])?);
                    Some(ChannelPaint { channel, color })
                })
                .collect::<Option<Vec<_>>>()
                .ok_or("material")?,
        ),
        _ => return Err("material".into()),
    };
    let tip = match &v["tip"] {
        Value::Null => None,
        t => {
            let w = t["width"].as_u64().ok_or("tip")? as u32;
            let h = t["height"].as_u64().ok_or("tip")? as u32;
            let alpha = unhex(t["alpha"].as_str().ok_or("tip")?).ok_or("tip")?;
            Some(Arc::new(
                BrushTip::new(t["name"].as_str().unwrap_or(""), w, h, alpha)
                    .map_err(|_| "tip".to_owned())?,
            ))
        }
    };
    let depth = match &v["depth"] {
        Value::Null => None,
        _ => Some(num("depth", 0.05, 64.0)?),
    };
    Ok((
        name,
        PresetData {
            kind,
            brush,
            material,
            tip,
            angle: num("angle", -360.0, 360.0)?,
            follow: flag("follow")?,
            depth,
            symmetry: flag("symmetry")?,
        },
    ))
}

impl PathPresets {
    /// 置き場のフォルダを決めて読む（読めないファイルは読み飛ばして、最初の理由を残す）。
    pub fn attach(&mut self, dir: PathBuf) {
        self.list.clear();
        self.problem = None;
        let mut files: Vec<(u32, PathBuf)> = std::fs::read_dir(&dir)
            .map(|rd| {
                rd.flatten()
                    .filter_map(|e| {
                        let p = e.path();
                        let stem = p.file_stem()?.to_str()?.strip_prefix(PREFIX)?;
                        let id = stem.parse::<u32>().ok()?;
                        (p.extension()? == EXTENSION).then_some((id, p))
                    })
                    .collect()
            })
            .unwrap_or_default();
        files.sort();
        for (id, path) in files.into_iter().take(MAX_PRESETS) {
            match userfiles::read_checked(&path, MAX_FILE_BYTES)
                .map_err(|_| String::from("read"))
                .and_then(|t| decode(&t))
            {
                Ok((name, data)) => self.list.push(Preset { id, name, data }),
                Err(why) => {
                    if self.problem.is_none() {
                        self.problem = Some(format!(
                            "{}: {why}",
                            path.file_name().unwrap_or_default().to_string_lossy()
                        ));
                    }
                }
            }
        }
        self.dir = Some(dir);
    }

    pub fn list(&self) -> &[Preset] {
        &self.list
    }

    /// 1 つのプリセットのファイルを書く（置き場が無ければ何もしない）。
    fn write(&self, id: u32, name: &str, data: &PresetData) -> Result<(), String> {
        let Some(dir) = &self.dir else {
            return Ok(());
        };
        let text = encode(name, data);
        userfiles::write_text(
            &dir.join(format!("{PREFIX}{id}.{EXTENSION}")),
            &text,
            MAX_FILE_BYTES,
            |read| read == text,
        )
        .map_err(|e| match e {
            userfiles::FileError::Io(e) => e.to_string(),
            userfiles::FileError::TooLarge => "too large".into(),
            userfiles::FileError::NotText | userfiles::FileError::Mismatch => "mismatch".into(),
        })
    }

    /// 新しいプリセットとして保存する（名前は `base` に番号。置き場が無ければ覚えるだけ）。保存したプリセットの番号。
    pub fn save(&mut self, base: &str, data: PresetData) -> Result<u32, String> {
        if self.list.len() >= MAX_PRESETS {
            return Err(format!("{MAX_PRESETS}"));
        }
        let id = self.list.iter().map(|p| p.id + 1).max().unwrap_or(1);
        let name = userfiles::unused_name(base, |n| self.list.iter().any(|p| p.name == n));
        self.write(id, &name, &data)?;
        self.list.push(Preset { id, name, data });
        Ok(id)
    }

    /// 名前を変える（前後の空白と制御文字を除き、`MAX_NAME_CHARS` 文字まで。ほかのプリセットと同じ名前なら番号を付ける）。
    /// ファイルを書き直してから一覧を替える（書けなければ一覧もファイルも前のまま）。付けた名前。
    pub fn rename(&mut self, id: u32, name: &str) -> Result<String, RenameError> {
        let at = self
            .list
            .iter()
            .position(|p| p.id == id)
            .ok_or(RenameError::Missing)?;
        let base: String = name
            .chars()
            .filter(|c| !c.is_control())
            .collect::<String>()
            .trim()
            .chars()
            .take(MAX_NAME_CHARS)
            .collect();
        let base = base.trim_end();
        if base.is_empty() {
            return Err(RenameError::Empty);
        }
        if self.list[at].name == base {
            return Ok(base.to_owned());
        }
        let next = userfiles::unused_name(base, |n| {
            self.list.iter().any(|p| p.id != id && p.name == n)
        });
        self.write(id, &next, &self.list[at].data)
            .map_err(RenameError::Write)?;
        self.list[at].name = next.clone();
        Ok(next)
    }

    /// プリセットを消す（ファイルも）。
    pub fn delete(&mut self, id: u32) -> Result<(), String> {
        let Some(at) = self.list.iter().position(|p| p.id == id) else {
            return Ok(());
        };
        if let Some(dir) = &self.dir {
            userfiles::remove(&dir.join(format!("{PREFIX}{id}.{EXTENSION}")))
                .map_err(|e| e.to_string())?;
        }
        self.list.remove(at);
        Ok(())
    }

    pub fn get(&self, id: u32) -> Option<&Preset> {
        self.list.iter().find(|p| p.id == id)
    }
}

/// 名前の変更の失敗。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RenameError {
    /// そのプリセットが無い（消えた）。
    Missing,
    /// 空の名前。
    Empty,
    /// ファイルを書けない（理由の短い文）。
    Write(String),
}

/// プリセットの操作。
#[derive(Clone, Debug, PartialEq)]
pub enum PresetOp {
    /// 今のパス（無ければ次に作るパスの設定）を新しいプリセットにする。
    Save,
    /// 選んでいるパスへ当てる（1 回の Undo。パスが無ければ次に作るパスの設定にする）。
    Apply(u32),
    Delete(u32),
    /// 名前の入力欄を開く（プリセットのボタンの所）。
    BeginRename(u32),
    /// 名前を変える（設定のフォルダのファイルを書き直す）。
    Rename(u32, String),
}

impl From<PresetOp> for PathAction {
    fn from(op: PresetOp) -> PathAction {
        PathAction::Preset(op)
    }
}

/// 種類の名前（プリセットの名前のもと）。
fn base_name(lang: Lang, kind: &PathKind) -> &'static str {
    crate::panels::path_props::kind_name(lang, *kind)
}

impl AppState {
    /// 3D のパスのブラシの半径（モデルの単位）を画素へ（モデルが無ければ None）。
    fn path_world_unit(&self) -> Option<f64> {
        let m = self.view3d.full_model()?;
        Some(
            yolu_core::geometry::world_radius(m.rest_geometry(), 1.0, self.doc.width()).max(1e-12)
                as f64,
        )
    }

    /// 今のパス（無ければ次に作るパスの設定と今のブラシ）のプリセットの中身。
    fn path_preset_data(&self) -> Option<PresetData> {
        match self.path_layer() {
            Some((_, path)) => {
                let mut brush = super::path_brush(path).0;
                if !path.is_canvas() {
                    brush.radius /= self.path_world_unit()?;
                }
                let style = path.style();
                Some(PresetData {
                    kind: style.kind,
                    brush,
                    material: path.material().map(<[_]>::to_vec),
                    tip: style.tip.clone(),
                    angle: style.angle,
                    follow: style.follow,
                    depth: style.depth,
                    symmetry: style.symmetry != PathSymmetry::None,
                })
            }
            None => {
                let style = &self.path.next_style;
                Some(PresetData {
                    kind: style.kind,
                    brush: self.brush.settings(self.color.main, false),
                    material: None,
                    tip: style.tip.clone(),
                    angle: style.angle,
                    follow: style.follow,
                    depth: style.depth,
                    symmetry: false,
                })
            }
        }
    }

    /// プリセットの操作を当てる。
    pub(super) fn path_preset(&mut self, op: PresetOp) {
        let lang = self.lang;
        match op {
            PresetOp::Save => {
                let Some(data) = self.path_preset_data() else {
                    self.refuse(Source::Path, lang.pick("モデルがありません", "No model"));
                    return;
                };
                let base = base_name(lang, &data.kind);
                match self.path.presets.save(base, data) {
                    Ok(_) => self.info(
                        Source::Path,
                        lang.pick("パスのプリセットを保存しました。", "Path preset saved."),
                    ),
                    Err(why) => self.fail(
                        Source::Path,
                        lang.with_reason(
                            lang.pick(
                                "パスのプリセットを保存できません",
                                "Cannot save the path preset",
                            ),
                            why,
                        ),
                    ),
                }
            }
            PresetOp::BeginRename(id) => {
                if self.path.presets.get(id).is_some() {
                    self.path.preset_renaming = Some(id);
                    self.path.preset_rename_started = false;
                }
            }
            PresetOp::Rename(id, name) => match self.path.presets.rename(id, &name) {
                // 空の名前は前の名前のまま（ほかの一覧の名前の変更と同じ）
                Ok(_) | Err(RenameError::Missing | RenameError::Empty) => {}
                Err(RenameError::Write(why)) => self.fail(
                    Source::Path,
                    lang.with_reason(
                        lang.pick(
                            "パスのプリセットの名前を変えられません",
                            "Cannot rename the path preset",
                        ),
                        why,
                    ),
                ),
            },
            PresetOp::Delete(id) => {
                if self.path.preset_renaming == Some(id) {
                    self.path.preset_renaming = None;
                }
                if let Err(why) = self.path.presets.delete(id) {
                    self.fail(
                        Source::Path,
                        lang.with_reason(
                            lang.pick(
                                "パスのプリセットを削除できません",
                                "Cannot delete the path preset",
                            ),
                            why,
                        ),
                    );
                }
            }
            PresetOp::Apply(id) => {
                let Some(preset) = self.path.presets.get(id).cloned() else {
                    return;
                };
                self.path_apply_preset(preset.data);
            }
        }
    }

    /// プリセットを当てる。リボンの画像は、今のパスのリボンの画像か、アセットの最初の画像。
    fn path_apply_preset(&mut self, data: PresetData) {
        let lang = self.lang;
        let mut kind = data.kind;
        if let PathKind::Ribbon(r) = &mut kind {
            let current = self.path_layer().and_then(|(_, p)| match p.style().kind {
                PathKind::Ribbon(c) => Some(c.image),
                _ => None,
            });
            let first = self
                .shelf
                .resources()
                .iter()
                .filter(|res| res.kind == "image")
                .find_map(|res| crate::fx::inputs::image_id(&res.id));
            match current.or(first) {
                Some(image) => r.image = image,
                None => {
                    self.refuse(
                        Source::Path,
                        lang.pick("アセットに画像がありません", "No image in the assets"),
                    );
                    return;
                }
            }
            let resource = crate::fx::inputs::resource_id(r.image);
            if let Err(m) = self.use_shelf_image(&resource) {
                self.refuse(Source::Path, m);
                return;
            }
        }
        let Some(path) = self.path_layer().map(|(_, p)| p.clone()) else {
            // 次に作るパスの設定と、今のブラシ
            self.path.next_style = PathStyle {
                kind,
                tip: data.tip,
                angle: data.angle,
                follow: data.follow,
                depth: data.depth,
                symmetry: PathSymmetry::None,
            };
            let b = &mut self.brush;
            b.radius = (data.brush.radius as f32).clamp(0.5, crate::state::MAX_RADIUS);
            b.hardness = data.brush.hardness as f32;
            b.spacing = data.brush.spacing as f32;
            b.opacity = data.brush.opacity as f32;
            b.flow = data.brush.flow as f32;
            b.pressure_size = data.brush.pressure_size;
            b.pressure_opacity = data.brush.pressure_opacity;
            b.pressure_flow = data.brush.pressure_flow;
            b.anti_alias = data.brush.anti_alias;
            return;
        };
        // 組を持たないプリセットは、色をパスの今の色のまま（色は塗る値で、描き方の設定ではない）
        let mut brush = data.brush;
        if data.material.is_none() {
            brush.color = super::path_brush(&path).0.color;
        }
        if !path.is_canvas() {
            let Some(unit) = self.path_world_unit() else {
                self.refuse(Source::Path, lang.pick("モデルがありません", "No model"));
                return;
            };
            brush.radius *= unit;
        }
        // 組は、パスの基準のチャンネルとプリセットの組が合うときだけ（合わなければパスの今の組）
        let material = match data.material {
            Some(m) if m.iter().all(|p| self.doc.channel_info(p.channel).is_some()) => Some(m),
            _ => path.material().map(<[_]>::to_vec),
        };
        let mut style = path.style().clone();
        style.kind = kind;
        style.tip = data.tip;
        style.angle = data.angle;
        style.follow = data.follow;
        style.depth = if path.is_canvas() { None } else { data.depth };
        let next = with_style(
            &with_material(&with_brush(&path, PathBrush(brush)), material),
            style,
        );
        let Some((layer, _)) = self.path_target(None) else {
            return;
        };
        let keep = self.path_selected_index();
        if self.path_commit(Some(layer), next.clone(), keep) && data.symmetry {
            // 対称は、効いている対称定規の値で入れる（2 回目の Undo。効いている対称定規が無ければ断る）
            if next.style().symmetry == PathSymmetry::None {
                self.path_apply(PathAction::Symmetry(true));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_preset_round_trips_through_its_text_and_refuses_broken_files() {
        let data = PresetData {
            kind: PathKind::Smudge { strength: 0.25 },
            brush: BrushSettings {
                radius: 12.5,
                color: Rgba8::new(1, 2, 3, 4),
                anti_alias: AntiAlias::Strong,
                ..BrushSettings::default()
            },
            material: Some(vec![ChannelPaint {
                channel: Channel::Roughness,
                color: Rgba8::new(9, 9, 9, 255),
            }]),
            tip: Some(Arc::new(BrushTip::new("a", 2, 1, vec![7, 250]).unwrap())),
            angle: -12.0,
            follow: true,
            depth: Some(3.0),
            symmetry: true,
        };
        let text = encode("名前", &data);
        let (name, back) = decode(&text).unwrap();
        assert_eq!((name.as_str(), back), ("名前", data.clone()));
        assert!(decode("{\"format\":2}").is_err());
        assert!(decode(&text.replace("\"smudge\"", "\"spray\"")).is_err());
        assert!(decode("not json").is_err());
        // 縁のアンチエイリアス: なし は書かず（無ければ なし）、知らない値は断る
        assert!(text.contains("\"anti_alias\": \"strong\""), "{text}");
        assert_eq!(
            decode(&text.replace("\"strong\"", "\"soft\"")).err(),
            Some("anti_alias".to_owned())
        );
        let plain = PresetData {
            brush: BrushSettings {
                anti_alias: AntiAlias::None,
                ..data.brush
            },
            ..data.clone()
        };
        let text = encode("名前", &plain);
        assert!(!text.contains("anti_alias"), "{text}");
        assert_eq!(decode(&text).unwrap().1, plain);
    }
}
