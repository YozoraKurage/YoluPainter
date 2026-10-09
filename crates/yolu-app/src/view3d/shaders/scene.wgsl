// 3D ビューの面の描画。Unity 版のプレビューのシェーダー（PreviewSurface・PreviewSurfaceLit・環境の背景）と、Unity の Standard の BRDF
// （UnityStandardBRDF.cginc の BRDF1_Unity_PBS、リニア）を同じ式で移したもの。brdf.rs が CPU の参照（試験が照らす）。
//
// 出力は「画面にそのまま出す値」（ガンマ）。中立・チャンネルだけの表示はガンマの空間のまま計算し（Unity 版の中立の表示と同じ式）、
// マテリアル表示はリニアで解いて最後にガンマへ直す。トーンマッピングを使うときの描き先は HDR で、1 を超えた値もそのまま書く。
//
// 表示（u.mode.x）: 0 マテリアル（PBR）・1 中立・2 チャンネルだけ（光なし。u.mode.y が見るチャンネル）・3 メッシュマップだけ（光なし）。

const PI: f32 = 3.14159265358979;
const DIELECTRIC: f32 = 0.04;

struct Uniforms {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    // xyz: カメラの位置、w: 正投影なら 1（透視は 0）
    camera: vec4<f32>,
    // xyz: 光へ向かう向き
    light_dir: vec4<f32>,
    // マテリアル表示: 直接光（リニアの放射輝度）
    light_color: vec4<f32>,
    // マテリアル表示: 環境が無いときの一様な環境光（リニア）
    ambient: vec4<f32>,
    // 中立の表示: 光の倍率・環境光の倍率（ガンマの空間）
    neutral_light: vec4<f32>,
    neutral_ambient: vec4<f32>,
    // x: 環境を使う（1）、y: 環境の明るさ、z: 中立の映り込みの mip、w: 背景を描く（1）
    env: vec4<f32>,
    // x: cos θ、y: sin θ（環境を上の軸のまわりに θ 回す）、z: 背景の mip、w: 未使用
    env_rot: vec4<f32>,
    // x: 表示、y: チャンネルだけの表示で見る番号、z: 法線マップを使う、w: 接線が出来ている
    mode: vec4<f32>,
    sh: array<vec4<f32>, 9>,
    // 影: 世界 → (u, v, 深さ 0〜1。0 が光の側)
    shadow_matrix: mat4x4<f32>,
    // x: 影を使う（1）、y: ぼかしの半径（u v）、z: 深さの偏りの基本（深さの単位）、w: 法線の向きにずらす量（世界）
    shadow: vec4<f32>,
    // lilToon の光（liltoon/light.wgsl）: 環境光の SH を Unity の unity_SHAr・SHAg・SHAb・SHBr・SHBg・SHBb・SHC の形で（環境の明るさ込み。
    // 環境を使わないときは一様な環境光を L0 に）
    lil_sh: array<vec4<f32>, 7>,
    // カメラの上と、画面から手前への向き（Unity の視点の行列の 1 行目と 2 行目）
    lil_camera_up: vec4<f32>,
    lil_camera_front: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(7) var paint_sampler: sampler;
@group(0) @binding(8) var env_cube: texture_cube<f32>;
@group(0) @binding(9) var env_sampler: sampler;
@group(0) @binding(10) var map_tex: texture_2d<f32>;
@group(0) @binding(11) var shadow_map: texture_depth_2d;
@group(0) @binding(12) var shadow_sampler: sampler_comparison;

// group 1: マテリアル（テクスチャセット）ごとの絵。マテリアルごとに束ねを替えて描く（絵を持たない面は既定の 1 × 1 と painted = 0）。
@group(1) @binding(0) var color_tex: texture_2d<f32>;
@group(1) @binding(1) var metallic_tex: texture_2d<f32>;
@group(1) @binding(2) var roughness_tex: texture_2d<f32>;
@group(1) @binding(3) var normal_tex: texture_2d<f32>;
@group(1) @binding(4) var emission_tex: texture_2d<f32>;
@group(1) @binding(5) var height_tex: texture_2d<f32>;
struct SetParams {
    // x: 絵を貼る（1）、y: このセットの Normal を法線マップに読む（1）、z: 今のセット（1。焼いたメッシュマップは今のセットの面だけ）
    flags: vec4<f32>,
};
@group(1) @binding(6) var<uniform> set_params: SetParams;

struct VsIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) tangent: vec4<f32>,
};
struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) tangent: vec3<f32>,
    @location(3) bitangent: vec3<f32>,
    @location(4) uv: vec2<f32>,
};

