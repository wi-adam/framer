use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{self, Cursor, Read, Write},
    path::{Path, PathBuf},
    sync::OnceLock,
};

use framer_core::{
    Appearance, BuildingModel, ConstructionSystem, ElementId, Library, LibraryError, LibraryStamp,
    Material, MaterialSource, ModelError, ProjectError, Provenance, is_blake3_hash, load_library,
    load_project, save_library, save_project,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use zip::{
    CompressionMethod, DateTime, ZipArchive, ZipWriter, read::read_zipfile_from_stream,
    write::SimpleFileOptions,
};

const STARTER_LIBRARY_SOURCE: &str = include_str!("../../../libraries/framer-starter.framerlib");
pub const PACKAGE_FORMAT: &str = "framer.package";
pub const PACKAGE_SCHEMA_VERSION: u32 = 1;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibraryIssueKind {
    Diverged,
    OutOfDate,
    SourceMissing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryIssue {
    pub item: LibraryItem,
    pub source_id: ElementId,
    pub library_uid: String,
    pub version_id: String,
    pub kind: LibraryIssueKind,
    pub expected_hash: String,
    pub actual_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageManifest {
    pub format: String,
    pub schema_version: u32,
    pub project: String,
    pub assets: Vec<PackageAssetEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageAssetEntry {
    pub hash: String,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectPackage {
    pub project: BuildingModel,
    pub assets: BTreeMap<String, Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentAddressedAssetStore {
    root: PathBuf,
}

impl ContentAddressedAssetStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn put(&self, bytes: &[u8]) -> Result<String, LibraryImportError> {
        let hash = asset_content_hash(bytes);
        let path = self.path_for_hash(&hash)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| LibraryImportError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        fs::write(&path, bytes).map_err(|source| LibraryImportError::Io {
            path: path.clone(),
            source,
        })?;
        Ok(hash)
    }

    pub fn get(&self, hash: &str) -> Result<Option<Vec<u8>>, LibraryImportError> {
        let path = self.path_for_hash(hash)?;
        match fs::read(&path) {
            Ok(bytes) => {
                verify_asset_hash(hash, &bytes)?;
                Ok(Some(bytes))
            }
            Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(source) => Err(LibraryImportError::Io { path, source }),
        }
    }

    pub fn contains(&self, hash: &str) -> Result<bool, LibraryImportError> {
        Ok(self.path_for_hash(hash)?.is_file())
    }

    fn path_for_hash(&self, hash: &str) -> Result<PathBuf, LibraryImportError> {
        Ok(self.root.join(asset_file_name(hash)?))
    }
}

impl LibraryIssue {
    pub fn item_id(&self) -> &ElementId {
        match &self.item {
            LibraryItem::Material(id) | LibraryItem::System(id) => id,
        }
    }
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

pub fn detach_item(
    project: &mut BuildingModel,
    item: LibraryItem,
) -> Result<bool, LibraryImportError> {
    match item {
        LibraryItem::Material(id) => detach_material(project, &id),
        LibraryItem::System(id) => detach_system(project, &id),
    }
}

pub fn resync_item(
    project: &mut BuildingModel,
    library: &Library,
    library_content_hash: &str,
    item: LibraryItem,
) -> Result<ImportResult, LibraryImportError> {
    match item {
        LibraryItem::Material(id) => resync_material(project, library, library_content_hash, &id),
        LibraryItem::System(id) => resync_system(project, library, library_content_hash, &id),
    }
}

pub fn asset_content_hash(bytes: &[u8]) -> String {
    hash_bytes(bytes)
}

pub fn referenced_asset_hashes(project: &BuildingModel) -> BTreeSet<String> {
    let mut hashes = BTreeSet::new();
    for material in &project.materials {
        match &material.appearance {
            Appearance::SolidColor(_) => {}
            Appearance::Textured { texture, .. } => {
                hashes.insert(texture.hash.clone());
            }
            Appearance::DepthMapped { height, .. } => {
                hashes.insert(height.hash.clone());
            }
        }
    }
    hashes
}

pub fn collect_project_assets(
    project: &BuildingModel,
    store: &ContentAddressedAssetStore,
) -> Result<BTreeMap<String, Vec<u8>>, LibraryImportError> {
    let mut assets = BTreeMap::new();
    for hash in referenced_asset_hashes(project) {
        let Some(bytes) = store.get(&hash)? else {
            continue;
        };
        assets.insert(hash, bytes);
    }
    Ok(assets)
}

pub fn save_project_package(
    project: &BuildingModel,
    assets: &BTreeMap<String, Vec<u8>>,
) -> Result<Vec<u8>, LibraryImportError> {
    let project_json = save_project(project)?;
    let mut entries = BTreeMap::new();
    entries.insert("project.framer".to_owned(), project_json.into_bytes());

    let manifest = package_manifest(assets)?;
    let manifest_json = serde_json::to_string_pretty(&manifest)? + "\n";
    entries.insert("manifest.json".to_owned(), manifest_json.into_bytes());

    for (hash, bytes) in assets {
        verify_asset_hash(hash, bytes)?;
        entries.insert(asset_package_path(hash)?, bytes.clone());
    }

    write_stored_zip(&entries)
}

pub fn save_project_package_from_store(
    project: &BuildingModel,
    store: &ContentAddressedAssetStore,
) -> Result<Vec<u8>, LibraryImportError> {
    let assets = collect_project_assets(project, store)?;
    save_project_package(project, &assets)
}

pub fn load_project_package(bytes: &[u8]) -> Result<ProjectPackage, LibraryImportError> {
    let entries = read_stored_zip(bytes)?;
    let project_bytes = entries
        .get("project.framer")
        .ok_or(LibraryImportError::PackageProjectMissing)?;
    let project_source = std::str::from_utf8(project_bytes)
        .map_err(|source| LibraryImportError::PackageUtf8 { source })?;
    let project = load_project(project_source)?;

    let manifest_bytes = entries
        .get("manifest.json")
        .ok_or(LibraryImportError::PackageManifestMissing)?;
    let manifest_source = std::str::from_utf8(manifest_bytes)
        .map_err(|source| LibraryImportError::PackageUtf8 { source })?;
    let manifest: PackageManifest = serde_json::from_str(manifest_source)?;
    validate_manifest(&manifest)?;

    let mut assets = BTreeMap::new();
    for asset in &manifest.assets {
        let bytes =
            entries
                .get(&asset.path)
                .ok_or_else(|| LibraryImportError::PackageAssetMissing {
                    path: asset.path.clone(),
                })?;
        verify_asset_hash(&asset.hash, bytes)?;
        assets.insert(asset.hash.clone(), bytes.clone());
    }

    Ok(ProjectPackage { project, assets })
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

pub fn resync_material(
    project: &mut BuildingModel,
    library: &Library,
    library_content_hash: &str,
    project_id: &ElementId,
) -> Result<ImportResult, LibraryImportError> {
    let mut staged = project.clone();
    let material_index = staged
        .materials
        .iter()
        .position(|material| material.id == *project_id)
        .ok_or_else(|| LibraryImportError::ProjectMaterialNotFound {
            id: project_id.clone(),
        })?;
    let source = match &staged.materials[material_index].source {
        MaterialSource::Library(source) => source.clone(),
        MaterialSource::Project => {
            return Err(LibraryImportError::ItemHasNoLibrarySource {
                id: project_id.clone(),
            });
        }
    };
    ensure_library_matches(&source, library)?;

    let source_material = library_material(library, &source.source_id)?.clone();
    let local_id = staged.materials[material_index].id.clone();
    staged.materials[material_index] =
        vendor_material(library, &source_material, local_id.clone())?;

    ensure_library_stamp(&mut staged, library, library_content_hash);
    prune_unused_library_stamps(&mut staged);
    staged.sort_deterministically();
    staged.validate()?;
    *project = staged;

    Ok(ImportResult {
        materials: vec![local_id],
        system: None,
    })
}

pub fn detach_material(
    project: &mut BuildingModel,
    project_id: &ElementId,
) -> Result<bool, LibraryImportError> {
    let mut staged = project.clone();
    let material_index = staged
        .materials
        .iter()
        .position(|material| material.id == *project_id)
        .ok_or_else(|| LibraryImportError::ProjectMaterialNotFound {
            id: project_id.clone(),
        })?;

    if matches!(
        staged.materials[material_index].source,
        MaterialSource::Project
    ) {
        return Ok(false);
    }

    staged.materials[material_index].source = MaterialSource::Project;
    prune_unused_library_stamps(&mut staged);
    staged.sort_deterministically();
    staged.validate()?;
    *project = staged;
    Ok(true)
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

pub fn resync_system(
    project: &mut BuildingModel,
    library: &Library,
    library_content_hash: &str,
    project_id: &ElementId,
) -> Result<ImportResult, LibraryImportError> {
    let source_system = library_system_for_project(project, library, project_id)?.clone();
    let source = project_system_source(project, project_id)?.clone();
    ensure_library_matches(&source, library)?;

    let material_lookup = library
        .materials
        .iter()
        .map(|material| (material.id.clone(), material))
        .collect::<BTreeMap<_, _>>();
    let referenced_materials = referenced_system_materials(&source_system);
    let prefix = library_id_prefix(library);

    let mut staged = project.clone();
    let system_index = staged
        .systems
        .iter()
        .position(|system| system.id == *project_id)
        .ok_or_else(|| LibraryImportError::ProjectSystemNotFound {
            id: project_id.clone(),
        })?;
    let current_system = staged.systems[system_index].clone();
    let mut used = collect_project_ids(&staged);
    let mut material_remap = preferred_material_remap(&staged, &current_system, &source);

    let mut imported_materials = Vec::new();
    for source_material_id in referenced_materials {
        let source_material = material_lookup.get(&source_material_id).ok_or_else(|| {
            LibraryImportError::MissingReferencedMaterial {
                system: source_system.id.clone(),
                material: source_material_id.clone(),
            }
        })?;
        let local_id = material_remap
            .entry(source_material.id.clone())
            .or_insert_with(|| mint_project_id(&prefix, &source_material.id, &mut used))
            .clone();
        let material = vendor_material(library, source_material, local_id.clone())?;
        if let Some(material_index) = staged
            .materials
            .iter()
            .position(|candidate| candidate.id == local_id)
        {
            staged.materials[material_index] = material;
        } else {
            staged.materials.push(material);
        }
        imported_materials.push(local_id);
    }

    staged.systems[system_index] = vendor_system(
        library,
        &source_system,
        current_system.id.clone(),
        &material_remap,
    )?;
    ensure_library_stamp(&mut staged, library, library_content_hash);
    prune_unused_library_stamps(&mut staged);
    staged.sort_deterministically();
    staged.validate()?;
    *project = staged;

    Ok(ImportResult {
        materials: imported_materials,
        system: Some(project_id.clone()),
    })
}

pub fn detach_system(
    project: &mut BuildingModel,
    project_id: &ElementId,
) -> Result<bool, LibraryImportError> {
    let mut staged = project.clone();
    let system_index = staged
        .systems
        .iter()
        .position(|system| system.id == *project_id)
        .ok_or_else(|| LibraryImportError::ProjectSystemNotFound {
            id: project_id.clone(),
        })?;

    if staged.systems[system_index].source.is_none() {
        return Ok(false);
    }

    staged.systems[system_index].source = None;
    prune_unused_library_stamps(&mut staged);
    staged.sort_deterministically();
    staged.validate()?;
    *project = staged;
    Ok(true)
}

pub fn library_lifecycle_issues(
    project: &BuildingModel,
    current_libraries: &[Library],
) -> Result<Vec<LibraryIssue>, LibraryImportError> {
    let mut issues = Vec::new();
    let current_by_uid = current_libraries
        .iter()
        .map(|library| (library.uid.as_str(), library))
        .collect::<BTreeMap<_, _>>();

    for material in &project.materials {
        let MaterialSource::Library(source) = &material.source else {
            continue;
        };
        let current_hash = vendored_material_content_hash(material, source)?;
        if current_hash != source.content_hash {
            issues.push(library_issue(
                LibraryIssueKind::Diverged,
                LibraryItem::Material(material.id.clone()),
                source,
                Some(current_hash),
            ));
        }
        if let Some(library) = current_by_uid.get(source.library_uid.as_str()) {
            match find_library_material(library, &source.source_id) {
                Some(source_material) => {
                    let library_hash = material_content_hash(source_material)?;
                    if library_hash != source.content_hash {
                        issues.push(library_issue(
                            LibraryIssueKind::OutOfDate,
                            LibraryItem::Material(material.id.clone()),
                            source,
                            Some(library_hash),
                        ));
                    }
                }
                None => issues.push(library_issue(
                    LibraryIssueKind::SourceMissing,
                    LibraryItem::Material(material.id.clone()),
                    source,
                    None,
                )),
            }
        }
    }

    for system in &project.systems {
        let Some(source) = &system.source else {
            continue;
        };
        let current_hash = vendored_system_content_hash(project, system, source)?;
        if current_hash != source.content_hash {
            issues.push(library_issue(
                LibraryIssueKind::Diverged,
                LibraryItem::System(system.id.clone()),
                source,
                Some(current_hash),
            ));
        }
        if let Some(library) = current_by_uid.get(source.library_uid.as_str()) {
            match find_library_system(library, &source.source_id) {
                Some(source_system) => {
                    let library_hash = system_content_hash(source_system)?;
                    if library_hash != source.content_hash {
                        issues.push(library_issue(
                            LibraryIssueKind::OutOfDate,
                            LibraryItem::System(system.id.clone()),
                            source,
                            Some(library_hash),
                        ));
                    }
                }
                None => issues.push(library_issue(
                    LibraryIssueKind::SourceMissing,
                    LibraryItem::System(system.id.clone()),
                    source,
                    None,
                )),
            }
        }
    }

    Ok(issues)
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

pub fn starter_library_ref() -> Result<&'static LoadedLibrary, LibraryImportError> {
    if let Some(library) = STARTER_LIBRARY.get() {
        return Ok(library);
    }

    let loaded = load_verified_library(&LibraryBytes {
        source: STARTER_LIBRARY_SOURCE.to_owned(),
        expected_hash: None,
    })?;
    let _ = STARTER_LIBRARY.set(loaded);
    Ok(STARTER_LIBRARY
        .get()
        .expect("starter library should be initialized"))
}

pub fn starter_library() -> Result<LoadedLibrary, LibraryImportError> {
    starter_library_ref().cloned()
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

fn library_issue(
    kind: LibraryIssueKind,
    item: LibraryItem,
    source: &Provenance,
    actual_hash: Option<String>,
) -> LibraryIssue {
    LibraryIssue {
        item,
        source_id: source.source_id.clone(),
        library_uid: source.library_uid.clone(),
        version_id: source.version_id.clone(),
        kind,
        expected_hash: source.content_hash.clone(),
        actual_hash,
    }
}

fn vendored_material_content_hash(
    material: &Material,
    source: &Provenance,
) -> Result<String, LibraryImportError> {
    let mut material = material.clone();
    material.id = source.source_id.clone();
    material.source = MaterialSource::Project;
    hash_json(&material)
}

fn vendored_system_content_hash(
    project: &BuildingModel,
    system: &ConstructionSystem,
    source: &Provenance,
) -> Result<String, LibraryImportError> {
    let mut system = system.clone();
    let material_sources = project
        .materials
        .iter()
        .filter_map(|material| match &material.source {
            MaterialSource::Library(material_source)
                if material_source.library_uid == source.library_uid =>
            {
                Some((material.id.clone(), material_source.source_id.clone()))
            }
            _ => None,
        })
        .collect::<BTreeMap<_, _>>();

    system.id = source.source_id.clone();
    system.source = None;
    for layer in &mut system.layers {
        if let Some(source_id) = material_sources.get(&layer.material) {
            layer.material = source_id.clone();
        }
        if let Some(framing) = &mut layer.framing
            && let Some(cavity) = &mut framing.cavity_material
            && let Some(source_id) = material_sources.get(cavity)
        {
            *cavity = source_id.clone();
        }
    }
    hash_json(&system)
}

fn provenance(library: &Library, source_id: &ElementId, content_hash: String) -> Provenance {
    Provenance {
        library_uid: library.uid.clone(),
        version_id: library.version_id.clone(),
        source_id: source_id.clone(),
        content_hash,
    }
}

fn ensure_library_matches(
    source: &Provenance,
    library: &Library,
) -> Result<(), LibraryImportError> {
    if source.library_uid == library.uid {
        Ok(())
    } else {
        Err(LibraryImportError::LibraryUidMismatch {
            expected: source.library_uid.clone(),
            actual: library.uid.clone(),
        })
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

fn prune_unused_library_stamps(project: &mut BuildingModel) {
    let used = project
        .materials
        .iter()
        .filter_map(|material| match &material.source {
            MaterialSource::Library(source) => {
                Some((source.library_uid.clone(), source.version_id.clone()))
            }
            MaterialSource::Project => None,
        })
        .chain(project.systems.iter().filter_map(|system| {
            system
                .source
                .as_ref()
                .map(|source| (source.library_uid.clone(), source.version_id.clone()))
        }))
        .collect::<BTreeSet<_>>();
    project
        .libraries
        .retain(|stamp| used.contains(&(stamp.uid.clone(), stamp.version_id.clone())));
}

fn find_library_material<'a>(library: &'a Library, source_id: &ElementId) -> Option<&'a Material> {
    library
        .materials
        .iter()
        .find(|material| material.id == *source_id)
}

fn find_library_system<'a>(
    library: &'a Library,
    source_id: &ElementId,
) -> Option<&'a ConstructionSystem> {
    library
        .systems
        .iter()
        .find(|system| system.id == *source_id)
}

fn library_material<'a>(
    library: &'a Library,
    source_id: &ElementId,
) -> Result<&'a Material, LibraryImportError> {
    find_library_material(library, source_id).ok_or_else(|| LibraryImportError::MaterialNotFound {
        id: source_id.clone(),
    })
}

fn library_system<'a>(
    library: &'a Library,
    source_id: &ElementId,
) -> Result<&'a ConstructionSystem, LibraryImportError> {
    find_library_system(library, source_id).ok_or_else(|| LibraryImportError::SystemNotFound {
        id: source_id.clone(),
    })
}

fn project_system_source<'a>(
    project: &'a BuildingModel,
    project_id: &ElementId,
) -> Result<&'a Provenance, LibraryImportError> {
    let system = project
        .systems
        .iter()
        .find(|system| system.id == *project_id)
        .ok_or_else(|| LibraryImportError::ProjectSystemNotFound {
            id: project_id.clone(),
        })?;
    system
        .source
        .as_ref()
        .ok_or_else(|| LibraryImportError::ItemHasNoLibrarySource {
            id: project_id.clone(),
        })
}

fn library_system_for_project<'a>(
    project: &BuildingModel,
    library: &'a Library,
    project_id: &ElementId,
) -> Result<&'a ConstructionSystem, LibraryImportError> {
    let source = project_system_source(project, project_id)?;
    ensure_library_matches(source, library)?;
    library_system(library, &source.source_id)
}

