//! テクスチャセット。Unity 版と同じく、モデルのマテリアル 1 つ = セット 1 つで、文書（yolu-core の `Document`）はセットに 1 つ。
//!
//! 今のセットの文書は `AppState.doc`（キャンバス・レイヤーのパネル・試験が読む所）に置き、ほかのセットの文書は各セットの中にしまう。
//! 切り替えはこの 2 つを入れ替える（文書を写さない）。Undo・Redo は文書ごと（セットごと）。
//!
//! マテリアルの鍵は .ylp の形式 7 の `material` と同じ 3 つの形（名前と、アセットなら GUID と localFileId／マテリアルの無いスロットの
//! 全部／まだ結び付けていないスロットの番号）。モデルのマテリアルへの照合も Unity 版と同じ順: 識別子（GUID と localFileId）→
//! Unassigned → 名前（大文字小文字まで同じ、次に区別せず）→ スロットの番号。1 つのマテリアルは 1 つのセットだけが持つ。合わない
//! セットも残し、鍵も変えない（3D に見えないだけ。そのマテリアルを持つモデルに替えればまた付く）。合ったセットの鍵はそのマテリアルの
//! 鍵に書き換える（スロットの番号や古いアセットの鍵のまま保存しない）。

use std::sync::atomic::Ordering;

use yolu_protocol::{channel, MaterialInfo, MaterialKey as LinkKey};

use crate::canvas::view::ViewState;
use crate::engine::{Document, LayerId};
use crate::state::{blank_document, AppState, DEFAULT_DOCUMENT_SIZE};

pub use yolu_io::{MaterialAsset, MaterialRef};

/// Live Link の鍵から .ylp の鍵へ。
pub fn material_from_link(key: &LinkKey) -> MaterialRef {
    match key {
        LinkKey::Unassigned => MaterialRef::Unassigned,
        LinkKey::Material { name, asset } => MaterialRef::Material {
            name: name.clone(),
            asset: asset.as_ref().map(|(guid, file_id)| MaterialAsset {
                guid: guid.clone(),
                file_id: *file_id,
            }),
        },
    }
}

/// 鍵の、人に見せる説明。
pub fn describe_material(material: &MaterialRef) -> String {
    match material {
        MaterialRef::Material { name, asset: None } => format!("マテリアル「{name}」"),
        MaterialRef::Material {
            name,
            asset: Some(a),
        } => format!(
            "マテリアル「{name}」（アセット {}…・{}）",
            &a.guid[..a.guid.len().min(8)],
            a.file_id
        ),
        MaterialRef::Unassigned => "マテリアルの無いスロット".into(),
        MaterialRef::PendingSlot(n) => format!("まだマテリアルに付いていない（スロット {n}）"),
    }
}

/// 文書の ID（128 bit）を .ylp のセットの ID の形（小文字のハイフン付きの GUID）に。Unity 版と同じく、新しく作るセットの ID は
/// 文書の ID と同じ値にする（yolu-io の from_core が正本に書く文書の ID と同じ文字列）。
pub fn guid_string(id: u128) -> String {
    let h = format!("{id:032x}");
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

/// 名前が大文字小文字を区別せずにほかと重ならないように、要れば「 2」「 3」… を付ける（.ylp の決まり。書き出すファイルの名前に使う）。
pub fn unique_name<'a>(base: &str, taken: impl Iterator<Item = &'a str> + Clone) -> String {
    // yolu-io の検証と同じく大文字にそろえて比べる
    let free = |n: &str| !taken.clone().any(|t| t.to_uppercase() == n.to_uppercase());
    if free(base) {
        return base.to_owned();
    }
    (2..)
        .map(|i| format!("{base} {i}"))
        .find(|n| free(n))
        .expect("どこかで空く")
}

