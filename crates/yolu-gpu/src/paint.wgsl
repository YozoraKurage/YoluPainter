// yolu-core::blend・normal・adjust と brush の式。各レイヤーで半段切り上げの RGBA8 に戻す。
// 先頭に Rust が定数（OP_*・ADJ_*・LUT_WORDS・STACK_SLOTS・BLEND_OVERLAY）を書き足す（plan.rs の `shader_source`）。
// extra: ブラシではダブの数（合成では使わない。法線の種類のチャンネルは、重ねる式を単位ベクトルの式にしたシェーダーの形で選ぶ）。
struct Params { count: u32, layers: u32, size: u32, extra: u32 }
// 合成の 1 つの命令（plan.rs の命令の並び）。slot は source の面の番号（NONE なら塗りつぶしの色 fill）、mask はマスクの面の番号
// （NONE ならマスク無し。アルファが隠す量）。描かないレイヤーは並びに入れない。調整の命令は面も塗りつぶしの色も持たないので、slot に調整の式の
// 種類、fill に値・表の語の番号（tables の中）を置く。
struct Layer { kind: u32, opacity: f32, mode: u32, slot: u32, mask: u32, fill: u32, mask_invert: u32, mask_density: f32 }
const NONE: u32 = 0xffffffffu;
struct Dab { x: f32, y: f32, radius: f32, hardness: f32, ceiling: f32, flow: f32, color: u32, erase: u32 }
@group(0) @binding(0) var<storage, read> source: array<u32>;
@group(0) @binding(1) var<storage, read> layers: array<Layer>;
@group(0) @binding(2) var<storage, read_write> output_pixels: array<u32>;
@group(0) @binding(3) var<uniform> p: Params;
@group(0) @binding(4) var<storage, read> dabs: array<Dab>;
// 調整の表と値（成分ごとの 256 バイトを 4 つずつ詰めた語・輝度 → 色の語・f32 のビットの値）。
@group(0) @binding(7) var<storage, read> tables: array<u32>;
// 面ごと・束のタイルごとに 1 語（0 はその面のそのタイルに画素が無い。無いタイルは作業域を埋めず、読まない）。
@group(0) @binding(8) var<storage, read> presence: array<u32>;
// タイルごとの命令の番号の列（先頭にタイルごとの（始まり, 長さ）。plan.rs の `tile_program`。画素の無いレイヤーの命令を落とした列）。
@group(0) @binding(9) var<storage, read> programs: array<u32>;
fn unpack(v: u32) -> vec4f { return vec4f(f32(v & 255u), f32((v >> 8u)&255u), f32((v >> 16u)&255u), f32(v >> 24u))/255.0; }
fn pack(c: vec4f) -> u32 { let b = vec4u(floor(clamp(c, vec4f(0), vec4f(1))*255.0+0.5)); return b.x | (b.y<<8u) | (b.z<<16u) | (b.w<<24u); }
fn quant(c: vec4f) -> vec4f { return unpack(pack(c)); }
fn dodge(d:f32,s:f32)->f32 { if d<=0 {return 0;} if s>=1 {return 1;} return min(1,d/(1-s)); }
fn burn(d:f32,s:f32)->f32 { if d>=1 {return 1;} if s<=0 {return 0;} return 1-min(1,(1-d)/s); }
fn sep(m:u32,d:f32,s:f32)->f32 {
 var v=s;
 switch m {
 case 1u: {v=d*s;} case 2u: {v=d+s-d*s;}
 case 3u: {if d<=0.5 {v=2*d*s;} else {v=1-2*(1-d)*(1-s);}}
 case 4u: {v=min(d,s);} case 5u: {v=max(d,s);}
 case 6u: {v=dodge(d,s);} case 7u: {v=burn(d,s);}
 case 8u: {v=d+s;} case 9u: {v=d+s-1;}
 case 10u: {if s<=0.5 {v=2*d*s;} else {v=1-2*(1-d)*(1-s);}}
 case 11u: {if s<=0.5 {v=d-(1-2*s)*d*(1-d);} else {v=d+(2*s-1)*(sqrt(d)-d);}}
 case 12u: {if s<=0.5 {v=burn(d,2*s);} else {v=dodge(d,2*s-1);}}
 case 13u: {v=d+2*s-1;} case 14u: {if s<=0.5 {v=min(d,2*s);} else {v=max(d,2*s-1);}}
 case 15u: {v=select(0.0,1.0,d+s>=1-0.5/255.0);}
 case 16u: {v=abs(d-s);} case 17u: {v=d+s-2*d*s;} case 18u: {v=d-s;}
 case 19u: {if s<=0 {v=select(1.0,0.0,d<=0);} else {v=d/s;}}
 default: {}
 } return clamp(v,0,1);
}
fn lum(c:vec3f)->f32 {return 0.3*c.r+0.59*c.g+0.11*c.b;}
fn sat(c:vec3f)->f32 {return max(c.r,max(c.g,c.b))-min(c.r,min(c.g,c.b));}
fn setlum(c:vec3f,l:f32)->vec3f {
 var v=c+(l-lum(c)); let lm=lum(v); let n=min(v.r,min(v.g,v.b)); let x=max(v.r,max(v.g,v.b));
 if n<0 && lm-n>1e-12 {v=lm+(v-lm)*lm/(lm-n);}
 if x>1 && x-lm>1e-12 {v=lm+(v-lm)*(1-lm)/(x-lm);}
 return clamp(v,vec3f(0),vec3f(1));
}
fn setsat(c:vec3f,s:f32)->vec3f {
 let mx=max(c.r,max(c.g,c.b)); let mn=min(c.r,min(c.g,c.b));
 if mx-mn<=1e-12 {return vec3f(0);}
 return select(select((c-mn)*s/(mx-mn),vec3f(0),c==vec3f(mn)),vec3f(s),c==vec3f(mx));
}
fn rgb(m:u32,d:vec3f,s:vec3f)->vec3f {
 switch m {
 case 20u: {return setlum(setsat(s,sat(d)),lum(d));}
 case 21u: {return setlum(setsat(d,sat(s)),lum(d));}
 case 22u: {return setlum(s,lum(d));} case 23u: {return setlum(d,lum(s));}
 case 24u: {return select(d,s,s.r+s.g+s.b<d.r+d.g+d.b-0.5/255.0);}
 case 25u: {return select(d,s,s.r+s.g+s.b>d.r+d.g+d.b+0.5/255.0);}
 default: {return vec3f(sep(m,d.r,s.r),sep(m,d.g,s.g),sep(m,d.b,s.b));}
 }
}
fn blend(d:vec4f,s:vec4f,o:f32,m:u32)->vec4f {
 let sa=s.a*o; if sa<=0 {return d;}
 if d.a==0 {return quant(vec4f(s.rgb,sa));}
 if m==0u && sa==1 {return s;}
 let a=sa+d.a*(1-sa); let b=rgb(m,d.rgb,s.rgb);
 return quant(vec4f(((1-sa)*d.a*d.rgb+(1-d.a)*sa*s.rgb+d.a*sa*b)/a,a));
}
// @begin normal
// ───────── 法線の種類のチャンネル（core の normal.rs。バイトは c / 255 × 2 − 1 と読み、使う前に必ず正規化する） ─────────
fn n_norm(v: vec3f) -> vec3f { let l2=dot(v,v); if l2<1e-12 {return vec3f(0,0,1);} return v/sqrt(l2); }
fn n_dec(c: vec4f) -> vec3f { return n_norm(c.rgb*2.0-1.0); }
fn n_enc(v: vec3f, a: f32) -> vec4f { return quant(vec4f(n_norm(v)*0.5+0.5,a)); }
// Reoriented Normal Mapping: 細部 d を土台 b の向きへ回す。真内向き（z = −1）の土台は座標系を持たないので土台のまま。
fn n_rnm(b: vec3f, d: vec3f) -> vec3f {
 let t=vec3f(b.x,b.y,b.z+1.0); let u=vec3f(-d.x,-d.y,d.z);
 if t.z<=1e-6 {return b;}
 let k=dot(t,u)/t.z;
 return t*k-u;
}
fn n_combine(m: u32, b: vec3f, s: vec3f) -> vec3f { if m==BLEND_OVERLAY {return n_rnm(b,s);} return s; }
fn blend_n(d:vec4f,s:vec4f,o:f32,m:u32)->vec4f {
 let t=s.a*o; let da=d.a;
 if t<=0 {return d;}
 let b=n_dec(d); let sv=n_dec(s); let c=n_combine(m,b,sv);
 let wb=(1-t)*da; let ws=(1-da)*t; let wc=da*t;
 return n_enc(wb*b+ws*sv+wc*c,t+da*(1-t));
}
fn clip_n(g:vec4f,c:vec4f,amount:f32,m:u32)->vec4f {
 let t=c.a*amount;
 if t<=0 || g.a==0 {return g;}
 let gv=n_dec(g); let sv=n_dec(c); let cc=n_combine(m,gv,sv);
 return n_enc((1-t)*gv+t*cc,g.a);
}
fn fade_n(backdrop:vec4f,inner:vec4f,amount:f32)->vec4f {
 if amount>=1 {return inner;} if amount<=0 {return backdrop;}
 let ba=backdrop.a*(1-amount); let ia=inner.a*amount; let a=ba+ia;
 if a<=0 {return vec4f(0);}
 return n_enc(ba*n_dec(backdrop)+ia*n_dec(inner),a);
}
fn op_blend(d:vec4f,s:vec4f,o:f32,m:u32)->vec4f { return blend_n(d,s,o,m); }
fn op_clip(g:vec4f,c:vec4f,amount:f32,m:u32)->vec4f { return clip_n(g,c,amount,m); }
fn op_fade(backdrop:vec4f,inner:vec4f,amount:f32)->vec4f { return fade_n(backdrop,inner,amount); }
// @end normal

