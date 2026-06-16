//! Builds a renderable [`Scene`] from a Framer [`BuildingModel`].
//!
//! Materials are auto-derived from the model: exterior walls get painted
//! cladding, interior walls get drywall, windows/skylights become glass, doors
//! become solid panels, and garage doors become painted metal. A ground plane
//! and a procedural sky + sun complete the scene. The camera is derived from an
//! orbit state so the render matches the interactive 3D view's vantage.

use framer_core::{BuildingModel, OpeningKind, Wall, WallExposure};

use crate::aabb::Aabb;
use crate::camera::Camera;
use crate::geom::Triangle;
use crate::material::Material;
use crate::math::Vec3;
use crate::scene::{DirectionalSun, Scene, Sky};

/// Material palette indices (the order returned by [`palette`]).
pub const MAT_CLADDING: u32 = 0;
pub const MAT_DRYWALL: u32 = 1;
pub const MAT_GLASS: u32 = 2;
pub const MAT_DOOR: u32 = 3;
pub const MAT_GARAGE: u32 = 4;
pub const MAT_GROUND: u32 = 5;

/// Knobs for the render. Camera fields come from the app's orbit state; lighting
/// and exposure have sensible architectural defaults.
#[derive(Clone, Copy, Debug)]
pub struct RenderOptions {
    pub yaw: f32,
    pub pitch: f32,
    pub zoom: f32,
    pub aspect: f32,
    pub vfov_deg: f32,
    pub exposure: f32,
    pub sun: DirectionalSun,
    pub sky: Sky,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            // Matches the app's View3dState::default().
            yaw: -std::f32::consts::FRAC_PI_4,
            pitch: 0.42,
            zoom: 1.0,
            aspect: 16.0 / 9.0,
            vfov_deg: 36.0,
            exposure: 1.0,
            sun: DirectionalSun {
                // Low, warm late-afternoon sun for long, dramatic shadows.
                dir: Vec3::new(0.55, -0.42, 0.58).normalize(),
                irradiance: Vec3::new(1.0, 0.9, 0.74) * 4.4,
                angular_radius: 0.025,
            },
            sky: Sky {
                // Deep blue zenith fading to a bright, slightly warm horizon.
                zenith: Vec3::new(0.14, 0.30, 0.72),
                horizon: Vec3::new(0.80, 0.84, 0.90),
                ground: Vec3::new(0.22, 0.20, 0.16),
            },
        }
    }
}

/// The auto-derived material palette, indexed by the `MAT_*` constants.
pub fn palette() -> Vec<Material> {
    vec![
        Material::Diffuse {
            albedo: Vec3::new(0.66, 0.62, 0.55), // cladding — warm light grey
        },
        Material::Diffuse {
            albedo: Vec3::new(0.86, 0.86, 0.83), // drywall — near white
        },
        Material::Dielectric {
            ior: 1.5,
            tint: Vec3::new(0.90, 0.95, 0.94), // glass — faint cool tint
        },
        Material::Diffuse {
            albedo: Vec3::new(0.34, 0.20, 0.11), // door — stained wood
        },
        Material::Diffuse {
            albedo: Vec3::new(0.74, 0.74, 0.76), // garage door — painted metal
        },
        Material::Diffuse {
            albedo: Vec3::new(0.30, 0.33, 0.26), // ground — muted lawn
        },
    ]
}

/// The orbit framing (pivot + bounding radius) implied by the model geometry.
/// Derived from the triangle bounds alone — independent of the view — so it can
/// be cached across camera moves and used to re-aim the camera without rebuilding
/// triangles and the BVH (see [`SceneFraming::camera`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneFraming {
    pub center: Vec3,
    pub radius: f32,
}

impl SceneFraming {
    /// The orbit camera for this framing under the given view options.
    pub fn camera(&self, opts: &RenderOptions) -> Camera {
        Camera::orbit(
            self.center,
            self.radius,
            opts.yaw,
            opts.pitch,
            opts.zoom,
            opts.aspect,
            opts.vfov_deg,
        )
    }
}

/// Builds the model's triangles and the orbit framing they imply. Geometry-only:
/// the result depends on `model` but not on the view (yaw/pitch/zoom/aspect), so
/// it is safe to cache across camera moves.
fn geometry_from_model(model: &BuildingModel) -> (Vec<Triangle>, SceneFraming) {
    let mut tris: Vec<Triangle> = Vec::new();
    let mut bounds = Aabb::EMPTY;

    for wall in &model.walls {
        push_wall(&mut tris, &mut bounds, model, wall);
    }

    // Ground plane: a large quad just below the lowest wall base.
    let (center, radius) = if bounds == Aabb::EMPTY {
        (Vec3::ZERO, 120.0)
    } else {
        (
            bounds.centroid(),
            (bounds.extent().length() * 0.5).max(12.0),
        )
    };
    let ground_z = if bounds == Aabb::EMPTY {
        0.0
    } else {
        bounds.min.z
    };
    push_ground(&mut tris, center, radius, ground_z);

    (tris, SceneFraming { center, radius })
}

