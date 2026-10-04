//! UV の配置の比較。三角形の順と頂点の回転を正規化し、違う場合はテクセル被覆を比べる。
use crate::CoreError;
/// 反時計回り・時計回りを区別する三頂点。UV は正規化座標。
pub type UvTriangle = [[f64; 2]; 3];
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UvComparison {
    pub same: bool,
    pub kept: f64,
    pub added: f64,
}
pub const DEFAULT_RESOLUTION: u32 = 256;
pub const MAX_RESOLUTION: u32 = 4096;
pub fn same_triangles(a: &[UvTriangle], b: &[UvTriangle]) -> bool {
    fn canonical(ts: &[UvTriangle]) -> Vec<[[i64; 2]; 3]> {
        let mut out: Vec<_> = ts
            .iter()
            .map(|t| {
                let p = t.map(|p| {
                    p.map(|v| {
                        let v = (v * 1048576.0).round_ties_even();
                        if !v.is_finite()
                            || !(-9223372036854775808.0..9223372036854775808.0).contains(&v)
                        {
                            i64::MIN
                        } else {
                            v as i64
                        }
                    })
                });
                let first = (0..3).min_by_key(|&i| p[i]).unwrap();
                [p[first], p[(first + 1) % 3], p[(first + 2) % 3]]
            })
            .collect();
        out.sort();
        out
    }
    a.len() == b.len() && canonical(a) == canonical(b)
}
pub fn compare(
    before: &[UvTriangle],
    after: &[UvTriangle],
    resolution: u32,
) -> Result<UvComparison, CoreError> {
    if !(1..=MAX_RESOLUTION).contains(&resolution) {
        return Err(CoreError::InvalidArgument("UV 比較の解像度"));
    }
    if same_triangles(before, after) {
        return Ok(UvComparison {
            same: true,
            kept: 1.0,
            added: 0.0,
        });
    }
    let a = coverage(before, resolution);
    let b = coverage(after, resolution);
    let na = a.iter().filter(|v| **v).count();
    let nb = b.iter().filter(|v| **v).count();
    let both = a.iter().zip(&b).filter(|(a, b)| **a && **b).count();
    Ok(UvComparison {
        same: false,
        kept: if na == 0 {
            1.0
        } else {
            both as f64 / na as f64
        },
        added: if nb == 0 {
            0.0
        } else {
            (nb - both) as f64 / nb as f64
        },
    })
}
fn coverage(ts: &[UvTriangle], n: u32) -> Vec<bool> {
    crate::padding::coverage(
        n,
        n,
        ts.iter()
            .map(|t| t.map(|p| crate::glam::DVec2::from_array(p) * n as f64)),
    )
    .expect("検証済みの解像度")
}
