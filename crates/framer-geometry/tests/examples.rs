use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use framer_geometry::audit_project;

#[test]
fn every_checked_in_project_has_a_clean_geometry_audit() {
    let mut projects = checked_in_projects();
    projects.sort();
    assert!(!projects.is_empty(), "no checked-in .framer examples found");

    for path in projects {
        let source = fs::read_to_string(&path).unwrap_or_else(|error| {
            panic!("cannot read {}: {error}", path.display());
        });
        let model = framer_core::load_project(&source).unwrap_or_else(|error| {
            panic!("cannot load {}: {error}", path.display());
        });
        let plan = framer_solver::generate_project_plan(&model).unwrap_or_else(|error| {
            panic!("cannot solve {}: {error}", path.display());
        });
        let audit = audit_project(&model, &plan);
        assert!(
            audit.is_clean(),
            "{} has geometry violations:\n{:#?}",
            path.display(),
            audit.violations
        );
    }
}

#[test]
fn geometry_audit_cli_covers_clean_overlap_and_input_errors() {
    let binary = env!("CARGO_BIN_EXE_geometry-audit");
    let clean_path = project_root().join("demo-shell.framer");
    let clean = Command::new(binary).arg(&clean_path).output().unwrap();
    assert!(
        clean.status.success(),
        "{}",
        String::from_utf8_lossy(&clean.stderr)
    );
    assert_eq!(
        String::from_utf8(clean.stdout).unwrap(),
        format!("geometry-audit clean project={}\n", clean_path.display())
    );

    let overlapping = temporary_project("overlap");
    let source = fs::read_to_string(project_root().join("demo-two-bedroom.framer")).unwrap();
    let mut model = framer_core::load_project(&source).unwrap();
    let original = model
        .walls
        .iter()
        .find(|wall| wall.id.0 == "wall-right")
        .unwrap()
        .clone();
    let mut duplicate = original;
    duplicate.id = framer_core::ElementId::new("wall-right-copy");
    duplicate.name = "Right wall copy".to_string();
    duplicate.openings.clear();
    model.walls.push(duplicate);
    fs::write(&overlapping, framer_core::save_project(&model).unwrap()).unwrap();
    let overlap = Command::new(binary).arg(&overlapping).output().unwrap();
    assert_eq!(overlap.status.code(), Some(1));
    let stdout = String::from_utf8(overlap.stdout).unwrap();
    assert!(stdout.starts_with("geometry-audit violations="));
    assert!(
        stdout.contains(
            "body_a=finished-assembly/wall-front/wall body_b=finished-assembly/wall-right-copy/wall"
        ),
        "{stdout}"
    );

    let malformed = temporary_project("malformed");
    fs::write(&malformed, "not a Framer project").unwrap();
    let malformed_output = Command::new(binary).arg(&malformed).output().unwrap();
    assert_eq!(malformed_output.status.code(), Some(2));
    assert!(
        String::from_utf8(malformed_output.stderr)
            .unwrap()
            .contains("cannot load")
    );

    let missing = temporary_project("missing");
    let _ = fs::remove_file(&missing);
    let missing_output = Command::new(binary).arg(&missing).output().unwrap();
    assert_eq!(missing_output.status.code(), Some(2));
    assert!(
        String::from_utf8(missing_output.stderr)
            .unwrap()
            .contains("cannot read")
    );

    for args in [Vec::<&str>::new(), vec!["one", "two"]] {
        let usage = Command::new(binary).args(args).output().unwrap();
        assert_eq!(usage.status.code(), Some(2));
        assert_eq!(
            String::from_utf8(usage.stderr).unwrap(),
            "usage: geometry-audit <project.framer>\n"
        );
    }

    let _ = fs::remove_file(overlapping);
    let _ = fs::remove_file(malformed);
}

fn checked_in_projects() -> Vec<PathBuf> {
    fs::read_dir(project_root())
        .expect("examples/projects directory")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "framer")
        })
        .collect()
}

fn project_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/projects")
}

fn temporary_project(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "framer-geometry-audit-{label}-{}.framer",
        std::process::id()
    ))
}
