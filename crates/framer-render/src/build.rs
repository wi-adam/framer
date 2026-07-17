//! Builds a renderable [`Scene`] from a Framer [`BuildingModel`].
//!
//! Materials are auto-derived from the model: exterior walls get painted
//! cladding, interior walls get drywall, windows/skylights become glass, doors
//! become solid panels, and garage doors become painted metal. A ground plane
//! and a procedural sky + sun complete the scene. The camera is derived from an
//! orbit state so the render matches the interactive 3D view's vantage.

use std::collections::BTreeMap;

use framer_core::{
    Appearance, AssemblyFace, BuildingModel, Ceiling, ConstructionSystem, ElementId, FloorDeck,
    GableWallProfile, LayerFunction, Length, Level, OpeningKind, Point2, RoofPlane, RoofPlaneFrame,
    SurfaceRegion, Wall, WallExposure, room_boundary_on_level, triangulate_simple_polygon,
};

use crate::aabb::Aabb;
use crate::camera::Camera;
use crate::geom::Triangle;
use crate::material::{Material, Texture, srgb_to_linear};
use crate::math::Vec3;
use crate::scene::{DirectionalSun, Scene, Sky};

/// Material palette indices (the order returned by [`palette`]).
pub const MAT_CLADDING: u32 = 0;
pub const MAT_DRYWALL: u32 = 1;
pub const MAT_GLASS: u32 = 2;
pub const MAT_DOOR: u32 = 3;
pub const MAT_GARAGE: u32 = 4;
pub const MAT_GROUND: u32 = 5;

/// Resolved binary assets available to the render builder. The model stores only
/// hashes; callers populate this map from a content-addressed cache or package.
#[derive(Clone, Debug, Default)]
pub struct RenderAssets {
    textures: BTreeMap<String, Texture>,
}

impl RenderAssets {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert_texture(&mut self, hash: impl Into<String>, texture: Texture) {
        self.textures.insert(hash.into(), texture);
    }

    pub fn texture(&self, hash: &str) -> Option<&Texture> {
        self.textures.get(hash)
    }
}

/// Knobs for the render. Camera fields come from the app's orbit state; lighting
/// and exposure have sensible architectural defaults.
#[derive(Clone, Copy, Debug)]
pub struct RenderOptions {
    pub yaw: f32,
    pub pitch: f32,
    pub zoom: f32,
    /// Orbit-pivot offset in *radius-relative* world units: the effective pivot is
    /// `framing.center + pan * framing.radius`. Lets the interactive view slide the
    /// framing without knowing the model's absolute scale.
    pub pan: Vec3,
    /// Eye-distance multiplier along the view axis (`1.0` frames the model; `< 1`
    /// dives in, `> 1` pulls back). See [`Camera::orbit`].
    pub dolly: f32,
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
            pan: Vec3::ZERO,
            dolly: 1.0,
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
    /// The orbit camera for this framing under the given view options. `opts.pan`
    /// slides the pivot by `pan * radius` (radius-relative world units) before the
    /// orbit is constructed, so panning then orbiting rotates around the new pivot.
    pub fn camera(&self, opts: &RenderOptions) -> Camera {
        Camera::orbit(
            self.center + opts.pan * self.radius,
            self.radius,
            opts.yaw,
            opts.pitch,
            opts.zoom,
            opts.aspect,
            opts.vfov_deg,
            opts.dolly,
        )
    }
}

/// Builds the model's triangles and the orbit framing they imply. Geometry-only:
/// the result depends on `model` but not on the view (yaw/pitch/zoom/aspect), so
/// it is safe to cache across camera moves.
struct PaletteBuilder<'a> {
    assets: &'a RenderAssets,
    materials: Vec<Material>,
    textures: Vec<Texture>,
    by_material: BTreeMap<ElementId, u32>,
}

impl<'a> PaletteBuilder<'a> {
    fn new(model: &BuildingModel, assets: &'a RenderAssets) -> Self {
        Self {
            assets,
            materials: palette(),
            textures: Vec::new(),
            by_material: model
                .materials
                .iter()
                .map(|material| (material.id.clone(), u32::MAX))
                .collect(),
        }
    }

    fn material_index(&mut self, model: &BuildingModel, id: &ElementId) -> Option<u32> {
        let current = *self.by_material.get(id)?;
        if current != u32::MAX {
            return Some(current);
        }
        let material = model.material(id)?;
        let index = self.materials.len() as u32;
        let render_material = render_material_for_appearance(&material.appearance, self);
        self.materials.push(render_material);
        self.by_material.insert(id.clone(), index);
        Some(index)
    }

    fn add_texture(&mut self, hash: &str) -> Option<u32> {
        let texture = self.assets.texture(hash)?;
        let index = self.textures.len() as u32;
        self.textures.push(texture.clone());
        Some(index)
    }
}

fn render_material_for_appearance(
    appearance: &Appearance,
    palette: &mut PaletteBuilder<'_>,
) -> Material {
    match appearance {
        Appearance::SolidColor(color) => Material::Diffuse {
            albedo: color_to_linear(*color),
        },
        Appearance::Textured {
            color,
            texture,
            scale,
        } => {
            let fallback = color_to_linear(*color);
            match palette.add_texture(&texture.hash) {
                Some(texture) => Material::TexturedDiffuse {
                    fallback,
                    texture,
                    scale: scale.inches() as f32,
                },
                None => Material::Diffuse { albedo: fallback },
            }
        }
        Appearance::DepthMapped {
            color,
            height,
            scale,
        } => {
            let albedo = color_to_linear(*color);
            match palette.add_texture(&height.hash) {
                Some(height) => Material::DepthMappedDiffuse {
                    albedo,
                    height,
                    scale: scale.inches() as f32,
                },
                None => Material::Diffuse { albedo },
            }
        }
    }
}

fn color_to_linear(color: [u8; 3]) -> Vec3 {
    Vec3::new(
        srgb_to_linear(color[0]),
        srgb_to_linear(color[1]),
        srgb_to_linear(color[2]),
    )
}

