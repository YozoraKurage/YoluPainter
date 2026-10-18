//! リボン（[`TipShape::ribbon`]）: 筆先の画像を、ストロークの線の長さに沿って続けて写し、1 つの模様にする。
//!
//! - 画像の横の軸を線の向きに、縦の軸を線の幅に当てる（画像の高さがダブの直径。線の長さ方向の 1 枚の長さは 直径 × 幅 ÷ 高さ）。
//!   ダブの向きは線の向きだけで決め、筆先の角度・傾き・回転・角度のゆらぎは使わない（帯ごとに回すと模様がつながらない）。
//! - 各描点は、線の長さの「ここまで写した所」（`frontier`）から、そのダブの線の長さ + 間隔の半分までの帯だけを写す。帯の境は前の描点の帯の
//!   終わりと同じ値なので、まっすぐな線では帯の継ぎ目に抜けも重なりも無い（画素の中心で、帯の内か外かを決める）。曲がった線の外側は
//!   帯の扇の分だけ薄く、内側は重なる（保証しない）。押した点の描点は線の向きがまだ分からないので何も写さず、そこから写し始める
//!   （動かさないクリックは何も置かない）。
//! - 模様の進みは、写した長さ ÷ そのダブの半径（半径の単位）で数える。筆圧などで太さが変われば模様も伸び縮みし、つながりは保つ。
//! - 複数の筆先は、1 枚を写し切るごとに繰り返し方法（[`TipSelection`]）の順で次の 1 枚を選ぶ（ランダムは自分の乱数の列から引く）。1 つの帯が
//!   筆先の境をまたぐときは、筆先ごとの帯（ダブ）に分けて置く。一巡のみ・ランダムに一巡で使い切ったら、そのストロークは何も置かない。
//! - 描いた所から 1 間隔より遅れた帯（面に当たらなかったダブの後など）は、遅れを 1 間隔までに詰める（模様は進めない）。時刻で置いた
//!   ダブ（連続吹き付け）は線の長さが進まないので、帯が空で何も置かない。
//! - 1 つの描点が写す帯は [`MAX_RIBBON_PIECES`] 枚まで（筆先がとても細長いとき）。打ち切った残りは捨てずに、次の描点が `frontier` から
//!   続けて写す（抜けを作らない）。残りが [`MAX_RIBBON_BACKLOG_GAPS`] 間隔分を超えて溜まったら（写しが描点に間に合わない筆先と間隔）、
//!   ストロークを予算の断りで取り消す。
//!
//! 画素の覆いは、帯の範囲と画像の位置を持つダブの形（[`RibbonCut`]）を行の核（`rows`）と画素ごとの式が同じ式で測る。線の向きの端は
//! 画像の端の画素を延ばして読む（筆先の継ぎ目で、双線形の半画素の薄れを作らない）。

use super::*;

/// 1 つの描点の帯の上限の枚数。
pub const MAX_RIBBON_PIECES: usize = 64;
/// 打ち切って写し残した帯の長さの上限（描点の間隔の何個分か）。超えたら予算の断り。
pub const MAX_RIBBON_BACKLOG_GAPS: f64 = 16.0;
/// 筆先の選びのランダムの乱数の列の種へ混ぜる値（ほかの列と重ならない）。
const RIBBON_STREAM: i32 = 0x3C6E_F372;

/// リボンのダブの帯: 線の向きの範囲 [lo, hi)（ダブの中心から、線の向きに測った距離）と、筆先の画像の中心の位置（同じ向き）。単位はダブの半径。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RibbonCut {
    pub lo: f64,
    pub hi: f64,
    pub center: f64,
}

/// 1 つの帯: 写す筆先（並びの番号。None は `TipShape::image`）と反転、範囲。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RibbonPiece {
    pub tip: Option<usize>,
    pub flip_x: bool,
    pub flip_y: bool,
    pub cut: RibbonCut,
}

/// リボンのストロークごとの進み（[`TipRun`] が持つ）。
#[derive(Clone, Debug)]
pub(crate) struct RibbonRun {
    /// ここまで写した線の長さ（まだ写していなければ None）。
    frontier: Option<f64>,
    /// 今の筆先と反転、その筆先の中の位置（左の端からの、半径の単位の長さ）。
    current: Option<(Option<usize>, bool, bool)>,
    phase: f64,
    /// 一巡のみ・ランダムに一巡で使い切った。
    done: bool,
    random: NetRandom,
    /// 前の描点が枚数の上限で打ち切って、`frontier` から先を写し残している（次の描点は `frontier` から続ける）。
    behind: bool,
    /// 直前に帯を決めた描点の番号と、その帯（同じ描点の数の分のダブは同じ帯を使う）。
    stamp: Option<u64>,
    pieces: Vec<RibbonPiece>,
}

