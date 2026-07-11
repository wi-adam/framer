use framer_core::BuildingModel;
use framer_solver::ProjectFramePlan;

use crate::diagnostic::{
    GeometryOverlapViolation, GeometryQueryViolation, GeometryViolation, sort_violations,
};
use crate::query::solid_contact;
use crate::spatial::candidate_pairs;
use crate::{Aabb, PhysicalScene, build_physical_scene};

const TICK_INCHES: f64 = 1.0 / 16.0;
const MAX_PENETRATION_EPSILON_INCHES: f64 = TICK_INCHES / 1024.0;
const MIN_PENETRATION_EPSILON_INCHES: f64 = 1.0e-9;
const RELATIVE_PENETRATION_EPSILON: f64 = 1.0e-10;

/// Disposable result of auditing one regenerated physical scene.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GeometryAudit {
    pub violations: Vec<GeometryViolation>,
}

impl GeometryAudit {
    pub fn is_clean(&self) -> bool {
        self.violations.is_empty()
    }
}

/// Build and audit the physical scene for one already-validated model and plan.
pub fn audit_project(model: &BuildingModel, plan: &ProjectFramePlan) -> GeometryAudit {
    let scene = build_physical_scene(model, plan);
    audit_physical_scene(&scene)
}

/// Audit one identity-bearing physical scene. Broad-phase candidates never make
/// the pass/fail decision; every comparable candidate reaches the convex query.
pub fn audit_physical_scene(scene: &PhysicalScene) -> GeometryAudit {
    let mut violations: Vec<_> = scene
        .diagnostics
        .iter()
        .cloned()
        .map(GeometryViolation::BodyUnbuildable)
        .collect();
    let bodies = scene.bodies();
    for pair in candidate_pairs(bodies) {
        let left = &bodies[pair.left];
        let right = &bodies[pair.right];
        if left.body_ref == right.body_ref || left.body_ref.domain() != right.body_ref.domain() {
            continue;
        }
        match solid_contact(&left.solid, &right.solid) {
            Ok(Some(contact)) => {
                let epsilon = penetration_epsilon(left.aabb, right.aabb);
                if contact.penetration_depth > epsilon {
                    violations.push(GeometryViolation::Overlap(GeometryOverlapViolation::new(
                        left.body_ref.clone(),
                        right.body_ref.clone(),
                        contact.penetration_depth,
                        contact.witness,
                    )));
                }
            }
            Ok(None) => {}
            Err(error) => violations.push(GeometryViolation::QueryUnsupported(
                GeometryQueryViolation::new(
                    left.body_ref.clone(),
                    right.body_ref.clone(),
                    error.to_string(),
                ),
            )),
        }
    }
    sort_violations(&mut violations);
    GeometryAudit { violations }
}

fn penetration_epsilon(left: Aabb, right: Aabb) -> f64 {
    let scale = [
        left.min.x,
        left.min.y,
        left.min.z,
        left.max.x,
        left.max.y,
        left.max.z,
        right.min.x,
        right.min.y,
        right.min.z,
        right.max.x,
        right.max.y,
        right.max.z,
    ]
    .into_iter()
    .map(f64::abs)
    .fold(1.0, f64::max);
    (scale * RELATIVE_PENETRATION_EPSILON).clamp(
        MIN_PENETRATION_EPSILON_INCHES,
        MAX_PENETRATION_EPSILON_INCHES,
    )
}

#[cfg(test)]
mod tests;
