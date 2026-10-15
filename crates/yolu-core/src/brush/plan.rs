//! ダブの置き方（C# の Stamp の、画素に触る前の部分）: 描点（間隔の 1 刻み）ごとのフェード・傾き・速さ・筆圧の係数と、その描点に置く
//! ダブ（数の分）ごとの大きさ・散布・角度・真円率・不透明度・流量のゆらぎと筆先の選び。2D のストローク（[`super::StrokeState`]）と
//! 3D の面のストローク（`geometry::SurfaceStroke`）が同じ関数を呼ぶ（同じ設定・同じ入力の点の列・同じ乱数の列なら、同じダブの列）。
//! 座標の枠は呼び手のもの（2D は文書の画素、3D は画面の点を y が上向きに直したもの）。角度は反時計回り。

use super::*;

/// 描点 1 つの、ダブの組（数）で共通の値。
#[derive(Clone, Copy, Debug)]
pub(crate) struct StampControls {
    /// フェードと、影響元の傾き・速度・ランダムの、大きさ・不透明度・流量への係数（使わなければ 1）。
    pub size: f64,
    pub opacity: f64,
    pub flow: f64,
    /// ペンの倒れた向き（角度に足す。使わなければ 0）。
    pub tilt_turn: f64,
    /// 筆圧の応えを通した大きさの係数（筆圧を使っていなければ 1）。
    pub size_pressure: f64,
    /// 筆圧の応えを通した不透明度・流量の係数と、絵の具の量・濃さの係数（筆圧でない入力も掛けたもの）。
    pub pressure: PressureScale,
    /// 硬さ（影響元を使うなら × 係数）。
    pub hardness: f64,
    /// 硬さの影響元の係数（3D の丸い筆先が自分の精度の硬さに掛ける。使わなければ None）。
    pub hardness_factor: Option<f64>,
    /// 真円率の影響元の係数（使わなければ None）。
    pub roundness: Option<f64>,
    /// 質感の濃さの影響元の係数（使わなければ 1）。
    pub texture_density: f64,
    /// 絵の具の量・濃さの、筆圧でない入力の係数（使わなければ 1。3D の面のダブが画素の筆圧の係数に掛ける）。
    pub mix_others: (f64, f64),
    /// 色の変化の 4 項目（[`ColorItem::ALL`] の並び）の、筆圧・傾き・速度の係数の積（ランダムを除く。使わなければ 1）。ランダムの分は
    /// ダブごとに色の変化の乱数の列から引く（[`StrokeState::next_dab_color`]）。
    pub color: [f64; 4],
    /// 散布の 2 項目（[`SprayItem::ALL`] の並び）の、筆圧・傾き・速度の係数の積（ランダムを除く。使わなければ 1）。ランダムの分は
    /// 散布の乱数の列から引く（[`SprayRun`]）。
    pub spray: [f64; 2],
}

/// ダブ 1 つの置き方。
#[derive(Clone, Copy, Debug)]
pub(crate) struct DabPlan<'a> {
    /// 中心（散布の後）。
    pub x: f64,
    pub y: f64,
    /// 呼び手の渡した大きさ × 係数 × ゆらぎ（0 より大きい）。
    pub radius: f64,
    /// 角度（ラジアン、反時計回り）。
    pub angle: f64,
    pub roundness: f64,
    pub opacity_scale: f64,
    pub flow_scale: f64,
    pub tip: Option<&'a BrushTip>,
    /// 筆先の画像の左右・上下の反転（[`FlipRule`] をこのダブで決めたもの）。
    pub flip_x: bool,
    pub flip_y: bool,
}

/// ランダムな反転の乱数の列と、並べ替えの乱数の列の種へ混ぜる値（ゆらぎ・色・影響元・デュアルの列と重ならない）。
const FLIP_STREAM: i32 = 0x27D4_EB2F;
const SHUFFLE_STREAM: i32 = 0x1656_67B1;

