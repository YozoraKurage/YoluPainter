//! 3D ビューの質感の式（CPU の参照）。Unity 版のプレビューのシェーダー（`PreviewSurfaceLit`・`PreviewSurface`・`PreviewToneMap`）と、その
//! もとの Unity の Standard の BRDF（UnityStandardBRDF.cginc の `BRDF1_Unity_PBS`、リニアのカラースペースの分岐）を、GPU のシェーダー
//! （`shaders/scene.wgsl`・`shaders/tonemap.wgsl`）と同じ式・同じ順で書いたもの。
//!
//! 試験が「既知の値」と「GPU の絵」を照らす基準になり（CPU で同じ式を書いて照らす）、環境の畳み込み（`environment`）の GGX も
//! ここの式を使う。式を変えるときは WGSL と一緒に変える（試験が両方を照らす）。
//!
//! 色の空間: 塗った Color・Emission は sRGB の値（ガンマ）で持ち、質感の式はリニアで解く。出力は「画面にそのまま出す値」（sRGB に
//! 直した値）で、トーンマッピングはそれをリニアに戻して当てる（Unity 版と同じ約束）。

use yolu_core::glam::{Vec2, Vec3};

/// 誘電体の F0（Unity の `unity_ColorSpaceDielectricSpec`、リニア）。
pub const DIELECTRIC_SPEC: f32 = 0.04;
/// Unity の映り込みが粗さで読む mip の段数（`UNITY_SPECCUBE_LOD_STEPS`）。
pub const REFLECTION_STEPS: u32 = 6;
const PI: f32 = std::f32::consts::PI;

// ───────── 色の空間 ─────────

/// sRGB（ガンマ）→ リニア（厳密な区分の式）。
pub fn srgb_to_linear(v: f32) -> f32 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

/// Unity の `Mathf.GammaToLinearSpace`: リニアの色空間の Unity が、色のプロパティ（`[HDR]` でない色）・`[Gamma]` の数・光の色
/// （`_LightColor0`。光の強さをリニアにしない既定）に当てる変換。1 未満は sRGB の式で、1 以上は `pow(v, 2.2)`（Unity 2022.3 で
/// 測った値と同じ: 1.5 → 2.440、2.119 → 5.218、16.948 → 505.9）。sRGB の式を 1 の先へ延ばす [`srgb_to_linear`] は 1 を超える所で
/// 明るすぎる（16.948 → 約 790）。
pub fn unity_gamma_to_linear(v: f32) -> f32 {
    if v < 1.0 {
        srgb_to_linear(v)
    } else {
        v.powf(2.2)
    }
}

