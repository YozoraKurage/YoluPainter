//! Live Link で受けたマテリアルの値（Unity の本物の lilToon のマテリアル）を、テクスチャセットの「受けた見た目」にする。
//!
//! 利用者の設定と、Unity から来た値のどちらで描くかの決まり:
//! - Unity から来た値は、そのマテリアルのテクスチャセットの文書の「受けた見た目」（`Document::set_received_look`）として持つ。Undo にも
//!   版にも入らない（受け取りは利用者の操作ではなく、Undo で戻すと Unity の本物と食い違う）。
//! - 描く見た目は、受けた値の上に利用者の設定を重ねたもの（`MaterialLook::over`）。欄で変えた項目（利用者の設定にある項目）だけが勝ち、
//!   ほかは Unity の値で描く。節の「既定に戻す」（基本設定の節は描画モードと輪郭線の入切も）と「Unity の値に合わせる」で、その項目は
//!   Unity の値に戻る。描き方（標準・lilToon）は、欄で選んでいなければ Unity の値（lilToon）。
//! - Unity が lilToon でないと知らせたマテリアル（値なし）は、受けた見た目を外す。値を送らない古い Unity（機能の印が無い）からは何も
//!   来ないので、開いたファイルに保存してあった値のまま。
//! - 切断しても受けた見た目は残る（最後の値）。保存するかは設定（既定は保存する。`look::io`）。
//! - 描いていないスロットの絵は受けた見た目の中に持ち（保存しない）、全部のマテリアルの合計を [`MAX_RECEIVED_IMAGE_BYTES`] までにする。
//!   Unity が描いた絵を見せるスロット（流し込み先のうち、スタンドアロンが Unity へ出すチャンネルのもの。今は Color だけ）は、絵ではなく
//!   そのチャンネルで描く。ほかの流し込み先（Unity が元のテクスチャのまま見せる）は、ほかのスロットと同じく受けた絵で描く（Unity の
//!   見え方と同じ）。
//! - 後から足したスロット（メインカラー 2nd・リムシェードなど）を読む機能は、Unity がそのスロットを知らせたときだけ受けた見た目で
//!   入にする（`liltoon::FEATURES_OF_LATER_SLOTS`）。送るスロットの一覧が古い Unity からは入切と値だけが届き、知らないテクスチャを
//!   既定の白で読んで描くと Unity の見え方と大きく違う。

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use yolu_core::look::{
    LookKind, LookValue, MaterialLook, MissingImage, ReceivedImage, ReceivedLook, TextureSource,
    MAX_KEYWORDS, MAX_NAME, MAX_PROPERTIES, MAX_SHADER_NAME, MAX_TEXTURES,
};
use yolu_core::Channel;
use yolu_protocol::{
    ChannelRoute, MaterialTexture, MaterialValues, PropertyValue, SlotState, ValuesKind,
};

use crate::lang::Lang;
use crate::model::ModelSource;
use crate::state::AppState;

/// 受けた絵の画素のバイトの合計の上限（全部のマテリアル。超える絵は持たず、スロットは「読めない」）。
pub const MAX_RECEIVED_IMAGE_BYTES: u64 = 256 << 20;

/// 値が来ない・絵が揃わないマテリアルを、Unity に頼むまでの猶予（Unity はモデルの直後に値を送るので、それを待つ。絵が 1 枚届くたびに数え直す）。
pub const ASK_AFTER: Duration = Duration::from_secs(2);
/// 同じマテリアルを頼み直す間隔。
pub const ASK_AGAIN: Duration = Duration::from_secs(6);
/// 同じマテリアルを頼む回数の上限（Unity が答えない・答えられないとき、頼み続けない）。
pub const MAX_ASKS: u8 = 3;

/// 値の頼みの様子（マテリアルごと）。
struct Ask {
    /// 値が欠けている（絵が揃わない）と気づいた時刻。受けた内容が変わると数え直す。
    since: Instant,
    /// そのときの受けた内容の通し番号（変わったら進んでいる）。
    serial: Option<u64>,
    asked: u8,
    last: Option<Instant>,
}

/// マテリアル 1 つの受けたもの。
#[derive(Default)]
struct Entry {
    /// 最後の値（値なしの知らせは None）。
    values: Option<MaterialValues>,
    /// 届いた絵（スロットの名前 → 絵）。
    images: BTreeMap<String, Arc<ReceivedImage>>,
    /// 絵が来ると言われて、まだ届いていないスロット。
    awaiting: BTreeSet<String>,
    /// 届いたが予算を超えて持たなかったスロット。
    refused: BTreeSet<String>,
    /// 変わるたびに増える（当て直しの鍵）。
    serial: u64,
}

