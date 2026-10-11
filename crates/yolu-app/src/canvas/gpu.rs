//! キャンバスの表示を GPU の常駐の合成（yolu-gpu の `ResidentCompositor`）で出す道。
//!
//! - 合成は eframe と同じ wgpu の装置で行い、表示のテクスチャを egui のネイティブのテクスチャとして見せる（読み戻しも上げ直しも
//!   しない）。テクスチャは乗算済みで、行は core と同じ下から上（CPU の表示と同じ向き）。乗算済みへの変換の式は CPU の表示
//!   （`Color32::from_rgba_unmultiplied`）と同じ整数の式だが、合成の画素は GPU が f32、CPU が f64 の丸めなので、ウィンドウの絵で最大 1、
//!   多段の文書で 2 以内の差が出得る（表示だけの差）。
//! - GPU が合成できる文書: ラスター・塗りつぶし・マスク・クリッピング・26 の合成モード・チャンネルごとの合成・通過でも独立でも
//!   グループ（不透明度・マスク・クリッピング）・調整レイヤー（全種類）・法線の種類のチャンネル・効果（フィルター・Generator・塗りつぶしの
//!   グラデーションと投影・マスクのフィルター）のある文書。効果の出力は CPU（core）が評価して、タイルとして GPU へ上げる
//!   （効果の計算そのものは CPU のまま）。
//! - 見せるだけの写しで、保存・書き出し・3D ビューの値は core の正本（CPU）から作る。この道は正本を読むだけで書かない。
//! - 使えないときは理由を覚えて CPU の表示へ落ちる（[`Fallback`]）。理由が文書の中身（グループの入れ子が深すぎるなど）なら
//!   文書が変わったときに、装置や予算の失敗なら文書・チャンネル・レイヤーの数が変わったときにだけ、GPU を試し直す。毎フレームは試さない。
//!   文書を別のものに替えたとき（同じ文書 ID でも）は、[`GpuCanvas::invalidate`] で前の文書の常駐と失敗の記憶を捨てて作り直す。
//! - 予算（[`RESIDENT_BUDGET`]）は表示のテクスチャ・入力の GPU のコピーと同量の CPU のコピー・作業域（命令の並びと調整の表を含む）の
//!   合計。描いたタイル（効果のあるレイヤーは持ち得るタイルの上限）を全部常駐させて予算を超える文書は CPU へ落ちる（追い出しながら
//!   合成すると、レイヤーの不透明度などの全面の変更のたびに全タイルを上げ直して、CPU より何倍も遅いため）。予算の境で行き来しないよう、
//!   戻るのは予算の 8 割に収まってから。デバイスの上限を超える大きさは、GPU を試して失敗した理由を覚えて CPU へ落ちる。

use eframe::egui_wgpu::{self, wgpu};
use yolu_gpu::{
    device_can_composite, resident_requirements, GpuPainter, Options, ResidentCompositor,
    ResidentOptions, Unsupported, UpdateStats,
};

use crate::engine::{Channel, Document};
use crate::lang::Lang;

/// 予算を超えて CPU へ落ちたあと、GPU に戻る目安（予算に対する割合。境で行き来しないための余白）。
const RETURN_RATIO: f64 = 0.8;

/// 常駐の合成の予算。yolu-gpu の既定（768 MiB）より小さくする: 3D ビューや egui の資源と同じ装置を使い、OS やほかのアプリと
/// GPU のメモリを分け合うため。4096² の表示のテクスチャが 64 MiB。描いたタイルを全部常駐させて収まらない文書は、追い出しながら
/// 合成せず CPU へ落ちる（見積もりで先に選ぶ。追い出すと全面の変更のたびに全タイルを上げ直して遅い）。yolu-gpu の追い出しは、
/// 見積もりと実際の差（作業域の確保の揺れ）の余白としてだけ働く。表示と 1 束すら入らなければ CPU へ落ちる。
pub const RESIDENT_BUDGET: u64 = 512 << 20;

/// 表示の合成を GPU にするかの方針。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CanvasBackend {
    /// 使えるときは GPU。ソフトウェアのアダプター（CPU が GPU の代わりをする）では、CPU で直に合成するほうが速いので CPU。
    #[default]
    Auto,
    /// 使えるなら GPU（ソフトウェアのアダプターでも）。試験・計測用。
    Gpu,
    /// 常に CPU。
    Cpu,
}

