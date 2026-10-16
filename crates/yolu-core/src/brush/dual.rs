//! デュアルブラシ（2 つ目の筆先）の、2 つ目のダブごとの見え方: 影響元（[`DualItem`]。サイズ・濃度・間隔・厚さ）と向きの足し
//! （ペンの向き・ペン軸の回転・線の方向・ランダム）。
//!
//! - 入力（筆圧・傾き・速度）は、その 2 つ目のダブを道の上に並べた所の値（区間の両端の間で補間したもの）。ランダムは自分の列
//!   （[`DUAL_INFLUENCE_STREAM`]。ゆらぎ・色・影響元・デュアルの散布・反転・並べ替え・散布効果・色の変化の列と重ならない）から、2 つ目の
//!   ダブを並べるごとに サイズ → 濃度 → 間隔 → 厚さ（ランダムを使う項目だけ）→ 向きのランダム（強さが 0 より大きいとき）の順に引く。
//! - 係数は主の項目と同じ式（使っている入力の応えの積。[`Influence::factor`]）。サイズは半径、濃度は [`DualBrush::flow`]、間隔は
//!   次の 2 つ目のダブまでの間隔（下限は設定の間隔の [`MIN_SPACING_FACTOR`] 倍）、厚さは真円率（下限 0.01）に掛ける。
//! - 向きは主の筆先と同じ規則で、設定の角度に 線の方向 → ペンが倒れた向き → ペン軸の回転 → ランダム（強さ × ±180°）を足す。
//!
//! 2D のストロークと 3D の面のストロークは、2 つ目のダブを同じ順に並べ、同じ [`DualRun::look`] を呼ぶので、同じ設定・同じ入力の点の列なら
//! 同じ見え方の列になる。影響元も向きの足しも使わない 2 つ目の筆先は、何も引かず、いつも [`DualLook::PLAIN`]（今までの画素のまま）。

use std::f64::consts::PI;

use super::*;

/// デュアルの影響元の乱数の列の種へ混ぜる値。
pub(crate) const DUAL_INFLUENCE_STREAM: i32 = 0x6A09_E667;

/// 2 つ目の筆先の選び・反転・散布の種へ混ぜる値（今のどの列とも重ならない。反転・並べ替え・散布の列は、この種に主と同じ値を混ぜる）。
pub(crate) const DUAL_TIP_STREAM: i32 = 0x5BE0_CD19;

/// 間隔の影響元の係数の下限（係数 0 で同じ所に置き続けないため）。
pub(crate) const MIN_SPACING_FACTOR: f64 = 0.01;

/// 2 つ目のダブ 1 つの見え方（係数は使わなければ 1、足す向きは 0）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct DualLook {
    pub size: f64,
    pub flow: f64,
    /// 次の 2 つ目のダブまでの間隔の係数。
    pub spacing: f64,
    /// 真円率の係数。
    pub roundness: f64,
    /// 設定の角度に足す向き（ラジアン、反時計回り）。
    pub turn: f64,
}

impl DualLook {
    /// 何も変えない見え方。
    pub const PLAIN: DualLook = DualLook {
        size: 1.0,
        flow: 1.0,
        spacing: 1.0,
        roundness: 1.0,
        turn: 0.0,
    };
}

impl DualBrush {
    /// 2 つ目のダブごとに見え方が変わるか（影響元か向きの足しを使う）。
    pub(crate) fn varies(&self, influences: &Influences) -> bool {
        influences.dual_is_on()
            || self.angle_tilt
            || self.angle_rotation
            || self.angle_line
            || self.angle_random > 0.0
    }

    /// 2 つ目のダブの半径: 設定の半径 × サイズの係数 × サイズの連動の比（連動を使わなければ掛けない）。
    pub(crate) fn dab_radius(&self, look: &DualLook, link: f64) -> f64 {
        let mut radius = self.radius;
        if look.size != 1.0 {
            radius *= look.size;
        }
        if self.link_size {
            radius *= link;
        }
        radius
    }
}

