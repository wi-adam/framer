use framer_core::ElementId;
use framer_solver::MemberKind;

/// One point in derived physical space, measured in inches.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Point3 {
    pub const X: Self = Self::new(1.0, 0.0, 0.0);
    pub const Z: Self = Self::new(0.0, 0.0, 1.0);

    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    pub fn distance_squared(self, other: Self) -> f64 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        let dz = self.z - other.z;
        dx * dx + dy * dy + dz * dz
    }
}

/// An indexed triangle mesh describing a semantic solid. Member bodies expose
/// their exact exterior mesh for render/pick reuse. Assembly unions may retain
/// shared internal piece faces; collision queries consume `convex_pieces` as a
/// union, so those bookkeeping faces do not change occupied volume.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TriMesh {
    pub points: Vec<Point3>,
    pub triangles: Vec<[usize; 3]>,
}

impl TriMesh {
    pub fn append(&mut self, other: &Self) {
        let base = self.points.len();
        self.points.extend_from_slice(&other.points);
        self.triangles.extend(
            other
                .triangles
                .iter()
                .map(|triangle| triangle.map(|index| base + index)),
        );
    }
}

/// One convex piece of a semantic solid. Concave bodies are represented as a
/// union of these pieces, while retaining one [`BodyRef`] and one exterior mesh.
#[derive(Debug, Clone, PartialEq)]
pub struct ConvexPiece {
    mesh: TriMesh,
}

impl ConvexPiece {
    pub(crate) fn new(mesh: TriMesh) -> Option<Self> {
        (mesh.points.len() >= 4 && !mesh.triangles.is_empty()).then_some(Self { mesh })
    }

    pub fn mesh(&self) -> &TriMesh {
        &self.mesh
    }
}

/// A semantic solid's indexed surface plus its exact convex-union query lowering.
#[derive(Debug, Clone, PartialEq)]
pub struct PhysicalSolid {
    pub surface: TriMesh,
    pub convex_pieces: Vec<ConvexPiece>,
}

impl PhysicalSolid {
    pub fn new(surface: TriMesh, convex_pieces: Vec<ConvexPiece>) -> Option<Self> {
        (!surface.points.is_empty() && !surface.triangles.is_empty() && !convex_pieces.is_empty())
            .then_some(Self {
                surface,
                convex_pieces,
            })
    }

    pub fn aabb(&self) -> Aabb {
        Aabb::from_points(&self.surface.points)
    }
}

/// Axis-aligned bounds in physical inches.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aabb {
    pub min: Point3,
    pub max: Point3,
}

impl Aabb {
    pub fn from_points(points: &[Point3]) -> Self {
        let mut min = Point3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY);
        let mut max = Point3::new(f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
        for point in points {
            min.x = min.x.min(point.x);
            min.y = min.y.min(point.y);
            min.z = min.z.min(point.z);
            max.x = max.x.max(point.x);
            max.y = max.y.max(point.y);
            max.z = max.z.max(point.z);
        }
        Self { min, max }
    }
}

/// The two independently audited physical detail levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CollisionDomain {
    StructuralFraming,
    FinishedAssembly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AssemblyKind {
    Wall,
    FloorDeck,
    Ceiling,
    RoofPlane,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BodyKind {
    Assembly(AssemblyKind),
    FrameMember(MemberKind),
}

/// Stable semantic identity for a physical body. Internal convex pieces never
/// appear in diagnostics; all reports point back to this canonical reference.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BodyRef {
    pub domain: CollisionDomain,
    pub owner: ElementId,
    pub kind: BodyKind,
    pub member_id: Option<String>,
}

impl BodyRef {
    pub fn member(owner: ElementId, kind: MemberKind, member_id: impl Into<String>) -> Self {
        Self {
            domain: CollisionDomain::StructuralFraming,
            owner,
            kind: BodyKind::FrameMember(kind),
            member_id: Some(member_id.into()),
        }
    }

    pub fn assembly(owner: ElementId, kind: AssemblyKind) -> Self {
        Self {
            domain: CollisionDomain::FinishedAssembly,
            owner,
            kind: BodyKind::Assembly(kind),
            member_id: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PhysicalBody {
    pub body_ref: BodyRef,
    pub solid: PhysicalSolid,
    pub aabb: Aabb,
}

impl PhysicalBody {
    pub fn new(body_ref: BodyRef, solid: PhysicalSolid) -> Self {
        let aabb = solid.aabb();
        Self {
            body_ref,
            solid,
            aabb,
        }
    }
}

/// A fail-closed physical-body construction issue.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct GeometryBuildDiagnostic {
    pub code: &'static str,
    pub body_ref: BodyRef,
    pub message: String,
}

impl GeometryBuildDiagnostic {
    pub const CODE: &'static str = "geometry.body.unbuildable";

    pub fn unbuildable(body_ref: BodyRef, message: impl Into<String>) -> Self {
        Self {
            code: Self::CODE,
            body_ref,
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PhysicalScene {
    pub bodies: Vec<PhysicalBody>,
    pub diagnostics: Vec<GeometryBuildDiagnostic>,
}

impl PhysicalScene {
    pub fn body(&self, body_ref: &BodyRef) -> Option<&PhysicalBody> {
        self.bodies
            .binary_search_by(|body| body.body_ref.cmp(body_ref))
            .ok()
            .map(|index| &self.bodies[index])
    }

    pub(crate) fn finish(mut self) -> Self {
        self.bodies
            .sort_by(|left, right| left.body_ref.cmp(&right.body_ref));
        self.diagnostics.sort();
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_refs_have_stable_semantic_ordering() {
        let wall = BodyRef::assembly(ElementId::new("wall-a"), AssemblyKind::Wall);
        let member = BodyRef::member(ElementId::new("wall-a"), MemberKind::CommonStud, "stud-1");
        let later = BodyRef::member(ElementId::new("wall-b"), MemberKind::CommonStud, "stud-1");
        let mut refs = [later.clone(), wall.clone(), member.clone()];
        refs.sort();
        assert_eq!(refs, [member, later, wall]);
    }

    #[test]
    fn empty_scene_is_a_successful_stable_scene() {
        assert_eq!(PhysicalScene::default().finish(), PhysicalScene::default());
    }
}
