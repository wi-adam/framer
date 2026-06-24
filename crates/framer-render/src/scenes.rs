//! Fixed synthetic scenes used by tests. [`reference_scene`] exercises every
//! material (diffuse, metal, glass, emissive) under a sun + procedural sky; it is
//! the single source of truth for both the CPU golden-image regression test and
//! the app's GPU↔CPU parity test, so the two validate the *same* scene.

use framer_core::{
    BoardProfile, BuildingModel, Ceiling, CeilingSlope, ConstructionLayer, ConstructionSystem,
    FloorDeck, FramingPattern, FramingSpec, LayerFunction, Length, Material as CoreMaterial,
    MemberFamily, Point2, RoofPlane, Slope, SurfaceRegion, SystemKind,
};

use crate::build::{RenderOptions, scene_from_model};
use crate::camera::Camera;
use crate::geom::Triangle;
use crate::material::Material;
use crate::math::Vec3;
use crate::scene::{DirectionalSun, Scene, Sky};

/// Reference render dimensions and sampling (shared by the golden + parity tests).
pub const REFERENCE_WIDTH: u32 = 64;
pub const REFERENCE_HEIGHT: u32 = 48;
pub const REFERENCE_SPP: u32 = 12;
pub const REFERENCE_SEED: u64 = 7;

/// Pushes an axis-aligned cube centered at `c` with half-size `h`.
fn cube(tris: &mut Vec<Triangle>, c: Vec3, h: f32, mat: u32) {
    let corners = [
        Vec3::new(c.x - h, c.y - h, c.z - h),
        Vec3::new(c.x + h, c.y - h, c.z - h),
        Vec3::new(c.x + h, c.y + h, c.z - h),
        Vec3::new(c.x - h, c.y + h, c.z - h),
        Vec3::new(c.x - h, c.y - h, c.z + h),
        Vec3::new(c.x + h, c.y - h, c.z + h),
        Vec3::new(c.x + h, c.y + h, c.z + h),
        Vec3::new(c.x - h, c.y + h, c.z + h),
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
        tris.push(Triangle::new(
            corners[f[0]],
            corners[f[1]],
            corners[f[2]],
            mat,
        ));
        tris.push(Triangle::new(
            corners[f[0]],
            corners[f[2]],
            corners[f[3]],
            mat,
        ));
    }
}

/// A fixed scene: a ground plane plus three cubes (diffuse red, metal, glass)
/// lit by a warm sun against a blue-grey gradient sky.
pub fn reference_scene() -> Scene {
    let mut tris = Vec::new();
    // Ground (material 0).
    let s = 20.0;
    tris.push(Triangle::new(
        Vec3::new(-s, -s, 0.0),
        Vec3::new(s, -s, 0.0),
        Vec3::new(s, s, 0.0),
        0,
    ));
    tris.push(Triangle::new(
        Vec3::new(-s, -s, 0.0),
        Vec3::new(s, s, 0.0),
        Vec3::new(-s, s, 0.0),
        0,
    ));
    // Three cubes: diffuse red (1), metal (2), glass (3).
    cube(&mut tris, Vec3::new(-2.2, 0.0, 1.0), 1.0, 1);
    cube(&mut tris, Vec3::new(0.0, 0.0, 1.0), 1.0, 2);
    cube(&mut tris, Vec3::new(2.2, 0.0, 1.0), 1.0, 3);

    let materials = vec![
        Material::Diffuse {
            albedo: Vec3::new(0.6, 0.6, 0.58),
        },
        Material::Diffuse {
            albedo: Vec3::new(0.75, 0.25, 0.2),
        },
        Material::Metal {
            albedo: Vec3::new(0.95, 0.9, 0.85),
            roughness: 0.15,
        },
        Material::Dielectric {
            ior: 1.5,
            tint: Vec3::new(0.9, 0.95, 0.93),
        },
    ];

    let camera = Camera::orbit(
        Vec3::new(0.0, 0.0, 1.0),
        4.5,
        -0.6,
        0.35,
        1.0,
        REFERENCE_WIDTH as f32 / REFERENCE_HEIGHT as f32,
        42.0,
        1.0,
    );
    let sun = DirectionalSun {
        dir: Vec3::new(0.4, -0.3, 0.85).normalize(),
        irradiance: Vec3::new(1.0, 0.95, 0.85) * 4.0,
        angular_radius: 0.03,
    };
    let sky = Sky {
        zenith: Vec3::new(0.16, 0.32, 0.75),
        horizon: Vec3::new(0.78, 0.83, 0.9),
        ground: Vec3::new(0.2, 0.18, 0.15),
    };
    Scene::new(tris, materials, sun, sky, camera, 1.0)
}