/// Builds a render scene **and** its orbit framing. The framing lets an
/// interactive caller re-aim the camera on an orbit/zoom without rebuilding the
/// triangles + BVH (which are geometry-only and unchanged by a camera move).
pub fn build_scene(model: &BuildingModel, opts: &RenderOptions) -> (Scene, SceneFraming) {
    let (tris, framing) = geometry_from_model(model);
    let camera = framing.camera(opts);
    let scene = Scene::new(tris, palette(), opts.sun, opts.sky, camera, opts.exposure);
    (scene, framing)
}

/// Builds a render scene from the model. The result always contains a ground
/// plane and lighting even when the model has no walls.
pub fn scene_from_model(model: &BuildingModel, opts: &RenderOptions) -> Scene {
    build_scene(model, opts).0
}

/// Wall-local basis: `along` runs start→end, `side` is the perpendicular.
struct WallBasis {
    ox: f32,
    oy: f32,
    ax: f32,
    ay: f32,
    sx: f32,
    sy: f32,
}

impl WallBasis {
    fn new(wall: &Wall) -> Self {
        let ox = wall.start.x.inches() as f32;
        let oy = wall.start.y.inches() as f32;
        let dx = wall.end.x.inches() as f32 - ox;
        let dy = wall.end.y.inches() as f32 - oy;
        let len = (dx * dx + dy * dy).sqrt().max(1.0e-3);
        let ax = dx / len;
        let ay = dy / len;
        Self {
            ox,
            oy,
            ax,
            ay,
            sx: -ay,
            sy: ax,
        }
    }

    fn point(&self, local_x: f32, side: f32, z: f32) -> Vec3 {
        Vec3::new(
            self.ox + self.ax * local_x + self.sx * side,
            self.oy + self.ay * local_x + self.sy * side,
            z,
        )
    }
}

fn level_elevation(model: &BuildingModel, wall: &Wall) -> f32 {
    model
        .levels
        .iter()
        .find(|level| level.id.0 == wall.level.0)
        .map(|level| level.elevation.inches() as f32)
        .unwrap_or(0.0)
}

fn opening_panel_material(kind: OpeningKind) -> Option<u32> {
    match kind {
        OpeningKind::Window | OpeningKind::Skylight => Some(MAT_GLASS),
        OpeningKind::Door => Some(MAT_DOOR),
        OpeningKind::GarageDoor => Some(MAT_GARAGE),
        OpeningKind::Stair => None,
    }
}

fn push_wall(tris: &mut Vec<Triangle>, bounds: &mut Aabb, model: &BuildingModel, wall: &Wall) {
    let base = level_elevation(model, wall);
    let height = wall.height.inches() as f32;
    let length = wall.length.inches() as f32;
    let depth = wall.assembly.stud.nominal_depth().inches() as f32;
    let half = depth * 0.5;
    let basis = WallBasis::new(wall);
    let wall_mat = match wall.assembly.exposure {
        WallExposure::Exterior => MAT_CLADDING,
        WallExposure::Interior => MAT_DRYWALL,
    };

    // Track the wall's footprint for camera framing.
    for &(lx, sd, z) in &[
        (0.0, -half, base),
        (length, half, base + height),
        (0.0, half, base + height),
        (length, -half, base),
    ] {
        bounds.grow(basis.point(lx, sd, z));
    }

    let mut openings: Vec<_> = wall.openings.iter().collect();
    openings.sort_by_key(|o| o.left());

    let mut cursor = 0.0_f32;
    for opening in openings {
        let left = opening.left().inches() as f32;
        let right = opening.right().inches() as f32;
        let sill = opening.sill_height.inches() as f32;
        let top = opening.top().inches() as f32;

        // Solid wall before the opening.
        push_box(
            tris,
            &basis,
            cursor,
            left,
            half,
            base,
            base + height,
            wall_mat,
        );
        // Wall below the sill (windows).
        if sill > 0.0 {
            push_box(tris, &basis, left, right, half, base, base + sill, wall_mat);
        }
        // Wall above the opening (header band).
        if top < height {
            push_box(
                tris,
                &basis,
                left,
                right,
                half,
                base + top,
                base + height,
                wall_mat,
            );
        }
        // The opening's fill panel (glass / door / garage), thin and centered.
        if let Some(panel) = opening_panel_material(opening.kind) {
            let panel_half = (depth * 0.2).clamp(0.25, 0.75);
            push_box(
                tris,
                &basis,
                left,
                right,
                panel_half,
                base + sill,
                base + top,
                panel,
            );
        }
        cursor = right;
    }
    // Remaining solid wall after the last opening.
    push_box(
        tris,
        &basis,
        cursor,
        length,
        half,
        base,
        base + height,
        wall_mat,
    );
}