/// マテリアルから作るセットの名前（Unity 版と同じく、マテリアルの名前。マテリアルの無いスロットは Unassigned）。
pub fn name_for(key: &LinkKey) -> String {
    match key {
        LinkKey::Unassigned => "Unassigned".into(),
        LinkKey::Material { name, .. } if name.trim().is_empty() => "Material".into(),
        LinkKey::Material { name, .. } => name.clone(),
    }
}

/// マテリアルから作るセットの大きさ: Color の流し込み先のプロパティに今入っているテクスチャの大きさを、256〜4096 の 2 の冪に丸めたもの。
/// 流し込み先もテクスチャも無ければ新しい文書の既定（2048）。
pub fn size_for(info: &MaterialInfo) -> u32 {
    let property = info
        .routes
        .iter()
        .find(|r| r.channel == channel::COLOR)
        .map(|r| r.property.as_str());
    info.textures
        .iter()
        .find(|t| Some(t.name.as_str()) == property)
        .map(|t| t.width.max(t.height))
        .filter(|&w| w > 0)
        .map(|w| w.clamp(256, 4096).next_power_of_two().min(4096))
        .unwrap_or(DEFAULT_DOCUMENT_SIZE)
}

/// 今のセットでないあいだにしまっておく文書と表示の状態。
pub struct Stash {
    pub doc: Document,
    pub selected_layer: Option<LayerId>,
    pub view: ViewState,
    pub layer_scroll: f32,
}

impl std::fmt::Debug for Stash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Stash")
            .field(
                "doc",
                &(self.doc.width(), self.doc.height(), self.doc.layers().len()),
            )
            .field("selected_layer", &self.selected_layer)
            .finish()
    }
}

impl Stash {
    pub fn new(doc: Document) -> Stash {
        let selected_layer = doc.layers().last().map(|l| l.id());
        Stash {
            doc,
            selected_layer,
            view: ViewState::default(),
            layer_scroll: 0.0,
        }
    }
}

/// テクスチャセット 1 つ。
#[derive(Debug)]
pub struct TextureSet {
    /// プロセスの中で一意の番号（1 から。Live Link のセットの番号にも使う）。
    pub uid: u32,
    /// .ylp のセットの ID。
    pub id: String,
    pub name: String,
    /// 名前をマテリアルから付けたまま（利用者が変えていない。マテリアルに結び付けたとき名前を付け直す）。
    pub auto_name: bool,
    pub material: MaterialRef,
    /// 目（3D ビューと Unity に見せるか。2D のキャンバスでは隠しても描ける。保存しない）。
    pub visible: bool,
    /// 今のモデルの、このセットが受け持つマテリアルの番号（モデルが無い・合わなければ None）。
    pub bound: Option<u32>,
    /// 読むだけの理由（core で扱えない中身がある）。あれば描く・レイヤーを変える・名前を変えるを断る。
    pub read_only: Option<String>,
    /// 最後に開いた・保存した時の文書（id と版）。今の文書と同じなら、保存で正本を書き直さない（開いたファイルのバイト列のまま）。
    pub saved: Option<(u128, u64)>,
    /// 焼いたメッシュマップ（種類ごとに最後の 1 枚。文書の外の派生物で、Undo に入らない。.ylp にセットごとに保存する）。
    pub mesh_maps: crate::bake::MeshMapSet,
    stash: Option<Stash>,
}

impl TextureSet {
    pub fn is_current(&self) -> bool {
        self.stash.is_none()
    }
}

/// テクスチャセットの並び（上から表示する順）と今のセット。
#[derive(Debug)]
pub struct TextureSets {
    list: Vec<TextureSet>,
    current: usize,
}

