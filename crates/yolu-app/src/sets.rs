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
use crate::notice::Source;
use crate::state::{blank_document_in, AppState, DEFAULT_DOCUMENT_SIZE};

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

pub fn describe_material_in(material: &MaterialRef, lang: crate::lang::Lang) -> String {
    match material {
        MaterialRef::Material { name, .. } => lang.pick(
            format!("マテリアル「{name}」"),
            format!("Material “{name}”"),
        ),
        MaterialRef::Unassigned => lang.pick("マテリアルなし", "No material").into(),
        MaterialRef::PendingSlot(_) => lang.pick("マテリアルに未割り当て", "Unassigned").into(),
    }
}

/// 鍵の詳しい説明（ツールチップ用。アセットの識別子・スロットの番号つき）。
pub fn material_tooltip_in(material: &MaterialRef, lang: crate::lang::Lang) -> String {
    match material {
        MaterialRef::Material {
            name,
            asset: Some(a),
        } => {
            let guid = &a.guid[..a.guid.len().min(8)];
            lang.pick(
                format!("マテリアル「{name}」（アセット {guid}…・{}）", a.file_id),
                format!("Material “{name}” (asset {guid}… · {})", a.file_id),
            )
        }
        MaterialRef::PendingSlot(n) => lang.pick(
            format!("マテリアルに未割り当て（スロット {n}）"),
            format!("Unassigned (slot {n})"),
        ),
        other => describe_material_in(other, lang),
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
        .map(fit_side)
        .unwrap_or(DEFAULT_DOCUMENT_SIZE)
}

/// 絵の長い辺 `longest` から作るセットの辺: 256〜4096 の 2 の冪に丸めたもの（`size_for` と、何も触っていない最初のセットを元の絵の
/// 大きさで作り直すときが、同じ決め方を使う）。
pub fn fit_side(longest: u32) -> u32 {
    longest.clamp(256, 4096).next_power_of_two().min(4096)
}

/// 今の文書を替えるときに、呼ぶ側が決める物（`AppState::install_document`）。
#[derive(Clone, Debug, Default)]
pub struct Keep {
    /// 選ぶレイヤー（None は選ばない。`ensure_selection` が決め直す）。
    pub selected_layer: Option<LayerId>,
    /// 表示（拡大・位置。None は今の表示のまま）。
    pub view: Option<ViewState>,
    /// レイヤーの欄のスクロール（None は今のまま）。
    pub layer_scroll: Option<f32>,
}

impl Keep {
    /// レイヤーを選ばず、表示を既定に、レイヤーの欄を先頭に戻す（大きさの変わりうる別の文書）。
    pub fn reset() -> Keep {
        Keep {
            selected_layer: None,
            view: Some(ViewState::default()),
            layer_scroll: Some(0.0),
        }
    }
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
    /// 読むだけの理由（core で扱えない中身がある・効果の入力がそろわない）。あれば描く・レイヤーを変える・名前を変えるを断る。
    pub read_only: Option<String>,
    /// 読むだけの理由が、効果の入力（メッシュマップ・モデルのルート・画像）がそろわないこと。入力がそろったら編集できるようになる
    /// （core で扱えない中身が理由のときは false で、入力が替わっても読むだけのまま）。
    pub waiting_inputs: bool,
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

/// 最初のセットの名前（言語ごと）。
pub fn first_set_name(lang: crate::lang::Lang) -> &'static str {
    lang.pick(FIRST_SET_NAME, "Texture Set 1")
}

impl TextureSets {
    /// モデル無しの最初のセット 1 つ（文書は呼ぶ側の `AppState.doc`。鍵はスロット 0 = 最初に読んだモデルの最初のスロットのマテリアル）。
    pub fn first(doc: &Document) -> TextureSets {
        Self::first_in(doc, crate::lang::Lang::Ja)
    }

    /// 言語を替えたとき、既定の名前のまま（利用者が変えていない最初のセット）を新しい言語の名前にする。
    /// 利用者が付けた名前・マテリアルから付けた名前、ほかのセットの名前と重なるときは触らない。
    pub fn retitle_defaults(&mut self, from: crate::lang::Lang, to: crate::lang::Lang) {
        let (old, new) = (first_set_name(from), first_set_name(to));
        if old == new {
            return;
        }
        for i in 0..self.list.len() {
            let taken = self
                .list
                .iter()
                .any(|s| s.name.to_uppercase() == new.to_uppercase());
            let set = &mut self.list[i];
            if set.auto_name && set.name == old && !taken {
                set.name = new.into();
            }
        }
    }

    /// `first`の、名前を言語に合わせたもの。
    pub fn first_in(doc: &Document, lang: crate::lang::Lang) -> TextureSets {
        TextureSets {
            list: vec![TextureSet {
                uid: next_uid(),
                id: guid_string(doc.id()),
                name: first_set_name(lang).into(),
                auto_name: true,
                material: MaterialRef::PendingSlot(0),
                visible: true,
                bound: None,
                read_only: None,
                waiting_inputs: false,
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

    /// 今でないセットの文書を入れ替える（選んでいるレイヤーは新しい文書の一番上にする）。
    pub fn replace_stashed_doc(&mut self, index: usize, doc: Document) {
        if let Some(stash) = self.list.get_mut(index).and_then(|s| s.stash.as_mut()) {
            stash.selected_layer = doc.layers().last().map(|l| l.id());
            stash.doc = doc;
        }
    }

    /// 何も触っていない、今でないセットの文書を、同じセットのまま別の文書（別の大きさ）に替える（表示と選んでいるレイヤーは新しい文書に合わせて既定へ）。
    pub(crate) fn replace_untouched_stashed_doc(&mut self, index: usize, doc: Document) {
        if let Some(stash) = self.list.get_mut(index).and_then(|s| s.stash.as_mut()) {
            *stash = Stash::new(doc);
        }
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

    /// 新しく作ったセットを並べる（id・名前・名前をマテリアルから付けたままか・鍵・文書）。`from_parts` と違い、どのセットも「保存した時のまま」の印を付けない
    /// （ファイルに無いセットなので、保存は正本を書く）。返すのは並びと、最初のセットの文書（今のセット）。
    pub fn from_new_parts(
        parts: Vec<(String, String, bool, MaterialRef, Document)>,
    ) -> (TextureSets, Document) {
        assert!(!parts.is_empty(), "セットが 1 つは要る");
        let mut sets = TextureSets {
            list: Vec::with_capacity(parts.len()),
            current: 0,
        };
        for (id, name, auto_name, material, doc) in parts {
            sets.push(id, name, auto_name, material, None, doc);
        }
        let first = sets.list[0].stash.take().map(|s| s.doc);
        (sets, first.expect("最初のセットの文書"))
    }

    /// 今のセットでないセットを外す（uid。文書ごと捨てる。今のセットは呼ぶ側が先に替える）。外したセットを返す。
    pub(crate) fn take_other(&mut self, uid: u32) -> Option<TextureSet> {
        let index = self.index_of(uid)?;
        if index == self.current {
            return None;
        }
        let set = self.list.remove(index);
        if index < self.current {
            self.current -= 1;
        }
        Some(set)
    }

    /// 並びを替える（uid の並び。全部のセットを 1 回ずつ含むときだけ。今のセットは今のまま）。
    pub(crate) fn reorder(&mut self, order: &[u32]) -> bool {
        if order.len() != self.list.len() {
            return false;
        }
        let mut next = Vec::with_capacity(order.len());
        let mut rest = std::mem::take(&mut self.list);
        for uid in order {
            match rest.iter().position(|s| s.uid == *uid) {
                Some(i) => next.push(rest.remove(i)),
                None => {
                    // 足りない・重なる並びは受けない（元に戻す）
                    next.extend(rest);
                    self.list = next;
                    return false;
                }
            }
        }
        self.current = next.iter().position(|s| s.stash.is_none()).unwrap_or(0);
        self.list = next;
        true
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
            waiting_inputs: false,
            saved: None,
            mesh_maps: Default::default(),
            stash: Some(Stash::new(doc)),
        });
        self.list.len() - 1
    }
}

/// 鍵の排他の識別（同じ識別の鍵を 2 つのセットが持てない。名前だけの鍵は重なってよい）。
pub(crate) fn exclusive_key(material: &MaterialRef) -> Option<String> {
    match material {
        MaterialRef::Unassigned => Some("unassigned".into()),
        MaterialRef::PendingSlot(n) => Some(format!("slot:{n}")),
        MaterialRef::Material { asset: Some(a), .. } => {
            Some(format!("asset:{}:{}", a.guid, a.file_id))
        }
        MaterialRef::Material { asset: None, .. } => None,
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
    /// 新しく作ったセットの uid（`created` と同じ並び）。
    pub created_sets: Vec<u32>,
    /// モデルに合わなかったセットの名前（残してある）。
    pub unmatched: Vec<String>,
    /// セットの数の上限（`newproject::MAX_SETS`）に当たって、セットを作らなかったマテリアルの数。
    pub skipped: usize,
}

impl BindReport {
    /// セットを作れなかったマテリアルがあるときの知らせ（無ければ None）。
    pub fn limit_text(&self, lang: crate::lang::Lang) -> Option<String> {
        let limit = crate::newproject::MAX_SETS;
        (self.skipped > 0).then(|| {
            lang.pick(
                format!("テクスチャセットが上限（{limit}）に達したため、{} 個のマテリアルにはセットを作っていません。", self.skipped),
                format!("Texture set limit ({limit}) reached: {} material(s) have no set.", self.skipped),
            )
        })
    }
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
            return Err(self
                .lang
                .pick(
                    "そのテクスチャセットはありません。",
                    "Texture set not found.",
                )
                .into());
        }
        if index == self.sets.current_index() {
            return Ok(());
        }
        if self.is_stroking() {
            return Err(crate::lang::refusals::during_stroke(self.lang).into());
        }
        self.doc.end_coalescing();
        let incoming = self.sets.list[index]
            .stash
            .take()
            .expect("今でないセットは文書をしまっている");
        // 今の文書をしまってから入れる（予算の同期が、ほかのセットのしまった文書を読む）
        let outgoing = Stash {
            doc: std::mem::replace(&mut self.doc, incoming.doc),
            selected_layer: self.selected_layer,
            view: std::mem::take(&mut self.view),
            layer_scroll: self.ui.layer_scroll,
        };
        let previous = self.sets.current;
        self.sets.list[previous].stash = Some(outgoing);
        self.sets.current = index;
        self.settle_installed_document(Keep {
            selected_layer: incoming.selected_layer,
            view: Some(incoming.view),
            layer_scroll: Some(incoming.layer_scroll),
        });
        Ok(())
    }

    /// 今の文書を `doc` に替える（今の文書を替える口はすべてここを通る）。前の文書を指す画面の途中の状態（名前の変更・レイヤーのドラッグ・
    /// ポップアップ・選んだ効果）はいつも戻し、選択・3D ビュー・予算を新しい文書に合わせる。選んだレイヤー・表示・レイヤーのスクロールは `keep` の
    /// とおり。
    pub fn install_document(&mut self, doc: Document, keep: Keep) {
        self.doc = doc;
        self.settle_installed_document(keep);
    }

    /// [`install_document`](Self::install_document) の、文書を入れたあとの段（`switch_set` は今の文書をしまってから入れるので、ここを
    /// 直に呼ぶ）。
    fn settle_installed_document(&mut self, keep: Keep) {
        self.document_replaced();
        self.selected_layer = keep.selected_layer;
        if let Some(view) = keep.view {
            self.view = view;
        }
        if let Some(scroll) = keep.layer_scroll {
            self.ui.layer_scroll = scroll;
        }
        // 前の文書のレイヤーを指す途中の操作は捨てる
        self.ui.renaming = None;
        self.ui.layer_drag = None;
        self.popup = None;
        // 打っている文字は前の文書のもの（まとめた段は前の文書の履歴に残る）。開いた文書のテキストレイヤーのフォントは探し直して知らせる
        self.text.editing = None;
        self.text.press = None;
        self.text.pending = None;
        self.text.move_click = None;
        self.text.move_press = None;
        self.text.forget_fonts();
        self.text.check_fonts = true;
        self.fx.selected = None;
        // 前の文書の座標で打った多角形の点・量を聞くウィンドウは、新しい文書へ持ち越さない
        self.sel_doc_changed();
        self.ensure_selection();
        self.sync_view3d();
        // 予算はプロジェクト全体: 今のセットの文書に、設定からほかのセットの使用量を引いた分を入れ直す
        self.sync_budgets();
    }

    /// 今の文書を別のものに替えた（`doc` に別の `Document` を入れた）ことを知らせる。同じ文書 ID の別の中身（保存した ID が戻る
    /// .ylp や PSD の読み直し）でも、キャンバスの表示は前の文書の合成をここで捨てる。
    pub(crate) fn document_replaced(&mut self) {
        self.doc_epoch = self.doc_epoch.wrapping_add(1);
        // スポイトの印の見本は前の文書を読んだもの（文書の版は別の文書と同じ値になりうる）
        self.eyedrop.sample_cache = None;
        self.canvas.previous_end = None;
        self.canvas.current_end = None;
        self.canvas.shift_hold = None;
        self.canvas.ruler_constraint = None;
        self.canvas.clone_press = None;
        self.canvas.clone_offset = None;
        self.forget_clone_sources();
        self.view3d.input.previous_end = None;
        self.drafting_cancel();
        self.drafting.pen_down = None;
        // 選んでいた定規と、編集のモードで選んだ物・隠した物は前の文書のもの（レイヤーの ID は文書をまたいで同じ値になりうるので、別の文書の同じ番号の
        // レイヤーの物を指さない）
        self.rulers.selected = None;
        self.rulers.icon_drag = None;
        self.objects.selected = None;
        self.objects.hidden.clear();
    }

    /// 何も触っていないセット（`index`）の文書を、同じセット（uid・名前・鍵はそのまま）のまま別の文書に替える。履歴は持ち越さない。
    /// 表示（拡大・位置）と選んでいるレイヤーは、新しい文書の大きさに合わせて既定に戻す。Live Link が、何も触っていない最初のセットを元の絵の
    /// 大きさで作り直すときに使う（触っていないことは呼ぶ側が確かめる）。
    pub(crate) fn swap_untouched_set_document(&mut self, index: usize, doc: Document) {
        self.eyedrop.sample_cache = None;
        if let Some(set) = self.sets.get_mut(index) {
            // 新しく作るセットと同じく、セットの ID は文書の ID と同じ値
            set.id = guid_string(doc.id());
            set.saved = None;
            // 効果の入力の覚えは前の文書へ渡したもの（鍵はセットの uid に付き、文書の ID を含まない）。新しい文書へ渡し直させる
            self.fx.inputs.forget_set(set.uid);
        }
        if index == self.sets.current_index() {
            self.install_document(doc, Keep::reset());
        } else {
            self.sets.replace_untouched_stashed_doc(index, doc);
            self.sync_view3d();
            self.sync_budgets();
        }
    }

    /// セットの並びを丸ごと置き換える（開いたとき）。`current` の文書が `self.doc` になる。
    pub fn replace_sets(&mut self, sets: TextureSets, doc: Document) {
        self.replace_sets_with(sets, doc, true);
    }

    /// `replace_sets` の、今のモデルのマテリアルのうちセットの無いものにセットを作るか選べるもの（新規プロジェクトは、利用者が
    /// 選んだマテリアルだけをセットにする）。
    pub fn replace_sets_with(&mut self, sets: TextureSets, doc: Document, create_missing: bool) {
        self.sets = sets;
        // 効果の状態はプロジェクトのもの（復号した画像・入力の覚えも捨てる）。画像の復号の上限は持ち越す
        let image_limit = self.fx.inputs.image_limit;
        self.fx = Default::default();
        self.fx.inputs.image_limit = image_limit;
        // 新規プロジェクトのウィンドウで作ったときだけ、作ったあとで立てる（ウィンドウで選んだ解像度）
        self.resolution_chosen = false;
        self.ui.renaming_set = None;
        // モデルのマテリアルに結び付けてから文書を入れる（3D ビューの同期は、描くマテリアルが決まったあとの 1 回）。結び付けは
        // セットの並びとモデルだけを読み書きし、今の文書には触らない
        self.bind_model_to_sets(create_missing);
        self.install_document(doc, Keep::reset());
    }

    /// 今のモデル（無ければどれにも付けない）のマテリアルにセットを結び付け、セットの無いマテリアルにはセットを作る。
    /// 3D ビューで描くマテリアル・隠すマテリアルも合わせる。
    pub fn bind_model(&mut self) -> BindReport {
        let report = self.bind_model_to_sets(true);
        self.sync_view3d();
        report
    }

    /// 今のモデルのマテリアルにセットを結び付けるだけ（セットは作らない）。新規プロジェクトで選んだマテリアルだけをセットにしたとき・
    /// 開いたプロジェクトに、読み直したモデルを付けるときは、モデルの残りのマテリアルにセットを増やさない。
    pub fn bind_model_only(&mut self) -> BindReport {
        let report = self.bind_model_to_sets(false);
        self.sync_view3d();
        report
    }

    fn bind_model_to_sets(&mut self, create: bool) -> BindReport {
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
            if si.is_some() || !create {
                continue;
            }
            // .ylp は 64 セットまで。超えるマテリアルにはセットを作らず、数だけ知らせる（新規プロジェクトのウィンドウと同じ扱い）
            if self.sets.list.len() >= crate::newproject::MAX_SETS {
                report.skipped += 1;
                continue;
            }
            let size = size_for(&infos[mi]);
            let (doc, _) = blank_document_in(size, size, self.lang);
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
            report.created_sets.push(self.sets.list[index].uid);
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
            return Err(self
                .lang
                .pick(
                    "そのテクスチャセットはありません。",
                    "Texture set not found.",
                )
                .into());
        };
        if let Some(reason) = &self.sets.list[i].read_only {
            return Err(crate::lang::refusals::read_only_set(self.lang, reason));
        }
        if name.is_empty()
            || name.chars().any(|c| c.is_control())
            || name.encode_utf16().count() > 256
        {
            return Err(self
                .lang
                .pick(
                    "テクスチャセットの名前は 1〜256 文字で、制御文字は使えません。",
                    "Invalid texture set name (1–256 characters, no control characters).",
                )
                .into());
        }
        let upper = name.to_uppercase();
        if self
            .sets
            .list
            .iter()
            .any(|s| s.uid != uid && s.name.to_uppercase() == upper)
        {
            return Err(self.lang.pick(
                format!("「{name}」はほかのテクスチャセットと同じ名前です。"),
                format!("Another texture set is already named {name}."),
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

    /// 空のテクスチャセットを足して今のセットにする（テクスチャセットのパネルの足すボタン）: 今のセットと同じ大きさ・使うチャンネル・
    /// Normal の設定の、空のレイヤー 1 枚。モデルにセットの無いマテリアルがあれば最初のそれに付け、無ければモデルのどのマテリアルにも付けない
    /// （鍵は空いている仮のスロットの番号。プロジェクトの構成でマテリアルを選ぶ）。足したセットの uid を返す。
    pub fn add_texture_set(&mut self) -> Result<u32, String> {
        let lang = self.lang;
        if self.is_stroking() {
            return Err(crate::lang::refusals::during_stroke(lang).into());
        }
        if self.sets.len() >= crate::newproject::MAX_SETS {
            return Err(lang.pick(
                format!(
                    "1 つのプロジェクトのテクスチャセットは {} までです。",
                    crate::newproject::MAX_SETS
                ),
                format!(
                    "A project has at most {} texture sets.",
                    crate::newproject::MAX_SETS
                ),
            ));
        }
        let groups = self
            .model
            .as_ref()
            .map(crate::newproject::groups_of)
            .unwrap_or_default();
        let free = groups
            .iter()
            .find(|g| self.sets.iter().all(|s| s.bound != Some(g.index as u32)));
        let doc = {
            let like = &self.doc;
            crate::newproject::new_set_document(
                like.width(),
                like.height(),
                like.tile_size(),
                lang,
                &crate::newproject::used_channels(like),
                like.normal_settings(),
            )?
        };
        let base = match free {
            Some(g) => g.name.clone(),
            None => format!(
                "{} {}",
                lang.pick("テクスチャセット", "Texture Set"),
                self.sets.len() + 1
            ),
        };
        let name = unique_name(&base, self.sets.iter().map(|s| s.name.as_str()));
        let key = match free {
            Some(g) => g.key.clone(),
            None => MaterialRef::PendingSlot(self.unbound_pending_slot(&[])),
        };
        let uid_index = self
            .sets
            .push(guid_string(doc.id()), name.clone(), true, key, None, doc);
        let uid = self.sets.get(uid_index).expect("足した").uid;
        if let Some(set) = self.sets.get_mut(uid_index) {
            set.bound = free.map(|g| g.index as u32);
        }
        self.modified = true;
        self.switch_set(uid_index)?;
        self.info(
            Source::TextureSet,
            lang.pick(
                format!("テクスチャセット {name} を追加しました。"),
                format!("Added the texture set {name}."),
            ),
        );
        Ok(uid)
    }

    /// 仮のスロットの番号のうち、どのセットも使っておらず、モデルのスロットの数より後のもの（マテリアルに結び付けないセットの仮の鍵）。
    /// `also` は、まだセットに入れていない鍵（構成の下書き）。
    pub fn unbound_pending_slot(&self, also: &[&MaterialRef]) -> u16 {
        let used = |n: u16| {
            self.sets
                .iter()
                .any(|s| s.material == MaterialRef::PendingSlot(n))
                || also.iter().any(|k| **k == MaterialRef::PendingSlot(n))
        };
        let mut slot = self
            .model
            .as_ref()
            .map_or(0, |m| m.slots.len().min(u16::MAX as usize) as u16);
        while used(slot) && slot < u16::MAX {
            slot += 1;
        }
        slot
    }

    /// セットを消す（uid。その作業・履歴・焼いたメッシュマップも消え、取り消せない。確かめは呼ぶ側）。全部は消せない。今のセットを消すなら、
    /// 並びの次（無ければ前）のセットが今のセットになる。消したセットの名前を返す。
    pub fn remove_sets(&mut self, uids: &[u32]) -> Result<Vec<String>, String> {
        let lang = self.lang;
        if self.is_stroking() {
            return Err(crate::lang::refusals::during_stroke(lang).into());
        }
        let mut gone: Vec<u32> = uids
            .iter()
            .copied()
            .filter(|u| self.sets.index_of(*u).is_some())
            .collect();
        gone.sort_unstable();
        gone.dedup();
        if gone.is_empty() {
            return Ok(Vec::new());
        }
        if gone.len() >= self.sets.len() {
            return Err(lang
                .pick(
                    "プロジェクトには少なくとも 1 つのテクスチャセットが要ります。",
                    "A project keeps at least one texture set.",
                )
                .into());
        }
        let current = self.sets.current_index();
        if gone.contains(&self.sets.current().uid) {
            // 並びの次、無ければ前のセットを今のセットにする
            let n = self.sets.len();
            let next = (current + 1..n)
                .chain((0..current).rev())
                .find(|i| self.sets.get(*i).is_some_and(|s| !gone.contains(&s.uid)))
                .expect("全部は消さない");
            self.switch_set(next)?;
        }
        let mut names = Vec::new();
        for uid in &gone {
            if let Some(set) = self.sets.take_other(*uid) {
                names.push(set.name.clone());
                self.bake.skipped.remove(uid);
                if self.ui.renaming_set == Some(*uid) {
                    self.ui.renaming_set = None;
                }
            }
        }
        self.ui.set_scroll = 0.0;
        self.sync_mesh_map_view();
        self.sync_view3d();
        self.modified = true;
        Ok(names)
    }

    /// 2 つのセットが同じ排他の鍵（アセット・Unassigned・仮のスロットの番号）を持たないようにする（保存は重なりを断るので）。
    /// マテリアルに付いたセットの鍵を残し、ほかのセットの鍵は、Unassigned は名前に、仮のスロットの番号はモデルの外の番号に、
    /// アセットは名前だけに替える。
    pub fn dedupe_set_keys(&mut self) {
        let mut seen = std::collections::HashSet::new();
        let order: Vec<usize> = (0..self.sets.len())
            .filter(|i| self.sets.get(*i).is_some_and(|s| s.bound.is_some()))
            .chain(
                (0..self.sets.len())
                    .filter(|i| self.sets.get(*i).is_some_and(|s| s.bound.is_none())),
            )
            .collect();
        for i in order {
            let Some(key) = self.sets.get(i).and_then(|s| exclusive_key(&s.material)) else {
                continue;
            };
            if seen.insert(key) {
                continue;
            }
            let (material, name) = {
                let set = self.sets.get(i).expect("範囲内");
                (set.material.clone(), set.name.clone())
            };
            let next = match material {
                MaterialRef::PendingSlot(_) => {
                    MaterialRef::PendingSlot(self.unbound_pending_slot(&[]))
                }
                MaterialRef::Unassigned | MaterialRef::Material { .. } => {
                    MaterialRef::Material { name, asset: None }
                }
            };
            if let Some(key) = exclusive_key(&next) {
                seen.insert(key);
            }
            if let Some(set) = self.sets.get_mut(i) {
                set.material = next;
            }
        }
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

/// 試験の支え: 今の文書を替える口の試験（`install_document`）が、前の文書を指す画面の途中の状態を立てて、戻ったことを確かめる。
#[cfg(test)]
pub(crate) mod install_testing {
    use crate::state::{AppState, OpenPopup, PopupKind};

    /// 名前の変更・レイヤーのドラッグ・ポップアップ・効果の選びを、今の文書のレイヤーに向けて立て、レイヤーの欄をずらす。
    pub(crate) fn stir(app: &mut AppState) {
        let layer = app
            .selected_layer
            .or_else(|| app.doc.layers().first().map(|l| l.id()))
            .expect("レイヤーがある");
        app.ui.renaming = Some(layer);
        app.ui.layer_drag = Some(crate::m2::LayerDrag {
            id: layer,
            target: None,
        });
        app.popup = Some(OpenPopup {
            kind: PopupKind::LayerContext(layer),
            state: crate::ui::menu::PopupState::new(&egui::Context::default(), egui::Rect::ZERO),
        });
        app.fx.selected = Some(crate::fx::Selected::Anchor {
            id: yolu_core::AnchorId(1),
        });
        app.ui.layer_scroll = 37.0;
    }

    /// 前の文書を指す途中の状態が戻った。
    pub(crate) fn assert_settled(app: &AppState, what: &str) {
        assert!(app.ui.renaming.is_none(), "{what}: 名前の変更");
        assert!(app.ui.layer_drag.is_none(), "{what}: レイヤーのドラッグ");
        assert!(app.popup.is_none(), "{what}: ポップアップ");
        assert!(app.fx.selected.is_none(), "{what}: 効果の選び");
    }

    /// 表示を既定から動かす。
    pub(crate) fn zoom(app: &mut AppState) -> crate::canvas::view::ViewState {
        app.view.zoom = 3.0;
        app.view
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
        SceneModel::from_link(&Model {
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
        })
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
        let (doc, _) = crate::state::blank_document(32, 32);
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

    // ───────── 今の文書を替える口 ─────────

    use super::install_testing::{assert_settled, stir, zoom};

    /// 64 × 64 の 2 つ目のセットを足した状態。
    fn with_second_set() -> AppState {
        let mut s = AppState::new(32, 32);
        let (doc, _) = crate::state::blank_document(64, 64);
        s.sets.push(
            guid_string(doc.id()),
            "B".into(),
            false,
            MaterialRef::Unassigned,
            None,
            doc,
        );
        s
    }

    #[test]
    fn switching_sets_settles_the_ui_and_keeps_each_sets_view_and_scroll() {
        let mut s = with_second_set();
        let first_view = zoom(&mut s);
        stir(&mut s);
        s.switch_set(1).unwrap();
        assert_settled(&s, "switch_set");
        assert_eq!(
            s.view,
            crate::canvas::view::ViewState::default(),
            "入れ替え先の表示"
        );
        assert_eq!(s.ui.layer_scroll, 0.0, "入れ替え先のスクロール");
        assert!(s.selected_layer.is_some(), "選び直す");
        stir(&mut s);
        s.switch_set(0).unwrap();
        assert_settled(&s, "switch_set（戻す）");
        assert_eq!(s.view, first_view, "しまっていた表示");
        assert_eq!(s.ui.layer_scroll, 37.0, "しまっていたスクロール");
    }

    #[test]
    fn swapping_an_untouched_document_settles_the_ui_and_resets_the_view() {
        let mut s = AppState::new(32, 32);
        zoom(&mut s);
        stir(&mut s);
        let (doc, _) = crate::state::blank_document(64, 64);
        s.swap_untouched_set_document(s.sets.current_index(), doc);
        assert_settled(&s, "swap_untouched_set_document");
        assert_eq!(s.view, crate::canvas::view::ViewState::default());
        assert_eq!(s.ui.layer_scroll, 0.0);
        assert_eq!(s.doc.width(), 64);
    }

    #[test]
    fn replacing_the_sets_settles_the_ui_and_resets_the_view() {
        let mut s = AppState::new(32, 32);
        zoom(&mut s);
        stir(&mut s);
        s.ui.renaming_set = Some(s.sets.current().uid);
        let (doc, _) = crate::state::blank_document(64, 64);
        let sets = TextureSets::first(&doc);
        s.replace_sets_with(sets, doc, true);
        assert_settled(&s, "replace_sets_with");
        assert!(s.ui.renaming_set.is_none(), "セットの名前の変更");
        assert_eq!(s.view, crate::canvas::view::ViewState::default());
        assert_eq!(s.ui.layer_scroll, 0.0);
        assert!(s.selected_layer.is_some());
    }
}