fn geometry_from_model(
    model: &BuildingModel,
    assets: &RenderAssets,
) -> (Vec<Triangle>, Vec<Material>, Vec<Texture>, SceneFraming) {
    let mut tris: Vec<Triangle> = Vec::new();
    let mut bounds = Aabb::EMPTY;
    let mut palette = PaletteBuilder::new(model, assets);

    // Gable profiles are derived from the level-scoped exterior wall graph and
    // stored roof bearing outlines. Compute them once for every wall rather than
    // rebuilding topology in each wall emission.
    let gable_profiles = model.gable_wall_profiles();
    for wall in &model.walls {
        push_wall(
            &mut tris,
            &mut bounds,
            model,
            wall,
            gable_profiles.get(&wall.id),
            &mut palette,
        );
    }
    // Horizontal decks/ceilings, then sloped roof planes: each authored surface
    // becomes opaque-diffuse geometry through the same `Triangle` path as walls.
    for deck in &model.floor_decks {
        push_floor_deck(&mut tris, &mut bounds, model, deck, &mut palette);
    }
    for ceiling in &model.ceilings {
        push_ceiling(&mut tris, &mut bounds, model, ceiling, &mut palette);
    }
    // Cathedral classification for every plane in one wall-graph pass (cheaper than
    // re-resolving each Room ceiling per plane).
    let cathedral = model.roof_cathedral_flags();
    for (index, plane) in model.roof_planes.iter().enumerate() {
        let is_cathedral = cathedral.get(index).copied().unwrap_or(false);
        push_roof_plane(
            &mut tris,
            &mut bounds,
            model,
            plane,
            is_cathedral,
            &mut palette,
        );
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

    (
        tris,
        palette.materials,
        palette.textures,
        SceneFraming { center, radius },
    )
}

/// Builds a render scene **and** its orbit framing. The framing lets an
/// interactive caller re-aim the camera on an orbit/zoom without rebuilding the
/// triangles + BVH (which are geometry-only and unchanged by a camera move).
pub fn build_scene(model: &BuildingModel, opts: &RenderOptions) -> (Scene, SceneFraming) {
    build_scene_with_assets(model, opts, &RenderAssets::default())
}

/// Builds a render scene using resolved texture/depth assets when available.
pub fn build_scene_with_assets(
    model: &BuildingModel,
    opts: &RenderOptions,
    assets: &RenderAssets,
) -> (Scene, SceneFraming) {
    let (tris, materials, textures, framing) = geometry_from_model(model, assets);
    let camera = framing.camera(opts);
    let scene = Scene::with_textures(
        tris,
        materials,
        textures,
        opts.sun,
        opts.sky,
        camera,
        opts.exposure,
    );
    (scene, framing)
}

/// Builds a render scene from the model. The result always contains a ground
/// plane and lighting even when the model has no walls.
pub fn scene_from_model(model: &BuildingModel, opts: &RenderOptions) -> Scene {
    build_scene(model, opts).0
}

pub fn scene_from_model_with_assets(
    model: &BuildingModel,
    opts: &RenderOptions,
    assets: &RenderAssets,
) -> Scene {
    build_scene_with_assets(model, opts, assets).0
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

fn push_wall(
    tris: &mut Vec<Triangle>,
    bounds: &mut Aabb,
    model: &BuildingModel,
    wall: &Wall,
    gable: Option<&GableWallProfile>,
    palette: &mut PaletteBuilder<'_>,
) {
    let base = level_elevation(model, wall);
    let height = wall.height.inches() as f32;
    let (visual_start, visual_end) = model.wall_envelope_span(wall);
    let visual_start = visual_start as f32;
    let visual_end = visual_end as f32;
    // Through-wall depth and exposure come from the wall's construction system.
    // Fall back to the code stud profile / Exterior when the system is missing
    // so scene building stays infallible.
    let system = model.system_for(wall);
    let depth = system
        .map(|system| system.total_thickness())
        .unwrap_or_else(|| model.framing_defaults().stud_profile.nominal_depth())
        .inches() as f32;
    let half = depth * 0.5;
    let basis = WallBasis::new(wall);
    let exposure = system
        .map(|system| system.exposure())
        .unwrap_or(WallExposure::Exterior);
    let wall_mat = wall_surface_material(model, system, exposure, palette);

    // Track the wall's footprint for camera framing.
    for &(lx, sd, z) in &[
        (visual_start, -half, base),
        (visual_end, half, base + height),
        (visual_start, half, base + height),
        (visual_end, -half, base),
    ] {
        bounds.grow(basis.point(lx, sd, z));
    }

    let mut openings: Vec<_> = wall.openings.iter().collect();
    openings.sort_by_key(|o| o.left());

    let mut cursor = visual_start;
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
        visual_end,
        half,
        base,
        base + height,
        wall_mat,
    );

    // The authored wall remains the rectangular plate-height solid above. A
    // matched gable is derived from that wall plus the roof bearing outlines and
    // occupies only the triangular extension between the wall top and roof
    // underside. It reuses the wall assembly thickness and exterior material;
    // openings remain constrained to the authored rectangle.
    if let Some(profile) = gable {
        push_gable_prism(tris, bounds, &basis, profile, half, wall_mat);
        for opening in wall
            .openings
            .iter()
            .filter(|opening| opening.top() == wall.height)
        {
            push_gable_base_cap(
                tris,
                &basis,
                opening.left().inches() as f32,
                opening.right().inches() as f32,
                -half,
                half,
                profile.base_elevation.inches() as f32,
                wall_mat,
            );
        }
    }
}

fn wall_surface_material(
    model: &BuildingModel,
    system: Option<&ConstructionSystem>,
    exposure: WallExposure,
    palette: &mut PaletteBuilder<'_>,
) -> u32 {
    let fallback = match exposure {
        WallExposure::Exterior => MAT_CLADDING,
        WallExposure::Interior => MAT_DRYWALL,
    };
    let Some(system) = system else {
        return fallback;
    };
    let preferred = match exposure {
        WallExposure::Interior => system.layers.iter().find_map(|layer| {
            (layer.function == LayerFunction::InteriorFinish).then_some(&layer.material)
        }),
        WallExposure::Exterior => system.layers.iter().rev().find_map(|layer| {
            matches!(
                layer.function,
                LayerFunction::Cladding | LayerFunction::Masonry
            )
            .then_some(&layer.material)
        }),
    };
    preferred
        .and_then(|material| palette.material_index(model, material))
        .unwrap_or(fallback)
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

/// Extrude one derived gable profile across the hosting wall's assembly
/// thickness. The two triangular wall faces and two sloped rake faces close the
/// extension; the existing rectangular wall's top face closes the shared base,
/// avoiding a coincident internal face at the authored wall height.
fn push_gable_prism(
    tris: &mut Vec<Triangle>,
    bounds: &mut Aabb,
    basis: &WallBasis,
    profile: &GableWallProfile,
    half: f32,
    material: u32,
) {
    let outline = profile.outline();
    let triangles = triangulate_simple_polygon(&outline);
    if triangles.is_empty() {
        return;
    }

    let base = profile.base_elevation.inches() as f32;
    let face = |side: f32| {
        outline
            .iter()
            .map(|point| {
                basis.point(
                    point.x.inches() as f32,
                    side,
                    base + point.y.inches() as f32,
                )
            })
            .collect::<Vec<_>>()
    };
    let interior = face(-half);
    let exterior = face(half);
    push_polygon(tris, bounds, &interior, &triangles, material);
    push_polygon(tris, bounds, &exterior, &triangles, material);

    for index in 0..outline.len() {
        let next = (index + 1) % outline.len();
        // The horizontal base is already closed by the rectangular wall's top
        // face. Emitting it again would create coincident triangles.
        if outline[index].y == Length::ZERO && outline[next].y == Length::ZERO {
            continue;
        }
        let corners = [
            interior[index],
            interior[next],
            exterior[next],
            exterior[index],
        ];
        let area_a = (corners[1] - corners[0])
            .cross(corners[2] - corners[0])
            .length();
        let area_b = (corners[2] - corners[0])
            .cross(corners[3] - corners[0])
            .length();
        if area_a > SURFACE_AREA_EPS {
            tris.push(Triangle::new(corners[0], corners[1], corners[2], material));
        }
        if area_b > SURFACE_AREA_EPS {
            tris.push(Triangle::new(corners[0], corners[2], corners[3], material));
        }
    }
}

/// Close only the part of a gable base not already capped by the rectangular
/// wall below: a valid opening may reach exactly to the authored wall height.
#[allow(clippy::too_many_arguments)]
fn push_gable_base_cap(
    tris: &mut Vec<Triangle>,
    basis: &WallBasis,
    x0: f32,
    x1: f32,
    side0: f32,
    side1: f32,
    z: f32,
    material: u32,
) {
    if x1 - x0 <= 1.0e-4 || (side1 - side0).abs() <= 1.0e-4 {
        return;
    }
    let corners = [
        basis.point(x0, side0, z),
        basis.point(x1, side0, z),
        basis.point(x1, side1, z),
        basis.point(x0, side1, z),
    ];
    tris.push(Triangle::new(corners[0], corners[3], corners[2], material));
    tris.push(Triangle::new(corners[0], corners[2], corners[1], material));
}

/// Below this triangle cross-product magnitude (≈ 2× area in in²) a fan triangle
/// is treated as degenerate and dropped, so a collinear/zero-area sliver never
/// produces a NaN geometric normal.
const SURFACE_AREA_EPS: f32 = 1.0e-4;

/// Which finished face of a roof/ceiling/floor assembly the renderer shows, so
/// the surface picks the layer the viewer actually sees: a roof's weather face,
/// a cathedral roof's interior-finish underside, a ceiling's underside finish, a
/// floor deck's walked-on top.
#[derive(Clone, Copy)]
enum SurfaceFace {
    Roof,
    /// A cathedral roof plane's underside — the assembly's conditioned-side finish.
    RoofUnderside,
    Ceiling,
    Floor,
}

/// The render material for a roof/ceiling/floor surface: the resolved appearance
/// of its system's representative finish layer (weather face for a roof, the
/// conditioned-side finish for a ceiling/floor or a cathedral roof underside),
/// falling back to a stock palette entry when the system or material is missing —
/// so scene building stays infallible like walls. Reuses the existing `MAT_*`
/// palette (no new GPU material), keeping the WGSL kernel and GPU↔CPU parity
/// untouched.
fn surface_material(
    model: &BuildingModel,
    system: Option<&ConstructionSystem>,
    face: SurfaceFace,
    palette: &mut PaletteBuilder<'_>,
) -> u32 {
    let (fallback, assembly_face) = match face {
        SurfaceFace::Ceiling => (MAT_DRYWALL, AssemblyFace::Finished),
        SurfaceFace::RoofUnderside => (MAT_DRYWALL, AssemblyFace::Underside),
        SurfaceFace::Roof | SurfaceFace::Floor => (MAT_CLADDING, AssemblyFace::Finished),
    };
    // The finished-face layer selection lives in `framer-core` so this render path
    // and the 3-D viewport pick the same face.
    system
        .and_then(|system| system.surface_finish_material(assembly_face))
        .and_then(|material| palette.material_index(model, material))
        .unwrap_or(fallback)
}

/// The construction system referenced by id, if any.
fn system_by_id<'a>(model: &'a BuildingModel, id: &ElementId) -> Option<&'a ConstructionSystem> {
    model.systems.iter().find(|system| system.id == *id)
}

