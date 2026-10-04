//! C# の `System.Random`（種あり）と同じ列。Unity のエディタの Mono（.NET Framework の referencesource と同じ Knuth の引き算の
//! 列）で確かめた値と一致する。ブラシのゆらぎ・色の変化・デュアルブラシ・組み込みの筆先が使う（種と引く順も C# と同じにする）。
//!
//! 整数の計算は C# の既定（unchecked）と同じく桁あふれで回る。

const MBIG: i32 = i32::MAX;
const MSEED: i32 = 161_803_398;

/// C# の `new System.Random(seed)` と同じ列を出す乱数。
#[derive(Clone, Debug)]
pub struct NetRandom {
    seed_array: [i32; 56],
    inext: usize,
    inextp: usize,
}

impl NetRandom {
    /// C# の `new Random(seed)`。
    pub fn new(seed: i32) -> NetRandom {
        let mut a = [0i32; 56];
        let subtraction = if seed == i32::MIN {
            i32::MAX
        } else {
            seed.wrapping_abs()
        };
        let mut mj = MSEED.wrapping_sub(subtraction);
        a[55] = mj;
        let mut mk: i32 = 1;
        for i in 1..55 {
            let ii = (21 * i) % 55;
            a[ii] = mk;
            mk = mj.wrapping_sub(mk);
            if mk < 0 {
                mk = mk.wrapping_add(MBIG);
            }
            mj = a[ii];
        }
        for _ in 1..5 {
            for i in 1..56 {
                a[i] = a[i].wrapping_sub(a[1 + (i + 30) % 55]);
                if a[i] < 0 {
                    a[i] = a[i].wrapping_add(MBIG);
                }
            }
        }
        NetRandom {
            seed_array: a,
            inext: 0,
            inextp: 21,
        }
    }

    #[inline]
    fn internal_sample(&mut self) -> i32 {
        let mut next = self.inext + 1;
        if next >= 56 {
            next = 1;
        }
        let mut nextp = self.inextp + 1;
        if nextp >= 56 {
            nextp = 1;
        }
        let mut value = self.seed_array[next].wrapping_sub(self.seed_array[nextp]);
        if value == MBIG {
            value -= 1;
        }
        if value < 0 {
            value = value.wrapping_add(MBIG);
        }
        self.seed_array[next] = value;
        self.inext = next;
        self.inextp = nextp;
        value
    }

    /// C# の `NextDouble()`（0 以上 1 未満）。
    #[inline]
    pub fn next_double(&mut self) -> f64 {
        self.internal_sample() as f64 * (1.0 / MBIG as f64)
    }

    /// C# の `Next(maxValue)`（0 以上 max 未満。max は 0 以上）。
    #[inline]
    pub fn next_below(&mut self, max: i32) -> i32 {
        debug_assert!(max >= 0);
        (self.next_double() * max as f64) as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unity 2022.3 の Mono で `new Random(seed)` から引いた値（tools の外で 1 回だけ測った値。golden の sweeps でも見張る）。
    #[test]
    fn matches_monos_system_random() {
        let cases: [(i32, [u64; 6], [i32; 3], u64); 6] = [
            (
                0,
                [
                    0x3fe73d6286ae7ac5,
                    0x3fea278783344f0f,
                    0x3fe893a451b12749,
                    0x3fe1dc74dbe3b8ea,
                    0x3fca5f4b5d34be97,
                    0x3fe1e2625d63c4c5,
                ],
                [4, 113, 0],
                0x5ed79d4eca81cd2e,
            ),
            (
                -1,
                [
                    0x3fcfd45f463fa8bf,
                    0x3fbc59b7a038b36f,
                    0x3fdde380c33bc702,
                    0x3fe8b0fb20b161f6,
                    0x3fe50a65102a14ca,
                    0x3fdbb2b5cbb7656c,
                ],
                [1, 241, 0],
                0x2e6fb228bc15b8fa,
            ),
            (
                7,
                [
                    0x3fd886af25b10d5e,
                    0x3febe15398f7c2a7,
                    0x3fe52668c12a4cd2,
                    0x3faac20bd8358418,
                    0x3fd773a4caaee74a,
                    0x3fe5a32e18ab465c,
                ],
                [0, 244, 0],
                0xfb56e8ef9dfa9f00,
            ),
            (
                i32::MIN,
                [
                    0x3fe73d6286ae7ac5,
                    0x3fea278783344f0f,
                    0x3fe893a453312749,
                    0x3fe1dc74dbe3b8ea,
                    0x3fca5f4b5d34be97,
                    0x3fe1e2625ce3c4c5,
                ],
                [4, 113, 0],
                0x6abfb226ed5d67be,
            ),
            (
                0x2545F491 ^ 7,
                [
                    0x3fd6e1b168adc363,
                    0x3fe5a77cf7eb4efa,
                    0x3fbfca310c3f9462,
                    0x3fc2864a47250c95,
                    0x3fe2585aba64b0b5,
                    0x3fdf2f9737be5f2e,
                ],
                [2, 147, 0],
                0xe51c550aca6089f4,
            ),
            (
                -987654321,
                [
                    0x3fe5eae3a62bd5c7,
                    0x3fd5e33d12abc67a,
                    0x3fec7d7370b8fae7,
                    0x3fce6f8dab3cdf1b,
                    0x3fee917edc7d22fe,
                    0x3fdc3f24cbb87e4a,
                ],
                [1, 169, 0],
                0x4bca87ae950d6a46,
            ),
        ];
        for (seed, doubles, ints, hash) in cases {
            let mut r = NetRandom::new(seed);
            for d in doubles {
                assert_eq!(r.next_double().to_bits(), d, "種 {seed}");
            }
            assert_eq!(
                [r.next_below(5), r.next_below(256), r.next_below(1)],
                ints,
                "種 {seed}"
            );
            let mut h: u64 = 14695981039346656037;
            for _ in 0..100_000 {
                for b in r.next_double().to_bits().to_le_bytes() {
                    h ^= b as u64;
                    h = h.wrapping_mul(1099511628211);
                }
            }
            assert_eq!(h, hash, "種 {seed} の 10 万個");
        }
    }
}