/// 筆先の選びと反転と散布の、ストロークごとの進み（2D のストロークと 3D の面のストロークが 1 つずつ持つ）。
/// 今までの乱数の列（ゆらぎ・色・影響元・デュアル・筆先の乱数選び）は引かない。ランダムな反転と並べ替えは、自分の列だけを引く。
#[derive(Clone, Debug)]
pub(crate) struct TipRun {
    /// 順に選ぶ方法で筆先を選んだダブの数（次のダブが並びの何番目か）。
    k: u64,
    seed: i32,
    /// ランダムな反転の列（左右か上下がランダムのブラシだけ）。
    flips: Option<NetRandom>,
    /// 並べ替えた筆先の並び（`Shuffle` で最初に要るとき作る）。
    order: Option<Vec<usize>>,
    /// 散布効果の粒子の進み（散布の乱数の列）。
    pub spray: SprayRun,
}

impl TipRun {
    pub(crate) fn new(brush: &Brush) -> TipRun {
        let random_flip =
            brush.tip.flip_x_rule == FlipRule::Random || brush.tip.flip_y_rule == FlipRule::Random;
        TipRun {
            k: 0,
            seed: brush.seed,
            flips: random_flip.then(|| NetRandom::new(brush.seed ^ FLIP_STREAM)),
            order: None,
            spray: SprayRun::new(brush),
        }
    }

    /// 並びの長さ n の筆先から、次のダブの番号と、折り返しで戻る向きか（`PingPong` だけ）。置く筆先が無ければ None
    /// （`Once`・`Shuffle` で並びを使い切った）。`Random` はここを通らない（乱数の列から引く）。
    fn next_index(&mut self, selection: TipSelection, n: usize) -> Option<(usize, bool)> {
        let k = self.k;
        let len = n as u64;
        let picked = match selection {
            TipSelection::Random | TipSelection::Sequential => (k % len) as usize,
            TipSelection::PingPong => {
                if n == 1 {
                    0
                } else {
                    let period = 2 * (len - 1);
                    let pos = k % period;
                    let index = if pos < len { pos } else { period - pos };
                    let reversed = k > 0 && (pos == 0 || pos >= len);
                    self.k = k.saturating_add(1);
                    return Some((index as usize, reversed));
                }
            }
            TipSelection::Hold => k.min(len - 1) as usize,
            TipSelection::Once => {
                if k >= len {
                    return None;
                }
                k as usize
            }
            TipSelection::Shuffle => {
                if k >= len {
                    return None;
                }
                let seed = self.seed;
                let order = self.order.get_or_insert_with(|| {
                    let mut random = NetRandom::new(seed ^ SHUFFLE_STREAM);
                    let mut order: Vec<usize> = (0..n).collect();
                    for i in (1..n).rev() {
                        let j = random.next_below(i as i32 + 1) as usize;
                        order.swap(i, j);
                    }
                    order
                });
                order[k as usize]
            }
        };
        self.k = k.saturating_add(1);
        Some((picked, false))
    }

    /// ダブ 1 つの反転（左右・上下）。`reversed` は折り返しで戻る向きか。ランダムは、軸ごとに列から 1 つ引く（左右、上下の順）。
    fn flips(&mut self, tip: &TipShape, reversed: bool) -> (bool, bool) {
        let mut one = |rule: FlipRule, fixed: bool| match rule {
            FlipRule::Fixed => fixed,
            FlipRule::OnFold => reversed,
            FlipRule::Random => self.flips.as_mut().map_or(fixed, |r| r.next_double() < 0.5),
        };
        let x = one(tip.flip_x_rule, tip.flip_x);
        let y = one(tip.flip_y_rule, tip.flip_y);
        (x, y)
    }
}

