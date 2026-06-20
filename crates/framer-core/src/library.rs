use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    ConstructionSystem, Material, ModelError,
    model::{insert_unique_id, validate_element_id},
};

pub const LIBRARY_FORMAT: &str = "framer.library";
pub const LIBRARY_SCHEMA_VERSION: u32 = 1;
const MIN_SUPPORTED_LIBRARY_SCHEMA_VERSION: u32 = LIBRARY_SCHEMA_VERSION;
const STARTER_LIBRARY_SOURCE: &str = include_str!("../../../libraries/framer-starter.framerlib");

static STARTER_LIBRARY: OnceLock<Library> = OnceLock::new();

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Library {
    pub uid: String,
    pub version_id: String,
    pub version: String,
    pub coordinate: String,
    #[serde(default)]
    pub materials: Vec<Material>,
    #[serde(default)]
    pub systems: Vec<ConstructionSystem>,
}

impl Library {
    pub fn validate(&self) -> Result<(), ModelError> {
        let mut ids = BTreeSet::new();
        let mut material_lookup = BTreeMap::new();

        for material in &self.materials {
            validate_element_id(&material.id)?;
            insert_unique_id(&mut ids, &material.id)?;
            material_lookup.insert(material.id.clone(), material);
        }

        for system in &self.systems {
            system.validate(&material_lookup, &mut ids)?;
        }

        Ok(())
    }

    pub fn sort_deterministically(&mut self) {
        self.materials.sort_by(|left, right| left.id.cmp(&right.id));
        self.systems.sort_by(|left, right| left.id.cmp(&right.id));
    }

