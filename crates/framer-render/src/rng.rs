//! Deterministic, dependency-free random numbers for the path tracer.
//!
//! [`Pcg32`] is the PCG XSH-RR 64/32 generator (Melissa O'Neill). Per-pixel,
//! per-sample seeding ([`pixel_rng`]) makes every pixel an independent stream, so
//! a render is a pure function of the global seed and **independent of thread
//! scheduling** — parallel and single-threaded renders are byte-identical. The
//! WGSL renderer mirrors this generator.

/// PCG XSH-RR 64/32 generator. 8 bytes of state plus an odd stream selector.
#[derive(Clone, Debug)]
pub struct Pcg32 {
    state: u64,
    inc: u64,
}

impl Pcg32 {
    const MUL: u64 = 6364136223846793005;

    /// Seeds the generator following the reference `pcg32_srandom_r`:
    /// `init_seq` selects the stream (made odd), `init_state` the start point.
    #[inline]
    pub fn seed(init_state: u64, init_seq: u64) -> Self {
        let mut rng = Self {
            state: 0,
            inc: (init_seq << 1) | 1,
        };
        rng.next_u32();
        rng.state = rng.state.wrapping_add(init_state);
        rng.next_u32();
        rng
    }

    /// Advances the state and returns the next 32-bit output.
    #[inline]
    pub fn next_u32(&mut self) -> u32 {
        let old = self.state;
        self.state = old.wrapping_mul(Self::MUL).wrapping_add(self.inc);
        let xorshifted = (((old >> 18) ^ old) >> 27) as u32;
        let rot = (old >> 59) as u32;
        xorshifted.rotate_right(rot)
    }

    /// A uniform `f32` in `[0, 1)`. Uses the top 24 bits (PCG's highest quality)
    /// scaled by `2^-24`, so the result is never exactly `1.0`.
    #[inline]
    pub fn next_f32(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 * (1.0 / (1u32 << 24) as f32)
    }
}

/// Builds an independent generator for pixel `(x, y)`, sample index `sample`,
/// and a global `seed`. A SplitMix64 finalizer decorrelates neighbouring pixels
/// so each is a statistically independent stream.
#[inline]
pub fn pixel_rng(x: u32, y: u32, sample: u32, seed: u64) -> Pcg32 {
    let mut z = (x as u64) | ((y as u64) << 20) | ((sample as u64) << 40);
    z = z.wrapping_add(seed).wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    Pcg32::seed(z, seed ^ 0xDA3E_39CB_94B9_5BDB)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canary_matches_pcg_reference() {
        // The canonical `pcg_basic` demo sequence for seed (42, 54). If these
        // ever change, the generator was altered — and the GPU mirror + every
        // golden image would silently drift. Locked on purpose.
        let mut rng = Pcg32::seed(42, 54);
        let expected = [
            0xa15c02b7u32,
            0x7b47f409,
            0xba1d3330,
            0x83d2f293,
            0xbfa4784b,
            0xcbed606e,
        ];
        for (i, want) in expected.iter().enumerate() {
            assert_eq!(rng.next_u32(), *want, "mismatch at draw {i}");
        }
    }

    #[test]
    fn next_f32_is_in_unit_interval() {
        let mut rng = Pcg32::seed(1, 2);
        for _ in 0..100_000 {
            let x = rng.next_f32();
            assert!((0.0..1.0).contains(&x), "out of range: {x}");
        }
    }

    #[test]
    fn next_f32_mean_is_about_half() {
        let mut rng = Pcg32::seed(7, 7);
        let n = 1_000_000;
        let mut sum = 0.0f64;
        for _ in 0..n {
            sum += rng.next_f32() as f64;
        }
        let mean = sum / n as f64;
        assert!((mean - 0.5).abs() < 1e-3, "mean={mean}");
    }

    #[test]
    fn same_seed_same_sequence() {
        let mut a = Pcg32::seed(123, 456);
        let mut b = Pcg32::seed(123, 456);
        for _ in 0..1000 {
            assert_eq!(a.next_u32(), b.next_u32());
        }
    }

    #[test]
    fn pixel_rng_is_deterministic_and_decorrelated() {
        // Same coordinates -> same stream.
        let mut a = pixel_rng(10, 20, 3, 0xDEAD_BEEF);
        let mut b = pixel_rng(10, 20, 3, 0xDEAD_BEEF);
        assert_eq!(a.next_u32(), b.next_u32());

        // Adjacent pixels / samples -> different first draws (no obvious correlation).
        let p00 = pixel_rng(0, 0, 0, 1).next_u32();
        let p10 = pixel_rng(1, 0, 0, 1).next_u32();
        let p01 = pixel_rng(0, 1, 0, 1).next_u32();
        let p_s = pixel_rng(0, 0, 1, 1).next_u32();
        assert_ne!(p00, p10);
        assert_ne!(p00, p01);
        assert_ne!(p00, p_s);
        assert_ne!(p10, p01);
    }
}