/// リニア → sRGB（ガンマ。負は 0 として扱う。1 を超える値はそのまま超える）。
pub fn linear_to_srgb(v: f32) -> f32 {
    let v = v.max(0.0);
    if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

pub fn srgb_to_linear3(c: Vec3) -> Vec3 {
    Vec3::new(
        srgb_to_linear(c.x),
        srgb_to_linear(c.y),
        srgb_to_linear(c.z),
    )
}

pub fn linear_to_srgb3(c: Vec3) -> Vec3 {
    Vec3::new(
        linear_to_srgb(c.x),
        linear_to_srgb(c.y),
        linear_to_srgb(c.z),
    )
}

fn saturate(v: f32) -> f32 {
    v.clamp(0.0, 1.0)
}

fn pow5(v: f32) -> f32 {
    let v2 = v * v;
    v2 * v2 * v
}

// ───────── Unity の Standard の BRDF（BRDF1_Unity_PBS） ─────────

/// 金属の流儀: 反射しない割合 `1 − 反射率`（誘電体 0.04 から金属 1 へ）。
pub fn one_minus_reflectivity(metallic: f32) -> f32 {
    let one_minus_dielectric = 1.0 - DIELECTRIC_SPEC;
    one_minus_dielectric - metallic * one_minus_dielectric
}

/// 拡散の色・鏡面の色・`1 − 反射率`（`DiffuseAndSpecularFromMetallic`）。
pub fn diffuse_and_specular_from_metallic(albedo: Vec3, metallic: f32) -> (Vec3, Vec3, f32) {
    let spec = Vec3::splat(DIELECTRIC_SPEC).lerp(albedo, metallic);
    let omr = one_minus_reflectivity(metallic);
    (albedo * omr, spec, omr)
}

/// `FresnelTerm`（Schlick）。
pub fn fresnel_term(f0: Vec3, cos_a: f32) -> Vec3 {
    f0 + (Vec3::ONE - f0) * pow5(1.0 - cos_a)
}

/// `FresnelLerp`（F0 から F90 へ）。
pub fn fresnel_lerp(f0: Vec3, f90: f32, cos_a: f32) -> Vec3 {
    f0.lerp(Vec3::splat(f90), pow5(1.0 - cos_a))
}

/// `DisneyDiffuse`。
pub fn disney_diffuse(nv: f32, nl: f32, lh: f32, perceptual_roughness: f32) -> f32 {
    let fd90 = 0.5 + 2.0 * lh * lh * perceptual_roughness;
    let light_scatter = 1.0 + (fd90 - 1.0) * pow5(1.0 - nl);
    let view_scatter = 1.0 + (fd90 - 1.0) * pow5(1.0 - nv);
    light_scatter * view_scatter
}

/// `GGXTerm`（法線分布 D。`roughness` は知覚的な粗さの 2 乗）。
pub fn ggx_term(nh: f32, roughness: f32) -> f32 {
    let a2 = roughness * roughness;
    let d = (nh * a2 - nh) * nh + 1.0;
    (1.0 / PI) * a2 / (d * d + 1e-7)
}

/// `SmithJointGGXVisibilityTerm`（Unity の近似の形）。
pub fn smith_joint_ggx_visibility(nl: f32, nv: f32, roughness: f32) -> f32 {
    let a = roughness;
    let lambda_v = nl * (nv * (1.0 - a) + a);
    let lambda_l = nv * (nl * (1.0 - a) + a);
    0.5 / (lambda_v + lambda_l + 1e-5)
}

/// `Unity_SafeNormalize`。
fn safe_normalize(v: Vec3) -> Vec3 {
    v / v.dot(v).max(0.001).sqrt()
}

/// 1 点の面（リニア）。
#[derive(Clone, Copy, Debug)]
pub struct Surface {
    pub albedo: Vec3,
    pub metallic: f32,
    /// 滑らかさ（1 − Roughness）。
    pub smoothness: f32,
    /// 法線マップを当てた後の単位ベクトル（世界）。
    pub normal: Vec3,
    pub emission: Vec3,
}

/// 光（リニア）。`to_light` は光へ向かう単位ベクトル、`radiance` は Unity の `_LightColor0`（π を含んだ値: 面の真正面で拡散 = albedo × radiance）。
#[derive(Clone, Copy, Debug)]
pub struct Light {
    pub to_light: Vec3,
    pub radiance: Vec3,
}

/// 環境光（リニア）: 拡散（SH の値 = 照度 / π）と鏡面（粗さで読んだ映り込み）。
#[derive(Clone, Copy, Debug, Default)]
pub struct Indirect {
    pub diffuse: Vec3,
    pub specular: Vec3,
}

/// `BRDF1_Unity_PBS`（リニア）。発光は足さない（呼ぶ側で足す）。
pub fn standard_brdf(surface: &Surface, view: Vec3, light: &Light, indirect: &Indirect) -> Vec3 {
    let (diff_color, spec_color, omr) =
        diffuse_and_specular_from_metallic(surface.albedo, surface.metallic);
    let smoothness = surface.smoothness.clamp(0.0, 1.0);
    let perceptual_roughness = 1.0 - smoothness;
    let half_dir = safe_normalize(light.to_light + view);
    let mut normal = surface.normal;
    let shift = normal.dot(view);
    if shift < 0.0 {
        normal += view * (-shift + 1e-5);
    }
    let nv = saturate(normal.dot(view));
    let nl = saturate(normal.dot(light.to_light));
    let nh = saturate(normal.dot(half_dir));
    let lh = saturate(light.to_light.dot(half_dir));

    let diffuse_term = disney_diffuse(nv, nl, lh, perceptual_roughness) * nl;

    let roughness = (perceptual_roughness * perceptual_roughness).max(0.002);
    let v = smith_joint_ggx_visibility(nl, nv, roughness);
    let d = ggx_term(nh, roughness);
    let mut specular_term = v * d * PI;
    specular_term = (specular_term * nl).max(0.0);
    let surface_reduction = 1.0 / (roughness * roughness + 1.0);
    if spec_color == Vec3::ZERO {
        specular_term = 0.0;
    }
    let grazing = saturate(smoothness + (1.0 - omr));
    diff_color * (indirect.diffuse + light.radiance * diffuse_term)
        + light.radiance * fresnel_term(spec_color, lh) * specular_term
        + indirect.specular * fresnel_lerp(spec_color, grazing, nv) * surface_reduction
}

/// Unity の映り込みが粗さ（知覚的）で読む mip（`perceptualRoughness × (1.7 − 0.7 × perceptualRoughness) × 6`）。
pub fn mip_of_roughness(perceptual_roughness: f32) -> f32 {
    perceptual_roughness * (1.7 - 0.7 * perceptual_roughness) * REFLECTION_STEPS as f32
}

/// `mip_of_roughness` の逆（mip 0〜6 → 知覚的な粗さ 0〜1）。
pub fn roughness_of_mip(mip: u32) -> f32 {
    let m = mip.min(REFLECTION_STEPS) as f32 / REFLECTION_STEPS as f32;
    ((1.7 - (2.89 - 2.8 * m).max(0.0).sqrt()) / 1.4).clamp(0.0, 1.0)
}

// ───────── 中立の表示（PreviewSurface / PreviewSurfaceLit） ─────────

/// 中立の表示の環境の映り込みの粗さ（誘電体。塗った値は変えず、形を読む手がかりとして足す。Unity 版の `NeutralReflectionRoughness`）。
pub const NEUTRAL_REFLECTION_ROUGHNESS: f32 = 0.5;

// ───────── トーンマッピング（PreviewToneMap） ─────────

/// 曲線。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Curve {
    /// 露出を掛けて 0〜1 で切るだけ。
    #[default]
    None,
    /// Unity の Post Processing の NeutralTonemap。
    Neutral,
    /// Stephen Hill の ACES の近似（sRGB の原色）。
    Aces,
}