    pub fn into_deterministic(mut self) -> Self {
        self.sort_deterministically();
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LibraryDocument {
    // Keep the library fields explicit rather than `#[serde(flatten)] library:
    // Library`; serde flattening does not support `deny_unknown_fields`, and
    // `.framerlib` must reject unknown top-level keys just like `.framer`.
    pub format: String,
    pub schema_version: u32,
    pub uid: String,
    pub version_id: String,
    pub version: String,
    pub coordinate: String,
    #[serde(default)]
    pub materials: Vec<Material>,
    #[serde(default)]
    pub systems: Vec<ConstructionSystem>,
}

impl LibraryDocument {
    pub fn new(library: Library) -> Self {
        let library = library.into_deterministic();
        Self {
            format: LIBRARY_FORMAT.to_owned(),
            schema_version: LIBRARY_SCHEMA_VERSION,
            uid: library.uid,
            version_id: library.version_id,
            version: library.version,
            coordinate: library.coordinate,
            materials: library.materials,
            systems: library.systems,
        }
    }

    pub fn into_library(self) -> Library {
        Library {
            uid: self.uid,
            version_id: self.version_id,
            version: self.version,
            coordinate: self.coordinate,
            materials: self.materials,
            systems: self.systems,
        }
    }

    pub fn validate(&self) -> Result<(), LibraryError> {
        if self.format != LIBRARY_FORMAT {
            return Err(LibraryError::InvalidFormat {
                found: self.format.clone(),
            });
        }

        if !(MIN_SUPPORTED_LIBRARY_SCHEMA_VERSION..=LIBRARY_SCHEMA_VERSION)
            .contains(&self.schema_version)
        {
            return Err(LibraryError::UnsupportedSchemaVersion {
                found: self.schema_version,
                supported: LIBRARY_SCHEMA_VERSION,
            });
        }

        self.clone().into_library().validate()?;
        Ok(())
    }

    pub fn to_canonical_json(&self) -> Result<String, LibraryError> {
        let mut document = self.clone();
        document.schema_version = LIBRARY_SCHEMA_VERSION;
        document.sort_deterministically();
        let mut json = serde_json::to_string_pretty(&document)?;
        json.push('\n');
        Ok(json)
    }

    fn sort_deterministically(&mut self) {
        self.materials.sort_by(|left, right| left.id.cmp(&right.id));
        self.systems.sort_by(|left, right| left.id.cmp(&right.id));
    }
}

pub fn save_library(library: &Library) -> Result<String, LibraryError> {
    let document = LibraryDocument::new(library.clone());
    document.validate()?;
    document.to_canonical_json()
}

pub fn load_library(source: &str) -> Result<Library, LibraryError> {
    let header: LibrarySchemaHeader = serde_json::from_str(source)?;
    if header.format != LIBRARY_FORMAT {
        return Err(LibraryError::InvalidFormat {
            found: header.format,
        });
    }
    if !(MIN_SUPPORTED_LIBRARY_SCHEMA_VERSION..=LIBRARY_SCHEMA_VERSION)
        .contains(&header.schema_version)
    {
        return Err(LibraryError::UnsupportedSchemaVersion {
            found: header.schema_version,
            supported: LIBRARY_SCHEMA_VERSION,
        });
    }

    let mut document: LibraryDocument = serde_json::from_str(source)?;
    document.validate()?;
    document.sort_deterministically();
    Ok(document.into_library())
}

pub(crate) fn starter_library() -> Library {
    STARTER_LIBRARY
        .get_or_init(|| {
            load_library(STARTER_LIBRARY_SOURCE).expect("checked-in starter .framerlib is valid")
        })
        .clone()
}

#[derive(Deserialize)]
struct LibrarySchemaHeader {
    #[serde(default)]
    format: String,
    schema_version: u32,
}

#[derive(Debug, Error)]
pub enum LibraryError {
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("unsupported Framer library format {found:?}")]
    InvalidFormat { found: String },
    #[error("unsupported Framer library schema version {found}; this build supports {supported}")]
    UnsupportedSchemaVersion { found: u32, supported: u32 },
    #[error(transparent)]
    Model(#[from] ModelError),
}

#[cfg(test)]
mod tests {
    use crate::{
        ConstructionLayer, ConstructionSystem, ElementId, LayerFunction, Length, Material,
        SystemKind,
    };

    use super::*;

    fn checked_in_starter_library() -> Library {
        load_library(STARTER_LIBRARY_SOURCE).unwrap()
    }

    #[test]
    fn save_library_writes_schema_versioned_document() {
        let json = save_library(&checked_in_starter_library()).unwrap();

        assert!(json.starts_with("{\n  \"format\": \"framer.library\",\n"));
        assert!(json.contains("  \"schema_version\": 1,\n"));
        assert!(json.contains("  \"uid\": \"8f6ebee0-fbdc-4f29-9d90-0e3f3f0640a8\",\n"));
        assert!(json.contains("  \"materials\": ["));
        assert!(json.contains("  \"systems\": ["));
        assert!(!json.contains("\"authored\""));
    }

    #[test]
    fn save_library_is_deterministic_for_reordered_definitions() {
        let mut first = checked_in_starter_library();
        first.materials.reverse();
        first.systems.reverse();

        let second = checked_in_starter_library();

        assert_eq!(
            save_library(&first).unwrap(),
            save_library(&second).unwrap()
        );
    }

    #[test]
    fn library_round_trip_preserves_header_fields_and_definitions() {
        let library = Library {
            uid: "11111111-1111-4111-8111-111111111111".to_owned(),
            version_id: "019e9150-0000-7000-8000-000000000001".to_owned(),
            version: "2.3.4-sentinel".to_owned(),
            coordinate: "framer-lib://round-trip/sentinel".to_owned(),
            materials: vec![
                Material::solid_color("mat-round-trip", "Round-trip material", [1, 2, 3])
                    .with_tags(["sentinel"])
                    .with_r_per_inch_milli(3210),
            ],
            systems: vec![ConstructionSystem {
                id: ElementId::new("system-round-trip"),
                name: "Round-trip system".to_owned(),
                kind: SystemKind::Floor,
                layers: vec![ConstructionLayer::new(
                    LayerFunction::InteriorFinish,
                    "mat-round-trip",
                    Length::from_whole_inches(1),
                )],
            }],
        };

        let reloaded = load_library(&save_library(&library).unwrap()).unwrap();

        assert_eq!(reloaded, library.into_deterministic());
    }

    #[test]
    fn checked_in_starter_library_is_canonical() {
        let library = checked_in_starter_library();

        assert_eq!(save_library(&library).unwrap(), STARTER_LIBRARY_SOURCE);
    }

    #[test]
    fn starter_library_contains_previous_seed_catalog() {
        let library = starter_library();

        assert_eq!(
            library
                .materials
                .iter()
                .map(|material| material.id.0.as_str())
                .collect::<Vec<_>>(),
            vec![
                "mat-drywall",
                "mat-fiber-cement",
                "mat-mineral-wool",
                "mat-plywood",
                "mat-polyiso",
                "mat-rainscreen",
                "mat-spf",
            ]
        );
        assert_eq!(
            library
                .systems
                .iter()
                .map(|system| system.id.0.as_str())
                .collect::<Vec<_>>(),
            vec!["system-wall-exterior-1", "system-wall-interior-1"]
        );

        let exterior = library
            .systems
            .iter()
            .find(|system| system.id == ElementId::new("system-wall-exterior-1"))
            .unwrap();
        assert_eq!(exterior.kind, SystemKind::Wall);
        assert_eq!(exterior.layers.len(), 6);
        assert_eq!(exterior.layers[0].function, LayerFunction::InteriorFinish);
        assert_eq!(
            exterior
                .framing_layer()
                .unwrap()
                .framing
                .as_ref()
                .unwrap()
                .cavity_material,
            Some(ElementId::new("mat-mineral-wool"))
        );

        let interior = library
            .systems
            .iter()
            .find(|system| system.id == ElementId::new("system-wall-interior-1"))
            .unwrap();
        assert_eq!(interior.kind, SystemKind::Wall);
        assert_eq!(interior.layers.len(), 3);
        assert!(
            interior
                .framing_layer()
                .unwrap()
                .framing
                .as_ref()
                .unwrap()
                .cavity_material
                .is_none()
        );
    }

    #[test]
    fn load_library_rejects_unknown_top_level_data() {
        let source = r#"{
  "format": "framer.library",
  "schema_version": 1,
  "uid": "8f6ebee0-fbdc-4f29-9d90-0e3f3f0640a8",
  "version_id": "019e8b10-9b30-7c2b-8b4e-1db251cb8221",
  "version": "0.1.0",
  "coordinate": "framer-lib://framer/starter",
  "materials": [],
  "systems": [],
  "generated": {}
}"#;

        assert!(matches!(load_library(source), Err(LibraryError::Json(_))));
    }

    #[test]
    fn load_library_rejects_project_format() {
        let source = r#"{
  "format": "framer.project",
  "schema_version": 1
}"#;

        assert!(matches!(
            load_library(source),
            Err(LibraryError::InvalidFormat { found }) if found == "framer.project"
        ));
    }

    #[test]
    fn load_library_rejects_old_schema_with_unsupported_version_error() {
        let source = r#"{
  "format": "framer.library",
  "schema_version": 0
}"#;

        assert!(matches!(
            load_library(source),
            Err(LibraryError::UnsupportedSchemaVersion {
                found: 0,
                supported: LIBRARY_SCHEMA_VERSION
            })
        ));
    }

    #[test]
    fn validate_rejects_dangling_layer_material_reference() {
        let mut library = checked_in_starter_library();
        library.systems[0].layers[0].material = ElementId::new("mat-missing");

        assert!(matches!(
            save_library(&library),
            Err(LibraryError::Model(
                ModelError::LayerReferencesUnknownMaterial { .. }
            ))
        ));
    }
}
