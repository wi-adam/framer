use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{BuildingModel, ModelError};

pub const PROJECT_FORMAT: &str = "framer.project";
pub const PROJECT_SCHEMA_VERSION: u32 = 8;
/// The model is v8-only — older on-disk shapes and pre-provenance schemas are no
/// longer representable, so loading them must fail with a clear
/// unsupported-schema error rather than confusing serde errors.
const MIN_SUPPORTED_SCHEMA_VERSION: u32 = PROJECT_SCHEMA_VERSION;

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

        if !(MIN_SUPPORTED_SCHEMA_VERSION..=PROJECT_SCHEMA_VERSION).contains(&self.schema_version) {
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
        document.schema_version = PROJECT_SCHEMA_VERSION;
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
    // Peek the format/version header before deserializing into the v8-only model,
    // so an old schema fails with an explicit unsupported-schema error instead of
    // serde errors about fields that no longer exist in the current model shape.
    let header: SchemaHeader = serde_json::from_str(source)?;
    if header.format != PROJECT_FORMAT {
        return Err(ProjectError::InvalidFormat {
            found: header.format,
        });
    }
    if !(MIN_SUPPORTED_SCHEMA_VERSION..=PROJECT_SCHEMA_VERSION).contains(&header.schema_version) {
        return Err(ProjectError::UnsupportedSchemaVersion {
            found: header.schema_version,
            supported: PROJECT_SCHEMA_VERSION,
        });
    }

    let mut document: ProjectDocument = serde_json::from_str(source)?;
    document.validate()?;
    document.authored.sort_deterministically();
    Ok(document.into_model())
}

/// A minimal view of a project file's header, used to reject unsupported formats
/// and schema versions before attempting the full (v8-only) deserialization.
/// Deliberately omits `deny_unknown_fields` so it ignores `authored` and any
/// other body fields, including ones from older schemas.
#[derive(Deserialize)]
struct SchemaHeader {
    #[serde(default)]
    format: String,
    schema_version: u32,
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
    use crate::{
        BuildingModel, CodeProfile, ElementId, Length, LibraryStamp, MaterialSource, ModelError,
        Opening, Provenance, Wall,
    };

    use super::*;

    #[test]
    fn save_project_writes_schema_versioned_authored_model() {
        let json = save_project(&BuildingModel::demo_wall()).unwrap();

        assert!(json.starts_with("{\n  \"format\": \"framer.project\",\n"));
        assert!(json.contains("  \"schema_version\": 8,\n"));
        assert!(json.contains("  \"authored\": {"));
        assert!(json.contains("    \"levels\": ["));
        assert!(json.contains("    \"wall_joins\": ["));
        assert!(!json.contains("\"generated\""));
        assert!(!json.contains("\"cache\""));
        assert!(!json.contains("\"exports\""));
    }

    #[test]
    fn no_library_model_omits_provenance_fields() {
        let json = save_project(&BuildingModel::demo_wall()).unwrap();

        assert!(!json.contains("\"libraries\""));
        assert!(!json.contains("\"source\""));
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
  "schema_version": 8,
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
    fn load_project_rejects_old_schema_with_unsupported_version_error() {
        // A v7 document must be rejected by the header peek with a clear
        // unsupported-schema error, NOT serde errors from the current v8 model.
        // The pre-v8 body remains self-contained, but schema support is v8-only.
        let source = r#"{
  "format": "framer.project",
  "schema_version": 7,
  "authored": {
    "walls": [
      {
        "id": "wall-1",
        "name": "Wall",
        "length": {"ticks": 2304},
        "height": {"ticks": 1536},
        "assembly": "WoodStud2x4",
        "stud_spacing": {"ticks": 256},
        "openings": []
      }
    ]
  }
}"#;

        assert!(matches!(
            load_project(source),
            Err(ProjectError::UnsupportedSchemaVersion {
                found: 7,
                supported: PROJECT_SCHEMA_VERSION
            })
        ));
    }

    #[test]
    fn demo_two_bedroom_example_is_canonical() {
        let example = include_str!("../../../examples/projects/demo-two-bedroom.framer");

        let model = load_project(example).unwrap();

        assert_eq!(model.walls.len(), 6);
        assert_eq!(model.rooms.len(), 3);
        assert_eq!(
            model
                .wall_joins
                .iter()
                .filter(|join| join.kind == crate::WallJoinKind::Tee)
                .count(),
            4
        );
        assert_eq!(save_project(&model).unwrap(), example);
    }

    #[test]
    fn demo_wall_example_is_canonical() {
        let example = include_str!("../../../examples/projects/demo-wall.framer");

        let model = load_project(example).unwrap();

        assert_eq!(save_project(&model).unwrap(), example);
    }

    #[test]
    fn demo_shell_example_is_canonical() {
        let example = include_str!("../../../examples/projects/demo-shell.framer");

        let model = load_project(example).unwrap();

        assert_eq!(model.walls.len(), 4);
        assert_eq!(model.wall_joins.len(), 4);
        assert_eq!(save_project(&model).unwrap(), example);
    }

