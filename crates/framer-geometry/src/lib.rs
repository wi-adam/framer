//! UI-free physical-solid derivation for Framer's authored assemblies and
//! generated framing plan.
//!
//! This crate is the boundary between deterministic, integer-tick construction
//! semantics and disposable floating-point geometry. Presentation layers lower
//! [`PhysicalSolid`] into their own vertex/material formats; collision auditing
//! consumes the same identity-bearing solids.

#![forbid(unsafe_code)]

mod build;
mod solid;

pub use build::{
    RafterPrism, build_common_rafter_solid, build_physical_scene, matched_bearing_depth,
    ridge_face_setback,
};
pub use solid::{
    Aabb, AssemblyKind, BodyKind, BodyRef, CollisionDomain, ConvexPiece, GeometryBuildDiagnostic,
    PhysicalBody, PhysicalScene, PhysicalSolid, Point3, TriMesh,
};