/// サイズの連動の比 ratio（主の半径 ÷ 設定の半径、最低 1 画素の前）を、主の描点が実際に置く半径に合わせる: 「最低 1 画素」が入っていれば、
/// 主の半径は設定の半径 radius（文書の画素）のうち 0.5 画素より小さくならないので、比も `0.5 / radius` を下回らない。
#[inline]
pub(crate) fn lifted_link(ratio: f64, min_one_pixel: bool, radius: f64) -> f64 {
    if min_one_pixel && radius > 0.0 {
        f64_max(ratio, MIN_ONE_PIXEL_RADIUS / radius)
    } else {
        ratio
    }
}

/// 間隔 base（呼び手の単位）に間隔の係数 factor を掛けた、次の 2 つ目のダブまでの間隔（係数 1 は base のまま）。
#[inline]
pub(crate) fn step(base: f64, factor: f64) -> f64 {
    if factor == 1.0 {
        base
    } else {
        base * f64_max(MIN_SPACING_FACTOR, factor)
    }
}

/// ストロークの 2 つ目のダブの見え方の進み（2D のストロークと 3D の面のストロークが 1 つずつ持つ）。
#[derive(Clone, Debug, Default)]
pub(crate) struct DualRun {
    /// 見え方が変わるか（変わらなければ何も計算しない）。
    varies: bool,
    /// デュアルの影響元の乱数の列（ランダムを使うときだけ）。
    random: Option<NetRandom>,
}

impl DualRun {
    pub(crate) fn new(brush: &Brush) -> DualRun {
        let Some(d) = &brush.dual else {
            return DualRun::default();
        };
        let inf = &brush.influences;
        let random =
            DualItem::ALL.into_iter().any(|i| inf.dual(i).random.on) || d.angle_random > 0.0;
        DualRun {
            varies: d.varies(inf),
            random: random.then(|| NetRandom::new(brush.seed ^ DUAL_INFLUENCE_STREAM)),
        }
    }

    /// 道の上の位置 dab（入力はその所の値）に並べる 2 つ目のダブの見え方。
    pub(crate) fn look(&mut self, brush: &Brush, dab: &PendingDab) -> DualLook {
        let (true, Some(d)) = (self.varies, &brush.dual) else {
            return DualLook::PLAIN;
        };
        let inf = &brush.influences;
        let tilt = if inf.uses(Input::Tilt) || d.angle_tilt {
            pen_tilt::amount(dab.tilt_x, dab.tilt_y)
        } else {
            0.0
        };
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
        let mut f = [1.0; 4];
        for (slot, item) in f.iter_mut().zip(DualItem::ALL) {
            let influence = inf.dual(item);
            let u = if influence.random.on {
                self.random.as_mut().map(|r| r.next_double())
            } else {
                None
            };
            if let Some(v) = influence.factor(&x, u) {
                *slot = v;
            }
        }
        let mut turn = 0.0;
        if d.angle_line {
            turn += dab.direction;
        }
        if d.angle_tilt && tilt > 0.0 {
            turn += pen_tilt::azimuth(dab.tilt_x, dab.tilt_y);
        }
        if d.angle_rotation && dab.rotation != 0.0 {
            turn += dab.rotation;
        }
        if d.angle_random > 0.0 {
            if let Some(r) = self.random.as_mut() {
                turn += (r.next_double() * 2.0 - 1.0) * PI * d.angle_random;
            }
        }
        DualLook {
            size: f[0],
            flow: f[1],
            spacing: f[2],
            roundness: f[3],
            turn,
        }
    }
}

/// 2 つ目のダブ 1 つの置き方（中心・半径は呼び手の単位）: 選んだ筆先と反転、散布の粒子の向き。
#[derive(Clone, Copy, Debug)]
pub(crate) struct DualPlace<'a> {
    pub x: f64,
    pub y: f64,
    pub radius: f64,
    /// 散布の粒子の向き（ラジアン、反時計回り）。None は設定の角度 + 見え方の向き（[`DualLook::turn`]）。
    pub angle: Option<f64>,
    pub tip: Option<&'a BrushTip>,
    pub flip_x: bool,
    pub flip_y: bool,
}

