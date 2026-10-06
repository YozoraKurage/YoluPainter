//! 文書の合成を見せるテクスチャ。文書を PAGE 画素の頁に分け（大きい文書でも GPU のテクスチャの上限に収める）、
//! 初めと大きさが変わったときに頁を作り直し、あとは core が「変わった」と言うタイルだけを合成して、そのタイルの範囲だけを
//! テクスチャに上げる（egui の `set_partial`。egui-wgpu がその範囲だけ `write_texture` する）。CPU の頁の上げ方（見えている所から・
//! 時間の枠の中で）は [`super::cpu`]。
//! テクスチャの行は core と同じ下から上のまま（行を並べ替えない）。上下は描くときの UV で返す。
//! 見せるだけの写しで、保存の正本ではない（正本は core の straight RGBA8）。乗算済みへの変換は表示のためだけ。
//!
//! 合成は 2 つの道のどちらか。使えるときは GPU の常駐の合成（[`super::gpu`]。表示のテクスチャ 1 枚を egui へそのまま見せる）、
//! 使えない・予算を超える・合成できない機能があるときは理由を覚えて、上の CPU の頁へ落ちる。同時には持たない（落ちるとき・戻る
//! ときにもう一方の資源を手放す）。straight から乗算済みへの変換の式は両方の道で同じ整数の式だが、合成の画素は GPU が f32、
//! CPU が f64 の丸めなので、窓の絵で最大 1、多段の文書で 2 以内ずれ得る（表示だけ。保存・書き出し・3D は CPU の正本）。

use egui::{pos2, Color32, ColorImage, Painter, Pos2, Rect, TextureHandle, TextureOptions};

use super::cpu::{CpuCanvas, FrameBudget, Viewport};
use super::gpu::{CanvasBackend, Fallback, GpuCanvas, Shown};
use super::view::CanvasView;
use crate::engine::{Channel, Document, Rect as DocRect, RowOrder};
use crate::ui::theme as t;

/// 頁の一辺（タイルの倍数）。
pub const PAGE: u32 = 2048;
/// 市松の 1 マス（画面の点）。
const CHECKER_CELL: f32 = 8.0;

/// 画素 1 つがこの点数以上に拡大されたら、補間せずに画素の角を見せる（それ未満は滑らかに）。
pub const NEAREST_FROM_PIXEL_SIZE: f32 = 2.0;

/// 上げた量の記録（試験と状態の表示用）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UploadStats {
    /// 最後の `sync` で上げたタイルの数（作り直しなら全部）。
    pub last_tiles: usize,
    /// 最後の `sync` が全部を作り直したか。
    pub last_rebuilt: bool,
    /// これまでに上げたタイルの数。
    pub total_tiles: usize,
}

pub struct CanvasDisplay {
    /// CPU の頁（タイルごとの状態・上げる順。[`CpuCanvas`]）。
    cpu: CpuCanvas,
    size: (u32, u32),
    checker: Option<TextureHandle>,
    /// 次の `sync` で、この補間で頁を作り直す。
    want_nearest: bool,
    pub stats: UploadStats,
    /// 最後に見た文書の入れ替えの回数（[`CanvasDisplay::set_document_epoch`]）。
    epoch: u64,
    /// GPU の道（装置・常駐の合成・egui に見せたテクスチャ）。
    gpu: GpuCanvas,
    policy: CanvasBackend,
    /// 今の表示がどちらの合成か。
    shown: Shown,
    /// GPU で合成していない理由（GPU なら None）。
    fallback: Option<Fallback>,
    /// 見えている範囲（CPU の頁の上げる順に使う。None なら枠なしで全部を上げてから返る）。
    viewport: Option<Viewport>,
    budget: FrameBudget,
}

impl Default for CanvasDisplay {
    fn default() -> Self {
        CanvasDisplay {
            cpu: CpuCanvas::default(),
            size: (0, 0),
            checker: None,
            want_nearest: false,
            stats: UploadStats::default(),
            epoch: 0,
            gpu: GpuCanvas::default(),
            policy: CanvasBackend::from_env(),
            shown: Shown::Cpu,
            fallback: None,
            viewport: None,
            budget: FrameBudget::default(),
        }
    }
}

