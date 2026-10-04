use super::input::{cross, dot, sub};
#[derive(Clone, Copy, Default, Debug)]
struct Node {
    bounds: [f32; 6],
    first: usize,
    count: usize,
}
#[derive(Clone, Copy)]
struct Tri {
    a: [f64; 3],
    e1: [f64; 3],
    e2: [f64; 3],
    epsilon: f64,
    original: usize,
}
/// ベイク用。C# MeshRayBvh と同じ12ビン SAH と倍精度の交差判定。
pub struct MeshRayBvh {
    nodes: Vec<Node>,
    tris: Vec<Tri>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeshRayHit {
    pub distance: f64,
    pub triangle: usize,
    pub u: f64,
    pub v: f64,
}
const EMPTY: [f32; 6] = [f32::MAX, f32::MAX, f32::MAX, f32::MIN, f32::MIN, f32::MIN];
fn grow(a: &mut [f32; 6], b: &[f32; 6]) {
    if b[0] > b[3] {
        return;
    }
    for k in 0..3 {
        a[k] = a[k].min(b[k]);
        a[k + 3] = a[k + 3].max(b[k + 3]);
    }
}
fn area(b: [f32; 6]) -> f32 {
    if b[0] > b[3] {
        return 0.;
    }
    let x = b[3] - b[0];
    let y = b[4] - b[1];
    let z = b[5] - b[2];
    x * y + y * z + z * x
}
fn bin(center: f32, lo: f32, scale: f32) -> usize {
    (((center - lo) * scale) as i32).clamp(0, 11) as usize
}
impl MeshRayBvh {
    pub fn new(input: &super::MeshBakeInput, triangles: &[usize]) -> super::Result<Self> {
        super::check(
            triangles.iter().all(|t| *t < input.triangle_count()),
            "BVH の面番号が範囲外です",
        )?;
        Ok(Self::build(&input.corners, triangles))
    }
    pub(crate) fn build(corners: &[f32], triangles: &[usize]) -> Self {
        let mut bounds = Vec::with_capacity(triangles.len());
        let mut centers = Vec::with_capacity(triangles.len());
        for &t in triangles {
            let mut b = [0.; 6];
            let mut c = [0.; 3];
            for a in 0..3 {
                let k = t * 9 + a;
                b[a] = corners[k].min(corners[k + 3].min(corners[k + 6]));
                b[a + 3] = corners[k].max(corners[k + 3].max(corners[k + 6]));
                c[a] = (b[a] + b[a + 3]) * 0.5;
            }
            bounds.push(b);
            centers.push(c);
        }
        let mut order: Vec<usize> = (0..triangles.len()).collect();
        let mut nodes = if triangles.is_empty() {
            vec![]
        } else {
            vec![Node::default()]
        };
        if !order.is_empty() {
            build_node(&mut nodes, 0, &mut order, 0, 0, &bounds, &centers);
        }
        let tris = order
            .iter()
            .map(|&i| {
                let t = triangles[i];
                let k = t * 9;
                let a = std::array::from_fn(|j| corners[k + j] as f64);
                let e1 = std::array::from_fn(|j| corners[k + 3 + j] as f64 - a[j]);
                let e2 = std::array::from_fn(|j| corners[k + 6 + j] as f64 - a[j]);
                Tri {
                    a,
                    e1,
                    e2,
                    epsilon: 1e-9 * (dot(e1, e1) * dot(e2, e2)).sqrt(),
                    original: t,
                }
            })
            .collect();
        Self { nodes, tris }
    }
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }
    /// 節点と面の並びを C# Flatten と照合するための読み取り口。
    pub fn visit(
        &self,
        mut node: impl FnMut([f32; 6], usize, usize),
        mut triangle: impl FnMut(usize, [f32; 10]),
    ) {
        for n in &self.nodes {
            node(n.bounds, n.first, n.count);
        }
        for t in &self.tris {
            let mut v = [0.; 10];
            for i in 0..3 {
                v[i] = t.a[i] as f32;
                v[3 + i] = t.e1[i] as f32;
                v[6 + i] = t.e2[i] as f32;
            }
            v[9] = t.epsilon as f32;
            triangle(t.original, v);
        }
    }
    pub fn trace(
        &self,
        origin: [f64; 3],
        mut direction: [f64; 3],
        limit: f64,
        ignore: Option<usize>,
        any_hit: bool,
        ignore_backfaces: bool,
    ) -> Option<MeshRayHit> {
        if self.nodes.is_empty() {
            return None;
        }
        for d in &mut direction {
            if *d == 0. {
                *d = 1e-300;
            }
        }
        let inv = direction.map(|d| 1. / d);
        let oi = std::array::from_fn(|a| origin[a] * inv[a]);
        let mut best = limit;
        let mut hit = None;
        intersect_box(self.nodes[0].bounds, inv, oi, best)?;
        let mut stack = [(0usize, 0f64); 128];
        let mut sp = 0;
        let mut at = 0;
        loop {
            let n = self.nodes[at];
            if n.count > 0 {
                for t in &self.tris[n.first..n.first + n.count] {
                    let q0 = cross(direction, t.e2);
                    let det = dot(t.e1, q0);
                    if det > -t.epsilon && det < t.epsilon || ignore_backfaces && det < 0. {
                        continue;
                    }
                    let reciprocal = 1. / det;
                    let s = sub(origin, t.a);
                    let u = dot(s, q0) * reciprocal;
                    if !(0. ..=1.).contains(&u) {
                        continue;
                    }
                    let q = cross(s, t.e1);
                    let v = dot(direction, q) * reciprocal;
                    if v < 0. || u + v > 1. {
                        continue;
                    }
                    let distance = dot(t.e2, q) * reciprocal;
                    if distance <= 0. || distance >= best || Some(t.original) == ignore {
                        continue;
                    }
                    best = distance;
                    hit = Some(MeshRayHit {
                        distance,
                        triangle: t.original,
                        u,
                        v,
                    });
                    if any_hit {
                        return hit;
                    }
                }
            } else {
                let l = n.first;
                let left = intersect_box(self.nodes[l].bounds, inv, oi, best);
                let right = intersect_box(self.nodes[l + 1].bounds, inv, oi, best);
                match (left, right) {
                    (Some(a), Some(b)) => {
                        if a <= b {
                            at = l;
                            stack[sp] = (l + 1, b);
                        } else {
                            at = l + 1;
                            stack[sp] = (l, a);
                        }
                        sp += 1;
                        continue;
                    }
                    (Some(_), None) => {
                        at = l;
                        continue;
                    }
                    (None, Some(_)) => {
                        at = l + 1;
                        continue;
                    }
                    _ => {}
                }
            }
            loop {
                if sp == 0 {
                    return hit;
                }
                sp -= 1;
                if stack[sp].1 < best {
                    at = stack[sp].0;
                    break;
                }
            }
        }
    }
}
fn intersect_box(b: [f32; 6], inv: [f64; 3], oi: [f64; 3], limit: f64) -> Option<f64> {
    let mut a = [0.; 3];
    let mut c = [0.; 3];
    for k in 0..3 {
        a[k] = b[k + if inv[k] >= 0. { 0 } else { 3 }] as f64 * inv[k] - oi[k];
        c[k] = b[k + if inv[k] >= 0. { 3 } else { 0 }] as f64 * inv[k] - oi[k];
    }
    let mut lo = if a[0] > a[1] { a[0] } else { a[1] };
    if a[2] > lo {
        lo = a[2];
    }
    if lo < 0. {
        lo = 0.;
    }
    let mut hi = if c[0] < c[1] { c[0] } else { c[1] };
    if c[2] < hi {
        hi = c[2];
    }
    if limit < hi {
        hi = limit;
    }
    if lo <= hi {
        Some(lo)
    } else {
        None
    }
}
fn build_node(
    nodes: &mut Vec<Node>,
    at: usize,
    order: &mut [usize],
    start: usize,
    depth: usize,
    bounds: &[[f32; 6]],
    centers: &[[f32; 3]],
) {
    let mut b = EMPTY;
    let mut c = EMPTY;
    for &t in order.iter() {
        grow(&mut b, &bounds[t]);
        grow(
            &mut c,
            &[
                centers[t][0],
                centers[t][1],
                centers[t][2],
                centers[t][0],
                centers[t][1],
                centers[t][2],
            ],
        );
    }
    nodes[at] = Node {
        bounds: b,
        first: start,
        count: order.len(),
    };
    if order.len() <= 4 {
        return;
    }
    let e = [c[3] - c[0], c[4] - c[1], c[5] - c[2]];
    let axis = if e[0] >= e[1] && e[0] >= e[2] {
        0
    } else if e[1] >= e[2] {
        1
    } else {
        2
    };
    let mut mid = None;
    if e[axis] > 1e-30 && depth < 60 {
        let scale = 12. / e[axis];
        let mut counts = [0usize; 12];
        let mut bins = [EMPTY; 12];
        for &t in order.iter() {
            let i = bin(centers[t][axis], c[axis], scale);
            counts[i] += 1;
            grow(&mut bins[i], &bounds[t]);
        }
        let mut right_area = [0.; 12];
        let mut right_count = [0usize; 12];
        let mut acc = EMPTY;
        let mut count = 0;
        for i in (1..12).rev() {
            grow(&mut acc, &bins[i]);
            count += counts[i];
            right_area[i] = area(acc);
            right_count[i] = count;
        }
        acc = EMPTY;
        count = 0;
        let mut best = f32::MAX;
        let mut split = None;
        for i in 0..11 {
            grow(&mut acc, &bins[i]);
            count += counts[i];
            if count == 0 || right_count[i + 1] == 0 {
                continue;
            }
            let cost = area(acc) * count as f32 + right_area[i + 1] * right_count[i + 1] as f32;
            if cost < best {
                best = cost;
                split = Some(i);
            }
        }
        if let Some(split) = split {
            let mut left = 0;
            let mut right = order.len();
            while left < right {
                if bin(centers[order[left]][axis], c[axis], scale) <= split {
                    left += 1;
                } else {
                    right -= 1;
                    order.swap(left, right);
                }
            }
            if left > 0 && left < order.len() {
                mid = Some(left);
            }
        }
    }
    let mid = mid.unwrap_or_else(|| {
        mono_sort(order, centers, axis);
        order.len() / 2
    });
    let child = nodes.len();
    nodes.extend([Node::default(); 2]);
    nodes[at].first = child;
    nodes[at].count = 0;
    let (left, right) = order.split_at_mut(mid);
    build_node(nodes, child, left, start, depth + 1, bounds, centers);
    build_node(
        nodes,
        child + 1,
        right,
        start + mid,
        depth + 1,
        bounds,
        centers,
    );
}
// Mono Array.Sort の introsort。等しい中心も同じ順に置く。
fn mono_sort(order: &mut [usize], centers: &[[f32; 3]], axis: usize) {
    fn swap_if(a: &mut [usize], i: usize, j: usize, c: &[[f32; 3]], axis: usize) {
        if i != j && c[a[i]][axis] > c[a[j]][axis] {
            a.swap(i, j);
        }
    }
    fn sort(a: &mut [usize], c: &[[f32; 3]], axis: usize, depth: usize) {
        let n = a.len();
        if n < 2 {
            return;
        }
        if n <= 16 {
            if n == 2 {
                swap_if(a, 0, 1, c, axis);
                return;
            }
            if n == 3 {
                swap_if(a, 0, 1, c, axis);
                swap_if(a, 0, 2, c, axis);
                swap_if(a, 1, 2, c, axis);
                return;
            }
            for i in 1..n {
                let t = a[i];
                let mut j = i;
                while j > 0 && c[t][axis] < c[a[j - 1]][axis] {
                    a[j] = a[j - 1];
                    j -= 1;
                }
                a[j] = t;
            }
            return;
        }
        if depth == 0 {
            // 同じ比較のヒープ順にする。
            fn down(a: &mut [usize], mut i: usize, n: usize, c: &[[f32; 3]], axis: usize) {
                let d = a[i - 1];
                while i <= n / 2 {
                    let mut child = 2 * i;
                    if child < n && c[a[child - 1]][axis] < c[a[child]][axis] {
                        child += 1;
                    }
                    if c[d][axis] >= c[a[child - 1]][axis] {
                        break;
                    }
                    a[i - 1] = a[child - 1];
                    i = child;
                }
                a[i - 1] = d;
            }
            for i in (1..=n / 2).rev() {
                down(a, i, n, c, axis);
            }
            for i in (2..=n).rev() {
                a.swap(0, i - 1);
                down(a, 1, i - 1, c, axis);
            }
            return;
        }
        let m = (n - 1) / 2;
        swap_if(a, 0, m, c, axis);
        swap_if(a, 0, n - 1, c, axis);
        swap_if(a, m, n - 1, c, axis);
        let pivot = c[a[m]][axis];
        a.swap(m, n - 2);
        let mut left = 0;
        let mut right = n - 2;
        loop {
            left += 1;
            while c[a[left]][axis] < pivot {
                left += 1;
            }
            right -= 1;
            while pivot < c[a[right]][axis] {
                right -= 1;
            }
            if left >= right {
                break;
            }
            a.swap(left, right);
        }
        a.swap(left, n - 2);
        let (l, r) = a.split_at_mut(left);
        sort(&mut r[1..], c, axis, depth - 1);
        sort(l, c, axis, depth - 1);
    }
    let depth = 2 * ((usize::BITS - order.len().leading_zeros()) as usize);
    sort(order, centers, axis, depth);
}
