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

/// Sample index reserved for deriving a pixel's low-discrepancy scramble. It is
/// far outside the range of real sample indices, so its PCG stream never collides
/// with a sample's.
const SCRAMBLE_SAMPLE: u32 = 0x00FF_FF00;

/// A stratified sub-pixel jitter in `[0, 1)²` for `sample`, from an XOR-scrambled
/// Sobol′ (0,2)-sequence. Successive samples of a pixel land on a low-discrepancy
/// 2D lattice — cleaner antialiasing and faster pixel convergence than independent
/// PCG jitter — while a per-pixel scramble (from [`pixel_rng`]) decorrelates the
/// lattice between neighbours so the residual looks like noise, not a grid.
///
/// Built from integer ops only (`reverse_bits`, shifts, xor) plus the same 24-bit
/// `f32` conversion as [`Pcg32::next_f32`], so the WGSL mirror is bit-identical.
#[inline]
pub fn stratified_jitter(x: u32, y: u32, sample: u32, seed: u64) -> (f32, f32) {
    let mut scramble = pixel_rng(x, y, SCRAMBLE_SAMPLE, seed);
    let sx = scramble.next_u32();
    let sy = scramble.next_u32();
    (
        sobol_to_f32(van_der_corput(sample, sx)),
        sobol_to_f32(sobol_dim1(sample, sy)),
    )
}

/// Sobol′ dimension 0: the radical inverse base 2 (bit reversal), XOR-scrambled.
#[inline]
fn van_der_corput(n: u32, scramble: u32) -> u32 {
    n.reverse_bits() ^ scramble
}

/// Sobol′ dimension 1 (Gray-code direction numbers `2^31, 2^30, …`), XOR-scrambled.
#[inline]
fn sobol_dim1(n: u32, scramble: u32) -> u32 {
    let mut v: u32 = 1 << 31;
    let mut i = n;
    let mut r = scramble;
    while i != 0 {
        if i & 1 != 0 {
            r ^= v;
        }
        i >>= 1;
        v ^= v >> 1;
    }
    r
}

/// The top 24 bits of a scrambled Sobol′ integer as an `f32` in `[0, 1)`, matching
/// [`Pcg32::next_f32`]'s conversion exactly.
#[inline]
fn sobol_to_f32(bits: u32) -> f32 {
    (bits >> 8) as f32 * (1.0 / (1u32 << 24) as f32)
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
    fn stratified_jitter_is_deterministic_in_range_and_stratified() {
        // Deterministic and in [0, 1)².
        assert_eq!(stratified_jitter(3, 4, 5, 9), stratified_jitter(3, 4, 5, 9));
        for s in 0..256 {
            let (jx, jy) = stratified_jitter(7, 11, s, 0xABCD);
            assert!((0.0..1.0).contains(&jx) && (0.0..1.0).contains(&jy));
        }
        // Low-discrepancy: 16 samples of one pixel hit all 4×4 strata exactly once
        // (the defining property of the Sobol′ (0,2)-sequence under XOR scramble).
        let mut cells = [[0u32; 4]; 4];
        for s in 0..16 {
            let (jx, jy) = stratified_jitter(2, 9, s, 1);
            cells[(jy * 4.0) as usize][(jx * 4.0) as usize] += 1;
        }
        for row in cells {
            for c in row {
                assert_eq!(c, 1, "(0,2)-sequence did not stratify the 4×4 grid");
            }
        }
        // Neighbouring pixels get decorrelated lattices (different sample 0).
        assert_ne!(stratified_jitter(0, 0, 0, 1), stratified_jitter(1, 0, 0, 1));
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