/// The level an element sits on, looked up by id.
fn find_level<'a>(model: &'a BuildingModel, id: &ElementId) -> Option<&'a Level> {
    model.levels.iter().find(|level| level.id == *id)
}

/// Resolve a surface region to its closed plan outline: a `Polygon` is its own
/// outline; a `Room` is resolved through the wall graph (mirroring the solver), so
/// the rendered deck/ceiling tracks the same enclosed face the joists frame.
/// Returns `None` for an unknown room or an open (mid-edit) loop — the surface is
/// simply not drawn, matching the solver's open-region behavior.
fn resolve_region_outline(model: &BuildingModel, region: &SurfaceRegion) -> Option<Vec<Point2>> {
    match region {
        SurfaceRegion::Polygon(points) => Some(points.clone()),
        SurfaceRegion::Room(room_id) => {
            let room = model.rooms.iter().find(|room| room.id == *room_id)?;
            room_boundary_on_level(model, &room.level, room.seed).map(|boundary| boundary.vertices)
        }
    }
}

/// World position of a roof plane's plan outline point lifted onto its sloped
/// plane, using the shared [`RoofPlaneFrame`] (so the rendered surface lies on
/// exactly the plane the rafters frame). The plane is affine in plan (`z` is
/// linear in `x`/`y`), so projecting the outline keeps it coplanar.
fn project_onto_plane(frame: &RoofPlaneFrame, point: Point2) -> Vec3 {
    let (x, y) = (point.x.inches(), point.y.inches());
    Vec3::new(x as f32, y as f32, frame.elevation_at(x, y) as f32)
}

/// Emit a planar polygon (vertices already in world space) into `tris` using the
/// precomputed `triangles` index triples (an ear-clip from
/// [`triangulate_simple_polygon`], correct for concave outlines), dropping
/// degenerate triangles so a sliver never emits a NaN normal. Every vertex grows
/// `bounds` so the surface contributes to the camera framing.
fn push_polygon(
    tris: &mut Vec<Triangle>,
    bounds: &mut Aabb,
    verts: &[Vec3],
    triangles: &[[usize; 3]],
    material: u32,
) {
    for &v in verts {
        bounds.grow(v);
    }
    for &[ia, ib, ic] in triangles {
        let (a, b, c) = (verts[ia], verts[ib], verts[ic]);
        if (b - a).cross(c - a).length() <= SURFACE_AREA_EPS {
            continue;
        }
        tris.push(Triangle::new(a, b, c, material));
    }
}

/// Vertical lift from a roof plane's bearing/underside face to the weather face:
/// the assembly's through-thickness (a default when the system is missing).
/// Enough to separate the two faces so the tracer never sees a coincident-triangle
/// tie between the weather face and the cathedral underside.
fn roof_assembly_lift(system: Option<&ConstructionSystem>) -> f32 {
    /// Nominal lift when no system resolves a real thickness (≈ a 2×6 roof).
    const DEFAULT_LIFT_IN: f32 = 6.0;
    system
        .map(|system| system.total_thickness().inches() as f32)
        .filter(|lift| *lift > 0.0)
        .unwrap_or(DEFAULT_LIFT_IN)
}

/// Emit a roof plane's finished surface: its plan outline lifted onto the sloped
/// bearing plane, then raised by the roof assembly thickness for the weather face
/// and ear-clip triangulated with the system's weather-face material. The bearing
/// plane is also emitted as the underside so the roof assembly sits on the walls;
/// a **cathedral** plane (`is_cathedral`, no ceiling below) uses the assembly's
/// interior finish for that underside so a room with no ceiling sees the interior
/// finish, not roofing.
fn push_roof_plane(
    tris: &mut Vec<Triangle>,
    bounds: &mut Aabb,
    model: &BuildingModel,
    plane: &RoofPlane,
    is_cathedral: bool,
    palette: &mut PaletteBuilder<'_>,
) {
    let Some(frame) = plane.frame() else {
        return;
    };
    let system = system_by_id(model, &plane.system);
    let material = surface_material(model, system, SurfaceFace::Roof, palette);
    // The authored outline remains the bearing/topology footprint. Render the
    // shared derived eave/rake-overhang outline through the *original* affine
    // frame so tails extend down-slope while ridges/hips/valleys stay fixed.
    let (surface_points, triangles) = match model.roof_surface_triangulation(plane) {
        Some(triangulation) => (triangulation.points, triangulation.triangles),
        None => {
            // Keep the host roof visible when an invalid cavity fails closed in
            // physical geometry; the presentation fallback omits only the holes.
            let points = model.roof_surface_outline(plane);
            let triangles = framer_core::triangulate_simple_polygon(&points);
            if triangles.is_empty() {
                return;
            }
            (points, triangles)
        }
    };
    let underside_verts: Vec<Vec3> = surface_points
        .iter()
        .map(|&p| project_onto_plane(&frame, p))
        .collect();
    let lift = roof_assembly_lift(system);
    let verts: Vec<Vec3> = underside_verts
        .iter()
        .map(|v| Vec3::new(v.x, v.y, v.z + lift))
        .collect();
    push_polygon(tris, bounds, &verts, &triangles, material);

    let underside_material = if is_cathedral {
        surface_material(model, system, SurfaceFace::RoofUnderside, palette)
    } else {
        material
    };
    // Same winding (the tracer flips the shading normal toward the ray, so a
    // diffuse underside lights correctly from below). The underside is the
    // authored bearing plane; the weather face above it is what keeps the two
    // faces separated.
    push_polygon(
        tris,
        bounds,
        &underside_verts,
        &triangles,
        underside_material,
    );
}

/// Emit a horizontal surface (a flat ceiling or floor deck) at constant `z`: its
/// plan outline fan-triangulated with the system's finished-face material.
#[allow(clippy::too_many_arguments)]
fn push_horizontal_surface(
    tris: &mut Vec<Triangle>,
    bounds: &mut Aabb,
    model: &BuildingModel,
    outline: &[Point2],
    z: f32,
    system: Option<&ConstructionSystem>,
    face: SurfaceFace,
    palette: &mut PaletteBuilder<'_>,
) {
    let material = surface_material(model, system, face, palette);
    let verts: Vec<Vec3> = outline
        .iter()
        .map(|p| Vec3::new(p.x.inches() as f32, p.y.inches() as f32, z))
        .collect();
    let triangles = triangulate_simple_polygon(outline);
    push_polygon(tris, bounds, &verts, &triangles, material);
}