/// どの文書に、どの受けたものを当てたか（同じなら当て直さない）。
#[derive(Clone, PartialEq)]
struct Applied {
    doc: u128,
    serial: u64,
    routes: Vec<ChannelRoute>,
}

/// 受けた絵を持てなかった理由。
#[derive(Debug, PartialEq)]
pub enum TextureRefused {
    /// 命令の食い違い（世代・知らせていないスロット・無いマテリアル）。Unity への返事にする診断の文で、画面には出さない。
    Protocol(String),
    /// 受けた絵の予算を超えた。利用者に知らせる文（スロットの名前と短い理由、画面の言語）。
    OverBudget(String),
}

/// Live Link のつながり 1 つの、受けたマテリアルの値（`LiveLink` が持つ）。
#[derive(Default)]
pub struct LinkValues {
    generation: u32,
    /// 今のモデルのマテリアルの数（これより大きい番号の値・絵は断る）。
    materials: u32,
    entries: BTreeMap<u32, Entry>,
    applied: BTreeMap<u32, Applied>,
    next_serial: u64,
    /// 値を頼んだ様子（マテリアルの番号ごと）。
    asks: BTreeMap<u32, Ask>,
    /// 猶予と頼み直しの間隔（`None` は既定の [`ASK_AFTER`]・[`ASK_AGAIN`]。試験が短くする）。
    ask_timing: Option<(Duration, Duration)>,
}

impl LinkValues {
    /// 受けたものを全部捨てる（切った・つながりが替わった）。文書に当てた受けた見た目は残す（最後の値）。
    pub fn clear(&mut self) {
        *self = LinkValues::default();
    }

    /// 新しいモデル（世代と、マテリアルの数）を受けた: 前のモデルの値は捨てる（Unity は新しいモデルの値を送り直す）。
    pub fn model(&mut self, generation: u32, materials: u32) {
        if generation != self.generation {
            self.entries.clear();
            self.applied.clear();
            self.asks.clear();
            self.generation = generation;
        }
        self.materials = materials;
        // マテリアルが減った（同じ世代の送り直し）なら、無くなった番号の値も捨てる
        self.entries.retain(|m, _| *m < materials);
    }

    /// Unity に値を頼むマテリアル（頼みを出せるつながりのとき、毎フレーム呼ぶ。頼んだものは頼んだことにする）。`materials` は、テクスチャセットが
    /// 付いていて、値が来るはずのマテリアルの番号（マテリアルの無い組は、値が無いので呼び手が外す）。頼むのは次の 2 つで、どちらも猶予
    /// ([`ASK_AFTER`]) のあとに頼み、[`ASK_AGAIN`] の間隔で [`MAX_ASKS`] 回まで:
    /// - このつながり・このモデルで、値（「値なし」の知らせを含む）がまだ 1 つも来ていない（開いたセットの見た目が前のつながりのままのとき、
    ///   送り直しで値が欠けたとき）。
    /// - 値は来たが、来ると言われた絵が揃わない（絵が 1 枚届くたびに猶予は数え直す）。
    pub fn wanted(&mut self, materials: impl IntoIterator<Item = u32>, now: Instant) -> Vec<u32> {
        let (after, again) = self.ask_timing.unwrap_or((ASK_AFTER, ASK_AGAIN));
        let mut out = Vec::new();
        let mut live = BTreeSet::new();
        for material in materials {
            if material >= self.materials || !live.insert(material) {
                continue;
            }
            let entry = self.entries.get(&material);
            let missing = match entry {
                None => true,
                Some(e) => e.values.is_some() && !e.awaiting.is_empty(),
            };
            if !missing {
                self.asks.remove(&material);
                continue;
            }
            let serial = entry.map(|e| e.serial);
            let ask = self.asks.entry(material).or_insert(Ask {
                since: now,
                serial,
                asked: 0,
                last: None,
            });
            if ask.serial != serial {
                ask.serial = serial;
                ask.since = now;
            }
            if now.saturating_duration_since(ask.since) < after || ask.asked >= MAX_ASKS {
                continue;
            }
            if ask.last.is_some_and(|t| now.saturating_duration_since(t) < again) {
                continue;
            }
            ask.asked += 1;
            ask.last = Some(now);
            out.push(material);
        }
        // マテリアルから外れたセット（もう気にしない）の頼みの様子は捨てる
        self.asks.retain(|m, _| live.contains(m));
        out
    }

