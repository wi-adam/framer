use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::{Path, PathBuf},
    sync::OnceLock,
};

use framer_core::{
    BuildingModel, ConstructionSystem, ElementId, Library, LibraryError, LibraryStamp, Material,
    MaterialSource, ModelError, Provenance, load_library, save_library,
};
use thiserror::Error;

const STARTER_LIBRARY_SOURCE: &str = include_str!("../../../libraries/framer-starter.framerlib");
static STARTER_LIBRARY: OnceLock<LoadedLibrary> = OnceLock::new();

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Locator {
    Builtin { id: String },
    Local { path: PathBuf },
    Installed { id: String },
    Remote { url: String, hash: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryBytes {
    pub source: String,
    pub expected_hash: Option<String>,
}

pub trait LibraryResolver {
    fn resolve(&self, locator: &Locator) -> Result<LibraryBytes, LibraryImportError>;
}

#[derive(Debug, Clone, Default)]
pub struct LocalSearchPathResolver {
    roots: Vec<PathBuf>,
}

impl LocalSearchPathResolver {
    pub fn new(roots: impl IntoIterator<Item = PathBuf>) -> Self {
        Self {
            roots: roots.into_iter().collect(),
        }
    }
}

impl LibraryResolver for LocalSearchPathResolver {
    fn resolve(&self, locator: &Locator) -> Result<LibraryBytes, LibraryImportError> {
        match locator {
            Locator::Builtin { id } if id == "framer-starter" => Ok(LibraryBytes {
                source: STARTER_LIBRARY_SOURCE.to_owned(),
                expected_hash: None,
            }),
            Locator::Builtin { id } => Err(LibraryImportError::UnknownBuiltin { id: id.clone() }),
            Locator::Local { path } => Ok(LibraryBytes {
                source: read_library_source(path)?,
                expected_hash: None,
            }),
            Locator::Installed { id } => {
                let relative = if id.ends_with(".framerlib") {
                    PathBuf::from(id)
                } else {
                    PathBuf::from(format!("{id}.framerlib"))
                };
                for root in &self.roots {
                    let path = root.join(&relative);
                    if path.is_file() {
                        return Ok(LibraryBytes {
                            source: read_library_source(&path)?,
                            expected_hash: None,
                        });
                    }
                }
                Err(LibraryImportError::InstalledLibraryNotFound { id: id.clone() })
            }
            Locator::Remote { .. } => Err(LibraryImportError::RemoteUnsupported),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedLibrary {
    pub library: Library,
    pub content_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LibraryItem {
    Material(ElementId),
    System(ElementId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportResult {
    pub materials: Vec<ElementId>,
    pub system: Option<ElementId>,
}

pub fn load_verified_library(bytes: &LibraryBytes) -> Result<LoadedLibrary, LibraryImportError> {
    let library = load_library(&bytes.source)?;
    let content_hash = library_content_hash(&library)?;
    if let Some(expected) = &bytes.expected_hash
        && expected != &content_hash
    {
        return Err(LibraryImportError::ContentHashMismatch {
            expected: expected.clone(),
            actual: content_hash,
        });
    }
    Ok(LoadedLibrary {
        library,
        content_hash,
    })
}

pub fn import_from_locator(
    project: &mut BuildingModel,
    resolver: &impl LibraryResolver,
    locator: &Locator,
    item: LibraryItem,
) -> Result<ImportResult, LibraryImportError> {
    let bytes = resolver.resolve(locator)?;
    let loaded = load_verified_library(&bytes)?;
    import_item(project, &loaded.library, &loaded.content_hash, item)
}

pub fn import_item(
    project: &mut BuildingModel,
    library: &Library,
    library_content_hash: &str,
    item: LibraryItem,
) -> Result<ImportResult, LibraryImportError> {
    match item {
        LibraryItem::Material(id) => import_material(project, library, library_content_hash, &id),
        LibraryItem::System(id) => import_system(project, library, library_content_hash, &id),
    }
}

pub fn import_material(
    project: &mut BuildingModel,
    library: &Library,
    library_content_hash: &str,
    source_id: &ElementId,
) -> Result<ImportResult, LibraryImportError> {
    let source = library_material(library, source_id)?.clone();
    let prefix = library_id_prefix(library);
    let mut staged = project.clone();
    let mut used = collect_project_ids(&staged);
    let remapped = mint_project_id(&prefix, &source.id, &mut used);
    let material = vendor_material(library, &source, remapped.clone())?;

    ensure_library_stamp(&mut staged, library, library_content_hash);
    staged.materials.push(material);
    staged.sort_deterministically();
    staged.validate()?;
    *project = staged;

    Ok(ImportResult {
        materials: vec![remapped],
        system: None,
    })
}

pub fn import_system(
    project: &mut BuildingModel,
    library: &Library,
    library_content_hash: &str,
    source_id: &ElementId,
) -> Result<ImportResult, LibraryImportError> {
    let source_system = library_system(library, source_id)?.clone();
    let prefix = library_id_prefix(library);
    let material_lookup = library
        .materials
        .iter()
        .map(|material| (material.id.clone(), material))
        .collect::<BTreeMap<_, _>>();

    let mut referenced_materials = BTreeSet::new();
    for layer in &source_system.layers {
        referenced_materials.insert(layer.material.clone());
        if let Some(framing) = &layer.framing
            && let Some(cavity) = &framing.cavity_material
        {
            referenced_materials.insert(cavity.clone());
        }
    }

    let mut staged = project.clone();
    let mut used = collect_project_ids(&staged);
    let mut remap = BTreeMap::new();
    let mut imported_materials = Vec::new();
    for source_material_id in referenced_materials {
        let source_material = material_lookup.get(&source_material_id).ok_or_else(|| {
            LibraryImportError::MissingReferencedMaterial {
                system: source_system.id.clone(),
                material: source_material_id.clone(),
            }
        })?;
        let remapped = mint_project_id(&prefix, &source_material.id, &mut used);
        remap.insert(source_material.id.clone(), remapped.clone());
        imported_materials.push(vendor_material(library, source_material, remapped)?);
    }

    let remapped_system = mint_project_id(&prefix, &source_system.id, &mut used);
    let system = vendor_system(library, &source_system, remapped_system.clone(), &remap)?;

    ensure_library_stamp(&mut staged, library, library_content_hash);
    let material_ids = imported_materials
        .iter()
        .map(|material| material.id.clone())
        .collect::<Vec<_>>();
    staged.materials.extend(imported_materials);
    staged.systems.push(system);
    staged.sort_deterministically();
    staged.validate()?;
    *project = staged;

    Ok(ImportResult {
        materials: material_ids,
        system: Some(remapped_system),
    })
}

pub fn library_content_hash(library: &Library) -> Result<String, LibraryImportError> {
    Ok(hash_bytes(save_library(library)?.as_bytes()))
}

pub fn material_content_hash(material: &Material) -> Result<String, LibraryImportError> {
    let mut material = material.clone();
    material.source = MaterialSource::Project;
    hash_json(&material)
}

pub fn system_content_hash(system: &ConstructionSystem) -> Result<String, LibraryImportError> {
    let mut system = system.clone();
    system.source = None;
    hash_json(&system)
}

pub fn starter_library() -> Result<LoadedLibrary, LibraryImportError> {
    if let Some(library) = STARTER_LIBRARY.get() {
        return Ok(library.clone());
    }

    let loaded = load_verified_library(&LibraryBytes {
        source: STARTER_LIBRARY_SOURCE.to_owned(),
        expected_hash: None,
    })?;
    let _ = STARTER_LIBRARY.set(loaded.clone());
    Ok(loaded)
}

fn vendor_material(
    library: &Library,
    source: &Material,
    remapped: ElementId,
) -> Result<Material, LibraryImportError> {
    let mut material = source.clone();
    let content_hash = material_content_hash(source)?;
    material.id = remapped;
    material.source = MaterialSource::Library(provenance(library, &source.id, content_hash));
    Ok(material)
}

fn vendor_system(
    library: &Library,
    source: &ConstructionSystem,
    remapped: ElementId,
    material_remap: &BTreeMap<ElementId, ElementId>,
) -> Result<ConstructionSystem, LibraryImportError> {
    let mut system = source.clone();
    let content_hash = system_content_hash(source)?;
    system.id = remapped;
    system.source = Some(provenance(library, &source.id, content_hash));
    for layer in &mut system.layers {
        layer.material = material_remap
            .get(&layer.material)
            .cloned()
            .ok_or_else(|| LibraryImportError::MissingReferencedMaterial {
                system: source.id.clone(),
                material: layer.material.clone(),
            })?;
        if let Some(framing) = &mut layer.framing
            && let Some(cavity) = &mut framing.cavity_material
        {
            *cavity = material_remap.get(cavity).cloned().ok_or_else(|| {
                LibraryImportError::MissingReferencedMaterial {
                    system: source.id.clone(),
                    material: cavity.clone(),
                }
            })?;
        }
    }
    Ok(system)
}

fn provenance(library: &Library, source_id: &ElementId, content_hash: String) -> Provenance {
    Provenance {
        library_uid: library.uid.clone(),
        version_id: library.version_id.clone(),
        source_id: source_id.clone(),
        content_hash,
    }
}

fn ensure_library_stamp(project: &mut BuildingModel, library: &Library, content_hash: &str) {
    if project
        .libraries
        .iter()
        .any(|stamp| stamp.uid == library.uid && stamp.version_id == library.version_id)
    {
        return;
    }
    project.libraries.push(LibraryStamp {
        uid: library.uid.clone(),
        version_id: library.version_id.clone(),
        content_hash: content_hash.to_owned(),
        coordinate: library.coordinate.clone(),
        version: library.version.clone(),
    });
}

fn library_material<'a>(
    library: &'a Library,
    source_id: &ElementId,
) -> Result<&'a Material, LibraryImportError> {
    library
        .materials
        .iter()
        .find(|material| material.id == *source_id)
        .ok_or_else(|| LibraryImportError::MaterialNotFound {
            id: source_id.clone(),
        })
}

fn library_system<'a>(
    library: &'a Library,
    source_id: &ElementId,
) -> Result<&'a ConstructionSystem, LibraryImportError> {
    library
        .systems
        .iter()
        .find(|system| system.id == *source_id)
        .ok_or_else(|| LibraryImportError::SystemNotFound {
            id: source_id.clone(),
        })
}

fn hash_json<T: serde::Serialize>(value: &T) -> Result<String, LibraryImportError> {
    let mut json = serde_json::to_string_pretty(value)?;
    json.push('\n');
    Ok(hash_bytes(json.as_bytes()))
}

fn hash_bytes(bytes: &[u8]) -> String {
    format!("blake3:{}", blake3::hash(bytes).to_hex())
}

fn read_library_source(path: &Path) -> Result<String, LibraryImportError> {
    fs::read_to_string(path).map_err(|source| LibraryImportError::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn collect_project_ids(project: &BuildingModel) -> BTreeSet<ElementId> {
    let mut ids = BTreeSet::new();
    ids.extend(project.levels.iter().map(|level| level.id.clone()));
    ids.extend(project.materials.iter().map(|material| material.id.clone()));
    ids.extend(project.systems.iter().map(|system| system.id.clone()));
    ids.extend(project.walls.iter().map(|wall| wall.id.clone()));
    ids.extend(
        project
            .walls
            .iter()
            .flat_map(|wall| wall.openings.iter().map(|opening| opening.id.clone())),
    );
    ids.extend(
        project
            .walls
            .iter()
            .flat_map(|wall| wall.dimensions.iter().map(|dimension| dimension.id.clone())),
    );
    ids.extend(project.wall_joins.iter().map(|join| join.id.clone()));
    ids.extend(project.rooms.iter().map(|room| room.id.clone()));
    ids
}

fn mint_project_id(
    library_prefix: &str,
    source_id: &ElementId,
    used: &mut BTreeSet<ElementId>,
) -> ElementId {
    let base = ElementId::new(format!("{library_prefix}-{}", source_id.0));
    if used.insert(base.clone()) {
        return base;
    }

    for suffix in 2.. {
        let candidate = ElementId::new(format!("{}-{suffix}", base.0));
        if used.insert(candidate.clone()) {
            return candidate;
        }
    }
    unreachable!("unbounded integer suffix must eventually produce a free id")
}

fn library_id_prefix(library: &Library) -> String {
    let without_scheme = library
        .coordinate
        .strip_prefix("framer-lib://")
        .unwrap_or(&library.coordinate);
    let slug = without_scheme
        .chars()
        .map(|value| {
            if value.is_ascii_alphanumeric() {
                value.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    let collapsed = slug
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if collapsed.is_empty() {
        "library".to_owned()
    } else {
        collapsed
    }
}

#[derive(Debug, Error)]
pub enum LibraryImportError {
    #[error(transparent)]
    Library(#[from] LibraryError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Model(#[from] ModelError),
    #[error("library content hash mismatch: expected {expected}, got {actual}")]
    ContentHashMismatch { expected: String, actual: String },
    #[error("material {id:?} was not found in the library")]
    MaterialNotFound { id: ElementId },
    #[error("system {id:?} was not found in the library")]
    SystemNotFound { id: ElementId },
    #[error("system {system:?} references missing material {material:?}")]
    MissingReferencedMaterial {
        system: ElementId,
        material: ElementId,
    },
    #[error("unknown built-in library {id:?}")]
    UnknownBuiltin { id: String },
    #[error("installed library {id:?} was not found on the search path")]
    InstalledLibraryNotFound { id: String },
    #[error("remote library resolution is deferred to a later phase")]
    RemoteUnsupported,
    #[error("failed to read library file {path:?}")]
    Io { path: PathBuf, source: io::Error },
}

#[cfg(test)]
mod tests {
    use framer_core::{
        BoardProfile, CodeProfile, ConstructionLayer, FramingPattern, FramingSpec, LayerFunction,
        Length, SystemKind, load_project, save_project,
    };

    use super::*;

    fn fixture_library() -> Library {
        Library {
            uid: "11111111-1111-4111-8111-111111111111".to_owned(),
            version_id: "019e9150-0000-7000-8000-000000000001".to_owned(),
            version: "1.0.0".to_owned(),
            coordinate: "framer-lib://acme/walls".to_owned(),
            materials: vec![
                Material::solid_color("mat-cedar", "Cedar", [170, 110, 70]),
                Material::solid_color("mat-mineral-wool", "Mineral wool", [176, 188, 170]),
            ],
            systems: vec![ConstructionSystem {
                id: ElementId::new("system-rainscreen"),
                name: "Rainscreen wall".to_owned(),
                kind: SystemKind::Wall,
                source: None,
                layers: vec![
                    ConstructionLayer::new(
                        LayerFunction::InteriorFinish,
                        "mat-cedar",
                        Length::from_whole_inches(1),
                    ),
                    ConstructionLayer::new(
                        LayerFunction::Framing,
                        "mat-cedar",
                        BoardProfile::TwoByFour.nominal_depth(),
                    )
                    .with_framing(FramingSpec {
                        member: BoardProfile::TwoByFour,
                        spacing: Length::from_whole_inches(16),
                        pattern: FramingPattern::Single,
                        cavity_material: Some(ElementId::new("mat-mineral-wool")),
                    }),
                ],
            }],
        }
    }

    #[test]
    fn starter_library_hash_is_golden() {
        let loaded = starter_library().unwrap();

        assert_eq!(
            loaded.content_hash,
            "blake3:f0399fdb44f4f2c696d4b8a69955d174e5fa8aa8366aae0410c4d6a63f265a5f"
        );
    }

    #[test]
    fn verifies_expected_library_hash() {
        let loaded = starter_library().unwrap();
        let bytes = LibraryBytes {
            source: STARTER_LIBRARY_SOURCE.to_owned(),
            expected_hash: Some(loaded.content_hash.clone()),
        };

        assert_eq!(
            load_verified_library(&bytes).unwrap().content_hash,
            loaded.content_hash
        );

        let bad = LibraryBytes {
            source: STARTER_LIBRARY_SOURCE.to_owned(),
            expected_hash: Some("blake3:0000".to_owned()),
        };
        assert!(matches!(
            load_verified_library(&bad),
            Err(LibraryImportError::ContentHashMismatch { .. })
        ));
    }

    #[test]
    fn importing_system_vendors_material_closure_and_remaps_references() {
        let library = fixture_library();
        let hash = library_content_hash(&library).unwrap();
        let mut project = BuildingModel::new(CodeProfile::irc_2021_prescriptive());

        let imported = import_system(
            &mut project,
            &library,
            &hash,
            &ElementId::new("system-rainscreen"),
        )
        .unwrap();

        assert_eq!(
            imported.materials,
            vec![
                ElementId::new("acme-walls-mat-cedar"),
                ElementId::new("acme-walls-mat-mineral-wool")
            ]
        );
        assert_eq!(
            imported.system,
            Some(ElementId::new("acme-walls-system-rainscreen"))
        );
        let system = project
            .systems
            .iter()
            .find(|system| system.id == ElementId::new("acme-walls-system-rainscreen"))
            .unwrap();
        assert!(system.source.is_some());
        assert_eq!(
            system.layers[0].material,
            ElementId::new("acme-walls-mat-cedar")
        );
        assert_eq!(
            system.layers[1].framing.as_ref().unwrap().cavity_material,
            Some(ElementId::new("acme-walls-mat-mineral-wool"))
        );
        assert_eq!(project.libraries.len(), 1);
        project.validate().unwrap();
    }

    #[test]
    fn repeated_imports_from_same_library_version_share_one_stamp() {
        let library = fixture_library();
        let hash = library_content_hash(&library).unwrap();
        let mut project = BuildingModel::new(CodeProfile::irc_2021_prescriptive());

        import_material(&mut project, &library, &hash, &ElementId::new("mat-cedar")).unwrap();
        import_system(
            &mut project,
            &library,
            &hash,
            &ElementId::new("system-rainscreen"),
        )
        .unwrap();

        assert_eq!(project.libraries.len(), 1);
        assert_eq!(project.libraries[0].uid, library.uid);
        assert_eq!(project.libraries[0].version_id, library.version_id);
        assert_eq!(project.libraries[0].content_hash, hash);
        assert_eq!(
            project
                .materials
                .iter()
                .filter(|material| matches!(material.source, MaterialSource::Library(_)))
                .count(),
            3
        );
        assert!(
            project
                .systems
                .iter()
                .any(|system| system.id == ElementId::new("acme-walls-system-rainscreen"))
        );
        project.validate().unwrap();
    }

    #[test]
    fn import_collision_uses_lowest_free_suffix() {
        let library = fixture_library();
        let hash = library_content_hash(&library).unwrap();
        let mut project = BuildingModel::new(CodeProfile::irc_2021_prescriptive());
        project.materials.push(Material::solid_color(
            "acme-walls-mat-cedar",
            "Existing",
            [0, 0, 0],
        ));

        let imported =
            import_material(&mut project, &library, &hash, &ElementId::new("mat-cedar")).unwrap();

        assert_eq!(
            imported.materials,
            vec![ElementId::new("acme-walls-mat-cedar-2")]
        );
        let material = project
            .material(&ElementId::new("acme-walls-mat-cedar-2"))
            .unwrap();
        assert!(matches!(material.source, MaterialSource::Library(_)));
    }

    #[test]
    fn project_remains_self_contained_without_library_resolution() {
        let project = BuildingModel::new(CodeProfile::irc_2021_prescriptive());
        let saved = save_project(&project).unwrap();
        let reloaded = load_project(&saved).unwrap();

        assert!(reloaded.libraries.is_empty());
        reloaded.validate().unwrap();
    }

    #[test]
    fn local_search_path_resolves_installed_libraries() {
        let resolver = LocalSearchPathResolver::new([
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../libraries")
        ]);
        let loaded = load_verified_library(
            &resolver
                .resolve(&Locator::Installed {
                    id: "framer-starter".to_owned(),
                })
                .unwrap(),
        )
        .unwrap();

        assert_eq!(loaded.library.coordinate, "framer-lib://framer/starter");
    }
}