// ───────── 色の式での、組への重ね・フェード ─────────
// クリッピングされたレイヤーの色を組（下地）へ重ねる。下地のアルファはそのまま（下地の外へは描かない）。
fn clip_c(g:vec4f,c:vec4f,amount:f32,m:u32)->vec4f {
 let t=c.a*amount;
 if t<=0 || g.a==0 {return g;}
 return quant(vec4f(g.rgb+(rgb(m,g.rgb,c.rgb)-g.rgb)*t,g.a));
}
// 通過のグループのフェード（プリマルチプライドの補間。透明な側がもう片方を暗くしない）。
fn fade_c(backdrop:vec4f,inner:vec4f,amount:f32)->vec4f {
 if amount>=1 {return inner;} if amount<=0 {return backdrop;}
 let ba=backdrop.a*(1-amount); let ia=inner.a*amount; let a=ba+ia;
 if a<=0 {return vec4f(0);}
 return quant(vec4f((backdrop.rgb*ba+inner.rgb*ia)/a,a));
}
// @begin color
fn op_blend(d:vec4f,s:vec4f,o:f32,m:u32)->vec4f { return blend(d,s,o,m); }
fn op_clip(g:vec4f,c:vec4f,amount:f32,m:u32)->vec4f { return clip_c(g,c,amount,m); }
fn op_fade(backdrop:vec4f,inner:vec4f,amount:f32)->vec4f { return fade_c(backdrop,inner,amount); }
// @end color