impl CanvasBackend {
    /// 環境変数 `YOLUPAINTER_CANVAS`（auto・gpu・cpu）。無い・読めなければ `Auto`。
    pub fn from_env() -> CanvasBackend {
        Self::parse(std::env::var("YOLUPAINTER_CANVAS").ok().as_deref())
    }

    /// 設定の文字（大文字小文字・前後の空白は問わない）。知らない文字は `Auto`。
    pub fn parse(value: Option<&str>) -> CanvasBackend {
        match value.map(|v| v.trim().to_ascii_lowercase()).as_deref() {
            Some("gpu") => CanvasBackend::Gpu,
            Some("cpu") => CanvasBackend::Cpu,
            _ => CanvasBackend::Auto,
        }
    }
}

/// 今、表示の合成がどちらか。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shown {
    Cpu,
    Gpu,
}

/// GPU で合成しない理由。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fallback {
    /// 方針が CPU。
    Policy,
    /// wgpu の装置が無い（eframe の描画が wgpu でない・試験で装置を渡していない）。
    NoDevice,
    /// ソフトウェアのアダプター（`Auto` のとき）。
    SoftwareAdapter,
    /// 装置が合成に要る上限（compute・storage）を持たない（WebGL2 の上限で作った OpenGL の装置など）。
    DeviceLimits,
    /// この文書の中身を GPU で合成できない（グループの入れ子が深すぎる・文書にないチャンネル）。
    Unsupported(Unsupported),
    /// 描いたタイルを全部常駐させると GPU のメモリの予算を超える（文書が小さくなれば戻る）。
    OverBudget,
    /// GPU の初期化・実行の失敗（デバイスの上限の超過を含む）。
    Failed(Failure),
}

/// GPU の失敗の種類。日英の文は種類から作る（yolu-gpu の文は日本語なので、画面には出さない）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureKind {
    /// 文書の大きさが GPU のテクスチャの上限を超える。
    TextureLimit,
    /// 装置・常駐の初期化（パイプラインの作成・デバイスの上限）に失敗した。
    Init,
    /// 合成の実行（確保・検証・完了待ち）に失敗した。
    Run,
}

/// GPU の失敗。`detail` は yolu-gpu の文（記録・デバッグ用。画面には出さない）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    pub kind: FailureKind,
    pub detail: String,
}

impl Failure {
    fn new(kind: FailureKind, detail: impl Into<String>) -> Failure {
        Failure {
            kind,
            detail: detail.into(),
        }
    }

    /// 種類の短い文。
    pub fn describe(&self, lang: Lang) -> String {
        match self.kind {
            FailureKind::TextureLimit => lang.pick(
                "キャンバスの大きさが GPU のテクスチャの上限を超えます",
                "Canvas is larger than the GPU texture limit",
            ),
            FailureKind::Init => {
                lang.pick("GPU の初期化に失敗しました", "GPU initialization failed")
            }
            FailureKind::Run => lang.pick("GPU での合成に失敗しました", "GPU compositing failed"),
        }
        .into()
    }
}

impl Fallback {
    /// 理由の短い文（ツールチップなどに使える。画面にはまだ出さない）。
    pub fn describe(&self, lang: Lang) -> String {
        match self {
            Fallback::Policy => lang
                .pick("設定で GPU を使わない", "GPU turned off in the settings")
                .into(),
            Fallback::NoDevice => lang.pick("GPU が見つかりません", "No GPU found").into(),
            Fallback::SoftwareAdapter => lang
                .pick("ソフトウェアの GPU です", "Software GPU adapter")
                .into(),
            Fallback::DeviceLimits => lang
                .pick(
                    "GPU の機能が足りません",
                    "The GPU lacks the needed features",
                )
                .into(),
            Fallback::Unsupported(u) => match u {
                Unsupported::UnknownChannel => lang
                    .pick("プロジェクトにないチャンネルです", "Unknown channel")
                    .into(),
                Unsupported::GroupDepth => lang
                    .pick(
                        "グループの入れ子が深すぎて GPU で合成できません",
                        "Groups are nested too deeply to composite on the GPU",
                    )
                    .into(),
            },
            Fallback::OverBudget => lang
                .pick("GPU のメモリの予算を超えます", "Over the GPU memory budget")
                .into(),
            Fallback::Failed(failure) => failure.describe(lang),
        }
    }
}

