//! UDIM のシート（[`crate::udim`]）の文書の状態と操作: UDIM にする・やめる・タイルを合わせる。どれも 1 回の Undo で、画素（全レイヤー・
//! マスク・選択範囲）と並びを一緒に変える（大きさの変更と同じ交換の段。`resize.rs`）。

use std::sync::Arc;

use super::resize::{Fit, Mapping, ResizeReport, Shifted, Span};
use super::Document;
use crate::surface::PixelReader;
use crate::udim::{CellGrid, UdimLayout};
use crate::{CoreError, Reason, Rgba8};

/// 並べ替えたシートの写し: 行き先の画素のセルのタイルが、元のシートで置かれていたセルの同じ位置を読む（元に無いタイル・空のセルは透明）。
struct Relayout {
    from: Arc<UdimLayout>,
    to: Arc<UdimLayout>,
}

impl Relayout {
    /// 行き先の画素の、元のシートの位置。
    fn source_of(&self, x: u32, y: u32) -> Option<(u32, u32)> {
        let cell = self.to.cell_at(x, y)?;
        let tile = self.to.tile_of_cell(cell)?;
        let from = self.from.cell_of_tile(tile)?;
        let (a, b) = (self.to.cell_rect(cell), self.from.cell_rect(from));
        Some((x - a.x + b.x, y - a.y + b.y))
    }
}

impl Mapping for Relayout {
    // 縦横で別々には決まらない（`span` が全体を見る）。どちらも元の全体を返す（一様なタイルの近道を使わない）
    fn span_x(&self, _lo: u32, _hi: u32) -> Option<Span> {
        let (w, _) = self.from.sheet_size();
        Some(Span {
            lo: 0,
            hi: w as u32 - 1,
            inside: false,
        })
    }
    fn span_y(&self, _lo: u32, _hi: u32) -> Option<Span> {
        let (_, h) = self.from.sheet_size();
        Some(Span {
            lo: 0,
            hi: h as u32 - 1,
            inside: false,
        })
    }
    fn span(&self, x0: u32, x1: u32, y0: u32, y1: u32) -> Option<(Span, Span)> {
        // 行き先の矩形の角が同じセルに入るなら、そのセルの元の矩形だけ（入らなければ全体）
        let (a, b) = (self.to.cell_at(x0, y0), self.to.cell_at(x1, y1));
        if a.is_some() && a == b {
            let (lo, hi) = (self.source_of(x0, y0)?, self.source_of(x1, y1)?);
            return Some((
                Span {
                    lo: lo.0,
                    hi: hi.0,
                    inside: true,
                },
                Span {
                    lo: lo.1,
                    hi: hi.1,
                    inside: true,
                },
            ));
        }
        Some((self.span_x(x0, x1)?, self.span_y(y0, y1)?))
    }
    fn pixel(&self, source: &mut PixelReader<'_>, _normal: bool, x: u32, y: u32) -> Rgba8 {
        match self.source_of(x, y) {
            Some((sx, sy)) => source.pixel(sx as i64, sy as i64),
            None => Rgba8::TRANSPARENT,
        }
    }
}

impl Document {
    /// UDIM のシートの並び（UDIM でなければ None）。
    pub fn udim(&self) -> Option<&Arc<UdimLayout>> {
        self.udim.as_ref()
    }

    /// 近くの画素を読む効果・UV の外への塗り広げの区切り（UDIM ならセル、でなければ None = 文書全体が 1 つ）。
    pub fn udim_cells(&self) -> Option<CellGrid> {
        self.udim.as_ref().map(|l| {
            let (width, height) = l.cell_size();
            CellGrid { width, height }
        })
    }

    /// UV を読む効果がタイルの中の UV（各セルで 0〜1）を作る大きさ（UDIM ならセル、でなければ文書）。
    pub fn uv_cell(&self) -> (u32, u32) {
        self.udim
            .as_ref()
            .map_or((self.width, self.height), |l| l.cell_size())
    }

    /// 読み込みで並びを置く（履歴を作らない。読み込みの最中の、まだ何も評価していない文書だけに使う）。並びのシートが文書の大きさと
    /// 違えば断る。
    pub fn set_udim_for_load(&mut self, layout: Option<UdimLayout>) -> Result<(), CoreError> {
        if let Some(l) = &layout {
            if l.sheet_size() != (u64::from(self.width), u64::from(self.height)) {
                return Err(CoreError::InvalidArgument(Reason::UdimSheetMismatch));
            }
        }
        self.udim = layout.map(Arc::new);
        Ok(())
    }