    /// 猶予と頼み直しの間隔を決める（試験用）。
    pub fn set_ask_timing(&mut self, after: Duration, again: Duration) {
        self.ask_timing = Some((after, again));
    }

    /// 値を頼んだ回数（試験・診断用）。
    pub fn asked(&self, material: u32) -> u8 {
        self.asks.get(&material).map_or(0, |a| a.asked)
    }

    fn bump(&mut self) -> u64 {
        self.next_serial += 1;
        self.next_serial
    }

    /// 値を受けた。今のモデルの世代でない・モデルに無いマテリアルの番号なら断る（Unity への返事にする診断の文を返す）。
    pub fn receive_values(&mut self, values: MaterialValues) -> Result<(), String> {
        if values.generation != self.generation {
            return Err(format!(
                "マテリアルの値の世代 {} は今のモデルの世代 {} と違います",
                values.generation, self.generation
            ));
        }
        if values.material >= self.materials {
            return Err(format!(
                "マテリアルの値の番号 {} はモデルのマテリアルの数 {} を超えています",
                values.material, self.materials
            ));
        }
        let serial = self.bump();
        let entry = self.entries.entry(values.material).or_default();
        if values.kind == ValuesKind::None {
            *entry = Entry {
                serial,
                ..Entry::default()
            };
            return Ok(());
        }
        // 来ると言われた絵と、前と同じと言われた絵だけを残す（届くまでは前の絵で描く）
        let keep: BTreeSet<&str> = values
            .slots
            .iter()
            .filter(|s| matches!(s.state, SlotState::Follows | SlotState::Unchanged))
            .map(|s| s.name.as_str())
            .collect();
        entry.images.retain(|k, _| keep.contains(k.as_str()));
        entry.awaiting = values
            .slots
            .iter()
            .filter(|s| s.state == SlotState::Follows)
            .map(|s| s.name.clone())
            .collect();
        entry.refused.retain(|k| entry.awaiting.contains(k));
        entry.values = Some(values);
        entry.serial = serial;
        Ok(())
    }

    /// 描いていないスロットの絵を受けた。持てなければ理由を返す（世代・値が合わない命令の食い違いは Unity への診断、予算を超えたのは
    /// 利用者への知らせ）。
    pub fn receive_texture(&mut self, texture: MaterialTexture, lang: Lang) -> Result<(), TextureRefused> {
        if texture.generation != self.generation {
            return Err(TextureRefused::Protocol(format!(
                "スロットの絵の世代 {} は今のモデルの世代 {} と違います",
                texture.generation, self.generation
            )));
        }
        let (material, slot) = (texture.material, texture.slot.as_str());
        let others: u64 = self
            .entries
            .iter()
            .flat_map(|(m, e)| {
                e.images
                    .iter()
                    .filter(move |(k, _)| !(*m == material && k.as_str() == slot))
            })
            .map(|(_, i)| i.pixels.len() as u64)
            .sum();
        let serial = self.bump();
        let Some(entry) = self
            .entries
            .get_mut(&texture.material)
            .filter(|e| e.values.is_some() && e.awaiting.contains(&texture.slot))
        else {
            return Err(TextureRefused::Protocol(format!(
                "値で知らせていないスロットの絵です（マテリアル {}・{}）",
                texture.material, texture.slot
            )));
        };
        entry.awaiting.remove(&texture.slot);
        entry.serial = serial;
        if others + texture.pixels.len() as u64 > MAX_RECEIVED_IMAGE_BYTES {
            entry.images.remove(&texture.slot);
            entry.refused.insert(texture.slot.clone());
            let label = crate::look::liltoon::slot(&texture.slot).map_or(texture.slot.as_str(), |s| s.label(lang));
            return Err(TextureRefused::OverBudget(lang.pick(
                format!("Unity のテクスチャ（{label}）を持てません: 受けたテクスチャが多すぎます"),
                format!("Cannot keep the Unity texture ({label}): too many received textures"),
            )));
        }
        entry.refused.remove(&texture.slot);
        entry.images.insert(
            texture.slot,
            Arc::new(ReceivedImage {
                width: texture.width,
                height: texture.height,
                srgb: texture.srgb,
                pixels: texture.pixels.into(),
            }),
        );
        Ok(())
    }