/// A single named finish layer over a framing layer, ordered interior →
/// exterior (so `finish_first` places a roof's weather face last and a
/// ceiling/floor's finished face first).
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
    ConstructionSystem {
        id: framer_core::ElementId::new(id),
        name: id.to_owned(),
        kind,
        source: None,
        layers: if finish_first {
            vec![finish, framing]
        } else {
            vec![framing, finish]
        },
    }
}

/// The demo shell capped with a gable roof, plus a flat ceiling and a floor deck
/// over its 28ft × 20ft footprint. Distinctly colored finishes (charcoal roof,
/// white ceiling, wood subfloor) so the sloped + horizontal surfaces read in the
/// render.
fn roofed_model() -> BuildingModel {
    let ft = Length::from_feet;
    let mut model = BuildingModel::demo_shell();
    // A 9ft top plane so the 12"-below-top ceiling lands at the 8ft wall top.
    for level in &mut model.levels {
        if level.id.0 == "level-1" {
            level.height = Length::from_whole_inches(108);
        }
    }
    model.materials.push(CoreMaterial::solid_color(
        "mat-roof",
        "Shingle",
        [44, 46, 52],
    ));
    model.materials.push(CoreMaterial::solid_color(
        "mat-ceil",
        "Ceiling",
        [232, 232, 228],
    ));
    model.materials.push(CoreMaterial::solid_color(
        "mat-floor",
        "Subfloor",
        [150, 116, 78],
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

    // A 6:12 gable: two opposing planes springing at the 8ft wall top, sharing a
    // ridge along the long axis at y = 10ft (ridge rises 120in × 6/12 = 60" to
    // 156"). The south plane's eave is the y=0 edge; the north plane's is y=20.
    let slope = Slope::new(Length::from_whole_inches(6), Length::from_whole_inches(12));
    model.roof_planes.push(RoofPlane::new(
        "roof-south",
        "South roof",
        "level-1",
        "system-roof",
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
        "system-roof",
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

    let footprint = vec![
        Point2::new(Length::ZERO, Length::ZERO),
        Point2::new(ft(28.0), Length::ZERO),
        Point2::new(ft(28.0), ft(20.0)),
        Point2::new(Length::ZERO, ft(20.0)),
    ];
    model.ceilings.push(Ceiling::new(
        "ceiling-1",
        "Ceiling",
        "level-1",
        "system-ceiling",
        SurfaceRegion::Polygon(footprint.clone()),
        Length::from_whole_inches(12),
    ));
    model.floor_decks.push(FloorDeck::new(
        "deck-1",
        "Deck",
        "level-1",
        "system-floor",
        SurfaceRegion::Polygon(footprint),
    ));
    model
}

/// A model-derived scene: the [`roofed_model`] rendered through the production
/// `scene_from_model` path. The single source of truth for the roofed golden
/// image and the sloped-surface GPU↔CPU parity test, so both validate the same
/// sloped + horizontal geometry the app emits.
pub fn roofed_scene() -> Scene {
    let opts = RenderOptions {
        aspect: REFERENCE_WIDTH as f32 / REFERENCE_HEIGHT as f32,
        ..RenderOptions::default()
    };
    scene_from_model(&roofed_model(), &opts)
}

/// The demo shell with a **scissor vault** ceiling — two opposing 6:12 sloped
/// ceilings springing at the 8ft wall top and meeting at a ridge along y = 10ft
/// (rising 120in × 6/12 = 60" to 156") — plus a floor deck, and no roof so the
/// vaulted ceiling reads from above in the render. The source of truth for the
/// scissor golden image and its GPU↔CPU parity test, locking the sloped-ceiling
/// frame lift the app's mesher also uses.
fn scissor_model() -> BuildingModel {
    let ft = Length::from_feet;
    let mut model = BuildingModel::demo_shell();
    for level in &mut model.levels {
        if level.id.0 == "level-1" {
            level.height = Length::from_whole_inches(108);
        }
    }
    model.materials.push(CoreMaterial::solid_color(
        "mat-ceil",
        "Ceiling",
        [232, 232, 228],
    ));
    model.materials.push(CoreMaterial::solid_color(
        "mat-floor",
        "Subfloor",
        [150, 116, 78],
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

    // Two opposing sloped ceilings, each springing at the 8ft wall top (12" below the
    // 9ft level top) and rising 6:12 over its 10ft half to the shared ridge at y=10ft.
    let slope = CeilingSlope::new(
        Slope::new(Length::from_whole_inches(6), Length::from_whole_inches(12)),
        0,
    );
    let mut south = Ceiling::new(
        "ceiling-south",
        "South vault",
        "level-1",
        "system-ceiling",
        SurfaceRegion::Polygon(vec![
            Point2::new(Length::ZERO, Length::ZERO),
            Point2::new(ft(28.0), Length::ZERO),
            Point2::new(ft(28.0), ft(10.0)),
            Point2::new(Length::ZERO, ft(10.0)),
        ]),
        Length::from_whole_inches(12),
    );
    south.slope = Some(slope);
    model.ceilings.push(south);
    let mut north = Ceiling::new(
        "ceiling-north",
        "North vault",
        "level-1",
        "system-ceiling",
        SurfaceRegion::Polygon(vec![
            Point2::new(Length::ZERO, ft(20.0)),
            Point2::new(ft(28.0), ft(20.0)),
            Point2::new(ft(28.0), ft(10.0)),
            Point2::new(Length::ZERO, ft(10.0)),
        ]),
        Length::from_whole_inches(12),
    );
    north.slope = Some(slope);
    model.ceilings.push(north);

    model.floor_decks.push(FloorDeck::new(
        "deck-1",
        "Deck",
        "level-1",
        "system-floor",
        SurfaceRegion::Polygon(vec![
            Point2::new(Length::ZERO, Length::ZERO),
            Point2::new(ft(28.0), Length::ZERO),
            Point2::new(ft(28.0), ft(20.0)),
            Point2::new(Length::ZERO, ft(20.0)),
        ]),
    ));
    model
}

/// The [`scissor_model`] rendered through the production `scene_from_model` path —
/// the single source of truth for the scissor golden image and its GPU↔CPU parity
/// test.
pub fn scissor_scene() -> Scene {
    let opts = RenderOptions {
        aspect: REFERENCE_WIDTH as f32 / REFERENCE_HEIGHT as f32,
        ..RenderOptions::default()
    };
    scene_from_model(&scissor_model(), &opts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_scene_is_non_trivial() {
        let scene = reference_scene();
        // Ground (2) + three cubes (12 each) = 38 triangles, four materials.
        assert_eq!(scene.triangles.len(), 38);
        assert_eq!(scene.materials.len(), 4);
        assert!(!scene.bvh.nodes.is_empty());
    }

    #[test]
    fn roofed_scene_has_sloped_roof_geometry() {
        let scene = roofed_scene();
        // Demo-shell walls + decks + ground + a gable roof: plenty of triangles,
        // all finite, with at least one genuinely sloped roof triangle.
        assert!(
            scene.triangles.len() > 60,
            "roofed scene too small: {}",
            scene.triangles.len()
        );
        let sloped = scene.triangles.iter().any(|t| {
            let v1 = t.v0 + t.edge1;
            let v2 = t.v0 + t.edge2;
            (t.v0.z.max(v1.z).max(v2.z) - t.v0.z.min(v1.z).min(v2.z)) > 1.0
        });
        assert!(sloped, "no sloped roof triangle in roofed_scene");
        for t in &scene.triangles {
            assert!(
                t.v0.x.is_finite() && t.geom_normal.x.is_finite() && t.geom_normal.y.is_finite(),
                "non-finite geometry in roofed_scene"
            );
        }
        assert!(!scene.bvh.nodes.is_empty());
    }

    #[test]
    fn scissor_scene_lifts_the_vault_ceiling_via_the_frame() {
        let scene = scissor_scene();
        // The scissor vault is the only *tilted* geometry (walls are vertical, the
        // floor is horizontal): a triangle whose normal has both a vertical and a
        // horizontal component. It springs at the 8ft (96in) wall top and rises to
        // the 13ft (156in) ridge — pinning the sloped-ceiling frame lift.
        let vault: Vec<&Triangle> = scene
            .triangles
            .iter()
            .filter(|t| {
                let nz = t.geom_normal.z.abs();
                nz > 0.1 && nz < 0.99
            })
            .collect();
        assert!(
            !vault.is_empty(),
            "no tilted vault-ceiling triangle in scissor_scene"
        );
        let zs = || {
            vault
                .iter()
                .flat_map(|t| [t.v0.z, (t.v0 + t.edge1).z, (t.v0 + t.edge2).z])
        };
        let lo = zs().fold(f32::INFINITY, f32::min);
        let hi = zs().fold(f32::NEG_INFINITY, f32::max);
        assert!((lo - 96.0).abs() < 0.5, "vault eave at {lo}, want ~96in");
        assert!((hi - 156.0).abs() < 0.5, "vault ridge at {hi}, want ~156in");
        for t in &scene.triangles {
            assert!(
                t.v0.x.is_finite() && t.geom_normal.x.is_finite() && t.geom_normal.y.is_finite(),
                "non-finite geometry in scissor_scene"
            );
        }
        assert!(!scene.bvh.nodes.is_empty());
    }
}