impl Brush {
    /// 描点 1 つ（ストロークの index 番目）の係数（C# の Stamp の前半）。フェード（何番目の描点か）と影響元（[`Influences`]）を
    /// 項目ごとに掛け合わせる。`taper` は入り抜きの係数（0〜1。[`StrokeAssist::taper`]。範囲の外は 1）で、大きさ以外の掛ける項目
    /// （[`StrokeAssist::taper_item`]）には、影響元の係数の後ろへ掛ける（大きさへは呼び手が半径に掛ける）。`random` は影響元の乱数の列（どの項目もランダムを使わなければ None。使う項目ごとに、項目の並びの順に 1 つ引く）。
    pub(crate) fn stamp_controls(
        &self,
        dab: &PendingDab,
        index: u64,
        taper: f64,
        random: Option<&mut NetRandom>,
    ) -> StampControls {
        let c = &self.controls;
        let inf = &self.influences;
        let tilt = if inf.uses(Input::Tilt) || c.tilt_angle {
            pen_tilt::amount(dab.tilt_x, dab.tilt_y)
        } else {
            0.0
        };
        // 速いほど小さく・薄く（速さの上限で 0）。使っていなければ求めない
        let slow = if inf.uses(Input::Speed) {
            1.0 - clamp01(dab.speed / inf.speed_max)
        } else {
            1.0
        };
        let x = InputValues {
            pressure: dab.pressure,
            upright: 1.0 - tilt,
            slow,
        };
        let tapered = |item| self.assist.taper_item(item, taper);
        let [r_size, r_opacity, r_flow, r_hardness, r_roundness, r_texture, r_paint, r_density] =
            inf.draw_random(random);
        // フェードの後に、傾き・速度・ランダムをこの順で掛ける（使っていない入力は掛けない）
        let size_control = inf
            .size
            .times_others(dynamics::fade(c.fade_size, index), &x, r_size);
        let opacity_control = taper_times(
            inf.opacity
                .times_others(dynamics::fade(c.fade_opacity, index), &x, r_opacity),
            tapered(Item::Opacity),
        );
        let flow_control = taper_times(
            inf.flow
                .times_others(dynamics::fade(c.fade_flow, index), &x, r_flow),
            tapered(Item::Flow),
        );
        let tilt_turn = if c.tilt_angle && tilt > 0.0 {
            pen_tilt::azimuth(dab.tilt_x, dab.tilt_y)
        } else {
            0.0
        };
        let s = &self.base;
        // 筆圧は項目ごとの応え（最小値と曲線）を通す。既定の応えは筆圧をそのまま返し、使っていない項目は掛けない
        let size_pressure = inf.size.pressure_factor(dab.pressure).unwrap_or(1.0);
        let hardness_factor =
            taper_factor(inf.hardness.factor(&x, r_hardness), tapered(Item::Hardness));
        let hardness = match hardness_factor {
            Some(f) => s.hardness * f,
            None => s.hardness,
        };
        let mut pressure = self.pressure_scale(dab.pressure);
        let mix_others = (
            taper_times(
                inf.mix_paint.times_others(1.0, &x, r_paint),
                tapered(Item::MixPaint),
            ),
            taper_times(
                inf.mix_density.times_others(1.0, &x, r_density),
                tapered(Item::MixDensity),
            ),
        );
        if mix_others.0 != 1.0 {
            pressure.mix_paint *= mix_others.0;
        }
        if mix_others.1 != 1.0 {
            pressure.mix_density *= mix_others.1;
        }
        StampControls {
            size: size_control,
            opacity: opacity_control,
            flow: flow_control,
            tilt_turn,
            size_pressure,
            pressure,
            hardness,
            hardness_factor,
            roundness: taper_factor(
                inf.roundness.factor(&x, r_roundness),
                tapered(Item::Roundness),
            ),
            texture_density: taper_times(
                inf.texture_density.factor(&x, r_texture).unwrap_or(1.0),
                tapered(Item::TextureDensity),
            ),
            mix_others,
            color: inf.color_others(&x),
            spray: inf.spray_others(&x),
        }
    }

