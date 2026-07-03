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

use framer_render::render;
use framer_render::scene::Scene;
use framer_render::scenes::{
    REFERENCE_HEIGHT as HEIGHT, REFERENCE_SEED as SEED, REFERENCE_SPP as SPP,
    REFERENCE_WIDTH as WIDTH, hip_roof_scene, reference_scene, roofed_scene, scissor_scene,
};

fn golden_path(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/golden/{name}.rgba"))
}

/// Render `scene` and compare it byte-for-byte (within an f32-rounding tolerance)
/// to the committed golden `name`. Regenerate any golden with
/// `UPDATE_GOLDEN=1 cargo test -p framer-render --test golden`.
fn assert_matches_golden(name: &str, scene: &Scene) {
    let image = render(scene, WIDTH, HEIGHT, SPP, SEED);
    assert_eq!(image.len(), (WIDTH * HEIGHT * 4) as usize);

    let path = golden_path(name);
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
        "golden image {name} has a different size"
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
    assert!(
        mae < 1.0,
        "{name}: mean abs error {mae} too high (regression?)"
    );
    assert!(
        max < 12,
        "{name}: max pixel error {max} too high (regression?)"
    );
}

#[test]
fn reference_render_matches_golden() {
    assert_matches_golden("reference", &reference_scene());
}

/// Locks the sloped *ceiling* surfaces: a model-derived scissor-vault shell (two
/// opposing sloped ceilings meeting at a ridge) rendered through the production
/// scene-extraction path — the same frame lift the app's mesher uses.
#[test]
fn scissor_render_matches_golden() {
    assert_matches_golden("scissor", &scissor_scene());
}

/// Locks the sloped roof + horizontal ceiling/floor surfaces: a model-derived
/// gable-roofed shell rendered through the production scene-extraction path.
#[test]
fn roofed_render_matches_golden() {
    assert_matches_golden("roofed", &roofed_scene());
}

/// Locks the multi-plane hip roof path: four authored roof planes, each lifted
/// through its own frame, rendered through the same model-extraction path.
#[test]
fn hip_roof_render_matches_golden() {
    assert_matches_golden("hip-roof", &hip_roof_scene());
}