    /// 受けたものを、そのマテリアルに付いたテクスチャセットの文書へ当てる（毎フレーム。変わったセットだけ）。開き直しなどで文書が
    /// 替わったセットにも当て直す。今のつながりのモデルでなければ何もしない。
    pub fn apply(&mut self, state: &mut AppState, session: u64) {
        let ours = state.model.as_ref().filter(|m| {
            m.source == (ModelSource::LiveLink { session }) && m.generation == self.generation
        });
        let Some(model) = ours else {
            return;
        };
        let targets: Vec<(usize, u32, u32, Vec<ChannelRoute>)> = state
            .sets
            .iter()
            .enumerate()
            .filter_map(|(i, s)| {
                let m = s.bound?;
                let routes = model.materials.get(m as usize)?.routes.clone();
                Some((i, s.uid, m, routes))
            })
            .collect();
        for (index, uid, material, routes) in targets {
            let Some(entry) = self.entries.get(&material) else {
                continue;
            };
            let key = Applied {
                doc: state.set_doc(index).id(),
                serial: entry.serial,
                routes,
            };
            if self.applied.get(&uid) == Some(&key) {
                continue;
            }
            let received = entry
                .values
                .as_ref()
                .map(|v| received_look(v, &key.routes, &entry.images, &entry.refused));
            match state.set_doc_mut(index).set_received_look(received) {
                Ok(_) => {}
                Err(e) => state.message = state.lang.core_error(&e),
            }
            self.applied.insert(uid, key);
        }
    }

    /// 受けた絵のバイトの合計。
    pub fn image_bytes(&self) -> u64 {
        self.entries
            .values()
            .flat_map(|e| e.images.values())
            .map(|i| i.pixels.len() as u64)
            .sum()
    }
}

/// 名前が見た目の設定に入る形か（UTF-16 で 1〜`max` 文字、制御文字なし）。
fn name_fits(s: &str, max: usize) -> bool {
    let n = s.encode_utf16().count();
    n >= 1 && n <= max && !s.chars().any(char::is_control)
}

/// 人に見せる文を見た目の設定に入る形にする（制御文字を除き、`max` 文字まで）。
fn clean_text(s: &str, max: usize) -> String {
    let mut out = String::new();
    for c in s.chars().filter(|c| !c.is_control()) {
        if out.encode_utf16().count() + c.len_utf16() > max {
            break;
        }
        out.push(c);
    }
    out
}

/// 受けた値から、受けた見た目を作る。`routes` はマテリアルの流し込み先（スタンドアロンが Unity へ出すチャンネルのものだけ、絵ではなく
/// チャンネルで描く）、
/// `images` は届いた絵、`refused` は予算を超えて持たなかった絵のスロット。見た目の設定に入らない名前（長すぎる・制御文字）は落とす。
pub fn received_look(
    values: &MaterialValues,
    routes: &[ChannelRoute],
    images: &BTreeMap<String, Arc<ReceivedImage>>,
    refused: &BTreeSet<String>,
) -> ReceivedLook {
    let mut look = MaterialLook {
        kind: LookKind::LilToon,
        shader: if name_fits(&values.shader, MAX_SHADER_NAME) {
            values.shader.clone()
        } else {
            String::new()
        },
        ..MaterialLook::default()
    };
    for p in &values.properties {
        if look.properties.len() >= MAX_PROPERTIES || !name_fits(&p.name, MAX_NAME) {
            continue;
        }
        let v = match p.value {
            PropertyValue::Float(x) => LookValue::Float(x),
            PropertyValue::Int(x) => LookValue::Int(x),
            PropertyValue::Color(c) => LookValue::Color(c),
            PropertyValue::Vector(c) => LookValue::Vector(c),
        };
        look.properties.insert(p.name.clone(), v);
    }
    for k in &values.keywords {
        if look.keywords.len() < MAX_KEYWORDS
            && name_fits(k, MAX_NAME)
            && !k.contains(' ')
            && !look.keywords.contains(k)
        {
            look.keywords.push(k.clone());
        }
    }
    // Unity が描いた絵で見せるのは、スタンドアロンが出すチャンネルの流し込み先だけ
    for r in routes
        .iter()
        .filter(|r| crate::livelink::PUBLISHED_CHANNELS.contains(&r.channel))
    {
        if let Some(c) =
            Channel::from_index(r.channel as usize).filter(|_| name_fits(&r.property, MAX_NAME))
        {
            look.textures
                .insert(r.property.clone(), TextureSource::Channel(c));
        }
    }
    let mut out = ReceivedLook {
        look,
        source: clean_text(&values.source, MAX_SHADER_NAME),
        ..ReceivedLook::default()
    };
    for slot in &values.slots {
        if out.look.textures.contains_key(&slot.name)
            || !name_fits(&slot.name, MAX_NAME)
            || out.images.len() + out.missing.len() >= MAX_TEXTURES
        {
            continue;
        }
        let missing = match slot.state {
            SlotState::Empty => continue,
            SlotState::Follows | SlotState::Unchanged => match images.get(&slot.name) {
                Some(image) => {
                    out.images.insert(slot.name.clone(), image.clone());
                    continue;
                }
                None if refused.contains(&slot.name) || slot.state == SlotState::Unchanged => {
                    MissingImage::Unreadable
                }
                None => MissingImage::Pending,
            },
            SlotState::OverBudget => MissingImage::OverBudget,
            SlotState::Unreadable => MissingImage::Unreadable,
        };
        out.missing.insert(slot.name.clone(), missing);
    }
    leave_out_unlisted_features(&mut out, values);
    out
}

