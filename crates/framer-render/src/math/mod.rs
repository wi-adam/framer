//! Small, dependency-free linear-algebra primitives shared by the path tracer.
//!
//! All types are `f32` to mirror the WGSL GPU renderer exactly.

pub mod onb;
pub mod vec3;

pub use onb::Onb;
pub use vec3::Vec3;
