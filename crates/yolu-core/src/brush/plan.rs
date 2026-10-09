//! ダブの置き方（C# の Stamp の、画素に触る前の部分）: 描点（間隔の 1 刻み）ごとのフェード・傾き・速さ・筆圧の係数と、その描点に置く
//! ダブ（数の分）ごとの大きさ・散布・角度・真円率・不透明度・流量のゆらぎと筆先の選び。2D のストローク（[`super::StrokeState`]）と
//! 3D の面のストローク（`geometry::SurfaceStroke`）が同じ関数を呼ぶ（同じ設定・同じ入力の点の列・同じ乱数の列なら、同じダブの列）。
//! 座標の枠は呼び手のもの（2D は文書の画素、3D は画面の点を y が上向きに直したもの）。角度は反時計回り。

use super::*;

/// 描点 1 つの、ダブの組（数）で共通の値。
#[derive(Clone, Copy, Debug)]
pub(crate) struct StampControls {
    /// フェード・傾き・速さの、大きさ・不透明度・流量への係数（使わなければ 1）。
    pub size: f64,
    pub opacity: f64,
    pub flow: f64,
    /// ペンの倒れた向き（角度に足す。使わなければ 0）。
    pub tilt_turn: f64,
    /// 筆圧の応えを通した大きさの係数（切っていれば 1）。
    pub size_pressure: f64,
    /// 筆圧の応えを通した不透明度・流量などの係数。
    pub pressure: PressureScale,
    /// 硬さ（筆圧で変えるなら × 応え）。
    pub hardness: f64,
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
}

impl Brush {
    /// 描点 1 つ（ストロークの index 番目）の係数（C# の Stamp の前半）。フェード（何番目の描点か）と傾きは、筆圧と掛け合わせる。
    pub(crate) fn stamp_controls(&self, dab: &PendingDab, index: u64) -> StampControls {
        let c = &self.controls;
        let tilt = if c.tilt_size || c.tilt_opacity || c.tilt_flow || c.tilt_angle {
            pen_tilt::amount(dab.tilt_x, dab.tilt_y)
        } else {
            0.0
        };
        let mut size_control =
            dynamics::fade(c.fade_size, index) * (if c.tilt_size { 1.0 - tilt } else { 1.0 });
        let mut opacity_control =
            dynamics::fade(c.fade_opacity, index) * (if c.tilt_opacity { 1.0 - tilt } else { 1.0 });
        let mut flow_control =
            dynamics::fade(c.fade_flow, index) * (if c.tilt_flow { 1.0 - tilt } else { 1.0 });
        // 拡張: 速いほど小さく・薄く（速さの上限で 0）。切っていれば掛けない（C# と同じ値のまま）
        if c.speed_size || c.speed_opacity || c.speed_flow {
            let slow = 1.0 - clamp01(dab.speed / c.speed_max);
            if c.speed_size {
                size_control *= slow;
            }
            if c.speed_opacity {
                opacity_control *= slow;
            }
            if c.speed_flow {
                flow_control *= slow;
            }
        }
        let tilt_turn = if c.tilt_angle && tilt > 0.0 {
            pen_tilt::azimuth(dab.tilt_x, dab.tilt_y)
        } else {
            0.0
        };
        let s = &self.base;
        // 筆圧は項目ごとの応え（最小値と曲線）を通す。既定の応えは筆圧をそのまま返し、切っている項目は掛けない
        let size_pressure = if s.pressure_size {
            self.pressure.size.apply(dab.pressure)
        } else {
            1.0
        };
        let hardness = if c.pressure_hardness {
            s.hardness * self.pressure.hardness.apply(dab.pressure)
        } else {
            s.hardness
        };
        StampControls {
            size: size_control,
            opacity: opacity_control,
            flow: flow_control,
            tilt_turn,
            size_pressure,
            pressure: self.pressure_scale(dab.pressure),
            hardness,
        }
    }

    /// 描点の数の分の 1 つのダブ（C# の Stamp のループの中）。`radius` は呼び手の大きさ（2D は半径 × 筆圧 × 入り抜き）、`scatter_radius`
    /// は散布の届く長さの元の半径（公称の半径）。乱数はいつも同じ順に引く: 大きさ → 散布（x・y）→ 角度 → 真円率 → 不透明度 → 流量 →
    /// 筆先（乱数で選ぶとき）。大きさが 0 以下のダブは、大きさのゆらぎを引いた所で None（後の乱数は引かない）。
    pub(crate) fn next_dab<'a>(
        &'a self,
        c: &StampControls,
        dab: &PendingDab,
        radius: f64,
        scatter_radius: f64,
        random: &mut NetRandom,
        tip_index: &mut u64,
    ) -> Option<DabPlan<'a>> {
        let j = &self.jitter;
        let mut radius = radius;
        if c.size != 1.0 {
            radius *= c.size;
        }
        if j.size > 0.0 {
            radius *= 1.0 - j.size * random.next_double();
        }
        if radius <= 0.0 {
            return None;
        }
        let (mut x, mut y) = (dab.x, dab.y);
        if j.scatter > 0.0 {
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
        let mut roundness = self.tip.roundness;
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
        let tip: Option<&BrushTip> = match self.tip_list() {
            None => self.tip.image.as_deref(),
            Some(list) => {
                let i = match self.tip.selection {
                    TipSelection::Sequential => {
                        let i = (*tip_index % list.len() as u64) as usize;
                        *tip_index += 1;
                        i
                    }
                    TipSelection::Random => random.next_below(list.len() as i32) as usize,
                };
                Some(list[i].as_ref())
            }
        };
        Some(DabPlan {
            x,
            y,
            radius,
            angle,
            roundness,
            opacity_scale,
            flow_scale,
            tip,
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
            flip_x: self.tip.flip_x,
            flip_y: self.tip.flip_y,
            texture: self.texture.as_ref().filter(|t| t.depth > 0.0),
            dual: self.dual.as_ref().map(|d| d.mode),
            edge: Edge::OFF,
        }
        .with_aspect()
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
