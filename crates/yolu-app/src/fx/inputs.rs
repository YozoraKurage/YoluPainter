//! 効果の入力のつなぎ（Unity 版の `SetGeneratorInputs` / `IGeneratorInputs` に当たる）。
//!
//! Generator・形のグラデーション・塗りつぶしの画像と投影・デカールが読むメッシュマップ・モデルのルートの位置・画像は、文書の外のもの。
//! セットごとに今の条件（モデル・セットの文書の大きさ・スロット・ベイクの設定）と照合した状態（最新・古い・未確認）を付けて
//! `Document::set_effect_inputs` へ渡す。使えない入力を読む段は、core が入力のまま通して理由を返す（黒として読まない）。
//!
//! - **毎フレーム見る（`AppState::sync_effect_inputs`）が、作り直すのは鍵が変わったときだけ**: 鍵は、セットごとの
//!   (モデルがあるか・マップごとの実体と状態・文書が指している棚の画像)。焼き直し・モデルの差し替え・ベイクの設定の変更・文書の
//!   大きさの変更で鍵が変わり、core が読むレイヤー（Generator・画像・デカール・グラデーションのあるレイヤー）だけを描き直させる。
//! - **マップの写しは焼いたマップごとに 1 度**: 16 bit の正本（`BakedMeshMap`）から `MapInput`（`Arc` の写し）を作るのは
//!   焼き直したときだけで、状態だけが変わるときは写しを共有したまま状態を付け替える。
//! - **モデルの入力（指紋）は待たない**: モデルが替わったあとの入力は別のスレッドで作る（`bake_input_nowait`）。できるまでは前の入力のまま
//!   （起動の直後など、一度も渡していないときは入力無しのまま）。開くときだけは待って作る。作りかけの入力は、ベイクのウィンドウ・ID の色の
//!   ツール・入力待ちの読むだけのセット・.ylp を開いた直後の照合し直し（`expect_reopen_check`。モデルを読み終えた後の 1 回）のどれも無ければ、
//!   フレームの終わりに手放す（`release_idle_bake_input`）。そのあいだ編集できるセットは、モデルを替えても（ポーズを変えても）前の入力のまま
//!   照合する。
//! - **画像は使うものだけ復号する**: 棚の画像（.ylp の素材）のうち、文書の塗りつぶしレイヤーが指しているものと、画面が先に頼んだもの
//!   （`AppState::use_shelf_image`。文書が指したら文書の分になり、指さなくなれば手放す）、入力待ちの読むだけのセットが指していたものだけ。
//!   大きな画像を全部は開かない。復号した画素の合計は `Project::image_inputs` と同じ 768 MiB で通算して断る（1 枚ずつ復号しても
//!   呼び出しごとに 0 へ戻さない）。復号できなかった理由は画像の中身の鍵とともに覚え、中身が変わるか、持っている分が減れば試し直す。
//! - **モデルのルートの位置と向き**: Live Link の `Model` はルートの変換を運ばず、スタンドアロンのモデルのルートは原点・回転なし。
//!   モデルがあれば原点・回転なし、無ければ分からない（形のグラデーションと位置を読む投影は入力のまま通す）。
//! - **モデルの UV の位相**（レイヤーのフィルターが UV の継ぎ目をまたぐのに使う）: セットのマテリアルの組ごとに 1 つ作って渡す。作るのは安く
//!   （アイランド・継ぎ目・帯の写しは core が初めて要るときに作る）、ポーズで位置だけ変わったモデルでは前の物を使い続ける（`UvTopology::same_layout`）。
//! - **読むだけにする条件**: .ylp を開いたとき、入力がそろわない効果（マップが無い・古い・未確認・大きさ違い・ピンと違う・
//!   ルートが分からない・画像が無い）を持つセットは、保存した合成を見せる読むだけにして、足りない入力を言う。
//!   Anchor を選んでいない・ID の色が無いなど、文書の中で直せる設定の不備は理由にしない（直す画面が要るので編集させる）。
//!   入力がそろったら（ベイクした・モデルを読んだ・画像が棚に入った）、同じセットが編集できるようになる。

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use yolu_core::effects::{ImageId, ImageInput};
use yolu_core::generator::{self, MapKind, MapState};
use yolu_core::geometry::UvTopology;
use yolu_core::mesh_maps::{BakedMeshMap, MeshBakeInput, MeshMapKind, MeshMapState};
use yolu_core::{
    Document, EffectInputs, InactiveEffect, InactiveReason, InactiveTarget, MapInput, ModelFrame,
};

use crate::lang::Lang;
use crate::notice::Source;
use crate::state::AppState;
use crate::view3d::model::ViewModel;

/// Generator が読む種類から、焼いたマップの種類へ。
pub fn mesh_kind(kind: MapKind) -> MeshMapKind {
    match kind {
        MapKind::WorldNormal => MeshMapKind::WorldNormal,
        MapKind::Position => MeshMapKind::Position,
        MapKind::AmbientOcclusion => MeshMapKind::AmbientOcclusion,
        MapKind::Curvature => MeshMapKind::Curvature,
        MapKind::Thickness => MeshMapKind::Thickness,
        MapKind::TangentNormal => MeshMapKind::TangentNormal,
        MapKind::Height => MeshMapKind::Height,
        MapKind::Id => MeshMapKind::Id,
        MapKind::BentNormal => MeshMapKind::BentNormal,
        MapKind::Opacity => MeshMapKind::Opacity,
    }
}

fn map_state(state: MeshMapState) -> MapState {
    match state {
        MeshMapState::Current => MapState::Current,
        MeshMapState::Stale => MapState::Stale,
        MeshMapState::Unverified => MapState::Unverified,
    }
}

/// 文書へ渡した入力の鍵（同じなら渡し直さない）。
#[derive(Clone, Debug, PartialEq, Eq)]
struct SetKey {
    frame: bool,
    /// 渡した UV の位相（実体。無ければ 0）。
    topology: usize,
    /// (マップの種類, 焼いたマップの実体, 状態)。
    maps: Vec<(i32, usize, u8)>,
    /// 渡した画像（ID と中身の鍵）。
    images: Vec<(u128, String)>,
}