/// `sync` の結果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Synced {
    /// 合成し直したタイルの数。
    pub tiles: usize,
    /// 全部を作り直したか（初め・文書やチャンネルが替わった）。
    pub rebuilt: bool,
}

/// GPU の失敗を覚える鍵: 同じ文書・大きさ・チャンネル・レイヤーの数では試し直さない。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FailureKey {
    doc: u128,
    size: (u32, u32),
    channel: Channel,
    layers: usize,
}

impl FailureKey {
    fn of(doc: &Document, channel: Channel) -> FailureKey {
        FailureKey {
            doc: doc.id(),
            size: (doc.width(), doc.height()),
            channel,
            layers: doc.layers().len(),
        }
    }
}

/// 文書の版ごとの確認の結果。
#[derive(Clone, Copy, Debug)]
struct Checked {
    supported: Result<(), Unsupported>,
    /// 全部を常駐させるのに要る量（見積もれなければ None。デバイスの上限を超える文書は GPU を試して失敗を覚える）。
    needed: Option<u64>,
}

/// egui に登録した表示のテクスチャ。
struct Registered {
    id: egui::TextureId,
    view: wgpu::TextureView,
    nearest: bool,
}

pub struct GpuCanvas {
    rs: Option<egui_wgpu::RenderState>,
    /// 常駐の予算（既定は [`RESIDENT_BUDGET`]）。
    budget: u64,
    compositor: Option<ResidentCompositor>,
    registered: Option<Registered>,
    /// 今の表示が合成している文書・大きさ・チャンネル（替われば全部を作り直す）。
    bound: Option<(u128, (u32, u32), Channel)>,
    /// 装置や常駐の初期化に失敗した理由（どの文書でも試し直さない）。
    permanent: Option<Failure>,
    failed: Option<(FailureKey, Failure)>,
    /// レイヤーの並びと必要な量の確認の結果（文書の版が同じなら確かめ直さない）。
    checked: Option<((u128, u64, Channel), Checked)>,
    /// 予算を超えて CPU へ落ちている文書とチャンネル（予算の 8 割に収まるまで戻らない）。
    over_budget: Option<(u128, Channel)>,
    /// 最後の更新の記録。
    pub last: Option<UpdateStats>,
    /// 最後に更新した文書の版（同じ版・同じ補間なら更新しない）。
    synced_revision: Option<u64>,
    /// 最後に更新したときの core の変更記録の通し番号（これより小さくなったら、同じ ID の別の文書に替わっている）。
    synced_serial: Option<u64>,
    /// GPU で合成できず CPU へ落ちた回数（同じ条件で試し直していないことの試験用）。
    pub failures: u32,
}

impl Default for GpuCanvas {
    fn default() -> Self {
        GpuCanvas {
            rs: None,
            budget: RESIDENT_BUDGET,
            compositor: None,
            registered: None,
            bound: None,
            permanent: None,
            failed: None,
            checked: None,
            over_budget: None,
            last: None,
            synced_revision: None,
            synced_serial: None,
            failures: 0,
        }
    }
}

impl GpuCanvas {
    /// 常駐の予算。
    pub fn budget(&self) -> u64 {
        self.budget
    }

    /// 常駐の予算を替える（設定の GPU のメモリ・試験・計測用）。GPU の資源を手放し、過去の失敗を忘れて試し直す。
    pub fn set_budget(&mut self, bytes: u64) {
        self.budget = bytes;
        self.release();
        self.permanent = None;
        self.failed = None;
        self.checked = None;
        self.over_budget = None;
    }

    /// 文書を別のものに替えた（同じ文書 ID の別の中身を含む）。前の文書の常駐・表示・確認・失敗の記憶を捨てて、次の `sync` で作り直す。
    /// 装置や常駐の初期化の失敗（どの文書でも同じ）は残す。
    pub fn invalidate(&mut self) {
        self.release();
        self.failed = None;
        self.checked = None;
        self.over_budget = None;
    }

    /// eframe・試験の描画の状態（装置とキューと egui の描画）。None なら GPU の道は使えない。
    pub fn attach(&mut self, rs: Option<egui_wgpu::RenderState>) {
        self.release();
        self.rs = rs;
        self.permanent = None;
        self.failed = None;
        self.checked = None;
        self.over_budget = None;
    }