impl CanvasDisplay {
    /// 表示と同じチャンネルの合成を、最大 256 点の縮小像にする。行は上から下。
    /// CPU の正本を共用するので表示バックエンドの切り替えには依存しない。
    pub fn thumbnail(
        doc: &Document,
        channel: Channel,
    ) -> Result<ColorImage, crate::engine::CoreError> {
        let (w, h) = (doc.width(), doc.height());
        let scale = (256.0 / w.max(h) as f64).min(1.0);
        let (tw, th) = (
            (w as f64 * scale).round().max(1.0) as usize,
            (h as f64 * scale).round().max(1.0) as usize,
        );
        let mut sums = vec![[0u64; 5]; tw * th];
        let mut buffer = Vec::new();
        for y in (0..h).step_by(128) {
            for x in (0..w).step_by(128) {
                let (bw, bh) = (128.min(w - x), 128.min(h - y));
                buffer.resize((bw * bh * 4) as usize, 0);
                doc.composite_into(
                    channel,
                    DocRect::new(x, y, bw, bh),
                    &mut buffer,
                    RowOrder::BottomUp,
                )?;
                for py in 0..bh {
                    for px in 0..bw {
                        let tx = (x + px) as usize * tw / w as usize;
                        let ty = (y + py) as usize * th / h as usize;
                        let i = ((py * bw + px) * 4) as usize;
                        let p = Color32::from_rgba_unmultiplied(
                            buffer[i],
                            buffer[i + 1],
                            buffer[i + 2],
                            buffer[i + 3],
                        );
                        let sum = &mut sums[(th - 1 - ty) * tw + tx];
                        for c in 0..4 {
                            sum[c] += p[c] as u64;
                        }
                        sum[4] += 1;
                    }
                }
            }
        }
        Ok(ColorImage::new(
            [tw, th],
            sums.into_iter()
                .map(|s| {
                    let c = |i: usize| ((s[i] + s[4] / 2) / s[4].max(1)) as u8;
                    Color32::from_rgba_premultiplied(c(0), c(1), c(2), c(3))
                })
                .collect(),
        ))
    }

    pub fn new() -> CanvasDisplay {
        CanvasDisplay::default()
    }

    /// eframe・試験の描画の状態を渡す（GPU の道が使えるようになる）。None なら CPU の表示だけ。
    pub fn attach_render_state(&mut self, rs: Option<eframe::egui_wgpu::RenderState>) {
        self.gpu.attach(rs);
        self.leave_gpu();
        self.fallback = None;
    }

    /// 文書の入れ替えの回数（`AppState::doc_epoch`）を渡す。変わっていたら、前の文書の合成（CPU の頁・GPU の常駐）を捨てて
    /// 次の `sync` で作り直す。文書 ID が同じでも（保存した ID が戻る読み直し）変更記録の通し番号で差分を読まないため。
    pub fn set_document_epoch(&mut self, epoch: u64) {
        if epoch != self.epoch {
            self.epoch = epoch;
            self.invalidate();
        }
    }

    /// 前の文書の合成を全部捨てる（次の `sync` が全部を作り直す）。装置の初期化の失敗など、文書によらない記憶は残す。
    pub fn invalidate(&mut self) {
        self.gpu.invalidate();
        self.cpu.clear();
        self.fallback = None;
    }

    /// 表示の合成の方針（既定は環境変数 `YOLUPAINTER_CANVAS`、無ければ自動）。次の `sync` から効く。
    pub fn set_backend(&mut self, policy: CanvasBackend) {
        self.policy = policy;
    }

    pub fn backend(&self) -> CanvasBackend {
        self.policy
    }

    /// 今の表示がどちらの合成か。
    pub fn shown(&self) -> Shown {
        self.shown
    }

    /// GPU で合成していない理由（GPU で合成しているなら None）。
    pub fn fallback(&self) -> Option<&Fallback> {
        self.fallback.as_ref()
    }

    /// GPU の道（試験・計測用）。
    pub fn gpu(&self) -> &GpuCanvas {
        &self.gpu
    }

    /// GPU の表示のテクスチャを読み戻す（試験・計測用。乗算済みの RGBA8、行は文書の下から上）。
    pub fn read_gpu_display(&mut self, rect: DocRect) -> Result<Vec<u8>, String> {
        self.gpu.read_display(rect)
    }

    /// GPU の常駐の予算を替える（試験・計測用）。
    pub fn set_gpu_budget(&mut self, bytes: u64) {
        self.gpu.set_budget(bytes);
    }

    /// GPU の道から CPU の道へ移る（GPU の資源を手放し、CPU の頁は次の `sync` で全部を作り直す）。
    fn leave_gpu(&mut self) {
        if self.shown == Shown::Gpu || self.gpu.is_resident() {
            self.gpu.release();
        }
        self.shown = Shown::Cpu;
        self.cpu.clear();
    }

    /// 文書の変わった所をテクスチャに上げる。上げたタイルの数を返す。
    pub fn sync(&mut self, ctx: &egui::Context, doc: &Document) -> usize {
        self.sync_channel(ctx, doc, Channel::Color)
    }