/// セットの uid（プロセスの中で一意。開き直しても前のセットと重ならないので、Live Link に出していた前のセットと取り違えない）。
fn next_uid() -> u32 {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// 最初のセットの名前。
pub const FIRST_SET_NAME: &str = "テクスチャセット 1";

impl TextureSets {
    /// モデル無しの最初のセット 1 つ（文書は呼ぶ側の `AppState.doc`。鍵はスロット 0 = 最初に読んだモデルの最初のスロットのマテリアル）。
    pub fn first(doc: &Document) -> TextureSets {
        TextureSets {
            list: vec![TextureSet {
                uid: next_uid(),
                id: guid_string(doc.id()),
                name: FIRST_SET_NAME.into(),
                auto_name: true,
                material: MaterialRef::PendingSlot(0),
                visible: true,
                bound: None,
                read_only: None,
                saved: None,
                mesh_maps: Default::default(),
                stash: None,
            }],
            current: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }
    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }
    pub fn iter(&self) -> std::slice::Iter<'_, TextureSet> {
        self.list.iter()
    }
    pub fn get(&self, index: usize) -> Option<&TextureSet> {
        self.list.get(index)
    }
    pub fn get_mut(&mut self, index: usize) -> Option<&mut TextureSet> {
        self.list.get_mut(index)
    }
    pub fn current_index(&self) -> usize {
        self.current
    }
    pub fn current(&self) -> &TextureSet {
        &self.list[self.current]
    }
    pub fn index_of(&self, uid: u32) -> Option<usize> {
        self.list.iter().position(|s| s.uid == uid)
    }
    pub fn by_uid(&self, uid: u32) -> Option<&TextureSet> {
        self.list.iter().find(|s| s.uid == uid)
    }

    /// しまってある文書（今のセットなら None）。
    pub fn stashed_doc(&self, index: usize) -> Option<&Document> {
        self.list.get(index)?.stash.as_ref().map(|s| &s.doc)
    }
    pub fn stashed_doc_mut(&mut self, index: usize) -> Option<&mut Document> {
        self.list.get_mut(index)?.stash.as_mut().map(|s| &mut s.doc)
    }

    /// 開いたセットを並べる（id・名前・鍵・読むだけの理由・文書）。返すのは並びと、今のセット（current）の文書。どのセットも
    /// 「開いた時のまま」の印を付ける。
    pub fn from_parts(
        parts: Vec<(String, String, MaterialRef, Option<String>, Document)>,
        current: usize,
    ) -> (TextureSets, Document) {
        assert!(!parts.is_empty(), "セットが 1 つは要る");
        let current = current.min(parts.len() - 1);
        let mut sets = TextureSets {
            list: Vec::with_capacity(parts.len()),
            current,
        };
        let mut current_doc = None;
        for (i, (id, name, material, read_only, doc)) in parts.into_iter().enumerate() {
            let saved = Some((doc.id(), doc.revision()));
            let index = sets.push(id, name, false, material, read_only, doc);
            sets.list[index].saved = saved;
            if i == current {
                current_doc = sets.list[index].stash.take().map(|s| s.doc);
            }
        }
        (sets, current_doc.expect("今のセットの文書"))
    }

    /// セットを後ろに足す（文書はしまう）。返すのは位置。
    #[allow(clippy::too_many_arguments)]
    pub fn push(
        &mut self,
        id: String,
        name: String,
        auto_name: bool,
        material: MaterialRef,
        read_only: Option<String>,
        doc: Document,
    ) -> usize {
        let uid = next_uid();
        self.list.push(TextureSet {
            uid,
            id,
            name,
            auto_name,
            material,
            visible: true,
            bound: None,
            read_only,
            saved: None,
            mesh_maps: Default::default(),
            stash: Some(Stash::new(doc)),
        });
        self.list.len() - 1
    }
}