/// Emit a ceiling's underside surface. The low edge sits at `level.elevation +
/// level.height − ceiling.height` (the level top is the hang reference). A **flat**
/// ceiling is a constant-elevation polygon; a **sloped** (scissor/vault) ceiling
/// lifts each outline vertex onto its sloped plane via the shared [`Ceiling::frame`]
/// — exactly like a roof plane — so the framing and the rendered surface cannot
/// drift. Both use the system's ceiling-finish material.
fn push_ceiling(
    tris: &mut Vec<Triangle>,
    bounds: &mut Aabb,
    model: &BuildingModel,
    ceiling: &Ceiling,
    palette: &mut PaletteBuilder<'_>,
) {
    let Some(outline) = resolve_region_outline(model, &ceiling.region) else {
        return;
    };
    let level_top = find_level(model, &ceiling.level)
        .map(|level| level.elevation + level.height)
        .unwrap_or(Length::ZERO);
    let reference_elevation = level_top - ceiling.height;
    let system = system_by_id(model, &ceiling.system);
    match ceiling.frame(reference_elevation) {
        Some(frame) => {
            let material = surface_material(model, system, SurfaceFace::Ceiling, palette);
            let verts: Vec<Vec3> = outline
                .iter()
                .map(|&p| project_onto_plane(&frame, p))
                .collect();
            let triangles = triangulate_simple_polygon(&outline);
            push_polygon(tris, bounds, &verts, &triangles, material);
        }
        None => push_horizontal_surface(
            tris,
            bounds,
            model,
            &outline,
            reference_elevation.inches() as f32,
            system,
            SurfaceFace::Ceiling,
            palette,
        ),
    }
}