fn referenced_system_materials(system: &ConstructionSystem) -> BTreeSet<ElementId> {
    let mut referenced = BTreeSet::new();
    for layer in &system.layers {
        referenced.insert(layer.material.clone());
        if let Some(framing) = &layer.framing
            && let Some(cavity) = &framing.cavity_material
        {
            referenced.insert(cavity.clone());
        }
    }
    referenced
}

fn preferred_material_remap(
    project: &BuildingModel,
    current_system: &ConstructionSystem,
    system_source: &Provenance,
) -> BTreeMap<ElementId, ElementId> {
    let materials_by_id = project
        .materials
        .iter()
        .map(|material| (material.id.clone(), material))
        .collect::<BTreeMap<_, _>>();
    let mut remap = BTreeMap::new();

    for local_id in referenced_system_materials(current_system) {
        if let Some(material) = materials_by_id.get(&local_id)
            && let MaterialSource::Library(material_source) = &material.source
            && material_source.library_uid == system_source.library_uid
        {
            remap
                .entry(material_source.source_id.clone())
                .or_insert(local_id);
        }
    }

    for material in &project.materials {
        if let MaterialSource::Library(material_source) = &material.source
            && material_source.library_uid == system_source.library_uid
        {
            remap
                .entry(material_source.source_id.clone())
                .or_insert_with(|| material.id.clone());
        }
    }

    remap
}