/// セットの鍵をモデルのマテリアルに照合する（Unity 版と同じ順: 識別子 → Unassigned → 名前（同じ、次に大文字小文字を区別せず）→
/// スロット）。返すのはマテリアルごとのセットの位置。1 つのセットは 1 つのマテリアルにしか付かない。`slots` はスロットごとの
/// マテリアルの番号（`SceneModel::slots`）。
pub fn match_materials(
    sets: &[&MaterialRef],
    materials: &[LinkKey],
    slots: &[u32],
) -> Vec<Option<usize>> {
    let mut by_material: Vec<Option<usize>> = vec![None; materials.len()];
    let mut taken = vec![false; sets.len()];
    let assign = |by: &mut Vec<Option<usize>>, taken: &mut Vec<bool>, m: usize, s: usize| {
        by[m] = Some(s);
        taken[s] = true;
    };
    // 1. 識別子
    for (mi, m) in materials.iter().enumerate() {
        let LinkKey::Material {
            asset: Some((guid, file_id)),
            ..
        } = m
        else {
            continue;
        };
        let found = (0..sets.len()).find(|&si| {
            !taken[si]
                && matches!(sets[si], MaterialRef::Material { asset: Some(a), .. }
                    if a.guid == *guid && a.file_id == *file_id)
        });
        if let Some(si) = found {
            assign(&mut by_material, &mut taken, mi, si);
        }
    }
    // 2. マテリアルの無いスロット
    for (mi, m) in materials.iter().enumerate() {
        if by_material[mi].is_none() && *m == LinkKey::Unassigned {
            if let Some(si) =
                (0..sets.len()).find(|&si| !taken[si] && *sets[si] == MaterialRef::Unassigned)
            {
                assign(&mut by_material, &mut taken, mi, si);
            }
        }
    }
    // 3・4. 名前（同じ、次に大文字小文字を区別せず）
    for exact in [true, false] {
        for (mi, m) in materials.iter().enumerate() {
            let LinkKey::Material { name, .. } = m else {
                continue;
            };
            if by_material[mi].is_some() {
                continue;
            }
            let same = |a: &str| {
                if exact {
                    a == name
                } else {
                    a.to_lowercase() == name.to_lowercase()
                }
            };
            let found = (0..sets.len()).find(|&si| {
                !taken[si] && matches!(sets[si], MaterialRef::Material { name: n, .. } if same(n))
            });
            if let Some(si) = found {
                assign(&mut by_material, &mut taken, mi, si);
            }
        }
    }
    // 5. まだ結び付けていないスロットの番号
    for si in 0..sets.len() {
        let MaterialRef::PendingSlot(n) = sets[si] else {
            continue;
        };
        if taken[si] {
            continue;
        }
        if let Some(&mi) = slots.get(*n as usize) {
            if (mi as usize) < materials.len() && by_material[mi as usize].is_none() {
                assign(&mut by_material, &mut taken, mi as usize, si);
            }
        }
    }
    by_material
}

/// モデルにセットを結び付けた結果。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BindReport {
    /// 前からあるセットで、マテリアルに付いたものの数。
    pub matched: usize,
    /// 新しく作ったセットの名前。
    pub created: Vec<String>,
    /// モデルに合わなかったセットの名前（残してある）。
    pub unmatched: Vec<String>,
}

impl AppState {
    /// セットの文書（今のセットなら `self.doc`）。
    pub fn set_doc(&self, index: usize) -> &Document {
        if index == self.sets.current_index() {
            &self.doc
        } else {
            self.sets
                .stashed_doc(index)
                .expect("今でないセットは文書をしまっている")
        }
    }

    pub fn set_doc_mut(&mut self, index: usize) -> &mut Document {
        if index == self.sets.current_index() {
            &mut self.doc
        } else {
            self.sets
                .stashed_doc_mut(index)
                .expect("今でないセットは文書をしまっている")
        }
    }

    /// 今のセットが読むだけなら、その理由。
    pub fn read_only_reason(&self) -> Option<&str> {
        self.sets.current().read_only.as_deref()
    }

    /// 今の文書を変えてよいか（描いていない・読むだけでない）。
    pub fn can_edit(&self) -> bool {
        !self.is_stroking() && self.read_only_reason().is_none()
    }

    /// モデルのマテリアルの番号を受け持つセットの位置。
    pub fn set_for_material(&self, material: u32) -> Option<usize> {
        self.sets.iter().position(|s| s.bound == Some(material))
    }

