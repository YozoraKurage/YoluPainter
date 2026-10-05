//! lilToon のインスペクターが、いくつかのプロパティを 1 つの項目にまとめたり、値を別の目盛りで見せたりする欄の換算（lilToon 2.3.4 の
//! `lilEditorGUI.UV4Decal`・`DrawSpecularMode`・ラメ・アルファマスク・`GetRemapMinValue` と同じ式）。欄（`panel`）はここで値を見せ、
//! 動かした値をここで保存する値と操作（`LookOp`）にする。

use yolu_core::look::LookValue;

use super::liltoon;
use super::LookOp;

/// lilToon の `RoundFloat1000000`（欄に見せる値を 10⁻⁶ に丸める）。
fn round6(v: f32) -> f32 {
    (v * 1_000_000.0 + 0.5).floor() * 0.000_001
}

/// 表の 'static の名前（メインカラー 2nd・3rd の接頭辞と、名前の後ろ）。
fn layer_prop(prefix: &str, rest: &str) -> Option<&'static str> {
    liltoon::prop(&format!("{prefix}Tex{rest}")).map(|p| p.name)
}

fn floats(values: impl IntoIterator<Item = (&'static str, f32)>) -> Vec<(&'static str, LookValue)> {
    values.into_iter().map(|(n, v)| (n, LookValue::Float(v))).collect()
}

// ───────── 光沢のタイプ ─────────

/// 光沢のタイプの番号（0 無効・1 リアル・2 トゥーン。lilToon の `DrawSpecularMode`）。
pub fn specular_mode(apply: bool, toon: bool) -> usize {
    match (apply, toon) {
        (false, _) => 0,
        (true, false) => 1,
        (true, true) => 2,
    }
}

/// 光沢のタイプを選ぶ操作（`_ApplySpecular`・`_SpecularToon` を 1 回の Undo で）。
pub fn specular_op(mode: usize) -> LookOp {
    let (apply, toon) = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0)][mode.min(2)];
    LookOp::Values {
        values: floats([("_ApplySpecular", apply), ("_SpecularToon", toon)]),
        drag: false,
    }
}

// ───────── デカール ─────────

/// ミラーモードの番号（0 通常・1 反転・2 左のみ・3 右のみ・4 右のみ・反転。lilToon の `UV4Decal` と同じ読み方: 左のみが勝つ）。
pub fn mirror_mode(left: bool, right: bool, flip: bool) -> usize {
    let mut m = 0;
    if right {
        m = 3;
    }
    if flip {
        m += 1;
    }
    if left {
        m = 2;
    }
    m
}

/// 複製モードの番号（0 通常・1 左右対称・2 反転）。
pub fn copy_mode(copy: bool, flip: bool) -> usize {
    match (copy, flip) {
        (_, true) => 2,
        (true, false) => 1,
        _ => 0,
    }
}

/// ミラーモードを選ぶ操作（`IsLeftOnly`・`IsRightOnly`・`ShouldFlipMirror` を 1 回の Undo で）。`prefix` は `_Main2nd`・`_Main3rd`。
pub fn mirror_op(prefix: &str, mode: usize) -> LookOp {
    let (l, r, f) = [(0.0, 0.0, 0.0), (0.0, 0.0, 1.0), (1.0, 0.0, 0.0), (0.0, 1.0, 0.0), (0.0, 1.0, 1.0)][mode.min(4)];
    let names = ["IsLeftOnly", "IsRightOnly", "ShouldFlipMirror"];
    LookOp::Values {
        values: floats(names.into_iter().zip([l, r, f]).filter_map(|(n, v)| Some((layer_prop(prefix, n)?, v)))),
        drag: false,
    }
}

/// 複製モードを選ぶ操作（`ShouldCopy`・`ShouldFlipCopy` を 1 回の Undo で）。
pub fn copy_op(prefix: &str, mode: usize) -> LookOp {
    let (c, f) = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0)][mode.min(2)];
    let names = ["ShouldCopy", "ShouldFlipCopy"];
    LookOp::Values {
        values: floats(names.into_iter().zip([c, f]).filter_map(|(n, v)| Some((layer_prop(prefix, n)?, v)))),
        drag: false,
    }
}

/// デカールの入切（lilToon の `UV4Decal` と同じく、切にするとミラーと複製のフラグも 0 に戻す。シェーダーはデカールの入切に関わらず
/// これらのフラグで隠し・裏返すので、残すと欄に出ないフラグで描き続ける。1 回の Undo）。
pub fn decal_op(prefix: &str, on: bool) -> LookOp {
    let mut names = vec!["IsDecal"];
    if !on {
        names.extend(["IsLeftOnly", "IsRightOnly", "ShouldFlipMirror", "ShouldCopy", "ShouldFlipCopy"]);
    }
    LookOp::Values {
        values: floats(names.into_iter().filter_map(|n| {
            let v = if n == "IsDecal" { f32::from(on) } else { 0.0 };
            Some((layer_prop(prefix, n)?, v))
        })),
        drag: false,
    }
}