impl RibbonRun {
    pub(crate) fn new(seed: i32) -> RibbonRun {
        RibbonRun {
            frontier: None,
            current: None,
            phase: 0.0,
            done: false,
            random: NetRandom::new(seed ^ RIBBON_STREAM),
            behind: false,
            stamp: None,
            pieces: Vec::new(),
        }
    }
}

/// 筆先の画像の、線の長さ方向の 1 枚の長さ（半径の単位: 2 × 幅 ÷ 高さ）。
fn tip_length(tip: &BrushTip) -> f64 {
    2.0 * tip.width() as f64 / tip.height() as f64
}

impl Brush {
    /// 帯の筆先（並びの番号。None は `tip.image`）。
    pub(crate) fn ribbon_tip(&self, index: Option<usize>) -> Option<&BrushTip> {
        match index {
            None => self.tip.image.as_deref(),
            Some(i) => self.tip.images.get(i).map(|t| t.as_ref()),
        }
    }

    /// 置き方 plan を、帯 piece を写すダブにする（筆先と反転を帯のものにし、向きは線の向き）。
    pub(crate) fn ribbon_plan<'a>(
        &'a self,
        plan: &DabPlan<'a>,
        piece: &RibbonPiece,
        direction: f64,
    ) -> DabPlan<'a> {
        DabPlan {
            tip: self.ribbon_tip(piece.tip),
            flip_x: piece.flip_x,
            flip_y: piece.flip_y,
            angle: direction,
            ribbon: Some(piece.cut),
            ..*plan
        }
    }
}

impl TipRun {
    /// 描点 `stamp`（線の長さ `arc`）の、半径 `radius` のダブが写す帯（呼び手の単位。`gap` は描点の間隔の見込み）。同じ描点は前と同じ帯。
    /// リボンでないストロークは空。写しが描点に間に合わない筆先と間隔は、`StrokeBudgetExceeded`（ストロークを取り消す）。
    pub(crate) fn ribbon_pieces(
        &mut self,
        brush: &Brush,
        stamp: u64,
        arc: f64,
        radius: f64,
        gap: f64,
    ) -> Result<Vec<RibbonPiece>, CoreError> {
        let Some(mut run) = self.ribbon.take() else {
            return Ok(Vec::new());
        };
        let mut result = Ok(());
        if run.stamp != Some(stamp) {
            run.stamp = Some(stamp);
            run.pieces.clear();
            if radius > 0.0 && radius.is_finite() && !run.done {
                result = self.cut_ribbon(brush, &mut run, arc, radius, gap);
            }
        }
        let pieces = run.pieces.clone();
        self.ribbon = Some(run);
        result.map(|()| pieces)
    }