    /// 今のセットを替える（描いている間は断る）。表示（拡大・回転）と選んだレイヤーはセットごとに覚える。
    pub fn switch_set(&mut self, index: usize) -> Result<(), String> {
        if index >= self.sets.len() {
            return Err("そのテクスチャセットはありません。".into());
        }
        if index == self.sets.current_index() {
            return Ok(());
        }
        if self.is_stroking() {
            return Err("描いている間はテクスチャセットを替えません。".into());
        }
        self.doc.end_coalescing();
        let incoming = self.sets.list[index]
            .stash
            .take()
            .expect("今でないセットは文書をしまっている");
        let outgoing = Stash {
            doc: std::mem::replace(&mut self.doc, incoming.doc),
            selected_layer: self.selected_layer,
            view: std::mem::replace(&mut self.view, incoming.view),
            layer_scroll: self.layer_scroll,
        };
        let previous = self.sets.current;
        self.sets.list[previous].stash = Some(outgoing);
        self.sets.current = index;
        self.selected_layer = incoming.selected_layer;
        self.layer_scroll = incoming.layer_scroll;
        // 前の文書のレイヤーを指す途中の操作は捨てる
        self.renaming = None;
        self.layer_drag = None;
        self.popup = None;
        self.ensure_selection();
        self.sync_view3d();
        Ok(())
    }

    /// セットの並びを丸ごと置き換える（開いたとき）。`current` の文書が `self.doc` になる。
    pub fn replace_sets(&mut self, sets: TextureSets, doc: Document) {
        self.sets = sets;
        self.doc = doc;
        self.selected_layer = None;
        self.view = ViewState::default();
        self.layer_scroll = 0.0;
        self.renaming = None;
        self.renaming_set = None;
        self.layer_drag = None;
        self.popup = None;
        self.ensure_selection();
        self.bind_model();
    }

    /// 今のモデル（無ければどれにも付けない）のマテリアルにセットを結び付け、セットの無いマテリアルにはセットを作る。
    /// 3D ビューで描くマテリアル・隠すマテリアルも合わせる。
    pub fn bind_model(&mut self) -> BindReport {
        let report = self.bind_model_to_sets();
        self.sync_view3d();
        report
    }

