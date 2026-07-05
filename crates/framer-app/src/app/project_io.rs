use std::fs;
use std::path::{Path, PathBuf};

pub(super) const DEFAULT_PROJECT_PATH: &str = "examples/projects/demo-shell.framer";

pub(super) fn write_text_file(path: &Path, contents: String) -> Result<(), String> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::write(path, contents).map_err(|error| error.to_string())
}

pub(super) fn export_paths(project_path: &str) -> (PathBuf, PathBuf) {
    let base = export_base(project_path);
    (base.with_extension("svg"), base.with_extension("csv"))
}

pub(super) fn compliance_report_path(project_path: &str) -> PathBuf {
    export_base(project_path).with_extension("compliance.csv")
}

fn export_base(project_path: &str) -> PathBuf {
    let trimmed = project_path.trim();
    if trimmed.is_empty() {
        PathBuf::from("framer-export.framer")
    } else {
        PathBuf::from(trimmed)
    }
}