    /// GPU で合成しない理由（GPU で合成できるなら None）。レイヤーの並びと過去の失敗だけを見る軽い確認で、GPU には触れない。
    pub fn decide(
        &mut self,
        policy: CanvasBackend,
        doc: &Document,
        channel: Channel,
    ) -> Option<Fallback> {
        if policy == CanvasBackend::Cpu {
            return Some(Fallback::Policy);
        }
        let Some(rs) = &self.rs else {
            return Some(Fallback::NoDevice);
        };
        if let Some(failure) = &self.permanent {
            return Some(Fallback::Failed(failure.clone()));
        }
        if let Some((key, failure)) = &self.failed {
            if *key == FailureKey::of(doc, channel) {
                return Some(Fallback::Failed(failure.clone()));
            }
        }
        // 装置が合成のシェーダーを動かせない（予算の見積もりも作れず、予算を超えると見えてしまう）
        if !device_can_composite(&rs.device.limits()) {
            return Some(Fallback::DeviceLimits);
        }
        let key = (doc.id(), doc.revision(), channel);
        let checked = match self.checked {
            Some((k, c)) if k == key => c,
            _ => {
                let options = Self::options(self.budget);
                let c = Checked {
                    supported: yolu_gpu::supports(doc, channel),
                    needed: resident_requirements(doc, channel, &options, &rs.device.limits())
                        .ok()
                        .map(|r| r.total_bytes()),
                };
                self.checked = Some((key, c));
                c
            }
        };
        if let Err(u) = checked.supported {
            return Some(Fallback::Unsupported(u));
        }
        if policy == CanvasBackend::Auto
            && rs.adapter.get_info().device_type == wgpu::DeviceType::Cpu
        {
            return Some(Fallback::SoftwareAdapter);
        }
        if let Some(needed) = checked.needed {
            let this = (doc.id(), channel);
            let budget = self.budget;
            let limit = if self.over_budget == Some(this) {
                (budget as f64 * RETURN_RATIO) as u64
            } else {
                budget
            };
            if needed > limit {
                self.over_budget = Some(this);
                return Some(Fallback::OverBudget);
            }
            self.over_budget = None;
        }
        None
    }

    fn options(budget: u64) -> ResidentOptions {
        ResidentOptions {
            resident_budget_bytes: budget,
            // 表示を読み戻さない（読み戻しの予算は使わない）
            readback_budget_bytes: 1 << 20,
            premultiplied_display: true,
            ..Default::default()
        }
    }

    fn create(rs: &egui_wgpu::RenderState, budget: u64) -> Result<ResidentCompositor, Failure> {
        let init = |e: yolu_gpu::GpuError| Failure::new(FailureKind::Init, e.to_string());
        let painter = GpuPainter::from_device(
            rs.adapter.get_info(),
            rs.device.clone(),
            rs.queue.clone(),
            Options::default(),
        )
        .map_err(init)?;
        ResidentCompositor::with_gpu(painter, Self::options(budget)).map_err(init)
    }

    /// 表示のテクスチャを文書の今の状態へ更新して、egui に見せる。`decide` が None のときに呼ぶ。失敗したら GPU の資源を手放し、
    /// 理由を覚える（同じ条件では試し直さない）。
    pub fn sync(
        &mut self,
        doc: &Document,
        channel: Channel,
        nearest: bool,
    ) -> Result<Synced, Fallback> {
        let Some(rs) = self.rs.clone() else {
            return Err(Fallback::NoDevice);
        };
        let key = (doc.id(), (doc.width(), doc.height()), channel);
        // 変更記録の通し番号が戻っていたら、同じ ID の別の文書（読み直し）に替わっている。差分で更新できないので作り直す
        // （通し番号が偶然大きいままの読み直しは、呼び手が `invalidate` で知らせる）。
        if self.bound == Some(key)
            && self
                .synced_serial
                .is_some_and(|serial| doc.change_serial() < serial)
        {
            self.invalidate();
        }
        if self.compositor.is_none() {
            match Self::create(&rs, self.budget) {
                Ok(c) => self.compositor = Some(c),
                Err(failure) => {
                    self.permanent = Some(failure.clone());
                    self.failures += 1;
                    return Err(Fallback::Failed(failure));
                }
            }
        }
        // 文書も補間も前のフレームのままなら、何もしない（毎フレームのレイヤーの並びの確認も省く）
        if self.bound == Some(key)
            && self.synced_revision == Some(doc.revision())
            && self
                .registered
                .as_ref()
                .is_some_and(|r| r.nearest == nearest)
        {
            return Ok(Synced {
                tiles: 0,
                rebuilt: false,
            });
        }
        let rebuilt = self.bound != Some(key);
        let compositor = self.compositor.as_mut().expect("作った");
        let stats = match compositor.update(doc, channel) {
            Ok(stats) => stats,
            Err(e) => return Err(self.fail(&rs, doc, channel, e.to_string())),
        };
        let view = match compositor.display() {
            Ok(d) => d.view.clone(),
            Err(e) => return Err(self.fail(&rs, doc, channel, e.to_string())),
        };
        self.bound = Some(key);
        self.synced_revision = Some(doc.revision());
        self.synced_serial = Some(doc.change_serial());
        self.last = Some(stats);
        self.register(&rs, view, nearest);
        Ok(Synced {
            tiles: stats.updated_tiles,
            rebuilt,
        })
    }