    #[test]
    fn wall_system_and_tags_round_trip() {
        use crate::ElementId;

        let code = CodeProfile::irc_2021_prescriptive();
        let mut model = BuildingModel::new(code.clone());
        let mut wall = Wall::new("wall-1", "Wall", Length::from_feet(12.0), &code);
        wall.system = ElementId::new("system-wall-interior-1");
        wall.tags = vec!["load-bearing".to_owned(), "shear".to_owned()];
        model.walls.push(wall);

        let json = save_project(&model).unwrap();
        assert!(json.contains("\"system\": \"system-wall-interior-1\""));
        assert!(json.contains("\"systems\": ["));
        assert!(json.contains("\"materials\": ["));
        assert!(json.contains("\"function\": \"Framing\""));
        assert!(json.contains("\"appearance\": {"));

        let reloaded = load_project(&json).unwrap();
        let wall = &reloaded.walls[0];
        assert_eq!(wall.system, ElementId::new("system-wall-interior-1"));
        let system = reloaded.system_for(wall).unwrap();
        assert_eq!(system.exposure(), crate::WallExposure::Interior);
        assert_eq!(
            system.framing_layer().unwrap().function,
            crate::LayerFunction::Framing
        );
        assert!(reloaded.material(&ElementId::new("mat-drywall")).is_some());
        assert_eq!(
            wall.tags,
            vec!["load-bearing".to_owned(), "shear".to_owned()]
        );
    }

    #[test]
    fn material_properties_round_trip_deterministically() {
        use crate::{Appearance, Material, PropertyValue};

        let code = CodeProfile::irc_2021_prescriptive();

        let mut first = BuildingModel::new(code.clone());
        let mut material = Material::solid_color("mat-custom", "Custom", [10, 20, 30]);
        material
            .properties
            .insert("r_per_inch_milli".to_owned(), PropertyValue::Int(3300));
        material
            .properties
            .insert("cost_cents".to_owned(), PropertyValue::Int(125));
        material
            .properties
            .insert("note".to_owned(), PropertyValue::Text("spec".to_owned()));
        first.materials.push(material.clone());

        // The same material with its property map inserted in a different order
        // must serialize identically (BTreeMap => ordered).
        let mut second = BuildingModel::new(code);
        let mut reordered = Material::solid_color("mat-custom", "Custom", [10, 20, 30]);
        reordered
            .properties
            .insert("note".to_owned(), PropertyValue::Text("spec".to_owned()));
        reordered
            .properties
            .insert("cost_cents".to_owned(), PropertyValue::Int(125));
        reordered
            .properties
            .insert("r_per_inch_milli".to_owned(), PropertyValue::Int(3300));
        second.materials.push(reordered);

        assert_eq!(
            save_project(&first).unwrap(),
            save_project(&second).unwrap()
        );

        let reloaded = load_project(&save_project(&first).unwrap()).unwrap();
        let material = reloaded
            .material(&crate::ElementId::new("mat-custom"))
            .unwrap();
        assert_eq!(material.appearance, Appearance::SolidColor([10, 20, 30]));
        assert_eq!(material.r_per_inch_milli(), 3300);
    }

    #[test]
    fn library_provenance_round_trips_with_identity_table() {
        let mut model = BuildingModel::new(CodeProfile::irc_2021_prescriptive());
        model.libraries.push(LibraryStamp {
            uid: "11111111-1111-4111-8111-111111111111".to_owned(),
            version_id: "019e9150-0000-7000-8000-000000000001".to_owned(),
            content_hash: "blake3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .to_owned(),
            coordinate: "framer-lib://round-trip/sentinel".to_owned(),
            version: "1.0.0".to_owned(),
        });
        let material_source = Provenance {
            library_uid: "11111111-1111-4111-8111-111111111111".to_owned(),
            version_id: "019e9150-0000-7000-8000-000000000001".to_owned(),
            source_id: ElementId::new("mat-drywall"),
            content_hash: "blake3:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                .to_owned(),
        };
        model.materials[0].source = MaterialSource::Library(material_source.clone());
        model.systems[0].source = Some(Provenance {
            source_id: ElementId::new("system-wall-exterior-1"),
            ..material_source
        });

        let reloaded = load_project(&save_project(&model).unwrap()).unwrap();

        assert_eq!(reloaded.libraries.len(), 1);
        assert!(matches!(
            &reloaded.materials[0].source,
            MaterialSource::Library(source)
                if source.source_id == ElementId::new("mat-drywall")
        ));
        assert_eq!(
            reloaded.systems[0].source.as_ref().unwrap().source_id,
            ElementId::new("system-wall-exterior-1")
        );
    }

    #[test]
    fn validation_rejects_dangling_wall_system() {
        let code = CodeProfile::irc_2021_prescriptive();
        let mut model = BuildingModel::new(code.clone());
        let mut wall = Wall::new("wall-1", "Wall", Length::from_feet(12.0), &code);
        wall.system = ElementId::new("system-does-not-exist");
        model.walls.push(wall);

        assert!(matches!(
            save_project(&model),
            Err(ProjectError::Model(
                ModelError::WallReferencesUnknownSystem { .. }
            ))
        ));
    }

    #[test]
    fn room_round_trips_through_save_and_load() {
        use crate::{Point2, Room, RoomUsage};

        let code = CodeProfile::irc_2021_prescriptive();
        let mut model = BuildingModel::new(code.clone());
        model
            .walls
            .push(Wall::new("wall-1", "Wall", Length::from_feet(12.0), &code));
        model.rooms.push(Room::new(
            "room-1",
            "Living room",
            RoomUsage::Living,
            "level-1",
            Point2::new(Length::from_feet(6.0), Length::from_feet(6.0)),
        ));

        let json = save_project(&model).unwrap();
        assert!(json.contains("\"schema_version\": 8,"));
        assert!(json.contains("\"rooms\": ["));
        assert!(json.contains("\"usage\": \"Living\""));

        let reloaded = load_project(&json).unwrap();
        assert_eq!(reloaded.rooms.len(), 1);
        assert_eq!(reloaded.rooms[0].usage, RoomUsage::Living);
        assert_eq!(
            reloaded.rooms[0].seed,
            Point2::new(Length::from_feet(6.0), Length::from_feet(6.0))
        );
    }
}