fn hash_json<T: serde::Serialize>(value: &T) -> Result<String, LibraryImportError> {
    let mut json = serde_json::to_string_pretty(value)?;
    json.push('\n');
    Ok(hash_bytes(json.as_bytes()))
}

fn hash_bytes(bytes: &[u8]) -> String {
    format!("blake3:{}", blake3::hash(bytes).to_hex())
}

fn package_manifest(
    assets: &BTreeMap<String, Vec<u8>>,
) -> Result<PackageManifest, LibraryImportError> {
    let mut entries = Vec::with_capacity(assets.len());
    for (hash, bytes) in assets {
        verify_asset_hash(hash, bytes)?;
        entries.push(PackageAssetEntry {
            hash: hash.clone(),
            path: asset_package_path(hash)?,
        });
    }
    Ok(PackageManifest {
        format: PACKAGE_FORMAT.to_owned(),
        schema_version: PACKAGE_SCHEMA_VERSION,
        project: "project.framer".to_owned(),
        assets: entries,
    })
}

fn validate_manifest(manifest: &PackageManifest) -> Result<(), LibraryImportError> {
    if manifest.format != PACKAGE_FORMAT {
        return Err(LibraryImportError::InvalidPackageFormat {
            found: manifest.format.clone(),
        });
    }
    if manifest.schema_version != PACKAGE_SCHEMA_VERSION {
        return Err(LibraryImportError::UnsupportedPackageSchemaVersion {
            found: manifest.schema_version,
            supported: PACKAGE_SCHEMA_VERSION,
        });
    }
    if manifest.project != "project.framer" {
        return Err(LibraryImportError::InvalidPackageProjectPath {
            path: manifest.project.clone(),
        });
    }
    for asset in &manifest.assets {
        validate_asset_hash(&asset.hash)?;
        let expected_path = asset_package_path(&asset.hash)?;
        if asset.path != expected_path {
            return Err(LibraryImportError::InvalidPackageAssetPath {
                hash: asset.hash.clone(),
                path: asset.path.clone(),
            });
        }
    }
    Ok(())
}

