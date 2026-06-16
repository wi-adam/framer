// PCG XSH-RR 64/32 random number generator, mirroring framer_render::rng exactly.
//
// WGSL has no 64-bit integer type, so the 64-bit PCG state is emulated with a
// `vec2<u32>` holding (low 32 bits in .x, high 32 bits in .y). The 64-bit
// multiply/add/shift helpers below reproduce wrapping `u64` arithmetic bit-for-
// bit, so this generator produces the *identical* stream to the CPU's `Pcg32`
// (the parity test locks this against the canonical pcg_basic canary sequence).

struct Rng {
    state: vec2<u32>,
    inc: vec2<u32>,
};

// 6364136223846793005 = 0x5851F42D4C957F2D  (low, high)
const PCG_MUL: vec2<u32> = vec2<u32>(0x4c957f2du, 0x5851f42du);

// Full 32x32 -> 64-bit product, returned as (low, high).
fn mul_u32_full(a: u32, b: u32) -> vec2<u32> {
    let a_lo = a & 0xffffu;
    let a_hi = a >> 16u;
    let b_lo = b & 0xffffu;
    let b_hi = b >> 16u;

    let ll = a_lo * b_lo;
    let lh = a_lo * b_hi;
    let hl = a_hi * b_lo;
    let hh = a_hi * b_hi;

    var lo = ll;
    var hi = hh;

    // Add lh << 16 with carry into the high word.
    let lh_lo = lh << 16u;
    var sum = lo + lh_lo;
    var carry = select(0u, 1u, sum < lo);
    lo = sum;
    hi = hi + (lh >> 16u) + carry;

    // Add hl << 16 with carry into the high word.
    let hl_lo = hl << 16u;
    sum = lo + hl_lo;
    carry = select(0u, 1u, sum < lo);
    lo = sum;
    hi = hi + (hl >> 16u) + carry;

    return vec2<u32>(lo, hi);
}

// Low 64 bits of a 64x64 multiply.
fn umul64(a: vec2<u32>, b: vec2<u32>) -> vec2<u32> {
    let ll = mul_u32_full(a.x, b.x);
    // Cross terms only contribute to the high word (mod 2^64).
    let mid = a.x * b.y + a.y * b.x;
    return vec2<u32>(ll.x, ll.y + mid);
}

fn uadd64(a: vec2<u32>, b: vec2<u32>) -> vec2<u32> {
    let lo = a.x + b.x;
    let carry = select(0u, 1u, lo < a.x);
    return vec2<u32>(lo, a.y + b.y + carry);
}

fn uxor64(a: vec2<u32>, b: vec2<u32>) -> vec2<u32> {
    return vec2<u32>(a.x ^ b.x, a.y ^ b.y);
}

// Logical right shift by k in [0, 63].
fn ushr64(a: vec2<u32>, k: u32) -> vec2<u32> {
    if (k == 0u) {
        return a;
    }
    if (k < 32u) {
        let lo = (a.x >> k) | (a.y << (32u - k));
        return vec2<u32>(lo, a.y >> k);
    }
    return vec2<u32>(a.y >> (k - 32u), 0u);
}

// Logical left shift by k in [0, 63].
fn ushl64(a: vec2<u32>, k: u32) -> vec2<u32> {
    if (k == 0u) {
        return a;
    }
    if (k < 32u) {
        let hi = (a.y << k) | (a.x >> (32u - k));
        return vec2<u32>(a.x << k, hi);
    }
    return vec2<u32>(0u, a.x << (k - 32u));
}

fn rotr32(x: u32, r: u32) -> u32 {
    return (x >> r) | (x << ((32u - r) & 31u));
}

fn pcg_next_u32(rng: ptr<function, Rng>) -> u32 {
    let old = (*rng).state;
    (*rng).state = uadd64(umul64(old, PCG_MUL), (*rng).inc);
    // xorshifted = (((old >> 18) ^ old) >> 27) as u32
    let x = ushr64(uxor64(ushr64(old, 18u), old), 27u);
    let rot = ushr64(old, 59u).x;
    return rotr32(x.x, rot);
}