    /// 描点の数の分の 1 つのダブ（C# の Stamp のループの中）。`radius` は呼び手の大きさ（2D は半径 × 筆圧 × 入り抜き）、`scatter_radius`
    /// は散布の届く長さの元の半径（公称の半径）。乱数はいつも同じ順に引く: 大きさ → 散布（x・y）→ 角度 → 真円率 → 不透明度 → 流量 →
    /// 筆先（乱数で選ぶとき）。大きさが 0 以下のダブは、大きさのゆらぎを引いた所で None（後の乱数は引かない）。
    /// `min_radius` は呼び手の単位での大きさの下限（最低 1 ピクセル。0 は下限なし）: フェード・傾き・速さの係数とゆらぎで細くなった後に
    /// 下から押さえるので、大きさ 0 のダブも残る（下限なしのときは今までと同じ）。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn next_dab<'a>(
        &'a self,
        c: &StampControls,
        dab: &PendingDab,
        radius: f64,
        min_radius: f64,
        scatter_radius: f64,
        random: &mut NetRandom,
        tips: &mut TipRun,
    ) -> Option<DabPlan<'a>> {
        let j = &self.jitter;
        let mut radius = radius;
        if c.size != 1.0 {
            radius *= c.size;
        }
        if j.size > 0.0 {
            radius *= 1.0 - j.size * random.next_double();
        }
        if min_radius > 0.0 {
            radius = f64_max(radius, min_radius);
        }
        if radius <= 0.0 {
            return None;
        }
        let (mut x, mut y) = (dab.x, dab.y);
        if j.scatter > 0.0 && self.spray.is_none() {
            let reach = scatter_radius * 2.0 * j.scatter;
            x += (random.next_double() * 2.0 - 1.0) * reach;
            y += (random.next_double() * 2.0 - 1.0) * reach;
        }
        let mut angle = self.tip.angle * std::f64::consts::PI / 180.0
            + (if self.tip.follow_direction {
                dab.direction
            } else {
                0.0
            });
        if c.tilt_turn != 0.0 {
            angle += c.tilt_turn;
        }
        if self.controls.rotation_angle && dab.rotation != 0.0 {
            angle += dab.rotation; // 拡張: ペンの軸の回転
        }
        if j.angle > 0.0 {
            angle += (random.next_double() * 2.0 - 1.0) * std::f64::consts::PI * j.angle;
        }
        // 真円率の影響元は、ゆらぎの前に掛ける（下限はゆらぎと同じ 0.01）
        let mut roundness = match c.roundness {
            Some(f) => f64_max(0.01, self.tip.roundness * f),
            None => self.tip.roundness,
        };
        if j.roundness > 0.0 {
            roundness = f64_max(0.01, roundness * (1.0 - j.roundness * random.next_double()));
        }
        let mut opacity_scale = if j.opacity > 0.0 {
            1.0 - j.opacity * random.next_double()
        } else {
            1.0
        };
        let mut flow_scale = if j.flow > 0.0 {
            1.0 - j.flow * random.next_double()
        } else {
            1.0
        };
        if c.opacity != 1.0 {
            opacity_scale *= c.opacity;
        }
        if c.flow != 1.0 {
            flow_scale *= c.flow;
        }
        let mut reversed = false;
        let tip: Option<&BrushTip> = match self.tip_list() {
            None => self.tip.image.as_deref(),
            Some(list) => {
                let i = match self.tip.selection {
                    TipSelection::Random => random.next_below(list.len() as i32) as usize,
                    selection => {
                        // 並びを使い切った（一巡のみ・ランダムに一巡）あとは、何も置かない
                        let (i, back) = tips.next_index(selection, list.len())?;
                        reversed = back;
                        i
                    }
                };
                Some(list[i].as_ref())
            }
        };
        let (flip_x, flip_y) = tips.flips(&self.tip, reversed);
        Some(DabPlan {
            x,
            y,
            radius,
            angle,
            roundness,
            opacity_scale,
            flow_scale,
            tip,
            flip_x,
            flip_y,
        })
    }

    /// 置き方からダブの形（筆先の反転・紙の質感・デュアルの合わせ方を付け、画像の縦横比を当てる）。
    pub(crate) fn dab_shape<'a>(&'a self, plan: &DabPlan<'a>, c: &StampControls) -> DabShape<'a> {
        let (cos, sin) = cos_sin(plan.angle);
        DabShape {
            x: plan.x,
            y: plan.y,
            radius: plan.radius,
            cos,
            sin,
            roundness: plan.roundness,
            aspect_x: 1.0,
            aspect_y: 1.0,
            hardness: c.hardness,
            pressure: c.pressure,
            opacity_scale: plan.opacity_scale,
            flow_scale: plan.flow_scale,
            tip: plan.tip,
            plain: plan.angle == 0.0 && plan.roundness == 1.0,
            flip_x: plan.flip_x,
            flip_y: plan.flip_y,
            texture: self.texture.as_ref().filter(|t| t.depth > 0.0),
            texture_density: c.texture_density,
            dual: self.dual.as_ref().map(|d| d.mode),
            edge: Edge::OFF,
        }
        .with_aspect()
    }
}

