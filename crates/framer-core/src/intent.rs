use serde::{Deserialize, Serialize};

use crate::{ElementId, Predicate};

/// Stable identity for one project-authored assertion.
///
/// Authored ids deliberately occupy a type-distinct namespace from regenerated assertion ids.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AuthoredIntentId(pub ElementId);

impl AuthoredIntentId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(ElementId::new(value))
    }
}

/// Stable identity for one project-authored intent override.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct IntentOverrideId(pub ElementId);

impl IntentOverrideId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(ElementId::new(value))
    }
}

/// Stable identity for one vendored library version recorded in authored project metadata.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LibraryVersionRef {
    pub uid: String,
    pub version_id: String,
}

impl LibraryVersionRef {
    pub fn new(uid: impl Into<String>, version_id: impl Into<String>) -> Self {
        Self {
            uid: uid.into(),
            version_id: version_id.into(),
        }
    }
}

/// Closed, typed references to authored semantic entities that exist in schema v14.
///
/// Nested records keep their own stable [`ElementId`]; no vector index or display label enters
/// semantic identity. `Site` is the one singleton authored record without an element id.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum AuthoredEntityRef {
    Site,
    LibraryVersion(LibraryVersionRef),
    StandardsPack(ElementId),
    Material(ElementId),
    ConstructionSystem(ElementId),
    Furnishing(ElementId),
    MepObject(ElementId),
    Level(ElementId),
    Wall(ElementId),
    Opening(ElementId),
    Dimension(ElementId),
    WallJoin(ElementId),
    Room(ElementId),
    FurnishingInstance(ElementId),
    MepInstance(ElementId),
    RoofPlane(ElementId),
    RoofOpening(ElementId),
    Ceiling(ElementId),
    FloorDeck(ElementId),
    BracedWallLine(ElementId),
    BracedPanel(ElementId),
    IntentOverride(IntentOverrideId),
}

impl AuthoredEntityRef {
    pub fn element_id(&self) -> Option<&ElementId> {
        match self {
            Self::Site | Self::LibraryVersion(_) => None,
            Self::StandardsPack(id)
            | Self::Material(id)
            | Self::ConstructionSystem(id)
            | Self::Furnishing(id)
            | Self::MepObject(id)
            | Self::Level(id)
            | Self::Wall(id)
            | Self::Opening(id)
            | Self::Dimension(id)
            | Self::WallJoin(id)
            | Self::Room(id)
            | Self::FurnishingInstance(id)
            | Self::MepInstance(id)
            | Self::RoofPlane(id)
            | Self::RoofOpening(id)
            | Self::Ceiling(id)
            | Self::FloorDeck(id)
            | Self::BracedWallLine(id)
            | Self::BracedPanel(id) => Some(id),
            Self::IntentOverride(id) => Some(&id.0),
        }
    }

    pub const fn kind_label(&self) -> &'static str {
        match self {
            Self::Site => "site context",
            Self::LibraryVersion(_) => "library version",
            Self::StandardsPack(_) => "standards pack",
            Self::Material(_) => "material",
            Self::ConstructionSystem(_) => "construction system",
            Self::Furnishing(_) => "furnishing family",
            Self::MepObject(_) => "MEP family",
            Self::Level(_) => "level",
            Self::Wall(_) => "wall",
            Self::Opening(_) => "opening",
            Self::Dimension(_) => "dimension",
            Self::WallJoin(_) => "wall join",
            Self::Room(_) => "room",
            Self::FurnishingInstance(_) => "furnishing instance",
            Self::MepInstance(_) => "MEP instance",
            Self::RoofPlane(_) => "roof plane",
            Self::RoofOpening(_) => "roof opening",
            Self::Ceiling(_) => "ceiling",
            Self::FloorDeck(_) => "floor deck",
            Self::BracedWallLine(_) => "braced wall line",
            Self::BracedPanel(_) => "braced panel",
            Self::IntentOverride(_) => "intent override",
        }
    }
}

/// Product-wide classification for authored intent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum IntentDomain {
    SpatialProgram,
    Construction,
    StructuralPerformance,
    EnvelopeBuildingScience,
    Mep,
    Compliance,
    Resource,
    FabricationInstallation,
    OperationalMaintenance,
    Aesthetic,
}

/// Deterministic preference tier. Larger numbers represent stronger preferences; zero is invalid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PreferencePriority(pub u16);

/// Boolean governing modes supported by the first persisted authored-intent slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum AuthoredIntentMode {
    Requirement,
    Preference { priority: PreferencePriority },
}

/// Exact project participants for one cross-object assertion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExactIntentScope {
    pub subject: AuthoredEntityRef,
    pub participants: Vec<AuthoredEntityRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum ProjectIntentScope {
    Exact(ExactIntentScope),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum IntentExpression {
    FactPredicate(Predicate),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum IntentSource {
    User,
}

/// Persisted project-authored cross-object assertion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntentAssertion {
    pub id: AuthoredIntentId,
    pub domain: IntentDomain,
    pub mode: AuthoredIntentMode,
    pub scope: ProjectIntentScope,
    pub expression: IntentExpression,
    pub source: IntentSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
}

/// Explicit authored exceptions to project assertions. A waiver changes the assertion outcome;
/// it is not independently evaluated as another assertion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum IntentOverride {
    Waive {
        id: IntentOverrideId,
        target: AuthoredIntentId,
        reason: String,
        source: IntentSource,
    },
}

impl IntentOverride {
    pub const fn id(&self) -> &IntentOverrideId {
        match self {
            Self::Waive { id, .. } => id,
        }
    }

    pub const fn target(&self) -> &AuthoredIntentId {
        match self {
            Self::Waive { target, .. } => target,
        }
    }
}
