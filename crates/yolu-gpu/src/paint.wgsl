// yolu-core::blend と brush の式。各層で半段切り上げの RGBA8 に戻す。
struct Params { count: u32, layers: u32, size: u32, dabs: u32 }
struct Layer { opacity: f32, mode: u32, clip: u32, enabled: u32 }
struct Dab { x: f32, y: f32, radius: f32, hardness: f32, ceiling: f32, flow: f32, color: u32, erase: u32 }
@group(0) @binding(0) var<storage, read> source: array<u32>;
@group(0) @binding(1) var<storage, read> layers: array<Layer>;
@group(0) @binding(2) var<storage, read_write> output_pixels: array<u32>;
@group(0) @binding(3) var<uniform> p: Params;
@group(0) @binding(4) var<storage, read> dabs: array<Dab>;
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
fn evaluate(i:u32) -> vec4f {
 var result=vec4f(0); var k=0u;
 loop {
 if k>=p.layers {break;}
 let base=layers[k]; var g=unpack(source[k*p.count+i]); k+=1u;
 loop {if k>=p.layers {break;} if layers[k].clip==0u {break;}
 let l=layers[k]; let s=unpack(source[k*p.count+i]); let t=s.a*l.opacity;
 if l.enabled!=0u && t>0 && g.a>0 {g=quant(vec4f(g.rgb+(rgb(l.mode,g.rgb,s.rgb)-g.rgb)*t,g.a));}
 k+=1u;
 }
 if base.enabled!=0u {result=blend(result,g,base.opacity,base.mode);}
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
 for(var k=0u;k<p.dabs;k+=1u) {
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
