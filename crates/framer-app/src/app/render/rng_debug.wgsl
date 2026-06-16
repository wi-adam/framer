// Debug harness for the RNG parity test. Concatenated after rng.wgsl, it dumps a
// fixed battery of generator outputs into a storage buffer so the headless test
// can compare them against framer_render::rng on the CPU, bit-for-bit.
//
// Layout of `out`:
//   [0..6)   first 6 next_u32 of Pcg32::seed(42, 54)   (the pcg_basic canary)
//   [6..10)  first 4 next_u32 of pixel_rng(10, 20, 3, seed=0xDEADBEEF)
//   [10..14) first 4 next_u32 of pixel_rng(0, 0, 0, seed=1)
//   [14..16) first 2 next_u32 of pixel_rng(63, 47, 11, seed=0x1234_5678_9ABC_DEF0)

@group(0) @binding(0) var<storage, read_write> out: array<u32>;

@compute @workgroup_size(1, 1, 1)
fn main() {
    var canary = pcg_seed(vec2<u32>(42u, 0u), vec2<u32>(54u, 0u));
    for (var i = 0u; i < 6u; i = i + 1u) {
        out[i] = pcg_next_u32(&canary);
    }

    var a = pixel_rng(10u, 20u, 3u, vec2<u32>(0xdeadbeefu, 0u));
    for (var i = 0u; i < 4u; i = i + 1u) {
        out[6u + i] = pcg_next_u32(&a);
    }

    var b = pixel_rng(0u, 0u, 0u, vec2<u32>(1u, 0u));
    for (var i = 0u; i < 4u; i = i + 1u) {
        out[10u + i] = pcg_next_u32(&b);
    }

    var c = pixel_rng(63u, 47u, 11u, vec2<u32>(0x9abcdef0u, 0x12345678u));
    out[14u] = pcg_next_u32(&c);
    out[15u] = pcg_next_u32(&c);
}