fn neutral_curve(x: Vec3, a: f32, b: f32, c: f32, d: f32, e: f32, f: f32) -> Vec3 {
    ((x * (x * a + Vec3::splat(c * b)) + Vec3::splat(d * e))
        / (x * (x * a + Vec3::splat(b)) + Vec3::splat(d * f)))
        - Vec3::splat(e / f)
}

pub fn neutral_tonemap(x: Vec3) -> Vec3 {
    let (a, b, c, d, e, f, white) = (0.2, 0.29, 0.24, 0.272, 0.02, 0.3, 5.3);
    let white_scale = 1.0 / neutral_curve(Vec3::splat(white), a, b, c, d, e, f).x;
    neutral_curve(x * white_scale, a, b, c, d, e, f) * white_scale
}

pub fn aces_tonemap(color: Vec3) -> Vec3 {
    let v = Vec3::new(
        0.59719 * color.x + 0.35458 * color.y + 0.04823 * color.z,
        0.07600 * color.x + 0.90834 * color.y + 0.01566 * color.z,
        0.02840 * color.x + 0.13383 * color.y + 0.83777 * color.z,
    );
    let a = v * (v + Vec3::splat(0.0245786)) - Vec3::splat(0.000090537);
    let b = v * (v * 0.983729 + Vec3::splat(0.432_951)) + Vec3::splat(0.238081);
    let c = a / b;
    Vec3::new(
        1.60475 * c.x - 0.53108 * c.y - 0.07367 * c.z,
        -0.10208 * c.x + 1.10813 * c.y - 0.00605 * c.z,
        -0.00327 * c.x - 0.07276 * c.y + 1.07602 * c.z,
    )
}