/// v に入り抜きの係数 f を掛ける（掛けない項目は v のまま）。
#[inline]
fn taper_times(v: f64, f: Option<f64>) -> f64 {
    match f {
        Some(f) => v * f,
        None => v,
    }
}

/// 影響元の係数（使わなければ None）に入り抜きの係数 f を掛ける（掛けない項目は変えない。影響元を使わない項目は f だけ）。
#[inline]
fn taper_factor(factor: Option<f64>, f: Option<f64>) -> Option<f64> {
    match (factor, f) {
        (Some(a), Some(b)) => Some(a * b),
        (None, Some(b)) => Some(b),
        (a, None) => a,
    }
}

impl StrokeAssist {
    /// 長さ end のストロークの、線の長さ arc の所のダブの大きさの係数: 入りで 0 から育ち、抜きで 0 へ細る（C# の Taper）。end が
    /// 有限でない（まだ終わりが分からない）間は抜きを掛けない。長さの単位は呼び手のもの（2D は文書の画素、3D は画面の点）。
    pub(crate) fn taper(&self, arc: f64, end: f64) -> f64 {
        let mut f = 1.0;
        if self.taper_in > 0.0 {
            f = f64_min(f, arc / self.taper_in);
        }
        if self.taper_out > 0.0 && end.is_finite() {
            f = f64_min(f, (end - arc) / self.taper_out);
        }
        f64_max(0.0, f64_min(1.0, f))
    }

    /// 項目 item に掛ける入り抜きの係数（[`StrokeAssist::taper`] の t から。`最小値 + (1 − 最小値) × t`）。掛けない項目と、入り抜きの
    /// 範囲の外（t が 1）は None。最小値 0 の項目は t そのもの（既定の大きさだけの入り抜きは、浮動小数の演算を足さない）。
    pub(crate) fn taper_item(&self, item: Item, t: f64) -> Option<f64> {
        let min = self.taper_targets.min(item)?;
        if t >= 1.0 {
            return None;
        }
        Some(if min == 0.0 { t } else { min + (1.0 - min) * t })
    }

    /// 手ぶれ補正の糸: 筆 pen が入力 to に引かれて動いた先（糸の長さより近ければ、たるんでいて動かないので None）。
    pub(crate) fn pull(&self, pen: (f64, f64), to: (f64, f64)) -> Option<(f64, f64)> {
        let (dx, dy) = (to.0 - pen.0, to.1 - pen.1);
        let d = (dx * dx + dy * dy).sqrt();
        if d <= self.stabilizer {
            return None; // 糸がたるんでいる間は筆は動かない
        }
        let k = (d - self.stabilizer) / d;
        Some((pen.0 + dx * k, pen.1 + dy * k))
    }
}

impl DualBrush {
    /// 2 つ目の筆先のダブの形（中心 x・y、半径 radius は呼び手の単位。2D の溜まりと同じく、丸も回転の式で測る）。
    pub(crate) fn dab_shape(&self, x: f64, y: f64, radius: f64) -> DabShape<'_> {
        rows::dual_cover_shape(&dual_shape(self, x, y, radius, AntiAlias::None))
    }
}