    fn bind_model_to_sets(&mut self) -> BindReport {
        let mut report = BindReport::default();
        for s in &mut self.sets.list {
            s.bound = None;
        }
        let Some(model) = &self.model else {
            return report;
        };
        let keys: Vec<LinkKey> = model.materials.iter().map(|m| m.key.clone()).collect();
        let matched = {
            let refs: Vec<&MaterialRef> = self.sets.list.iter().map(|s| &s.material).collect();
            match_materials(&refs, &keys, &model.slots)
        };
        let infos = model.materials.clone();
        for (mi, si) in matched.iter().enumerate() {
            let Some(si) = *si else { continue };
            let set = &mut self.sets.list[si];
            set.bound = Some(mi as u32);
            set.material = material_from_link(&keys[mi]);
            report.matched += 1;
            if set.auto_name {
                let base = name_for(&keys[mi]);
                let others: Vec<String> = self
                    .sets
                    .list
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| *i != si)
                    .map(|(_, s)| s.name.clone())
                    .collect();
                self.sets.list[si].name = unique_name(&base, others.iter().map(String::as_str));
            }
        }
        for (mi, si) in matched.iter().enumerate() {
            if si.is_some() {
                continue;
            }
            let size = size_for(&infos[mi]);
            let (doc, _) = blank_document(size, size);
            let name = unique_name(
                &name_for(&keys[mi]),
                self.sets.list.iter().map(|s| s.name.as_str()),
            );
            let index = self.sets.push(
                guid_string(doc.id()),
                name.clone(),
                true,
                material_from_link(&keys[mi]),
                None,
                doc,
            );
            self.sets.list[index].bound = Some(mi as u32);
            report.created.push(name);
        }
        report.unmatched = self
            .sets
            .iter()
            .filter(|s| s.bound.is_none())
            .map(|s| s.name.clone())
            .collect();
        report
    }

    /// セットの名前を変える（読むだけのセットは断る）。
    pub fn rename_set(&mut self, uid: u32, name: &str) -> Result<(), String> {
        let name = name.trim();
        let Some(i) = self.sets.index_of(uid) else {
            return Err("そのテクスチャセットはありません。".into());
        };
        if let Some(reason) = &self.sets.list[i].read_only {
            return Err(format!("読むだけのテクスチャセットです: {reason}"));
        }
        if name.is_empty()
            || name.chars().any(|c| c.is_control())
            || name.encode_utf16().count() > 256
        {
            return Err("テクスチャセットの名前は 1〜256 文字で、制御文字は使えません。".into());
        }
        let upper = name.to_uppercase();
        if self
            .sets
            .list
            .iter()
            .any(|s| s.uid != uid && s.name.to_uppercase() == upper)
        {
            return Err(format!(
                "「{name}」はほかのテクスチャセットと同じ名前です。"
            ));
        }
        let set = &mut self.sets.list[i];
        if set.name != name {
            set.name = name.to_owned();
            set.auto_name = false;
            self.modified = true;
        }
        Ok(())
    }

    /// セットの目を切り替える（3D ビューではそのマテリアルの面を隠す・見せる。Live Link では Unity に出す・外す）。
    pub fn toggle_set_visible(&mut self, uid: u32) {
        if let Some(i) = self.sets.index_of(uid) {
            let set = &mut self.sets.list[i];
            set.visible = !set.visible;
        }
        self.sync_view3d();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::SceneModel;
    use yolu_protocol::{ChannelRoute, MeshData, Model, Submesh, TextureProperty};

    fn asset(name: &str, guid: char, file_id: i64) -> LinkKey {
        LinkKey::Material {
            name: name.into(),
            asset: Some((std::iter::repeat_n(guid, 32).collect(), file_id)),
        }
    }
    fn named(name: &str) -> LinkKey {
        LinkKey::Material {
            name: name.into(),
            asset: None,
        }
    }

    #[test]
    fn matching_follows_the_unity_order() {
        let a = material_from_link(&asset("Skin", 'a', 2100000));
        let renamed = material_from_link(&asset("OldName", 'b', 7));
        let unassigned = MaterialRef::Unassigned;
        let hair = MaterialRef::Material {
            name: "hair".into(),
            asset: None,
        };
        let slot = MaterialRef::PendingSlot(1);
        let orphan = MaterialRef::Material {
            name: "Gone".into(),
            asset: None,
        };
        let sets = [&orphan, &slot, &hair, &unassigned, &renamed, &a];
        let materials = [
            asset("Skin", 'a', 2100000), // 識別子で a
            asset("NewName", 'b', 7),    // 名前が変わっても識別子で renamed
            LinkKey::Unassigned,         // unassigned
            named("Hair"),               // 大文字小文字を区別せずに hair
            named("Eye"),                // スロット 1 のマテリアル → slot
            named("Extra"),              // どれにも合わない → 新しいセット
        ];
        let slots = [0, 4, 5];
        let got = match_materials(&sets, &materials, &slots);
        assert_eq!(got, [Some(5), Some(4), Some(3), Some(2), Some(1), None]);
    }

    #[test]
    fn one_material_gets_one_set_and_exact_names_win() {
        let upper = MaterialRef::Material {
            name: "Skin".into(),
            asset: None,
        };
        let lower = MaterialRef::Material {
            name: "skin".into(),
            asset: None,
        };
        // 小文字のセットが先にあっても、同じ名前のセットが先に取る
        let got = match_materials(&[&lower, &upper], &[named("Skin"), named("SKIN")], &[]);
        assert_eq!(got, [Some(1), Some(0)]);
        // 2 つのセットが同じスロットを指しても、1 つだけが付く
        let s0 = MaterialRef::PendingSlot(0);
        let s0b = MaterialRef::PendingSlot(0);
        let got = match_materials(&[&s0, &s0b], &[named("Body")], &[0]);
        assert_eq!(got, [Some(0)]);
        // 識別子の違うアセットは名前で付く（アセットを複製して GUID が変わった など）
        let old = material_from_link(&asset("Body", 'c', 1));
        let got = match_materials(&[&old], &[asset("Body", 'd', 1)], &[]);
        assert_eq!(got, [Some(0)]);
    }

    fn info(key: LinkKey, size: u32) -> MaterialInfo {
        MaterialInfo {
            key,
            shader: "Standard".into(),
            textures: vec![TextureProperty {
                name: "_MainTex".into(),
                width: size,
                height: size / 2,
            }],
            routes: vec![ChannelRoute {
                channel: channel::COLOR,
                property: "_MainTex".into(),
            }],
        }
    }

    fn scene(materials: Vec<MaterialInfo>) -> SceneModel {
        let n = materials.len() as u32;
        SceneModel::from_link(
            &Model {
                generation: 1,
                name: "試し".into(),
                materials,
                meshes: vec![MeshData {
                    key: "0".into(),
                    name: "Body".into(),
                    skinned: false,
                    positions: vec![[0.0; 3]; 3],
                    normals: vec![],
                    uv0: vec![],
                    submeshes: (0..n)
                        .map(|m| Submesh {
                            material: m,
                            indices: vec![0, 1, 2],
                        })
                        .collect(),
                }],
            },
            1,
        )
    }

    #[test]
    fn sizes_follow_the_color_texture() {
        assert_eq!(size_for(&info(named("a"), 1000)), 1024);
        assert_eq!(size_for(&info(named("a"), 100)), 256);
        assert_eq!(size_for(&info(named("a"), 9000)), 4096);
        assert_eq!(size_for(&info(named("a"), 0)), DEFAULT_DOCUMENT_SIZE);
        let mut no_route = info(named("a"), 512);
        no_route.routes.clear();
        assert_eq!(size_for(&no_route), DEFAULT_DOCUMENT_SIZE);
    }

    #[test]
    fn set_ids_are_the_document_ids_as_written_by_yolu_io() {
        let (doc, _) = blank_document(32, 32);
        let native = yolu_io::NativeDocument::from_core(&doc).unwrap();
        assert_eq!(guid_string(doc.id()), native.id());
        assert_eq!(guid_string(1), "00000000-0000-0000-0000-000000000001");
    }

    #[test]
    fn names_do_not_collide_ignoring_case() {
        let taken = ["Skin", "skin 2", "Hair"];
        assert_eq!(unique_name("SKIN", taken.iter().copied()), "SKIN 3");
        assert_eq!(unique_name("Eye", taken.iter().copied()), "Eye");
        let mut s = AppState::new(64, 64);
        s.model = Some(scene(vec![
            info(named("Skin"), 256),
            info(named("skin"), 256),
        ]));
        s.bind_model();
        let names: Vec<&str> = s.sets.iter().map(|x| x.name.as_str()).collect();
        assert_eq!(names, ["Skin", "skin 2"], "同じ名前の別のマテリアル");
        let uid = s.sets.get(1).unwrap().uid;
        assert!(s.rename_set(uid, "SKIN").is_err());
        assert!(s.rename_set(uid, "Skin2").is_ok());
    }

    #[test]
    fn the_first_set_takes_slot_zero_and_keeps_what_was_painted() {
        let mut s = AppState::new(64, 64);
        let painted = s.doc.id();
        s.model = Some(scene(vec![
            info(asset("Skin", 'a', 5), 512),
            info(LinkKey::Unassigned, 0),
        ]));
        let report = s.bind_model();
        assert_eq!(report.matched, 1);
        assert_eq!(report.created, ["Unassigned"]);
        assert!(report.unmatched.is_empty());
        assert_eq!(s.sets.len(), 2);
        let first = s.sets.get(0).unwrap();
        assert_eq!(first.name, "Skin", "自動の名前はマテリアルの名前に付け直す");
        assert_eq!(first.material, material_from_link(&asset("Skin", 'a', 5)));
        assert_eq!(first.bound, Some(0));
        assert_eq!(
            s.doc.id(),
            painted,
            "最初のセットの文書（描いたもの）はそのまま"
        );
        assert_eq!(s.doc.width(), 64, "大きさも変えない");
        let second = s.sets.get(1).unwrap();
        assert_eq!(second.bound, Some(1));
        assert_eq!(s.set_doc(1).width(), DEFAULT_DOCUMENT_SIZE);
        assert_eq!(s.set_for_material(1), Some(1));
    }

    #[test]
    fn switching_swaps_the_document_and_remembers_the_view() {
        let mut s = AppState::new(64, 64);
        s.model = Some(scene(vec![info(named("A"), 256), info(named("B"), 512)]));
        s.bind_model();
        let a = s.doc.id();
        s.view.zoom = 3.0;
        s.apply(crate::state::Action::NewLayer);
        let a_layer = s.selected_layer;
        assert_eq!(s.switch_set(1), Ok(()));
        assert_eq!(s.sets.current_index(), 1);
        assert_eq!(s.doc.width(), 512);
        assert_ne!(s.doc.id(), a);
        assert_eq!(s.view.zoom, 1.0, "新しいセットは画面に合わせた表示");
        assert_eq!(s.doc.layers().len(), 1);
        assert!(s.selected_layer.is_some());
        assert_eq!(s.set_doc(0).id(), a);
        s.switch_set(0).unwrap();
        assert_eq!(s.doc.id(), a);
        assert_eq!(s.view.zoom, 3.0);
        assert_eq!(s.selected_layer, a_layer);
        assert!(s.switch_set(5).is_err());
    }

    #[test]
    fn switching_is_refused_while_stroking() {
        let mut s = AppState::new(64, 64);
        s.model = Some(scene(vec![info(named("A"), 256), info(named("B"), 256)]));
        s.bind_model();
        let layer = s.selected_layer.unwrap();
        let brush = s.stroke_settings(false);
        s.stroke = Some(s.doc.begin_stroke(layer, &brush).unwrap());
        assert!(s.switch_set(1).is_err());
        assert_eq!(s.sets.current_index(), 0);
        let stroke = s.stroke.take().unwrap();
        s.doc.cancel_stroke(stroke);
        assert!(s.switch_set(1).is_ok());
    }

    #[test]
    fn unmatched_sets_stay_with_their_keys() {
        let mut s = AppState::new(64, 64);
        s.model = Some(scene(vec![info(named("A"), 256)]));
        s.bind_model();
        s.model = Some(scene(vec![info(named("B"), 256)]));
        let report = s.bind_model();
        assert_eq!(report.created, ["B"]);
        assert_eq!(report.unmatched, ["A"]);
        let a = s.sets.get(0).unwrap();
        assert_eq!(a.bound, None);
        assert_eq!(
            a.material,
            MaterialRef::Material {
                name: "A".into(),
                asset: None
            },
            "合わないセットの鍵は変えない"
        );
        // モデルを戻せばまた付く（新しいセットは作らない）
        s.model = Some(scene(vec![info(named("A"), 256)]));
        let report = s.bind_model();
        assert_eq!((report.matched, report.created.len()), (1, 0));
        assert_eq!(s.sets.get(0).unwrap().bound, Some(0));
    }

    #[test]
    fn renaming_stops_the_automatic_name() {
        let mut s = AppState::new(64, 64);
        let uid = s.sets.current().uid;
        assert!(s.rename_set(uid, "  ").is_err());
        s.rename_set(uid, "顔").unwrap();
        s.model = Some(scene(vec![info(named("Skin"), 256)]));
        s.bind_model();
        assert_eq!(s.sets.current().name, "顔", "利用者の付けた名前は残す");
        assert_eq!(s.sets.current().bound, Some(0));
    }
}