/// 後から足したスロットを読む機能のうち、Unity がそのスロットを知らせなかったもの（送るスロットの一覧が古い Unity のパッケージ。
/// 機能の入切と値だけが届く）を、受けた見た目では切る。知らされなかったスロットは「読めない」（欄の理由）。知らされたスロットは、
/// テクスチャが無い（`Empty`）ものも含めて Unity と同じに描く（Unity も既定のテクスチャで読む）。欄で機能を入にすれば描く
/// （利用者の設定が勝つ）。
fn leave_out_unlisted_features(out: &mut ReceivedLook, values: &MaterialValues) {
    let listed = |slot: &str| {
        values.slots.iter().any(|s| s.name == slot) || out.look.textures.contains_key(slot)
    };
    for (toggle, slots) in crate::look::liltoon::FEATURES_OF_LATER_SLOTS {
        if !crate::look::liltoon::on(&out.look, toggle) {
            continue;
        }
        let unlisted: Vec<&str> = slots.iter().copied().filter(|s| !listed(s)).collect();
        if unlisted.is_empty() {
            continue;
        }
        let off = match out.look.properties.get(*toggle) {
            Some(LookValue::Int(_)) => LookValue::Int(0),
            _ => LookValue::Float(0.0),
        };
        out.look.properties.insert((*toggle).to_owned(), off);
        for slot in unlisted {
            if out.images.len() + out.missing.len() < MAX_TEXTURES {
                out.missing.insert(slot.to_owned(), MissingImage::Unreadable);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yolu_protocol::{PropertyEntry, SlotTexture};

    fn values(generation: u32) -> MaterialValues {
        MaterialValues {
            generation,
            material: 0,
            kind: ValuesKind::LilToon,
            shader: "Hidden/lilToonOutline".into(),
            source: "lilToon 2.3.4 · Standard/Opaque+Outline".into(),
            properties: vec![
                PropertyEntry {
                    name: "_ShadowBorder".into(),
                    value: PropertyValue::Float(0.3),
                },
                PropertyEntry {
                    name: "_UseShadow".into(),
                    value: PropertyValue::Int(1),
                },
                PropertyEntry {
                    name: "x".repeat(MAX_NAME + 1),
                    value: PropertyValue::Float(1.0),
                },
            ],
            keywords: vec!["_A".into(), "_A".into()],
            slots: vec![
                SlotTexture {
                    name: "_MatCapTex".into(),
                    state: SlotState::Follows,
                    width: 2,
                    height: 2,
                },
                SlotTexture {
                    name: "_ShadowColorTex".into(),
                    state: SlotState::OverBudget,
                    width: 4096,
                    height: 4096,
                },
                SlotTexture {
                    name: "_MainTex".into(),
                    state: SlotState::Follows,
                    width: 2,
                    height: 2,
                },
            ],
        }
    }

    fn routes() -> Vec<ChannelRoute> {
        vec![
            ChannelRoute {
                channel: yolu_protocol::channel::COLOR,
                property: "_MainTex".into(),
            },
            // Normal は Unity へ出さない（Unity は元のテクスチャのまま見せる）ので、チャンネルでは描かない
            ChannelRoute {
                channel: yolu_protocol::channel::NORMAL,
                property: "_BumpMap".into(),
            },
        ]
    }

    fn texture(generation: u32, slot: &str, bytes: usize) -> MaterialTexture {
        MaterialTexture {
            generation,
            material: 0,
            slot: slot.into(),
            width: 1,
            height: (bytes / 4) as u32,
            srgb: true,
            pixels: vec![7; bytes],
        }
    }

    #[test]
    fn values_become_a_received_lil_toon_look_with_the_routes_as_channels() {
        let r = received_look(&values(1), &routes(), &BTreeMap::new(), &BTreeSet::new());
        assert_eq!(r.look.kind, LookKind::LilToon);
        assert_eq!(r.look.shader, "Hidden/lilToonOutline");
        assert_eq!(r.look.float("_ShadowBorder", 0.5), 0.3);
        assert_eq!(r.look.properties.len(), 2, "入らない名前は落とす");
        assert_eq!(r.look.keywords, vec!["_A".to_owned()]);
        assert_eq!(
            r.look.textures["_MainTex"],
            TextureSource::Channel(Channel::Color)
        );
        assert!(
            !r.look.textures.contains_key("_BumpMap"),
            "Unity が元のテクスチャで見せる流し込み先"
        );
        // 流し込み先のスロットは絵を待たない。来る絵は「届いていない」、送らない絵は理由つき
        assert_eq!(r.missing["_MatCapTex"], MissingImage::Pending);
        assert_eq!(r.missing["_ShadowColorTex"], MissingImage::OverBudget);
        assert!(!r.missing.contains_key("_MainTex"));
        assert_eq!(r.source, "lilToon 2.3.4 · Standard/Opaque+Outline");
        assert!(r.validate().is_ok());
    }

    #[test]
    fn a_feature_whose_later_slot_unity_did_not_list_is_left_out() {
        // 送るスロットの一覧が古い Unity: メインカラー 2nd とリムシェードは入だが、そのスロットを知らせない
        let mut v = values(1);
        v.properties.push(PropertyEntry {
            name: "_UseMain2ndTex".into(),
            value: PropertyValue::Int(1),
        });
        v.properties.push(PropertyEntry {
            name: "_UseRimShade".into(),
            value: PropertyValue::Float(1.0),
        });
        let r = received_look(&v, &routes(), &BTreeMap::new(), &BTreeSet::new());
        assert!(!crate::look::liltoon::on(&r.look, "_UseMain2ndTex"));
        assert_eq!(r.look.properties["_UseMain2ndTex"], LookValue::Int(0), "型は Unity のまま");
        assert!(!crate::look::liltoon::on(&r.look, "_UseRimShade"));
        assert_eq!(r.missing["_Main2ndTex"], MissingImage::Unreadable);
        assert_eq!(r.missing["_Main2ndBlendMask"], MissingImage::Unreadable);
        assert_eq!(r.missing["_RimShadeMask"], MissingImage::Unreadable);
        assert!(!r.missing.contains_key("_Main3rdTex"), "切の機能のスロットは理由を出さない");
        assert!(r.validate().is_ok());
        // スロットを知らせる Unity: テクスチャが無くても（Unity も既定のテクスチャで読む）入のまま
        for name in ["_Main2ndTex", "_Main2ndBlendMask"] {
            v.slots.push(SlotTexture {
                name: name.into(),
                state: SlotState::Empty,
                width: 0,
                height: 0,
            });
        }
        let r = received_look(&v, &routes(), &BTreeMap::new(), &BTreeSet::new());
        assert!(crate::look::liltoon::on(&r.look, "_UseMain2ndTex"));
        assert!(!r.missing.contains_key("_Main2ndTex"));
        assert!(!crate::look::liltoon::on(&r.look, "_UseRimShade"));
        // 流し込み先（チャンネルで描くスロット）も知らされたスロット
        let mut routed = routes();
        routed.push(ChannelRoute {
            channel: yolu_protocol::channel::COLOR,
            property: "_RimShadeMask".into(),
        });
        let r = received_look(&v, &routed, &BTreeMap::new(), &BTreeSet::new());
        assert!(crate::look::liltoon::on(&r.look, "_UseRimShade"));
    }

    #[test]
    fn textures_wait_for_their_values_and_stay_within_the_budget() {
        let mut link = LinkValues::default();
        link.model(4, 1);
        // 世代の違う値・知らせていない絵は断る（Unity への診断で、画面には出さない）
        assert!(link.receive_values(values(3)).is_err());
        assert!(matches!(
            link.receive_texture(texture(4, "_MatCapTex", 16), Lang::Ja),
            Err(TextureRefused::Protocol(_))
        ));
        link.receive_values(values(4)).unwrap();
        assert!(matches!(
            link.receive_texture(texture(4, "_Other", 16), Lang::Ja),
            Err(TextureRefused::Protocol(_))
        ));
        link.receive_texture(texture(4, "_MatCapTex", 16), Lang::Ja)
            .unwrap();
        assert_eq!(link.image_bytes(), 16);
        let e = &link.entries[&0];
        let r = received_look(e.values.as_ref().unwrap(), &routes(), &e.images, &e.refused);
        assert_eq!(r.images["_MatCapTex"].pixels.len(), 16);
        assert!(!r.missing.contains_key("_MatCapTex"));
        // 同じ値をもう一度受けても（絵は来る途中）、届くまでは前の絵を持つ
        link.receive_values(values(4)).unwrap();
        assert_eq!(link.image_bytes(), 16);
        // 予算を超える絵は持たず、スロットは「読めない」
        let big = (MAX_RECEIVED_IMAGE_BYTES + 4) as usize;
        let mut huge = texture(4, "_MatCapTex", 4);
        huge.pixels = vec![0; big];
        // 利用者への知らせは画面の言語で、スロットの名前（欄と同じ）と短い理由
        assert_eq!(
            link.receive_texture(huge, Lang::En),
            Err(TextureRefused::OverBudget(
                "Cannot keep the Unity texture (MatCap): too many received textures".into()
            ))
        );
        assert_eq!(link.image_bytes(), 0);
        let e = &link.entries[&0];
        let r = received_look(e.values.as_ref().unwrap(), &routes(), &e.images, &e.refused);
        assert_eq!(r.missing["_MatCapTex"], MissingImage::Unreadable);
        // 値なしの知らせは全部を捨てる。新しいモデルの世代でも捨てる
        let mut none = values(4);
        none.kind = ValuesKind::None;
        link.receive_values(none).unwrap();
        assert!(link.entries[&0].values.is_none());
        link.receive_values(values(4)).unwrap();
        link.model(5, 1);
        assert!(link.entries.is_empty());
    }

    #[test]
    fn values_and_textures_for_a_material_the_model_does_not_have_are_refused() {
        let mut link = LinkValues::default();
        // モデルが無い間（マテリアル 0 個）は何も受けない
        assert!(link.receive_values(values(0)).is_err());
        link.model(2, 2);
        let mut outside = values(2);
        outside.material = 2;
        assert!(link.receive_values(outside.clone()).is_err());
        outside.material = u32::MAX;
        assert!(link.receive_values(outside).is_err());
        let mut far = texture(2, "_MatCapTex", 16);
        far.material = 7;
        assert!(matches!(
            link.receive_texture(far, Lang::Ja),
            Err(TextureRefused::Protocol(_))
        ));
        assert!(link.entries.is_empty(), "範囲外の番号は溜めない");
        // 範囲の中は受ける
        let mut inside = values(2);
        inside.material = 1;
        link.receive_values(inside).unwrap();
        assert_eq!(link.entries.keys().copied().collect::<Vec<_>>(), vec![1]);
        // 同じ世代でマテリアルが減ったら、無くなった番号の値も捨てる
        link.model(2, 1);
        assert!(link.entries.is_empty());
    }

    fn secs(t0: Instant, s: f32) -> Instant {
        t0 + Duration::from_secs_f32(s)
    }

    #[test]
    fn a_material_without_values_is_asked_for_after_the_grace_a_few_times() {
        let mut link = LinkValues::default();
        link.model(1, 3);
        let t0 = Instant::now();
        // 値が来るのを待つ猶予（Unity はモデルの直後に値を送る）
        assert!(link.wanted([0, 1], t0).is_empty());
        assert!(link.wanted([0, 1], secs(t0, 1.9)).is_empty());
        assert_eq!(link.wanted([0, 1], secs(t0, 2.0)), vec![0, 1]);
        assert_eq!((link.asked(0), link.asked(1)), (1, 1));
        // 頼んだ直後は頼まない。間隔をあけて頼み直す。回数には上限がある
        assert!(link.wanted([0, 1], secs(t0, 2.5)).is_empty());
        assert!(link.wanted([0, 1], secs(t0, 7.9)).is_empty());
        assert_eq!(link.wanted([0, 1], secs(t0, 8.0)), vec![0, 1]);
        assert_eq!(link.wanted([0, 1], secs(t0, 14.0)), vec![0, 1]);
        assert_eq!(link.asked(0), MAX_ASKS);
        assert!(link.wanted([0, 1], secs(t0, 60.0)).is_empty(), "答えない Unity に頼み続けない");
        // 同じ番号を重ねて渡しても 1 つ。モデルに無い番号・どのセットにも付いていないマテリアルは頼まない
        let mut link = LinkValues::default();
        link.model(1, 3);
        let _ = link.wanted([2, 2, 7], t0);
        assert_eq!(link.wanted([2, 2, 7], secs(t0, 2.0)), vec![2]);
    }

    #[test]
    fn a_material_whose_values_arrived_is_not_asked_even_if_they_say_no_values() {
        let mut link = LinkValues::default();
        link.model(4, 2);
        let t0 = Instant::now();
        let _ = link.wanted([0, 1], t0);
        // マテリアル 0 は lilToon でない（値なし）、マテリアル 1 は絵の来ない値: どちらも「来た」ので頼まない
        let none = MaterialValues {
            kind: ValuesKind::None,
            properties: vec![],
            keywords: vec![],
            slots: vec![],
            ..values(4)
        };
        link.receive_values(none).unwrap();
        let mut plain = values(4);
        plain.material = 1;
        plain.slots.clear();
        link.receive_values(plain).unwrap();
        assert!(link.wanted([0, 1], secs(t0, 3.0)).is_empty());
        assert_eq!((link.asked(0), link.asked(1)), (0, 0));
    }

    #[test]
    fn a_material_whose_pictures_do_not_arrive_is_asked_for_after_the_grace_counted_again_at_each_picture() {
        let mut link = LinkValues::default();
        link.model(4, 1);
        let t0 = Instant::now();
        // 値は来たが、絵（_MatCapTex と _MainTex）が来ない
        link.receive_values(values(4)).unwrap();
        assert!(link.wanted([0], t0).is_empty());
        // 絵が 1 枚届くたびに猶予を数え直す（大きな絵が続いて届いているあいだは、頼まない）
        link.receive_texture(texture(4, "_MatCapTex", 16), Lang::Ja).unwrap();
        assert!(link.wanted([0], secs(t0, 1.5)).is_empty());
        assert!(link.wanted([0], secs(t0, 3.4)).is_empty(), "数え直した 1.5 秒から 2 秒たっていない");
        assert_eq!(link.wanted([0], secs(t0, 3.6)), vec![0]);
        // 揃ったら頼まない（頼みの様子も捨てる）
        link.receive_texture(texture(4, "_MainTex", 16), Lang::Ja).unwrap();
        assert!(link.wanted([0], secs(t0, 20.0)).is_empty());
        assert_eq!(link.asked(0), 0);
    }

    #[test]
    fn a_new_model_and_an_unbound_material_forget_the_asking() {
        let mut link = LinkValues::default();
        link.model(1, 2);
        let t0 = Instant::now();
        let _ = link.wanted([0, 1], t0);
        assert_eq!(link.wanted([0, 1], secs(t0, 2.0)), vec![0, 1]);
        // セットが外れたマテリアル（もう渡されない）の様子は捨てる
        assert!(link.wanted([0], secs(t0, 2.1)).is_empty());
        assert_eq!(link.asked(1), 0);
        assert_eq!(link.asked(0), 1);
        // 新しいモデル（別の世代）: 頼む回数は数え直し
        link.model(2, 2);
        assert_eq!(link.asked(0), 0);
        let _ = link.wanted([0], secs(t0, 3.0));
        assert_eq!(link.wanted([0], secs(t0, 5.0)), vec![0]);
        // 同じ世代の送り直し（マテリアルの数だけ変わる）では、頼む回数を引き継ぐ
        link.model(2, 3);
        assert_eq!(link.asked(0), 1);
        // 猶予・間隔は試験が短くできる
        let mut quick = LinkValues::default();
        quick.model(1, 1);
        quick.set_ask_timing(Duration::ZERO, Duration::from_millis(10));
        assert_eq!(quick.wanted([0], t0), vec![0]);
        assert!(quick.wanted([0], t0).is_empty());
        assert_eq!(quick.wanted([0], secs(t0, 0.02)), vec![0]);
    }
}
