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
use framer_render::scenes::{
    REFERENCE_HEIGHT as HEIGHT, REFERENCE_SEED as SEED, REFERENCE_SPP as SPP,
    REFERENCE_WIDTH as WIDTH, reference_scene,
};

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
