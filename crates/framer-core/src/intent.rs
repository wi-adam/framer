use crate::ElementId;

/// Stable identity for one vendored library version recorded in authored project metadata.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
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

/// Closed, typed references to authored semantic entities that exist in schema v13.
///
/// This type is schema-neutral in Slice 1: no new field is added to [`crate::BuildingModel`].
/// Nested records keep their own stable [`ElementId`]; no vector index or display label enters
/// semantic identity. `Site` is the one singleton authored record without an element id.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
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
        }
    }
}

/// Future persisted assertion id. Defining its type does not add project data or change schema
/// v13; it establishes a namespace that can never be confused with derived assertion identity.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AuthoredIntentId(pub ElementId);

impl AuthoredIntentId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(ElementId::new(value))
    }
}
