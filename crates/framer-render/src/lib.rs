//! `framer-render` — physically based rendering for Framer designs.
//!
//! This crate is the UI-agnostic, fully tested source of truth for Framer's
//! path-traced renderer. It extracts a [`scene::Scene`] from a building model,
//! builds a BVH, and renders it with a CPU path tracer (diffuse / metal /
//! dielectric-glass materials, a directional sun, a procedural sky, multiple
//! importance sampling, and ACES tone mapping). The app's WGSL compute path
//! tracer mirrors this exact math, fed by the same scene.
//!
//! The library has **zero runtime dependencies** beyond `framer-core`; `image`
//! (PNG export) and `rayon` (parallel rendering) are optional and gated behind
//! the `cli` and `parallel` features respectively. All math is `f32` to match
//! WGSL's precision.
#![forbid(unsafe_code)]

pub mod math;
pub mod rng;