/// 2 つ目の筆先の選び・反転・散布の、ストロークごとの進み（2D のストロークと 3D の面のストロークが 1 つずつ持つ）。主の
/// [`TipRun`]・[`SprayRun`] を、種 `ブラシの種 ^` [`DUAL_TIP_STREAM`] で 1 つ持つ: 乱数で選ぶ筆先はこの種の列から、反転・並べ替え・散布は
/// この種に主と同じ値を混ぜた列から引く。2 つ目の筆先が 1 枚で、反転がランダムでなく、散布も無ければ、何も引かない。
#[derive(Clone, Debug)]
pub(crate) struct DualTips {
    run: TipRun,
    /// 乱数で選ぶ筆先の列（主の描点の乱数の列にあたる。2 つ目の筆先を乱数で選ぶときだけ引く）。
    random: NetRandom,
}

impl DualTips {
    pub(crate) fn new(brush: &Brush) -> DualTips {
        let seed = brush.seed ^ DUAL_TIP_STREAM;
        let (rules, spray) = match &brush.dual {
            Some(d) => (d.flip_rules(), d.spray.is_some()),
            None => (TipShape::default().flip_rules(), false),
        };
        DualTips {
            run: TipRun::seeded(seed, rules, SprayRun::seeded(spray, seed)),
            random: NetRandom::new(seed),
        }
    }

    /// 並べた 2 つ目のダブ 1 つに置く数（散布があれば粒子の数、無ければ [`DualBrush::count`]）。
    pub(crate) fn count(&mut self, d: &DualBrush) -> u32 {
        let on = d.spray.as_ref().map(|s| SprayUse::plain(s, d.angle));
        self.run.spray.particles(on).unwrap_or(d.count)
    }

    /// 2 つ目のダブ 1 つの置き方: `at` の所（散布の線の方向も `at` から）に半径 `radius` で置く 2 つ目のダブの筆先と反転（主の
    /// [`TipRun::pick`]）と、散布の粒子（主の [`SprayRun::place_with`]。`frame` は呼び手の枠）。並びを使い切った・大きさ 0 の粒子は None。
    pub(crate) fn place<'a>(
        &mut self,
        d: &'a DualBrush,
        at: &PendingDab,
        radius: f64,
        frame: SprayFrame,
    ) -> Option<DualPlace<'a>> {
        let (tip, flip_x, flip_y) = self.run.pick(
            d.tip.as_deref(),
            &d.tips,
            d.selection,
            d.flip_rules(),
            &mut self.random,
        )?;
        let place = DualPlace {
            x: at.x,
            y: at.y,
            radius,
            angle: None,
            tip,
            flip_x,
            flip_y,
        };
        let Some(s) = &d.spray else {
            return Some(place);
        };
        let plan = DabPlan {
            x: at.x,
            y: at.y,
            radius,
            angle: 0.0,
            roundness: 1.0,
            opacity_scale: 1.0,
            flow_scale: 1.0,
            tip,
            flip_x,
            flip_y,
        };
        let p = self
            .run
            .spray
            .place_with(Some(SprayUse::plain(s, d.angle)), at, plan, frame)?;
        Some(DualPlace {
            x: p.x,
            y: p.y,
            radius: p.radius,
            angle: Some(p.angle),
            ..place
        })
    }
}

/// 2 つ目の溜まりへ置く覆い c に、2 つ目の紙の質感の文書の画素 (x, y) の値を掛ける（[`rows::dual_textured`] の 1 画素。3D の面の溜まりが
/// テクセルごとに呼ぶ）。
pub(crate) fn textured_at(tex: &PaperTexture, c: f32, x: i64, y: i64) -> f32 {
    rows::dual_textured(tex, &rows::Grain::new(tex, y), tex.depth as f32, c, x)
}