/// Pushes an axis-aligned box (in wall-local coords) as 12 triangles. The box
/// spans `[x0, x1]` along the wall, `[-half, half]` across its thickness, and
/// `[z0, z1]` vertically. Degenerate boxes are skipped.
#[allow(clippy::too_many_arguments)]
fn push_box(
    tris: &mut Vec<Triangle>,
    basis: &WallBasis,
    x0: f32,
    x1: f32,
    half: f32,
    z0: f32,
    z1: f32,
    material: u32,
) {
    if x1 - x0 <= 1.0e-4 || z1 - z0 <= 1.0e-4 {
        return;
    }
    let c = [
        basis.point(x0, -half, z0),
        basis.point(x1, -half, z0),
        basis.point(x1, half, z0),
        basis.point(x0, half, z0),
        basis.point(x0, -half, z1),
        basis.point(x1, -half, z1),
        basis.point(x1, half, z1),
        basis.point(x0, half, z1),
    ];
    const FACES: [[usize; 4]; 6] = [
        [0, 1, 2, 3],
        [4, 5, 6, 7],
        [0, 1, 5, 4],
        [1, 2, 6, 5],
        [2, 3, 7, 6],
        [3, 0, 4, 7],
    ];
    for f in FACES {
        tris.push(Triangle::new(c[f[0]], c[f[1]], c[f[2]], material));
        tris.push(Triangle::new(c[f[0]], c[f[2]], c[f[3]], material));
    }
}