    /// 失敗を覚えて GPU の資源を手放す（同じ文書・大きさ・チャンネル・レイヤーの数では試し直さない）。種類は、文書の大きさが装置の
    /// テクスチャの上限を超えるか（超えるなら `TextureLimit`）で決め、yolu-gpu の文は `detail` に持つだけにする。
    fn fail(
        &mut self,
        rs: &egui_wgpu::RenderState,
        doc: &Document,
        channel: Channel,
        detail: String,
    ) -> Fallback {
        self.release();
        let kind = Self::failure_kind(doc, rs.device.limits().max_texture_dimension_2d);
        let failure = Failure::new(kind, detail);
        self.failed = Some((FailureKey::of(doc, channel), failure.clone()));
        self.failures += 1;
        Fallback::Failed(failure)
    }

    /// 合成の失敗の種類（表示のテクスチャの上限を超える文書か、それ以外の実行の失敗か）。
    fn failure_kind(doc: &Document, max_texture_dimension: u32) -> FailureKind {
        if doc.width() > max_texture_dimension || doc.height() > max_texture_dimension {
            FailureKind::TextureLimit
        } else {
            FailureKind::Run
        }
    }

    /// 表示のテクスチャを egui に登録する（テクスチャか補間が替わったときだけ登録し直す）。
    fn register(&mut self, rs: &egui_wgpu::RenderState, view: wgpu::TextureView, nearest: bool) {
        if self
            .registered
            .as_ref()
            .is_some_and(|r| r.view == view && r.nearest == nearest)
        {
            return;
        }
        let filter = if nearest {
            wgpu::FilterMode::Nearest
        } else {
            wgpu::FilterMode::Linear
        };
        let sampler = wgpu::SamplerDescriptor {
            label: Some("yolu-canvas"),
            mag_filter: filter,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        };
        let mut renderer = rs.renderer.write();
        let id = match &self.registered {
            Some(r) => {
                renderer.update_egui_texture_from_wgpu_texture_with_sampler_options(
                    &rs.device, &view, sampler, r.id,
                );
                r.id
            }
            None => {
                renderer.register_native_texture_with_sampler_options(&rs.device, &view, sampler)
            }
        };
        drop(renderer);
        self.registered = Some(Registered { id, view, nearest });
    }

    /// 見せるテクスチャ（egui の id と、表示のテクスチャの補間）。
    pub fn texture(&self) -> Option<(egui::TextureId, bool)> {
        self.registered.as_ref().map(|r| (r.id, r.nearest))
    }

    /// GPU の資源を手放す（egui の登録を外し、常駐のタイルと表示のテクスチャを捨てる）。失敗の記憶は残す。
    pub fn release(&mut self) {
        if let (Some(rs), Some(r)) = (&self.rs, self.registered.take()) {
            rs.renderer.write().free_texture(&r.id);
        }
        self.compositor = None;
        self.bound = None;
        self.synced_revision = None;
        self.synced_serial = None;
        self.last = None;
    }

