//! Roof, ceiling, and floor surface emission and outline lifting.

use eframe::egui::Color32;
use framer_core::{BuildingModel, ElementId, Length, Point2, RoofPlane, SurfaceRegion};

use super::super::theme;
use super::style::{SurfaceFace, surface_color};
use super::{PickSolid, Point3, SceneBuilder, brighten, color_to_rgba};
use crate::app::{Selection, ViewClick};

enum SurfaceReverseFace {
    /// A cutaway/translucent presentation uses one face so alpha is applied once.
    Omit,
    /// Coincident reverse face with the same material (flat ceiling/deck).
    Same,
    /// Physically separated underside with its own material (finished roof).
    Distinct { color: Color32, verts: Vec<Point3> },
}

impl SceneBuilder {
    fn push_surface(
        &mut self,
        outline: &[Point3],
        triangles: &[[usize; 3]],
        color: Color32,
        reverse_face: SurfaceReverseFace,
        click: ViewClick,
        selected: bool,
    ) {
        if outline.len() < 3 {
            return;
        }
        let shade = |c: Color32| color_to_rgba(if selected { brighten(c, 30) } else { c });
        // Both faces share the same triangulation; the normal (uniform per face)
        // decides which way each lights, so winding need not be reversed.
        let up = polygon_normal(outline);
        self.push_face(outline, triangles, up, shade(color));
        match reverse_face {
            SurfaceReverseFace::Omit => {}
            // Flat ceiling/floor surfaces: both faces share one color, so the
            // coincident back face has no z-fight to resolve.
            SurfaceReverseFace::Same => self.push_face(outline, triangles, -up, shade(color)),
            // A cathedral roof underside: a distinct interior finish at the
            // springing/bearing plane. The weather face is already lifted clear,
            // avoiding z-fighting without pushing the underside into the wall.
            SurfaceReverseFace::Distinct {
                color: under_color,
                verts: under_verts,
            } => {
                self.push_face(&under_verts, triangles, -up, shade(under_color));
                self.points.extend_from_slice(&under_verts);
            }
        }
        self.points.extend_from_slice(outline);
        self.picks.push(PickSolid::surface(
            click,
            SURFACE_PICK_PRIORITY,
            outline.to_vec(),
        ));
    }
}

pub(super) fn push_ceiling_surfaces(
    builder: &mut SceneBuilder,
    model: &BuildingModel,
    selection: &Selection,
) {
    for ceiling in &model.ceilings {
        let Some(plan) = region_outline_plan(model, &ceiling.region) else {
            continue;
        };
        // The ceiling's low-edge building elevation: it hangs `height` below the
        // level top. A sloped (scissor/vault) ceiling lifts each plan vertex onto
        // its sloped plane via the shared frame, exactly like a roof plane; a flat
        // ceiling stays at a constant elevation.
        let reference_elevation = model
            .levels
            .iter()
            .find(|level| level.id == ceiling.level)
            .map(|level| level.elevation + level.height - ceiling.height)
            .unwrap_or(Length::ZERO);
        let verts = match ceiling.frame(reference_elevation) {
            Some(frame) => plan
                .iter()
                .map(|p| {
                    let (x, y) = (p.x.inches(), p.y.inches());
                    Point3::vector(x as f32, y as f32, frame.elevation_at(x, y) as f32)
                })
                .collect(),
            None => lift_outline(&plan, reference_elevation.inches() as f32),
        };
        let triangles = framer_core::triangulate_simple_polygon(&plan);
        let color = surface_color(model, &ceiling.system, SurfaceFace::Ceiling);
        let selected = matches!(selection, Selection::Ceiling(id) if id == &ceiling.id.0);
        builder.push_surface(
            &verts,
            &triangles,
            color,
            SurfaceReverseFace::Same,
            ViewClick::Ceiling {
                id: ceiling.id.0.clone(),
            },
            selected,
        );
    }
}

pub(super) fn push_floor_surfaces(
    builder: &mut SceneBuilder,
    model: &BuildingModel,
    selection: &Selection,
) {
    for deck in &model.floor_decks {
        let z = level_elevation(model, &deck.level);
        let Some(plan) = region_outline_plan(model, &deck.region) else {
            continue;
        };
        let verts = lift_outline(&plan, z);
        let triangles = framer_core::triangulate_simple_polygon(&plan);
        let color = surface_color(model, &deck.system, SurfaceFace::Floor);
        let selected = matches!(selection, Selection::FloorDeck(id) if id == &deck.id.0);
        builder.push_surface(
            &verts,
            &triangles,
            color,
            SurfaceReverseFace::Same,
            ViewClick::FloorDeck {
                id: deck.id.0.clone(),
            },
            selected,
        );
    }
}

const SURFACE_PICK_PRIORITY: u8 = 1;
/// Alpha of authored roof sheets in the generated Plan cutaway. They stay present
/// for assembly context and picking without hiding the opaque framing members.
const PLAN_ROOF_ALPHA: u8 = 88;