// ───────── 調整（core の adjust。画素は保存したままの RGB のバイトに式を当て、アルファは変えない） ─────────
// @begin adjust
fn quant3(v: vec3f) -> vec3f { return floor(clamp(v,vec3f(0),vec3f(1))*255.0+0.5)/255.0; }
fn table_byte(base: u32, v: u32) -> u32 { return (tables[base+(v>>2u)] >> ((v&3u)*8u)) & 255u; }
fn hue_to_rgb(p0:f32,q:f32,t0:f32)->f32 {
 var t=t0; if t<0 {t+=1;} if t>1 {t-=1;}
 if t<1.0/6.0 {return p0+(q-p0)*6*t;}
 if t<0.5 {return q;}
 if t<2.0/3.0 {return p0+(q-p0)*(2.0/3.0-t)*6;}
 return p0;
}
// 色相・彩度・明度（RGB → HSL → RGB）。hue は度を 360 で割った値。
fn hue_saturation(c: vec3f, hue: f32, saturation: f32, lightness: f32) -> vec3f {
 let mx=max(c.r,max(c.g,c.b)); let mn=min(c.r,min(c.g,c.b));
 var l=(mx+mn)/2; var h=0.0; var s=0.0; let d=mx-mn;
 if d>1e-12 {
 if l>0.5 {s=d/(2-mx-mn);} else {s=d/(mx+mn);}
 if mx==c.r {h=(c.g-c.b)/d+select(0.0,6.0,c.g<c.b);}
 else if mx==c.g {h=(c.b-c.r)/d+2;}
 else {h=(c.r-c.g)/d+4;}
 h/=6;
 }
 h+=hue; h-=floor(h);
 s=clamp(s*(1+saturation),0.0,1.0);
 if lightness>=0 {l=l+(1-l)*lightness;} else {l=l*(1+lightness);}
 if s<=0 {return vec3f(l);}
 let q=select(l+s-l*s,l*(1+s),l<0.5);
 let p0=2*l-q;
 return vec3f(hue_to_rgb(p0,q,h+1.0/3.0),hue_to_rgb(p0,q,h),hue_to_rgb(p0,q,h-1.0/3.0));
}
fn luma_unit(c: vec3f) -> f32 { return 0.2126*c.r+0.7152*c.g+0.0722*c.b; }
fn luma_byte(b: vec3u) -> u32 { return (2126u*b.x+7152u*b.y+722u*b.z+5000u)/10000u; }
fn table_f32(at: u32) -> f32 { return bitcast<f32>(tables[at]); }
// 調整した色（0〜1 のバイトの値。アルファは呼び手が元のまま持つ）。値・表は tables の o.fill から。
fn adjust_color(o: Layer, c: vec4f) -> vec3f {
 let b=vec3u(floor(c.rgb*255.0+0.5));
 let t0=o.fill;
 switch o.slot {
 case ADJ_INVERT: {return vec3f(vec3u(255u)-b)/255.0;}
 case ADJ_LUT3: {return vec3f(f32(table_byte(t0,b.x)),f32(table_byte(t0+LUT_WORDS,b.y)),f32(table_byte(t0+2u*LUT_WORDS,b.z)))/255.0;}
 case ADJ_HUE_SATURATION: {return quant3(hue_saturation(c.rgb,table_f32(t0),table_f32(t0+1u),table_f32(t0+2u)));}
 case ADJ_GRADIENT: {
 // 輝度 → ランプの色（A は元の色へ戻す割合）
 let t=tables[t0+luma_byte(b)];
 let m=vec3u(t&255u,(t>>8u)&255u,(t>>16u)&255u); let a=t>>24u; let inv=255u-a;
 return vec3f((b*inv+m*a+vec3u(127u))/vec3u(255u))/255.0;
 }
 case ADJ_BALANCE: {
 if table_f32(t0+10u)!=0.0 {return c.rgb;}
 let y=luma_unit(c.rgb);
 let w=vec3f((1-y)*(1-y),4*y*(1-y),y*y);
 let shadows=vec3f(table_f32(t0),table_f32(t0+1u),table_f32(t0+2u));
 let mids=vec3f(table_f32(t0+3u),table_f32(t0+4u),table_f32(t0+5u));
 let highs=vec3f(table_f32(t0+6u),table_f32(t0+7u),table_f32(t0+8u));
 var v=c.rgb+(shadows*w.x+mids*w.y+highs*w.z)/100.0*0.3;
 if table_f32(t0+9u)!=0.0 {v+=y-luma_unit(clamp(v,vec3f(0),vec3f(1)));}
 return quant3(v);
 }
 case ADJ_THRESHOLD: {return vec3f(select(0.0,1.0,luma_byte(b)>=tables[t0]));}
 case ADJ_POSTERIZE: {
 let n=tables[t0];
 let tier=min(b*n/vec3u(255u),vec3u(n-1u));
 return vec3f((tier*255u+vec3u((n-1u)/2u))/vec3u(n-1u))/255.0;
 }
 default: {return c.rgb;}
 }
}
// @end adjust