/// 2 つ目の筆先のダブの形（C# の DualDabAt の式）。level はアンチエイリアスの段（2D。ダブの座標は描く先の画素）。
pub(super) fn dual_shape(
    dual: &DualBrush,
    x: f64,
    y: f64,
    radius: f64,
    level: AntiAlias,
) -> DualShape<'_> {
    let angle = dual.angle * std::f64::consts::PI / 180.0;
    let (cos, sin) = cos_sin(angle);
    let (mut aspect_x, mut aspect_y) = (1.0, 1.0);
    if let Some(t) = &dual.tip {
        if t.width() >= t.height() {
            aspect_y = t.height() as f64 / t.width() as f64;
        } else {
            aspect_x = t.width() as f64 / t.height() as f64;
        }
    }
    let mut shape = DualShape {
        x,
        y,
        radius,
        cos,
        sin,
        roundness: dual.roundness,
        hardness: dual.hardness,
        aspect_x,
        aspect_y,
        tip: dual.tip.as_deref(),
        edge: Edge::OFF,
    };
    if level != AntiAlias::None {
        // 縁と小さなダブの広げは主の筆先と同じ式（画像の筆先は半径・真円率を広げる）
        let cover = rows::dual_cover_shape(&shape).with_edge(level);
        shape.radius = cover.radius;
        shape.roundness = cover.roundness;
        shape.edge = cover.edge;
    }
    shape
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_flow_multiplies_the_flow_control() {
        let mut brush = Brush::default();
        brush.influences.flow.speed.on = true;
        brush.influences.speed_max = 3000.0;
        let dab = PendingDab {
            x: 0.0,
            y: 0.0,
            pressure: 1.0,
            arc: 0.0,
            direction: 0.0,
            tilt_x: 0.0,
            tilt_y: 0.0,
            rotation: 0.0,
            speed: 1500.0,
        };
        let c = brush.stamp_controls(&dab, 0, 1.0, None);
        // 速さ 1500 / 上限 3000 → 1 − clamp01(0.5) = 0.5 を流量の係数に掛ける（足さない）
        assert!((c.flow - 0.5).abs() < 1e-12, "{}", c.flow);
    }

    fn still_dab() -> PendingDab {
        PendingDab {
            x: 0.0,
            y: 0.0,
            pressure: 1.0,
            arc: 0.0,
            direction: 0.0,
            tilt_x: 0.0,
            tilt_y: 0.0,
            rotation: 0.0,
            speed: 0.0,
        }
    }

    /// ブラシの最初の描点の計画（大きさ 4）。
    fn first_plan<'a>(brush: &'a Brush, dab: &PendingDab) -> (StampControls, DabPlan<'a>) {
        let c = brush.stamp_controls(dab, 0, 1.0, None);
        let mut random = NetRandom::new(0);
        let mut tips = TipRun::new(brush);
        let plan = brush
            .next_dab(&c, dab, 4.0, 0.0, 4.0, &mut random, &mut tips)
            .expect("大きさは正");
        (c, plan)
    }

    #[test]
    fn the_pen_rotation_turns_the_dab_the_same_way() {
        // ペンの軸の回転は筆先の角度に足す（回した向きに回る）
        let mut brush = Brush::default();
        brush.controls.rotation_angle = true;
        let dab = PendingDab {
            rotation: 0.25,
            ..still_dab()
        };
        let (_, plan) = first_plan(&brush, &dab);
        assert!((plan.angle - 0.25).abs() < 1e-15, "{}", plan.angle);
    }

    #[test]
    fn a_texture_with_no_depth_is_not_applied() {
        // 深さ 0 の質感は「無し」と同じで当てない。深さがあれば当てる
        let applied = |depth: f64| {
            let brush = Brush {
                texture: Some(PaperTexture::new(builtin_tip("grain").unwrap(), depth)),
                ..Brush::default()
            };
            let (c, plan) = first_plan(&brush, &still_dab());
            brush.dab_shape(&plan, &c).texture.is_some()
        };
        assert!(!applied(0.0));
        assert!(applied(0.5));
    }

    #[test]
    fn the_tilt_turns_the_dab_only_when_the_angle_follows_the_tilt() {
        // 傾きで大きさだけを変えるブラシは、ペンを倒した向きへ筆先を回さない
        let dab = PendingDab {
            tilt_x: 0.3,
            tilt_y: 0.4,
            ..still_dab()
        };
        let mut brush = Brush::default();
        brush.influences.size.tilt.on = true;
        assert_eq!(brush.stamp_controls(&dab, 0, 1.0, None).tilt_turn, 0.0);
        // 角度を傾きに沿わせると、倒した向きだけ回す
        brush.controls.tilt_angle = true;
        let turn = brush.stamp_controls(&dab, 0, 1.0, None).tilt_turn;
        assert!((turn - pen_tilt::azimuth(0.3, 0.4)).abs() < 1e-15, "{turn}");
    }
}