fn verify_asset_hash(expected: &str, bytes: &[u8]) -> Result<(), LibraryImportError> {
    validate_asset_hash(expected)?;
    let actual = asset_content_hash(bytes);
    if actual == expected {
        Ok(())
    } else {
        Err(LibraryImportError::AssetHashMismatch {
            expected: expected.to_owned(),
            actual,
        })
    }
}

fn validate_asset_hash(hash: &str) -> Result<(), LibraryImportError> {
    if is_blake3_hash(hash) {
        Ok(())
    } else {
        Err(LibraryImportError::InvalidAssetHash {
            hash: hash.to_owned(),
        })
    }
}

fn asset_file_name(hash: &str) -> Result<String, LibraryImportError> {
    validate_asset_hash(hash)?;
    let hex = hash
        .strip_prefix("blake3:")
        .expect("validated asset hash must carry the blake3 prefix");
    Ok(format!("blake3-{hex}"))
}

fn asset_package_path(hash: &str) -> Result<String, LibraryImportError> {
    Ok(format!("assets/{}", asset_file_name(hash)?))
}

fn write_stored_zip(entries: &BTreeMap<String, Vec<u8>>) -> Result<Vec<u8>, LibraryImportError> {
    u16::try_from(entries.len()).map_err(|_| LibraryImportError::PackageTooLarge)?;
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .last_modified_time(DateTime::default())
        .unix_permissions(0o644)
        .large_file(false);
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));

    for (name, bytes) in entries {
        validate_package_zip_entry_name(name)?;
        u32::try_from(bytes.len()).map_err(|_| LibraryImportError::PackageTooLarge)?;
        zip.start_file(name, options).map_err(package_zip_error)?;
        zip.write_all(bytes).map_err(package_zip_io_error)?;
    }

    let out = zip.finish().map_err(package_zip_error)?;
    Ok(out.into_inner())
}

fn read_stored_zip(bytes: &[u8]) -> Result<BTreeMap<String, Vec<u8>>, LibraryImportError> {
    let streamed_names = read_streamed_zip_entry_names(bytes)?;

    let cursor = Cursor::new(bytes);
    let mut zip = ZipArchive::new(cursor).map_err(package_zip_error)?;
    u16::try_from(zip.len()).map_err(|_| LibraryImportError::PackageTooLarge)?;
    if zip.len() != streamed_names.len() {
        return Err(LibraryImportError::InvalidPackageZip(
            "package central directory entries do not match local file entries".to_owned(),
        ));
    }
    let mut entries = BTreeMap::new();
    for index in 0..zip.len() {
        let mut file = zip.by_index(index).map_err(package_zip_error)?;
        let name = file.name().to_owned();
        validate_package_zip_entry_name(&name)?;
        if !streamed_names.contains(&name) {
            return Err(LibraryImportError::InvalidPackageZip(format!(
                "package central directory entry {name:?} has no matching local file entry"
            )));
        }
        if !file.is_file() {
            return Err(LibraryImportError::InvalidPackageZip(format!(
                "package entry {name:?} is not a regular file"
            )));
        }
        if file.compression() != CompressionMethod::Stored {
            return Err(LibraryImportError::InvalidPackageZip(format!(
                "package entry {name:?} is not stored"
            )));
        }
        if file.compressed_size() != file.size() {
            return Err(LibraryImportError::InvalidPackageZip(format!(
                "stored package entry {name:?} has mismatched sizes"
            )));
        }
        let size = usize::try_from(file.size()).map_err(|_| LibraryImportError::PackageTooLarge)?;
        let mut data = Vec::with_capacity(size);
        file.read_to_end(&mut data).map_err(package_zip_io_error)?;
        if data.len() != size {
            return Err(LibraryImportError::InvalidPackageZip(format!(
                "package entry {name:?} length mismatch"
            )));
        }
        if entries.insert(name.clone(), data).is_some() {
            return Err(LibraryImportError::InvalidPackageZip(format!(
                "duplicate package entry {name:?}"
            )));
        }
    }

    Ok(entries)
}

fn read_streamed_zip_entry_names(bytes: &[u8]) -> Result<BTreeSet<String>, LibraryImportError> {
    let mut cursor = Cursor::new(bytes);
    let mut names = BTreeSet::new();
    while let Some(file) = read_zipfile_from_stream(&mut cursor).map_err(package_zip_error)? {
        let name = file.name().to_owned();
        validate_package_zip_entry_name(&name)?;
        if !names.insert(name.clone()) {
            return Err(LibraryImportError::InvalidPackageZip(format!(
                "duplicate package entry {name:?}"
            )));
        }
    }
    Ok(names)
}

fn validate_package_zip_entry_name(name: &str) -> Result<(), LibraryImportError> {
    let invalid = name.is_empty()
        || name.starts_with('/')
        || name.contains('\\')
        || name
            .split('/')
            .any(|component| component.is_empty() || component == "." || component == "..");
    if invalid {
        return Err(LibraryImportError::InvalidPackageZip(format!(
            "invalid package entry name {name:?}"
        )));
    }
    Ok(())
}

fn package_zip_error(error: zip::result::ZipError) -> LibraryImportError {
    LibraryImportError::InvalidPackageZip(error.to_string())
}