/// デカールのタイリング・オフセット（`_ST`）を、欄の位置と大きさ（X 座標・Y 座標・X 軸サイズ・Y 軸サイズ）で。複製モードのときの
/// X 座標は右半分（0.5〜1）で見せる。
pub fn decal_shown(st: [f32; 4], copy: bool) -> [f32; 4] {
    let (sx, px) = if st[0] == 0.0 { (0.000_001, 0.5) } else { (1.0 / st[0], (0.5 - st[2]) / st[0]) };
    let (sy, py) = if st[1] == 0.0 { (0.000_001, 0.5) } else { (1.0 / st[1], (0.5 - st[3]) / st[1]) };
    let (sx, sy, px, py) = (round6(sx), round6(sy), round6(px), round6(py));
    let px = if copy && px < 0.5 { 1.0 - px } else { px };
    [px, py, sx, sy]
}

/// 欄の位置と大きさから、保存するタイリング・オフセット。
pub fn decal_st(shown: [f32; 4]) -> [f32; 4] {
    let [px, py, mut sx, mut sy] = shown;
    if sx == 0.0 {
        sx = 0.000_001;
    }
    if sy == 0.0 {
        sy = 0.000_001;
    }
    let (sx, sy) = (1.0 / sx, 1.0 / sy);
    [sx, sy, -px * sx + 0.5, -py * sy + 0.5]
}

/// デカールの位置と大きさを動かす操作（`st` はタイリング・オフセットのプロパティ）。
pub fn decal_st_op(st: &'static str, shown: [f32; 4], drag: bool) -> LookOp {
    LookOp::Value {
        name: st,
        value: LookValue::Vector(decal_st(shown)),
        drag,
    }
}

// ───────── ラメ ─────────

/// `_GlitterParams1`・`_GlitterSensitivity` を、欄の値（サイズ X・サイズ Y・パーティクルサイズ・密度・感度）で。
pub fn glitter_shown(params1: [f32; 4], sensitivity: f32) -> [f32; 5] {
    let size = if params1[2] == 0.0 { 0.0 } else { params1[2].max(0.0).sqrt() };
    let density = (1.0 / params1[3].max(1e-6)).sqrt() / 1.5;
    [
        256.0 / params1[0].max(1e-6),
        256.0 / params1[1].max(1e-6),
        size,
        round6(density),
        round6(sensitivity / density.max(1e-6)),
    ]
}

/// 欄の値から、保存する `_GlitterParams1`・`_GlitterSensitivity`（感度は 0.25 以上。lilToon と同じ）。
pub fn glitter_values(shown: [f32; 5]) -> ([f32; 4], f32) {
    let [sx, sy, size, density, sensitivity] = shown;
    let (sx, sy) = (sx.max(0.000_000_1), sy.max(0.000_000_1));
    let density = density.max(0.001);
    (
        [256.0 / sx, 256.0 / sy, size * size, 1.0 / (density * density * 1.5 * 1.5)],
        (sensitivity * density).max(0.25),
    )
}

/// ラメの大きさ・密度・感度を動かす操作（2 つのプロパティを 1 回の Undo で）。
pub fn glitter_op(shown: [f32; 5], drag: bool) -> LookOp {
    let (params1, sensitivity) = glitter_values(shown);
    LookOp::Values {
        values: vec![
            ("_GlitterParams1", LookValue::Vector(params1)),
            ("_GlitterSensitivity", LookValue::Float(sensitivity)),
        ],
        drag,
    }
}

// ───────── アルファマスク ─────────

/// `_AlphaMaskScale`・`_AlphaMaskValue` を、欄の値（反転・透明度）で。
pub fn alpha_mask_shown(scale: f32, value: f32) -> (bool, f32) {
    let invert = scale < 0.0;
    (invert, value - if invert { 1.0 } else { 0.0 })
}

/// 欄の反転・透明度を動かす操作（2 つのプロパティを 1 回の Undo で）。
pub fn alpha_mask_op(invert: bool, transparency: f32, drag: bool) -> LookOp {
    LookOp::Values {
        values: floats([
            ("_AlphaMaskScale", if invert { -1.0 } else { 1.0 }),
            ("_AlphaMaskValue", transparency + if invert { 1.0 } else { 0.0 }),
        ]),
        drag,
    }
}

// ───────── スケールとオフセットの最小・最大 ─────────

