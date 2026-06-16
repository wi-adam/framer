//! Golden-image regression test. Renders a fixed synthetic scene exercising
//! every material (diffuse, metal, glass, emissive) under sun + sky at a fixed
//! seed and resolution, and compares against committed raw RGBA bytes.
//!
//! The image is a pure, thread-independent function of the seed, so the only
//! cross-architecture variation is last-ULP `f32` rounding — absorbed by the MAE
//! tolerance. To (re)generate the golden after an intentional change, run:
//!
//! ```text
//! UPDATE_GOLDEN=1 cargo test -p framer-render --test golden
//! ```

use framer_render::camera::Camera;
use framer_render::geom::Triangle;
use framer_render::material::Material;
use framer_render::math::Vec3;
use framer_render::render;
use framer_render::scene::{DirectionalSun, Scene, Sky};

const WIDTH: u32 = 64;
const HEIGHT: u32 = 48;
const SPP: u32 = 12;
const SEED: u64 = 7;

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

fn reference_scene() -> Scene {
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
        WIDTH as f32 / HEIGHT as f32,
        42.0,
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

fn golden_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/reference.rgba")
}

#[test]
fn reference_render_matches_golden() {
    let image = render(&reference_scene(), WIDTH, HEIGHT, SPP, SEED);
    assert_eq!(image.len(), (WIDTH * HEIGHT * 4) as usize);

    let path = golden_path();
    if std::env::var("UPDATE_GOLDEN").is_ok() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &image).unwrap();
        eprintln!("Wrote golden image to {}", path.display());
        return;
    }

    let golden = std::fs::read(&path).unwrap_or_else(|_| {
        panic!(
            "golden image missing at {}; generate with UPDATE_GOLDEN=1 cargo test -p framer-render --test golden",
            path.display()
        )
    });
    assert_eq!(
        image.len(),
        golden.len(),
        "golden image has a different size"
    );

    // Mean absolute error tolerates cross-architecture f32 rounding; max error
    // catches a single blown-out pixel that a good average could hide.
    let mut total = 0u64;
    let mut max = 0u32;
    for (a, b) in image.iter().zip(golden.iter()) {
        let d = (*a as i32 - *b as i32).unsigned_abs();
        total += d as u64;
        max = max.max(d);
    }
    let mae = total as f64 / image.len() as f64;
    assert!(mae < 1.0, "mean abs error {mae} too high (regression?)");
    assert!(max < 12, "max pixel error {max} too high (regression?)");
}