    /// 文書の変わった所をテクスチャに上げる（`channel` の合成を出す）。上げた（合成し直した）タイルの数を返す。
    /// GPU で合成できるときは GPU、できなければ理由を覚えて CPU（毎フレームは GPU を試し直さない）。
    pub fn sync_channel(&mut self, ctx: &egui::Context, doc: &Document, channel: Channel) -> usize {
        match self.gpu.decide(self.policy, doc, channel) {
            None => {
                if self.shown != Shown::Gpu {
                    // CPU の頁を手放して GPU の道へ移る
                    self.cpu.clear();
                    self.shown = Shown::Gpu;
                }
                match self.gpu.sync(doc, channel, self.want_nearest) {
                    Ok(synced) => {
                        self.fallback = None;
                        self.size = (doc.width(), doc.height());
                        self.stats = UploadStats {
                            last_tiles: synced.tiles,
                            last_rebuilt: synced.rebuilt,
                            total_tiles: self.stats.total_tiles + synced.tiles,
                        };
                        return synced.tiles;
                    }
                    Err(reason) => {
                        self.fallback = Some(reason);
                        self.leave_gpu();
                    }
                }
            }
            Some(reason) => {
                if self.shown == Shown::Gpu {
                    self.leave_gpu();
                }
                self.fallback = Some(reason);
            }
        }
        self.sync_cpu(ctx, doc, channel)
    }

    /// 見えている範囲を渡す。渡すと CPU の頁は、見えているタイルから時間の枠の中で順に上げ（残りは次のフレーム）、渡さない（既定）と
    /// 全部を上げてから返る。画面の側はフレームごとに、`sync` の前に渡す。
    pub fn set_viewport(&mut self, viewport: Option<Viewport>) {
        self.viewport = viewport;
    }

    /// CPU の頁でまだ正確でないタイルの数（GPU の道では 0）。
    pub fn pending_tiles(&self) -> usize {
        if self.shown == Shown::Cpu {
            self.cpu.pending()
        } else {
            0
        }
    }

    /// CPU の頁で、見えている範囲（[`CanvasDisplay::set_viewport`]）にあるまだ正確でないタイルの数（範囲が無ければ全部）。
    pub fn pending_visible_tiles(&self) -> usize {
        if self.shown != Shown::Cpu {
            return 0;
        }
        match &self.viewport {
            Some(v) => self.cpu.pending_in(&v.visible),
            None => self.cpu.pending(),
        }
    }

    /// 見えている範囲で、まだ何も見せていない（正確な絵も粗い絵も無い）タイルの数。
    pub fn unshown_visible_tiles(&self) -> usize {
        if self.shown != Shown::Cpu {
            return 0;
        }
        match &self.viewport {
            Some(v) => self.cpu.unshown_in(&v.visible),
            None => self
                .cpu
                .unshown_in(&DocRect::new(0, 0, self.size.0, self.size.1)),
        }
    }

    /// CPU の頁のタイルの状態（試験用）。
    pub fn tile_state(&self, coord: crate::engine::TileCoord) -> Option<super::cpu::TileState> {
        self.cpu.tile_state(coord)
    }

    /// CPU の道: 文書の変わった所を頁のテクスチャに上げる。
    fn sync_cpu(&mut self, ctx: &egui::Context, doc: &Document, channel: Channel) -> usize {
        self.size = (doc.width(), doc.height());
        let schedule = self.viewport.as_ref().map(|v| (v, &self.budget));
        let report = self
            .cpu
            .sync(ctx, doc, channel, self.want_nearest, schedule);
        self.stats = UploadStats {
            last_tiles: report.tiles,
            last_rebuilt: report.rebuilt,
            total_tiles: self.stats.total_tiles + report.tiles,
        };
        report.tiles
    }

    fn checker_texture(&mut self, ctx: &egui::Context) -> egui::TextureId {
        self.checker
            .get_or_insert_with(|| {
                let image = ColorImage::new(
                    [2, 2],
                    vec![
                        t::CHECKER_LIGHT,
                        t::CHECKER_DARK,
                        t::CHECKER_DARK,
                        t::CHECKER_LIGHT,
                    ],
                );
                ctx.load_texture(
                    "canvas-checker",
                    image,
                    TextureOptions {
                        magnification: egui::TextureFilter::Nearest,
                        minification: egui::TextureFilter::Nearest,
                        wrap_mode: egui::TextureWrapMode::Repeat,
                        mipmap_mode: None,
                    },
                )
            })
            .id()
    }

    /// 文書の矩形（画素の座標 x0..x1 × y0..y1）を、写しで回した 4 隅の四角として描く。
    pub(crate) fn quad(
        painter: &Painter,
        texture: egui::TextureId,
        view: &CanvasView,
        corners: [(f64, f64); 4],
        uvs: [Pos2; 4],
    ) {
        super::cpu::quad(painter, texture, view, corners, uvs);
    }