fn push_ground(tris: &mut Vec<Triangle>, center: Vec3, radius: f32, z: f32) {
    let r = radius * 6.0 + 240.0;
    let cx = center.x;
    let cy = center.y;
    let corners = [
        Vec3::new(cx - r, cy - r, z),
        Vec3::new(cx + r, cy - r, z),
        Vec3::new(cx + r, cy + r, z),
        Vec3::new(cx - r, cy + r, z),
    ];
    tris.push(Triangle::new(
        corners[0], corners[1], corners[2], MAT_GROUND,
    ));
    tris.push(Triangle::new(
        corners[0], corners[2], corners[3], MAT_GROUND,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use framer_core::{CodeProfile, Length, Opening, Point2};

    fn material_histogram(scene: &Scene) -> std::collections::HashMap<u32, usize> {
        let mut h = std::collections::HashMap::new();
        for t in &scene.triangles {
            *h.entry(t.material).or_insert(0) += 1;
        }
        h
    }

    fn wall_model(exposure: WallExposure, openings: Vec<Opening>) -> BuildingModel {
        let code = CodeProfile::irc_2021_prescriptive();
        let mut model = BuildingModel::new(code.clone());
        let mut wall = Wall::new("wall-1", "Wall", Length::from_feet(12.0), &code);
        wall.start = Point2::new(Length::ZERO, Length::ZERO);
        wall.end = Point2::new(Length::from_feet(12.0), Length::ZERO);
        wall.assembly.exposure = exposure;
        wall.openings = openings;
        model.walls.push(wall);
        model
    }

    #[test]
    fn build_scene_matches_scene_from_model() {
        // The split build path must produce exactly what the public entry point does.
        let model = wall_model(WallExposure::Exterior, vec![]);
        let opts = RenderOptions {
            yaw: 0.7,
            pitch: 0.3,
            zoom: 1.5,
            aspect: 1.4,
            ..RenderOptions::default()
        };
        let (scene, _framing) = build_scene(&model, &opts);
        let direct = scene_from_model(&model, &opts);
        assert_eq!(scene.camera, direct.camera);
        assert_eq!(scene.triangles, direct.triangles);
        assert_eq!(scene.materials, direct.materials);
    }

    #[test]
    fn reaiming_camera_matches_full_rebuild() {
        // Cache geometry once for view A, then re-aim the camera to view B without
        // rebuilding triangles/BVH — the result must match a full rebuild at B.
        let model = wall_model(WallExposure::Exterior, vec![]);
        let view_a = RenderOptions {
            yaw: 0.3,
            pitch: 0.4,
            zoom: 1.0,
            aspect: 1.6,
            ..RenderOptions::default()
        };
        let view_b = RenderOptions {
            yaw: 1.1,
            pitch: 0.2,
            zoom: 2.0,
            aspect: 1.2,
            ..RenderOptions::default()
        };

        let (mut scene, framing) = build_scene(&model, &view_a);
        let tris_before = scene.triangles.clone();
        scene.camera = framing.camera(&view_b);

        let full = scene_from_model(&model, &view_b);
        assert_eq!(
            scene.camera, full.camera,
            "re-aimed camera must equal a full rebuild at the new view"
        );
        assert_eq!(
            scene.triangles, tris_before,
            "re-aiming must not disturb the cached geometry"
        );
        assert_eq!(
            scene.triangles, full.triangles,
            "cached geometry must equal a fresh build's geometry"
        );
    }

    #[test]
    fn empty_model_still_has_ground_and_camera() {
        let model = BuildingModel::new(CodeProfile::irc_2021_prescriptive());
        let scene = scene_from_model(&model, &RenderOptions::default());
        let hist = material_histogram(&scene);
        assert_eq!(hist.get(&MAT_GROUND).copied().unwrap_or(0), 2);
        assert!(scene.triangles.len() >= 2);
    }

    #[test]
    fn exterior_wall_uses_cladding_not_drywall() {
        let model = wall_model(WallExposure::Exterior, vec![]);
        let scene = scene_from_model(&model, &RenderOptions::default());
        let hist = material_histogram(&scene);
        assert!(hist.get(&MAT_CLADDING).copied().unwrap_or(0) >= 12);
        assert_eq!(hist.get(&MAT_DRYWALL).copied().unwrap_or(0), 0);
    }

    #[test]
    fn interior_wall_uses_drywall_not_cladding() {
        let model = wall_model(WallExposure::Interior, vec![]);
        let scene = scene_from_model(&model, &RenderOptions::default());
        let hist = material_histogram(&scene);
        assert!(hist.get(&MAT_DRYWALL).copied().unwrap_or(0) >= 12);
        assert_eq!(hist.get(&MAT_CLADDING).copied().unwrap_or(0), 0);
    }

    #[test]
    fn window_opening_becomes_glass() {
        let window = Opening::window(
            "w1",
            "Window",
            Length::from_feet(6.0),
            Length::from_feet(3.0),
            Length::from_feet(4.0),
            Length::from_feet(3.0),
        );
        let model = wall_model(WallExposure::Exterior, vec![window]);
        let scene = scene_from_model(&model, &RenderOptions::default());
        let hist = material_histogram(&scene);
        assert!(
            hist.get(&MAT_GLASS).copied().unwrap_or(0) > 0,
            "no glass emitted"
        );
        assert_eq!(hist.get(&MAT_DOOR).copied().unwrap_or(0), 0);
    }

    #[test]
    fn door_opening_becomes_solid_panel_not_glass() {
        let door = Opening::door(
            "d1",
            "Door",
            Length::from_feet(6.0),
            Length::from_feet(3.0),
            Length::from_feet(6.7),
        );
        let model = wall_model(WallExposure::Exterior, vec![door]);
        let scene = scene_from_model(&model, &RenderOptions::default());
        let hist = material_histogram(&scene);
        assert!(
            hist.get(&MAT_DOOR).copied().unwrap_or(0) > 0,
            "no door panel"
        );
        assert_eq!(hist.get(&MAT_GLASS).copied().unwrap_or(0), 0);
    }

    #[test]
    fn camera_frames_the_model_center() {
        let model = wall_model(WallExposure::Exterior, vec![]);
        let scene = scene_from_model(&model, &RenderOptions::default());
        // Wall runs 12ft along +x at y=0; center x should be ~6ft = 72in.
        assert!(
            (scene.camera.center.x - 72.0).abs() < 12.0,
            "center={:?}",
            scene.camera.center
        );
    }

    #[test]
    fn demo_shell_extracts_a_non_trivial_scene() {
        let model = BuildingModel::demo_shell();
        let scene = scene_from_model(&model, &RenderOptions::default());
        assert!(
            scene.triangles.len() > 50,
            "demo shell produced too few triangles: {}",
            scene.triangles.len()
        );
        // Geometry must be finite (no NaNs from degenerate walls).
        for t in &scene.triangles {
            assert!(t.v0.x.is_finite() && t.v0.y.is_finite() && t.v0.z.is_finite());
        }
    }
}