fn pcg_seed(init_state: vec2<u32>, init_seq: vec2<u32>) -> Rng {
    var rng: Rng;
    var inc = ushl64(init_seq, 1u);
    inc.x = inc.x | 1u;
    rng.inc = inc;
    rng.state = vec2<u32>(0u, 0u);
    let _ignored0 = pcg_next_u32(&rng);
    rng.state = uadd64(rng.state, init_state);
    let _ignored1 = pcg_next_u32(&rng);
    return rng;
}

fn pcg_next_f32(rng: ptr<function, Rng>) -> f32 {
    let u = pcg_next_u32(rng);
    return f32(u >> 8u) * (1.0 / 16777216.0);
}

// Builds an independent generator for pixel (x, y), sample index, and a 64-bit
// `seed` (passed as low/high), via the same SplitMix64 finalizer as the CPU.
fn pixel_rng(x: u32, y: u32, sample: u32, seed: vec2<u32>) -> Rng {
    var z = vec2<u32>(x, 0u);
    z = z | ushl64(vec2<u32>(y, 0u), 20u);
    z = z | ushl64(vec2<u32>(sample, 0u), 40u);
    // 0x9E3779B97F4A7C15
    z = uadd64(z, uadd64(seed, vec2<u32>(0x7f4a7c15u, 0x9e3779b9u)));
    // 0xBF58476D1CE4E5B9
    z = umul64(uxor64(z, ushr64(z, 30u)), vec2<u32>(0x1ce4e5b9u, 0xbf58476du));
    // 0x94D049BB133111EB
    z = umul64(uxor64(z, ushr64(z, 27u)), vec2<u32>(0x133111ebu, 0x94d049bbu));
    z = uxor64(z, ushr64(z, 31u));
    // 0xDA3E39CB94B95BDB
    let seq = uxor64(seed, vec2<u32>(0x94b95bdbu, 0xda3e39cbu));
    return pcg_seed(z, seq);
}

// ---- Low-discrepancy sub-pixel jitter (mirrors framer_render::rng) ----------

const SCRAMBLE_SAMPLE: u32 = 0x00ffff00u;

// Sobol' dimension 0: radical inverse base 2 (bit reversal), XOR-scrambled.
fn van_der_corput(n: u32, scramble: u32) -> u32 {
    return reverseBits(n) ^ scramble;
}

// Sobol' dimension 1 (Gray-code direction numbers 2^31, 2^30, ...), XOR-scrambled.
fn sobol_dim1(n: u32, scramble: u32) -> u32 {
    var v: u32 = 1u << 31u;
    var i = n;
    var r = scramble;
    loop {
        if (i == 0u) {
            break;
        }
        if ((i & 1u) != 0u) {
            r = r ^ v;
        }
        i = i >> 1u;
        v = v ^ (v >> 1u);
    }
    return r;
}

fn sobol_to_f32(bits: u32) -> f32 {
    return f32(bits >> 8u) * (1.0 / 16777216.0);
}

// Stratified sub-pixel jitter in [0,1)^2 for `sample`, from an XOR-scrambled
// Sobol' (0,2)-sequence with a per-pixel scramble. Bit-identical to the CPU.
fn stratified_jitter(x: u32, y: u32, sample: u32, seed: vec2<u32>) -> vec2<f32> {
    var scramble = pixel_rng(x, y, SCRAMBLE_SAMPLE, seed);
    let sx = pcg_next_u32(&scramble);
    let sy = pcg_next_u32(&scramble);
    return vec2<f32>(
        sobol_to_f32(van_der_corput(sample, sx)),
        sobol_to_f32(sobol_dim1(sample, sy)),
    );
}