/// 復号できなかった画像（失敗は、同じ中身・同じ上限で、持っている分が減らないあいだは試し直さない）。
struct ImageFailure {
    /// 失敗したときの画像の中身の鍵（索引の `content`）。
    content: String,
    /// 失敗したときに持っていた復号済みの画素のバイト数（これが減れば、予算で断った画像を試し直す）。
    used: usize,
    /// 失敗したときの上限（上限が替われば試し直す）。
    limit: usize,
    reason: String,
}

/// 効果の入力の覚え。
#[derive(Default)]
pub struct InputsState {
    /// セット（uid）ごとに、文書へ渡した入力の鍵。
    keys: HashMap<u32, SetKey>,
    /// 入力がそろわず読むだけにしたセットを、前に開き直そうとしたときの鍵とレイヤーの画素の予算（同じ鍵・予算では開き直さない。
    /// 設定で予算を上げれば、同じ鍵でも試し直す）。
    attempts: HashMap<u32, (SetKey, u64)>,
    /// マップの写し（焼いたマップごとに 1 度）。
    maps: HashMap<(u32, MeshMapKind), (Arc<BakedMeshMap>, MapInput)>,
    /// 復号した棚の画像（リソースの ID → 中身の鍵と入力）。
    images: HashMap<ImageId, (String, ImageInput)>,
    /// 画面が先に頼んだ画像（文書がまだ指していないもの。文書が指したら文書の分になるので外す）。
    requested: HashSet<ImageId>,
    /// 入力待ちの読むだけのセット（uid）が、開く前に指していた画像（読むだけのあいだは文書が指さないので覚える。編集できるように
    /// なる・セットが無くなると外す）。
    waiting: HashMap<u32, HashSet<ImageId>>,
    /// 復号できなかった画像（リソースの ID）。
    image_errors: HashMap<ImageId, ImageFailure>,
    /// 復号した画像の画素の合計に許すバイト数（既定は `Project::image_inputs` と同じ 768 MiB。試験が小さくする）。
    pub image_limit: Option<usize>,
    /// 試験用: 文書へ入力を渡した回数。
    pub passed: u64,
    /// マテリアルの組ごとの UV の位相と、それを作った（または同じ位相と確かめた）モデル。モデルは持ち続ける: 番地や世代の数で比べると、
    /// 解放されたモデルの番地に別のモデルが作られたときに、前のモデルの位相を使い続ける。持つのは次の同期までの間（毎フレーム見る）。
    topologies: HashMap<i32, (Arc<ViewModel>, Arc<UvTopology>)>,
}

impl InputsState {
    /// セット（uid）の文書へ渡した入力の覚えを捨てる（文書を別のものに替えたとき。次の同期で、新しい文書へ入力を渡し直す）。
    pub(crate) fn forget_set(&mut self, uid: u32) {
        self.keys.remove(&uid);
        self.attempts.remove(&uid);
    }

    /// 復号できなかった画像の理由。
    pub fn image_error(&self, id: ImageId) -> Option<&str> {
        self.image_errors.get(&id).map(|f| f.reason.as_str())
    }

    /// 復号している画像の画素の合計バイト数。
    pub fn decoded_image_bytes(&self) -> usize {
        self.images
            .values()
            .map(|(_, image)| image.pixels.len())
            .sum()
    }

    /// 復号している画像の数。
    pub fn decoded_image_count(&self) -> usize {
        self.images.len()
    }

    /// この画像を復号して持っているか。
    pub fn has_decoded_image(&self, id: ImageId) -> bool {
        self.images.contains_key(&id)
    }
}

/// 棚のリソースの ID（ハイフン付きの GUID）から、文書の画像の ID へ。
pub fn image_id(resource_id: &str) -> Option<ImageId> {
    let hex = resource_id.replace('-', "");
    (hex.len() == 32)
        .then(|| u128::from_str_radix(&hex, 16).ok())
        .flatten()
        .filter(|v| *v != 0)
        .map(ImageId)
}

