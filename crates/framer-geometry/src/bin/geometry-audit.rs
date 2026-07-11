use std::process::ExitCode;

fn run(path: &str) -> Result<framer_geometry::GeometryAudit, String> {
    let source =
        std::fs::read_to_string(path).map_err(|error| format!("cannot read {path}: {error}"))?;
    let model = framer_core::load_project(&source)
        .map_err(|error| format!("cannot load {path}: {error}"))?;
    let plan = framer_solver::generate_project_plan(&model)
        .map_err(|error| format!("cannot solve {path}: {error}"))?;
    Ok(framer_geometry::audit_project(&model, &plan))
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: geometry-audit <project.framer>");
        return ExitCode::from(2);
    };
    if args.next().is_some() {
        eprintln!("usage: geometry-audit <project.framer>");
        return ExitCode::from(2);
    }

    match run(&path) {
        Ok(audit) if audit.is_clean() => {
            println!("geometry-audit clean project={path}");
            ExitCode::SUCCESS
        }
        Ok(audit) => {
            println!(
                "geometry-audit violations={} project={path}",
                audit.violations.len()
            );
            for violation in audit.violations {
                println!("{violation}");
            }
            ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("geometry-audit error: {error}");
            ExitCode::from(2)
        }
    }
}
