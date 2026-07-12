//! Deterministic, seedable PRNG: xoshiro256** seeded via SplitMix64.
//!
//! The old JS sim monkey-patched the global `Math.random` for reproducible
//! training, which silently broke whenever new code reached for randomness.
//! Here every world owns its own `Rng`, so determinism is explicit and local.

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Rng {
    s: [u64; 4],
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        // SplitMix64 to fill the xoshiro state from a single seed.
        let mut sm = seed;
        let mut next = || {
            sm = sm.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = sm;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        };
        Rng {
            s: [next(), next(), next(), next()],
        }
    }

    pub(crate) fn has_valid_state(&self) -> bool {
        self.s.iter().any(|&word| word != 0)
    }

    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        let result = self.s[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = self.s[1] << 17;
        self.s[2] ^= self.s[0];
        self.s[3] ^= self.s[1];
        self.s[1] ^= self.s[2];
        self.s[0] ^= self.s[3];
        self.s[2] ^= t;
        self.s[3] = self.s[3].rotate_left(45);
        result
    }

    /// Uniform float in [0, 1).
    #[inline]
    pub fn f32(&mut self) -> f32 {
        ((self.next_u64() >> 40) as f32) / ((1u32 << 24) as f32)
    }

    /// Integer in [0, n). Returns 0 for n <= 0.
    #[inline]
    pub fn below(&mut self, n: i32) -> i32 {
        if n <= 0 {
            return 0;
        }
        (self.next_u64() % n as u64) as i32
    }

    /// Integer in [lo, hi).
    #[inline]
    pub fn range(&mut self, lo: i32, hi: i32) -> i32 {
        if hi <= lo {
            return lo;
        }
        lo + self.below(hi - lo)
    }

    /// One of {-1, 0, +1}, for a random walk step.
    #[inline]
    pub fn step(&mut self) -> i32 {
        self.below(3) - 1
    }

    #[inline]
    pub fn chance(&mut self, p: f32) -> bool {
        self.f32() < p
    }

    /// Standard-normal sample via Box-Muller (used by brain mutation later).
    pub fn gaussian(&mut self) -> f32 {
        let u = self.f32().max(f32::EPSILON);
        let v = self.f32();
        (-2.0 * u.ln()).sqrt() * (std::f32::consts::TAU * v).cos()
    }
}