    /// 画素の角を見せる補間にするか（拡大が `NEAREST_FROM_PIXEL_SIZE` 以上か）を次の `sync` へ伝える。変わったら true。
    pub fn set_nearest(&mut self, want: bool) -> bool {
        let changed = want != self.want_nearest;
        self.want_nearest = want;
        changed
    }

    /// 透明の市松と合成の絵を描く（painter はキャンバスの矩形で切ったもの）。
    pub fn paint(&mut self, painter: &Painter, view: &CanvasView) {
        // 補間の切り替えは境を越えたときだけ（作り直し・登録し直しは次のフレームの `sync`）
        if self.set_nearest(view.pixel_size() >= NEAREST_FROM_PIXEL_SIZE) {
            painter.ctx().request_repaint();
        }
        let (w, h) = (self.size.0 as f64, self.size.1 as f64);
        let gpu_texture = if self.shown == Shown::Gpu {
            self.gpu.texture()
        } else {
            None
        };
        if gpu_texture.is_none() && self.cpu.is_empty() {
            return;
        }
        // 市松は画面の大きさが一定（拡大しても 8 点のマス）で、画像と一緒に回る
        let checker = self.checker_texture(painter.ctx());
        let k = view.pixel_size() / (2.0 * CHECKER_CELL);
        let (cu, cv) = (w as f32 * k, h as f32 * k);
        let corners = [(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)];
        Self::quad(
            painter,
            checker,
            view,
            corners,
            [pos2(0.0, cv), pos2(cu, cv), pos2(cu, 0.0), pos2(0.0, 0.0)],
        );
        let full = [
            pos2(0.0, 0.0),
            pos2(1.0, 0.0),
            pos2(1.0, 1.0),
            pos2(0.0, 1.0),
        ];
        if let Some((texture, _)) = gpu_texture {
            // 文書全体の 1 枚。行 0 が文書の y0（下）で、UV の v がそのまま y に比例する（頁と同じ向き）
            Self::quad(painter, texture, view, corners, full);
            return;
        }
        self.cpu.paint(painter, view);
    }

    /// 画素の角を見せる補間か（試験用）。
    pub fn is_nearest(&self) -> bool {
        match (self.shown, self.gpu.texture()) {
            (Shown::Gpu, Some((_, nearest))) => nearest,
            _ => self.cpu.is_nearest(),
        }
    }

    /// 頁の数（試験用。GPU の道では 0）。
    pub fn page_count(&self) -> usize {
        self.cpu.page_count()
    }

    /// 頁の文書の矩形（試験用。無ければ None）。
    pub fn page_rect(&self, index: usize) -> Option<Rect> {
        self.cpu.page_rect(index).map(|p| {
            Rect::from_min_size(
                pos2(p.x as f32, p.y as f32),
                egui::vec2(p.width as f32, p.height as f32),
            )
        })
    }

    /// 頁のテクスチャの番号（試験用）。
    pub fn page_texture(&self, index: usize) -> Option<egui::TextureId> {
        self.cpu.page_texture(index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn painted(size: u32) -> Document {
        let mut doc = Document::new(size, size).unwrap();
        let layer = doc.add_layer("a").unwrap();
        doc.set_pixel(layer, 3, 4, crate::engine::Rgba8::new(10, 20, 30, 255))
            .unwrap();
        doc
    }

    #[test]
    fn without_a_device_the_cpu_pages_show_the_document() {
        let ctx = egui::Context::default();
        let doc = painted(300);
        let mut display = CanvasDisplay::new();
        display.set_backend(CanvasBackend::Auto);
        let tiles = display.sync(&ctx, &doc);
        assert!(tiles >= 9, "初めは全部を作る: {tiles}");
        assert_eq!(display.shown(), Shown::Cpu);
        assert_eq!(display.fallback(), Some(&Fallback::NoDevice));
        assert_eq!(display.page_count(), 1);
        assert!(display.stats.last_rebuilt);
        // 変わらなければ何も上げない
        assert_eq!(display.sync(&ctx, &doc), 0);
        assert!(!display.stats.last_rebuilt);
    }

    #[test]
    fn a_cpu_policy_is_remembered_as_the_reason() {
        let ctx = egui::Context::default();
        let doc = painted(64);
        let mut display = CanvasDisplay::new();
        display.set_backend(CanvasBackend::Cpu);
        display.sync(&ctx, &doc);
        assert_eq!(display.fallback(), Some(&Fallback::Policy));
        assert_eq!(display.backend(), CanvasBackend::Cpu);
    }
}