/// 2 つ目のダブを並べた所（散布の粒子の線の方向は `direction`）。散布の置き方が読むのは位置と線の方向だけ。
pub(crate) fn spot(x: f64, y: f64, direction: f64) -> PendingDab {
    PendingDab {
        x,
        y,
        pressure: 1.0,
        arc: 0.0,
        direction,
        tilt_x: 0.0,
        tilt_y: 0.0,
        rotation: 0.0,
        speed: 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(pressure: f64) -> PendingDab {
        PendingDab {
            x: 0.0,
            y: 0.0,
            pressure,
            arc: 0.0,
            direction: 0.5,
            tilt_x: 0.0,
            tilt_y: 0.0,
            rotation: 0.25,
            speed: 0.0,
        }
    }

    fn dual_brush(dual: DualBrush) -> Brush {
        Brush {
            dual: Some(dual),
            seed: 5,
            ..Brush::default()
        }
    }

    #[test]
    fn a_plain_dual_brush_draws_nothing_and_looks_plain() {
        let b = dual_brush(DualBrush::default());
        let mut run = DualRun::new(&b);
        assert!(run.random.is_none());
        assert_eq!(run.look(&b, &at(0.3)), DualLook::PLAIN);
    }

    #[test]
    fn the_pressure_scales_the_items_it_is_turned_on_for() {
        let mut b = dual_brush(DualBrush::default());
        b.influences.dual_size.pressure.on = true;
        b.influences.dual_thickness.pressure.on = true;
        let mut run = DualRun::new(&b);
        let look = run.look(&b, &at(0.4));
        assert_eq!(
            (look.size, look.flow, look.spacing, look.roundness),
            (0.4, 1.0, 1.0, 0.4)
        );
    }

    #[test]
    fn the_angle_adds_the_line_the_rotation_and_the_random_turn() {
        let mut b = dual_brush(DualBrush {
            angle_line: true,
            angle_rotation: true,
            ..DualBrush::default()
        });
        let mut run = DualRun::new(&b);
        assert!((run.look(&b, &at(1.0)).turn - 0.75).abs() < 1e-15);
        b.dual.as_mut().unwrap().angle_random = 0.5;
        let mut run = DualRun::new(&b);
        let mut r = NetRandom::new(5 ^ DUAL_INFLUENCE_STREAM);
        let want = 0.75 + (r.next_double() * 2.0 - 1.0) * PI * 0.5;
        assert_eq!(run.look(&b, &at(1.0)).turn, want);
    }

    /// 選び・反転の比べに使う 4 枚の筆先（組み込みの別々の画像）。
    fn four_tips() -> Vec<Arc<BrushTip>> {
        ["grain", "dots", "rim", "charcoal"]
            .iter()
            .map(|id| builtin_tip(id).unwrap())
            .collect()
    }

    fn index_of(list: &[Arc<BrushTip>], tip: Option<&BrushTip>) -> Option<usize> {
        let tip = tip?;
        list.iter().position(|t| std::ptr::eq(t.as_ref(), tip))
    }

    /// 主のブラシ（種 seed ^ DUAL_TIP_STREAM、同じ筆先と規則）の描点を n 個置いたときの、筆先の番号と反転の並び（主の `next_dab`）。
    fn main_sequence(
        tips: &[Arc<BrushTip>],
        selection: TipSelection,
        rules: [(FlipRule, bool); 2],
        n: usize,
    ) -> Vec<Option<(Option<usize>, bool, bool)>> {
        let main = Brush {
            seed: 5 ^ DUAL_TIP_STREAM,
            tip: TipShape {
                images: tips.to_vec(),
                selection,
                flip_x_rule: rules[0].0,
                flip_x: rules[0].1,
                flip_y_rule: rules[1].0,
                flip_y: rules[1].1,
                ..TipShape::default()
            },
            ..Brush::default()
        };
        let mut random = NetRandom::new(main.seed);
        let mut run = TipRun::new(&main);
        let dab = at(1.0);
        (0..n)
            .map(|i| {
                let c = main.stamp_controls(&dab, i as u64, 1.0, None);
                main.next_dab(&c, &dab, 4.0, 0.0, 4.0, &mut random, &mut run)
                    .map(|p| (index_of(tips, p.tip), p.flip_x, p.flip_y))
            })
            .collect()
    }

    /// 2 つ目（種 5、同じ筆先と規則）の 2 つ目のダブを n 個置いたときの並び（[`DualTips::place`]）。
    fn dual_sequence(
        tips: &[Arc<BrushTip>],
        selection: TipSelection,
        rules: [(FlipRule, bool); 2],
        n: usize,
    ) -> Vec<Option<(Option<usize>, bool, bool)>> {
        let b = dual_brush(DualBrush {
            tips: tips.to_vec(),
            selection,
            flip_x_rule: rules[0].0,
            flip_x: rules[0].1,
            flip_y_rule: rules[1].0,
            flip_y: rules[1].1,
            ..DualBrush::default()
        });
        let d = b.dual.as_ref().unwrap();
        let mut run = DualTips::new(&b);
        let frame = SprayFrame::document(d.radius, 0.0);
        (0..n)
            .map(|_| {
                run.place(d, &spot(0.0, 0.0, 0.5), 4.0, frame)
                    .map(|p| (index_of(tips, p.tip), p.flip_x, p.flip_y))
            })
            .collect()
    }

    #[test]
    fn the_secondary_picks_and_flips_its_tips_through_the_main_brushs_functions() {
        // 主の種を 2 つ目の種（ブラシの種 ^ DUAL_TIP_STREAM）にすると、繰り返し方法 6 種・反転 4 種のどれでも、選ばれる筆先の番号と
        // 反転の並びが主と同じ（同じ関数・同じ乱数の列を通る）
        let tips = four_tips();
        let flips = [
            [(FlipRule::Fixed, false), (FlipRule::Fixed, false)],
            [(FlipRule::Fixed, true), (FlipRule::Fixed, false)],
            [(FlipRule::Random, false), (FlipRule::Random, false)],
            [(FlipRule::OnFold, false), (FlipRule::Fixed, true)],
        ];
        for selection in [
            TipSelection::Random,
            TipSelection::Sequential,
            TipSelection::PingPong,
            TipSelection::Hold,
            TipSelection::Once,
            TipSelection::Shuffle,
        ] {
            for rules in flips {
                let want = main_sequence(&tips, selection, rules, 24);
                let got = dual_sequence(&tips, selection, rules, 24);
                assert_eq!(got, want, "{selection:?} {rules:?}");
            }
        }
        // 例: 折り返しは 0 1 2 3 2 1 0 1 …、折り返し時に反転は戻る向きだけ
        let got = dual_sequence(
            &tips,
            TipSelection::PingPong,
            [(FlipRule::OnFold, false), (FlipRule::Fixed, false)],
            8,
        );
        let indices: Vec<usize> = got.iter().map(|g| g.unwrap().0.unwrap()).collect();
        assert_eq!(indices, [0, 1, 2, 3, 2, 1, 0, 1]);
        let back: Vec<bool> = got.iter().map(|g| g.unwrap().1).collect();
        assert_eq!(back, [false, false, false, false, true, true, true, false]);
        // 一巡のみは 4 つの後は置かない
        let once = dual_sequence(&tips, TipSelection::Once, flips[0], 6);
        assert!(once[..4].iter().all(Option::is_some) && once[4..].iter().all(Option::is_none));
    }

    #[test]
    fn the_secondary_spray_places_the_main_brushs_particles() {
        // 主に同じ散布・同じ角度・種 ^ DUAL_TIP_STREAM を入れると、粒子の数・位置・大きさ・向きが同じ（影響元は使わない）
        let spray = Spray {
            size: 3.0,
            link_size: true,
            density: 6.0,
            bias: 0.4,
            angle: 20.0,
            direction: SprayDirection::Whole,
            random_angle: Some(0.5),
        };
        let b = dual_brush(DualBrush {
            radius: 4.0,
            angle: 30.0,
            spray: Some(spray),
            ..DualBrush::default()
        });
        let d = b.dual.as_ref().unwrap();
        let mut main = Brush {
            seed: 5 ^ DUAL_TIP_STREAM,
            spray: Some(spray),
            influences: Influences::none(),
            ..Brush::default()
        };
        main.base.radius = 4.0;
        main.tip.angle = 30.0;
        let dab = PendingDab {
            x: 10.0,
            y: 20.0,
            direction: 0.5,
            ..at(1.0)
        };
        let frame = SprayFrame::document(4.0, 0.0);
        let mut want = Vec::new();
        let (mut random, mut run) = (NetRandom::new(main.seed), TipRun::new(&main));
        for i in 0..5 {
            let c = main.stamp_controls(&dab, i, 1.0, None);
            for _ in 0..run.spray.count(&main, &c) {
                let plan = main
                    .next_dab(&c, &dab, 4.0, 0.0, 4.0, &mut random, &mut run)
                    .and_then(|p| run.spray.place(&main, &c, &dab, p, frame));
                want.push(plan.map(|p| (p.x, p.y, p.radius, p.angle)));
            }
        }
        let mut got = Vec::new();
        let mut tips = DualTips::new(&b);
        for _ in 0..5 {
            for _ in 0..tips.count(d) {
                let p = tips.place(d, &spot(10.0, 20.0, 0.5), 4.0, frame);
                got.push(p.map(|p| (p.x, p.y, p.radius, p.angle.unwrap())));
            }
        }
        assert_eq!(got.len(), 30);
        assert_eq!(got, want);
        // 散布の間は旧形の数を使わない
        let counted = DualBrush {
            count: 4,
            ..d.clone()
        };
        assert_eq!(DualTips::new(&b).count(&counted), 6);
    }

    #[test]
    fn a_plain_secondary_draws_nothing_from_its_new_streams() {
        // 筆先 1 枚・反転なし・散布なしの 2 つ目は、筆先も反転もそのまま（今の 2 つ目のダブの画素のまま）
        let b = dual_brush(DualBrush {
            tip: builtin_tip("dots"),
            count: 3,
            ..DualBrush::default()
        });
        let d = b.dual.as_ref().unwrap();
        let mut tips = DualTips::new(&b);
        assert_eq!(tips.count(d), 3);
        let p = tips
            .place(d, &spot(1.0, 2.0, 0.0), 5.0, SprayFrame::document(8.0, 0.0))
            .unwrap();
        assert!(std::ptr::eq(p.tip.unwrap(), d.tip.as_deref().unwrap()));
        assert_eq!(
            (p.x, p.y, p.radius, p.angle, p.flip_x, p.flip_y),
            (1.0, 2.0, 5.0, None, false, false)
        );
        assert!(tips.run.spray.particles(None).is_none());
        let fresh = NetRandom::new(5 ^ DUAL_TIP_STREAM).next_double();
        assert_eq!(tips.random.next_double(), fresh, "引いていない");
    }

    #[test]
    fn the_secondary_texture_is_the_main_ceilings_formula_on_the_pool() {
        // 乗算: 溜まりへ置く覆い c に掛けた値は、主の天井（不透明度の係数 c）に質感を掛けた値と同じ。ほかの 7 つ: 主の天井と同じ式
        // c + (合わせた値 − c) × 深さ
        let image = builtin_tip("grain").unwrap();
        for mode in TextureMode::ALL {
            let tex = PaperTexture {
                mode,
                scale: 1.5,
                angle: if mode == TextureMode::Overlay {
                    30.0
                } else {
                    0.0
                },
                ..PaperTexture::new(image.clone(), 0.7)
            };
            let depth = 0.7f32;
            for y in 0..5i64 {
                let g = rows::Grain::new(&tex, y);
                for x in 0..9i64 {
                    for c in [0.0f32, 0.3, 0.75, 1.0] {
                        let got = rows::dual_textured(&tex, &g, depth, c, x);
                        let want = match mode.as_blend() {
                            None => rows::texture_scale_one(&g, depth, c, x),
                            Some(m) => {
                                let grain = rows::grain_one(&g, x);
                                c + (rows::combine32(m, c, grain) - c) * depth
                            }
                        };
                        assert_eq!(got, want, "{mode:?} ({x}, {y}) {c}");
                        assert_eq!(textured_at(&tex, c, x, y), got, "{mode:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn the_new_stream_differs_from_every_other_stream() {
        let streams = [
            0,
            DUAL_TIP_STREAM,
            DUAL_INFLUENCE_STREAM,
            DUAL_STREAM,
            INFLUENCE_STREAM,
            COLOR_STREAM,
        ];
        for (i, a) in streams.iter().enumerate() {
            for b in &streams[i + 1..] {
                assert_ne!(a, b);
            }
        }
    }

    #[test]
    fn the_spacing_factor_has_a_floor() {
        assert_eq!(step(4.0, 1.0), 4.0);
        assert_eq!(step(4.0, 0.5), 2.0);
        assert_eq!(step(4.0, 0.0), 4.0 * MIN_SPACING_FACTOR);
    }
}