    fn cut_ribbon(
        &mut self,
        brush: &Brush,
        run: &mut RibbonRun,
        arc: f64,
        radius: f64,
        gap: f64,
    ) -> Result<(), CoreError> {
        // 最初の描点は線の向きがまだ決まらない（押した点）ので写さず、そこから写し始める
        let Some(frontier) = run.frontier else {
            run.frontier = Some(arc);
            return Ok(());
        };
        let end = arc + gap * 0.5;
        // 枚数の上限で写し残しているときは、遅れを詰めずに `frontier` から続ける
        let start = if run.behind {
            frontier
        } else {
            frontier.max(arc - gap)
        };
        if end.is_nan() || end <= start {
            return Ok(());
        }
        let list = brush.tip_list().unwrap_or(&[]);
        let mut pos = start;
        let mut capped = false;
        while pos < end {
            if run.pieces.len() >= MAX_RIBBON_PIECES {
                capped = true;
                break;
            }
            let (tip, flip_x, flip_y) = match run.current {
                Some(c) => c,
                None => {
                    let Some(c) = self.pick_index(
                        list.len(),
                        brush.tip.selection,
                        brush.tip.flip_rules(),
                        &mut run.random,
                    ) else {
                        run.done = true;
                        break;
                    };
                    run.current = Some(c);
                    run.phase = 0.0;
                    c
                }
            };
            let Some(image) = brush.ribbon_tip(tip) else {
                run.done = true;
                break;
            };
            let length = tip_length(image);
            let left = (length - run.phase) * radius;
            if left.is_nan() || left <= 0.0 {
                run.current = None;
                continue;
            }
            let reaches = pos + left <= end;
            let stop = if reaches { pos + left } else { end };
            let lo = (pos - arc) / radius;
            run.pieces.push(RibbonPiece {
                tip,
                flip_x,
                flip_y,
                cut: RibbonCut {
                    lo,
                    hi: (stop - arc) / radius,
                    center: lo - run.phase + length * 0.5,
                },
            });
            if reaches {
                run.current = None;
                run.phase = 0.0;
            } else {
                run.phase += (stop - pos) / radius;
            }
            pos = stop;
        }
        run.frontier = Some(pos);
        run.behind = capped;
        if capped && end - pos > MAX_RIBBON_BACKLOG_GAPS * gap {
            return Err(CoreError::StrokeBudgetExceeded);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tip(w: u32, h: u32) -> Arc<BrushTip> {
        Arc::new(BrushTip::new("t", w, h, vec![255; (w * h) as usize]).unwrap())
    }

    fn ribbon_brush(images: Vec<Arc<BrushTip>>, selection: TipSelection) -> Brush {
        let mut b = Brush::default();
        b.tip.images = images;
        b.tip.selection = selection;
        b.tip.ribbon = true;
        b
    }

    #[test]
    fn consecutive_stamps_cover_the_arc_without_gaps() {
        // 幅 4・高さ 1 の筆先: 1 枚の長さは半径の 8 倍。半径 2・間隔 3 の描点を並べると、帯は前の帯の終わりから続く
        let brush = ribbon_brush(vec![tip(4, 1)], TipSelection::Sequential);
        let mut run = TipRun::new(&brush);
        let mut covered = 0.0;
        for k in 0..20u64 {
            let arc = 3.0 * k as f64;
            for p in run.ribbon_pieces(&brush, k, arc, 2.0, 3.0).unwrap() {
                let (lo, hi) = (arc + p.cut.lo * 2.0, arc + p.cut.hi * 2.0);
                assert!((lo - covered).abs() < 1e-9, "{k}: {lo} {covered}");
                assert!(hi > lo);
                covered = hi;
            }
        }
        assert!((covered - (57.0 + 1.5)).abs() < 1e-9, "{covered}");
    }

    #[test]
    fn several_tips_follow_the_repeat_order_by_arc_length() {
        let brush = ribbon_brush(
            vec![tip(2, 1), tip(2, 1), tip(2, 1)],
            TipSelection::PingPong,
        );
        let mut run = TipRun::new(&brush);
        let mut order = Vec::new();
        for k in 0..40u64 {
            for p in run.ribbon_pieces(&brush, k, k as f64, 1.0, 1.0).unwrap() {
                if order.last() != Some(&p.tip) {
                    order.push(p.tip);
                }
            }
        }
        // 1 枚の長さは半径の 4 倍（線の長さ 4）なので、40 の長さで 10 枚: 0 1 2 1 0 1 2 1 0 1
        let want: Vec<Option<usize>> = [0, 1, 2, 1, 0, 1, 2, 1, 0, 1].map(Some).to_vec();
        assert_eq!(order, want);
    }

    #[test]
    fn the_same_stamp_reuses_its_cut_and_still_dabs_add_nothing() {
        let brush = ribbon_brush(vec![tip(4, 1)], TipSelection::Sequential);
        let mut run = TipRun::new(&brush);
        // 押した点の描点は写さない
        assert!(run
            .ribbon_pieces(&brush, 0, 0.0, 2.0, 3.0)
            .unwrap()
            .is_empty());
        let a = run.ribbon_pieces(&brush, 1, 3.0, 2.0, 3.0).unwrap();
        assert_eq!(run.ribbon_pieces(&brush, 1, 3.0, 2.0, 3.0).unwrap(), a);
        assert!(!a.is_empty());
        // 線の長さの進まない描点（時刻で置いたダブ）は帯が空
        assert!(run
            .ribbon_pieces(&brush, 2, 3.0, 2.0, 3.0)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn once_stops_after_the_last_tip() {
        let brush = ribbon_brush(vec![tip(1, 1), tip(1, 1)], TipSelection::Once);
        let mut run = TipRun::new(&brush);
        let mut total = 0.0;
        for k in 0..20u64 {
            for p in run.ribbon_pieces(&brush, k, k as f64, 1.0, 1.0).unwrap() {
                total += p.cut.hi - p.cut.lo;
            }
        }
        // 1 枚の長さ 2（半径 1）× 2 枚
        assert!((total - 4.0).abs() < 1e-9, "{total}");
    }
}