/// 画像の ID に対応する棚の画像の ID（小文字のハイフン付き GUID。棚に無ければ `shelf_resource` で見つからない）。
pub fn resource_id(id: ImageId) -> String {
    let h = format!("{:032x}", id.0);
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

/// 棚の画像の色空間（索引の文字と core の型）。
pub fn space_of(resource: &yolu_io::Resource) -> (&'static str, yolu_core::ImageColorSpace) {
    match resource.metadata.get("colorSpace").and_then(|v| v.as_str()) {
        Some("srgb") => ("srgb", yolu_core::ImageColorSpace::Srgb),
        Some("linear") => ("linear", yolu_core::ImageColorSpace::Linear),
        _ => ("unspecified", yolu_core::ImageColorSpace::Unspecified),
    }
}

/// 入力がそろわないことが理由の、効いていない効果（Anchor を選んでいない・ID の色が無いなど、文書の中の設定の不備は含めない）。
/// モデルが無いこと（`NoModel`: アイランドごとのばらつきのアイランドの図を渡せない）も、モデルのルートが無いこと（`MissingFrame`）と同じく入力の不足。
/// アイランドの図を予算で断ったこと（`IslandMap`）は、文書の大きさと予算の設定なので含めない。
pub fn is_input_problem(effect: &InactiveEffect) -> bool {
    use generator::Inactive as I;
    match &effect.reason {
        InactiveReason::Generator(
            I::MissingMap(_)
            | I::StaleMap(_)
            | I::UnverifiedMap(_)
            | I::MapSize(_)
            | I::PinMismatch(_)
            | I::MissingFrame
            | I::NoModel,
        ) => true,
        // 画像が棚に無い・予算を超えて読めない
        InactiveReason::Rejected(_) => matches!(effect.target, InactiveTarget::FillImage(_)),
        _ => false,
    }
}

/// 文書の、入力がそろわない効果の一覧。
pub fn missing_inputs(doc: &Document) -> Vec<InactiveEffect> {
    doc.inactive_effect_list()
        .into_iter()
        .filter(is_input_problem)
        .collect()
}

/// 効果 1 件の名前（レイヤーの名前は利用者のデータなのでそのまま）。
fn what(
    lang: Lang,
    effect: &InactiveEffect,
    channel_name: &dyn Fn(yolu_core::Channel) -> String,
) -> String {
    match effect.target {
        InactiveTarget::Generator { mask, kind } => {
            let name = super::names::generator_name(lang, kind);
            if mask {
                lang.pick(format!("{name}（マスク）"), format!("{name} (mask)"))
            } else {
                name.to_owned()
            }
        }
        InactiveTarget::FillGradient(c) => lang.pick(
            format!("グラデーション（{}）", channel_name(c)),
            format!("Gradient ({})", channel_name(c)),
        ),
        InactiveTarget::Decal => lang.pick("デカール", "Decal").to_owned(),
        InactiveTarget::FillImage(c) => lang.pick(
            format!("画像（{}）", channel_name(c)),
            format!("Image ({})", channel_name(c)),
        ),
        InactiveTarget::FillPoints(c) => lang.pick(
            format!("点のグラデーション（{}）", channel_name(c)),
            format!("Point gradient ({})", channel_name(c)),
        ),
    }
}

/// 文書の効果が読むメッシュマップの種類（Generator・グラデーション・ピンの種類と、位置を読む投影が要る位置・法線）。
/// 読まないマップは文書へ渡さない: 4096² のマップは 1 枚 48〜112 MiB（1〜3 チャンネルの 16 bit と、塗った範囲の 8 bit の掛け算。
/// 測った値ではない）で、渡すと写しも持つ。
pub fn needed_maps(doc: &Document) -> Vec<MeshMapKind> {
    use yolu_core::fill_image::ProjectionMode;
    let mut want = [false; 10];
    fn add_settings(want: &mut [bool; 10], g: &generator::Settings) {
        for kind in g.used_maps().into_iter().chain(g.pins.keys().copied()) {
            want[mesh_kind(kind) as usize] = true;
        }
    }
    for layer in doc.layers() {
        for target in [
            yolu_core::FilterTarget::Content,
            yolu_core::FilterTarget::Mask,
        ] {
            for stage in doc.filters_of(layer.id(), target).unwrap_or(&[]) {
                if let Some(g) = stage.settings().generator_settings() {
                    add_settings(&mut want, g);
                }
            }
        }
        for (_, g) in layer.fill_gradients() {
            add_settings(&mut want, g);
        }
        // モデルの空間の点のグラデーションは、位置のマップを読む
        if layer
            .fill_point_gradients()
            .any(|(_, g)| g.space == yolu_core::fill_points::PointSpace::Model)
        {
            want[MeshMapKind::Position as usize] = true;
        }
        // 位置を読む投影（デカール・トライプラナー・平面ほか）は、位置と法線のマップが要る
        if layer.kind() == yolu_core::LayerKind::Fill
            && layer.projection().mode != ProjectionMode::Uv
        {
            want[MeshMapKind::Position as usize] = true;
            want[MeshMapKind::WorldNormal as usize] = true;
        }
    }
    MeshMapKind::ALL
        .into_iter()
        .filter(|k| want[*k as usize])
        .collect()
}

/// 効いていない塗りつぶしの画像が指す画像の ID（そのレイヤーのそのチャンネルの画像）。
pub fn fill_image_of(doc: &Document, effect: &InactiveEffect) -> Option<ImageId> {
    let InactiveTarget::FillImage(channel) = effect.target else {
        return None;
    };
    doc.layer(effect.layer)?
        .fill_images()
        .find(|(c, _)| *c == channel)
        .map(|(_, id)| id)
}

/// 読むだけにする理由の文（初めの 3 件と数。足りない入力を言う）。`image_failure` は、効いていない画像の行の画像を
/// 復号できなかった理由（無ければ None）。
pub fn missing_text(
    lang: Lang,
    effects: &[InactiveEffect],
    channel_name: &dyn Fn(yolu_core::Channel) -> String,
    image_failure: &dyn Fn(&InactiveEffect) -> Option<String>,
) -> String {
    let shown: Vec<String> = effects
        .iter()
        .take(3)
        .map(|e| {
            format!(
                "「{}」{}: {}",
                e.layer_name,
                what(lang, e, channel_name),
                crate::lang::inactive_effect_reason(lang, e, image_failure(e).as_deref())
            )
        })
        .collect();
    let more = effects.len().saturating_sub(3);
    match lang {
        Lang::Ja => {
            let mut text = shown.join("、");
            if more > 0 {
                text += &format!(" ほか {more} 件");
            }
            format!("効果の入力がそろっていない（{text}）")
        }
        Lang::En => {
            let mut text = shown.join("; ");
            if more > 0 {
                text += &format!(" and {more} more");
            }
            format!("Effect inputs are missing ({text})")
        }
    }
}

impl AppState {
    /// 入力を作るために必要なモデルの入力（指紋）。今のモデルの入力を待つとき（`bake_input_followed`）は、作り終えていなければ None
    /// （`wait` なら作り終えるまで待つ）。待たないとき（ウィンドウを閉じていて、ID の色のツールも開いた直後の照合し直しも無い）は、手元の入力を
    /// モデルが違っても返し、新しく作り始めない（`release_idle_bake_input` が毎フレーム捨てるので、作ってもできない）。
    /// 焼いたマップを 1 枚も持たなければ要らないので、入力無しとして Some(None)。
    fn effect_bake_input(&mut self, wait: bool) -> Option<Option<Arc<MeshBakeInput>>> {
        if self.sets.iter().all(|s| s.mesh_maps.is_empty()) || self.view3d.full_model().is_none() {
            return Some(None); // マップが無い・モデルが無い（照合できない）
        }
        if wait {
            Some(self.bake_input().ok())
        } else if self.bake.window.is_some() {
            // ベイクのウィンドウが入力を作る（同じ入力を別に作り始めて、ウィンドウの「確かめている」を先に終わらせない）
            self.bake_input_ready().map(|r| r.ok())
        } else if self.bake_input_followed() {
            self.bake_input_nowait().map(|r| r.ok())
        } else {
            // 手元の入力が無ければ作る側（上）に来るので、ここには必ずある
            self.bake_input_held().map(|r| r.ok())
        }
    }

    /// 文書の画像として渡すもの（文書が指している棚の画像と、頼まれた画像、入力待ちのセットが指していた画像）を復号する
    /// （済んでいて中身が同じなら何もしない）。持っている画素の合計が上限を超える画像は、理由つきで断る。
    fn refresh_effect_images(&mut self) {
        let mut pointed: HashSet<ImageId> = HashSet::new();
        for i in 0..self.sets.len() {
            if self.sets.get(i).is_some_and(|s| s.read_only.is_some()) {
                continue;
            }
            for layer in self.set_doc(i).layers() {
                pointed.extend(layer.image_ids());
                // パスのリボンの画像（描き直すときに読む）
                pointed.extend(yolu_core::paths::list_images(layer.paths()));
            }
            pointed.extend(crate::look::image_ids(self.set_doc(i)));
        }
        let waiting_sets: HashSet<u32> = self
            .sets
            .iter()
            .filter(|s| s.waiting_inputs)
            .map(|s| s.uid)
            .collect();
        let state = &mut self.fx.inputs;
        let present: HashMap<ImageId, &yolu_io::Resource> = self
            .shelf
            .resources()
            .iter()
            .filter(|r| r.kind == "image")
            .filter_map(|r| image_id(&r.id).map(|id| (id, r)))
            .collect();
        // 頼まれた画像は、棚にあって文書がまだ指していないあいだだけ（指したら文書の分。棚から消えたら要らない）
        state
            .requested
            .retain(|id| present.contains_key(id) && !pointed.contains(id));
        state.waiting.retain(|uid, _| waiting_sets.contains(uid));
        let mut wanted: Vec<ImageId> = pointed
            .iter()
            .chain(state.requested.iter())
            .chain(state.waiting.values().flatten())
            .copied()
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        wanted.sort_by_key(|id| id.0); // 予算で断る画像が、フレームごとに替わらないように
        let wanted_set: HashSet<ImageId> = wanted.iter().copied().collect();
        // 棚から消えた画像・使わなくなった画像・中身の替わった画像は手放す
        state.images.retain(|id, (content, _)| {
            wanted_set.contains(id) && present.get(id).is_some_and(|r| &image_key(r) == content)
        });
        let mut used = state.decoded_image_bytes();
        // 失敗は、同じ中身・同じ上限で、持っている分が減っていないあいだだけ覚える（中身が変われば別の画像、減れば予算で断った画像を通せる）
        let limit = state.image_limit.unwrap_or(yolu_io::MAX_TOTAL_BYTES);
        state.image_errors.retain(|id, failure| {
            wanted_set.contains(id)
                && present
                    .get(id)
                    .is_some_and(|r| image_key(r) == failure.content)
                && used >= failure.used
                && limit == failure.limit
        });
        for id in wanted {
            let Some(resource) = present.get(&id) else {
                continue;
            };
            if state.images.contains_key(&id) || state.image_errors.contains_key(&id) {
                continue;
            }
            match self
                .shelf
                .shelf()
                .image_inputs_within(Some(&[resource.id.as_str()]), used, limit)
            {
                Ok(mut found) => {
                    if let Some((found_id, image)) = found.pop() {
                        used += image.pixels.len();
                        state.images.insert(found_id, (image_key(resource), image));
                    }
                }
                Err(e) => {
                    state.image_errors.insert(
                        id,
                        ImageFailure {
                            content: image_key(resource),
                            used,
                            limit,
                            reason: self.lang.io_error(&e),
                        },
                    );
                }
            }
        }
    }

    /// 棚の画像を効果の入力にする（文書の塗りつぶしレイヤーが指す前に、画面が先に呼ぶ）。復号して全部のセットの文書へ渡し、
    /// 文書の画像の ID を返す（`Document::set_fill_image` に渡せる）。棚に無い・復号できないときは理由。
    pub fn use_shelf_image(&mut self, resource_id: &str) -> Result<ImageId, String> {
        let lang = self.lang;
        let id = image_id(resource_id).ok_or_else(|| {
            lang.pick(
                "その画像はアセットにありません",
                "No such image in the project's assets",
            )
            .to_owned()
        })?;
        let present = self
            .shelf
            .resources()
            .iter()
            .any(|r| r.kind == "image" && r.id == resource_id);
        if !present {
            return Err(lang
                .pick(
                    "その画像はアセットにありません",
                    "No such image in the project's assets",
                )
                .into());
        }
        self.fx.inputs.requested.insert(id);
        self.fx.inputs.image_errors.remove(&id);
        self.refresh_effect_images();
        if let Some(why) = self.fx.inputs.image_error(id).map(str::to_owned) {
            self.fx.inputs.requested.remove(&id); // 断ったものは誰も持たない
            return Err(why);
        }
        self.sync_effect_inputs_with(true);
        Ok(id)
    }

    /// `use_shelf_image` で頼んだ画像を手放す（頼んだ操作が断られて、どのレイヤーも指さないとき）。ほかのレイヤーが指している画像は、そのまま持つ。
    /// 手放さないと、断られた画像が棚から消えるまで復号したまま残り、通算の予算を食い続ける。
    pub fn release_shelf_image(&mut self, id: ImageId) {
        if self.fx.inputs.requested.remove(&id) {
            self.sync_effect_inputs_with(true);
        }
    }

    /// セットのマテリアルの組ごとの UV の位相を、今のモデルに合わせる（モデルが替わっても、UV と隣り合わせが同じなら前の物のまま）。
    fn refresh_topologies(&mut self) {
        let Some(model) = self.view3d.full_model().cloned() else {
            self.fx.inputs.topologies.clear();
            return;
        };
        let materials: HashSet<i32> = (0..self.sets.len())
            .filter_map(|i| self.set_material(i))
            .collect();
        let held = &mut self.fx.inputs.topologies;
        held.retain(|m, _| materials.contains(m));
        for material in materials {
            match held.get_mut(&material) {
                Some((made_from, _)) if Arc::ptr_eq(made_from, &model) => {}
                Some((made_from, topology)) if topology.same_layout(&model.geometry) => {
                    *made_from = model.clone();
                }
                _ => {
                    let topology = UvTopology::new(model.geometry.clone(), Some(material));
                    held.insert(material, (model.clone(), Arc::new(topology)));
                }
            }
        }
    }

    /// 1 つのセットの今の鍵（モデルの入力と、渡す画像から）。`needed` は読むマップの種類（読むだけのセットは文書が無いので全部）。
    fn set_key(
        &self,
        index: usize,
        input: Option<&MeshBakeInput>,
        frame: bool,
        needed: &[MeshMapKind],
    ) -> SetKey {
        let topology = self
            .set_material(index)
            .and_then(|m| self.fx.inputs.topologies.get(&m))
            .map_or(0, |(_, t)| Arc::as_ptr(t) as usize);
        let Some(set) = self.sets.get(index) else {
            return SetKey {
                frame,
                topology,
                maps: Vec::new(),
                images: Vec::new(),
            };
        };
        let expected = (!needed.is_empty()).then(|| self.mesh_map_expectation(index, input));
        let maps = set
            .mesh_maps
            .iter()
            .filter(|m| needed.contains(&m.kind()))
            .filter_map(|m| {
                let state = m.provenance().check(expected.as_ref()?).state;
                Some((
                    m.kind() as i32,
                    Arc::as_ptr(m) as usize,
                    match state {
                        MeshMapState::Current => 0,
                        MeshMapState::Stale => 1,
                        MeshMapState::Unverified => 2,
                    },
                ))
            })
            .collect();
        let mut images: Vec<(u128, String)> = self
            .fx
            .inputs
            .images
            .iter()
            .map(|(id, (content, _))| (id.0, content.clone()))
            .collect();
        images.sort();
        SetKey {
            frame,
            topology,
            maps,
            images,
        }
    }

    /// 1 つのセットの入力を組む（マップの写しは焼いたマップごとに 1 度だけ作る）。
    fn build_effect_inputs(
        &mut self,
        index: usize,
        input: Option<&MeshBakeInput>,
        frame: bool,
        needed: &[MeshMapKind],
    ) -> EffectInputs {
        let topology = self
            .set_material(index)
            .and_then(|m| self.fx.inputs.topologies.get(&m))
            .map(|(_, t)| t.clone());
        let mut inputs = EffectInputs::new()
            .with_frame(frame.then(ModelFrame::default))
            .with_topology(topology);
        let Some(set) = self.sets.get(index) else {
            return inputs;
        };
        let uid = set.uid;
        let expected = self.mesh_map_expectation(index, input);
        let maps: Vec<Arc<BakedMeshMap>> = set
            .mesh_maps
            .iter()
            .filter(|m| needed.contains(&m.kind()))
            .cloned()
            .collect();
        let live: HashSet<MeshMapKind> = maps.iter().map(|m| m.kind()).collect();
        self.fx
            .inputs
            .maps
            .retain(|(u, k), _| *u != uid || live.contains(k));
        for map in maps {
            let state = map_state(map.provenance().check(&expected).state);
            let key = (uid, map.kind());
            let mut item = match self.fx.inputs.maps.get(&key) {
                Some((held, item)) if Arc::ptr_eq(held, &map) => item.clone(),
                _ => match MapInput::from_baked(&map, state) {
                    Ok(item) => {
                        self.fx.inputs.maps.insert(key, (map.clone(), item.clone()));
                        item
                    }
                    Err(_) => continue,
                },
            };
            item.state = state;
            if let Ok(next) = inputs.clone().with_map(item) {
                inputs = next;
            }
        }
        for (id, (_, image)) in &self.fx.inputs.images {
            inputs = inputs.with_image(*id, image.clone());
        }
        inputs
    }

    /// 毎フレーム: 効果の入力（焼いたマップ・モデルのルート・画像）を、変わったセットの文書へ渡す。入力がそろって読むだけを抜けられる
    /// セットがあれば、編集できるようにする。
    pub fn sync_effect_inputs(&mut self) {
        self.sync_effect_inputs_with(false);
    }

    /// `sync_effect_inputs` の、モデルの入力を作り終えるまで待つ版（.ylp を開いた直後など、結果をすぐ使うとき）。
    pub fn sync_effect_inputs_with(&mut self, wait: bool) {
        self.refresh_effect_images();
        self.refresh_topologies();
        let Some(input) = self.effect_bake_input(wait) else {
            return; // モデルの入力を作っている最中（前の入力のまま）
        };
        let frame = self.view3d.full_model().is_some();
        let live: HashSet<u32> = self.sets.iter().map(|s| s.uid).collect();
        self.fx.inputs.keys.retain(|u, _| live.contains(u));
        self.fx.inputs.attempts.retain(|u, _| live.contains(u));
        self.fx.inputs.maps.retain(|(u, _), _| live.contains(u));
        for index in 0..self.sets.len() {
            let Some(set) = self.sets.get(index) else {
                continue;
            };
            let (uid, waiting, read_only) = (set.uid, set.waiting_inputs, set.read_only.is_some());
            if read_only && !waiting {
                continue; // core で扱えない中身: 入力が替わっても編集できるようにならない
            }
            // 読むだけのセットの文書は保存した合成の 1 枚のレイヤー（効果が無い）なので、読むマップは持っている全部で見る
            let needed = if waiting {
                self.sets
                    .get(index)
                    .map(|s| s.mesh_maps.iter().map(|m| m.kind()).collect())
                    .unwrap_or_default()
            } else {
                needed_maps(self.set_doc(index))
            };
            let key = self.set_key(index, input.as_deref(), frame, &needed);
            if waiting {
                // 同じ入力・同じ予算では試し直さない（予算を設定で上げたら、入力が同じでも試す）
                let budget = self.load_source_bytes();
                let attempt = (key.clone(), budget);
                if self.fx.inputs.attempts.get(&uid) == Some(&attempt) {
                    continue;
                }
                self.fx.inputs.attempts.insert(uid, attempt);
                if self.unlock_set(index, input.as_deref(), frame, budget) {
                    self.fx.inputs.keys.insert(uid, key);
                }
                continue;
            }
            let images_match = self.fx.inputs.images.iter().all(|(id, (_, image))| {
                self.set_doc(index)
                    .effect_inputs()
                    .image(*id)
                    .is_some_and(|current| {
                        current.hash == image.hash && current.color_space == image.color_space
                    })
            });
            if self.fx.inputs.keys.get(&uid) == Some(&key) && images_match {
                continue;
            }
            let inputs = self.build_effect_inputs(index, input.as_deref(), frame, &needed);
            // 効果の入力が替わると、文書の版は上がらないまま見える値が変わる（スポイトの印の見本を読み直させる）
            self.eyedrop.sample_cache = None;
            match self.set_doc_mut(index).set_effect_inputs(inputs) {
                Ok(()) => {
                    self.fx.inputs.keys.insert(uid, key);
                    self.fx.inputs.passed += 1;
                }
                Err(e) => self.notify(
                    crate::notice::Kind::of_core(&e),
                    Source::Effect,
                    self.lang.core_error(&e),
                ),
            }
        }
    }

    /// 読むだけにしていたセットの正本を core の文書にして、入力をそろえてみる。そろえば文書を入れ替えて編集できるようにする。
    /// 正本は開くときと同じ予算（`budget`: レイヤーの画素に許すバイト数）で変換する。変換できなければ（予算を超えるなど）、理由を
    /// 読むだけの理由と状態の帯へ出して false（同じ予算のうちは試し直さない）。
    fn unlock_set(
        &mut self,
        index: usize,
        input: Option<&MeshBakeInput>,
        frame: bool,
        budget: u64,
    ) -> bool {
        let Some(set) = self.sets.get(index) else {
            return false;
        };
        let Some(project) = &self.project else {
            return false;
        };
        let Some(native) = project.project().sets().iter().find(|s| s.id == set.id) else {
            return false;
        };
        let selection = native.selection.clone();
        let look = project.project().look(&set.id);
        let received = project.project().received_look(&set.id);
        let saved_selections = project.project().saved_selections(&set.id);
        let mut doc = match crate::project::to_core(&native.document, self.lang, budget) {
            Ok(doc) => doc,
            Err(reason) => {
                let name = set.name.clone();
                self.fail(
                    Source::TextureSet,
                    self.lang.with_reason(
                        self.lang.pick(
                            format!("テクスチャセット「{name}」を編集できません"),
                            format!("Cannot edit texture set \"{name}\""),
                        ),
                        &reason,
                    ),
                );
                if let Some(set) = self.sets.get_mut(index) {
                    set.read_only = Some(reason);
                }
                return false;
            }
        };
        let inputs = self.build_effect_inputs(index, input, frame, &needed_maps(&doc));
        if doc.set_effect_inputs(inputs).is_err() || !missing_inputs(&doc).is_empty() {
            return false;
        }
        // 選択範囲（selection.bin）は開いたときと同じく文書に戻す
        let restored = crate::selection::io::restore_into(&mut doc, selection.as_ref(), self.lang);
        // 見た目の設定（look.json）も開いたときと同じく戻す（読めなければ標準のまま、理由を言う）
        let look_restored = crate::look::io::restore_from(&mut doc, look, self.lang).and(
            crate::look::io::restore_received_from(&mut doc, received, self.lang),
        );
        // 名前を付けて残した選択範囲も戻す（読めない項目は飛ばして理由を言う。ファイルには残る）
        let saved_restored = match saved_selections {
            Ok(read) => crate::selection::io::restore_saved_into(&mut doc, read, self.lang),
            Err(e) => Err(self.lang.io_error(&e)),
        };
        let saved = Some((doc.id(), doc.revision()));
        self.put_set_doc(index, doc);
        if let Some(set) = self.sets.get_mut(index) {
            set.read_only = None;
            set.waiting_inputs = false;
            set.saved = saved;
        }
        let name = self.sets.get(index).map_or("", |s| s.name.as_str());
        let mut text = self.lang.pick(
            format!("入力がそろったので、テクスチャセット「{name}」を編集できます。"),
            format!("Texture set \"{name}\" is editable now that its inputs are in place."),
        );
        // 選択範囲・見た目・覚えた選択範囲を戻せなかったら、開いたときと同じく理由を言う（気をつけること）
        let mut kind = crate::notice::Kind::Info;
        for e in [restored.err(), look_restored.err(), saved_restored.err()]
            .into_iter()
            .flatten()
        {
            text += &format!(" {e}");
            kind = crate::notice::Kind::Warning;
        }
        self.notify(kind, Source::TextureSet, text);
        true
    }

    /// セットの文書を入れ替える（今のセットなら画面の文書。選んでいるレイヤー・選択の状態は新しい文書に合わせる）。
    fn put_set_doc(&mut self, index: usize, doc: Document) {
        self.eyedrop.sample_cache = None;
        if index == self.sets.current_index() {
            // 同じ文書を編集できるようにするだけで大きさは変わらないので、表示は今のまま
            self.install_document(
                doc,
                crate::sets::Keep {
                    selected_layer: None,
                    view: None,
                    layer_scroll: Some(0.0),
                },
            );
        } else {
            self.sets.replace_stashed_doc(index, doc);
            self.sync_view3d();
        }
    }

    /// .ylp を開いた直後に、入力がそろわない効果を持つセットを読むだけにして（保存した合成を見せる）、足りない入力を言う。
    /// 読むだけにしたセットの名前を返す。
    pub fn lock_sets_missing_inputs(&mut self) -> Vec<String> {
        self.sync_effect_inputs_with(true);
        let mut locked = Vec::new();
        for index in 0..self.sets.len() {
            let Some(set) = self.sets.get(index) else {
                continue;
            };
            if set.read_only.is_some() {
                continue;
            }
            let missing = missing_inputs(self.set_doc(index));
            if missing.is_empty() {
                continue;
            }
            let lang = self.lang;
            let (name, id) = (set.name.clone(), set.id.clone());
            let doc = self.set_doc(index);
            let (width, height) = (doc.width(), doc.height());
            let reason = {
                let names = |c: yolu_core::Channel| crate::m2::channel_name(lang, doc, c);
                let failure = |e: &InactiveEffect| {
                    fill_image_of(doc, e)
                        .and_then(|id| self.fx.inputs.image_error(id))
                        .map(str::to_owned)
                };
                missing_text(lang, &missing, &names, &failure)
            };
            let png = self
                .project
                .as_ref()
                .and_then(|p| {
                    p.project()
                        .migrated_entries()
                        .get(&format!("sets/{id}/composite/Color.png"))
                        .cloned()
                })
                .and_then(|b| b.bytes().ok());
            let (mut preview, note) =
                crate::project::preview_document(png.as_deref(), width, height, lang);
            // ベイクの優先と手動の ID の色は、マップが古いかの照合とベイクが読む文書の状態（`mesh_map_expectation`・`bake_target`）。保存した
            // 合成の文書へも写す（写さないと既定の値と照合し、既定でない値で焼いたマップは古いままで、モデルを読んでも編集できるようにならない）
            let _ = preview.restore_bake_priority(doc.bake_priority().clone());
            let _ = preview.restore_id_colors(doc.id_colors().clone());
            let reason = match note {
                Some(n) => format!("{reason}。{n}"),
                None => reason,
            };
            // 読むだけのあいだは文書が指す画像が見えなくなる（保存した合成の 1 枚のレイヤー）ので、そのセットが指していた画像は
            // セットごとに覚え、棚に入ったら復号する（入力がそろうのを待つ。編集できるようになるかセットが無くなれば手放す）
            let wanted: HashSet<ImageId> = self
                .set_doc(index)
                .layers()
                .iter()
                .flat_map(|l| {
                    l.image_ids()
                        .chain(yolu_core::paths::list_images(l.paths()))
                        .collect::<Vec<_>>()
                })
                .collect();
            let uid = self.sets.get(index).map_or(0, |s| s.uid);
            self.fx.inputs.waiting.insert(uid, wanted);
            self.put_set_doc(index, preview);
            if let Some(set) = self.sets.get_mut(index) {
                set.read_only = Some(reason);
                set.waiting_inputs = true;
            }
            self.fx
                .inputs
                .keys
                .remove(&self.sets.get(index).map_or(0, |s| s.uid));
            locked.push(name);
        }
        locked
    }
}

fn image_key(resource: &yolu_io::Resource) -> String {
    format!("{}:{}", resource.content, space_of(resource).0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::m2::Edit;
    use crate::project::open_within;
    use crate::state::Action;
    use yolu_core::{Channel, Rgba8};

    #[test]
    fn resource_ids_round_trip() {
        let id = ImageId(0x0123_4567_89ab_cdef_0011_2233_4455_6677);
        let text = resource_id(id);
        assert_eq!(text, "01234567-89ab-cdef-0011-223344556677");
        assert_eq!(image_id(&text), Some(id));
        assert_eq!(image_id(&text.replace('-', "")), Some(id));
        assert_eq!(image_id("00000000-0000-0000-0000-000000000000"), None);
        assert_eq!(image_id("not-a-guid"), None);
        assert_eq!(image_id("1234"), None);
    }

    const IMAGE: &str = "00000000-0000-4000-8000-000000000001";

    /// 入力待ちの読むだけのセットを編集できるようにするときのレイヤーの画素の予算は、開くときと同じ（設定の予算。`load_source_bytes`）。
    /// 変換できなければ、黙って戻らず、読むだけの理由と状態の帯に予算のことを出す。
    #[test]
    fn a_waiting_set_is_converted_within_the_same_pixel_budget_as_opening_and_says_why_when_over() {
        let dir = std::env::temp_dir().join(format!("yolu-fx-unlock-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("waiting.ylp");
        // 256 × 256 の 4 タイルに画素のあるレイヤーと、プロジェクトにまだ無い画像を指す塗りつぶしレイヤー
        let mut source = AppState::new(256, 256);
        let base = source.selected_layer.unwrap();
        for (x, y) in [(0, 0), (128, 0), (0, 128), (128, 128)] {
            source
                .doc
                .set_pixel(base, x, y, Rgba8::new(1, 2, 3, 255))
                .unwrap();
        }
        source.apply(Action::M2(Edit::NewFill));
        let fill = source.selected_layer.unwrap();
        source
            .doc
            .set_fill_images_for_load(
                fill,
                &[(Channel::Color, image_id(IMAGE).unwrap())],
                Default::default(),
            )
            .unwrap();
        source.apply(Action::SaveProjectAs(path.clone()));
        assert!(!source.modified, "{}", source.message);
        let native = yolu_io::NativeDocument::from_core(&source.doc).unwrap();
        let bytes = crate::project::to_core(
            &yolu_io::SetDocument::in_memory(native.clone()),
            Lang::Ja,
            u64::MAX,
        )
        .unwrap()
        .allocated_bytes();
        assert!(bytes >= 4 * 65536);

        // 開く（予算にちょうど収まる）: 画像が無いので、入力待ちの読むだけ
        let mut a = AppState::new(64, 64);
        open_within(&mut a, &path, bytes);
        assert!(a.sets.current().waiting_inputs, "{}", a.message);
        assert!(a
            .read_only_reason()
            .is_some_and(|r| r.contains("効果の入力がそろっていない")));
        // 画像が棚に入り、復号できた（入力はそろった）
        let mut shelf = yolu_io::shelf::Shelf::new(crate::shelf::SHELF_BUDGET);
        shelf
            .add_image(
                IMAGE,
                "image",
                &[9, 8, 7, 255],
                1,
                1,
                "srgb",
                Default::default(),
            )
            .unwrap();
        a.shelf = crate::shelf::ShelfState::with_shelf(shelf);
        a.refresh_effect_images();
        assert_eq!(a.fx.inputs.decoded_image_count(), 1);

        // 1 バイト足りない予算: 開く経路と同じく予算として断り、理由を読むだけの理由と状態の帯へ出す（黙って戻らない）
        let mut refused = AppState::new(64, 64);
        open_within(&mut refused, &path, bytes - 1);
        let open_reason = refused
            .read_only_reason()
            .expect("開くときも断る")
            .to_owned();
        assert!(!a.unlock_set(0, None, false, bytes - 1));
        let reason = a.read_only_reason().expect("読むだけのまま").to_owned();
        assert!(reason.contains("予算"), "{reason}");
        assert!(
            open_reason.starts_with(&reason),
            "開くときと同じ断りの文: {open_reason}"
        );
        assert!(
            a.message.contains("編集できません") && a.message.contains("予算"),
            "{}",
            a.message
        );
        assert!(a.sets.current().waiting_inputs, "予算を上げれば試せる");

        // ちょうどの予算なら、開く経路と同じく変換できて、入力もそろっているので編集できる
        assert!(a.unlock_set(0, None, false, bytes), "{}", a.message);
        assert!(a.read_only_reason().is_none());
        assert!(!a.sets.current().waiting_inputs);
        assert!(
            a.doc.inactive_effect_list().is_empty(),
            "{:?}",
            a.doc.inactive_effect_list()
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// 入力の不足に数えるもの: モデルが無いこと（アイランドの図を渡せない）はモデルのルートが無いことと同じ。アイランドの図を予算で断ったことは
    /// 文書の設定の側なので数えない。
    #[test]
    fn a_missing_model_is_an_input_problem_but_a_refused_island_map_is_not() {
        let effect = |why: generator::Inactive| InactiveEffect {
            layer: yolu_core::LayerId(1),
            layer_name: "Base".into(),
            target: InactiveTarget::Generator {
                mask: false,
                kind: generator::Kind::UvIslandVariation,
            },
            reason: InactiveReason::Generator(why),
        };
        assert!(is_input_problem(&effect(generator::Inactive::NoModel)));
        assert!(is_input_problem(&effect(generator::Inactive::MissingFrame)));
        assert!(!is_input_problem(&effect(generator::Inactive::IslandMap)));
    }

    /// 読むだけにする理由の文: 復号できなかった画像は、その理由を言う（棚に無いときだけ「プロジェクトに画像が無い」）。
    #[test]
    fn the_read_only_reason_names_why_an_image_could_not_be_read() {
        let effect = |why: &str| InactiveEffect {
            layer: yolu_core::LayerId(1),
            layer_name: "Fill".into(),
            target: InactiveTarget::FillImage(Channel::Color),
            reason: InactiveReason::Rejected(why.into()),
        };
        let names = |_: Channel| "Color".to_owned();
        let missing = [effect("プロジェクトにその画像が無い")];
        let none = |_: &InactiveEffect| None;
        let ja = missing_text(Lang::Ja, &missing, &names, &none);
        assert!(
            ja.contains("プロジェクトに画像が無い") && !ja.contains("読めません"),
            "{ja}"
        );
        let en = missing_text(Lang::En, &missing, &names, &none);
        assert!(en.contains("The image is not in the project"), "{en}");
        // 壊れた PNG・予算超過などで復号できなかったときは、その理由
        let broken = |_: &InactiveEffect| Some("PNGが不正です".to_owned());
        let ja = missing_text(Lang::Ja, &missing, &names, &broken);
        assert!(
            ja.contains("画像を読めません（PNGが不正です）") && !ja.contains("画像が無い"),
            "{ja}"
        );
        let broken = |_: &InactiveEffect| Some("Invalid PNG".to_owned());
        let en = missing_text(Lang::En, &missing, &names, &broken);
        assert!(en.contains("Cannot read the image (Invalid PNG)"), "{en}");
        // 画像以外の効果には使わない
        let other = [InactiveEffect {
            target: InactiveTarget::Decal,
            reason: InactiveReason::Generator(generator::Inactive::MissingFrame),
            ..effect("")
        }];
        let ja = missing_text(Lang::Ja, &other, &names, &broken);
        assert!(!ja.contains("画像を読めません"), "{ja}");
    }

    /// 入力がそろったセットの文書を今のセットへ入れると、前の文書を指す画面の途中の状態とレイヤーの欄のスクロールを戻し、表示は今のまま
    /// （同じ文書を編集できるようにするだけで、大きさは変わらない）。
    #[test]
    fn putting_the_current_sets_document_settles_the_ui_and_keeps_the_view() {
        use crate::sets::install_testing::{assert_settled, stir, zoom};
        let mut s = AppState::new(32, 32);
        let view = zoom(&mut s);
        stir(&mut s);
        let (doc, _) = crate::state::blank_document(32, 32);
        s.put_set_doc(s.sets.current_index(), doc);
        assert_settled(&s, "put_set_doc");
        assert_eq!(s.view, view, "表示は今のまま");
        assert_eq!(s.ui.layer_scroll, 0.0);
        assert!(s.selected_layer.is_some());
    }
}