@vertex
fn vs_main(v: VsIn) -> VsOut {
    var o: VsOut;
    o.clip = u.view_proj * vec4<f32>(v.position, 1.0);
    o.world = v.position;
    o.normal = v.normal;
    o.tangent = v.tangent.xyz;
    // Unity の頂点シェーダーと同じ: cross(normal, tangent) × w
    o.bitangent = cross(v.normal, v.tangent.xyz) * v.tangent.w;
    o.uv = v.uv;
    return o;
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((max(c, vec3<f32>(0.0)) + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let v = max(c, vec3<f32>(0.0));
    let lo = v * 12.92;
    let hi = 1.055 * pow(v, vec3<f32>(1.0 / 2.4)) - vec3<f32>(0.055);
    return select(hi, lo, v <= vec3<f32>(0.0031308));
}

// リニアの乗算済みの色 → ガンマの乗算済み（ガンマの値 × α）
fn premultiplied_gamma(c: vec4<f32>) -> vec3<f32> {
    if (c.a <= 0.0) {
        return vec3<f32>(0.0);
    }
    return linear_to_srgb(c.rgb / c.a) * c.a;
}

fn saturate(x: f32) -> f32 {
    return clamp(x, 0.0, 1.0);
}

fn pow5(x: f32) -> f32 {
    let x2 = x * x;
    return x2 * x2 * x;
}

// 環境を上の軸のまわりに θ 回したとき、世界の向き d に見える元の環境の向き（R(−θ) d）
fn to_source(d: vec3<f32>) -> vec3<f32> {
    let c = u.env_rot.x;
    let s = u.env_rot.y;
    return vec3<f32>(d.x * c - d.z * s, d.y, d.x * s + d.z * c);
}

// Unity の SphericalHarmonicsL2 の並びと式（拡散の色 = 照度 / π）
fn evaluate_sh(n: vec3<f32>) -> vec3<f32> {
    var r = u.sh[0].rgb + u.sh[1].rgb * n.y + u.sh[2].rgb * n.z + u.sh[3].rgb * n.x;
    r = r + u.sh[4].rgb * (n.x * n.y) + u.sh[5].rgb * (n.y * n.z) + u.sh[6].rgb * (3.0 * n.z * n.z - 1.0);
    r = r + u.sh[7].rgb * (n.x * n.z) + u.sh[8].rgb * (n.x * n.x - n.y * n.y);
    return max(r, vec3<f32>(0.0));
}

fn checker_gamma(uv: vec2<f32>) -> vec3<f32> {
    let cell = floor(uv.x * 24.0) + floor(uv.y * 24.0);
    let c = cell - 2.0 * floor(cell * 0.5);
    return mix(vec3<f32>(0.24), vec3<f32>(0.34), c);
}

// ───────── 影（光から見た深さのマップ。Unity 版の YPShadow と同じ 16 点の回すサンプリング） ─────────

const POISSON = array<vec2<f32>, 16>(
    vec2<f32>(-0.94201624, -0.39906216), vec2<f32>(0.94558609, -0.76890725), vec2<f32>(-0.09418410, -0.92938870), vec2<f32>(0.34495938, 0.29387760),
    vec2<f32>(-0.91588581, 0.45771432), vec2<f32>(-0.81544232, -0.87912464), vec2<f32>(-0.38277543, 0.27676845), vec2<f32>(0.97484398, 0.75648379),
    vec2<f32>(0.44323325, -0.97511554), vec2<f32>(0.53742981, -0.47373420), vec2<f32>(-0.26496911, -0.41893023), vec2<f32>(0.79197514, 0.19090188),
    vec2<f32>(-0.24188840, 0.99706507), vec2<f32>(-0.81409955, 0.91437590), vec2<f32>(0.19984126, 0.78641367), vec2<f32>(0.14383161, -0.14100790),
);

// 影の量（0 = 光が当たる、1 = 影）。normal は面の法線（世界）、pixel は画面の画素の位置（ぼかしの向きを画素ごとに回す）。
// Unity 版は深さの偏りをぼかしの半径に比例する定数で押さえていたが、ここでは面の傾きから深さの勾配を出して、読む点ごとに
// 面の上の深さへ直す（受け手が平面とみなす偏り）。ぼかしを広げても、光に寝た面が自分に影を落とさない。
fn shadow_amount(world: vec3<f32>, normal: vec3<f32>, to_light: vec3<f32>, pixel: vec2<f32>) -> f32 {
    if (u.shadow.x < 0.5) {
        return 0.0;
    }
    // 光に対して寝た面ほど法線の向きに大きくずらす（影のにきびを避ける）
    let nl = dot(normal, to_light);
    let p = world + normal * u.shadow.w * saturate(1.5 - abs(nl));
    let s = (u.shadow_matrix * vec4<f32>(p, 1.0)).xyz;
    if (s.x < 0.0 || s.y < 0.0 || s.x > 1.0 || s.y > 1.0 || s.z > 1.0) {
        return 0.0;
    }
    // 面の上で (u, v) を du, dv 動かしたときの深さの変化: (n_u, n_v) / n_l（光に寝るほど急。寝すぎた面は 0.15 で頭打ち）
    let ns = (u.shadow_matrix * vec4<f32>(normal, 0.0)).xyz;
    let unit = ns / max(length(ns), 1e-12);
    let slope = unit.xy / max(-unit.z, 0.15);
    let bias = u.shadow.z * (1.0 + length(slope));
    let angle = 6.2831853 * fract(52.9829189 * fract(dot(pixel, vec2<f32>(0.06711056, 0.00583715))));
    let ca = cos(angle);
    let sa = sin(angle);
    var lit = 0.0;
    for (var k = 0; k < 16; k = k + 1) {
        let o = POISSON[k];
        let offset = vec2<f32>(o.x * ca - o.y * sa, o.x * sa + o.y * ca) * u.shadow.y;
        let depth = s.z + dot(slope, offset) - bias;
        // 比較サンプラー（線形）: 深さが覆われていなければ 1。4 点の補間つき
        lit = lit + textureSampleCompareLevel(shadow_map, shadow_sampler, s.xy + offset, depth);
    }
    return 1.0 - lit / 16.0;
}

// ───────── Unity の Standard の BRDF（BRDF1_Unity_PBS、リニア） ─────────

fn fresnel_term(f0: vec3<f32>, cos_a: f32) -> vec3<f32> {
    return f0 + (vec3<f32>(1.0) - f0) * pow5(1.0 - cos_a);
}

fn fresnel_lerp(f0: vec3<f32>, f90: f32, cos_a: f32) -> vec3<f32> {
    return mix(f0, vec3<f32>(f90), pow5(1.0 - cos_a));
}

fn disney_diffuse(nv: f32, nl: f32, lh: f32, perceptual_roughness: f32) -> f32 {
    let fd90 = 0.5 + 2.0 * lh * lh * perceptual_roughness;
    let light_scatter = 1.0 + (fd90 - 1.0) * pow5(1.0 - nl);
    let view_scatter = 1.0 + (fd90 - 1.0) * pow5(1.0 - nv);
    return light_scatter * view_scatter;
}

fn ggx_term(nh: f32, roughness: f32) -> f32 {
    let a2 = roughness * roughness;
    let d = (nh * a2 - nh) * nh + 1.0;
    return (1.0 / PI) * a2 / (d * d + 1e-7);
}

fn smith_joint_ggx_visibility(nl: f32, nv: f32, roughness: f32) -> f32 {
    let a = roughness;
    let lambda_v = nl * (nv * (1.0 - a) + a);
    let lambda_l = nv * (nl * (1.0 - a) + a);
    return 0.5 / (lambda_v + lambda_l + 1e-5);
}

fn safe_normalize(v: vec3<f32>) -> vec3<f32> {
    return v / sqrt(max(0.001, dot(v, v)));
}

fn standard_brdf(
    albedo: vec3<f32>, metallic: f32, smoothness_in: f32, normal_in: vec3<f32>, view: vec3<f32>,
    to_light: vec3<f32>, light_color: vec3<f32>, gi_diffuse: vec3<f32>, gi_specular: vec3<f32>,
) -> vec3<f32> {
    let one_minus_dielectric = 1.0 - DIELECTRIC;
    let spec_color = mix(vec3<f32>(DIELECTRIC), albedo, metallic);
    let one_minus_reflectivity = one_minus_dielectric - metallic * one_minus_dielectric;
    let diff_color = albedo * one_minus_reflectivity;

    let smoothness = clamp(smoothness_in, 0.0, 1.0);
    let perceptual_roughness = 1.0 - smoothness;
    let half_dir = safe_normalize(to_light + view);
    var normal = normal_in;
    let shift_amount = dot(normal, view);
    if (shift_amount < 0.0) {
        normal = normal + view * (-shift_amount + 1e-5);
    }
    let nv = saturate(dot(normal, view));
    let nl = saturate(dot(normal, to_light));
    let nh = saturate(dot(normal, half_dir));
    let lh = saturate(dot(to_light, half_dir));

    let diffuse_term = disney_diffuse(nv, nl, lh, perceptual_roughness) * nl;

    let roughness = max(perceptual_roughness * perceptual_roughness, 0.002);
    let vis = smith_joint_ggx_visibility(nl, nv, roughness);
    let d = ggx_term(nh, roughness);
    var specular_term = vis * d * PI;
    specular_term = max(0.0, specular_term * nl);
    let surface_reduction = 1.0 / (roughness * roughness + 1.0);
    if (all(spec_color == vec3<f32>(0.0))) {
        specular_term = 0.0;
    }
    let grazing = saturate(smoothness + (1.0 - one_minus_reflectivity));
    return diff_color * (gi_diffuse + light_color * diffuse_term)
        + light_color * fresnel_term(spec_color, lh) * specular_term
        + gi_specular * fresnel_lerp(spec_color, grazing, nv) * surface_reduction;
}

@fragment
fn fs_main(f: VsOut) -> @location(0) vec4<f32> {
    // 標本の取り出しは条件の外で（導関数の一様性）。絵は乗算済み（行は文書と同じ下から上: v がそのまま文書の y）。Color・Emission は
    // sRGB の形式で、読んだ値はリニア（リニアで補間した値。Color はリニアの乗算済み）
    let tex_color = textureSample(color_tex, paint_sampler, f.uv);
    let tex_metallic = textureSample(metallic_tex, paint_sampler, f.uv).r;
    let tex_roughness = textureSample(roughness_tex, paint_sampler, f.uv).r;
    let tex_normal = textureSample(normal_tex, paint_sampler, f.uv).rgb;
    let tex_emission = textureSample(emission_tex, paint_sampler, f.uv).rgb;
    // 市松の上に重ねるのは今までどおりガンマの値で（画面の 2D のキャンバスと同じ見え方）
    let color_gamma = premultiplied_gamma(tex_color);
    let tex_height = textureSample(height_tex, paint_sampler, f.uv).r;
    let tex_map = textureSample(map_tex, paint_sampler, f.uv);
    let painted = set_params.flags.x > 0.5;
    let background = checker_gamma(f.uv);
    var base_gamma = background;
    if (painted) {
        base_gamma = background * (1.0 - tex_color.a) + color_gamma;
    }

    // チャンネルだけ・メッシュマップだけ（光なし）: 値をそのまま出す
    if (u.mode.x > 1.5) {
        var c = base_gamma;
        if (u.mode.x > 2.5) {
            // 焼いたメッシュマップ（乗算済み。覆わないテクセルは透明 = 市松）
            c = background;
            if (painted && set_params.flags.z > 0.5) {
                c = background * (1.0 - tex_map.a) + tex_map.rgb;
            }
        } else if (painted) {
            let ch = i32(u.mode.y + 0.5);
            if (ch == 1) {
                c = vec3<f32>(tex_metallic);
            } else if (ch == 2) {
                c = vec3<f32>(tex_roughness);
            } else if (ch == 3) {
                c = tex_normal;
            } else if (ch == 4) {
                c = linear_to_srgb(tex_emission);
            } else if (ch == 5) {
                c = vec3<f32>(tex_height);
            }
        }
        return vec4<f32>(c, 1.0);
    }

    let geometric = normalize(f.normal);
    var n = geometric;
    if (painted && set_params.flags.y > 0.5 && u.mode.z > 0.5 && u.mode.w > 0.5) {
        // YoluPainter の Normal の出力: リニアの RGB に詰めた接空間の法線（OpenGL の Y+）。補間したままの接線・従接線・法線で読む
        let t = tex_normal * 2.0 - vec3<f32>(1.0);
        n = normalize(f.tangent * t.x + f.bitangent * t.y + geometric * t.z);
    }
    let to_light = normalize(u.light_dir.xyz);
    let view = normalize(u.camera.xyz - f.world);
    let env_on = u.env.x > 0.5;
    // 影は面の法線（補間した頂点の法線）で引く。法線マップの傾きは影の位置を動かさない
    let shadow = shadow_amount(f.world, geometric, to_light, f.clip.xy);
    let intensity = u.env.y;

    // 中立の表示（PreviewSurface / PreviewSurfaceLit）: ガンマの空間のまま、0.35 + 0.65 × saturate(n·L)。環境を使うときは環境の拡散と映り込み
    if (u.mode.x > 0.5) {
        var ambient = u.neutral_ambient.rgb;
        var reflection = vec3<f32>(0.0);
        if (env_on) {
            ambient = evaluate_sh(to_source(n)) * intensity;
            let nv = saturate(dot(n, view));
            let fresnel = 0.04 + (0.5 - 0.04) * pow5(1.0 - nv);
            reflection = textureSampleLevel(env_cube, env_sampler, to_source(reflect(-view, n)), u.env.z).rgb * fresnel * intensity;
        }
        let light = ambient + u.neutral_light.rgb * saturate(dot(n, to_light)) * (1.0 - shadow);
        return vec4<f32>(base_gamma * light + reflection, 1.0);
    }

    // マテリアル表示（金属の流儀の PBR）。リニアで解いて、最後にガンマへ
    var albedo = srgb_to_linear(base_gamma);
    var metallic = 0.0;
    var smoothness = 0.5;
    var emission = vec3<f32>(0.0);
    if (painted) {
        metallic = tex_metallic;
        smoothness = 1.0 - tex_roughness;
        emission = tex_emission;
    }
    var gi_diffuse = u.ambient.rgb;
    var gi_specular = u.ambient.rgb;
    if (env_on) {
        gi_diffuse = evaluate_sh(to_source(n)) * intensity;
        // 粗さ（知覚的）に Unity と同じ mip（r × (1.7 − 0.7 r) × 6）
        let perceptual_roughness = 1.0 - clamp(smoothness, 0.0, 1.0);
        let mip = perceptual_roughness * (1.7 - 0.7 * perceptual_roughness) * 6.0;
        gi_specular = textureSampleLevel(env_cube, env_sampler, to_source(reflect(-view, n)), mip).rgb * intensity;
    }
    // 影は直接光（拡散も鏡面も）だけを消す。環境光・映り込み・発光は残る（Unity 版は掛け算の重ね描きで、拡散だけを近似していた）
    let direct = u.light_color.rgb * (1.0 - shadow);
    let color = standard_brdf(albedo, metallic, smoothness, n, view, to_light, direct, gi_diffuse, gi_specular) + emission;
    return vec4<f32>(linear_to_srgb(color), 1.0);
}

// ───────── 背景（環境をぼかして画面いっぱいに） ─────────

struct BgOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs_bg(@builtin(vertex_index) i: u32) -> BgOut {
    let x = f32(i32(i & 1u) * 4 - 1);
    let y = f32(i32(i >> 1u) * 4 - 1);
    var o: BgOut;
    o.clip = vec4<f32>(x, y, 0.5, 1.0);
    o.ndc = vec2<f32>(x, y);
    return o;
}

@fragment
fn fs_bg(f: BgOut) -> @location(0) vec4<f32> {
    let p = u.inv_view_proj * vec4<f32>(f.ndc, 0.5, 1.0);
    var d = normalize(p.xyz / p.w - u.camera.xyz);
    if (u.camera.w > 0.5) {
        // 正投影: 視線はどこも前の向き（画面の奥へ）
        d = -u.lil_camera_front.xyz;
    }
    let c = textureSampleLevel(env_cube, env_sampler, to_source(d), u.env_rot.z).rgb * u.env.y;
    return vec4<f32>(linear_to_srgb(c), 1.0);
}