// ───────── 命令 ─────────
// レイヤーの画素（面の画素か塗りつぶしの色）。画素の無い面の命令は、タイルの命令の列が落とすので、ここでは有無を見ない。
fn layer_pixel(l: Layer, i: u32) -> vec4f {
 if l.slot == NONE {return unpack(l.fill);}
 return unpack(source[l.slot*p.count+i]);
}
// 不透明度 × マスクの値（core の RasterMask::factor と同じ式。隠す量 = マスクのアルファ。マスクのタイルが無ければ隠す量 0）。
fn layer_amount(l: Layer, i: u32, t: vec2u) -> f32 {
 if l.mask == NONE {return l.opacity;}
 var hide=0.0;
 if presence[l.mask*t.y+t.x]!=0u {hide=f32(source[l.mask*p.count+i]>>24u)/255.0;}
 var factor=1.0-l.mask_density*hide;
 if l.mask_invert!=0u {factor=1.0-l.mask_density*(1.0-hide);}
 return l.opacity*factor;
}
// @begin adjust
// 調整した色を下の色とモードで組み合わせ、量（不透明度 × マスク）で戻す。アルファは下のまま。完全に透明な画素はそのまま。
fn adjust_pixel(below: vec4f, o: Layer, i: u32, t: vec2u) -> vec4f {
 let amount=layer_amount(o,i,t);
 if amount<=0 || below.a==0 {return below;}
 let mixed=rgb(o.mode,below.rgb,adjust_color(o,below));
 return quant(vec4f(below.rgb+(mixed-below.rgb)*amount,below.a));
}
// @end adjust
fn evaluate(i:u32) -> vec4f {
 let area=p.size*p.size;
 let t=vec2u(i/area,p.count/area);   // 束の中のタイルの番号・束のタイルの数
 var result=vec4f(0);   // 今の段の結果
 var clipped=vec4f(0);    // クリッピングの組（下地とクリッピングのレイヤーを重ねた値）
 // @begin stack
 var stack: array<u32, STACK_SLOTS>;   // グループの入れ子で退避する値（バイトの値なので 1 語に詰められる）
 var top=0u;
 // @end stack
 var n=programs[2u*t.x]; let end=n+programs[2u*t.x+1u];
 loop {
 if n>=end {break;}
 let o=layers[programs[n]]; n+=1u;
 switch o.kind {
 case OP_LAYER: {
 let s=layer_pixel(o,i); let a=layer_amount(o,i,t);
 result=op_blend(result,s,a,o.mode);
 }
 case OP_LOAD: {clipped=layer_pixel(o,i);}
 case OP_CLIP: {
 let s=layer_pixel(o,i); let a=layer_amount(o,i,t);
 clipped=op_clip(clipped,s,a,o.mode);
 }
 case OP_BLEND: {
 let a=layer_amount(o,i,t);
 result=op_blend(result,clipped,a,o.mode);
 }
 // @begin stack
 case OP_PUSH_ISO: {stack[top]=pack(result); stack[top+1u]=pack(clipped); top+=2u; result=vec4f(0);}
 case OP_POP_BASE: {clipped=result; top-=2u; result=unpack(stack[top]);}
 case OP_POP_CLIP: {
 let over=result; top-=2u; result=unpack(stack[top]); clipped=unpack(stack[top+1u]);
 let a=layer_amount(o,i,t);
 clipped=op_clip(clipped,over,a,o.mode);
 }
 case OP_PUSH_PASS: {stack[top]=pack(result); top+=1u;}
 case OP_POP_PASS: {
 top-=1u; let backdrop=unpack(stack[top]); let a=layer_amount(o,i,t);
 result=op_fade(backdrop,result,a);
 }
 // @end stack
 // @begin adjust
 case OP_ADJUST: {result=adjust_pixel(result,o,i,t);}
 case OP_CLIP_ADJUST: {clipped=adjust_pixel(clipped,o,i,t);}
 // @end adjust
 default: {}
 }
 }
 return result;
}
@compute @workgroup_size(64)
fn composite(@builtin(global_invocation_id) id:vec3u) {
 if id.x<p.count {output_pixels[id.x]=pack(evaluate(id.x));}
}
@compute @workgroup_size(64)
fn brush(@builtin(global_invocation_id) id:vec3u) {
 let i=id.x; if i>=p.count {return;}
 let start=unpack(source[i]); var result=start; var wash=0.0;
 let xy=vec2f(f32(i%p.size)+0.5,f32(i/p.size)+0.5);
 for(var k=0u;k<p.extra;k+=1u) {
 let d=dabs[k]; if d.radius<=0 {continue;}
 let distance=length(xy-vec2f(d.x,d.y))/d.radius;
 if distance>1 || d.ceiling<=0 || d.flow<=0 {continue;}
 var coverage=1.0;
 if distance>d.hardness {let t=(1-distance)/(1-d.hardness); coverage=t*t*(3-2*t);}
 let flow=coverage*d.flow; if flow<=0 {continue;}
 if wash>=d.ceiling {continue;}
 wash=wash+(d.ceiling-wash)*min(1,flow);
 let color=unpack(d.color);
 if d.erase!=0u {result=quant(vec4f(start.rgb,start.a*(1-wash*color.a))); if result.a==0 {result=vec4f(0);}}
 else {result=blend(start,color,min(1,wash),0u);}
 }
 output_pixels[i]=pack(result);
}