/// スケールとオフセット（value = saturate(x × scale + offset)）を、欄の最小（値 0 になる x）・最大（値 1 になる x）で（lilToon の
/// `GetRemapMinValue`・`GetRemapMaxValue`。−0.01〜1.01 に収める）。スケールがほぼ 0 なら 0・1。
pub fn remap_shown(scale: f32, offset: f32) -> (f32, f32) {
    if scale.abs() < 1e-6 {
        return (0.0, 1.0);
    }
    let clamp = |v: f32| round6(v.clamp(-0.01, 1.01));
    (clamp(-offset / scale), clamp((1.0 - offset) / scale))
}

/// 欄の最小・最大から、保存するスケールとオフセット（同じ値なら最大を 0.001 足す。lilToon と同じ）。
pub fn remap_values(min: f32, mut max: f32) -> (f32, f32) {
    if min == max {
        max += 0.001;
    }
    (1.0 / (max - min), min / (min - max))
}

/// 輪郭線のハイライトの Min・Max を動かす操作（`_OutlineLitScale`・`_OutlineLitOffset` を 1 回の Undo で）。
pub fn outline_lit_op(min: f32, max: f32, drag: bool) -> LookOp {
    let (scale, offset) = remap_values(min, max);
    LookOp::Values {
        values: floats([("_OutlineLitScale", scale), ("_OutlineLitOffset", offset)]),
        drag,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_vec(name: &str) -> [f32; 4] {
        liltoon::prop(name).unwrap().default
    }

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn the_modes_read_and_write_like_the_liltoon_inspector() {
        // ミラーモード: 左のみが勝つ、右のみと反転を合わせて 4。書いた値を読むと同じ番号
        assert_eq!(mirror_mode(false, false, false), 0);
        assert_eq!(mirror_mode(false, false, true), 1);
        assert_eq!(mirror_mode(true, false, false), 2);
        assert_eq!(mirror_mode(false, true, false), 3);
        assert_eq!(mirror_mode(false, true, true), 4);
        assert_eq!(mirror_mode(true, true, true), 2);
        for mode in 0..5 {
            let LookOp::Values { values, drag: false } = mirror_op("_Main2nd", mode) else {
                panic!("1 回の操作");
            };
            let on = |n: &str| values.iter().any(|(k, v)| *k == n && *v == LookValue::Float(1.0));
            assert_eq!(values.len(), 3);
            assert_eq!(mirror_mode(on("_Main2ndTexIsLeftOnly"), on("_Main2ndTexIsRightOnly"), on("_Main2ndTexShouldFlipMirror")), mode);
        }
        for mode in 0..3 {
            let LookOp::Values { values, .. } = copy_op("_Main3rd", mode) else {
                panic!("1 回の操作");
            };
            let on = |n: &str| values.iter().any(|(k, v)| *k == n && *v == LookValue::Float(1.0));
            assert_eq!(copy_mode(on("_Main3rdTexShouldCopy"), on("_Main3rdTexShouldFlipCopy")), mode);
        }
        for mode in 0..3 {
            let LookOp::Values { values, .. } = specular_op(mode) else {
                panic!("1 回の操作");
            };
            let on = |n: &str| values.iter().any(|(k, v)| *k == n && *v == LookValue::Float(1.0));
            assert_eq!(specular_mode(on("_ApplySpecular"), on("_SpecularToon")), mode);
        }
        // 既定（`_ApplySpecular` = 1・`_SpecularToon` = 1）は lilToon の欄と同じくトゥーン
        let d = |n: &str| liltoon::prop(n).unwrap().default[0] > 0.5;
        assert_eq!(specular_mode(d("_ApplySpecular"), d("_SpecularToon")), 2);
    }

    #[test]
    fn turning_the_decal_off_clears_the_mirror_and_copy_flags() {
        let LookOp::Values { values, drag: false } = decal_op("_Main2nd", false) else {
            panic!("1 回の操作");
        };
        let names: Vec<&str> = values.iter().map(|(n, _)| *n).collect();
        assert_eq!(
            names,
            [
                "_Main2ndTexIsDecal",
                "_Main2ndTexIsLeftOnly",
                "_Main2ndTexIsRightOnly",
                "_Main2ndTexShouldFlipMirror",
                "_Main2ndTexShouldCopy",
                "_Main2ndTexShouldFlipCopy"
            ]
        );
        assert!(values.iter().all(|(_, v)| *v == LookValue::Float(0.0)));
        // 入にするときはフラグを変えない
        let LookOp::Values { values, .. } = decal_op("_Main3rd", true) else {
            panic!("1 回の操作");
        };
        assert_eq!(values, vec![("_Main3rdTexIsDecal", LookValue::Float(1.0))]);
    }

    #[test]
    fn the_decal_position_and_scale_read_like_the_liltoon_inspector_and_round_trip() {
        // 既定の (1, 1, 0, 0) は lilToon の欄で X 座標 0.5・Y 座標 0.5・サイズ 1・1
        assert_eq!(decal_shown(default_vec("_Main2ndTex_ST"), false), [0.5, 0.5, 1.0, 1.0]);
        // 比べの場面の値（大きさ 0.2・中心 (1/6, 1/4)）
        let shown = decal_shown([5.0, 5.0, -1.0 / 3.0, -0.75], false);
        assert!(near(shown[0], 1.0 / 6.0) && near(shown[1], 0.25) && near(shown[2], 0.2) && near(shown[3], 0.2), "{shown:?}");
        // 欄の値 → 保存する値 → 欄の値
        for s in [[0.3, 0.7, 0.25, -0.4], [0.9, 0.1, 1.0, 0.05], [0.5, 0.5, -1.0, 1.0]] {
            let back = decal_shown(decal_st(s), false);
            for k in 0..4 {
                assert!(near(back[k], s[k]), "{s:?} → {back:?}");
            }
        }
        // 複製モードでは X 座標を右半分で見せる（左に置いたデカールも同じ所を指す）
        let left = decal_st([0.2, 0.5, 0.3, 0.3]);
        assert!(near(decal_shown(left, true)[0], 0.8));
        // 大きさ 0 は 0 で割らない
        let st = decal_st([0.5, 0.5, 0.0, 0.0]);
        assert!(st.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn the_glitter_fields_read_like_the_liltoon_inspector_and_round_trip() {
        // 既定の (256, 256, 0.16, 50)・感度 0.25 は、lilToon の欄でサイズ 1・1、パーティクルサイズ 0.4、密度 √(1/50)/1.5、感度 0.25 / 密度
        let shown = glitter_shown(default_vec("_GlitterParams1"), liltoon::prop("_GlitterSensitivity").unwrap().default[0]);
        let density = (1.0f32 / 50.0).sqrt() / 1.5;
        assert!(near(shown[0], 1.0) && near(shown[1], 1.0) && near(shown[2], 0.4), "{shown:?}");
        assert!(near(shown[3], density) && (shown[4] - 0.25 / density).abs() < 1e-4, "{shown:?}");
        for s in [[0.5, 2.0, 0.3, 0.2, 5.0], [4.0, 4.0, 1.5, 0.9, 1.0], [1.0, 1.0, 0.0, 0.05, 10.0]] {
            let (p, sens) = glitter_values(s);
            let back = glitter_shown(p, sens);
            for k in 0..5 {
                assert!((back[k] - s[k]).abs() < 1e-3 * s[k].abs().max(1.0), "{s:?} → {back:?}");
            }
        }
        // 感度は 0.25 より下げない（lilToon と同じ）
        assert_eq!(glitter_values([1.0, 1.0, 0.4, 0.1, 0.0]).1, 0.25);
    }

    #[test]
    fn the_alpha_mask_fields_read_like_the_liltoon_inspector_and_round_trip() {
        let d = |n: &str| liltoon::prop(n).unwrap().default[0];
        assert_eq!(alpha_mask_shown(d("_AlphaMaskScale"), d("_AlphaMaskValue")), (false, 0.0));
        for (invert, transparency) in [(false, 0.4), (true, -0.3), (true, 0.0), (false, -1.0)] {
            let LookOp::Values { values, .. } = alpha_mask_op(invert, transparency, false) else {
                panic!("1 回の操作");
            };
            let get = |n: &str| match values.iter().find(|(k, _)| *k == n).unwrap().1 {
                LookValue::Float(v) => v,
                _ => unreachable!(),
            };
            let back = alpha_mask_shown(get("_AlphaMaskScale"), get("_AlphaMaskValue"));
            assert_eq!(back.0, invert);
            assert!(near(back.1, transparency));
        }
    }

    #[test]
    fn the_remap_min_max_read_like_the_liltoon_inspector_and_round_trip() {
        // 輪郭線のハイライトの既定（スケール 10・オフセット −8）は lilToon の欄で Min 0.8・Max 0.9
        let d = |n: &str| liltoon::prop(n).unwrap().default[0];
        let (min, max) = remap_shown(d("_OutlineLitScale"), d("_OutlineLitOffset"));
        assert!(near(min, 0.8) && near(max, 0.9), "{min} {max}");
        for (lo, hi) in [(0.2, 0.6), (0.0, 1.0), (0.9, 0.1), (-0.01, 1.01)] {
            let (s, o) = remap_values(lo, hi);
            let (a, b) = remap_shown(s, o);
            assert!(near(a, lo) && near(b, hi), "{lo} {hi} → {a} {b}");
        }
        // 同じ値は最大を 0.001 足す
        let (s, o) = remap_values(0.5, 0.5);
        let (a, b) = remap_shown(s, o);
        assert!(near(a, 0.5) && near(b, 0.501));
    }
}