fn package_zip_io_error(error: io::Error) -> LibraryImportError {
    LibraryImportError::InvalidPackageZip(error.to_string())
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
    Project(#[from] ProjectError),
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
    #[error("project material {id:?} was not found")]
    ProjectMaterialNotFound { id: ElementId },
    #[error("project system {id:?} was not found")]
    ProjectSystemNotFound { id: ElementId },
    #[error("project item {id:?} is not library-backed")]
    ItemHasNoLibrarySource { id: ElementId },
    #[error("library uid mismatch: item references {expected:?}, got {actual:?}")]
    LibraryUidMismatch { expected: String, actual: String },
    #[error("asset hash {hash:?} must be a full lowercase blake3:<hex> content hash")]
    InvalidAssetHash { hash: String },
    #[error("asset content hash mismatch: expected {expected}, got {actual}")]
    AssetHashMismatch { expected: String, actual: String },
    #[error("invalid Framer package format {found:?}")]
    InvalidPackageFormat { found: String },
    #[error("unsupported Framer package schema version {found}; this build supports {supported}")]
    UnsupportedPackageSchemaVersion { found: u32, supported: u32 },
    #[error("package project path must be project.framer, got {path:?}")]
    InvalidPackageProjectPath { path: String },
    #[error("asset {hash:?} has invalid package path {path:?}")]
    InvalidPackageAssetPath { hash: String, path: String },
    #[error("package is missing project.framer")]
    PackageProjectMissing,
    #[error("package is missing manifest.json")]
    PackageManifestMissing,
    #[error("package is missing asset entry {path:?}")]
    PackageAssetMissing { path: String },
    #[error("package text is not valid UTF-8")]
    PackageUtf8 { source: std::str::Utf8Error },
    #[error("invalid Framer package zip: {0}")]
    InvalidPackageZip(String),
    #[error("Framer package is too large for the v1 zip writer")]
    PackageTooLarge,
}

#[cfg(test)]
mod tests {
    use framer_core::{
        Appearance, AssetRef, BoardProfile, CodeProfile, ConstructionLayer, FramingPattern,
        FramingSpec, LayerFunction, Length, ModelError, SystemKind, TextureRole, load_project,
        save_project,
    };

    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

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

    fn temp_path(name: &str) -> PathBuf {
        let id = TEMP_COUNTER.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!("framer-library-{name}-{}-{id}", std::process::id()))
    }

    fn textured_project() -> (BuildingModel, BTreeMap<String, Vec<u8>>) {
        let bytes = b"not-a-real-png-but-content-addressed".to_vec();
        let hash = asset_content_hash(&bytes);
        let mut model = BuildingModel::new(CodeProfile::irc_2021_prescriptive());
        let mut material = Material::solid_color("mat-textured", "Textured", [48, 96, 144]);
        material.appearance = Appearance::Textured {
            color: [48, 96, 144],
            texture: AssetRef::new(hash.clone(), "image/png", TextureRole::Texture),
            scale: Length::from_whole_inches(24),
        };
        model.materials.push(material);

        let mut assets = BTreeMap::new();
        assets.insert(hash, bytes);
        (model, assets)
    }

    fn package_entries(
        model: &BuildingModel,
        assets: &BTreeMap<String, Vec<u8>>,
    ) -> BTreeMap<String, Vec<u8>> {
        let mut entries = BTreeMap::new();
        entries.insert(
            "project.framer".to_owned(),
            save_project(model).unwrap().into_bytes(),
        );
        entries.insert(
            "manifest.json".to_owned(),
            manifest_bytes(&package_manifest(assets).unwrap()),
        );
        for (hash, bytes) in assets {
            entries.insert(asset_package_path(hash).unwrap(), bytes.clone());
        }
        entries
    }

    fn manifest_bytes(manifest: &PackageManifest) -> Vec<u8> {
        (serde_json::to_string_pretty(manifest).unwrap() + "\n").into_bytes()
    }

    fn test_zip(entries: &BTreeMap<String, Vec<u8>>) -> Vec<u8> {
        let entries = entries
            .iter()
            .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
            .collect::<Vec<_>>();
        test_zip_entries(&entries)
    }

    fn test_zip_entries(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Stored)
            .last_modified_time(DateTime::default());
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        for (name, bytes) in entries {
            zip.start_file(name, options).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    fn set_zip_u16_after_signature(
        bytes: &mut [u8],
        signature: &[u8; 4],
        offset: usize,
        value: u16,
    ) {
        for index in 0..bytes.len().saturating_sub(signature.len() + offset + 1) {
            if bytes[index..].starts_with(signature) {
                bytes[index + offset..index + offset + 2].copy_from_slice(&value.to_le_bytes());
            }
        }
    }

    fn replace_central_zip_name(bytes: &mut [u8], from: &[u8], to: &[u8]) {
        assert_eq!(from.len(), to.len());
        let name_offset = 46;
        for index in 0..bytes.len().saturating_sub(name_offset + from.len()) {
            if bytes[index..].starts_with(b"PK\x01\x02") {
                let name_start = index + name_offset;
                let name_end = name_start + from.len();
                if &bytes[name_start..name_end] == from {
                    bytes[name_start..name_end].copy_from_slice(to);
                }
            }
        }
    }

    fn set_zip_u32_after_signature(
        bytes: &mut [u8],
        signature: &[u8; 4],
        offset: usize,
        value: u32,
    ) {
        for index in 0..bytes.len().saturating_sub(signature.len() + offset + 3) {
            if bytes[index..].starts_with(signature) {
                bytes[index + offset..index + offset + 4].copy_from_slice(&value.to_le_bytes());
            }
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
    fn content_addressed_asset_store_writes_by_blake3_hash() {
        let root = temp_path("cas");
        let store = ContentAddressedAssetStore::new(&root);
        let bytes = b"asset-bytes";

        let hash = store.put(bytes).unwrap();

        assert_eq!(hash, asset_content_hash(bytes));
        assert!(store.contains(&hash).unwrap());
        assert_eq!(store.get(&hash).unwrap(), Some(bytes.to_vec()));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn project_package_is_deterministic_and_round_trips_assets() {
        let (model, assets) = textured_project();

        let first = save_project_package(&model, &assets).unwrap();
        let second = save_project_package(&model, &assets).unwrap();
        assert_eq!(first, second);

        let loaded = load_project_package(&first).unwrap();
        assert_eq!(
            save_project(&loaded.project).unwrap(),
            save_project(&model).unwrap()
        );
        assert_eq!(loaded.assets, assets);
    }

    #[test]
    fn project_package_rejects_unsafe_zip_entry_names() {
        let mut entries = BTreeMap::new();
        entries.insert("../project.framer".to_owned(), b"{}".to_vec());
        let package = test_zip(&entries);

        assert!(matches!(
            load_project_package(&package),
            Err(LibraryImportError::InvalidPackageZip(_))
        ));
    }

    #[test]
    fn project_package_rejects_invalid_zip_bytes() {
        assert!(matches!(
            load_project_package(b"not a zip"),
            Err(LibraryImportError::InvalidPackageZip(_))
        ));
    }

    #[test]
    fn project_package_rejects_non_stored_zip_entries() {
        let mut package = test_zip_entries(&[("project.framer", b"{}")]);
        set_zip_u16_after_signature(&mut package, b"PK\x03\x04", 8, 8);
        set_zip_u16_after_signature(&mut package, b"PK\x01\x02", 10, 8);

        assert!(matches!(
            load_project_package(&package),
            Err(LibraryImportError::InvalidPackageZip(_))
        ));
    }

    #[test]
    fn project_package_rejects_duplicate_zip_entry_names() {
        let mut package = test_zip_entries(&[
            ("project.framer", br#"{"first":true}"#),
            ("projecu.framer", br#"{"second":true}"#),
        ]);
        replace_central_zip_name(&mut package, b"projecu.framer", b"project.framer");

        assert!(matches!(
            load_project_package(&package),
            Err(LibraryImportError::InvalidPackageZip(_))
        ));
    }

    #[test]
    fn project_package_rejects_zip_symlink_entries() {
        let mut package = test_zip_entries(&[("project.framer", b"target")]);
        set_zip_u32_after_signature(&mut package, b"PK\x01\x02", 38, 0o120777 << 16);

        assert!(matches!(
            load_project_package(&package),
            Err(LibraryImportError::InvalidPackageZip(_))
        ));
    }

    #[test]
    fn project_package_rejects_stored_size_mismatch() {
        let mut package = test_zip_entries(&[("project.framer", b"{}")]);
        set_zip_u32_after_signature(&mut package, b"PK\x01\x02", 20, 1);

        assert!(matches!(
            load_project_package(&package),
            Err(LibraryImportError::InvalidPackageZip(_))
        ));
    }

    #[test]
    fn project_package_rejects_missing_project_entry() {
        let (model, assets) = textured_project();

        let mut missing_project = package_entries(&model, &assets);
        missing_project.remove("project.framer");
        assert!(matches!(
            load_project_package(&test_zip(&missing_project)),
            Err(LibraryImportError::PackageProjectMissing)
        ));
    }

    #[test]
    fn project_package_rejects_missing_manifest_entry() {
        let (model, assets) = textured_project();

        let mut missing_manifest = package_entries(&model, &assets);
        missing_manifest.remove("manifest.json");
        assert!(matches!(
            load_project_package(&test_zip(&missing_manifest)),
            Err(LibraryImportError::PackageManifestMissing)
        ));
    }

    #[test]
    fn project_package_rejects_missing_asset_entry() {
        let (model, assets) = textured_project();

        let mut missing_asset = package_entries(&model, &assets);
        let asset_path = missing_asset
            .keys()
            .find(|path| path.starts_with("assets/"))
            .unwrap()
            .clone();
        missing_asset.remove(&asset_path);
        assert!(matches!(
            load_project_package(&test_zip(&missing_asset)),
            Err(LibraryImportError::PackageAssetMissing { path }) if path == asset_path
        ));
    }

    #[test]
    fn project_package_rejects_invalid_manifest_format() {
        let (model, assets) = textured_project();
        let mut entries = package_entries(&model, &assets);
        let mut manifest = package_manifest(&assets).unwrap();

        manifest.format = "wrong.package".to_owned();
        entries.insert("manifest.json".to_owned(), manifest_bytes(&manifest));
        assert!(matches!(
            load_project_package(&test_zip(&entries)),
            Err(LibraryImportError::InvalidPackageFormat { found }) if found == "wrong.package"
        ));
    }

    #[test]
    fn project_package_rejects_unsupported_manifest_schema_version() {
        let (model, assets) = textured_project();
        let mut entries = package_entries(&model, &assets);

        let mut manifest = package_manifest(&assets).unwrap();
        manifest.schema_version = PACKAGE_SCHEMA_VERSION + 1;
        entries.insert("manifest.json".to_owned(), manifest_bytes(&manifest));
        assert!(matches!(
            load_project_package(&test_zip(&entries)),
            Err(LibraryImportError::UnsupportedPackageSchemaVersion { found, supported })
                if found == PACKAGE_SCHEMA_VERSION + 1 && supported == PACKAGE_SCHEMA_VERSION
        ));
    }

    #[test]
    fn project_package_rejects_invalid_manifest_project_path() {
        let (model, assets) = textured_project();
        let mut entries = package_entries(&model, &assets);

        let mut manifest = package_manifest(&assets).unwrap();
        manifest.project = "other.framer".to_owned();
        entries.insert("manifest.json".to_owned(), manifest_bytes(&manifest));
        assert!(matches!(
            load_project_package(&test_zip(&entries)),
            Err(LibraryImportError::InvalidPackageProjectPath { path }) if path == "other.framer"
        ));
    }

    #[test]
    fn project_package_rejects_invalid_manifest_asset_path() {
        let (model, assets) = textured_project();
        let mut entries = package_entries(&model, &assets);
        let mut manifest = package_manifest(&assets).unwrap();
        manifest.assets[0].path = "assets/not-the-content-hash".to_owned();
        entries.insert("manifest.json".to_owned(), manifest_bytes(&manifest));

        assert!(matches!(
            load_project_package(&test_zip(&entries)),
            Err(LibraryImportError::InvalidPackageAssetPath { path, .. })
                if path == "assets/not-the-content-hash"
        ));
    }

    #[test]
    fn project_package_rejects_asset_hash_mismatch() {
        let (model, mut assets) = textured_project();
        let hash = assets.keys().next().unwrap().clone();
        assets.insert(hash, b"different-bytes".to_vec());

        assert!(matches!(
            save_project_package(&model, &assets),
            Err(LibraryImportError::AssetHashMismatch { .. })
        ));
    }

    #[test]
    fn project_package_from_store_includes_only_available_referenced_assets() {
        let root = temp_path("package-store");
        let store = ContentAddressedAssetStore::new(&root);
        let (model, assets) = textured_project();
        let (hash, bytes) = assets.iter().next().unwrap();
        assert_eq!(store.put(bytes).unwrap(), *hash);

        let package = save_project_package_from_store(&model, &store).unwrap();
        let loaded = load_project_package(&package).unwrap();

        assert_eq!(loaded.assets, assets);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn resolver_loads_builtin_and_local_libraries() {
        let resolver = LocalSearchPathResolver::default();
        let builtin = load_verified_library(
            &resolver
                .resolve(&Locator::Builtin {
                    id: "framer-starter".to_owned(),
                })
                .unwrap(),
        )
        .unwrap();
        let local = load_verified_library(
            &resolver
                .resolve(&Locator::Local {
                    path: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                        .join("../../libraries/framer-starter.framerlib"),
                })
                .unwrap(),
        )
        .unwrap();

        assert_eq!(builtin.library.coordinate, "framer-lib://framer/starter");
        assert_eq!(local.library.coordinate, builtin.library.coordinate);
        assert_eq!(local.content_hash, builtin.content_hash);
    }

    #[test]
    fn vendored_item_provenance_uses_source_normalized_content_hashes() {
        let mut library = fixture_library();
        let stale_source = Provenance {
            library_uid: "22222222-2222-4222-8222-222222222222".to_owned(),
            version_id: "019e9150-0000-7000-8000-000000000002".to_owned(),
            source_id: ElementId::new("stale-source"),
            content_hash: "blake3:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
                .to_owned(),
        };
        library.materials[0].source = MaterialSource::Library(stale_source.clone());
        library.systems[0].source = Some(stale_source);
        let library_hash = library_content_hash(&library).unwrap();
        let expected_material_hash = material_content_hash(&library.materials[0]).unwrap();
        let expected_system_hash = system_content_hash(&library.systems[0]).unwrap();
        let mut project = BuildingModel::new(CodeProfile::irc_2021_prescriptive());

        let imported_material = import_material(
            &mut project,
            &library,
            &library_hash,
            &ElementId::new("mat-cedar"),
        )
        .unwrap();
        let material = project.material(&imported_material.materials[0]).unwrap();
        let MaterialSource::Library(material_source) = &material.source else {
            panic!("vendored material should have provenance");
        };
        assert_eq!(material_source.content_hash, expected_material_hash);

        let imported_system = import_system(
            &mut project,
            &library,
            &library_hash,
            &ElementId::new("system-rainscreen"),
        )
        .unwrap();
        let system = project
            .systems
            .iter()
            .find(|system| Some(&system.id) == imported_system.system.as_ref())
            .unwrap();
        assert_eq!(
            system.source.as_ref().unwrap().content_hash,
            expected_system_hash
        );
        project.validate().unwrap();
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
    fn newly_imported_items_have_no_lifecycle_issues() {
        let library = fixture_library();
        let hash = library_content_hash(&library).unwrap();
        let mut project = BuildingModel::new(CodeProfile::irc_2021_prescriptive());

        import_system(
            &mut project,
            &library,
            &hash,
            &ElementId::new("system-rainscreen"),
        )
        .unwrap();

        assert_eq!(library_lifecycle_issues(&project, &[library]).unwrap(), []);
    }

    #[test]
    fn local_material_edits_emit_divergence_until_detached() {
        let library = fixture_library();
        let hash = library_content_hash(&library).unwrap();
        let mut project = BuildingModel::new(CodeProfile::irc_2021_prescriptive());
        let imported =
            import_material(&mut project, &library, &hash, &ElementId::new("mat-cedar")).unwrap();
        let local_id = imported.materials[0].clone();
        let material = project
            .materials
            .iter_mut()
            .find(|material| material.id == local_id)
            .unwrap();
        material.name = "Locally edited cedar".to_owned();

        let issues = library_lifecycle_issues(&project, &[]).unwrap();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].kind, LibraryIssueKind::Diverged);
        assert_eq!(issues[0].item, LibraryItem::Material(local_id.clone()));

        assert!(detach_material(&mut project, &local_id).unwrap());
        assert!(library_lifecycle_issues(&project, &[]).unwrap().is_empty());
        assert!(project.libraries.is_empty());
        assert!(!detach_material(&mut project, &local_id).unwrap());
    }

    #[test]
    fn local_system_edits_emit_divergence_after_clean_import() {
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
        let local_system_id = imported.system.unwrap();

        assert!(
            library_lifecycle_issues(&project, std::slice::from_ref(&library))
                .unwrap()
                .is_empty()
        );

        let system = project
            .systems
            .iter_mut()
            .find(|system| system.id == local_system_id)
            .unwrap();
        system.layers[1].framing.as_mut().unwrap().cavity_material = None;

        let issues = library_lifecycle_issues(&project, &[]).unwrap();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].kind, LibraryIssueKind::Diverged);
        assert_eq!(issues[0].item, LibraryItem::System(local_system_id));
    }

    #[test]
    fn missing_source_items_emit_source_missing_for_materials_and_systems() {
        let library = fixture_library();
        let hash = library_content_hash(&library).unwrap();

        let mut material_project = BuildingModel::new(CodeProfile::irc_2021_prescriptive());
        let imported_material = import_material(
            &mut material_project,
            &library,
            &hash,
            &ElementId::new("mat-cedar"),
        )
        .unwrap();
        let local_material_id = imported_material.materials[0].clone();
        let mut material_missing = library.clone();
        material_missing
            .materials
            .retain(|material| material.id != ElementId::new("mat-cedar"));

        let material_issues =
            library_lifecycle_issues(&material_project, &[material_missing]).unwrap();
        assert_eq!(material_issues.len(), 1);
        assert_eq!(material_issues[0].kind, LibraryIssueKind::SourceMissing);
        assert_eq!(
            material_issues[0].item,
            LibraryItem::Material(local_material_id)
        );

        let mut system_project = BuildingModel::new(CodeProfile::irc_2021_prescriptive());
        let imported_system = import_system(
            &mut system_project,
            &library,
            &hash,
            &ElementId::new("system-rainscreen"),
        )
        .unwrap();
        let local_system_id = imported_system.system.unwrap();
        let mut system_missing = library;
        system_missing
            .systems
            .retain(|system| system.id != ElementId::new("system-rainscreen"));

        let system_issues = library_lifecycle_issues(&system_project, &[system_missing]).unwrap();
        assert_eq!(system_issues.len(), 1);
        assert_eq!(system_issues[0].kind, LibraryIssueKind::SourceMissing);
        assert_eq!(system_issues[0].item, LibraryItem::System(local_system_id));
    }

    #[test]
    fn updated_library_content_emits_out_of_date_and_resync_updates_material() {
        let library = fixture_library();
        let hash = library_content_hash(&library).unwrap();
        let mut project = BuildingModel::new(CodeProfile::irc_2021_prescriptive());
        let imported =
            import_material(&mut project, &library, &hash, &ElementId::new("mat-cedar")).unwrap();
        let local_id = imported.materials[0].clone();

        let mut updated = library.clone();
        updated.version_id = "019e9150-0000-7000-8000-000000000099".to_owned();
        updated.version = "1.1.0".to_owned();
        updated.materials[0].name = "Updated cedar".to_owned();
        let updated_hash = library_content_hash(&updated).unwrap();

        let issues = library_lifecycle_issues(&project, &[updated.clone()]).unwrap();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].kind, LibraryIssueKind::OutOfDate);
        assert_eq!(issues[0].item, LibraryItem::Material(local_id.clone()));

        resync_material(&mut project, &updated, &updated_hash, &local_id).unwrap();
        let material = project.material(&local_id).unwrap();
        assert_eq!(material.name, "Updated cedar");
        let MaterialSource::Library(source) = &material.source else {
            panic!("re-synced material should stay library-backed");
        };
        assert_eq!(source.version_id, updated.version_id);
        assert_eq!(project.libraries.len(), 1);
        assert_eq!(project.libraries[0].version_id, updated.version_id);
        assert!(
            library_lifecycle_issues(&project, &[updated])
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn resync_system_preserves_project_ids_and_updates_material_closure() {
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
        let local_system_id = imported.system.unwrap();
        let local_material_ids = imported.materials;

        let mut updated = library.clone();
        updated.version_id = "019e9150-0000-7000-8000-000000000100".to_owned();
        updated.version = "1.1.0".to_owned();
        updated.systems[0].name = "Updated rainscreen wall".to_owned();
        updated.materials[0].name = "Updated cedar".to_owned();
        let updated_hash = library_content_hash(&updated).unwrap();

        let issues = library_lifecycle_issues(&project, &[updated.clone()]).unwrap();
        assert!(
            issues.iter().any(|issue| {
                issue.kind == LibraryIssueKind::OutOfDate
                    && issue.item == LibraryItem::System(local_system_id.clone())
            }),
            "changed source system should be reported out of date"
        );

        resync_system(&mut project, &updated, &updated_hash, &local_system_id).unwrap();

        let system = project
            .systems
            .iter()
            .find(|system| system.id == local_system_id)
            .unwrap();
        assert_eq!(system.name, "Updated rainscreen wall");
        assert_eq!(
            system.layers[0].material,
            ElementId::new("acme-walls-mat-cedar")
        );
        assert_eq!(
            system.layers[1].framing.as_ref().unwrap().cavity_material,
            Some(ElementId::new("acme-walls-mat-mineral-wool"))
        );
        assert_eq!(
            system.source.as_ref().unwrap().version_id,
            updated.version_id
        );
        for local_material_id in local_material_ids {
            let material = project.material(&local_material_id).unwrap();
            let MaterialSource::Library(source) = &material.source else {
                panic!("closure material should stay library-backed");
            };
            assert_eq!(source.version_id, updated.version_id);
        }
        assert_eq!(
            project
                .material(&ElementId::new("acme-walls-mat-cedar"))
                .unwrap()
                .name,
            "Updated cedar"
        );
        assert!(
            library_lifecycle_issues(&project, &[updated])
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn detach_system_clears_source_and_reports_false_when_already_detached() {
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
        let local_system_id = imported.system.unwrap();

        for material_id in imported.materials {
            assert!(detach_material(&mut project, &material_id).unwrap());
        }
        assert_eq!(project.libraries.len(), 1);

        assert!(detach_system(&mut project, &local_system_id).unwrap());
        assert!(
            project
                .systems
                .iter()
                .find(|system| system.id == local_system_id)
                .unwrap()
                .source
                .is_none()
        );
        assert!(project.libraries.is_empty());
        assert!(
            library_lifecycle_issues(&project, &[library])
                .unwrap()
                .is_empty()
        );
        assert!(!detach_system(&mut project, &local_system_id).unwrap());
    }

    #[test]
    fn resync_rejects_mismatched_library_uid() {
        let library = fixture_library();
        let hash = library_content_hash(&library).unwrap();
        let mut project = BuildingModel::new(CodeProfile::irc_2021_prescriptive());
        let imported_material =
            import_material(&mut project, &library, &hash, &ElementId::new("mat-cedar")).unwrap();
        let imported_system = import_system(
            &mut project,
            &library,
            &hash,
            &ElementId::new("system-rainscreen"),
        )
        .unwrap();

        let mut impostor = library.clone();
        impostor.uid = "22222222-2222-4222-8222-222222222222".to_owned();
        let impostor_hash = library_content_hash(&impostor).unwrap();

        assert!(matches!(
            resync_material(
                &mut project,
                &impostor,
                &impostor_hash,
                &imported_material.materials[0],
            ),
            Err(LibraryImportError::LibraryUidMismatch { .. })
        ));
        assert!(matches!(
            resync_system(
                &mut project,
                &impostor,
                &impostor_hash,
                imported_system.system.as_ref().unwrap(),
            ),
            Err(LibraryImportError::LibraryUidMismatch { .. })
        ));
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
    fn failed_material_import_rolls_back_project_mutation() {
        let mut library = fixture_library();
        library.materials[0].id = ElementId::new("Bad Material");
        let mut project = BuildingModel::new(CodeProfile::irc_2021_prescriptive());
        let before = save_project(&project).unwrap();

        let result = import_material(
            &mut project,
            &library,
            "blake3:test",
            &ElementId::new("Bad Material"),
        );

        assert!(matches!(
            result,
            Err(LibraryImportError::Model(
                ModelError::InvalidElementId { .. }
            ))
        ));
        assert_eq!(save_project(&project).unwrap(), before);
        assert!(project.libraries.is_empty());
    }

    #[test]
    fn failed_system_import_rolls_back_project_mutation() {
        let mut library = fixture_library();
        library.systems[0].id = ElementId::new("Bad System");
        let mut project = BuildingModel::new(CodeProfile::irc_2021_prescriptive());
        let before = save_project(&project).unwrap();

        let result = import_system(
            &mut project,
            &library,
            "blake3:test",
            &ElementId::new("Bad System"),
        );

        assert!(matches!(
            result,
            Err(LibraryImportError::Model(
                ModelError::InvalidElementId { .. }
            ))
        ));
        assert_eq!(save_project(&project).unwrap(), before);
        assert!(project.libraries.is_empty());
    }

    #[test]
    fn import_errors_are_explicit_for_missing_items_and_resolvers() {
        let library = fixture_library();
        let mut project = BuildingModel::new(CodeProfile::irc_2021_prescriptive());

        assert!(matches!(
            import_material(
                &mut project,
                &library,
                "blake3:test",
                &ElementId::new("mat-missing"),
            ),
            Err(LibraryImportError::MaterialNotFound { id })
                if id == ElementId::new("mat-missing")
        ));
        assert!(matches!(
            import_system(
                &mut project,
                &library,
                "blake3:test",
                &ElementId::new("system-missing"),
            ),
            Err(LibraryImportError::SystemNotFound { id })
                if id == ElementId::new("system-missing")
        ));

        let mut dangling = fixture_library();
        dangling.systems[0].layers[0].material = ElementId::new("mat-missing");
        assert!(matches!(
            import_system(
                &mut project,
                &dangling,
                "blake3:test",
                &ElementId::new("system-rainscreen"),
            ),
            Err(LibraryImportError::MissingReferencedMaterial { system, material })
                if system == ElementId::new("system-rainscreen")
                    && material == ElementId::new("mat-missing")
        ));

        let resolver = LocalSearchPathResolver::default();
        assert!(matches!(
            resolver.resolve(&Locator::Builtin {
                id: "missing".to_owned(),
            }),
            Err(LibraryImportError::UnknownBuiltin { id }) if id == "missing"
        ));
        assert!(matches!(
            resolver.resolve(&Locator::Installed {
                id: "missing".to_owned(),
            }),
            Err(LibraryImportError::InstalledLibraryNotFound { id }) if id == "missing"
        ));
        assert!(matches!(
            resolver.resolve(&Locator::Remote {
                url: "https://example.invalid/library.framerlib".to_owned(),
                hash: "blake3:test".to_owned(),
            }),
            Err(LibraryImportError::RemoteUnsupported)
        ));
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
