use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{BuildingModel, ModelError};

pub const PROJECT_FORMAT: &str = "framer.project";
pub const PROJECT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectDocument {
    pub format: String,
    pub schema_version: u32,
    pub authored: BuildingModel,
}

impl ProjectDocument {
    pub fn new(authored: BuildingModel) -> Self {
        Self {
            format: PROJECT_FORMAT.to_owned(),
            schema_version: PROJECT_SCHEMA_VERSION,
            authored: authored.into_deterministic(),
        }
    }

    pub fn into_model(self) -> BuildingModel {
        self.authored
    }

    pub fn validate(&self) -> Result<(), ProjectError> {
        if self.format != PROJECT_FORMAT {
            return Err(ProjectError::InvalidFormat {
                found: self.format.clone(),
            });
        }

        if self.schema_version != PROJECT_SCHEMA_VERSION {
            return Err(ProjectError::UnsupportedSchemaVersion {
                found: self.schema_version,
                supported: PROJECT_SCHEMA_VERSION,
            });
        }

        self.authored.validate()?;
        Ok(())
    }

    pub fn to_canonical_json(&self) -> Result<String, ProjectError> {
        let mut document = self.clone();
        document.authored.sort_deterministically();
        let mut json = serde_json::to_string_pretty(&document)?;
        json.push('\n');
        Ok(json)
    }
}

pub fn save_project(model: &BuildingModel) -> Result<String, ProjectError> {
    let document = ProjectDocument::new(model.clone());
    document.validate()?;
    document.to_canonical_json()
}

pub fn load_project(source: &str) -> Result<BuildingModel, ProjectError> {
    let mut document: ProjectDocument = serde_json::from_str(source)?;
    document.validate()?;
    document.authored.sort_deterministically();
    Ok(document.into_model())
}

#[derive(Debug, Error)]
pub enum ProjectError {
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("unsupported Framer project format {found:?}")]
    InvalidFormat { found: String },
    #[error("unsupported Framer project schema version {found}; this build supports {supported}")]
    UnsupportedSchemaVersion { found: u32, supported: u32 },
    #[error(transparent)]
    Model(#[from] ModelError),
}

#[cfg(test)]
mod tests {
    use crate::{BuildingModel, CodeProfile, Length, Opening, Wall};

    use super::*;

    #[test]
    fn save_project_writes_schema_versioned_authored_model() {
        let json = save_project(&BuildingModel::demo_wall()).unwrap();

        assert!(json.starts_with("{\n  \"format\": \"framer.project\",\n"));
        assert!(json.contains("  \"schema_version\": 1,\n"));
        assert!(json.contains("  \"authored\": {"));
        assert!(!json.contains("\"generated\""));
        assert!(!json.contains("\"cache\""));
        assert!(!json.contains("\"exports\""));
    }

    #[test]
    fn save_project_is_deterministic_for_reordered_authored_objects() {
        let code = CodeProfile::irc_2021_prescriptive();

        let mut first = BuildingModel::new(code.clone());
        let mut wall = Wall::new("wall-1", "Wall", Length::from_feet(12.0), &code);
        wall.openings.push(Opening::window(
            "opening-b",
            "B",
            Length::from_inches(96.0),
            Length::from_inches(24.0),
            Length::from_inches(24.0),
            Length::from_inches(36.0),
        ));
        wall.openings.push(Opening::door(
            "opening-a",
            "A",
            Length::from_inches(36.0),
            Length::from_inches(24.0),
            Length::from_inches(80.0),
        ));
        first.walls.push(wall.clone());

        let mut second = BuildingModel::new(code);
        wall.openings.reverse();
        second.walls.push(wall);

        assert_eq!(
            save_project(&first).unwrap(),
            save_project(&second).unwrap()
        );
    }

    #[test]
    fn load_project_rejects_unknown_top_level_data() {
        let source = r#"{
  "format": "framer.project",
  "schema_version": 1,
  "authored": {
    "code": {
      "code": "Irc2021",
      "display_name": "IRC 2021 prescriptive starter profile",
      "default_wall_height": {"ticks": 1536},
      "default_stud_spacing": {"ticks": 256},
      "double_top_plate": true,
      "default_header_depth": {"ticks": 144},
      "stud_profile": "TwoByFour",
      "plate_profile": "TwoByFour",
      "header_profile": "TwoByTen"
    },
    "walls": []
  },
  "generated": {}
}"#;

        assert!(matches!(load_project(source), Err(ProjectError::Json(_))));
    }

    #[test]
    fn demo_wall_example_is_canonical() {
        let example = include_str!("../../../examples/projects/demo-wall.framer");

        let model = load_project(example).unwrap();

        assert_eq!(save_project(&model).unwrap(), example);
    }
}