    /// 表示のテクスチャの中身を読み戻す（試験・計測用。乗算済みの RGBA8 で、行は文書の下から上）。表示を持っていなければ Err。
    pub fn read_display(&mut self, rect: crate::engine::Rect) -> Result<Vec<u8>, String> {
        let compositor = self
            .compositor
            .as_mut()
            .ok_or_else(|| "GPU の表示がありません".to_string())?;
        let request = compositor
            .request_readback(rect)
            .map_err(|e| e.to_string())?;
        compositor
            .finish_readback(request)
            .map_err(|e| e.to_string())
    }

    /// GPU の常駐が今持っている（試験用）。
    pub fn is_resident(&self) -> bool {
        self.compositor.is_some()
    }

    /// アダプターの情報（装置があれば。試験・計測用）。
    pub fn adapter_info(&self) -> Option<wgpu::AdapterInfo> {
        self.rs.as_ref().map(|rs| rs.adapter.get_info())
    }
}

impl Drop for GpuCanvas {
    fn drop(&mut self) {
        self.release();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_setting_is_parsed_leniently() {
        assert_eq!(CanvasBackend::parse(None), CanvasBackend::Auto);
        assert_eq!(CanvasBackend::parse(Some("GPU")), CanvasBackend::Gpu);
        assert_eq!(CanvasBackend::parse(Some(" cpu ")), CanvasBackend::Cpu);
        assert_eq!(CanvasBackend::parse(Some("auto")), CanvasBackend::Auto);
        assert_eq!(CanvasBackend::parse(Some("vulkan")), CanvasBackend::Auto);
    }

    fn has_japanese(text: &str) -> bool {
        text.chars()
            .any(|c| matches!(c, '\u{3000}'..='\u{30ff}' | '\u{4e00}'..='\u{9fff}' | '\u{ff00}'..='\u{ffef}'))
    }

    #[test]
    fn every_reason_reads_in_both_languages() {
        let reasons = [
            Fallback::Policy,
            Fallback::NoDevice,
            Fallback::SoftwareAdapter,
            Fallback::DeviceLimits,
            Fallback::Unsupported(Unsupported::UnknownChannel),
            Fallback::Unsupported(Unsupported::GroupDepth),
            Fallback::OverBudget,
        ];
        // 失敗は種類ごとに、yolu-gpu の日本語の文を持っていても、画面の言語の文だけを出す
        let failures = [
            FailureKind::TextureLimit,
            FailureKind::Init,
            FailureKind::Run,
        ]
        .map(|kind| Fallback::Failed(Failure::new(kind, "表示テクスチャがデバイス上限を超える")));
        for reason in reasons.iter().chain(&failures) {
            let ja = reason.describe(Lang::Ja);
            let en = reason.describe(Lang::En);
            assert!(!ja.is_empty() && !en.is_empty());
            assert!(has_japanese(&ja), "{ja}");
            assert!(!has_japanese(&en), "{en}");
            assert_ne!(ja, en);
        }
        let texts: std::collections::HashSet<String> =
            failures.iter().map(|f| f.describe(Lang::En)).collect();
        assert_eq!(texts.len(), 3, "種類ごとに違う文");
    }

    /// 実際の yolu-gpu のエラー文（日本語）で: 英語の画面には日本語が混ざらず、上限の失敗は上限の文になる。
    #[test]
    fn real_yolu_gpu_errors_never_leak_into_the_english_reason() {
        let wide = Document::with_tile_size(20_000, 16, 16).unwrap();
        let limits = wgpu::Limits::default();
        let e = resident_requirements(
            &wide,
            Channel::Color,
            &GpuCanvas::options(RESIDENT_BUDGET),
            &limits,
        )
        .unwrap_err();
        assert!(has_japanese(&e.to_string()), "yolu-gpu の文は日本語: {e}");
        let kind = GpuCanvas::failure_kind(&wide, limits.max_texture_dimension_2d);
        assert_eq!(kind, FailureKind::TextureLimit);
        let failure = Failure::new(kind, e.to_string());
        assert!(!has_japanese(&failure.describe(Lang::En)));
        assert!(has_japanese(&failure.describe(Lang::Ja)));
        assert!(!failure.describe(Lang::Ja).contains(&e.to_string()));
        // 上限に収まる文書の失敗は実行の失敗
        let small = Document::with_tile_size(64, 64, 16).unwrap();
        assert_eq!(
            GpuCanvas::failure_kind(&small, limits.max_texture_dimension_2d),
            FailureKind::Run
        );
    }
}
