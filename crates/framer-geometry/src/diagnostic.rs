use std::cmp::Ordering;
use std::fmt;

use crate::{BodyRef, GeometryBuildDiagnostic, Point3};

/// A maintained query adapter could not evaluate a candidate body pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeometryQueryViolation {
    pub body_a: BodyRef,
    pub body_b: BodyRef,
    pub message: String,
}

impl GeometryQueryViolation {
    pub const CODE: &'static str = "geometry.query.unsupported";

    pub(crate) fn new(body_a: BodyRef, body_b: BodyRef, message: impl Into<String>) -> Self {
        let [body_a, body_b] = canonical_pair(body_a, body_b);
        Self {
            body_a,
            body_b,
            message: message.into(),
        }
    }
}

/// Two comparable semantic bodies penetrate beyond the numerical query epsilon.
#[derive(Debug, Clone, PartialEq)]
pub struct GeometryOverlapViolation {
    pub body_a: BodyRef,
    pub body_b: BodyRef,
    pub penetration_depth: f64,
    pub witness: Point3,
}

impl GeometryOverlapViolation {
    pub const CODE: &'static str = "geometry.overlap";

    pub(crate) fn new(
        body_a: BodyRef,
        body_b: BodyRef,
        penetration_depth: f64,
        witness: Point3,
    ) -> Self {
        let [body_a, body_b] = canonical_pair(body_a, body_b);
        Self {
            body_a,
            body_b,
            penetration_depth,
            witness,
        }
    }
}

/// One deterministic, machine-readable geometry audit failure.
#[derive(Debug, Clone, PartialEq)]
pub enum GeometryViolation {
    BodyUnbuildable(GeometryBuildDiagnostic),
    QueryUnsupported(GeometryQueryViolation),
    Overlap(GeometryOverlapViolation),
}

impl GeometryViolation {
    pub fn code(&self) -> &'static str {
        match self {
            Self::BodyUnbuildable(diagnostic) => diagnostic.code,
            Self::QueryUnsupported(_) => GeometryQueryViolation::CODE,
            Self::Overlap(_) => GeometryOverlapViolation::CODE,
        }
    }

    pub fn body_a(&self) -> &BodyRef {
        match self {
            Self::BodyUnbuildable(diagnostic) => &diagnostic.body_ref,
            Self::QueryUnsupported(diagnostic) => &diagnostic.body_a,
            Self::Overlap(diagnostic) => &diagnostic.body_a,
        }
    }

    pub fn body_b(&self) -> Option<&BodyRef> {
        match self {
            Self::BodyUnbuildable(_) => None,
            Self::QueryUnsupported(diagnostic) => Some(&diagnostic.body_b),
            Self::Overlap(diagnostic) => Some(&diagnostic.body_b),
        }
    }
}

impl fmt::Display for GeometryViolation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BodyUnbuildable(diagnostic) => write!(
                formatter,
                "{} body={} message={}",
                diagnostic.code, diagnostic.body_ref, diagnostic.message
            ),
            Self::QueryUnsupported(diagnostic) => write!(
                formatter,
                "{} body_a={} body_b={} message={}",
                GeometryQueryViolation::CODE,
                diagnostic.body_a,
                diagnostic.body_b,
                diagnostic.message
            ),
            Self::Overlap(diagnostic) => write!(
                formatter,
                "{} body_a={} body_b={} depth={:.6}in witness=({:.6},{:.6},{:.6})in",
                GeometryOverlapViolation::CODE,
                diagnostic.body_a,
                diagnostic.body_b,
                diagnostic.penetration_depth,
                diagnostic.witness.x,
                diagnostic.witness.y,
                diagnostic.witness.z
            ),
        }
    }
}

pub(crate) fn sort_violations(violations: &mut [GeometryViolation]) {
    violations.sort_by(compare_violation);
}

fn canonical_pair(left: BodyRef, right: BodyRef) -> [BodyRef; 2] {
    if left <= right {
        [left, right]
    } else {
        [right, left]
    }
}

fn compare_violation(left: &GeometryViolation, right: &GeometryViolation) -> Ordering {
    left.body_a()
        .cmp(right.body_a())
        .then_with(|| left.body_b().cmp(&right.body_b()))
        .then_with(|| left.code().cmp(right.code()))
        .then_with(|| compare_payload(left, right))
}

fn compare_payload(left: &GeometryViolation, right: &GeometryViolation) -> Ordering {
    match (left, right) {
        (GeometryViolation::BodyUnbuildable(left), GeometryViolation::BodyUnbuildable(right)) => {
            left.message.cmp(&right.message)
        }
        (GeometryViolation::QueryUnsupported(left), GeometryViolation::QueryUnsupported(right)) => {
            left.message.cmp(&right.message)
        }
        (GeometryViolation::Overlap(left), GeometryViolation::Overlap(right)) => left
            .penetration_depth
            .total_cmp(&right.penetration_depth)
            .then_with(|| left.witness.x.total_cmp(&right.witness.x))
            .then_with(|| left.witness.y.total_cmp(&right.witness.y))
            .then_with(|| left.witness.z.total_cmp(&right.witness.z)),
        _ => Ordering::Equal,
    }
}

#[cfg(test)]
mod tests {
    use framer_core::ElementId;

    use super::*;
    use crate::{AssemblyKind, BodyRef};

    #[test]
    fn diagnostic_pairs_are_canonical_and_sort_by_body_identity() {
        let early = BodyRef::assembly(ElementId::new("a"), AssemblyKind::Wall);
        let late = BodyRef::assembly(ElementId::new("z"), AssemblyKind::Wall);
        let overlap = GeometryOverlapViolation::new(
            late.clone(),
            early.clone(),
            0.25,
            Point3::new(1.0, 2.0, 3.0),
        );
        assert_eq!(overlap.body_a, early);
        assert_eq!(overlap.body_b, late);
    }
}
