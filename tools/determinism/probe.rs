//! 数学式だけの例。rustc -O tools/determinism/probe.rs -o target/determinism/probe
//! 入出力は「識別子 関数 xの64ビットhex yの64ビットhex [結果hex]」。
use std::io::{self, BufRead, Write};
fn main() {
    let mathf = std::env::args().any(|s| s == "--mathf");
    let mut out = io::BufWriter::new(io::stdout().lock());
    for line in io::stdin().lock().lines() {
        let line = line.unwrap();
        let t: Vec<_> = line.split_whitespace().collect();
        let x = std::hint::black_box(f64::from_bits(u64::from_str_radix(t[2],16).unwrap()));
        let y = std::hint::black_box(f64::from_bits(u64::from_str_radix(t[3],16).unwrap()));
        if mathf {
            let (x,y)=(x as f32,y as f32);
            let z=match t[1] {
                "sin"=>x.sin(), "cos"=>x.cos(), "tan"=>x.tan(), "atan"=>x.atan(),
                "atan2"=>x.atan2(y), "sqrt"=>x.sqrt(), "exp"=>x.exp(), "pow"=>x.powf(y), "log"=>x.ln(),
                _=>panic!("MathF は単体の関数だけを測ります"),
            };
            writeln!(out,"{} {:08x}",line,z.to_bits()).unwrap();
            continue;
        }
        let max = std::f64::consts::FRAC_PI_2;
        let z = match t[1] {
            "sin" => x.sin(), "cos" => x.cos(), "tan" => x.tan(),
            "atan" => x.atan(), "atan2" => x.atan2(y), "sqrt" => x.sqrt(),
            "exp" => x.exp(), "pow" => x.powf(y), "log" => x.ln(),
            "amount" => {
                let (a,b)=(x.abs(), y.abs());
                if a<=0.0 && b<=0.0 {0.0} else if a>=max-1e-9 || b>=max-1e-9 {1.0}
                else {let (a,b)=(a.tan(),b.tan()); ((a*a+b*b).sqrt().atan()/max).min(1.0)}
            },
            "azimuth" => if x==0.0 && y==0.0 {0.0} else {
                y.clamp(-max+1e-9,max-1e-9).tan().atan2(x.clamp(-max+1e-9,max-1e-9).tan())
            },
            _ => panic!("未知の関数"),
        };
        writeln!(out,"{} {:016x}",line,z.to_bits()).unwrap();
    }
}