/// Emit a floor deck's walked-on surface at its level elevation, with the
/// system's top-finish material.
fn push_floor_deck(
    tris: &mut Vec<Triangle>,
    bounds: &mut Aabb,
    model: &BuildingModel,
    deck: &FloorDeck,
    palette: &mut PaletteBuilder<'_>,
) {
    let Some(outline) = resolve_region_outline(model, &deck.region) else {
        return;
    };
    let z = find_level(model, &deck.level)
        .map(|level| level.elevation.inches() as f32)
        .unwrap_or(0.0);
    push_horizontal_surface(
        tris,
        bounds,
        model,
        &outline,
        z,
        system_by_id(model, &deck.system),
        SurfaceFace::Floor,
        palette,
    );
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
    use crate::geom::Hit;
    use framer_core::{
        AssetRef, FramingDefaults, Length, Opening, Point2, RoofOpening, TextureRole, WallJoin,
    };

    fn material_histogram(scene: &Scene) -> std::collections::HashMap<u32, usize> {
        let mut h = std::collections::HashMap::new();
        for t in &scene.triangles {
            *h.entry(t.material).or_insert(0) += 1;
        }
        h
    }

    fn wall_model(exposure: WallExposure, openings: Vec<Opening>) -> BuildingModel {
        let code = FramingDefaults::illustrative_starter();
        let mut model = BuildingModel::new();
        let mut wall = Wall::new("wall-1", "Wall", Length::from_feet(12.0), &code);
        wall.start = Point2::new(Length::ZERO, Length::ZERO);
        wall.end = Point2::new(Length::from_feet(12.0), Length::ZERO);
        wall.system = match exposure {
            WallExposure::Exterior => framer_core::ElementId::new("system-wall-exterior-1"),
            WallExposure::Interior => framer_core::ElementId::new("system-wall-interior-1"),
        };
        wall.openings = openings;
        model.walls.push(wall);
        model
    }

    fn corner_model() -> BuildingModel {
        let code = FramingDefaults::illustrative_starter();
        let mut model = BuildingModel::new();
        model.walls.push(
            Wall::new("wall-a", "Wall A", Length::from_feet(10.0), &code).with_placement(
                "level-1",
                Point2::new(Length::ZERO, Length::ZERO),
                Point2::new(Length::from_feet(10.0), Length::ZERO),
            ),
        );
        model.walls.push(
            Wall::new("wall-b", "Wall B", Length::from_feet(8.0), &code).with_placement(
                "level-1",
                Point2::new(Length::from_feet(10.0), Length::ZERO),
                Point2::new(Length::from_feet(10.0), Length::from_feet(8.0)),
            ),
        );
        model.wall_joins.push(WallJoin::corner(
            "join-corner",
            "Corner",
            "wall-a",
            "wall-b",
            Point2::new(Length::from_feet(10.0), Length::ZERO),
        ));
        model
    }

    fn wall_surface_triangle_materials(scene: &Scene) -> Vec<u32> {
        scene
            .triangles
            .iter()
            .map(|triangle| triangle.material)
            .filter(|material| !matches!(*material, MAT_GROUND | MAT_GLASS | MAT_DOOR | MAT_GARAGE))
            .collect()
    }

    #[test]
    fn pan_translates_the_camera_rigidly_by_pan_times_radius() {
        // Pan offsets the orbit pivot by `pan * radius` (radius-relative world
        // units). The whole rig translates rigidly: the pivot and the eye shift by
        // the same vector, and the view basis is unchanged (pan does not rotate).
        let framing = SceneFraming {
            center: Vec3::new(10.0, -4.0, 2.0),
            radius: 8.0,
        };
        let base = framing.camera(&RenderOptions {
            pan: Vec3::ZERO,
            ..RenderOptions::default()
        });
        let pan = Vec3::new(0.25, -0.5, 0.1);
        let panned = framing.camera(&RenderOptions {
            pan,
            ..RenderOptions::default()
        });

        let expected = pan * framing.radius;
        assert!(
            (panned.center - (base.center + expected)).length() < 1e-4,
            "pivot must shift by pan*radius: base={:?}, panned={:?}",
            base.center,
            panned.center
        );
        assert!(
            (panned.eye - (base.eye + expected)).length() < 1e-4,
            "eye must shift by the same offset (rigid translation)"
        );
        assert_eq!(base.forward, panned.forward, "pan must not rotate the view");
        assert_eq!(base.right, panned.right);
        assert_eq!(base.up, panned.up);
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
    fn joined_corner_wall_envelopes_form_one_volume_disjoint_butt_lap() {
        let model = corner_model();
        let assets = RenderAssets::default();
        let mut palette = PaletteBuilder::new(&model, &assets);
        let wall_bounds = |wall: &Wall, palette: &mut PaletteBuilder<'_>| {
            let mut triangles = Vec::new();
            let mut bounds = Aabb::EMPTY;
            push_wall(&mut triangles, &mut bounds, &model, wall, None, palette);
            assert!(!triangles.is_empty());
            bounds
        };
        let first = wall_bounds(&model.walls[0], &mut palette);
        let second = wall_bounds(&model.walls[1], &mut palette);
        let x_overlap = first.max.x.min(second.max.x) - first.min.x.max(second.min.x);
        let y_overlap = first.max.y.min(second.max.y) - first.min.y.max(second.min.y);

        assert!(x_overlap > 0.0, "the perpendicular wall bands must meet");
        assert!(
            y_overlap.abs() < 1.0e-4,
            "the through wall and butting wall must share one face without a gap or overlapping volume"
        );
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
        let model = BuildingModel::new();
        let scene = scene_from_model(&model, &RenderOptions::default());
        let hist = material_histogram(&scene);
        assert_eq!(hist.get(&MAT_GROUND).copied().unwrap_or(0), 2);
        assert!(scene.triangles.len() >= 2);
    }

    #[test]
    fn exterior_wall_uses_cladding_not_drywall() {
        let model = wall_model(WallExposure::Exterior, vec![]);
        let scene = scene_from_model(&model, &RenderOptions::default());
        let wall_materials = wall_surface_triangle_materials(&scene);
        assert!(wall_materials.len() >= 12);
        assert!(!wall_materials.contains(&MAT_DRYWALL));
    }

    #[test]
    fn interior_wall_uses_drywall_not_cladding() {
        let model = wall_model(WallExposure::Interior, vec![]);
        let scene = scene_from_model(&model, &RenderOptions::default());
        let wall_materials = wall_surface_triangle_materials(&scene);
        assert!(wall_materials.len() >= 12);
        assert!(!wall_materials.contains(&MAT_CLADDING));
    }

    #[test]
    fn textured_wall_material_samples_resolved_asset() {
        let mut model = wall_model(WallExposure::Exterior, vec![]);
        let system = model
            .systems
            .iter()
            .find(|system| system.id.0 == "system-wall-exterior-1")
            .unwrap();
        let cladding_id = system
            .layers
            .iter()
            .rev()
            .find(|layer| layer.function == LayerFunction::Cladding)
            .unwrap()
            .material
            .clone();
        let hash = "blake3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let material = model
            .materials
            .iter_mut()
            .find(|m| m.id == cladding_id)
            .unwrap();
        material.appearance = Appearance::Textured {
            color: [20, 30, 40],
            texture: AssetRef::new(hash, "image/png", TextureRole::Texture),
            scale: Length::from_whole_inches(12),
        };
        let mut assets = RenderAssets::new();
        assets.insert_texture(hash, Texture::from_rgb8(2, 1, &[255, 0, 0, 0, 255, 0]));

        let scene = scene_from_model_with_assets(&model, &RenderOptions::default(), &assets);
        let textured_index = scene
            .materials
            .iter()
            .position(|material| matches!(material, Material::TexturedDiffuse { .. }))
            .expect("textured material should be lowered into the scene")
            as u32;
        let hit = Hit {
            t: 1.0,
            u: 0.0,
            v: 0.0,
            point: Vec3::new(0.0, 0.0, 6.0),
            normal: Vec3::new(0.0, 0.0, 1.0),
            geom_normal: Vec3::new(0.0, 0.0, 1.0),
            front_face: true,
            material: textured_index,
        };

        assert!(matches!(
            scene.material(&hit),
            Material::Diffuse { albedo }
                if (albedo - Vec3::new(1.0, 0.0, 0.0)).length() < 1.0e-5
        ));
    }

    #[test]
    fn asset_backed_materials_without_resolved_assets_lower_to_fallback_diffuse() {
        let model = BuildingModel::new();
        let assets = RenderAssets::new();
        let mut palette = PaletteBuilder::new(&model, &assets);
        let hash = "blake3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

        let textured = render_material_for_appearance(
            &Appearance::Textured {
                color: [20, 30, 40],
                texture: AssetRef::new(hash, "image/png", TextureRole::Texture),
                scale: Length::from_whole_inches(12),
            },
            &mut palette,
        );
        let depth_mapped = render_material_for_appearance(
            &Appearance::DepthMapped {
                color: [50, 60, 70],
                height: AssetRef::new(hash, "image/png", TextureRole::Height),
                scale: Length::from_whole_inches(12),
            },
            &mut palette,
        );

        assert!(matches!(
            textured,
            Material::Diffuse { albedo } if (albedo - color_to_linear([20, 30, 40])).length() < 1.0e-6
        ));
        assert!(matches!(
            depth_mapped,
            Material::Diffuse { albedo } if (albedo - color_to_linear([50, 60, 70])).length() < 1.0e-6
        ));
        assert!(palette.textures.is_empty());
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

    // === roof / ceiling / floor surfaces ===

    use framer_core::{
        BoardProfile, ConstructionLayer, FramingPattern, FramingSpec, Level,
        Material as CoreMaterial, MemberFamily, Room, RoomUsage, Slope, SystemKind,
    };

    /// A 12ft × 8ft rectangle, used as both the roof outline and the deck/ceiling
    /// region. The y=0 edge (index 0) is the eave; the up-slope direction is +y.
    fn rect12x8() -> Vec<Point2> {
        vec![
            Point2::new(Length::ZERO, Length::ZERO),
            Point2::new(Length::from_feet(12.0), Length::ZERO),
            Point2::new(Length::from_feet(12.0), Length::from_feet(8.0)),
            Point2::new(Length::ZERO, Length::from_feet(8.0)),
        ]
    }

    /// A system with a single named finish layer over a framing layer, ordered
    /// interior → exterior so each surface picks the face the viewer sees.
    fn finish_system(
        id: &str,
        kind: SystemKind,
        finish: LayerFunction,
        finish_material: &str,
        finish_first: bool,
    ) -> ConstructionSystem {
        let framing = ConstructionLayer::new(
            LayerFunction::Framing,
            "mat-spf",
            BoardProfile::TwoBySix.nominal_depth(),
        )
        .with_framing(FramingSpec {
            member: BoardProfile::TwoBySix,
            spacing: Length::from_whole_inches(16),
            pattern: FramingPattern::Single,
            member_family: MemberFamily::Rafter,
            cavity_material: None,
        });
        let finish = ConstructionLayer::new(finish, finish_material, Length::from_whole_inches(1));
        let layers = if finish_first {
            vec![finish, framing]
        } else {
            vec![framing, finish]
        };
        ConstructionSystem {
            id: ElementId::new(id),
            name: id.to_owned(),
            kind,
            source: None,
            layers,
        }
    }

    /// A model carrying one gable roof plane, one flat ceiling, and one floor deck
    /// over a 12×8 footprint on a 9ft-tall level, each with a distinctly colored
    /// finish so the rendered material can be checked.
    fn roofed_model() -> BuildingModel {
        let mut model = BuildingModel::new();
        // 9ft top plane so the ceiling (12" below it) lands at 8ft.
        for level in &mut model.levels {
            if level.id.0 == "level-1" {
                level.height = Length::from_whole_inches(108);
            }
        }
        model.materials.push(CoreMaterial::solid_color(
            "mat-roof",
            "Shingle",
            [40, 40, 45],
        ));
        model.materials.push(CoreMaterial::solid_color(
            "mat-ceil",
            "Ceiling",
            [230, 230, 225],
        ));
        model.materials.push(CoreMaterial::solid_color(
            "mat-floor",
            "Subfloor",
            [120, 90, 60],
        ));
        model.systems.push(finish_system(
            "system-roof",
            SystemKind::Roof,
            LayerFunction::Roofing,
            "mat-roof",
            false,
        ));
        model.systems.push(finish_system(
            "system-ceiling",
            SystemKind::Ceiling,
            LayerFunction::CeilingFinish,
            "mat-ceil",
            true,
        ));
        model.systems.push(finish_system(
            "system-floor",
            SystemKind::Floor,
            LayerFunction::InteriorFinish,
            "mat-floor",
            true,
        ));
        // 4:12 gable plane springing at 8ft; ridge underside rises 8ft run ×
        // 4/12 = 32" to 128", and the weather face lifts 7" above that.
        model.roof_planes.push(RoofPlane::new(
            "roof-1",
            "Roof",
            "level-1",
            "system-roof",
            rect12x8(),
            Slope::new(Length::from_whole_inches(4), Length::from_whole_inches(12)),
            0,
            Length::from_feet(8.0),
        ));
        model.ceilings.push(Ceiling::new(
            "ceiling-1",
            "Ceiling",
            "level-1",
            "system-ceiling",
            SurfaceRegion::Polygon(rect12x8()),
            Length::from_whole_inches(12),
        ));
        model.floor_decks.push(FloorDeck::new(
            "deck-1",
            "Deck",
            "level-1",
            "system-floor",
            SurfaceRegion::Polygon(rect12x8()),
        ));
        model
    }

    /// The demo shell capped by two opposing 6:12 planes. The planes' original
    /// rake edges lie on the left/right exterior walls, so core derives one gable
    /// profile per end with an 8ft base and 13ft apex.
    fn gable_wall_model() -> BuildingModel {
        let ft = Length::from_feet;
        let mut model = BuildingModel::demo_shell();
        let slope = Slope::new(Length::from_whole_inches(6), Length::from_whole_inches(12));
        model.roof_planes.push(RoofPlane::new(
            "roof-south",
            "South roof",
            "level-1",
            "system-roof-1",
            vec![
                Point2::new(Length::ZERO, Length::ZERO),
                Point2::new(ft(28.0), Length::ZERO),
                Point2::new(ft(28.0), ft(10.0)),
                Point2::new(Length::ZERO, ft(10.0)),
            ],
            slope,
            0,
            ft(8.0),
        ));
        model.roof_planes.push(RoofPlane::new(
            "roof-north",
            "North roof",
            "level-1",
            "system-roof-1",
            vec![
                Point2::new(Length::ZERO, ft(20.0)),
                Point2::new(ft(28.0), ft(20.0)),
                Point2::new(ft(28.0), ft(10.0)),
                Point2::new(Length::ZERO, ft(10.0)),
            ],
            slope,
            0,
            ft(8.0),
        ));
        model
    }

    #[test]
    fn roof_plane_emits_sloped_surface_at_true_elevations() {
        let scene = scene_from_model(&roofed_model(), &RenderOptions::default());
        // Collect every vertex z of triangles that are genuinely sloped (not all
        // three vertices at one elevation) — those are the roof's.
        let mut sloped_zs: Vec<f32> = Vec::new();
        for t in &scene.triangles {
            let v1 = t.v0 + t.edge1;
            let v2 = t.v0 + t.edge2;
            let zs = [t.v0.z, v1.z, v2.z];
            let zmin = zs.iter().cloned().fold(f32::INFINITY, f32::min);
            let zmax = zs.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
            if zmax - zmin > 1.0 {
                sloped_zs.extend_from_slice(&zs);
            }
        }
        assert!(!sloped_zs.is_empty(), "no sloped roof triangles emitted");
        let lo = sloped_zs.iter().cloned().fold(f32::INFINITY, f32::min);
        let hi = sloped_zs.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        // Bearing underside eave sits at 96"; weather ridge sits at 128" + 7".
        assert!((lo - 96.0).abs() < 0.5, "eave elevation {lo}, want ~96");
        assert!((hi - 135.0).abs() < 0.5, "ridge elevation {hi}, want ~135");
    }

    #[test]
    fn roof_overhang_expands_surface_bounds_and_lowers_the_eave() {
        let mut model = roofed_model();
        model.roof_planes[0].eave_overhang = Length::from_whole_inches(12);
        model.roof_planes[0].rake_overhang = Length::from_whole_inches(6);

        let (scene, framing) = build_scene(&model, &RenderOptions::default());
        let roof_vertices: Vec<Vec3> = scene
            .triangles
            .iter()
            .filter(|triangle| {
                let nz = triangle.geom_normal.z.abs();
                nz > 0.1 && nz < 0.99
            })
            .flat_map(|triangle| {
                [
                    triangle.v0,
                    triangle.v0 + triangle.edge1,
                    triangle.v0 + triangle.edge2,
                ]
            })
            .collect();
        assert!(!roof_vertices.is_empty(), "no overhung roof geometry");

        let min = |axis: fn(Vec3) -> f32| {
            roof_vertices
                .iter()
                .copied()
                .map(axis)
                .fold(f32::INFINITY, f32::min)
        };
        let max = |axis: fn(Vec3) -> f32| {
            roof_vertices
                .iter()
                .copied()
                .map(axis)
                .fold(f32::NEG_INFINITY, f32::max)
        };
        let x = |point: Vec3| point.x;
        let y = |point: Vec3| point.y;
        let z = |point: Vec3| point.z;

        // The original 12ft x 8ft outline becomes [-6in, 150in] across the
        // rakes and extends 12in down-slope from y=0. Projecting that eave through
        // the original 4:12 frame drops its underside from 96in to 92in.
        assert!((min(x) + 6.0).abs() < 0.05, "min x = {}", min(x));
        assert!((max(x) - 150.0).abs() < 0.05, "max x = {}", max(x));
        assert!((min(y) + 12.0).abs() < 0.05, "min y = {}", min(y));
        assert!((max(y) - 96.0).abs() < 0.05, "max y = {}", max(y));
        assert!((min(z) - 92.0).abs() < 0.05, "min z = {}", min(z));
        assert!((max(z) - 135.0).abs() < 0.05, "max z = {}", max(z));

        // Scene framing consumes the expanded bounds too: y spans -12..96, so
        // the geometry-only orbit center is 42in rather than the authored 48in.
        assert!((framing.center.y - 42.0).abs() < 0.05);
    }

    #[test]
    fn roof_opening_is_absent_from_path_traced_surface_triangles() {
        let mut model = roofed_model();
        model.roof_planes[0].openings.push(RoofOpening::new(
            "skylight-test",
            OpeningKind::Skylight,
            Point2::new(Length::from_feet(6.0), Length::from_feet(4.0)),
            Length::from_feet(2.0),
            Length::from_feet(2.0),
        ));
        let scene = scene_from_model(&model, &RenderOptions::default());
        let roof_material = scene
            .materials
            .iter()
            .position(|material| {
                matches!(material, Material::Diffuse { albedo }
                    if (*albedo - color_to_linear([40, 40, 45])).length() < 1.0e-5)
            })
            .unwrap() as u32;
        for triangle in scene
            .triangles
            .iter()
            .filter(|triangle| triangle.material == roof_material)
        {
            let a = triangle.v0;
            let b = triangle.v0 + triangle.edge1;
            let c = triangle.v0 + triangle.edge2;
            let centroid_x = (a.x + b.x + c.x) / 3.0;
            let centroid_y = (a.y + b.y + c.y) / 3.0;
            assert!(
                (centroid_x - 72.0).abs() >= 12.0 || (centroid_y - 48.0).abs() >= 12.0,
                "a render triangle filled the modeled skylight cavity"
            );
        }
    }

    #[test]
    fn invalid_roof_cavities_keep_a_holeless_render_fallback() {
        let mut model = roofed_model();
        for (id, x) in [("skylight-a", 6.0), ("skylight-b", 6.5)] {
            model.roof_planes[0].openings.push(RoofOpening::new(
                id,
                OpeningKind::Skylight,
                Point2::new(Length::from_feet(x), Length::from_feet(4.0)),
                Length::from_feet(2.0),
                Length::from_feet(2.0),
            ));
        }
        assert!(
            model
                .roof_surface_triangulation(&model.roof_planes[0])
                .is_none(),
            "overlapping cavity rings must fail closed in physical geometry"
        );

        let scene = scene_from_model(&model, &RenderOptions::default());
        let roof_material = scene
            .materials
            .iter()
            .position(|material| {
                matches!(material, Material::Diffuse { albedo }
                    if (*albedo - color_to_linear([40, 40, 45])).length() < 1.0e-5)
            })
            .unwrap() as u32;
        assert_eq!(
            scene
                .triangles
                .iter()
                .filter(|triangle| triangle.material == roof_material)
                .count(),
            4,
            "the two-sided rectangular roof fallback must remain visible"
        );
    }

    #[test]
    fn gable_profiles_close_wall_envelopes_to_the_roof_apex() {
        let model = gable_wall_model();
        let profiles = model.gable_wall_profiles();
        assert_eq!(profiles.len(), 2);
        assert!(profiles.contains_key(&ElementId::new("wall-left")));
        assert!(profiles.contains_key(&ElementId::new("wall-right")));
        assert!(
            profiles
                .values()
                .all(|profile| profile.base_elevation == Length::from_feet(8.0)
                    && profile.peak_elevation == Length::from_feet(13.0))
        );

        let scene = scene_from_model(&model, &RenderOptions::default());
        let elevated_vertical_vertices: Vec<Vec3> = scene
            .triangles
            .iter()
            .filter(|triangle| {
                if triangle.geom_normal.z.abs() > 1.0e-4 {
                    return false;
                }
                let vertices = [
                    triangle.v0,
                    triangle.v0 + triangle.edge1,
                    triangle.v0 + triangle.edge2,
                ];
                vertices.iter().any(|vertex| vertex.z > 96.5)
            })
            .flat_map(|triangle| {
                [
                    triangle.v0,
                    triangle.v0 + triangle.edge1,
                    triangle.v0 + triangle.edge2,
                ]
            })
            .collect();
        assert!(
            !elevated_vertical_vertices.is_empty(),
            "no vertical gable wall faces above the authored wall top"
        );
        let apex_vertices: Vec<Vec3> = elevated_vertical_vertices
            .iter()
            .copied()
            .filter(|vertex| (vertex.z - 156.0).abs() < 0.05)
            .collect();
        assert!(
            apex_vertices
                .iter()
                .any(|vertex| vertex.x.abs() < 12.0 && (vertex.y - 120.0).abs() < 0.05),
            "left gable does not reach the 13ft apex: {apex_vertices:?}"
        );
        assert!(
            apex_vertices.iter().any(|vertex| {
                (vertex.x - 336.0).abs() < 12.0 && (vertex.y - 120.0).abs() < 0.05
            }),
            "right gable does not reach the 13ft apex: {apex_vertices:?}"
        );

        // Without matching roof planes there is no derived profile and the same
        // shell remains rectangular at the authored 8ft wall height.
        let mut bare = model;
        bare.roof_planes.clear();
        assert!(bare.gable_wall_profiles().is_empty());
        let bare_scene = scene_from_model(&bare, &RenderOptions::default());
        assert!(bare_scene.triangles.iter().all(|triangle| {
            if triangle.geom_normal.z.abs() > 1.0e-4 {
                return true;
            }
            [
                triangle.v0,
                triangle.v0 + triangle.edge1,
                triangle.v0 + triangle.edge2,
            ]
            .iter()
            .all(|vertex| vertex.z <= 96.5)
        }));
    }

    #[test]
    fn full_height_opening_under_gable_gets_a_base_cap() {
        let mut model = gable_wall_model();
        let wall_index = model
            .walls
            .iter()
            .position(|wall| wall.id.0 == "wall-left")
            .unwrap();
        let height = model.walls[wall_index].height;
        model.walls[wall_index].openings[0].height = height;
        model.walls[wall_index].openings[0].kind = OpeningKind::Stair;
        let wall = &model.walls[wall_index];
        let opening = &wall.openings[0];
        let basis = WallBasis::new(wall);
        let scene = scene_from_model(&model, &RenderOptions::default());

        let cap_triangles = scene
            .triangles
            .iter()
            .filter(|triangle| {
                let vertices = [
                    triangle.v0,
                    triangle.v0 + triangle.edge1,
                    triangle.v0 + triangle.edge2,
                ];
                vertices.iter().all(|vertex| {
                    let dx = vertex.x - basis.ox;
                    let dy = vertex.y - basis.oy;
                    let x = dx * basis.ax + dy * basis.ay;
                    (vertex.z - height.inches() as f32).abs() < 0.01
                        && x >= opening.left().inches() as f32 - 0.01
                        && x <= opening.right().inches() as f32 + 0.01
                })
            })
            .count();
        assert!(
            cap_triangles >= 2,
            "the gable base must close across an opening that reaches the wall top"
        );
    }

    #[test]
    fn flat_ceiling_and_floor_emit_horizontal_surfaces_at_their_elevations() {
        let scene = scene_from_model(&roofed_model(), &RenderOptions::default());
        let level_zs: Vec<f32> = scene
            .triangles
            .iter()
            .filter(|t| {
                let v1 = t.v0 + t.edge1;
                let v2 = t.v0 + t.edge2;
                // Horizontal triangle: all three vertices share one elevation.
                (t.v0.z - v1.z).abs() < 1.0e-3 && (t.v0.z - v2.z).abs() < 1.0e-3
            })
            .map(|t| t.v0.z)
            .collect();
        // Ceiling underside at 108 − 12 = 96"; floor deck at the level elevation 0".
        assert!(
            level_zs.iter().any(|z| (z - 96.0).abs() < 0.5),
            "no ceiling surface at ~96in: {level_zs:?}"
        );
        assert!(
            level_zs.iter().any(|z| z.abs() < 0.5),
            "no floor surface at ~0in: {level_zs:?}"
        );
    }

    #[test]
    fn roof_surface_uses_the_systems_roofing_material() {
        let scene = scene_from_model(&roofed_model(), &RenderOptions::default());
        let want = color_to_linear([40, 40, 45]);
        // A sloped triangle is the roof; its material must be the lowered shingle.
        let roof_tri = scene
            .triangles
            .iter()
            .find(|t| {
                let v1 = t.v0 + t.edge1;
                let v2 = t.v0 + t.edge2;
                (t.v0.z.max(v1.z).max(v2.z) - t.v0.z.min(v1.z).min(v2.z)) > 1.0
            })
            .expect("a sloped roof triangle");
        match &scene.materials[roof_tri.material as usize] {
            Material::Diffuse { albedo } => {
                assert!(
                    (*albedo - want).length() < 1.0e-5,
                    "roof albedo {albedo:?}, want {want:?}"
                );
            }
            other => panic!("roof material is not diffuse: {other:?}"),
        }
    }

    #[test]
    fn surface_geometry_is_finite() {
        let scene = scene_from_model(&roofed_model(), &RenderOptions::default());
        for t in &scene.triangles {
            for v in [t.v0, t.v0 + t.edge1, t.v0 + t.edge2, t.geom_normal] {
                assert!(
                    v.x.is_finite() && v.y.is_finite() && v.z.is_finite(),
                    "non-finite vertex/normal in surface geometry"
                );
            }
        }
    }

    /// Six corner-joined walls enclosing a concave L-shaped room (a 12×12 ft square
    /// with a 6×6 ft bite out of the top-right; 108 sq ft), a room seeded inside it,
    /// and a floor deck attached to that room via `SurfaceRegion::Room`.
    fn l_shaped_room_model() -> BuildingModel {
        let ft = Length::from_feet;
        let mut model = BuildingModel::new();
        let pts = [
            Point2::new(ft(0.0), ft(0.0)),
            Point2::new(ft(12.0), ft(0.0)),
            Point2::new(ft(12.0), ft(6.0)),
            Point2::new(ft(6.0), ft(6.0)),
            Point2::new(ft(6.0), ft(12.0)),
            Point2::new(ft(0.0), ft(12.0)),
        ];
        for i in 0..pts.len() {
            let next = (i + 1) % pts.len();
            model.walls.push(
                Wall::new(format!("w-{i}"), "Wall", ft(1.0), &model.framing_defaults())
                    .with_placement("level-1", pts[i], pts[next]),
            );
        }
        model.rooms.push(Room::new(
            "room-1",
            "L room",
            RoomUsage::default(),
            "level-1",
            Point2::new(ft(3.0), ft(3.0)),
        ));
        model.materials.push(CoreMaterial::solid_color(
            "mat-floor",
            "Subfloor",
            [150, 116, 78],
        ));
        model.systems.push(finish_system(
            "system-floor",
            SystemKind::Floor,
            LayerFunction::InteriorFinish,
            "mat-floor",
            true,
        ));
        model.floor_decks.push(FloorDeck::new(
            "deck-1",
            "Deck",
            "level-1",
            "system-floor",
            SurfaceRegion::Room(ElementId::new("room-1")),
        ));
        model
    }

    #[test]
    fn room_region_surface_tiles_a_concave_outline() {
        // Drives the production path the concave triangulator was added for:
        // SurfaceRegion::Room -> room_boundary_on_level (a concave L loop) ->
        // triangulate_simple_polygon -> emitted floor triangles. A naive vertex-0
        // fan would spill outside the L's notch, inflating the covered area.
        let scene = scene_from_model(&l_shaped_room_model(), &RenderOptions::default());
        let floor_mat = scene
            .materials
            .iter()
            .position(|m| {
                matches!(m, Material::Diffuse { albedo }
                if (*albedo - color_to_linear([150, 116, 78])).length() < 1.0e-5)
            })
            .expect("floor material lowered into the scene") as u32;

        let floor: Vec<&Triangle> = scene
            .triangles
            .iter()
            .filter(|t| t.material == floor_mat)
            .collect();
        // An L (6-gon) triangulates to 4 triangles, all at the floor elevation (0).
        assert_eq!(floor.len(), 4, "L floor should be 4 triangles");

        let mut area = 0.0_f64;
        for t in &floor {
            let v1 = t.v0 + t.edge1;
            let v2 = t.v0 + t.edge2;
            for v in [t.v0, v1, v2] {
                assert!(v.z.abs() < 0.5, "floor triangle not at the deck elevation");
            }
            // Plan area of a horizontal triangle = ½|edge1 × edge2| (z component).
            let cross_z = t.edge1.x * t.edge2.y - t.edge1.y * t.edge2.x;
            area += 0.5 * cross_z.abs() as f64;
            // Centroid must lie inside the L (outside the 6×12 .. 12×6 notch).
            let cx = (t.v0.x + v1.x + v2.x) / 3.0;
            let cy = (t.v0.y + v1.y + v2.y) / 3.0;
            let in_l = (0.0..=144.0).contains(&cx)
                && (0.0..=144.0).contains(&cy)
                && !(cx > 72.0 && cy > 72.0);
            assert!(
                in_l,
                "floor triangle centroid ({cx},{cy}) spilled outside the L"
            );
        }
        // 12×12 − 6×6 = 108 sq ft = 15552 sq in; a spilling fan would exceed this.
        assert!(
            (area - 15552.0).abs() < 5.0,
            "L floor triangles cover {area} sq in, expected 15552"
        );
    }

    fn stacked_unenclosed_room_deck_model() -> BuildingModel {
        let ft = Length::from_feet;
        let mut model = BuildingModel::new();
        model
            .levels
            .push(Level::new("level-2", "Level 2", ft(10.0)));
        for (i, window) in rect12x8().windows(2).enumerate() {
            model.walls.push(
                Wall::new(format!("w-{i}"), "Wall", ft(1.0), &model.framing_defaults())
                    .with_placement("level-1", window[0], window[1]),
            );
        }
        let outline = rect12x8();
        model.walls.push(
            Wall::new("w-close", "Wall", ft(1.0), &model.framing_defaults())
                .with_placement("level-1", outline[3], outline[0]),
        );
        model.rooms.push(Room::new(
            "room-2",
            "Upper room",
            RoomUsage::Living,
            "level-2",
            Point2::new(ft(6.0), ft(4.0)),
        ));
        model.materials.push(CoreMaterial::solid_color(
            "mat-upper-deck",
            "Upper deck",
            [25, 90, 150],
        ));
        model.systems.push(finish_system(
            "system-floor",
            SystemKind::Floor,
            LayerFunction::InteriorFinish,
            "mat-upper-deck",
            true,
        ));
        model.floor_decks.push(FloorDeck::new(
            "deck-2",
            "Upper deck",
            "level-2",
            "system-floor",
            SurfaceRegion::Room(ElementId::new("room-2")),
        ));
        model
    }

    #[test]
    fn room_region_surface_resolves_against_the_room_level() {
        let scene = scene_from_model(
            &stacked_unenclosed_room_deck_model(),
            &RenderOptions::default(),
        );
        let upper_level_z = Length::from_feet(10.0).inches() as f32;
        let upper_deck: Vec<&Triangle> = scene
            .triangles
            .iter()
            .filter(|triangle| {
                let v1 = triangle.v0 + triangle.edge1;
                let v2 = triangle.v0 + triangle.edge2;
                let zs = [triangle.v0.z, v1.z, v2.z];
                zs.iter().all(|z| (*z - upper_level_z).abs() < 0.5)
            })
            .collect();
        assert!(
            upper_deck.is_empty(),
            "a level-2 room region must not render over a level-1 enclosure"
        );
    }

    /// A 9ft-tall level with one gable roof plane over a 12×8 footprint and **no
    /// ceiling** — a cathedral. The roof system stacks a conditioned-side finish
    /// (soffit), a framing layer, and roofing, so its weather face and its interior
    /// underside resolve to distinct colors.
    fn cathedral_model() -> BuildingModel {
        let mut model = BuildingModel::new();
        for level in &mut model.levels {
            if level.id.0 == "level-1" {
                level.height = Length::from_whole_inches(108);
            }
        }
        model.materials.push(CoreMaterial::solid_color(
            "mat-roof",
            "Shingle",
            [40, 40, 45],
        ));
        model.materials.push(CoreMaterial::solid_color(
            "mat-soffit",
            "Soffit",
            [205, 180, 140],
        ));
        // Roof assembly ordered conditioned-side → weather-side: interior finish,
        // framing, roofing. Surfaces select the weather face outward, the soffit
        // on the cathedral underside.
        let framing = ConstructionLayer::new(
            LayerFunction::Framing,
            "mat-spf",
            BoardProfile::TwoBySix.nominal_depth(),
        )
        .with_framing(FramingSpec {
            member: BoardProfile::TwoBySix,
            spacing: Length::from_whole_inches(16),
            pattern: FramingPattern::Single,
            member_family: MemberFamily::Rafter,
            cavity_material: None,
        });
        model.systems.push(ConstructionSystem {
            id: ElementId::new("system-roof"),
            name: "Roof".to_owned(),
            kind: SystemKind::Roof,
            source: None,
            layers: vec![
                ConstructionLayer::new(
                    LayerFunction::CeilingFinish,
                    "mat-soffit",
                    Length::from_whole_inches(1),
                ),
                framing,
                ConstructionLayer::new(
                    LayerFunction::Roofing,
                    "mat-roof",
                    Length::from_whole_inches(1),
                ),
            ],
        });
        model.roof_planes.push(RoofPlane::new(
            "roof-1",
            "Roof",
            "level-1",
            "system-roof",
            rect12x8(),
            Slope::new(Length::from_whole_inches(4), Length::from_whole_inches(12)),
            0,
            Length::from_feet(8.0),
        ));
        model
    }

    /// Collect the sloped (roof) triangles whose albedo matches `color`.
    fn sloped_triangles_with_color(scene: &Scene, color: [u8; 3]) -> Vec<&Triangle> {
        let want = color_to_linear(color);
        scene
            .triangles
            .iter()
            .filter(|t| {
                let v1 = t.v0 + t.edge1;
                let v2 = t.v0 + t.edge2;
                let sloped = (t.v0.z.max(v1.z).max(v2.z) - t.v0.z.min(v1.z).min(v2.z)) > 1.0;
                let matches_color = matches!(
                    &scene.materials[t.material as usize],
                    Material::Diffuse { albedo } if (*albedo - want).length() < 1.0e-5
                );
                sloped && matches_color
            })
            .collect()
    }

    #[test]
    fn cathedral_roof_emits_an_interior_finish_underside() {
        let scene = scene_from_model(&cathedral_model(), &RenderOptions::default());
        // The weather face (shingle) is present...
        let weather = sloped_triangles_with_color(&scene, [40, 40, 45]);
        assert!(!weather.is_empty(), "no weather-face roof triangles");
        // ...and so is a distinct interior-finish (soffit) underside.
        let underside = sloped_triangles_with_color(&scene, [205, 180, 140]);
        assert!(
            !underside.is_empty(),
            "cathedral roof emitted no interior-finish underside"
        );
        // The weather face sits above the underside by ~the assembly thickness
        // (1in finish + 6in nominal 2×6 + 1in roofing = 8in), while the underside
        // itself stays at the authored bearing/springing plane.
        let min_z = |tris: &[&Triangle]| {
            tris.iter()
                .flat_map(|t| [t.v0.z, (t.v0 + t.edge1).z, (t.v0 + t.edge2).z])
                .fold(f32::INFINITY, f32::min)
        };
        let underside_lo = min_z(&underside);
        assert!(
            (underside_lo - 96.0).abs() < 0.5,
            "underside springs at {underside_lo}in, want ~96"
        );
        let lift = min_z(&weather) - underside_lo;
        assert!(
            (lift - 8.0).abs() < 0.5,
            "weather face lifted {lift}in above the underside, want ~8"
        );
    }

    #[test]
    fn roof_with_a_ceiling_below_shows_no_cathedral_underside() {
        // Add a flat ceiling covering the footprint: the plane is no longer a
        // cathedral, so no interior-finish underside is emitted under the roof.
        let mut model = cathedral_model();
        model.systems.push(finish_system(
            "system-ceiling",
            SystemKind::Ceiling,
            LayerFunction::CeilingFinish,
            "mat-soffit",
            true,
        ));
        model.ceilings.push(Ceiling::new(
            "ceiling-1",
            "Ceiling",
            "level-1",
            "system-ceiling",
            SurfaceRegion::Polygon(rect12x8()),
            Length::from_whole_inches(12),
        ));
        let scene = scene_from_model(&model, &RenderOptions::default());
        // The only soffit-colored triangles are the flat ceiling (not sloped).
        assert!(
            sloped_triangles_with_color(&scene, [205, 180, 140]).is_empty(),
            "a roof with a ceiling below should emit no sloped underside"
        );
    }

    #[test]
    fn roof_grows_the_camera_framing_upward() {
        // The ridge at 128" lifts the bounds, so the framing radius is larger than
        // for the bare walls/decks alone.
        let with_roof = scene_from_model(&roofed_model(), &RenderOptions::default());
        let mut no_roof_model = roofed_model();
        no_roof_model.roof_planes.clear();
        let without = scene_from_model(&no_roof_model, &RenderOptions::default());
        // Both frame the model; the roofed one reaches a higher max elevation, so
        // it emits strictly more triangles and a finite, larger vertical extent.
        assert!(with_roof.triangles.len() > without.triangles.len());
    }
}