/// 画面にそのまま出す値（ガンマ）に、露出（EV）と曲線を当てた値（ガンマ、0〜1）。`PreviewToneMap.shader` と同じ順:
/// リニアに直し → 露出の倍率 → 曲線 → 0〜1 で切る → ガンマに戻す。
pub fn tone_map(display: Vec3, curve: Curve, exposure_ev: f32) -> Vec3 {
    let mut x = srgb_to_linear3(display.max(Vec3::ZERO));
    x *= 2f32.powf(exposure_ev);
    x = match curve {
        Curve::None => x,
        Curve::Neutral => neutral_tonemap(x),
        Curve::Aces => aces_tonemap(x),
    };
    linear_to_srgb3(x.clamp(Vec3::ZERO, Vec3::ONE))
}

// ───────── 法線マップ・接線 ─────────

/// 接空間の法線（OpenGL の Y+、リニアの RGB に詰めたもの `c × 2 − 1`）を、接線・従接線・法線（世界。補間したまま）で世界へ。
/// Unity の `normalize(tangent × t.x + bitangent × t.y + normal × t.z)`。
pub fn perturb_normal(t: Vec3, tangent: Vec3, bitangent: Vec3, normal: Vec3) -> Vec3 {
    (tangent * t.x + bitangent * t.y + normal * t.z).normalize()
}

/// 従接線: `cross(normal, tangent) × w`（Unity の頂点シェーダーと同じ。w は接線の向きの符号）。
pub fn bitangent(normal: Vec3, tangent: Vec3, w: f32) -> Vec3 {
    normal.cross(tangent) * w
}

// ───────── 環境（SH） ─────────

/// Unity の SphericalHarmonicsL2 の並びと式（c0 + c1 y + c2 z + c3 x + c4 xy + c5 yz + c6 (3z²−1) + c7 xz + c8 (x²−y²)）で、
/// 向き `n` の拡散の色（照度 / π）。負は 0。
pub fn evaluate_sh(sh: &[Vec3; 9], n: Vec3) -> Vec3 {
    let mut r = sh[0] + sh[1] * n.y + sh[2] * n.z + sh[3] * n.x;
    r += sh[4] * (n.x * n.y) + sh[5] * (n.y * n.z) + sh[6] * (3.0 * n.z * n.z - 1.0);
    r += sh[7] * (n.x * n.z) + sh[8] * (n.x * n.x - n.y * n.y);
    r.max(Vec3::ZERO)
}

