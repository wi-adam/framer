//! Integration tests for the headless `render` binary. Gated behind the `cli`
//! feature so the default `cargo test --workspace` (which doesn't build the bin)
//! still compiles. Run with: `cargo test -p framer-render --features cli`.
#![cfg(feature = "cli")]

use std::process::Command;

fn demo_shell_path() -> String {
    format!(
        "{}/../../examples/projects/demo-shell.framer",
        env!("CARGO_MANIFEST_DIR")
    )
}

#[test]
fn renders_demo_shell_to_valid_png() {
    let out = std::env::temp_dir().join("framer_render_cli_demo.png");
    let _ = std::fs::remove_file(&out);

    let status = Command::new(env!("CARGO_BIN_EXE_render"))
        .args([
            &demo_shell_path(),
            out.to_str().unwrap(),
            "--width",
            "80",
            "--height",
            "56",
            "--spp",
            "4",
        ])
        .status()
        .expect("failed to launch render binary");
    assert!(status.success(), "render exited with failure");

    let bytes = std::fs::read(&out).expect("expected an output PNG");
    assert!(bytes.len() > 100, "PNG suspiciously small");
    // PNG magic number.
    assert_eq!(
        &bytes[..8],
        &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A],
        "output is not a PNG"
    );
    let _ = std::fs::remove_file(&out);
}

#[test]
fn fails_clearly_on_missing_input() {
    let status = Command::new(env!("CARGO_BIN_EXE_render"))
        .args(["/no/such/project.framer", "/tmp/should_not_exist.png"])
        .status()
        .expect("failed to launch render binary");
    assert!(!status.success(), "missing input should be a non-zero exit");
}

#[test]
fn fails_on_bad_arguments() {
    let status = Command::new(env!("CARGO_BIN_EXE_render"))
        .args(["only-one-arg"])
        .status()
        .expect("failed to launch render binary");
    assert!(!status.success());
}