    /// シートの大きさが辺の上限（[`Document::MAX_NATIVE_SIDE`]）と元画素の予算に収まるか（収まらなければ、タイルの大きさを下げれば入る、
    /// という理由で断る）。
    fn check_sheet(&self, layout: &UdimLayout) -> Result<(u32, u32), CoreError> {
        let (w, h) = layout.sheet_size();
        let side = u64::from(Self::MAX_NATIVE_SIDE);
        if w > side || h > side {
            return Err(CoreError::InvalidArgument(Reason::UdimSheetTooLarge));
        }
        // 1 枚の RGBA8 の面が元画素の予算に入らないシートは、どのレイヤーも描き切れない
        if w * h * 4 > self.source_budget {
            return Err(CoreError::InvalidArgument(Reason::UdimOverBudget));
        }
        Ok((w as u32, h as u32))
    }

    /// UDIM のシートにする（1 回の Undo）。`tiles` は使っているタイル（2 個以上）で、セルは今の文書の大きさ。今の画素（全レイヤー・
    /// マスク・選択範囲）は 1001 のセル（1001 を使っていなければ一番小さい番号のセル）へ置く。
    pub fn enable_udim(&mut self, tiles: Vec<u16>) -> Result<ResizeReport, CoreError> {
        self.ensure_no_stroke()?;
        if self.udim.is_some() {
            return Err(CoreError::InvalidArgument(Reason::AlreadyUdim));
        }
        let layout = UdimLayout::packed(tiles, self.width, self.height)?;
        if layout.tiles().len() < 2 {
            return Err(CoreError::InvalidArgument(Reason::UdimTiles));
        }
        let (w, h) = self.check_sheet(&layout)?;
        let home = layout.cell_of_tile(crate::udim::FIRST_TILE).unwrap_or(0);
        let r = layout.cell_rect(home);
        let offset = (r.x as i32, r.y as i32);
        let map = Shifted {
            offset,
            source: (self.width, self.height),
        };
        let (mut copy, mut report) =
            self.resize_surfaces(w, h, &map, Fit::Shift(offset), &mut || false)?;
        copy.udim = Some(Arc::new(layout));
        self.commit_resized(copy, &mut report)?;
        Ok(report)
    }

    /// UDIM をやめる（1 回の Undo）。1001 のセル（無ければ最初のセル）だけを残して、セルの大きさの普通の文書に戻す。ほかのセルの中身は
    /// 消える（Undo で戻る）。
    pub fn disable_udim(&mut self) -> Result<ResizeReport, CoreError> {
        self.ensure_no_stroke()?;
        let Some(layout) = self.udim.clone() else {
            return Err(CoreError::InvalidArgument(Reason::NotUdim));
        };
        let keep = layout.cell_of_tile(crate::udim::FIRST_TILE).unwrap_or(0);
        let r = layout.cell_rect(keep);
        let offset = (-(r.x as i32), -(r.y as i32));
        let (w, h) = layout.cell_size();
        let map = Shifted {
            offset,
            source: (self.width, self.height),
        };
        let (mut copy, mut report) =
            self.resize_surfaces(w, h, &map, Fit::Shift(offset), &mut || false)?;
        copy.udim = None;
        self.commit_resized(copy, &mut report)?;
        Ok(report)
    }

    /// 並びにタイルを足す（1 回の Undo。今のタイルは消さない・中身のあるセルも消さない）。並べ方が変われば、各タイルのセルの画素を新しい
    /// 位置へ移す。2D のパス・テキスト・2D の定規は、物ごとに、元の並びで中心（パスは点の箱の中心・テキストは基準の点・定規は 2 点の真ん中）の入る
    /// セルのタイルが動いた分だけずらす（パスは新しい位置で描き直す）。足すタイルが無ければ何もせず None。
    pub fn sync_udim(
        &mut self,
        tiles: impl IntoIterator<Item = u16>,
    ) -> Result<Option<ResizeReport>, CoreError> {
        self.ensure_no_stroke()?;
        let Some(from) = self.udim.clone() else {
            return Err(CoreError::InvalidArgument(Reason::NotUdim));
        };
        let to = from.with_tiles(tiles)?;
        if to == *from {
            return Ok(None);
        }
        let (w, h) = self.check_sheet(&to)?;
        let to = Arc::new(to);
        let map = Relayout {
            from,
            to: to.clone(),
        };
        let fit = Fit::Cells {
            from: &map.from,
            to: &map.to,
        };
        let (mut copy, mut report) = self.resize_surfaces(w, h, &map, fit, &mut || false)?;
        copy.udim = Some(to);
        self.commit_resized(copy, &mut report)?;
        Ok(Some(report))
    }
}