/// Emit every roof assembly through the model's shared overhang derivation. The
/// original bearing frame projects the expanded points, so an eave-tail vertex
/// falls below the springing elevation instead of redefining that datum.
pub(super) fn push_roof_surfaces(
    builder: &mut SceneBuilder,
    model: &BuildingModel,
    selection: &Selection,
    transparent: bool,
) {
    // Classify all planes in one wall-graph pass rather than resolving each Room
    // ceiling again per plane.
    let cathedral = model.roof_cathedral_flags();
    for (index, plane) in model.roof_planes.iter().enumerate() {
        let surface_outline = model.roof_surface_outline(plane);
        let Some(bearing_verts) = roof_plane_outline_world(plane, &surface_outline) else {
            continue;
        };
        let triangles = framer_core::triangulate_simple_polygon(&surface_outline);
        let apply_alpha = |color| {
            if transparent {
                theme::with_alpha(color, PLAN_ROOF_ALPHA)
            } else {
                color
            }
        };
        let color = apply_alpha(surface_color(model, &plane.system, SurfaceFace::Roof));
        // The authored frame is the bearing/underside face. Lift only the weather
        // face by assembly thickness so the two remain separated.
        let lift = roof_assembly_lift(model, &plane.system);
        let weather_verts = lift_roof_face(&bearing_verts, lift);
        let underside_color = if cathedral.get(index).copied().unwrap_or(false) {
            surface_color(model, &plane.system, SurfaceFace::RoofUnderside)
        } else {
            surface_color(model, &plane.system, SurfaceFace::Roof)
        };
        let reverse_face = if transparent {
            SurfaceReverseFace::Omit
        } else {
            SurfaceReverseFace::Distinct {
                color: underside_color,
                verts: bearing_verts,
            }
        };
        let selected = matches!(selection, Selection::RoofPlane(id) if id == &plane.id.0);
        builder.push_surface(
            &weather_verts,
            &triangles,
            color,
            reverse_face,
            ViewClick::RoofPlane {
                id: plane.id.0.clone(),
            },
            selected,
        );
    }
}

/// Vertical lift from a roof plane's bearing/underside face to the weather face:
/// the assembly's through-thickness (a default when the system is missing).
/// Separates the faces so the underside reads from inside while the weather face
/// reads from outside, without drawing the underside below the wall top.
fn roof_assembly_lift(model: &BuildingModel, system_id: &ElementId) -> f32 {
    /// Nominal lift when no system resolves a real thickness (≈ a 2×6 roof).
    const DEFAULT_LIFT_IN: f32 = 6.0;
    model
        .systems
        .iter()
        .find(|system| system.id == *system_id)
        .map(|system| system.total_thickness().inches() as f32)
        .filter(|lift| *lift > 0.0)
        .unwrap_or(DEFAULT_LIFT_IN)
}

/// Lift a sloped roof face vertically while preserving its plan outline and
/// slope. The model's roof plane is the springing/bearing face; this derives the
/// visible weather face above it.
fn lift_roof_face(outline: &[Point3], lift: f32) -> Vec<Point3> {
    outline
        .iter()
        .map(|p| Point3::vector(p.x, p.y, p.z + lift))
        .collect()
}

/// A level's floor elevation (inches), or 0 when the level is missing.
pub(super) fn level_elevation(model: &BuildingModel, level_id: &ElementId) -> f32 {
    model
        .levels
        .iter()
        .find(|level| level.id == *level_id)
        .map(|level| level.elevation.inches() as f32)
        .unwrap_or(0.0)
}

/// A ceiling/floor-deck region's closed plan outline. `Room` regions resolve
/// through the wall graph (mirroring the solver), so the drawn surface tracks the
/// same enclosed face the joists frame; an unknown room or an open (mid-edit) loop
/// yields `None` and the surface is simply skipped.
pub(super) fn region_outline_plan(
    model: &BuildingModel,
    region: &SurfaceRegion,
) -> Option<Vec<Point2>> {
    let outline = match region {
        SurfaceRegion::Polygon(points) => points.clone(),
        SurfaceRegion::Room(room_id) => {
            let room = model.rooms.iter().find(|room| room.id == *room_id)?;
            framer_core::room_boundary_on_level(model, &room.level, room.seed)?.vertices
        }
    };
    (outline.len() >= 3).then_some(outline)
}

/// Lift a plan outline to constant elevation `z` (a flat ceiling/floor surface).
fn lift_outline(outline: &[Point2], z: f32) -> Vec<Point3> {
    outline
        .iter()
        .map(|point| Point3::vector(point.x.inches() as f32, point.y.inches() as f32, z))
        .collect()
}

/// A roof plane's plan outline lifted onto its sloped plane via the shared
/// [`framer_core::RoofPlaneFrame`] — the same affine elevation field the solver's
/// framing and the path-traced render use, so the slab lies on exactly the plane
/// the rafters frame. `None` for a degenerate outline (no eave length).
fn roof_plane_outline_world(plane: &RoofPlane, outline: &[Point2]) -> Option<Vec<Point3>> {
    let frame = plane.frame()?;
    Some(
        outline
            .iter()
            .map(|p| {
                let (x, y) = (p.x.inches(), p.y.inches());
                Point3::vector(x as f32, y as f32, frame.elevation_at(x, y) as f32)
            })
            .collect(),
    )
}

/// The unit normal of a planar polygon (Newell's method), oriented upward (+z) so
/// a surface's top face faces the sky. Falls back to +Z for a degenerate polygon.
fn polygon_normal(verts: &[Point3]) -> Point3 {
    let n = verts.len();
    let (mut nx, mut ny, mut nz) = (0.0_f32, 0.0_f32, 0.0_f32);
    for i in 0..n {
        let a = verts[i];
        let b = verts[(i + 1) % n];
        nx += (a.y - b.y) * (a.z + b.z);
        ny += (a.z - b.z) * (a.x + b.x);
        nz += (a.x - b.x) * (a.y + b.y);
    }
    let length = (nx * nx + ny * ny + nz * nz).sqrt();
    if length <= f32::EPSILON {
        return Point3::Z;
    }
    let sign = if nz < 0.0 { -1.0 } else { 1.0 };
    Point3::vector(sign * nx / length, sign * ny / length, sign * nz / length)
}