/// 環境を上の軸のまわりに θ 回したとき、世界の向き d に見える元の環境の向き（R(−θ) d）。`rotation` は (cos θ, sin θ)。
pub fn to_source(d: Vec3, rotation: Vec2) -> Vec3 {
    let (c, s) = (rotation.x, rotation.y);
    Vec3::new(d.x * c - d.z * s, d.y, d.x * s + d.z * c)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32, tol: f32) -> bool {
        (a - b).abs() <= tol
    }

    fn close3(a: Vec3, b: Vec3, tol: f32) -> bool {
        (a - b).abs().max_element() <= tol
    }

    #[test]
    fn srgb_round_trip_and_known_points() {
        assert!(close(srgb_to_linear(0.0), 0.0, 1e-7));
        assert!(close(srgb_to_linear(1.0), 1.0, 1e-6));
        // sRGB 0.5 → リニア 0.21404
        assert!(close(srgb_to_linear(0.5), 0.214_041, 1e-5));
        // Unity の GammaToLinearSpace（Unity 2022.3 のリニアの色空間で、色のプロパティ・光の色を float の描き先へ書いて測った値）
        for (v, unity) in [
            (0.02, 0.001_547_987_6),
            (0.5, 0.214_041_14),
            (1.0, 1.0),
            (1.5, 2.440_061_6),
            (2.119, 5.217_808),
            (16.948, 505.895_33),
        ] {
            assert!(close(unity_gamma_to_linear(v), unity, unity * 1e-5), "{v}");
        }
        // 1 未満は sRGB の式と同じ、1 を超えると sRGB の式を延ばしたものより暗い
        assert_eq!(unity_gamma_to_linear(0.7), srgb_to_linear(0.7));
        assert!(srgb_to_linear(16.948) > 780.0);
        for i in 0..=255 {
            let v = i as f32 / 255.0;
            assert!(close(linear_to_srgb(srgb_to_linear(v)), v, 1e-5), "{i}");
        }
        assert_eq!(linear_to_srgb(-1.0), 0.0);
        assert!(
            linear_to_srgb(4.0) > 1.0,
            "1 を超える値は切らない（トーンマッピングが後で当てる）"
        );
    }

    #[test]
    fn metallic_flow_matches_unity_dielectric_constants() {
        let (diff, spec, omr) = diffuse_and_specular_from_metallic(Vec3::new(0.8, 0.4, 0.2), 0.0);
        assert!(close(omr, 0.96, 1e-6));
        assert!(close3(spec, Vec3::splat(0.04), 1e-7));
        assert!(close3(diff, Vec3::new(0.8, 0.4, 0.2) * 0.96, 1e-6));
        let (diff, spec, omr) = diffuse_and_specular_from_metallic(Vec3::new(0.8, 0.4, 0.2), 1.0);
        assert!(close(omr, 0.0, 1e-6));
        assert!(close3(spec, Vec3::new(0.8, 0.4, 0.2), 1e-6));
        assert!(close3(diff, Vec3::ZERO, 1e-6), "金属は拡散しない");
        let (_, spec, omr) = diffuse_and_specular_from_metallic(Vec3::ONE, 0.5);
        assert!(close(omr, 0.48, 1e-6));
        assert!(close3(spec, Vec3::splat(0.52), 1e-6));
    }

    #[test]
    fn ggx_and_visibility_known_values() {
        // D(nh = 1) = 1 / (π a²)（a = roughness）
        let a = 0.5;
        assert!(close(ggx_term(1.0, a), 1.0 / (PI * a * a), 1e-3));
        // 粗さ 1 の GGX は半球で一様に近い: D(nh) = 1 / π
        assert!(close(ggx_term(0.3, 1.0), 1.0 / PI, 1e-3));
        // 可視性: 垂直（nl = nv = 1）で 0.5 / (2 + 1e-5)
        assert!(close(smith_joint_ggx_visibility(1.0, 1.0, 0.3), 0.25, 1e-4));
        // 法線分布の正規化: ∫ D(nh) nh dω = 1（半球。数値積分）
        for roughness in [0.2f32, 0.5, 0.9] {
            let n = 4000;
            let mut sum = 0.0f64;
            for i in 0..n {
                let theta = (i as f64 + 0.5) / n as f64 * std::f64::consts::FRAC_PI_2;
                let nh = theta.cos() as f32;
                sum += ggx_term(nh, roughness) as f64
                    * nh as f64
                    * 2.0
                    * std::f64::consts::PI
                    * theta.sin()
                    * (std::f64::consts::FRAC_PI_2 / n as f64);
            }
            assert!(close(sum as f32, 1.0, 0.02), "粗さ {roughness}: {sum}");
        }
    }

    fn head_on(albedo: Vec3, metallic: f32, smoothness: f32) -> Vec3 {
        // 光も目も法線の向き（真正面）。環境光なし
        let surface = Surface {
            albedo,
            metallic,
            smoothness,
            normal: Vec3::Z,
            emission: Vec3::ZERO,
        };
        let light = Light {
            to_light: Vec3::Z,
            radiance: Vec3::ONE,
        };
        standard_brdf(&surface, Vec3::Z, &light, &Indirect::default())
    }

    #[test]
    fn head_on_lambert_and_specular_peak() {
        // 粗さ 1（滑らかさ 0）の誘電体: 拡散は Disney の項（真正面は fd90 = 0.5 + 2 × 1 = 2.5 の 1 × 1 = 1）で albedo × 0.96、
        // 鏡面は 0.04 を粗い分布で薄く
        let rough = head_on(Vec3::splat(0.5), 0.0, 0.0);
        assert!(rough.x > 0.48 && rough.x < 0.52, "{rough:?}");
        // つるつる（滑らかさ 1）の金属は、真正面で鏡面のピークが立つ（D が大きい）。粗い金属の 10 倍以上
        let glossy = head_on(Vec3::ONE, 1.0, 1.0);
        let dull = head_on(Vec3::ONE, 1.0, 0.0);
        assert!(glossy.x > dull.x * 10.0, "{glossy:?} {dull:?}");
        // 金属は拡散が無く、鏡面の色が albedo の色
        let metal = head_on(Vec3::new(1.0, 0.5, 0.0), 1.0, 0.5);
        assert!(metal.x > metal.y && metal.y > metal.z * 1.5, "{metal:?}");
        assert!(metal.z < 1e-3, "青は反射しない金属: {metal:?}");
    }

    #[test]
    fn energy_is_bounded_for_a_white_dielectric() {
        // 真正面の白い誘電体（粗さ 0.5）を真上の光で照らすと、拡散 0.96 に鏡面が少し乗る程度で、2 を超えない
        let c = head_on(Vec3::ONE, 0.0, 0.5);
        assert!(c.x > 0.9 && c.x < 2.0, "{c:?}");
    }

    #[test]
    fn indirect_terms_follow_unity() {
        // 光なしで環境だけ: 白い誘電体の拡散は 0.96 × diffuse、鏡面は F(0.04 → grazing) × reduction × specular
        let surface = Surface {
            albedo: Vec3::ONE,
            metallic: 0.0,
            smoothness: 0.5,
            normal: Vec3::Z,
            emission: Vec3::ZERO,
        };
        let light = Light {
            to_light: Vec3::Z,
            radiance: Vec3::ZERO,
        };
        let c = standard_brdf(
            &surface,
            Vec3::Z,
            &light,
            &Indirect {
                diffuse: Vec3::splat(0.5),
                specular: Vec3::splat(1.0),
            },
        );
        // 真正面（nv = 1）の FresnelLerp は F0 = 0.04。surfaceReduction = 1 / (0.25² + 1)
        let expected = 0.5 * 0.96 + 0.04 / (0.0625 + 1.0);
        assert!(close(c.x, expected, 1e-4), "{c:?} vs {expected}");
        // 斜めから見ると映り込みが強くなる（フレネル）
        let v = Vec3::new(0.9, 0.0, 0.1).normalize();
        let grazing = standard_brdf(
            &surface,
            v,
            &light,
            &Indirect {
                diffuse: Vec3::ZERO,
                specular: Vec3::ONE,
            },
        );
        assert!(grazing.x > 0.2, "斜めの映り込み: {grazing:?}");
    }

    #[test]
    fn mip_mapping_is_the_unity_curve_and_invertible() {
        assert!(close(mip_of_roughness(0.0), 0.0, 1e-6));
        assert!(close(mip_of_roughness(1.0), 6.0, 1e-5));
        assert!(close(mip_of_roughness(0.5), 4.05, 1e-4));
        for mip in 0..=6 {
            let r = roughness_of_mip(mip);
            assert!(close(mip_of_roughness(r), mip as f32, 1e-3), "mip {mip}");
        }
    }

    #[test]
    fn tone_map_none_clips_and_curves_are_monotonic_and_bounded() {
        let mid = Vec3::splat(0.5);
        assert!(close3(tone_map(mid, Curve::None, 0.0), mid, 1e-5));
        // +1 EV はリニアで 2 倍
        let lin = srgb_to_linear(0.5) * 2.0;
        assert!(close(
            tone_map(mid, Curve::None, 1.0).x,
            linear_to_srgb(lin),
            1e-5
        ));
        // 明るすぎる値は 1 で切る（None）
        assert!(close(
            tone_map(Vec3::splat(3.0), Curve::None, 0.0).x,
            1.0,
            1e-6
        ));
        for curve in [Curve::Neutral, Curve::Aces] {
            let mut last = -1.0;
            for i in 0..=40 {
                let v = tone_map(Vec3::splat(i as f32 * 0.1), curve, 0.0).x;
                assert!(
                    v >= last - 1e-6 && (0.0..=1.0).contains(&v),
                    "{curve:?} {i}"
                );
                last = v;
            }
            // 暗い所は沈み、明るい所は 1 に近づく
            assert!(tone_map(Vec3::splat(0.02), curve, 0.0).x < 0.12);
            assert!(tone_map(Vec3::splat(8.0), curve, 0.0).x > 0.95);
        }
        // Neutral は 1 の入力（ガンマ）を 1 には戻さない（肩を持つ。Unity の Post Processing の曲線で約 0.816）
        let n = tone_map(Vec3::ONE, Curve::Neutral, 0.0).x;
        assert!(close(n, 0.816, 0.01), "{n}");
        // ACES（Unity 版のシェーダーの式。入力に 1/0.6 の前の露出は掛けない）: リニア 0.18 → 約 0.106
        let linear_018 = tone_map(Vec3::splat(linear_to_srgb(0.18)), Curve::Aces, 0.0).x;
        assert!(
            close(srgb_to_linear(linear_018), 0.106, 0.005),
            "{linear_018}"
        );
    }

    #[test]
    fn sh_uniform_environment_and_rotation() {
        let mut sh = [Vec3::ZERO; 9];
        sh[0] = Vec3::splat(0.7);
        for n in [Vec3::X, Vec3::Y, Vec3::NEG_Z] {
            assert!(close3(evaluate_sh(&sh, n), Vec3::splat(0.7), 1e-6));
        }
        // 90° 回すと、元の +X の向きに見えるのは世界の ... （to_source は R(−θ) d）
        let r = Vec2::new(0.0, 1.0); // θ = 90°
        let d = to_source(Vec3::Z, r);
        assert!(close3(d, Vec3::new(-1.0, 0.0, 0.0), 1e-6), "{d:?}");
        assert!(close3(to_source(Vec3::Y, r), Vec3::Y, 1e-6), "上は動かない");
    }

    #[test]
    fn normal_map_frame_matches_unity() {
        // 平らな法線 (0, 0, 1) は法線のまま、+X は接線へ、+Y は従接線へ
        let n = Vec3::Z;
        let t = Vec3::X;
        let b = bitangent(n, t, 1.0);
        assert!(close3(b, Vec3::new(0.0, 1.0, 0.0), 1e-6), "cross(Z, X) = Y");
        assert!(close3(perturb_normal(Vec3::Z, t, b, n), n, 1e-6));
        let tilted = perturb_normal(Vec3::new(1.0, 0.0, 1.0).normalize(), t, b, n);
        assert!(tilted.x > 0.7 && tilted.z > 0.7);
        let flipped = bitangent(n, t, -1.0);
        assert!(close3(flipped, Vec3::new(0.0, -1.0, 0.0), 1e-6));
    }
}
