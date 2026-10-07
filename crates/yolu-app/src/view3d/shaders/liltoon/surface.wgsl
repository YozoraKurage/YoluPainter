// 出どころ: lilToon 2.3.4（MIT、Copyright (c) 2020-present lilxyzw）の lil_pass_forward_normal.hlsl・lil_common_frag.hlsl。許諾と出どころの全文は ../THIRD-PARTY-NOTICES.md。
// ───────── 面 ─────────

@fragment
fn fs_liltoon(f: VsOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    return lil_output(lil_shade(f, front), i32(lil.p[P_MODE].x + 0.5) == 2);
}

// 半透明を sRGB の描き先へ（ハードウェアがリニアで重ねる。Unity と同じ重ね方）: リニアの乗算済みのまま書く。
@fragment
fn fs_liltoon_linear(f: VsOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    return lil_shade(f, front);
}

// 面の色（リニア。半透明は乗算済み）。
fn lil_shade(f: VsOut, front: bool) -> vec4<f32> {
    let facing = select(-1.0, 1.0, front);
    let mode = i32(lil.p[P_MODE].x + 0.5);
    let transparent = mode == 2;
    let uv0 = f.uv;
    // メインの UV（lilCalcDoubleSideUV と lilCalcUV。角度は時刻 0 の回転）
    let uvp = lil.p[P_UV];
    var uv_base = uv0;
    if (facing < uvp.y - 1.0) {
        uv_base.x = uv_base.x + 1.0;
    }
    let main_st = lil.p[P_MAIN_ST];
    let uv_main = rotate_uv(uv_base * main_st.xy + main_st.zw, uvp.x);
    // 勾配は条件の外で（導関数の一様性）
    let gx = dpdx(uv_main);
    let gy = dpdy(uv_main);
    let gx0 = dpdx(uv0);
    let gy0 = dpdy(uv0);

    // メインカラー（3D ビューの約束: Color の何も描いていない所は、不透明なら市松の上に見せる）
    var col = slot_grad(SLOT_MAIN, uv_main, gx, gy);
    if (mode == 0 && lil.slot_flags[SLOT_MAIN].y > 0.5 && i32(lil.slot_flags[SLOT_MAIN].z + 0.5) == 0) {
        let p = textureSampleGrad(color_tex, paint_sampler, uv_main, gx, gy);
        let g = checker_gamma(uv0) * (1.0 - p.a) + premultiplied_gamma(p);
        col = vec4<f32>(srgb_to_linear(g), 1.0);
    }
    // 色調補正
    let before = col.rgb;
    let adjust_mask = slot_grad(SLOT_ADJUST_MASK, uv_main, gx, gy).r;
    col = vec4<f32>(mix(before, lil_tone(col.rgb, lil.p[P_HSVG]), adjust_mask), col.a);
    col = col * lil.p[P_COLOR];

    // 法線（ノーマルマップ・ノーマルマップ 2nd を接空間で重ねる）
    let geometric = normalize(f.normal);
    var n = geometric;
    let has_tangents = lil.p[P_FLAGS].x > 0.5;
    var normalmap = vec3<f32>(0.0, 0.0, 1.0);
    var use_normalmap = false;
    let bump = lil.p[P_BUMP];
    if (feat(F_BUMP) && bump.x > 0.5) {
        let bump_st = lil.p[P_BUMP_ST];
        let t = slot_grad(SLOT_BUMP, uv_main * bump_st.xy + bump_st.zw, gx * bump_st.xy, gy * bump_st.xy);
        normalmap = lil_unpack_normal(t, bump.y, slot_received(SLOT_BUMP));
        use_normalmap = true;
    }
    let bump2 = lil.p[P_BUMP2];
    if (feat(F_BUMP2) && bump2.x > 0.5) {
        let st2 = lil.p[P_BUMP2_ST];
        let t2 = slot_grad(SLOT_BUMP2, uv0 * st2.xy + st2.zw, gx0 * st2.xy, gy0 * st2.xy);
        let scale2 = bump2.y * slot_grad(SLOT_BUMP2_MASK, uv_main, gx, gy).r;
        normalmap = lil_blend_normal(normalmap, lil_unpack_normal(t2, scale2, slot_received(SLOT_BUMP2)));
        use_normalmap = true;
    }
    var tangent = f.tangent;
    var bitangent = f.bitangent;
    if (use_normalmap && has_tangents) {
        n = normalize(tangent * normalmap.x + bitangent * normalmap.y + geometric * normalmap.z);
    }
    let flip = lil.p[P_LIGHT2].w;
    if (facing < flip - 1.0) {
        n = -n;
    }
    let orig_n = geometric;
    let view = normalize(u.camera.xyz - f.world);
    let nv = saturate(dot(n, view));
    let nvabs = abs(dot(n, view));
    let depth = distance(u.camera.xyz, f.world);
    // マットキャップの UV の向き（lilToon の fd.uvMat）
    let uv_mat = view_dir_xy(n) * 0.5 + 0.5;
    // 右手の接空間か（デカールのミラー。Unity の接線の w）
    let right_hand = dot(cross(f.normal, f.tangent), f.bitangent) > 0.0;

    // 光
    let light = lil_main_light();
    let to_light = light.dir;
    var light_color = light.color;
    let ind_light = light.ind;
    let inv_lighting = sat3((vec3<f32>(1.0) - light_color) * sqrt(light_color));
    let attenuation = 1.0 - shadow_amount(f.world, geometric, normalize(u.light_dir.xyz), f.clip.xy);
    // 影を受け取る（主な光の影のマップ）
    let calculated = saturate(attenuation + distance(to_light, normalize(u.light_dir.xyz)));

    // メインカラー 2nd・3rd（光の前に重ねる分。光の後の分は下）
    var layer2 = vec4<f32>(0.0);
    var layer3 = vec4<f32>(0.0);
    let l2 = lil.p[P_LAYER2];
    if (feat(F_MAIN2) && l2.x > 0.5) {
        let o = lil_layer(P_LAYER2, SLOT_MAIN2, SLOT_MAIN2_MASK, uv0, uv_mat, uv_main, gx, gy, nv, depth, facing, right_hand, col.a, mode);
        layer2 = o.color;
        col.a = o.alpha;
        col = vec4<f32>(lil_blend(col.rgb, layer2.rgb, vec3<f32>(layer2.a * l2.y), i32(l2.z + 0.5)), col.a);
    }
    let l3 = lil.p[P_LAYER3];
    if (feat(F_MAIN3) && l3.x > 0.5) {
        let o = lil_layer(P_LAYER3, SLOT_MAIN3, SLOT_MAIN3_MASK, uv0, uv_mat, uv_main, gx, gy, nv, depth, facing, right_hand, col.a, mode);
        layer3 = o.color;
        col.a = o.alpha;
        col = vec4<f32>(lil_blend(col.rgb, layer3.rgb, vec3<f32>(layer3.a * l3.y), i32(l3.z + 0.5)), col.a);
    }

    // 異方性反射（接線・光沢・マットキャップの法線）
    var matcap_n = n;
    var matcap2_n = n;
    var reflection_n = n;
    var anisotropy = 0.0;
    var perceptual_roughness = 1.0;
    var aniso_spec = false;
    let an = lil.p[P_ANISO];
    if (feat(F_ANISO) && an.x > 0.5) {
        let ast = lil.p[P_ANISO_ST];
        let tmap = slot_grad(SLOT_ANISO_TANGENT, uv_main * ast.xy + ast.zw, gx * ast.xy, gy * ast.xy);
        let at = lil_unpack_normal(tmap, 1.0, slot_received(SLOT_ANISO_TANGENT));
        tangent = ortho_normalize(normalize(f.tangent * at.x + f.bitangent * at.y + geometric * at.z), n);
        bitangent = cross(n, tangent);
        let mst = lil.p[P_ANISO_MASK_ST];
        anisotropy = an.y * slot_grad(SLOT_ANISO_MASK, uv_main * mst.xy + mst.zw, gx * mst.xy, gy * mst.xy).r;
        // lilGetAnisotropyNormalWS
        var adir = tangent;
        if (anisotropy > 0.0) {
            adir = bitangent;
        }
        adir = ortho_normalize(view, adir);
        let aniso_n = normalize(mix(n, adir, abs(anisotropy)));
        if (an.z > 0.5) {
            reflection_n = aniso_n;
            perceptual_roughness = saturate(1.2 - abs(anisotropy));
            aniso_spec = true;
        }
        if (an.w > 0.5) {
            matcap_n = aniso_n;
        }
        if (lil.p[P_ANISO_Q].x > 0.5) {
            matcap2_n = aniso_n;
        }
    }

    // 影（導関数を使う値は、アルファで捨てる前に取る）
    var shadowmix = 1.0;
    let sp = lil.p[P_SHADOW];
    let lod = lil.p[P_SHADOW_LOD];
    let s1 = lil.p[P_SHADOW1];
    let s2 = lil.p[P_SHADOW2];
    let s3 = lil.p[P_SHADOW3];
    let flat = lil.p[P_SHADOW_FLAT];
    let mask_type = i32(sp.z + 0.5);
    var lns = vec4<f32>(1.0);
    var ssm = vec4<f32>(1.0);
    var shadow1_tex = vec4<f32>(0.0);
    var shadow2_tex_in = vec4<f32>(0.0);
    var shadow3_tex_in = vec4<f32>(0.0);
    if (feat(F_SHADOW)) {
        let ssm_tex = slot_grad(SLOT_SHADOW_STRENGTH, uv_main, max(abs(gx), vec2<f32>(lod.x)), max(abs(gy), vec2<f32>(lod.x)));
        let blur_mask = slot_grad(SLOT_SHADOW_BLUR, uv_main, max(abs(gx), vec2<f32>(lod.z)), max(abs(gy), vec2<f32>(lod.z)));
        let border_tex = slot_grad(SLOT_SHADOW_BORDER, uv_main, max(abs(gx), vec2<f32>(lod.y)), max(abs(gy), vec2<f32>(lod.y)));
        shadow1_tex = slot_grad(SLOT_SHADOW1_TEX, uv_main, gx, gy);
        shadow2_tex_in = slot_grad(SLOT_SHADOW2_TEX, uv_main, gx, gy);
        shadow3_tex_in = slot_grad(SLOT_SHADOW3_TEX, uv_main, gx, gy);
        let n1 = mix(orig_n, n, s1.z);
        let n2 = mix(orig_n, n, s2.z);
        let n3 = mix(orig_n, n, s3.z);
        ssm = ssm_tex;
        lns.x = saturate(dot(to_light, n1) * 0.5 + 0.5);
        lns.y = saturate(dot(to_light, n2) * 0.5 + 0.5);
        lns.z = saturate(dot(to_light, n3) * 0.5 + 0.5);
        var aa = lil.p[P_LIGHT2].y;
        if (mask_type == 2) {
            // SDF（顔の影）。物の向きは世界の向き（モデルは世界の空間）: 右が −X、前が +Z
            let face_r = vec3<f32>(-1.0, 0.0, 0.0);
            let l_dot_r = dot(to_light.xz, face_r.xz);
            let sdf = select(ssm.r, ssm.g, l_dot_r < 0.0);
            var face_f = vec3<f32>(0.0, 0.0, 1.0);
            face_f.y = face_f.y * flat.y;
            face_f = select(normalize(face_f), vec3<f32>(0.0), dot(face_f, face_f) == 0.0);
            var face_l = to_light;
            face_l.y = face_l.y * flat.y;
            face_l = select(normalize(face_l), vec3<f32>(0.0), dot(face_l, face_l) == 0.0);
            let ln_sdf = dot(face_l, face_f);
            lns = mix(vec4<f32>(saturate(ln_sdf * 0.5 + sdf * 0.5 + 0.25)), lns, ssm.b);
            aa = 0.0;
            ssm.r = ssm.a;
        }
        lns.x = lns.x * mix(1.0, calculated, s1.w);
        lns.y = lns.y * mix(1.0, calculated, s2.w);
        lns.z = lns.z * mix(1.0, calculated, s3.w);
        // ぼかしマスク
        let blur1 = s1.y * blur_mask.r;
        let blur2 = s2.y * blur_mask.g;
        let blur3 = s3.y * blur_mask.b;
        // AO
        var bm = border_tex;
        let ao = lil.p[P_AO_SHIFT];
        let ao2 = lil.p[P_AO_SHIFT2];
        bm.r = saturate(bm.r * ao.x + ao.y);
        bm.g = saturate(bm.g * ao.z + ao.w);
        bm.b = saturate(bm.b * ao2.x + ao2.y);
        let post_ao = sp.w > 0.5;
        if (!post_ao) {
            lns = vec4<f32>(lns.xyz * bm.rgb, lns.w);
        }
        let fw_x = fwidth(lns.x);
        let fw_y = fwidth(lns.y);
        let fw_z = fwidth(lns.z);
        lns.w = lns.x;
        lns.x = lil_tooning_ns(aa, lns.x, s1.x, blur1, fw_x);
        lns.y = lil_tooning_ns(aa, lns.y, s2.x, blur2, fw_y);
        lns.w = lil_tooning_ns_range(aa, lns.w, s1.x, blur1, flat.z, fw_x);
        lns.z = lil_tooning_ns(aa, lns.z, s3.x, blur3, fw_z);
        if (post_ao) {
            lns = lns * bm.rgbr;
        }
        lns = clamp(lns, vec4<f32>(0.0), vec4<f32>(1.0));
    }

    let aa_main = lil.p[P_LIGHT2].y;
    // リムシェード（導関数は捨てる前に）
    var rim_shade = 0.0;
    let rs = lil.p[P_RIM_SHADE];
    if (feat(F_RIM_SHADE) && rs.x > 0.5) {
        let rs_n = mix(orig_n, n, rs.y);
        let rs_nv = abs(dot(rs_n, view));
        let r0 = pow(saturate(1.0 - rs_nv), lil.p[P_RIM_SHADE2].x);
        rim_shade = saturate(lil_tooning_ns(aa_main, r0, rs.z, rs.w, fwidth(r0)));
        rim_shade = rim_shade * lil.p[P_RIM_SHADE_COLOR].a * slot_grad(SLOT_RIM_SHADE_MASK, uv_main, gx, gy).r;
    }
    // 逆光ライト（導関数は捨てる前に）
    var backlight = 0.0;
    var backlight_color = vec4<f32>(0.0);
    let bl = lil.p[P_BACKLIGHT];
    if (feat(F_BACKLIGHT) && bl.x > 0.5) {
        let blp = lil.p[P_BACKLIGHT_P];
        let bl_n = mix(orig_n, n, blp.x);
        let bst = lil.p[P_BACKLIGHT_ST];
        backlight_color = lil.p[P_BACKLIGHT_COLOR] * slot_grad(SLOT_BACKLIGHT, uv_main * bst.xy + bst.zw, gx * bst.xy, gy * bst.xy);
        let hl = dot(view, to_light);
        let factor = pow(saturate(-hl * 0.5 + 0.5), blp.w);
        var bl_ln = dot(normalize(-view * lil.p[P_BACKLIGHT_Q].x + to_light), bl_n) * 0.5 + 0.5;
        if (bl.z > 0.5) {
            bl_ln = bl_ln * calculated;
        }
        bl_ln = saturate(lil_tooning_ns(aa_main, bl_ln, blp.y, blp.z, fwidth(bl_ln)));
        backlight = saturate(factor * bl_ln);
        if (facing < bl.w - 1.0) {
            backlight = 0.0;
        }
    }
    // 光沢（滑らかさ・金属度・光沢の項。導関数は捨てる前に）
    var smoothness = 1.0;
    var roughness = 1.0;
    var metallic = 0.0;
    var spec_term = 0.0;
    var spec_fresnel_lh = 1.0;
    let rf = lil.p[P_REFL];
    let rfp = lil.p[P_REFL_P];
    let rfq = lil.p[P_REFL_Q];
    if (feat(F_REFLECTION) && rf.x > 0.5) {
        let sst = lil.p[P_SMOOTH_ST];
        smoothness = rf.y * slot_grad(SLOT_SMOOTHNESS, uv_main * sst.xy + sst.zw, gx * sst.xy, gy * sst.xy).r;
        // GSAA
        let dnx = abs(dpdx(n));
        let dny = abs(dpdy(n));
        let dxy = max(dot(dnx, dnx), dot(dny, dny));
        let gsaa = dxy / (dxy * 5.0 + 0.002) * lil.p[P_REFL_R].y;
        smoothness = min(smoothness, saturate(1.0 - gsaa));
        perceptual_roughness = perceptual_roughness - smoothness * perceptual_roughness;
        roughness = perceptual_roughness * perceptual_roughness;
        let mst = lil.p[P_METAL_ST];
        metallic = rf.z * slot_grad(SLOT_METALLIC, uv_main * mst.xy + mst.zw, gx * mst.xy, gy * mst.xy).r;
        // lilCalcSpecular（主な光。トゥーンと GGX、異方性反射）
        let sn = mix(orig_n, n, rfp.z);
        let h = normalize(view + to_light);
        let nh = saturate(dot(sn, h));
        if (rfp.y > 0.5 && !aniso_spec) {
            let v = pow(nh, 1.0 / roughness);
            spec_term = saturate(lil_tooning_ns(aa_main, v, rfp.w, rfq.x, fwidth(v)));
            spec_fresnel_lh = -1.0;
        } else {
            let snv = saturate(dot(sn, view));
            let snl = saturate(dot(sn, to_light));
            let lh = saturate(dot(to_light, h));
            var ggx = 0.0;
            var lambda_v = 0.0;
            var lambda_l = 0.0;
            if (aniso_spec) {
                let rt = max(roughness * (1.0 + anisotropy), 0.002);
                let rb = max(roughness * (1.0 - anisotropy), 0.002);
                let tv = dot(tangent, view);
                let bv = dot(bitangent, view);
                let tl = dot(tangent, to_light);
                let bl2 = dot(bitangent, to_light);
                lambda_v = snl * length(vec3<f32>(rt * tv, rb * bv, snv));
                lambda_l = snv * length(vec3<f32>(rt * tl, rb * bl2, snl));
                let a1 = lil.p[P_ANISO1];
                let a2 = lil.p[P_ANISO2];
                let rt1 = rt * a1.x;
                let rb1 = rb * a1.y;
                let rt2 = rt * a2.x;
                let rb2 = rb * a2.y;
                let nst = lil.p[P_ANISO_NOISE_ST];
                let noise = slot_grad(SLOT_ANISO_NOISE, uv_main * nst.xy + nst.zw, gx * nst.xy, gy * nst.xy).r - 0.5;
                let shift1 = noise * a1.w + a1.z;
                let shift2 = noise * a2.w + a2.z;
                let t1 = normalize(tangent - sn * shift1);
                let b1 = normalize(bitangent - sn * shift1);
                let t2 = normalize(tangent - sn * shift2);
                let b2 = normalize(bitangent - sn * shift2);
                let r1 = rt1 * rb1;
                let r2 = rt2 * rb2;
                let v1 = vec3<f32>(dot(t1, h) * rb1, dot(b1, h) * rt1, nh * r1);
                let v2 = vec3<f32>(dot(t2, h) * rb2, dot(b2, h) * rt2, nh * r2);
                let w1 = r1 / dot(v1, v1);
                let w2 = r2 / dot(v2, v2);
                let s = lil.p[P_ANISO_S];
                ggx = r1 * w1 * w1 * s.x + r2 * w2 * w2 * s.y;
            } else {
                let r2v = max(roughness, 0.002);
                lambda_v = snl * (snv * (1.0 - r2v) + r2v);
                lambda_l = snv * (snl * (1.0 - r2v) + r2v);
                let rr2 = r2v * r2v;
                let d = (nh * rr2 - nh) * nh + 1.0;
                ggx = rr2 / (d * d + 1e-7);
            }
            let sjggx = 0.5 / (lambda_v + lambda_l + 1e-5);
            spec_term = sjggx * ggx * snl;
            if (aniso_spec && rfp.y > 0.5) {
                spec_term = lil_tooning_step(aa_main, spec_term, 0.5, fwidth(spec_term));
                spec_fresnel_lh = -1.0;
            } else {
                spec_fresnel_lh = lh;
            }
        }
    }
    // ラメ（導関数は捨てる前に）
    var glitter = vec3<f32>(0.0);
    let gl = lil.p[P_GLITTER];
    let glq = lil.p[P_GLITTER_Q];
    if (feat(F_GLITTER) && gl.x > 0.5) {
        let gl_n = mix(orig_n, n, glq.w);
        let glr = lil.p[P_GLITTER_R];
        glitter = lil_glitter(uv0, gl_n, view, u.lil_camera_front.xyz, to_light, lil.p[P_GLITTER_P1], lil.p[P_GLITTER_P2], glr.x, glr.y, glr.z);
    }
    // リムライト（ライト方向あり。導関数は捨てる前に）
    let rim = lil.p[P_RIM];
    let rp = lil.p[P_RIM_P];
    let rq = lil.p[P_RIM_Q];
    let rr = lil.p[P_RIM_R];
    let rim_n = mix(orig_n, n, rim.w);
    let rim_nv = abs(dot(rim_n, view));
    let ln_raw = dot(to_light, rim_n) * 0.5 + 0.5;
    let ln_dir = saturate((ln_raw + rr.x) / (1.0 + rr.x));
    let ln_indir = saturate((1.0 - ln_raw + rr.y) / (1.0 + rr.y));
    var rim_f = pow(saturate(1.0 - rim_nv), rp.z);
    if (facing < rq.y - 1.0) {
        rim_f = 0.0;
    }
    let rim_dir_in = mix(rim_f, rim_f * ln_dir, rq.w);
    let rim_ind_in = rim_f * ln_indir * rq.w;
    let fw_rd = fwidth(rim_dir_in);
    let fw_ri = fwidth(rim_ind_in);
    let uv_rim = vec2<f32>(nvabs, nvabs);
    let gx_rim = dpdx(uv_rim);
    let gy_rim = dpdy(uv_rim);
    // アルファマスク（不透明の描画モードでは使わない）
    if (feat(F_ALPHA_MASK)) {
        let ast = lil.p[P_ALPHA_MASK_ST];
        let am_tex = slot_grad(SLOT_ALPHA_MASK, uv_main * ast.xy + ast.zw, gx * ast.xy, gy * ast.xy).r;
        col.a = apply_alpha_mask(col.a, am_tex, mode);
    }
    let fw_alpha = fwidth(col.a);
    let cutoff = lil.p[P_MODE].y;
    if (mode == 0) {
        col.a = 1.0;
    } else if (mode == 1) {
        col.a = saturate((col.a - cutoff) / max(fw_alpha, 0.0001) + 0.5);
        if (col.a == 0.0) {
            discard;
        }
    } else if (col.a - cutoff < 0.0) {
        discard;
    }
    let albedo = col.rgb;

    // 裏面を影に
    let bfshadow = select(1.0, 1.0 - lil.p[P_LIGHT2].z, facing < 0.0);
    lns.x = lns.x * bfshadow;
    lns.y = lns.y * bfshadow;
    lns.w = lns.w * bfshadow;
    lns.z = lns.z * bfshadow;
    if (feat(F_SHADOW) && sp.x > 0.5) {
        shadowmix = lns.x;
        var strength = sp.y;
        if (mask_type == 1) {
            // 平面（物の前 (0, 0.25, 1) の向き）
            let flat_n = normalize(vec3<f32>(0.0, 0.25, 1.0));
            var ln_flat = saturate((dot(flat_n, to_light) + flat.x) / flat.y);
            ln_flat = ln_flat * mix(1.0, calculated, s1.w);
            lns = mix(vec4<f32>(ln_flat), lns, ssm.r);
        } else {
            strength = strength * ssm.r;
        }
        lns.x = mix(1.0, lns.x, strength);
        // 影色（LUT は描かない: 通常の影色テクスチャとして読む）
        var indirect = mix(albedo, shadow1_tex.rgb, shadow1_tex.a) * lil.p[P_SHADOW1_COLOR].rgb;
        let c2 = lil.p[P_SHADOW2_COLOR];
        let shadow2 = mix(albedo, shadow2_tex_in.rgb, shadow2_tex_in.a) * c2.rgb;
        lns.y = c2.a - lns.y * c2.a;
        indirect = mix(indirect, shadow2, lns.y);
        let c3 = lil.p[P_SHADOW3_COLOR];
        let shadow3 = mix(albedo, shadow3_tex_in.rgb, shadow3_tex_in.a) * c3.rgb;
        lns.z = c3.a - lns.z * c3.a;
        indirect = mix(indirect, shadow3, lns.z);
        indirect = mix(indirect, indirect * albedo, flat.w);
        let direct = albedo * light_color;
        indirect = indirect * light_color;
        indirect = mix(indirect, albedo, sat3(ind_light * lil.p[P_LIGHT2].x));
        indirect = min(indirect, direct);
        indirect = mix(indirect, direct, lns.w * lil.p[P_SHADOW_BORDER_COLOR].rgb);
        col = vec4<f32>(mix(indirect, direct, lns.x), col.a);
    } else {
        col = vec4<f32>(col.rgb * light_color, col.a);
    }
    let max_limit = lil.p[P_LIGHT].y;
    light_color = min(light_color, vec3<f32>(max_limit));
    shadowmix = saturate(shadowmix);
    col = vec4<f32>(min(col.rgb, albedo * max_limit), col.a);

    // メインカラー 2nd・3rd の光の後の分（ライトの明るさを反映しない分）
    if (feat(F_MAIN2) && l2.x > 0.5) {
        col = vec4<f32>(lil_blend(col.rgb, layer2.rgb, vec3<f32>(layer2.a - layer2.a * l2.y), i32(l2.z + 0.5)), col.a);
    }
    if (feat(F_MAIN3) && l3.x > 0.5) {
        col = vec4<f32>(lil_blend(col.rgb, layer3.rgb, vec3<f32>(layer3.a - layer3.a * l3.y), i32(l3.z + 0.5)), col.a);
    }

    // リムシェード
    if (feat(F_RIM_SHADE) && rs.x > 0.5) {
        col = vec4<f32>(mix(col.rgb, col.rgb * lil.p[P_RIM_SHADE_COLOR].rgb, rim_shade), col.a);
    }
    // 逆光ライト
    if (feat(F_BACKLIGHT) && bl.x > 0.5) {
        let bc = vec4<f32>(mix(backlight_color.rgb, backlight_color.rgb * albedo, bl.y), backlight_color.a);
        col = vec4<f32>(col.rgb + backlight * bc.a * bc.rgb * light_color, col.a);
    }

    // 乗算済み（半透明）
    if (transparent) {
        col = vec4<f32>(col.rgb * col.a, col.a);
    }

    // 光沢（光沢の項と環境光の反射）
    if (feat(F_REFLECTION) && rf.x > 0.5) {
        let rfr = lil.p[P_REFL_R];
        let blend_mode = i32(rfr.x + 0.5);
        col = vec4<f32>(col.rgb - metallic * col.rgb, col.a);
        let specular = mix(vec3<f32>(rf.w), albedo, metallic);
        let cst = lil.p[P_REFL_COLOR_ST];
        var rcolor = lil.p[P_REFL_COLOR] * slot_grad(SLOT_REFL_COLOR, uv_main * cst.xy + cst.zw, gx * cst.xy, gy * cst.xy);
        if (transparent && rfq.w > 0.5) {
            rcolor.a = rcolor.a * col.a;
        }
        if (rfp.x > 0.5) {
            var refl = vec3<f32>(spec_term);
            if (spec_fresnel_lh >= 0.0) {
                // lilFresnelTerm
                let a = 1.0 - spec_fresnel_lh;
                refl = spec_term * (specular + (vec3<f32>(1.0) - specular) * (a * a * a * a * a));
            }
            col = vec4<f32>(lil_blend(col.rgb, rcolor.rgb * light_color, refl * rcolor.a, blend_mode), col.a);
        }
        if (rfq.y > 0.5) {
            let rn = mix(orig_n, reflection_n, rfq.z);
            // 環境光の反射（3D ビューの環境。Unity の反射プローブと同じ mip の選び方）。環境なしは一様な環境光
            var env = u.ambient.rgb;
            if (u.env.x > 0.5) {
                let mip = perceptual_roughness * (1.7 - 0.7 * perceptual_roughness) * 6.0;
                env = textureSampleLevel(env_cube, env_sampler, to_source(reflect(-view, rn)), mip).rgb * u.env.y;
            }
            let one_minus_reflectivity = (1.0 - DIELECTRIC) - metallic * (1.0 - DIELECTRIC);
            let grazing = saturate(smoothness + (1.0 - one_minus_reflectivity));
            let surface_reduction = 1.0 / (roughness * roughness + 1.0);
            let a = 1.0 - nv;
            let fresnel = mix(specular, vec3<f32>(grazing), a * a * a * a * a);
            let refl = surface_reduction * env * fresnel;
            col = vec4<f32>(lil_blend(col.rgb, rcolor.rgb, refl * rcolor.a, blend_mode), col.a);
        }
    }

    // マットキャップ
    let mc = lil.p[P_MATCAP];
    let mcb = lil.p[P_MC_BUMP];
    if (feat(F_MATCAP) && mc.x > 0.5) {
        let mcp = lil.p[P_MATCAP_P];
        let mcq = lil.p[P_MATCAP_Q];
        var mc_n = mix(orig_n, matcap_n, mcq.x);
        if (mcb.x > 0.5 && has_tangents) {
            let bst = lil.p[P_MC_BUMP_ST];
            let t = slot_grad(SLOT_MATCAP_BUMP, uv_main * bst.xy + bst.zw, gx * bst.xy, gy * bst.xy);
            let nm = lil_unpack_normal(t, mcb.y, slot_received(SLOT_MATCAP_BUMP));
            mc_n = normalize(f.tangent * nm.x + f.bitangent * nm.y + geometric * nm.z);
            if (facing < flip - 1.0) {
                mc_n = -mc_n;
            }
        }
        let mc_uv = matcap_uv(normalize(mc_n), view, lil.p[P_MATCAP_ST], mcq.y > 0.5, mcq.z > 0.5);
        let mc_tex = slot_level(SLOT_MATCAP, mc_uv, mcp.w);
        let mc_mask_st = lil.p[P_MATCAP_MASK_ST];
        let mc_mask = slot_grad(SLOT_MATCAP_MASK, uv_main * mc_mask_st.xy + mc_mask_st.zw, gx * mc_mask_st.xy, gy * mc_mask_st.xy).rgb;
        var c = lil.p[P_MATCAP_COLOR] * mc_tex;
        c = vec4<f32>(mix(c.rgb, c.rgb * light_color, mcp.x), mix(c.a, c.a * shadowmix, mcp.y));
        if (transparent && mcq.w > 0.5) {
            c.a = c.a * col.a;
        }
        if (facing < mcp.z - 1.0) {
            c.a = 0.0;
        }
        c = vec4<f32>(mix(c.rgb, c.rgb * albedo, mc.w), c.a);
        col = vec4<f32>(lil_blend(col.rgb, c.rgb, mc.y * c.a * mc_mask, i32(mc.z + 0.5)), col.a);
    }
    let mc2 = lil.p[P_MATCAP2];
    if (feat(F_MATCAP2) && mc2.x > 0.5) {
        let mc2p = lil.p[P_MATCAP2_P];
        let mc2q = lil.p[P_MATCAP2_Q];
        // 2nd は lilToon と同じく、混ぜた法線を正規化しないで UV を出す
        var mc2_n = mix(orig_n, matcap2_n, mc2q.x);
        if (mcb.z > 0.5 && has_tangents) {
            let bst = lil.p[P_MC2_BUMP_ST];
            let t = slot_grad(SLOT_MATCAP2_BUMP, uv_main * bst.xy + bst.zw, gx * bst.xy, gy * bst.xy);
            let nm = lil_unpack_normal(t, mcb.w, slot_received(SLOT_MATCAP2_BUMP));
            mc2_n = normalize(f.tangent * nm.x + f.bitangent * nm.y + geometric * nm.z);
            if (facing < flip - 1.0) {
                mc2_n = -mc2_n;
            }
        }
        let mc2_uv = matcap_uv(mc2_n, view, lil.p[P_MATCAP2_ST], mc2q.y > 0.5, mc2q.z > 0.5);
        let mc2_tex = slot_level(SLOT_MATCAP2, mc2_uv, mc2p.w);
        let mc2_mask_st = lil.p[P_MATCAP2_MASK_ST];
        let mc2_mask = slot_grad(SLOT_MATCAP2_MASK, uv_main * mc2_mask_st.xy + mc2_mask_st.zw, gx * mc2_mask_st.xy, gy * mc2_mask_st.xy).rgb;
        var c = lil.p[P_MATCAP2_COLOR] * mc2_tex;
        c = vec4<f32>(mix(c.rgb, c.rgb * light_color, mc2p.x), mix(c.a, c.a * shadowmix, mc2p.y));
        if (transparent && mc2q.w > 0.5) {
            c.a = c.a * col.a;
        }
        if (facing < mc2p.z - 1.0) {
            c.a = 0.0;
        }
        c = vec4<f32>(mix(c.rgb, c.rgb * albedo, mc2.w), c.a);
        col = vec4<f32>(lil_blend(col.rgb, c.rgb, mc2.y * c.a * mc2_mask, i32(mc2.z + 0.5)), col.a);
    }

    // リムライト（ライト方向あり。値は上で取った）
    if (feat(F_RIM) && rim.x > 0.5) {
        let rim_st = lil.p[P_RIM_ST];
        let rim_tex = slot_grad(SLOT_RIM, uv_main * rim_st.xy + rim_st.zw, gx * rim_st.xy, gy * rim_st.xy);
        var rim_color = lil.p[P_RIM_COLOR] * rim_tex;
        let rim_indir_color = lil.p[P_RIM_INDIR_COLOR] * rim_tex;
        rim_color = vec4<f32>(mix(rim_color.rgb, rim_color.rgb * albedo, rim.z), rim_color.a);
        var rim_dir = saturate(lil_tooning_ns(aa_main, rim_dir_in, rp.x, rp.y, fw_rd));
        var rim_ind = saturate(lil_tooning_ns(aa_main, rim_ind_in, rr.z, rr.w, fw_ri));
        rim_dir = mix(rim_dir, rim_dir * shadowmix, rq.x);
        rim_ind = mix(rim_ind, rim_ind * shadowmix, rq.x);
        if (transparent && rq.z > 0.5) {
            rim_dir = rim_dir * col.a;
            rim_ind = rim_ind * col.a;
        }
        let rim_mul = vec3<f32>(1.0 - rp.w) + light_color * rp.w;
        let blend_mode = i32(rim.y + 0.5);
        col = vec4<f32>(lil_blend(col.rgb, rim_color.rgb * rim_mul, vec3<f32>(rim_dir * rim_color.a), blend_mode), col.a);
        col = vec4<f32>(lil_blend(col.rgb, rim_indir_color.rgb * rim_mul, vec3<f32>(rim_ind * rim_indir_color.a), blend_mode), col.a);
    }

    // ラメ
    if (feat(F_GLITTER) && gl.x > 0.5) {
        let gst = lil.p[P_GLITTER_ST];
        var gc = lil.p[P_GLITTER_COLOR] * slot_grad(SLOT_GLITTER, uv_main * gst.xy + gst.zw, gx * gst.xy, gy * gst.xy);
        gc = vec4<f32>(gc.rgb * glitter, gc.a);
        gc = vec4<f32>(mix(gc.rgb, gc.rgb * albedo, gl.z), gc.a);
        if (transparent && glq.z > 0.5) {
            gc.a = gc.a * col.a;
        }
        if (facing < glq.y - 1.0) {
            gc.a = 0.0;
        }
        gc.a = mix(gc.a, gc.a * shadowmix, glq.x);
        gc = vec4<f32>(mix(gc.rgb, gc.rgb * light_color, gl.w), gc.a);
        col = vec4<f32>(col.rgb + gc.rgb * gc.a, col.a);
    }

    // 発光
    let em = lil.p[P_EMISSION];
    if (feat(F_EMISSION) && em.x > 0.5) {
        let emx = lil.p[P_EMISSION_X];
        let em_st = lil.p[P_EMISSION_ST];
        let em_rim = i32(emx.y + 0.5) == 4;
        let em_uv = rotate_uv(select(uv0, uv_rim, em_rim) * em_st.xy + em_st.zw, emx.z);
        let em_tex = slot_grad(SLOT_EMISSION, em_uv, select(gx0, gx_rim, em_rim) * em_st.xy, select(gy0, gy_rim, em_rim) * em_st.xy);
        let em_mask_st = lil.p[P_EMISSION_MASK_ST];
        let em_mask = slot_grad(SLOT_EMISSION_MASK, rotate_uv(uv0 * em_mask_st.xy + em_mask_st.zw, emx.w), gx0 * em_mask_st.xy, gy0 * em_mask_st.xy);
        var c = lil.p[P_EMISSION_COLOR] * em_tex * em_mask;
        c = vec4<f32>(mix(c.rgb, c.rgb * inv_lighting, emx.x), c.a);
        c = vec4<f32>(mix(c.rgb, c.rgb * albedo, em.w), c.a);
        var b = em.y * c.a;
        if (transparent) {
            b = b * col.a;
        }
        col = vec4<f32>(lil_blend(col.rgb, c.rgb, vec3<f32>(b), i32(em.z + 0.5)), col.a);
    }
    let em2 = lil.p[P_EMISSION2];
    if (feat(F_EMISSION2) && em2.x > 0.5) {
        let em2x = lil.p[P_EMISSION2_X];
        let em2_st = lil.p[P_EMISSION2_ST];
        let em2_rim = i32(em2x.y + 0.5) == 4;
        let em2_uv = rotate_uv(select(uv0, uv_rim, em2_rim) * em2_st.xy + em2_st.zw, em2x.z);
        let em2_tex = slot_grad(SLOT_EMISSION2, em2_uv, select(gx0, gx_rim, em2_rim) * em2_st.xy, select(gy0, gy_rim, em2_rim) * em2_st.xy);
        let em2_mask_st = lil.p[P_EMISSION2_MASK_ST];
        let em2_mask = slot_grad(SLOT_EMISSION2_MASK, rotate_uv(uv0 * em2_mask_st.xy + em2_mask_st.zw, em2x.w), gx0 * em2_mask_st.xy, gy0 * em2_mask_st.xy);
        var c = lil.p[P_EMISSION2_COLOR] * em2_tex * em2_mask;
        c = vec4<f32>(mix(c.rgb, c.rgb * inv_lighting, em2x.x), c.a);
        c = vec4<f32>(mix(c.rgb, c.rgb * albedo, em2.w), c.a);
        var b = em2.y * c.a;
        if (transparent) {
            b = b * col.a;
        }
        col = vec4<f32>(lil_blend(col.rgb, c.rgb, vec3<f32>(b), i32(em2.z + 0.5)), col.a);
    }

    // 裏面の色
    if (feat(F_BACKFACE)) {
        let back = lil.p[P_BACKFACE];
        if (facing < 0.0) {
            col = vec4<f32>(mix(col.rgb, back.rgb * light_color, back.a), col.a);
        }
    }

    // 距離フェード
    if (feat(F_DISTANCE_FADE)) {
        col = distance_fade(col, depth, orig_n, view, facing, transparent, false);
    }
    if (!transparent) {
        col.a = 1.0;
    }
    return col;
}