// 表示先は読み取らず、各呼び出しが担当する画素だけを書く。
@group(0) @binding(5) var display_image: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(6) var<storage, read> tile_coords: array<vec2u>;
@compute @workgroup_size(64)
fn display(@builtin(global_invocation_id) id:vec3u) {
 let i=id.x; if i>=p.count {return;}
 let area=p.size*p.size; let local=i%area;
 let xy=tile_coords[i/area]*p.size+vec2u(local%p.size,local/p.size);
 if all(xy<textureDimensions(display_image)) {textureStore(display_image,xy,evaluate(i));}
}

// 乗算済みの表示（egui のテクスチャと同じ）。Color32::from_rgba_unmultiplied と同じ整数の式で、CPU の表示とバイトまで一致する。
fn premultiply_byte(v: u32, a: u32) -> u32 { let q = v*a + 128u; return (q + (q >> 8u)) >> 8u; }
@compute @workgroup_size(64)
fn display_premultiplied(@builtin(global_invocation_id) id:vec3u) {
 let i=id.x; if i>=p.count {return;}
 let area=p.size*p.size; let local=i%area;
 let xy=tile_coords[i/area]*p.size+vec2u(local%p.size,local/p.size);
 if all(xy<textureDimensions(display_image)) {
 let c=pack(evaluate(i));
 let a=c>>24u;
 var rgba=vec4u(c&255u,(c>>8u)&255u,(c>>16u)&255u,a);
 if a==0u {rgba=vec4u(0u);}
 else if a<255u {rgba=vec4u(premultiply_byte(rgba.x,a),premultiply_byte(rgba.y,a),premultiply_byte(rgba.z,a),a);}
 textureStore(display_image,xy,vec4f(rgba)/255.0);
 }
}
