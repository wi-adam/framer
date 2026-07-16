use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::standards::{
    BracedPanel, BracedWallLine, FramingDefaults, ResolvedStandards, SiteContext, StandardsPack,
    resolve_standards,
};
use crate::{
    ConstraintSystem, ConstraintVariable, Length, LinearConstraint, LinearExpression, Point2,
    PolygonTriangulation, triangulate_polygon_with_holes,
};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ElementId(pub String);

impl ElementId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn is_valid(&self) -> bool {
        let mut chars = self.0.chars();
        let Some(first) = chars.next() else {
            return false;
        };

        is_id_start(first) && chars.all(is_id_continue)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LibraryStamp {
    pub uid: String,
    pub version_id: String,
    pub content_hash: String,
    pub coordinate: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    pub library_uid: String,
    pub version_id: String,
    pub source_id: ElementId,
    pub content_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildingModel {
    #[serde(default)]
    pub site: SiteContext,
    pub standards: Vec<ElementId>,
    pub standards_packs: Vec<StandardsPack>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub libraries: Vec<LibraryStamp>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub materials: Vec<Material>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub systems: Vec<ConstructionSystem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub furnishings: Vec<Furnishing>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mep_objects: Vec<MepObject>,
    #[serde(default = "default_levels")]
    pub levels: Vec<Level>,
    pub walls: Vec<Wall>,
    #[serde(default)]
    pub wall_joins: Vec<WallJoin>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rooms: Vec<Room>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub furnishing_instances: Vec<FurnishingInstance>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mep_instances: Vec<MepInstance>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roof_planes: Vec<RoofPlane>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ceilings: Vec<Ceiling>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub floor_decks: Vec<FloorDeck>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub braced_wall_lines: Vec<BracedWallLine>,
}

/// Derived local-x spans for one wall's physical corner laps. These values are
/// disposable solver/presentation facts: authored wall endpoints and opening
/// offsets remain centerline-based and are never mutated by corner treatment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WallPhysicalSpans {
    pub primary_framing: (Length, Length),
    pub counter_lap_framing: (Length, Length),
    pub envelope: (f64, f64),
}

impl Default for BuildingModel {
    fn default() -> Self {
        Self::new()
    }
}

impl BuildingModel {
    pub fn new() -> Self {
        let (materials, systems) = Self::starter_library();
        let (standards, standards_packs) = default_standards_stack();
        Self {
            site: SiteContext::default(),
            standards,
            standards_packs,
            libraries: Vec::new(),
            materials,
            systems,
            furnishings: Vec::new(),
            mep_objects: Vec::new(),
            levels: default_levels(),
            walls: Vec::new(),
            wall_joins: Vec::new(),
            rooms: Vec::new(),
            furnishing_instances: Vec::new(),
            mep_instances: Vec::new(),
            roof_planes: Vec::new(),
            ceilings: Vec::new(),
            floor_decks: Vec::new(),
            braced_wall_lines: Vec::new(),
        }
    }

    pub fn demo_wall() -> Self {
        let (standards, standards_packs) = default_standards_stack();
        let defaults = standards_packs[0].tables.defaults.clone();
        let mut wall = Wall::new("wall-1", "Demo wall", Length::from_feet(28.0), &defaults);
        wall.openings = vec![
            Opening::door(
                "opening-door-1",
                "Door",
                Length::from_inches(36.0),
                Length::from_inches(36.0),
                Length::from_inches(80.0),
            ),
            Opening::door(
                "opening-garage-1",
                "Garage Door",
                Length::from_inches(232.0),
                Length::from_inches(96.0),
                Length::from_inches(84.0),
            )
            .with_kind(OpeningKind::GarageDoor),
            Opening::window(
                "opening-window-1",
                "Window",
                Length::from_inches(108.0),
                Length::from_inches(48.0),
                Length::from_inches(48.0),
                Length::from_inches(36.0),
            ),
        ];

        let (materials, systems) = Self::starter_library();
        Self {
            site: SiteContext::default(),
            standards,
            standards_packs,
            libraries: Vec::new(),
            materials,
            systems,
            furnishings: Vec::new(),
            mep_objects: Vec::new(),
            levels: default_levels(),
            walls: vec![wall],
            wall_joins: Vec::new(),
            rooms: Vec::new(),
            furnishing_instances: Vec::new(),
            mep_instances: Vec::new(),
            roof_planes: Vec::new(),
            ceilings: Vec::new(),
            floor_decks: Vec::new(),
            braced_wall_lines: Vec::new(),
        }
    }

    pub fn demo_shell() -> Self {
        let (standards, standards_packs) = default_standards_stack();
        let defaults = standards_packs[0].tables.defaults.clone();
        let mut front = Wall::new(
            "wall-front",
            "Front wall",
            Length::from_feet(28.0),
            &defaults,
        )
        .with_placement(
            "level-1",
            Point2::new(Length::ZERO, Length::ZERO),
            Point2::new(Length::from_feet(28.0), Length::ZERO),
        );
        front.openings = vec![
            Opening::door(
                "opening-front-door",
                "Front door",
                Length::from_feet(5.0),
                Length::from_inches(36.0),
                Length::from_inches(80.0),
            ),
            Opening::door(
                "opening-front-garage",
                "Garage door",
                Length::from_feet(19.5),
                Length::from_feet(8.0),
                Length::from_inches(84.0),
            )
            .with_kind(OpeningKind::GarageDoor),
        ];

        let mut right = Wall::new(
            "wall-right",
            "Right wall",
            Length::from_feet(20.0),
            &defaults,
        )
        .with_placement(
            "level-1",
            Point2::new(Length::from_feet(28.0), Length::ZERO),
            Point2::new(Length::from_feet(28.0), Length::from_feet(20.0)),
        );
        right.openings.push(Opening::window(
            "opening-right-window",
            "Right window",
            Length::from_feet(10.0),
            Length::from_inches(42.0),
            Length::from_inches(42.0),
            Length::from_inches(36.0),
        ));

        let mut back = Wall::new("wall-back", "Back wall", Length::from_feet(28.0), &defaults)
            .with_placement(
                "level-1",
                Point2::new(Length::from_feet(28.0), Length::from_feet(20.0)),
                Point2::new(Length::ZERO, Length::from_feet(20.0)),
            );
        back.openings = vec![
            Opening::window(
                "opening-back-left-window",
                "Back left window",
                Length::from_feet(7.0),
                Length::from_feet(4.0),
                Length::from_feet(4.0),
                Length::from_feet(3.0),
            ),
            Opening::window(
                "opening-back-right-window",
                "Back right window",
                Length::from_feet(20.0),
                Length::from_feet(4.0),
                Length::from_feet(4.0),
                Length::from_feet(3.0),
            ),
        ];

        let mut left = Wall::new("wall-left", "Left wall", Length::from_feet(20.0), &defaults)
            .with_placement(
                "level-1",
                Point2::new(Length::ZERO, Length::from_feet(20.0)),
                Point2::new(Length::ZERO, Length::ZERO),
            );
        left.openings.push(Opening::door(
            "opening-left-service-door",
            "Service door",
            Length::from_feet(8.0),
            Length::from_inches(36.0),
            Length::from_inches(80.0),
        ));

        let (materials, systems) = Self::starter_library();
        Self {
            site: SiteContext::default(),
            standards,
            standards_packs,
            libraries: Vec::new(),
            materials,
            systems,
            furnishings: Vec::new(),
            mep_objects: Vec::new(),
            levels: default_levels(),
            walls: vec![front, right, back, left],
            wall_joins: vec![
                WallJoin::corner(
                    "join-front-right",
                    "Front right corner",
                    "wall-front",
                    "wall-right",
                    Point2::new(Length::from_feet(28.0), Length::ZERO),
                ),
                WallJoin::corner(
                    "join-right-back",
                    "Right back corner",
                    "wall-right",
                    "wall-back",
                    Point2::new(Length::from_feet(28.0), Length::from_feet(20.0)),
                ),
                WallJoin::corner(
                    "join-back-left",
                    "Back left corner",
                    "wall-back",
                    "wall-left",
                    Point2::new(Length::ZERO, Length::from_feet(20.0)),
                ),
                WallJoin::corner(
                    "join-left-front",
                    "Left front corner",
                    "wall-left",
                    "wall-front",
                    Point2::new(Length::ZERO, Length::ZERO),
                ),
            ],
            rooms: Vec::new(),
            furnishing_instances: Vec::new(),
            mep_instances: Vec::new(),
            roof_planes: Vec::new(),
            ceilings: Vec::new(),
            floor_decks: Vec::new(),
            braced_wall_lines: Vec::new(),
        }
        .into_deterministic()
    }

    /// A 24ft × 16ft shell partitioned into two bedrooms and a living area by
    /// interior walls that meet the exterior (and each other) at tee joins. Used
    /// as the rooms/interior-walls example project.
    pub fn demo_two_bedroom() -> Self {
        let (standards, standards_packs) = default_standards_stack();
        let defaults = standards_packs[0].tables.defaults.clone();
        let ft = Length::from_feet;
        let wall = |id: &str, name: &str, start: Point2, end: Point2| {
            Wall::new(id, name, ft(1.0), &defaults).with_placement("level-1", start, end)
        };

        let mut front = wall(
            "wall-front",
            "Front wall",
            Point2::new(Length::ZERO, Length::ZERO),
            Point2::new(ft(24.0), Length::ZERO),
        );
        front.openings.push(Opening::door(
            "opening-front-door",
            "Front door",
            ft(6.0),
            Length::from_inches(36.0),
            Length::from_inches(80.0),
        ));

        let mut walls = vec![
            front,
            wall(
                "wall-right",
                "Right wall",
                Point2::new(ft(24.0), Length::ZERO),
                Point2::new(ft(24.0), ft(16.0)),
            ),
            wall(
                "wall-back",
                "Back wall",
                Point2::new(ft(24.0), ft(16.0)),
                Point2::new(Length::ZERO, ft(16.0)),
            ),
            wall(
                "wall-left",
                "Left wall",
                Point2::new(Length::ZERO, ft(16.0)),
                Point2::new(Length::ZERO, Length::ZERO),
            ),
            wall(
                "wall-mid",
                "Center partition",
                Point2::new(ft(12.0), Length::ZERO),
                Point2::new(ft(12.0), ft(16.0)),
            ),
            wall(
                "wall-bed",
                "Bedroom partition",
                Point2::new(Length::ZERO, ft(8.0)),
                Point2::new(ft(12.0), ft(8.0)),
            ),
        ];

        // The two interior partitions use the interior-partition system; only the
        // perimeter (front/right/back/left) stays on the exterior system.
        for wall in &mut walls {
            if wall.id == ElementId::new("wall-mid") || wall.id == ElementId::new("wall-bed") {
                wall.system = ElementId::new("system-wall-interior-1");
            }
        }

        let wall_joins = vec![
            WallJoin::corner(
                "join-front-right",
                "Front right corner",
                "wall-front",
                "wall-right",
                Point2::new(ft(24.0), Length::ZERO),
            ),
            WallJoin::corner(
                "join-right-back",
                "Right back corner",
                "wall-right",
                "wall-back",
                Point2::new(ft(24.0), ft(16.0)),
            ),
            WallJoin::corner(
                "join-back-left",
                "Back left corner",
                "wall-back",
                "wall-left",
                Point2::new(Length::ZERO, ft(16.0)),
            ),
            WallJoin::corner(
                "join-left-front",
                "Left front corner",
                "wall-left",
                "wall-front",
                Point2::new(Length::ZERO, Length::ZERO),
            ),
            WallJoin::new(
                "join-mid-front",
                "Center partition at front",
                WallJoinKind::Tee,
                "wall-front",
                "wall-mid",
                Point2::new(ft(12.0), Length::ZERO),
            ),
            WallJoin::new(
                "join-mid-back",
                "Center partition at back",
                WallJoinKind::Tee,
                "wall-back",
                "wall-mid",
                Point2::new(ft(12.0), ft(16.0)),
            ),
            WallJoin::new(
                "join-bed-left",
                "Bedroom partition at left",
                WallJoinKind::Tee,
                "wall-left",
                "wall-bed",
                Point2::new(Length::ZERO, ft(8.0)),
            ),
            WallJoin::new(
                "join-bed-mid",
                "Bedroom partition at center",
                WallJoinKind::Tee,
                "wall-mid",
                "wall-bed",
                Point2::new(ft(12.0), ft(8.0)),
            ),
        ];

        let rooms = vec![
            Room::new(
                "room-bed-1",
                "Bedroom 1",
                RoomUsage::Bedroom,
                "level-1",
                Point2::new(ft(6.0), ft(4.0)),
            ),
            Room::new(
                "room-bed-2",
                "Bedroom 2",
                RoomUsage::Bedroom,
                "level-1",
                Point2::new(ft(6.0), ft(12.0)),
            ),
            Room::new(
                "room-living",
                "Living",
                RoomUsage::Living,
                "level-1",
                Point2::new(ft(18.0), ft(8.0)),
            ),
        ];

        let (materials, systems) = Self::starter_library();
        Self {
            site: SiteContext::default(),
            standards,
            standards_packs,
            libraries: Vec::new(),
            materials,
            systems,
            furnishings: Vec::new(),
            mep_objects: Vec::new(),
            levels: default_levels(),
            walls,
            wall_joins,
            rooms,
            furnishing_instances: Vec::new(),
            mep_instances: Vec::new(),
            roof_planes: Vec::new(),
            ceilings: Vec::new(),
            floor_decks: Vec::new(),
            braced_wall_lines: Vec::new(),
        }
        .into_deterministic()
    }

    pub fn resolved_standards(&self) -> ResolvedStandards {
        let stack: Vec<&StandardsPack> = self
            .standards
            .iter()
            .filter_map(|id| self.standards_packs.iter().find(|pack| pack.id == *id))
            .collect();
        resolve_standards(&stack)
    }

    pub fn framing_defaults(&self) -> FramingDefaults {
        self.standards
            .iter()
            .rev()
            .find_map(|id| self.standards_packs.iter().find(|pack| pack.id == *id))
            .map(|pack| pack.tables.defaults.clone())
            .unwrap_or_else(FramingDefaults::irc_2021_starter)
    }

    pub fn base_standards_name(&self) -> Option<&str> {
        let base = self.standards.first()?;
        self.standards_packs
            .iter()
            .find(|pack| pack.id == *base)
            .map(|pack| pack.name.as_str())
    }

    pub fn validate(&self) -> Result<(), ModelError> {
        let mut ids = BTreeSet::new();
        let mut level_ids = BTreeSet::new();
        for level in &self.levels {
            validate_element_id(&level.id)?;
            insert_unique_id(&mut ids, &level.id)?;
            level_ids.insert(level.id.clone());
        }

        if self.levels.is_empty() {
            return Err(ModelError::MissingLevel);
        }

        let mut standards_lookup = BTreeSet::new();
        for pack in &self.standards_packs {
            validate_element_id(&pack.id)?;
            insert_unique_id(&mut ids, &pack.id)?;
            pack.validate()?;
            standards_lookup.insert(pack.id.clone());
        }
        let mut standards_stack = BTreeSet::new();
        for pack in &self.standards {
            validate_element_id(pack)?;
            if !standards_stack.insert(pack.clone()) {
                return Err(ModelError::StandardsStackDuplicatePack { pack: pack.clone() });
            }
            if !standards_lookup.contains(pack) {
                return Err(ModelError::StandardsStackReferencesUnknownPack { pack: pack.clone() });
            }
        }

        let mut material_lookup = BTreeMap::new();
        for material in &self.materials {
            validate_element_id(&material.id)?;
            insert_unique_id(&mut ids, &material.id)?;
            material.validate()?;
            material_lookup.insert(material.id.clone(), material);
        }

        let mut system_lookup = BTreeMap::new();
        for system in &self.systems {
            system.validate(&material_lookup, &mut ids)?;
            system_lookup.insert(system.id.clone(), system);
        }

        let mut furnishing_lookup = BTreeMap::new();
        for furnishing in &self.furnishings {
            furnishing.validate(&mut ids)?;
            furnishing_lookup.insert(furnishing.id.clone(), furnishing);
        }

        let mut mep_lookup = BTreeMap::new();
        for object in &self.mep_objects {
            object.validate(&mut ids)?;
            mep_lookup.insert(object.id.clone(), object);
        }

        let mut wall_lookup = BTreeMap::new();
        for wall in &self.walls {
            wall.validate()?;
            insert_unique_id(&mut ids, &wall.id)?;
            if !level_ids.contains(&wall.level) {
                return Err(ModelError::WallReferencesUnknownLevel {
                    wall: wall.id.clone(),
                    level: wall.level.clone(),
                });
            }
            match system_lookup.get(&wall.system) {
                Some(system) if system.kind == SystemKind::Wall => {}
                Some(_) => {
                    return Err(ModelError::WallSystemWrongKind {
                        wall: wall.id.clone(),
                        system: wall.system.clone(),
                    });
                }
                None => {
                    return Err(ModelError::WallReferencesUnknownSystem {
                        wall: wall.id.clone(),
                        system: wall.system.clone(),
                    });
                }
            }
            wall_lookup.insert(wall.id.clone(), wall);
            for opening in &wall.openings {
                insert_unique_id(&mut ids, &opening.id)?;
            }
            for dimension in &wall.dimensions {
                insert_unique_id(&mut ids, &dimension.id)?;
            }
            for panel in &wall.bracing {
                validate_element_id(&panel.id)?;
                insert_unique_id(&mut ids, &panel.id)?;
                if panel.length <= Length::ZERO {
                    return Err(ModelError::BracingPanelInvalidLength {
                        wall: wall.id.clone(),
                        panel: panel.id.clone(),
                    });
                }
                if panel.offset < Length::ZERO || panel.offset + panel.length > wall.length {
                    return Err(ModelError::BracingPanelOutOfBounds {
                        wall: wall.id.clone(),
                        panel: panel.id.clone(),
                    });
                }
            }
        }

        for join in &self.wall_joins {
            join.validate()?;
            insert_unique_id(&mut ids, &join.id)?;

            let Some(first) = wall_lookup.get(&join.first_wall) else {
                return Err(ModelError::JoinReferencesUnknownWall {
                    join: join.id.clone(),
                    wall: join.first_wall.clone(),
                });
            };
            let Some(second) = wall_lookup.get(&join.second_wall) else {
                return Err(ModelError::JoinReferencesUnknownWall {
                    join: join.id.clone(),
                    wall: join.second_wall.clone(),
                });
            };

            // The join point must connect the two walls per the join kind:
            // - Corner/EndToEnd: an endpoint of both walls.
            // - Tee: an endpoint of one (the partition) on the interior of the
            //   other (the through wall).
            // - Cross: interior to both walls.
            let connects = match join.kind {
                WallJoinKind::Corner | WallJoinKind::EndToEnd => {
                    first.has_endpoint(join.point) && second.has_endpoint(join.point)
                }
                WallJoinKind::Tee => {
                    (first.has_endpoint(join.point) && second.point_on_interior(join.point))
                        || (second.has_endpoint(join.point) && first.point_on_interior(join.point))
                }
                WallJoinKind::Cross => {
                    first.point_on_interior(join.point) && second.point_on_interior(join.point)
                }
            };
            if !connects {
                return Err(ModelError::JoinPointDoesNotConnectWalls {
                    join: join.id.clone(),
                });
            }
        }

        for instance in &self.furnishing_instances {
            instance.validate(&mut ids)?;
            if !level_ids.contains(&instance.level) {
                return Err(ModelError::FurnishingInstanceReferencesUnknownLevel {
                    instance: instance.id.clone(),
                    level: instance.level.clone(),
                });
            }
            if !furnishing_lookup.contains_key(&instance.family) {
                return Err(ModelError::FurnishingInstanceReferencesUnknownFamily {
                    instance: instance.id.clone(),
                    family: instance.family.clone(),
                });
            }
        }

        for instance in &self.mep_instances {
            instance.validate(&mut ids)?;
            if !level_ids.contains(&instance.level) {
                return Err(ModelError::MepInstanceReferencesUnknownLevel {
                    instance: instance.id.clone(),
                    level: instance.level.clone(),
                });
            }
            if !mep_lookup.contains_key(&instance.family) {
                return Err(ModelError::MepInstanceReferencesUnknownFamily {
                    instance: instance.id.clone(),
                    family: instance.family.clone(),
                });
            }
        }

        for line in &self.braced_wall_lines {
            validate_element_id(&line.id)?;
            validate_element_id(&line.level)?;
            insert_unique_id(&mut ids, &line.id)?;
            if !level_ids.contains(&line.level) {
                return Err(ModelError::BracedWallLineReferencesUnknownLevel {
                    braced_wall_line: line.id.clone(),
                    level: line.level.clone(),
                });
            }
        }

        let mut room_ids = BTreeSet::new();
        for room in &self.rooms {
            validate_element_id(&room.id)?;
            insert_unique_id(&mut ids, &room.id)?;
            if !level_ids.contains(&room.level) {
                return Err(ModelError::RoomReferencesUnknownLevel {
                    room: room.id.clone(),
                    level: room.level.clone(),
                });
            }
            room_ids.insert(room.id.clone());
        }

        for roof in &self.roof_planes {
            validate_element_id(&roof.id)?;
            insert_unique_id(&mut ids, &roof.id)?;
            if !level_ids.contains(&roof.level) {
                return Err(ModelError::RoofPlaneReferencesUnknownLevel {
                    roof_plane: roof.id.clone(),
                    level: roof.level.clone(),
                });
            }
            match system_lookup.get(&roof.system) {
                Some(system) if system.kind == SystemKind::Roof => {}
                Some(_) => {
                    return Err(ModelError::RoofPlaneSystemWrongKind {
                        roof_plane: roof.id.clone(),
                        system: roof.system.clone(),
                    });
                }
                None => {
                    return Err(ModelError::RoofPlaneReferencesUnknownSystem {
                        roof_plane: roof.id.clone(),
                        system: roof.system.clone(),
                    });
                }
            }
            roof.validate_geometry(&mut ids)?;
        }
        for (index, first) in self.roof_planes.iter().enumerate() {
            for second in &self.roof_planes[index + 1..] {
                let shares_edge = first.level == second.level
                    && (0..first.outline.len())
                        .any(|edge| roof_planes_share_edge(first, edge, second));
                if shares_edge
                    && (first.eave_overhang != second.eave_overhang
                        || first.rake_overhang != second.rake_overhang)
                {
                    return Err(ModelError::RoofPlaneConnectedOverhangMismatch {
                        first: first.id.clone(),
                        second: second.id.clone(),
                    });
                }
            }
        }

        for ceiling in &self.ceilings {
            validate_element_id(&ceiling.id)?;
            insert_unique_id(&mut ids, &ceiling.id)?;
            if !level_ids.contains(&ceiling.level) {
                return Err(ModelError::CeilingReferencesUnknownLevel {
                    ceiling: ceiling.id.clone(),
                    level: ceiling.level.clone(),
                });
            }
            match system_lookup.get(&ceiling.system) {
                Some(system) if system.kind == SystemKind::Ceiling => {}
                Some(_) => {
                    return Err(ModelError::CeilingSystemWrongKind {
                        ceiling: ceiling.id.clone(),
                        system: ceiling.system.clone(),
                    });
                }
                None => {
                    return Err(ModelError::CeilingReferencesUnknownSystem {
                        ceiling: ceiling.id.clone(),
                        system: ceiling.system.clone(),
                    });
                }
            }
            validate_surface_region(&ceiling.region, &room_ids, &ceiling.id)?;
            // A sloped ceiling is a planar surface like a roof plane: it needs an
            // explicit polygon outline (a Room boundary has no stable edge order)
            // whose `low_edge` it springs from, and a positive run. Flat ceilings
            // (`slope == None`) keep today's checks. Mirrors `RoofPlane` validation.
            if let Some(slope) = &ceiling.slope {
                let SurfaceRegion::Polygon(points) = &ceiling.region else {
                    return Err(ModelError::CeilingSlopeRequiresPolygonRegion {
                        ceiling: ceiling.id.clone(),
                    });
                };
                if (slope.low_edge as usize) >= points.len() {
                    return Err(ModelError::CeilingSlopeLowEdgeOutOfRange {
                        ceiling: ceiling.id.clone(),
                    });
                }
                if slope.pitch.run <= Length::ZERO {
                    return Err(ModelError::CeilingInvalidSlope {
                        ceiling: ceiling.id.clone(),
                    });
                }
            }
        }

        for deck in &self.floor_decks {
            validate_element_id(&deck.id)?;
            insert_unique_id(&mut ids, &deck.id)?;
            if !level_ids.contains(&deck.level) {
                return Err(ModelError::FloorDeckReferencesUnknownLevel {
                    floor_deck: deck.id.clone(),
                    level: deck.level.clone(),
                });
            }
            match system_lookup.get(&deck.system) {
                Some(system) if system.kind == SystemKind::Floor => {}
                Some(_) => {
                    return Err(ModelError::FloorDeckSystemWrongKind {
                        floor_deck: deck.id.clone(),
                        system: deck.system.clone(),
                    });
                }
                None => {
                    return Err(ModelError::FloorDeckReferencesUnknownSystem {
                        floor_deck: deck.id.clone(),
                        system: deck.system.clone(),
                    });
                }
            }
            validate_surface_region(&deck.region, &room_ids, &deck.id)?;
        }

        Ok(())
    }

    pub fn sort_deterministically(&mut self) {
        self.standards_packs
            .sort_by(|left, right| left.id.cmp(&right.id));
        self.libraries.sort_by(|left, right| {
            left.uid
                .cmp(&right.uid)
                .then_with(|| left.version_id.cmp(&right.version_id))
        });
        self.materials.sort_by(|left, right| left.id.cmp(&right.id));
        // Systems sort by id; layer ORDER is semantic (interior -> exterior) and
        // must never be reordered.
        self.systems.sort_by(|left, right| left.id.cmp(&right.id));
        self.furnishings
            .sort_by(|left, right| left.id.cmp(&right.id));
        self.mep_objects
            .sort_by(|left, right| left.id.cmp(&right.id));
        self.levels.sort_by(|left, right| left.id.cmp(&right.id));
        self.walls.sort_by(|left, right| left.id.cmp(&right.id));
        for wall in &mut self.walls {
            wall.sort_deterministically();
        }
        self.wall_joins
            .sort_by(|left, right| left.id.cmp(&right.id));
        self.rooms.sort_by(|left, right| left.id.cmp(&right.id));
        self.furnishing_instances
            .sort_by(|left, right| left.id.cmp(&right.id));
        self.mep_instances
            .sort_by(|left, right| left.id.cmp(&right.id));
        self.roof_planes
            .sort_by(|left, right| left.id.cmp(&right.id));
        // Outline order is geometry-significant and must never be reordered; only
        // the nested openings are id-keyed and canonicalized.
        for roof in &mut self.roof_planes {
            roof.openings.sort_by(|left, right| left.id.cmp(&right.id));
        }
        self.ceilings.sort_by(|left, right| left.id.cmp(&right.id));
        self.floor_decks
            .sort_by(|left, right| left.id.cmp(&right.id));
        self.braced_wall_lines
            .sort_by(|left, right| left.id.cmp(&right.id));
    }

    pub fn into_deterministic(mut self) -> Self {
        self.sort_deterministically();
        self
    }

    /// Resolve a wall's construction system from the project library. Later
    /// widened to search external/shared libraries via `MaterialSource`.
    pub fn system_for(&self, wall: &Wall) -> Option<&ConstructionSystem> {
        self.systems.iter().find(|system| system.id == wall.system)
    }

    /// The local-x span, in inches, used by physical wall envelopes. Authored
    /// wall length stays centerline-based. At a corner, one wall runs through to
    /// the adjoining wall's outside face and the adjoining wall retracts to the
    /// through wall's inside face, producing one closed butt/lap with no doubled
    /// volume. Half-tick values are retained exactly in this derived visual path.
    pub fn wall_envelope_span(&self, wall: &Wall) -> (f64, f64) {
        self.wall_physical_spans(wall).envelope
    }

    /// The primary local-x span for generated studs, bottom plates, and lower
    /// top plates. Structural joins meet at framing-layer faces rather than at
    /// the finished wall envelope's faces.
    pub fn wall_framing_span(&self, wall: &Wall) -> (Length, Length) {
        self.wall_physical_spans(wall).primary_framing
    }

    /// The opposite structural corner lap used by the upper member of a double
    /// top plate. Reversing the through/butt roles staggers its corner seam over
    /// the primary plate seam and ties the intersecting walls together.
    pub fn wall_counter_lap_framing_span(&self, wall: &Wall) -> (Length, Length) {
        self.wall_physical_spans(wall).counter_lap_framing
    }

    /// Derive all physical spans for one wall. Consumers processing many walls
    /// should use [`Self::wall_physical_spans_on_level`] so the level topology is
    /// solved once rather than once per wall and span kind.
    pub fn wall_physical_spans(&self, wall: &Wall) -> WallPhysicalSpans {
        let interior_sides = crate::topology::wall_interior_sides_on_level(self, &wall.level);
        self.wall_physical_spans_with_interior_sides(wall, &interior_sides)
    }

    /// Batched level-scoped form of [`Self::wall_physical_spans`].
    pub fn wall_physical_spans_on_level(
        &self,
        level: &ElementId,
    ) -> BTreeMap<ElementId, WallPhysicalSpans> {
        let interior_sides = crate::topology::wall_interior_sides_on_level(self, level);
        self.walls
            .iter()
            .filter(|wall| wall.level == *level)
            .map(|wall| {
                (
                    wall.id.clone(),
                    self.wall_physical_spans_with_interior_sides(wall, &interior_sides),
                )
            })
            .collect()
    }

    fn wall_physical_spans_with_interior_sides(
        &self,
        wall: &Wall,
        interior_sides: &BTreeMap<ElementId, bool>,
    ) -> WallPhysicalSpans {
        let primary_framing = self.wall_corner_span_half_ticks(
            wall,
            CornerLapPass::Primary,
            WallPhysicalBand::Framing,
            interior_sides,
        );
        let counter_lap_framing = self.wall_corner_span_half_ticks(
            wall,
            CornerLapPass::Counter,
            WallPhysicalBand::Framing,
            interior_sides,
        );
        let envelope = self.wall_corner_span_half_ticks(
            wall,
            CornerLapPass::Primary,
            WallPhysicalBand::Envelope,
            interior_sides,
        );
        WallPhysicalSpans {
            primary_framing: half_tick_span_to_lengths(primary_framing),
            counter_lap_framing: half_tick_span_to_lengths(counter_lap_framing),
            envelope: (
                half_ticks_to_inches(envelope.0),
                half_ticks_to_inches(envelope.1),
            ),
        }
    }

    fn wall_corner_span_half_ticks(
        &self,
        wall: &Wall,
        pass: CornerLapPass,
        band: WallPhysicalBand,
        interior_sides: &BTreeMap<ElementId, bool>,
    ) -> (i64, i64) {
        let mut start_extension = None;
        let mut start_retraction = None;
        let mut end_extension = None;
        let mut end_retraction = None;

        for join in &self.wall_joins {
            if !wall.has_endpoint(join.point) {
                continue;
            }
            let other_id = if join.first_wall == wall.id {
                Some(&join.second_wall)
            } else if join.second_wall == wall.id {
                Some(&join.first_wall)
            } else {
                None
            };
            let Some(other_id) = other_id else {
                continue;
            };
            let Some(other) = self
                .walls
                .iter()
                .find(|candidate| candidate.id == *other_id)
            else {
                continue;
            };
            if join.kind == WallJoinKind::Tee && !other.has_endpoint(join.point) {
                let along = wall_along_axis(wall);
                let direction = if wall.start == join.point {
                    along
                } else {
                    negate_axis(along)
                };
                let adjustment = self.wall_band_max_projection_half_ticks(
                    other,
                    direction,
                    band,
                    interior_sides,
                );
                if wall.start == join.point {
                    update_max(&mut start_retraction, adjustment);
                } else {
                    update_max(&mut end_retraction, adjustment);
                }
                continue;
            }
            if join.kind != WallJoinKind::Corner {
                continue;
            }
            let Some(primary_through) = self.primary_corner_through_wall(join, interior_sides)
            else {
                continue;
            };
            let through_id = match pass {
                CornerLapPass::Primary => primary_through,
                CornerLapPass::Counter if primary_through == join.first_wall => {
                    join.second_wall.clone()
                }
                CornerLapPass::Counter => join.first_wall.clone(),
            };
            let is_through = wall.id == through_id;
            let along = wall_along_axis(wall);
            let direction = if is_through {
                // Continue beyond the authored endpoint toward the far face of
                // the adjoining wall's selected band.
                if wall.start == join.point {
                    negate_axis(along)
                } else {
                    along
                }
            } else {
                // Move inward from the authored endpoint until reaching the far
                // face of the through wall's selected band.
                if wall.start == join.point {
                    along
                } else {
                    negate_axis(along)
                }
            };
            let adjustment =
                self.wall_band_max_projection_half_ticks(other, direction, band, interior_sides);

            if wall.start == join.point {
                if is_through {
                    update_max(&mut start_extension, adjustment);
                } else {
                    update_max(&mut start_retraction, adjustment);
                }
            } else if wall.end == join.point {
                if is_through {
                    update_max(&mut end_extension, adjustment);
                } else {
                    update_max(&mut end_retraction, adjustment);
                }
            }
        }

        let start = start_retraction.unwrap_or(0) - start_extension.unwrap_or(0);
        let end =
            wall.length.ticks() * 2 + end_extension.unwrap_or(0) - end_retraction.unwrap_or(0);
        if start <= end {
            (start, end)
        } else {
            // Degenerate/extremely short geometry must never invert a derived
            // cuboid or member span. Collapse at the authored midpoint.
            let midpoint = wall.length.ticks();
            (midpoint, midpoint)
        }
    }

    fn primary_corner_through_wall(
        &self,
        join: &WallJoin,
        interior_sides: &BTreeMap<ElementId, bool>,
    ) -> Option<ElementId> {
        let first = self
            .walls
            .iter()
            .find(|candidate| candidate.id == join.first_wall)?;
        let second = self
            .walls
            .iter()
            .find(|candidate| candidate.id == join.second_wall)?;

        if first.level == second.level
            && let (Some(first_plus), Some(second_plus)) = (
                interior_sides.get(&first.id),
                interior_sides.get(&second.id),
            )
        {
            let first_incoming = wall_runs_into_ccw_corner(first, *first_plus, join.point);
            let second_incoming = wall_runs_into_ccw_corner(second, *second_plus, join.point);
            if first_incoming != second_incoming {
                return Some(if first_incoming {
                    first.id.clone()
                } else {
                    second.id.clone()
                });
            }
        }

        // Free/open geometry has no room side from which to infer a loop order.
        // Element ids are canonical and independent of vector/join field order.
        Some(if first.id <= second.id {
            first.id.clone()
        } else {
            second.id.clone()
        })
    }

    fn wall_envelope_thickness(&self, wall: &Wall) -> Length {
        self.system_for(wall)
            .map(ConstructionSystem::total_thickness)
            .unwrap_or_else(|| self.framing_defaults().stud_profile.nominal_depth())
    }

    fn wall_framing_thickness(&self, wall: &Wall) -> Length {
        self.system_for(wall)
            .and_then(ConstructionSystem::framing_layer)
            .map(|layer| layer.thickness)
            .unwrap_or_else(|| self.framing_defaults().stud_profile.nominal_depth())
    }

    fn wall_band_max_projection_half_ticks(
        &self,
        wall: &Wall,
        direction: (i64, i64),
        band: WallPhysicalBand,
        interior_sides: &BTreeMap<ElementId, bool>,
    ) -> i64 {
        let (side0, side1) = match band {
            WallPhysicalBand::Envelope => {
                let half = self.wall_envelope_thickness(wall).ticks();
                (-half, half)
            }
            WallPhysicalBand::Framing => {
                let depth = self.wall_framing_thickness(wall);
                let Some(interior_on_plus) = interior_sides.get(&wall.id) else {
                    let half = depth.ticks();
                    return half;
                };
                let total = self.wall_envelope_thickness(wall).ticks();
                let offset = self
                    .system_for(wall)
                    .and_then(|system| {
                        let mut offset = Length::ZERO;
                        for layer in &system.layers {
                            if layer.function == LayerFunction::Framing {
                                return Some(offset);
                            }
                            offset += layer.thickness;
                        }
                        None
                    })
                    .unwrap_or(Length::ZERO)
                    .ticks();
                let sign = if *interior_on_plus { 1 } else { -1 };
                let interior_face = sign * (total - 2 * offset);
                let exterior_face = sign * (total - 2 * (offset + depth.ticks()));
                (
                    interior_face.min(exterior_face),
                    interior_face.max(exterior_face),
                )
            }
        };
        let side = wall_side_axis(wall);
        let projection_sign = side.0 * direction.0 + side.1 * direction.1;
        if projection_sign == 0 {
            // A malformed parallel `Corner` has no perpendicular face in the
            // endpoint direction. Fall back to a symmetric half-depth.
            return match band {
                WallPhysicalBand::Envelope => self.wall_envelope_thickness(wall).ticks(),
                WallPhysicalBand::Framing => self.wall_framing_thickness(wall).ticks(),
            };
        }
        (side0 * projection_sign).max(side1 * projection_sign)
    }

    /// The visible/takeoff outline of a roof plane after applying its authored
    /// plan-projected eave and rake overhangs. The persisted [`RoofPlane::outline`]
    /// remains the bearing/topology footprint: this derived polygon offsets only
    /// the designated eave and exposed rake edges, while exact same-level shared
    /// ridge/hip/valley edges stay fixed so adjacent planes cannot crack apart.
    ///
    /// The result keeps the authored vertex count/order and returns the authored
    /// outline byte-for-byte when both overhangs are zero. Consumers must project
    /// it through the plane's original [`RoofPlane::frame`], not build a new frame
    /// from this expanded outline.
    pub fn roof_surface_outline(&self, plane: &RoofPlane) -> Vec<Point2> {
        if plane.eave_overhang == Length::ZERO && plane.rake_overhang == Length::ZERO {
            return plane.outline.clone();
        }
        let Some(frame) = plane.frame() else {
            return plane.outline.clone();
        };
        let Some(high_edge) = roof_high_edge_index(plane, &frame) else {
            return plane.outline.clone();
        };
        let edge_count = plane.outline.len();
        let offsets: Vec<Length> = (0..edge_count)
            .map(|index| {
                if index == plane.eave_edge as usize % edge_count {
                    return plane.eave_overhang;
                }
                if index == high_edge
                    || self.roof_planes.iter().any(|other| {
                        other.id != plane.id
                            && other.level == plane.level
                            && roof_planes_share_edge(plane, index, other)
                    })
                {
                    Length::ZERO
                } else {
                    plane.rake_overhang
                }
            })
            .collect();
        offset_polygon_edges(&plane.outline, &offsets).unwrap_or_else(|| plane.outline.clone())
    }

    /// Hole-aware triangulation of the occupied roof assembly surface. The outer
    /// ring uses [`Self::roof_surface_outline`] so overhangs stay shared across
    /// all consumers; each modeled roof opening becomes a plane-local rectangle
    /// transformed through the original bearing frame.
    pub fn roof_surface_triangulation(&self, plane: &RoofPlane) -> Option<PolygonTriangulation> {
        let frame = plane.frame()?;
        let (origin_x, origin_y) = frame.eave_origin();
        let (along_x, along_y) = frame.eave_axis();
        let (up_x, up_y) = frame.up_slope();
        let point = |along: f64, up: f64| {
            Point2::new(
                Length::from_inches(origin_x + along_x * along + up_x * up),
                Length::from_inches(origin_y + along_y * along + up_y * up),
            )
        };
        let mut openings: Vec<_> = plane.openings.iter().collect();
        openings.sort_by(|left, right| left.id.cmp(&right.id));
        let holes: Vec<Vec<Point2>> = openings
            .iter()
            .map(|opening| {
                let cx = opening.center.x.inches();
                let cy = opening.center.y.inches();
                let half_width = opening.width.inches() / 2.0;
                let half_height = opening.height.inches() / 2.0;
                vec![
                    point(cx - half_width, cy - half_height),
                    point(cx + half_width, cy - half_height),
                    point(cx + half_width, cy + half_height),
                    point(cx - half_width, cy + half_height),
                ]
            })
            .collect();
        if holes
            .iter()
            .flatten()
            .any(|point| !crate::topology::point_in_polygon(*point, &plane.outline))
        {
            return None;
        }
        for (index, opening) in openings.iter().enumerate() {
            for other in &openings[index + 1..] {
                let overlaps_along =
                    (opening.center.x - other.center.x).abs() * 2 < opening.width + other.width;
                let overlaps_up =
                    (opening.center.y - other.center.y).abs() * 2 < opening.height + other.height;
                if overlaps_along && overlaps_up {
                    return None;
                }
            }
        }
        triangulate_polygon_with_holes(&self.roof_surface_outline(plane), &holes)
    }

    /// Exact-edge connected component containing `roof_plane`. Connected fields
    /// form one watertight roof assembly for overhang compatibility and UI edits.
    pub fn connected_roof_plane_ids(&self, roof_plane: &ElementId) -> BTreeSet<ElementId> {
        let Some(seed) = self
            .roof_planes
            .iter()
            .find(|plane| plane.id == *roof_plane)
        else {
            return BTreeSet::new();
        };
        let mut connected = BTreeSet::from([seed.id.clone()]);
        let mut frontier = vec![seed.id.clone()];
        while let Some(current_id) = frontier.pop() {
            let current = self
                .roof_planes
                .iter()
                .find(|plane| plane.id == current_id)
                .expect("frontier ids originate in roof_planes");
            for candidate in self
                .roof_planes
                .iter()
                .filter(|candidate| candidate.level == current.level)
            {
                if connected.contains(&candidate.id)
                    || !(0..current.outline.len())
                        .any(|edge| roof_planes_share_edge(current, edge, candidate))
                {
                    continue;
                }
                connected.insert(candidate.id.clone());
                frontier.push(candidate.id.clone());
            }
        }
        connected
    }

    /// Derive every simple matched gable profile once, keyed by the hosting wall.
    /// Only level-scoped exterior walls participate; interior partitions and free
    /// walls are omitted by [`crate::topology::wall_interior_sides_on_level`].
    pub fn gable_wall_profiles(&self) -> BTreeMap<ElementId, GableWallProfile> {
        let mut profiles = BTreeMap::new();
        let levels: BTreeSet<ElementId> =
            self.walls.iter().map(|wall| wall.level.clone()).collect();
        for level in levels {
            let exterior = crate::topology::wall_interior_sides_on_level(self, &level);
            for wall in self
                .walls
                .iter()
                .filter(|wall| wall.level == level && exterior.contains_key(&wall.id))
            {
                if let Some(profile) = derive_gable_wall_profile(self, wall) {
                    profiles.insert(wall.id.clone(), profile);
                }
            }
        }
        profiles
    }

    /// Per-wall convenience form of [`Self::gable_wall_profiles`]. Hot render
    /// paths should use the batched form so each level's wall graph is solved once.
    pub fn gable_wall_profile(&self, wall: &Wall) -> Option<GableWallProfile> {
        let exterior = crate::topology::wall_interior_sides_on_level(self, &wall.level);
        exterior
            .contains_key(&wall.id)
            .then(|| derive_gable_wall_profile(self, wall))
            .flatten()
    }

    /// Resolve a material by id from the project library.
    pub fn material(&self, id: &ElementId) -> Option<&Material> {
        self.materials.iter().find(|material| material.id == *id)
    }

    /// Whether a roof plane is a **cathedral** condition: no authored ceiling on the
    /// plane's level encloses its footprint, so the room below sees the roof
    /// assembly's conditioned-side finish on the underside rather than a ceiling.
    /// A ceiling of any pitch (flat or sloped) that covers the footprint disqualifies
    /// it — distinct from the solver's structural thrust-tie check, which counts only
    /// a *flat* ceiling at the plate. Coverage uses the same centroid-in-region
    /// containment the tie check uses; a degenerate (un-frameable) plane is reported
    /// as not-cathedral since it emits no underside.
    ///
    /// Resolving a `Room`-attached ceiling rebuilds the wall graph, so classifying
    /// *many* planes (the render/mesher hot path, run every repaint) should use
    /// [`Self::roof_cathedral_flags`] — one graph pass for all planes — not this
    /// per-plane form in a loop.
    pub fn roof_plane_is_cathedral(&self, plane: &RoofPlane) -> bool {
        plane_is_cathedral(plane, &self.resolve_ceiling_outlines())
    }

    /// Classify every roof plane as cathedral (no covering ceiling), aligned to
    /// `self.roof_planes`. The batched form of [`Self::roof_plane_is_cathedral`]:
    /// each ceiling's outline (and any wall-graph rebuild for a `Room` region) is
    /// resolved **once**, then every plane centroid is tested against the cached
    /// set — so the per-plane scan does not re-derive the graph per plane.
    pub fn roof_cathedral_flags(&self) -> Vec<bool> {
        let ceilings = self.resolve_ceiling_outlines();
        self.roof_planes
            .iter()
            .map(|plane| plane_is_cathedral(plane, &ceilings))
            .collect()
    }

    /// Each ceiling's level paired with its resolved plan outline (`Room` regions
    /// through the wall graph), skipping any that fail to resolve (unknown room or
    /// an open mid-edit loop). Computed once and reused across all roof planes.
    fn resolve_ceiling_outlines(&self) -> Vec<(ElementId, Vec<Point2>)> {
        self.ceilings
            .iter()
            .filter_map(|ceiling| {
                self.surface_region_outline(&ceiling.region)
                    .map(|outline| (ceiling.level.clone(), outline))
            })
            .collect()
    }

    /// Resolve a [`SurfaceRegion`] to its closed plan outline: a `Polygon` is its own
    /// outline; a `Room` is resolved through the wall graph (mirroring the solver and
    /// the renderers). `None` for an unknown room or an open (mid-edit) loop.
    fn surface_region_outline(&self, region: &SurfaceRegion) -> Option<Vec<Point2>> {
        match region {
            SurfaceRegion::Polygon(points) => Some(points.clone()),
            SurfaceRegion::Room(room_id) => {
                let room = self.rooms.iter().find(|room| room.id == *room_id)?;
                crate::topology::room_boundary_on_level(self, &room.level, room.seed)
                    .map(|boundary| boundary.vertices)
            }
        }
    }

    /// The seeded material catalog and construction systems for a new project.
    /// Deterministic (id-sorted on output); shared by `new`, the `demo_*`
    /// constructors, and the app's `new_project`.
    pub fn starter_library() -> (Vec<Material>, Vec<ConstructionSystem>) {
        let library = crate::library::starter_library();
        (library.materials, library.systems)
    }

    pub fn apply_driving_dimensions(&mut self) -> bool {
        let mut changed = false;
        for wall in &mut self.walls {
            changed |= wall.apply_driving_dimensions();
        }
        changed
    }

    /// Remove a wall and every join that references it. The wall's nested
    /// openings and dimensions are removed along with it. Returns whether a
    /// wall was actually removed.
    pub fn remove_wall(&mut self, wall: &ElementId) -> bool {
        let before = self.walls.len();
        self.walls.retain(|candidate| candidate.id != *wall);
        if self.walls.len() == before {
            return false;
        }

        self.wall_joins
            .retain(|join| join.first_wall != *wall && join.second_wall != *wall);
        true
    }

    /// Extend one existing wall when a newly drawn segment is its exact,
    /// non-overlapping collinear continuation. This keeps draw gestures as
    /// authored room/layout intent instead of turning every click into a
    /// physical framing break. The caller remains responsible for reconciling
    /// joins after the mutation.
    ///
    /// The continuation is deliberately conservative: it must identify exactly
    /// one wall on `level`, and the wall must still validate after extension.
    /// That final guard prevents a draw gesture from silently overriding a
    /// driving dimension. When the local start moves, nested opening and braced
    /// panel offsets shift by the added length so their world positions stay
    /// fixed.
    pub fn extend_collinear_wall(
        &mut self,
        level: &ElementId,
        segment_start: Point2,
        segment_end: Point2,
    ) -> Option<ElementId> {
        if segment_start == segment_end
            || (segment_start.x != segment_end.x && segment_start.y != segment_end.y)
        {
            return None;
        }

        let candidates = self
            .walls
            .iter()
            .enumerate()
            .filter(|(_, wall)| wall.level == *level)
            .flat_map(|(index, wall)| {
                [(segment_start, segment_end), (segment_end, segment_start)]
                    .into_iter()
                    .filter_map(move |(shared, new_endpoint)| {
                        wall_continuation_at_endpoint(wall, shared, new_endpoint).map(
                            |(extend_start, extension)| {
                                (index, extend_start, new_endpoint, extension)
                            },
                        )
                    })
            })
            .collect::<Vec<_>>();

        let [(index, extend_start, new_endpoint, extension)] = candidates.as_slice() else {
            return None;
        };
        if self.walls.iter().enumerate().any(|(other_index, wall)| {
            other_index != *index
                && wall.level == *level
                && wall_overlaps_segment_interior(wall, segment_start, segment_end)
        }) {
            return None;
        }

        let mut extended = self.walls[*index].clone();
        if *extend_start {
            extended.start = *new_endpoint;
            for opening in &mut extended.openings {
                opening.center += *extension;
            }
            for panel in &mut extended.bracing {
                panel.offset += *extension;
            }
        } else {
            extended.end = *new_endpoint;
        }
        extended.length += *extension;

        if extended.validate().is_err() {
            return None;
        }

        let id = extended.id.clone();
        self.walls[*index] = extended;
        Some(id)
    }

    /// Move the `which_end` endpoint of `wall` to `new_point`, dragging along every
    /// other wall endpoint that coincides with the old point so a shared corner
    /// stays connected ("move the joint"). Each moved wall's `length` is resynced
    /// from its new placement. Returns the ids of all walls whose geometry changed
    /// (empty when `wall` is unknown or the point is unchanged).
    ///
    /// This is the honest geometric primitive: it does not enforce the
    /// axis-aligned invariant — the caller (the snap-driven editor) chooses a
    /// `new_point` that keeps every affected wall orthogonal.
    pub fn move_wall_endpoint(
        &mut self,
        wall: &ElementId,
        which_end: WallEnd,
        new_point: Point2,
    ) -> Vec<ElementId> {
        let Some(old_point) = self
            .walls
            .iter()
            .find(|candidate| candidate.id == *wall)
            .map(|candidate| match which_end {
                WallEnd::Start => candidate.start,
                WallEnd::End => candidate.end,
            })
        else {
            return Vec::new();
        };
        if old_point == new_point {
            return Vec::new();
        }

        self.move_coincident_endpoints(old_point, new_point)
    }

    /// Translate a whole wall by `(dx, dy)`, dragging every wall endpoint that
    /// coincides with either of its ends so neighbours stretch to follow ("move
    /// the joint"). Returns the ids of all walls whose geometry changed.
    ///
    /// Like [`move_wall_endpoint`](Self::move_wall_endpoint) this is the honest
    /// primitive — the caller keeps the result orthogonal.
    pub fn translate_wall(&mut self, wall: &ElementId, dx: Length, dy: Length) -> Vec<ElementId> {
        let Some((start, end)) = self
            .walls
            .iter()
            .find(|candidate| candidate.id == *wall)
            .map(|candidate| (candidate.start, candidate.end))
        else {
            return Vec::new();
        };
        if dx == Length::ZERO && dy == Length::ZERO {
            return Vec::new();
        }

        let mut affected =
            self.move_coincident_endpoints(start, Point2::new(start.x + dx, start.y + dy));
        let new_end = Point2::new(end.x + dx, end.y + dy);
        for id in self.move_coincident_endpoints(end, new_end) {
            if !affected.contains(&id) {
                affected.push(id);
            }
        }
        affected
    }

    /// Move every wall endpoint at `old_point` to `new_point`, re-syncing each
    /// moved wall's length. Shared by endpoint and whole-wall moves.
    fn move_coincident_endpoints(
        &mut self,
        old_point: Point2,
        new_point: Point2,
    ) -> Vec<ElementId> {
        if old_point == new_point {
            return Vec::new();
        }
        let mut affected = Vec::new();
        for candidate in &mut self.walls {
            let mut changed = false;
            if candidate.start == old_point {
                candidate.start = new_point;
                changed = true;
            }
            if candidate.end == old_point {
                candidate.end = new_point;
                changed = true;
            }
            if changed {
                if let Some(length) = candidate.placement_length() {
                    candidate.length = length;
                }
                affected.push(candidate.id.clone());
            }
        }
        affected
    }

    /// Rebuild the wall-join set from current wall geometry, preserving the id and
    /// name of any join whose two walls still meet (across point moves and even
    /// kind changes). Run after a structural edit — drawing, deleting, or moving a
    /// wall — so joins stay consistent with the walls without churning ids.
    pub fn reconcile_joins(&mut self) {
        let desired = derive_wall_joins(&self.walls);
        let mut rebuilt: Vec<WallJoin> = Vec::with_capacity(desired.len());
        let mut taken = vec![false; self.wall_joins.len()];

        for join in &desired {
            // Reuse the existing join between the same unordered wall pair, if any.
            let matched = self
                .wall_joins
                .iter()
                .enumerate()
                .find(|(index, existing)| !taken[*index] && same_wall_pair(existing, join))
                .map(|(index, _)| index);

            let (id, name) = match matched {
                Some(index) => {
                    taken[index] = true;
                    (
                        self.wall_joins[index].id.0.clone(),
                        self.wall_joins[index].name.clone(),
                    )
                }
                None => (
                    next_reconciled_join_id(&self.wall_joins, &rebuilt),
                    default_join_name(join),
                ),
            };

            rebuilt.push(WallJoin::new(
                id,
                name,
                join.kind,
                join.first.0.clone(),
                join.second.0.clone(),
                join.point,
            ));
        }

        self.wall_joins = rebuilt;
    }
}

/// A join the geometry implies, before an id/name is assigned.
struct DesiredJoin {
    kind: WallJoinKind,
    /// For a `Tee` this is the through wall; for `Cross` the stable-ordered first.
    first: ElementId,
    second: ElementId,
    point: Point2,
}

/// Every join implied by same-level walls' current geometry: a `Corner` where two
/// walls share an endpoint, a `Tee` where one wall's endpoint lands on another's
/// interior, a `Cross` where two walls cross interior-to-interior.
fn derive_wall_joins(walls: &[Wall]) -> Vec<DesiredJoin> {
    let mut joins = Vec::new();
    for i in 0..walls.len() {
        for j in (i + 1)..walls.len() {
            if walls[i].start == walls[i].end || walls[j].start == walls[j].end {
                continue;
            }
            if let Some(join) = relate_walls(&walls[i], &walls[j]) {
                joins.push(join);
            }
        }
    }
    joins
}

/// Whether `new_endpoint` continues `wall` straight through `shared`, returning
/// which local end moves and the added length. The strict negative dot product
/// distinguishes a continuation from a coincident/overlapping stroke.
fn wall_continuation_at_endpoint(
    wall: &Wall,
    shared: Point2,
    new_endpoint: Point2,
) -> Option<(bool, Length)> {
    let (other, extend_start) = if wall.start == shared {
        (wall.end, true)
    } else if wall.end == shared {
        (wall.start, false)
    } else {
        return None;
    };

    let old_dx = (other.x - shared.x).ticks() as i128;
    let old_dy = (other.y - shared.y).ticks() as i128;
    let new_dx = (new_endpoint.x - shared.x).ticks() as i128;
    let new_dy = (new_endpoint.y - shared.y).ticks() as i128;
    if old_dx * new_dy - old_dy * new_dx != 0 || old_dx * new_dx + old_dy * new_dy >= 0 {
        return None;
    }

    let extension = if new_dx != 0 {
        (new_endpoint.x - shared.x).abs()
    } else {
        (new_endpoint.y - shared.y).abs()
    };
    (extension > Length::ZERO).then_some((extend_start, extension))
}

/// Whether an axis-aligned wall overlaps the positive-length interior of an
/// axis-aligned segment. Endpoint-only contact is allowed because reconciliation
/// can represent that junction; coincident framing runs are not.
fn wall_overlaps_segment_interior(wall: &Wall, start: Point2, end: Point2) -> bool {
    if start.y == end.y && wall.start.y == wall.end.y && wall.start.y == start.y {
        let wall_min = wall.start.x.min(wall.end.x);
        let wall_max = wall.start.x.max(wall.end.x);
        let segment_min = start.x.min(end.x);
        let segment_max = start.x.max(end.x);
        wall_min.max(segment_min) < wall_max.min(segment_max)
    } else if start.x == end.x && wall.start.x == wall.end.x && wall.start.x == start.x {
        let wall_min = wall.start.y.min(wall.end.y);
        let wall_max = wall.start.y.max(wall.end.y);
        let segment_min = start.y.min(end.y);
        let segment_max = start.y.max(end.y);
        wall_min.max(segment_min) < wall_max.min(segment_max)
    } else {
        false
    }
}

/// The single join relationship (if any) between two distinct walls, in priority
/// order: shared endpoint → `Corner`; endpoint-on-interior → `Tee` (through wall
/// first); interior crossing → `Cross`.
fn relate_walls(a: &Wall, b: &Wall) -> Option<DesiredJoin> {
    if a.level != b.level {
        return None;
    }
    for point in [a.start, a.end] {
        if b.has_endpoint(point) {
            return Some(DesiredJoin {
                kind: WallJoinKind::Corner,
                first: a.id.clone(),
                second: b.id.clone(),
                point,
            });
        }
    }
    for point in [a.start, a.end] {
        if b.point_on_interior(point) {
            return Some(DesiredJoin {
                kind: WallJoinKind::Tee,
                first: b.id.clone(),
                second: a.id.clone(),
                point,
            });
        }
    }
    for point in [b.start, b.end] {
        if a.point_on_interior(point) {
            return Some(DesiredJoin {
                kind: WallJoinKind::Tee,
                first: a.id.clone(),
                second: b.id.clone(),
                point,
            });
        }
    }
    if let Some(point) = ortho_crossing(a, b)
        && a.point_on_interior(point)
        && b.point_on_interior(point)
    {
        return Some(DesiredJoin {
            kind: WallJoinKind::Cross,
            first: a.id.clone(),
            second: b.id.clone(),
            point,
        });
    }
    None
}

/// The crossing point of one horizontal and one vertical axis-aligned wall, or
/// `None` when the walls are parallel (so cannot cross at a single point).
fn ortho_crossing(a: &Wall, b: &Wall) -> Option<Point2> {
    let a_horizontal = a.start.y == a.end.y;
    let a_vertical = a.start.x == a.end.x;
    let b_horizontal = b.start.y == b.end.y;
    let b_vertical = b.start.x == b.end.x;
    if a_horizontal && b_vertical {
        Some(Point2::new(b.start.x, a.start.y))
    } else if a_vertical && b_horizontal {
        Some(Point2::new(a.start.x, b.start.y))
    } else {
        None
    }
}

/// Whether an existing join connects the same unordered pair of walls as `desired`.
fn same_wall_pair(existing: &WallJoin, desired: &DesiredJoin) -> bool {
    (existing.first_wall == desired.first && existing.second_wall == desired.second)
        || (existing.first_wall == desired.second && existing.second_wall == desired.first)
}

/// Default name for a freshly-derived join, mirroring the draw tool's style.
fn default_join_name(join: &DesiredJoin) -> String {
    let kind = match join.kind {
        WallJoinKind::Corner => "corner",
        WallJoinKind::EndToEnd => "end-to-end",
        WallJoinKind::Tee => "tee",
        WallJoinKind::Cross => "cross",
    };
    format!("{} \u{2013} {} {}", join.first.0, join.second.0, kind)
}

/// The next free `join-N` id, unique against the kept joins and any already
/// staged in this reconcile pass.
fn next_reconciled_join_id(existing: &[WallJoin], staged: &[WallJoin]) -> String {
    let mut index = existing.len() + staged.len() + 1;
    loop {
        let id = format!("join-{index}");
        let collides = existing.iter().chain(staged).any(|join| join.id.0 == id);
        if !collides {
            return id;
        }
        index += 1;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CornerLapPass {
    Primary,
    Counter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WallPhysicalBand {
    Envelope,
    Framing,
}

fn wall_along_axis(wall: &Wall) -> (i64, i64) {
    (
        (wall.end.x - wall.start.x).ticks().signum(),
        (wall.end.y - wall.start.y).ticks().signum(),
    )
}

fn wall_side_axis(wall: &Wall) -> (i64, i64) {
    let along = wall_along_axis(wall);
    (-along.1, along.0)
}

fn negate_axis((x, y): (i64, i64)) -> (i64, i64) {
    (-x, -y)
}

fn update_max(slot: &mut Option<i64>, candidate: i64) {
    *slot = Some(slot.map_or(candidate, |current| current.max(candidate)));
}

/// Whether the wall's counterclockwise boundary direction arrives at `point`.
/// When the room lies on the authored plus/left side, authored start -> end is
/// already counterclockwise; otherwise the boundary direction is reversed.
fn wall_runs_into_ccw_corner(wall: &Wall, interior_on_plus_side: bool, point: Point2) -> bool {
    if interior_on_plus_side {
        wall.end == point
    } else {
        wall.start == point
    }
}

fn half_ticks_to_inches(half_ticks: i64) -> f64 {
    half_ticks as f64 / (Length::TICKS_PER_INCH * 2) as f64
}

fn half_tick_span_to_lengths((start, end): (i64, i64)) -> (Length, Length) {
    // Framing members remain integer-tick data. Standard dimensional lumber
    // depths are even tick counts. For a rare half-tick face, round the start
    // inward (ceil) and the end inward (floor): members never overlap and the
    // rule is invariant when authored wall direction swaps start/end.
    let ceil_half_ticks = |value: i64| value.div_euclid(2) + i64::from(value.rem_euclid(2) != 0);
    (
        Length::from_ticks(ceil_half_ticks(start)),
        Length::from_ticks(end.div_euclid(2)),
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Level {
    pub id: ElementId,
    pub name: String,
    pub elevation: Length,
    /// Height from this level's `elevation` to its top plane. The top plane
    /// (`elevation + height`) is the bearing/springing line for roofs and the
    /// hang reference for ceilings. Defaults to zero (top datum not yet
    /// authored); a zero height is omitted so existing fixtures stay byte-stable.
    #[serde(default, skip_serializing_if = "length_is_zero")]
    pub height: Length,
}

impl Level {
    pub fn new(id: impl Into<String>, name: impl Into<String>, elevation: Length) -> Self {
        Self {
            id: ElementId::new(id),
            name: name.into(),
            elevation,
            height: Length::ZERO,
        }
    }

    /// Set the level's height; the top plane is `elevation + height`.
    pub fn with_height(mut self, height: Length) -> Self {
        self.height = height;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum BoardProfile {
    TwoByFour,
    TwoBySix,
    TwoByEight,
    TwoByTen,
    TwoByTwelve,
}

impl BoardProfile {
    pub const fn label(self) -> &'static str {
        match self {
            Self::TwoByFour => "2x4",
            Self::TwoBySix => "2x6",
            Self::TwoByEight => "2x8",
            Self::TwoByTen => "2x10",
            Self::TwoByTwelve => "2x12",
        }
    }

    pub const fn thickness(self) -> Length {
        Length::from_ticks(24)
    }

    pub const fn nominal_depth(self) -> Length {
        match self {
            Self::TwoByFour => Length::from_whole_inches(4),
            Self::TwoBySix => Length::from_whole_inches(6),
            Self::TwoByEight => Length::from_whole_inches(8),
            Self::TwoByTen => Length::from_whole_inches(10),
            Self::TwoByTwelve => Length::from_whole_inches(12),
        }
    }
}

/// Whether `plane` is a cathedral against pre-resolved ceiling outlines (each
/// paired with its level): no same-level ceiling encloses the plane's footprint
/// centroid. A degenerate (un-frameable) plane is not-cathedral — it emits no
/// underside. Shared by the per-plane and batched [`BuildingModel`] entry points.
fn plane_is_cathedral(plane: &RoofPlane, ceiling_outlines: &[(ElementId, Vec<Point2>)]) -> bool {
    let Some(sample) = polygon_vertex_centroid(&plane.outline) else {
        return false;
    };
    !ceiling_outlines.iter().any(|(level, outline)| {
        *level == plane.level && crate::topology::point_in_polygon(sample, outline)
    })
}

/// The vertex centroid of a plan polygon — an interior sample point for a convex
/// footprint (a gable half, a hip trapezoid/triangle). Integer (i128) accumulation
/// keeps it exact; `None` for an empty outline. Mirrors the solver's tie-check
/// sample so cathedral detection and thrust-tie detection agree on the point.
fn polygon_vertex_centroid(points: &[Point2]) -> Option<Point2> {
    if points.is_empty() {
        return None;
    }
    let (mut sx, mut sy) = (0i128, 0i128);
    for point in points {
        sx += point.x.ticks() as i128;
        sy += point.y.ticks() as i128;
    }
    let n = points.len() as i128;
    Some(Point2::new(
        Length::from_ticks((sx / n) as i64),
        Length::from_ticks((sy / n) as i64),
    ))
}

/// Whether a wall faces the weather (drives sheathing intent and, later,
/// generated sheathing zones).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum WallExposure {
    #[default]
    Exterior,
    Interior,
}

impl WallExposure {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Exterior => "Exterior",
            Self::Interior => "Interior",
        }
    }
}

/// Which face of a roof/ceiling/floor assembly a finish lookup resolves — see
/// [`ConstructionSystem::surface_finish_material`]. Not persisted; a transient
/// render/UI input only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssemblyFace {
    /// The face a viewer sees by default: a roof's weather (outermost) face, a
    /// ceiling/floor's conditioned-side (innermost) finish.
    Finished,
    /// The conditioned-side (innermost) finish — a roof's cathedral underside,
    /// where a room with no ceiling sees the assembly's interior finish.
    Underside,
}

/// Authored sheathing intent for a wall. Quantities are not yet generated; this
/// records the design decision for the BOM and future sheathing zones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Sheathing {
    None,
    #[default]
    Osb716,
    Plywood12,
    Plywood58,
}

impl Sheathing {
    pub const fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Osb716 => "7/16\" OSB",
            Self::Plywood12 => "1/2\" Plywood",
            Self::Plywood58 => "5/8\" Plywood",
        }
    }
}

/// The structural class of a construction system. A closed enum: rendering, the
/// BOM, and validation reason about each kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum SystemKind {
    Wall,
    Floor,
    Roof,
    Ceiling,
}

impl SystemKind {
    pub const ALL: [Self; 4] = [Self::Wall, Self::Floor, Self::Roof, Self::Ceiling];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Wall => "Wall",
            Self::Floor => "Floor",
            Self::Roof => "Roof",
            Self::Ceiling => "Ceiling",
        }
    }
}

/// The structural role a layer plays in an assembly. A closed enum (the app's BOM
/// and rendering reason about each role); material substance stays open data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum LayerFunction {
    InteriorFinish,
    Framing,
    ContinuousInsulation,
    Sheathing,
    WeatherBarrier,
    AirGap,
    Cladding,
    Masonry,
    Structure,
    /// The weather face of a roof assembly (shingles, membrane, metal).
    Roofing,
    /// A water-resistive membrane beneath the roofing.
    Underlayment,
    /// The finished underside of a ceiling assembly.
    CeilingFinish,
    Other,
}

impl LayerFunction {
    pub const ALL: [Self; 13] = [
        Self::InteriorFinish,
        Self::Framing,
        Self::ContinuousInsulation,
        Self::Sheathing,
        Self::WeatherBarrier,
        Self::AirGap,
        Self::Cladding,
        Self::Masonry,
        Self::Structure,
        Self::Roofing,
        Self::Underlayment,
        Self::CeilingFinish,
        Self::Other,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::InteriorFinish => "Interior finish",
            Self::Framing => "Framing",
            Self::ContinuousInsulation => "Continuous insulation",
            Self::Sheathing => "Sheathing",
            Self::WeatherBarrier => "Weather barrier",
            Self::AirGap => "Air gap",
            Self::Cladding => "Cladding",
            Self::Masonry => "Masonry",
            Self::Structure => "Structure",
            Self::Roofing => "Roofing",
            Self::Underlayment => "Underlayment",
            Self::CeilingFinish => "Ceiling finish",
            Self::Other => "Other",
        }
    }
}

/// How the framing members in a framing layer are laid out across the cavity.
/// `Staggered`/`Double` are authored now but generation is deferred.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub enum FramingPattern {
    #[default]
    Single,
    Staggered,
    Double,
}

impl FramingPattern {
    pub const ALL: [Self; 3] = [Self::Single, Self::Staggered, Self::Double];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Single => "Single",
            Self::Staggered => "Staggered",
            Self::Double => "Double",
        }
    }
}

/// The framing detail of a `Framing` layer: member size, spacing, pattern, and an
/// optional between-studs cavity material (adds no extra through-wall depth).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FramingSpec {
    pub member: BoardProfile,
    pub spacing: Length,
    #[serde(default)]
    pub pattern: FramingPattern,
    /// Which family of member this framing layer produces (studs vs. rafters vs.
    /// joists) — authored metadata. The v1 solver selects the generator and the
    /// concrete `MemberKind` from the framed object (`Wall`/`RoofPlane`/`Ceiling`/
    /// `FloorDeck`), so it does not yet branch on `member_family`; the tag records
    /// the system's framing method for later family-based dispatch (e.g. trusses).
    /// Defaults to `Stud`; the default is omitted so existing wall systems (and the
    /// starter library) stay byte-stable and their content hashes are unchanged.
    #[serde(default, skip_serializing_if = "member_family_is_default")]
    pub member_family: MemberFamily,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cavity_material: Option<ElementId>,
}

/// One material layer of a construction system. `framing` is present iff
/// `function == Framing`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConstructionLayer {
    pub function: LayerFunction,
    pub material: ElementId,
    pub thickness: Length,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub framing: Option<FramingSpec>,
}

impl ConstructionLayer {
    pub fn new(function: LayerFunction, material: impl Into<String>, thickness: Length) -> Self {
        Self {
            function,
            material: ElementId::new(material),
            thickness,
            framing: None,
        }
    }

    pub fn with_framing(mut self, framing: FramingSpec) -> Self {
        self.framing = Some(framing);
        self
    }
}

/// A named, reusable construction system: an ordered stack of material layers
/// across an element's thickness. Applied to elements by reference. Layer order
/// is SEMANTIC (interior -> exterior) and is never sorted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConstructionSystem {
    pub id: ElementId,
    pub name: String,
    pub kind: SystemKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Provenance>,
    pub layers: Vec<ConstructionLayer>,
}

impl ConstructionSystem {
    /// The single `Framing` layer, if any.
    pub fn framing_layer(&self) -> Option<&ConstructionLayer> {
        self.layers
            .iter()
            .find(|layer| layer.function == LayerFunction::Framing)
    }

    /// Total through-wall thickness: the sum of every layer's thickness. Cavity
    /// insulation (inside the framing band) contributes no extra depth.
    pub fn total_thickness(&self) -> Length {
        self.layers
            .iter()
            .fold(Length::ZERO, |total, layer| total + layer.thickness)
    }

    /// Derived exposure: `Exterior` if any layer is a weather/outboard envelope
    /// role, else `Interior`. Re-scoped per [`SystemKind`] — a wall's weather
    /// face is its barrier/cladding/masonry/continuous-insulation, a roof's is
    /// its roofing/underlayment; floors and ceilings have no weather face in v1.
    pub fn exposure(&self) -> WallExposure {
        let weather_facing = |function: LayerFunction| match self.kind {
            SystemKind::Wall => matches!(
                function,
                LayerFunction::WeatherBarrier
                    | LayerFunction::Cladding
                    | LayerFunction::Masonry
                    | LayerFunction::ContinuousInsulation
            ),
            SystemKind::Roof => matches!(
                function,
                LayerFunction::Roofing
                    | LayerFunction::Underlayment
                    | LayerFunction::WeatherBarrier
                    | LayerFunction::ContinuousInsulation
            ),
            SystemKind::Floor | SystemKind::Ceiling => false,
        };
        if self
            .layers
            .iter()
            .any(|layer| weather_facing(layer.function))
        {
            WallExposure::Exterior
        } else {
            WallExposure::Interior
        }
    }

    /// The material of a finished face on a roof/ceiling/floor surface. The single
    /// definition of this rule so the path-traced render and the 3-D viewport pick
    /// the same face and cannot drift; each caller applies its own fallback (a stock
    /// palette entry / neutral color) when this is `None`. Returns `None` for a
    /// `Wall` (walls pick their face by exposure) or an empty system.
    ///
    /// [`AssemblyFace::Finished`] is the face a viewer sees by default: a roof's
    /// weather face (outermost), a ceiling/floor's conditioned-side finish
    /// (innermost). [`AssemblyFace::Underside`] is the conditioned-side (innermost)
    /// finish — a roof's **cathedral underside**, where a room with no ceiling sees
    /// the assembly's interior finish rather than the weather face; for a
    /// ceiling/floor (already viewed from the conditioned side) it resolves the same
    /// innermost finish as `Finished`.
    pub fn surface_finish_material(&self, face: AssemblyFace) -> Option<&ElementId> {
        // Weather-side (outermost) finish of a roof.
        let roof_weather = || {
            self.layers
                .iter()
                .rev()
                .find(|layer| layer.function == LayerFunction::Roofing)
                .or_else(|| {
                    self.layers.iter().rev().find(|layer| {
                        matches!(
                            layer.function,
                            LayerFunction::WeatherBarrier | LayerFunction::Sheathing
                        )
                    })
                })
                .or_else(|| self.layers.last())
        };
        // Conditioned-side (innermost) drywall/finish — a ceiling's underside and a
        // roof's cathedral underside resolve identically.
        let conditioned_finish = || {
            self.layers
                .iter()
                .find(|layer| {
                    matches!(
                        layer.function,
                        LayerFunction::CeilingFinish | LayerFunction::InteriorFinish
                    )
                })
                .or_else(|| self.layers.first())
        };
        // A floor's walked-on top (a bare subfloor falls back to its sheathing).
        let floor_finish = || {
            self.layers
                .iter()
                .find(|layer| {
                    matches!(
                        layer.function,
                        LayerFunction::InteriorFinish | LayerFunction::Sheathing
                    )
                })
                .or_else(|| self.layers.first())
        };
        let layer = match (self.kind, face) {
            (SystemKind::Roof, AssemblyFace::Finished) => roof_weather(),
            (SystemKind::Roof, AssemblyFace::Underside) => conditioned_finish(),
            (SystemKind::Ceiling, _) => conditioned_finish(),
            (SystemKind::Floor, _) => floor_finish(),
            (SystemKind::Wall, _) => None,
        };
        layer.map(|layer| &layer.material)
    }

    /// Clear-wall R-value in milli-R (R × 1000), exact integer math: the sum over
    /// layers of each layer material's [`Material::r_value_milli`] across the
    /// layer thickness (no inch rounding, so a 5/8" layer counts as 5/8"). The
    /// framing layer additionally contributes its cavity material's R over the
    /// framing depth. This is a clear-wall approximation — it ignores the
    /// framing-factor (parallel-path) derate, which is deferred.
    pub fn r_value_milli(&self, materials: &[Material]) -> i64 {
        let lookup = |id: &ElementId| materials.iter().find(|material| material.id == *id);
        let mut total = 0i64;
        for layer in &self.layers {
            if let Some(material) = lookup(&layer.material) {
                total += material.r_value_milli(layer.thickness);
            }
            if let Some(framing) = &layer.framing
                && let Some(cavity) = &framing.cavity_material
                && let Some(material) = lookup(cavity)
            {
                total += material.r_value_milli(layer.thickness);
            }
        }
        total
    }

    pub(crate) fn validate(
        &self,
        materials: &BTreeMap<ElementId, &Material>,
        ids: &mut BTreeSet<ElementId>,
    ) -> Result<(), ModelError> {
        validate_element_id(&self.id)?;
        insert_unique_id(ids, &self.id)?;

        if self.layers.is_empty() {
            return Err(ModelError::SystemHasNoLayers {
                system: self.id.clone(),
            });
        }

        let mut framing_layers = 0;
        for layer in &self.layers {
            if layer.thickness <= Length::ZERO {
                return Err(ModelError::InvalidLayerThickness {
                    system: self.id.clone(),
                });
            }

            let is_framing = layer.function == LayerFunction::Framing;
            if is_framing != layer.framing.is_some() {
                return Err(ModelError::LayerFramingMismatch {
                    system: self.id.clone(),
                });
            }

            if !materials.contains_key(&layer.material) {
                return Err(ModelError::LayerReferencesUnknownMaterial {
                    system: self.id.clone(),
                    material: layer.material.clone(),
                });
            }

            if let Some(framing) = &layer.framing {
                framing_layers += 1;
                if framing.spacing <= Length::ZERO {
                    return Err(ModelError::InvalidFramingSpacing {
                        system: self.id.clone(),
                    });
                }
                if let Some(cavity) = &framing.cavity_material
                    && !materials.contains_key(cavity)
                {
                    return Err(ModelError::LayerReferencesUnknownMaterial {
                        system: self.id.clone(),
                        material: cavity.clone(),
                    });
                }
            }
        }

        // Every framed assembly — wall, floor, roof, or ceiling — must have
        // exactly one framing layer, so the framing band is unambiguous.
        if framing_layers != 1 {
            return Err(ModelError::SystemFramingLayerCount {
                system: self.id.clone(),
                found: framing_layers,
            });
        }

        Ok(())
    }
}

/// The family of framing member a `Framing` layer produces, so the solver can
/// dispatch member geometry by family. A closed enum. Defaults to `Stud`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
pub enum MemberFamily {
    #[default]
    Stud,
    Rafter,
    CeilingJoist,
    FloorJoist,
    Truss,
}

impl MemberFamily {
    pub const ALL: [Self; 5] = [
        Self::Stud,
        Self::Rafter,
        Self::CeilingJoist,
        Self::FloorJoist,
        Self::Truss,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Stud => "Stud",
            Self::Rafter => "Rafter",
            Self::CeilingJoist => "Ceiling joist",
            Self::FloorJoist => "Floor joist",
            Self::Truss => "Truss",
        }
    }
}

/// A roof/ceiling pitch as an integer rise:run ratio of ticks (float-free, so it
/// round-trips deterministically and keeps `Eq`). True sloped lengths are derived
/// transiently in the solver/SVG boundary, never stored. Flat is `rise == 0`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Slope {
    pub rise: Length,
    pub run: Length,
}

impl Slope {
    pub const fn new(rise: Length, run: Length) -> Self {
        Self { rise, run }
    }

    /// A flat slope (zero rise over a conventional 12" run).
    pub fn flat() -> Self {
        Self {
            rise: Length::ZERO,
            run: Length::from_whole_inches(12),
        }
    }

    /// Whether this slope is flat (zero rise).
    pub fn is_flat(self) -> bool {
        self.rise == Length::ZERO
    }
}

/// A sloped ceiling's pitch plus the low (spring) edge it falls to — the downslope
/// reference that makes a bare [`Slope`] unambiguous. It mirrors a roof plane's
/// `(slope, eave_edge)`: the surface lies at the ceiling's `height` along `low_edge`
/// and rises across the region at `pitch`, reusing the [`RoofPlaneFrame`] affine
/// lift. Because `low_edge` indexes an explicit outline, a sloped ceiling needs a
/// [`SurfaceRegion::Polygon`] region (a `Room` boundary has no stable edge order);
/// validation enforces this. A `None` slope on a [`Ceiling`] stays flat and
/// byte-identical.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CeilingSlope {
    pub pitch: Slope,
    /// Index into the ceiling's polygon outline of the low (spring) edge `(i, i+1)`,
    /// mirroring [`RoofPlane::eave_edge`].
    pub low_edge: u32,
}

impl CeilingSlope {
    pub const fn new(pitch: Slope, low_edge: u32) -> Self {
        Self { pitch, low_edge }
    }
}

/// The direction floor/ceiling joists span across a region. `Shorter` (the
/// default) spans the shorter clear dimension; `Along`/`Across` follow the
/// region's principal axes; `Explicit` carries an in-plane direction vector
/// `(x, y)` in ticks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SpanDirection {
    #[default]
    Shorter,
    Along,
    Across,
    Explicit(Point2),
}

/// The plan-area a ceiling or floor deck covers: an enclosed room (by id) or an
/// explicit polygon outline in plan ticks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SurfaceRegion {
    Room(ElementId),
    Polygon(Vec<Point2>),
}

/// A 2-D, plane-local opening hosted on a roof plane (e.g. a skylight). Distinct
/// from the 1-D wall [`Opening`]: it carries a 2-D `center` in the roof plane's
/// local basis. Nested in [`RoofPlane::openings`] by containment (no
/// back-reference).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoofOpening {
    pub id: ElementId,
    pub kind: OpeningKind,
    pub center: Point2,
    pub width: Length,
    pub height: Length,
}

impl RoofOpening {
    pub fn new(
        id: impl Into<String>,
        kind: OpeningKind,
        center: Point2,
        width: Length,
        height: Length,
    ) -> Self {
        Self {
            id: ElementId::new(id),
            kind,
            center,
            width,
            height,
        }
    }
}

/// A single planar (sloped or flat) structural roof face: a plan-projected
/// polygon `outline`, a `slope` (rise:run), a designated `eave_edge` (the
/// downslope bearing edge, indexed into `outline`), a `reference_elevation` (the
/// bearing/springing line), and eave/rake overhangs. References a `Roof` system.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoofPlane {
    pub id: ElementId,
    pub name: String,
    pub level: ElementId,
    pub system: ElementId,
    pub outline: Vec<Point2>,
    pub slope: Slope,
    /// Index into `outline` of the eave (downslope) edge: edge `(i, i+1)`.
    pub eave_edge: u32,
    pub reference_elevation: Length,
    #[serde(default)]
    pub eave_overhang: Length,
    #[serde(default)]
    pub rake_overhang: Length,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub openings: Vec<RoofOpening>,
}

impl RoofPlane {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        level: impl Into<String>,
        system: impl Into<String>,
        outline: Vec<Point2>,
        slope: Slope,
        eave_edge: u32,
        reference_elevation: Length,
    ) -> Self {
        Self {
            id: ElementId::new(id),
            name: name.into(),
            level: ElementId::new(level),
            system: ElementId::new(system),
            outline,
            slope,
            eave_edge,
            reference_elevation,
            eave_overhang: Length::ZERO,
            rake_overhang: Length::ZERO,
            openings: Vec::new(),
        }
    }

    pub fn with_eave_overhang(mut self, overhang: Length) -> Self {
        self.eave_overhang = overhang;
        self
    }

    pub fn with_rake_overhang(mut self, overhang: Length) -> Self {
        self.rake_overhang = overhang;
        self
    }

    /// The plane's affine elevation field: its eave origin, up-slope unit normal,
    /// eave length, rise-per-plan-run ratio, and springing elevation — the single
    /// definition of "where a roof plane sits in 3-D", shared by the solver's
    /// framing geometry and the renderers' surface emission so they cannot drift.
    /// Float-valued but purely derived (never stored), like
    /// [`polygon_area_square_inches`](crate::polygon_area_square_inches). `None` for
    /// a degenerate outline (fewer than 3 points or a zero-length eave edge).
    pub fn frame(&self) -> Option<RoofPlaneFrame> {
        surface_frame(
            &self.outline,
            self.slope,
            self.eave_edge,
            self.reference_elevation,
        )
    }

    /// Validate the plane's own geometry (outline, eave edge, slope) and its
    /// nested openings, registering opening ids in the shared `ids` set. The
    /// level/system references are checked by [`BuildingModel::validate`].
    fn validate_geometry(&self, ids: &mut BTreeSet<ElementId>) -> Result<(), ModelError> {
        if self.outline.len() < 3 {
            return Err(ModelError::RoofPlaneOutlineTooFewPoints {
                roof_plane: self.id.clone(),
            });
        }
        if polygon_has_redundant_vertices(&self.outline) {
            return Err(ModelError::RoofPlaneOutlineHasRedundantVertices {
                roof_plane: self.id.clone(),
            });
        }
        if polygon_self_intersects(&self.outline) {
            return Err(ModelError::RoofPlaneOutlineSelfIntersecting {
                roof_plane: self.id.clone(),
            });
        }
        if (self.eave_edge as usize) >= self.outline.len() {
            return Err(ModelError::RoofPlaneEaveEdgeOutOfRange {
                roof_plane: self.id.clone(),
            });
        }
        if self.slope.run <= Length::ZERO {
            return Err(ModelError::RoofPlaneInvalidSlope {
                roof_plane: self.id.clone(),
            });
        }
        if self.eave_overhang < Length::ZERO || self.rake_overhang < Length::ZERO {
            return Err(ModelError::RoofPlaneInvalidOverhang {
                roof_plane: self.id.clone(),
            });
        }
        for opening in &self.openings {
            validate_element_id(&opening.id)?;
            insert_unique_id(ids, &opening.id)?;
            if opening.width <= Length::ZERO || opening.height <= Length::ZERO {
                return Err(ModelError::InvalidOpeningSize {
                    opening: opening.id.clone(),
                });
            }
        }
        Ok(())
    }
}

/// Build a planar surface's affine elevation field from its plan outline, pitch,
/// low (eave/spring) edge, and the building elevation at that low edge. The single
/// definition of the lift shared by [`RoofPlane::frame`] and [`Ceiling::frame`], so
/// a roof plane and a sloped ceiling project identically. `None` for a degenerate
/// outline (fewer than three points or a zero-length low edge).
pub fn surface_frame(
    outline: &[Point2],
    slope: Slope,
    low_edge: u32,
    reference_elevation: Length,
) -> Option<RoofPlaneFrame> {
    let n = outline.len();
    if n < 3 {
        return None;
    }
    let i = low_edge as usize % n;
    let a = outline[i];
    let b = outline[(i + 1) % n];
    let (ax, ay) = (a.x.inches(), a.y.inches());
    let ex = b.x.inches() - ax;
    let ey = b.y.inches() - ay;
    let eave_length = (ex * ex + ey * ey).sqrt();
    if eave_length <= f64::EPSILON {
        return None;
    }
    // Up-slope unit normal: perpendicular to the low edge, flipped to point toward
    // the outline centroid (so it is independent of the polygon's winding).
    let (mut nx, mut ny) = (-ey / eave_length, ex / eave_length);
    let cx = outline.iter().map(|p| p.x.inches()).sum::<f64>() / n as f64;
    let cy = outline.iter().map(|p| p.y.inches()).sum::<f64>() / n as f64;
    let (mx, my) = (ax + ex / 2.0, ay + ey / 2.0);
    if nx * (cx - mx) + ny * (cy - my) < 0.0 {
        nx = -nx;
        ny = -ny;
    }
    let run = slope.run.inches();
    let rise_over_run = if run > 0.0 {
        slope.rise.inches() / run
    } else {
        0.0
    };
    Some(RoofPlaneFrame {
        eave_origin: (ax, ay),
        eave_axis: (ex / eave_length, ey / eave_length),
        up_slope: (nx, ny),
        eave_length,
        rise_over_run,
        reference_elevation: reference_elevation.inches(),
    })
}

/// A roof plane's plan-local elevation field (all lengths in inches), from
/// [`RoofPlane::frame`] / [`Ceiling::frame`] via [`surface_frame`]. The plane is
/// affine in plan — `z` is linear in `x`/`y` — so projecting any plan point keeps
/// it coplanar. Fields are private and the type is constructed only by
/// [`surface_frame`], so [`Self::up_slope`] is always a true unit vector; read it
/// (and the rest) through the accessors.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RoofPlaneFrame {
    eave_origin: (f64, f64),
    eave_axis: (f64, f64),
    up_slope: (f64, f64),
    eave_length: f64,
    rise_over_run: f64,
    reference_elevation: f64,
}

/// A schema-neutral triangular extension from an authored exterior wall top to
/// two matched roof rake edges. All elevations are absolute building elevations;
/// `peak_x` is measured from the authored wall start in wall-local plan distance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GableWallProfile {
    pub width: Length,
    pub peak_x: Length,
    pub base_elevation: Length,
    pub peak_elevation: Length,
    pub peak: Point2,
}

impl GableWallProfile {
    pub fn peak_height(&self) -> Length {
        self.peak_elevation - self.base_elevation
    }

    /// Triangular wall-local `(x, height-above-authored-wall-top)` outline, with
    /// counter-clockwise winding for direct use by the shared ear-clipper.
    pub fn outline(&self) -> Vec<Point2> {
        vec![
            Point2::new(Length::ZERO, Length::ZERO),
            Point2::new(self.width, Length::ZERO),
            Point2::new(self.peak_x, self.peak_height()),
        ]
    }

    /// Height above the authored wall top at `x`, linearly interpolated along the
    /// two rake edges. Out-of-span values clamp to the wall ends.
    pub fn height_at(&self, x: Length) -> Length {
        let x = x.clamp(Length::ZERO, self.width);
        let height = self.peak_height().inches();
        if x <= self.peak_x {
            let run = self.peak_x.inches();
            return if run <= f64::EPSILON {
                Length::ZERO
            } else {
                Length::from_inches(height * x.inches() / run)
            };
        }
        let run = (self.width - self.peak_x).inches();
        if run <= f64::EPSILON {
            Length::ZERO
        } else {
            Length::from_inches(height * (self.width - x).inches() / run)
        }
    }
}

fn roof_high_edge_index(plane: &RoofPlane, frame: &RoofPlaneFrame) -> Option<usize> {
    if plane.outline.len() < 3 {
        return None;
    }
    let mut best_index = 0;
    let mut best_distance = f64::NEG_INFINITY;
    for index in 0..plane.outline.len() {
        let a = plane.outline[index];
        let b = plane.outline[(index + 1) % plane.outline.len()];
        let distance = frame.up_slope_distance(
            (a.x.inches() + b.x.inches()) / 2.0,
            (a.y.inches() + b.y.inches()) / 2.0,
        );
        if distance > best_distance {
            best_distance = distance;
            best_index = index;
        }
    }
    Some(best_index)
}

fn roof_planes_share_edge(plane: &RoofPlane, edge_index: usize, other: &RoofPlane) -> bool {
    let edge = (
        plane.outline[edge_index],
        plane.outline[(edge_index + 1) % plane.outline.len()],
    );
    (0..other.outline.len()).any(|index| {
        let candidate = (
            other.outline[index],
            other.outline[(index + 1) % other.outline.len()],
        );
        (edge.0 == candidate.0 && edge.1 == candidate.1)
            || (edge.0 == candidate.1 && edge.1 == candidate.0)
    })
}

/// Offset each polygon edge by its outward plan distance and intersect adjacent
/// offset lines. Rounds only the final intersections back to integer ticks.
fn offset_polygon_edges(points: &[Point2], offsets: &[Length]) -> Option<Vec<Point2>> {
    if points.len() < 3 || points.len() != offsets.len() {
        return None;
    }
    let signed_area2 = points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .map(|(a, b)| {
            a.x.ticks() as i128 * b.y.ticks() as i128 - b.x.ticks() as i128 * a.y.ticks() as i128
        })
        .sum::<i128>();
    if signed_area2 == 0 {
        return None;
    }
    let ccw = signed_area2 > 0;
    let mut lines = Vec::with_capacity(points.len());
    for (index, (&a, &b)) in points.iter().zip(points.iter().cycle().skip(1)).enumerate() {
        let (ax, ay) = (a.x.inches(), a.y.inches());
        let (dx, dy) = (b.x.inches() - ax, b.y.inches() - ay);
        let length = (dx * dx + dy * dy).sqrt();
        if length <= f64::EPSILON {
            return None;
        }
        let (nx, ny) = if ccw {
            (dy / length, -dx / length)
        } else {
            (-dy / length, dx / length)
        };
        let distance = offsets[index].inches();
        lines.push(((ax + nx * distance, ay + ny * distance), (dx, dy)));
    }

    let mut outline = Vec::with_capacity(points.len());
    for index in 0..points.len() {
        let previous = lines[(index + points.len() - 1) % points.len()];
        let current = lines[index];
        let intersection = line_intersection(previous, current)?;
        outline.push(Point2::new(
            Length::from_inches(intersection.0),
            Length::from_inches(intersection.1),
        ));
    }
    Some(outline)
}

fn line_intersection(
    first: ((f64, f64), (f64, f64)),
    second: ((f64, f64), (f64, f64)),
) -> Option<(f64, f64)> {
    let denominator = first.1.0 * second.1.1 - first.1.1 * second.1.0;
    if denominator.abs() <= 1.0e-9 {
        return None;
    }
    let delta = (second.0.0 - first.0.0, second.0.1 - first.0.1);
    let t = (delta.0 * second.1.1 - delta.1 * second.1.0) / denominator;
    Some((first.0.0 + first.1.0 * t, first.0.1 + first.1.1 * t))
}

#[derive(Clone)]
struct GableEdgeCandidate {
    roof: ElementId,
    start_x: Length,
    end_x: Length,
    start_elevation: Length,
    end_elevation: Length,
    start: Point2,
    end: Point2,
}

fn derive_gable_wall_profile(model: &BuildingModel, wall: &Wall) -> Option<GableWallProfile> {
    let level_elevation = model
        .levels
        .iter()
        .find(|level| level.id == wall.level)
        .map(|level| level.elevation)
        .unwrap_or(Length::ZERO);
    let base_elevation = level_elevation + wall.height;
    let mut edges = Vec::new();

    for plane in model
        .roof_planes
        .iter()
        .filter(|plane| plane.level == wall.level)
    {
        // Rendering and other read-only consumers can inspect an unvalidated
        // in-progress model. One unrelated degenerate plane must not suppress an
        // otherwise complete gable derived from the remaining valid planes.
        let Some(frame) = plane.frame() else {
            continue;
        };
        for index in 0..plane.outline.len() {
            if index == plane.eave_edge as usize % plane.outline.len() {
                continue;
            }
            let a = plane.outline[index];
            let b = plane.outline[(index + 1) % plane.outline.len()];
            let Some(mut ax) = wall_local_x(wall, a) else {
                continue;
            };
            let Some(mut bx) = wall_local_x(wall, b) else {
                continue;
            };
            if ax > bx {
                std::mem::swap(&mut ax, &mut bx);
                let (a_elevation, b_elevation) = (
                    Length::from_inches(frame.elevation_at(b.x.inches(), b.y.inches())),
                    Length::from_inches(frame.elevation_at(a.x.inches(), a.y.inches())),
                );
                edges.push(GableEdgeCandidate {
                    roof: plane.id.clone(),
                    start_x: ax,
                    end_x: bx,
                    start_elevation: a_elevation,
                    end_elevation: b_elevation,
                    start: b,
                    end: a,
                });
            } else {
                edges.push(GableEdgeCandidate {
                    roof: plane.id.clone(),
                    start_x: ax,
                    end_x: bx,
                    start_elevation: Length::from_inches(
                        frame.elevation_at(a.x.inches(), a.y.inches()),
                    ),
                    end_elevation: Length::from_inches(
                        frame.elevation_at(b.x.inches(), b.y.inches()),
                    ),
                    start: a,
                    end: b,
                });
            }
        }
    }

    if edges.len() != 2 || edges[0].roof == edges[1].roof {
        return None;
    }
    edges.sort_by_key(|edge| (edge.start_x, edge.end_x, edge.roof.clone()));
    let left = &edges[0];
    let right = &edges[1];
    let tolerance = Length::from_ticks(1);
    let near = |a: Length, b: Length| (a - b).abs() <= tolerance;
    if !near(left.start_x, Length::ZERO)
        || !near(right.end_x, wall.length)
        || !near(left.end_x, right.start_x)
        || left.end != right.start
        || !near(left.start_elevation, base_elevation)
        || !near(right.end_elevation, base_elevation)
        || !near(left.end_elevation, right.start_elevation)
        || left.end_elevation <= base_elevation + tolerance
    {
        return None;
    }

    Some(GableWallProfile {
        width: wall.length,
        peak_x: left.end_x,
        base_elevation,
        peak_elevation: left.end_elevation,
        peak: left.end,
    })
}

/// Project a point on the wall centerline into authored wall-local x. Exact
/// tick-valued collinearity prevents a nearby parallel roof edge from being
/// mistaken for a wall rake; one tick of endpoint tolerance absorbs derived
/// length rounding for non-axis-aligned walls.
fn wall_local_x(wall: &Wall, point: Point2) -> Option<Length> {
    let (dx, dy) = (
        wall.end.x.ticks() as i128 - wall.start.x.ticks() as i128,
        wall.end.y.ticks() as i128 - wall.start.y.ticks() as i128,
    );
    let (px, py) = (
        point.x.ticks() as i128 - wall.start.x.ticks() as i128,
        point.y.ticks() as i128 - wall.start.y.ticks() as i128,
    );
    if dx * py - dy * px != 0 {
        return None;
    }
    let denominator = (dx * dx + dy * dy) as f64;
    if denominator <= f64::EPSILON {
        return None;
    }
    let t = (px * dx + py * dy) as f64 / denominator;
    let x = Length::from_inches(wall.length.inches() * t);
    let tolerance = Length::from_ticks(1);
    (x >= Length::ZERO - tolerance && x <= wall.length + tolerance)
        .then_some(x.clamp(Length::ZERO, wall.length))
}

impl RoofPlaneFrame {
    /// The eave edge's first endpoint (the up-slope distance origin), inches.
    pub fn eave_origin(&self) -> (f64, f64) {
        self.eave_origin
    }

    /// Unit direction from the authored eave edge's first endpoint to its second.
    pub fn eave_axis(&self) -> (f64, f64) {
        self.eave_axis
    }

    /// Up-slope direction in plan (toward the outline centroid): a dimensionless
    /// unit vector.
    pub fn up_slope(&self) -> (f64, f64) {
        self.up_slope
    }

    /// Eave-edge length, inches — the axis rafters array along.
    pub fn eave_length(&self) -> f64 {
        self.eave_length
    }

    /// Rise per unit plan run (`slope.rise / slope.run`); 0 when flat.
    pub fn rise_over_run(&self) -> f64 {
        self.rise_over_run
    }

    /// Springing (reference/bearing) elevation, inches.
    pub fn reference_elevation(&self) -> f64 {
        self.reference_elevation
    }

    /// Up-slope plan distance of a plan point from the eave line, inches (negative
    /// down-slope of the eave, e.g. on an overhang tail).
    pub fn up_slope_distance(&self, x: f64, y: f64) -> f64 {
        (x - self.eave_origin.0) * self.up_slope.0 + (y - self.eave_origin.1) * self.up_slope.1
    }

    /// The plane's true building elevation at a plan point, inches: the eave
    /// springing raised by the up-slope distance times the pitch.
    pub fn elevation_at(&self, x: f64, y: f64) -> f64 {
        self.reference_elevation + self.up_slope_distance(x, y) * self.rise_over_run
    }

    /// Convert surface-local `(along-eave, up-slope)` plan distances back to an
    /// exact tick-rounded world-plan point. Derived framing uses this to carry
    /// unambiguous endpoints instead of asking each renderer to reconstruct them.
    pub fn plan_point_at(&self, along_eave: Length, up_slope: Length) -> Point2 {
        let along = along_eave.inches();
        let up = up_slope.inches();
        Point2::new(
            Length::from_inches(
                self.eave_origin.0 + self.eave_axis.0 * along + self.up_slope.0 * up,
            ),
            Length::from_inches(
                self.eave_origin.1 + self.eave_axis.1 * along + self.up_slope.1 * up,
            ),
        )
    }
}

/// A per-region finished ceiling surface at an authored `height` below the level
/// top. Flat in v1 (`slope` is `None`); a region with no `Ceiling` is a cathedral
/// condition. References a `Ceiling` system.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ceiling {
    pub id: ElementId,
    pub name: String,
    pub level: ElementId,
    pub system: ElementId,
    pub region: SurfaceRegion,
    pub height: Length,
    /// `None` keeps the ceiling flat (and byte-identical); `Some` makes it a planar
    /// sloped (scissor/vault) surface — see [`CeilingSlope`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slope: Option<CeilingSlope>,
}

impl Ceiling {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        level: impl Into<String>,
        system: impl Into<String>,
        region: SurfaceRegion,
        height: Length,
    ) -> Self {
        Self {
            id: ElementId::new(id),
            name: name.into(),
            level: ElementId::new(level),
            system: ElementId::new(system),
            region,
            height,
            slope: None,
        }
    }

    /// The sloped surface's affine lift, if this ceiling carries a slope: the joists,
    /// mesh, and render all project through it, exactly as a roof plane does. `None`
    /// for a flat ceiling, a non-polygon region (validation forbids a sloped one), or
    /// a degenerate outline. `reference_elevation` is the building elevation at the
    /// low edge — the caller derives it from the level top and `height`
    /// (`level.elevation + level.height − ceiling.height`).
    pub fn frame(&self, reference_elevation: Length) -> Option<RoofPlaneFrame> {
        let slope = self.slope?;
        let SurfaceRegion::Polygon(outline) = &self.region else {
            return None;
        };
        surface_frame(outline, slope.pitch, slope.low_edge, reference_elevation)
    }
}

/// The horizontal structural deck of a level: its `region` plus a joist `span`
/// direction. A flat ceiling and a floor deck share the same joisting generator.
/// References a `Floor` system.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FloorDeck {
    pub id: ElementId,
    pub name: String,
    pub level: ElementId,
    pub system: ElementId,
    pub region: SurfaceRegion,
    #[serde(default)]
    pub span: SpanDirection,
}

impl FloorDeck {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        level: impl Into<String>,
        system: impl Into<String>,
        region: SurfaceRegion,
    ) -> Self {
        Self {
            id: ElementId::new(id),
            name: name.into(),
            level: ElementId::new(level),
            system: ElementId::new(system),
            region,
            span: SpanDirection::Shorter,
        }
    }

    pub fn with_span(mut self, span: SpanDirection) -> Self {
        self.span = span;
        self
    }
}

/// Validate a [`SurfaceRegion`]: a `Room` reference must resolve to a known room;
/// a `Polygon` must have at least three points and not self-intersect. `owner` is
/// the id of the ceiling/floor deck carrying the region, for error reporting.
fn validate_surface_region(
    region: &SurfaceRegion,
    room_ids: &BTreeSet<ElementId>,
    owner: &ElementId,
) -> Result<(), ModelError> {
    match region {
        SurfaceRegion::Room(room) => {
            if !room_ids.contains(room) {
                return Err(ModelError::SurfaceRegionReferencesUnknownRoom {
                    element: owner.clone(),
                    room: room.clone(),
                });
            }
        }
        SurfaceRegion::Polygon(points) => {
            if points.len() < 3 {
                return Err(ModelError::SurfaceRegionPolygonTooFewPoints {
                    element: owner.clone(),
                });
            }
            if polygon_self_intersects(points) {
                return Err(ModelError::SurfaceRegionPolygonSelfIntersecting {
                    element: owner.clone(),
                });
            }
        }
    }
    Ok(())
}

/// Whether a closed polygon (implicitly closing `points[n-1] -> points[0]`) has
/// any pair of non-adjacent edges that cross or touch. Integer-tick orientation
/// math in `i128`, so it is exact and deterministic.
fn polygon_self_intersects(points: &[Point2]) -> bool {
    // Tolerate an explicitly-closed ring (the last vertex repeats the first):
    // closure is implicit here, so the duplicate would otherwise read as a
    // zero-length edge touching the first edge and be mis-reported as a crossing.
    let points = match points {
        [first, .., last] if first == last => &points[..points.len() - 1],
        _ => points,
    };
    let n = points.len();
    if n < 4 {
        return false;
    }
    for i in 0..n {
        let a1 = points[i];
        let a2 = points[(i + 1) % n];
        for j in (i + 1)..n {
            // Skip edges that share a vertex: the next edge, and the wrap-around
            // adjacency between the last edge and the first.
            if j == i + 1 || (i == 0 && j == n - 1) {
                continue;
            }
            let b1 = points[j];
            let b2 = points[(j + 1) % n];
            if segments_intersect(a1, a2, b1, b2) {
                return true;
            }
        }
    }
    false
}

/// Roof outlines are stored as implicit rings. Consecutive duplicate points,
/// an explicit closing point, or a middle point on a straight run make offset
/// line intersections ambiguous and are therefore rejected at validation.
fn polygon_has_redundant_vertices(points: &[Point2]) -> bool {
    if points.len() < 3 {
        return false;
    }
    (0..points.len()).any(|index| {
        let previous = points[(index + points.len() - 1) % points.len()];
        let current = points[index];
        let next = points[(index + 1) % points.len()];
        if previous == current || current == next {
            return true;
        }
        let ax = current.x.ticks() as i128 - previous.x.ticks() as i128;
        let ay = current.y.ticks() as i128 - previous.y.ticks() as i128;
        let bx = next.x.ticks() as i128 - current.x.ticks() as i128;
        let by = next.y.ticks() as i128 - current.y.ticks() as i128;
        ax * by - ay * bx == 0
    })
}

/// Whether closed segments `p1p2` and `p3p4` intersect (proper crossing or
/// collinear touch).
fn segments_intersect(p1: Point2, p2: Point2, p3: Point2, p4: Point2) -> bool {
    let d1 = orientation(p3, p4, p1);
    let d2 = orientation(p3, p4, p2);
    let d3 = orientation(p1, p2, p3);
    let d4 = orientation(p1, p2, p4);

    let proper =
        ((d1 > 0 && d2 < 0) || (d1 < 0 && d2 > 0)) && ((d3 > 0 && d4 < 0) || (d3 < 0 && d4 > 0));
    proper
        || (d1 == 0 && on_segment(p3, p4, p1))
        || (d2 == 0 && on_segment(p3, p4, p2))
        || (d3 == 0 && on_segment(p1, p2, p3))
        || (d4 == 0 && on_segment(p1, p2, p4))
}

/// Twice the signed area of triangle `abc` in `i128` ticks² (positive = CCW).
fn orientation(a: Point2, b: Point2, c: Point2) -> i128 {
    let abx = (b.x.ticks() - a.x.ticks()) as i128;
    let aby = (b.y.ticks() - a.y.ticks()) as i128;
    let acx = (c.x.ticks() - a.x.ticks()) as i128;
    let acy = (c.y.ticks() - a.y.ticks()) as i128;
    abx * acy - aby * acx
}

/// Whether point `p`, assumed collinear with `a`-`b`, lies within the segment's
/// bounding box (i.e. on the segment).
fn on_segment(a: Point2, b: Point2, p: Point2) -> bool {
    let within = |v: i64, x: i64, y: i64| (x.min(y)..=x.max(y)).contains(&v);
    within(p.x.ticks(), a.x.ticks(), b.x.ticks()) && within(p.y.ticks(), a.y.ticks(), b.y.ticks())
}

/// serde `skip_serializing_if` predicate: a zero length is the unset default.
fn length_is_zero(value: &Length) -> bool {
    *value == Length::ZERO
}

/// serde `skip_serializing_if` predicate: `Stud` is the framing default.
fn member_family_is_default(value: &MemberFamily) -> bool {
    *value == MemberFamily::Stud
}

/// Where a material is defined. `Project` materials are embedded first-party
/// definitions; `Library` materials are vendored into the model with descriptive
/// provenance.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum MaterialSource {
    #[default]
    Project,
    Library(Provenance),
}

impl MaterialSource {
    pub fn is_project(&self) -> bool {
        matches!(self, Self::Project)
    }
}

/// A typed property value for the extensible material property map. Float-free
/// and `Eq` so the model stays deterministic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum PropertyValue {
    Int(i64),
    Length(Length),
    Text(String),
    Flag(bool),
}

/// Binary asset role carried by an [`AssetRef`]. The bytes live outside the
/// canonical model; the role records how render/build code should interpret the
/// content when a caller has a local asset store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum TextureRole {
    Texture,
    Height,
}

/// A content-addressed binary asset reference. The model stores only the hash
/// and enough metadata to interpret locally cached bytes; missing assets degrade
/// to the material's flat fallback color.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetRef {
    pub hash: String,
    pub media_type: String,
    pub role: TextureRole,
}

impl AssetRef {
    pub fn new(hash: impl Into<String>, media_type: impl Into<String>, role: TextureRole) -> Self {
        Self {
            hash: hash.into(),
            media_type: media_type.into(),
            role,
        }
    }

    fn validate(&self, expected_role: TextureRole) -> Result<(), ModelError> {
        if self.role != expected_role {
            return Err(ModelError::AssetRoleMismatch {
                expected: expected_role,
                found: self.role,
            });
        }
        if !is_blake3_hash(&self.hash) {
            return Err(ModelError::InvalidAssetHash {
                hash: self.hash.clone(),
            });
        }
        if self.media_type.trim().is_empty() {
            return Err(ModelError::InvalidAssetMediaType {
                hash: self.hash.clone(),
            });
        }
        Ok(())
    }
}

/// Authored finish for a material. Textured/depth mapped materials remain
/// self-contained: their binary bytes live in an out-of-band content-addressed
/// asset store and the fallback color keeps open/render possible without it.
///
/// GROWTH PATH (not built now):
///   - `LappedSiding { color, reveal: Length }` — parametric, may affect geometry
///   - `Masonry { unit, coursing, color }` — depth-mapped brick/block
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Appearance {
    SolidColor([u8; 3]),
    Textured {
        color: [u8; 3],
        texture: AssetRef,
        scale: Length,
    },
    DepthMapped {
        color: [u8; 3],
        height: AssetRef,
        scale: Length,
    },
}

impl Appearance {
    /// A representative color for this appearance.
    pub fn color(&self) -> [u8; 3] {
        match self {
            Self::SolidColor(color) => *color,
            Self::Textured { color, .. } | Self::DepthMapped { color, .. } => *color,
        }
    }

    fn validate(&self) -> Result<(), ModelError> {
        match self {
            Self::SolidColor(_) => Ok(()),
            Self::Textured { texture, scale, .. } => {
                validate_asset_scale(*scale)?;
                texture.validate(TextureRole::Texture)
            }
            Self::DepthMapped { height, scale, .. } => {
                validate_asset_scale(*scale)?;
                height.validate(TextureRole::Height)
            }
        }
    }
}

/// An open, extensible material definition referenced by stable id. Substance,
/// properties, and appearance are open data so external/shared libraries plug in
/// via the same reference + resolver without schema churn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Material {
    pub id: ElementId,
    pub name: String,
    #[serde(default, skip_serializing_if = "MaterialSource::is_project")]
    pub source: MaterialSource,
    pub appearance: Appearance,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub properties: BTreeMap<String, PropertyValue>,
}

impl Material {
    /// A project material with a solid-color appearance and no extra properties.
    pub fn solid_color(id: impl Into<String>, name: impl Into<String>, color: [u8; 3]) -> Self {
        Self {
            id: ElementId::new(id),
            name: name.into(),
            source: MaterialSource::Project,
            appearance: Appearance::SolidColor(color),
            tags: Vec::new(),
            properties: BTreeMap::new(),
        }
    }

    pub fn with_tags<I, S>(mut self, tags: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.tags = tags.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_r_per_inch_milli(mut self, r_per_inch_milli: i64) -> Self {
        self.properties.insert(
            "r_per_inch_milli".to_owned(),
            PropertyValue::Int(r_per_inch_milli),
        );
        self
    }

    pub(crate) fn validate(&self) -> Result<(), ModelError> {
        self.appearance.validate()
    }

    /// The representative color from this material's appearance.
    pub fn color(&self) -> [u8; 3] {
        self.appearance.color()
    }

    /// The well-known `r_per_inch_milli` property (R × 1000 per inch), or 0 when
    /// absent or not an integer.
    pub fn r_per_inch_milli(&self) -> i64 {
        match self.properties.get("r_per_inch_milli") {
            Some(PropertyValue::Int(value)) => *value,
            _ => 0,
        }
    }

    /// This material's R-value in milli-R (R × 1000) across `thickness`, by exact
    /// integer math over ticks (no inch rounding): a 5/8" layer of an R/in=900
    /// material contributes 562 milli-R, not 900.
    pub fn r_value_milli(&self, thickness: Length) -> i64 {
        self.r_per_inch_milli() * thickness.ticks() / Length::TICKS_PER_INCH
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectSize {
    pub width: Length,
    pub depth: Length,
    pub height: Length,
}

impl ObjectSize {
    pub const fn new(width: Length, depth: Length, height: Length) -> Self {
        Self {
            width,
            depth,
            height,
        }
    }

    fn is_positive(&self) -> bool {
        self.width > Length::ZERO && self.depth > Length::ZERO && self.height > Length::ZERO
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum QuarterTurn {
    #[default]
    Deg0,
    Deg90,
    Deg180,
    Deg270,
}

impl QuarterTurn {
    pub const ALL: [Self; 4] = [Self::Deg0, Self::Deg90, Self::Deg180, Self::Deg270];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Deg0 => "0 deg",
            Self::Deg90 => "90 deg",
            Self::Deg180 => "180 deg",
            Self::Deg270 => "270 deg",
        }
    }

    pub const fn is_zero(&self) -> bool {
        matches!(self, Self::Deg0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum MepObjectKind {
    Electrical,
    Lighting,
    Plumbing,
    Mechanical,
    Other,
}

impl MepObjectKind {
    pub const ALL: [Self; 5] = [
        Self::Electrical,
        Self::Lighting,
        Self::Plumbing,
        Self::Mechanical,
        Self::Other,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Electrical => "Electrical",
            Self::Lighting => "Lighting",
            Self::Plumbing => "Plumbing",
            Self::Mechanical => "Mechanical",
            Self::Other => "Other",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Furnishing {
    pub id: ElementId,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Provenance>,
    pub size: ObjectSize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub properties: BTreeMap<String, PropertyValue>,
}

impl Furnishing {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        width: Length,
        depth: Length,
        height: Length,
    ) -> Self {
        Self {
            id: ElementId::new(id),
            name: name.into(),
            source: None,
            size: ObjectSize::new(width, depth, height),
            tags: Vec::new(),
            properties: BTreeMap::new(),
        }
    }

    pub(crate) fn validate(&self, ids: &mut BTreeSet<ElementId>) -> Result<(), ModelError> {
        validate_element_id(&self.id)?;
        insert_unique_id(ids, &self.id)?;
        if !self.size.is_positive() {
            return Err(ModelError::InvalidObjectSize {
                object: self.id.clone(),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MepObject {
    pub id: ElementId,
    pub name: String,
    pub kind: MepObjectKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Provenance>,
    pub size: ObjectSize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub properties: BTreeMap<String, PropertyValue>,
}

impl MepObject {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        kind: MepObjectKind,
        width: Length,
        depth: Length,
        height: Length,
    ) -> Self {
        Self {
            id: ElementId::new(id),
            name: name.into(),
            kind,
            source: None,
            size: ObjectSize::new(width, depth, height),
            tags: Vec::new(),
            properties: BTreeMap::new(),
        }
    }

    pub(crate) fn validate(&self, ids: &mut BTreeSet<ElementId>) -> Result<(), ModelError> {
        validate_element_id(&self.id)?;
        insert_unique_id(ids, &self.id)?;
        if !self.size.is_positive() {
            return Err(ModelError::InvalidObjectSize {
                object: self.id.clone(),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FurnishingInstance {
    pub id: ElementId,
    pub name: String,
    pub family: ElementId,
    #[serde(default = "default_level_id")]
    pub level: ElementId,
    pub position: Point2,
    #[serde(default, skip_serializing_if = "QuarterTurn::is_zero")]
    pub rotation: QuarterTurn,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

impl FurnishingInstance {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        family: impl Into<String>,
        level: impl Into<String>,
        position: Point2,
    ) -> Self {
        Self {
            id: ElementId::new(id),
            name: name.into(),
            family: ElementId::new(family),
            level: ElementId::new(level),
            position,
            rotation: QuarterTurn::Deg0,
            tags: Vec::new(),
        }
    }

    pub(crate) fn validate(&self, ids: &mut BTreeSet<ElementId>) -> Result<(), ModelError> {
        validate_element_id(&self.id)?;
        validate_element_id(&self.family)?;
        validate_element_id(&self.level)?;
        insert_unique_id(ids, &self.id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MepInstance {
    pub id: ElementId,
    pub name: String,
    pub family: ElementId,
    #[serde(default = "default_level_id")]
    pub level: ElementId,
    pub position: Point2,
    #[serde(default, skip_serializing_if = "QuarterTurn::is_zero")]
    pub rotation: QuarterTurn,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

impl MepInstance {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        family: impl Into<String>,
        level: impl Into<String>,
        position: Point2,
    ) -> Self {
        Self {
            id: ElementId::new(id),
            name: name.into(),
            family: ElementId::new(family),
            level: ElementId::new(level),
            position,
            rotation: QuarterTurn::Deg0,
            tags: Vec::new(),
        }
    }

    pub(crate) fn validate(&self, ids: &mut BTreeSet<ElementId>) -> Result<(), ModelError> {
        validate_element_id(&self.id)?;
        validate_element_id(&self.family)?;
        validate_element_id(&self.level)?;
        insert_unique_id(ids, &self.id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Wall {
    pub id: ElementId,
    pub name: String,
    #[serde(default = "default_level_id")]
    pub level: ElementId,
    #[serde(default)]
    pub start: Point2,
    #[serde(default)]
    pub end: Point2,
    pub length: Length,
    pub height: Length,
    pub system: ElementId,
    pub openings: Vec<Opening>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bracing: Vec<BracedPanel>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dimensions: Vec<DimensionConstraint>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

impl Wall {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        length: Length,
        defaults: &FramingDefaults,
    ) -> Self {
        Self {
            id: ElementId::new(id),
            name: name.into(),
            level: default_level_id(),
            start: Point2::new(Length::ZERO, Length::ZERO),
            end: Point2::new(length, Length::ZERO),
            length,
            height: defaults.default_wall_height,
            system: ElementId::new("system-wall-exterior-1"),
            openings: Vec::new(),
            bracing: Vec::new(),
            dimensions: Vec::new(),
            tags: Vec::new(),
        }
    }

    pub fn with_placement(mut self, level: impl Into<String>, start: Point2, end: Point2) -> Self {
        self.level = ElementId::new(level);
        self.start = start;
        self.end = end;
        if let Some(length) = self.placement_length() {
            self.length = length;
        }
        self
    }

    pub fn validate(&self) -> Result<(), ModelError> {
        validate_element_id(&self.id)?;
        validate_element_id(&self.level)?;
        validate_element_id(&self.system)?;

        if self.length <= Length::ZERO {
            return Err(ModelError::InvalidWallLength {
                wall: self.id.clone(),
            });
        }

        if self.height <= Length::ZERO {
            return Err(ModelError::InvalidWallHeight {
                wall: self.id.clone(),
            });
        }

        let Some(placement_length) = self.placement_length() else {
            return Err(ModelError::InvalidWallPlacement {
                wall: self.id.clone(),
            });
        };

        if placement_length != self.length {
            return Err(ModelError::WallLengthPlacementMismatch {
                wall: self.id.clone(),
            });
        }

        let mut spans = Vec::with_capacity(self.openings.len());
        let mut opening_ids = BTreeSet::new();
        for opening in &self.openings {
            validate_element_id(&opening.id)?;
            insert_unique_id(&mut opening_ids, &opening.id)?;

            if opening.width <= Length::ZERO || opening.height <= Length::ZERO {
                return Err(ModelError::InvalidOpeningSize {
                    opening: opening.id.clone(),
                });
            }

            if opening.left() < Length::ZERO || opening.right() > self.length {
                return Err(ModelError::OpeningOutOfBounds {
                    wall: self.id.clone(),
                    opening: opening.id.clone(),
                });
            }

            if opening.top() > self.height {
                return Err(ModelError::OpeningTooTall {
                    wall: self.id.clone(),
                    opening: opening.id.clone(),
                });
            }

            spans.push((opening.left(), opening.right(), opening.id.clone()));
        }

        let mut dimension_ids = BTreeSet::new();
        for dimension in &self.dimensions {
            dimension.validate(&opening_ids, self.length, self.height)?;
            insert_unique_id(&mut dimension_ids, &dimension.id)?;
            if self.would_overconstrain_driving_dimension(dimension) {
                return Err(ModelError::OverconstrainedDimension {
                    dimension: dimension.id.clone(),
                    expected: dimension.value.unwrap_or(Length::ZERO),
                    actual: self
                        .dimension_measurement(dimension)
                        .unwrap_or(Length::ZERO),
                });
            }
            if let Some((expected, actual)) = self.driving_dimension_offsets(dimension)
                && expected != actual
            {
                return Err(ModelError::UnsatisfiedDrivingDimension {
                    dimension: dimension.id.clone(),
                    expected,
                    actual,
                });
            }
        }

        spans.sort_by_key(|(left, _, _)| *left);
        for pair in spans.windows(2) {
            let (_, first_right, first_id) = &pair[0];
            let (second_left, _, second_id) = &pair[1];
            if first_right > second_left {
                return Err(ModelError::OverlappingOpenings {
                    first: first_id.clone(),
                    second: second_id.clone(),
                });
            }
        }

        Ok(())
    }

    pub fn sort_deterministically(&mut self) {
        self.openings.sort_by(|left, right| left.id.cmp(&right.id));
        self.bracing.sort_by(|left, right| left.id.cmp(&right.id));
        self.dimensions
            .sort_by(|left, right| left.id.cmp(&right.id));
    }

    pub fn placement_length(&self) -> Option<Length> {
        if self.start.y == self.end.y && self.start.x != self.end.x {
            Some((self.end.x - self.start.x).abs())
        } else if self.start.x == self.end.x && self.start.y != self.end.y {
            Some((self.end.y - self.start.y).abs())
        } else {
            None
        }
    }

    pub fn point_at_local_x(&self, x: Length) -> Point2 {
        if self.start.y == self.end.y {
            let direction: i64 = if self.end.x >= self.start.x { 1 } else { -1 };
            Point2::new(self.start.x + x * direction, self.start.y)
        } else {
            let direction: i64 = if self.end.y >= self.start.y { 1 } else { -1 };
            Point2::new(self.start.x, self.start.y + x * direction)
        }
    }

    pub fn local_x_for_point(&self, point: Point2) -> Option<Length> {
        if self.start.y == self.end.y && point.y == self.start.y {
            Some((point.x - self.start.x).abs())
        } else if self.start.x == self.end.x && point.x == self.start.x {
            Some((point.y - self.start.y).abs())
        } else {
            None
        }
        .filter(|x| *x <= self.length)
    }

    pub fn has_endpoint(&self, point: Point2) -> bool {
        self.start == point || self.end == point
    }

    /// Whether `point` lies on this wall's segment (endpoints included). General
    /// over straight segments via collinearity plus projection bounds.
    pub fn point_on_segment(&self, point: Point2) -> bool {
        let edge_x = (self.end.x - self.start.x).ticks();
        let edge_y = (self.end.y - self.start.y).ticks();
        let offset_x = (point.x - self.start.x).ticks();
        let offset_y = (point.y - self.start.y).ticks();
        if edge_x * offset_y - edge_y * offset_x != 0 {
            return false;
        }
        let projection = offset_x * edge_x + offset_y * edge_y;
        projection >= 0 && projection <= edge_x * edge_x + edge_y * edge_y
    }

    /// Whether `point` lies on the wall's interior (on the segment, not an
    /// endpoint) — the mid-span condition for a Tee/Cross through wall.
    pub fn point_on_interior(&self, point: Point2) -> bool {
        self.point_on_segment(point) && !self.has_endpoint(point)
    }

    pub fn dimension_measurement(&self, dimension: &DimensionConstraint) -> Option<Length> {
        let start = dimension.start.coordinate(self, dimension.axis)?;
        let end = dimension.end.coordinate(self, dimension.axis)?;
        Some((end - start).abs())
    }

    pub fn remove_opening(&mut self, opening: &ElementId) -> bool {
        let previous_opening_count = self.openings.len();
        self.openings.retain(|candidate| candidate.id != *opening);
        if self.openings.len() == previous_opening_count {
            return false;
        }

        self.dimensions
            .retain(|dimension| !dimension.references_opening(opening));
        true
    }

    pub fn is_driving_dimension_satisfied(&self, dimension: &DimensionConstraint) -> bool {
        if dimension.kind != DimensionKind::Driving {
            return true;
        }

        self.driving_dimension_offsets(dimension)
            .is_some_and(|(expected, actual)| expected == actual)
    }

    pub fn would_overconstrain_driving_dimension(&self, candidate: &DimensionConstraint) -> bool {
        self.is_driving_dimension_overconstrained_against(
            candidate,
            self.dimensions
                .iter()
                .filter(|dimension| dimension.id != candidate.id),
        )
    }

    pub fn driving_dimension_offsets(
        &self,
        dimension: &DimensionConstraint,
    ) -> Option<(Length, Length)> {
        if dimension.kind != DimensionKind::Driving {
            return None;
        }

        let value = dimension.value?;
        let start = dimension.start.coordinate(self, dimension.axis)?;
        let end = dimension.end.coordinate(self, dimension.axis)?;
        let expected = match dimension.direction {
            DimensionDirection::Forward => value,
            DimensionDirection::Backward => Length::ZERO - value,
        };
        let actual = end - start;
        Some((expected, actual))
    }

    fn is_driving_dimension_overconstrained_against<'a>(
        &self,
        candidate: &DimensionConstraint,
        existing: impl IntoIterator<Item = &'a DimensionConstraint>,
    ) -> bool {
        if candidate.kind != DimensionKind::Driving {
            return false;
        }

        let Some(candidate) = self.driving_dimension_constraint(candidate) else {
            return false;
        };
        self.driving_constraint_system(existing)
            .would_overconstrain(&candidate)
    }

    fn driving_constraint_system<'a>(
        &self,
        dimensions: impl IntoIterator<Item = &'a DimensionConstraint>,
    ) -> ConstraintSystem {
        ConstraintSystem::from_constraints(
            self.dimension_variables(),
            dimensions
                .into_iter()
                .filter_map(|dimension| self.driving_dimension_constraint(dimension)),
        )
    }

    fn dimension_variables(&self) -> BTreeSet<ConstraintVariable> {
        let mut variables = BTreeSet::new();
        variables.insert(wall_constraint_variable(&self.id, "length"));
        variables.insert(wall_constraint_variable(&self.id, "height"));
        for opening in &self.openings {
            variables.insert(opening_constraint_variable(&opening.id, "center-x"));
            variables.insert(opening_constraint_variable(&opening.id, "width"));
            variables.insert(opening_constraint_variable(&opening.id, "bottom"));
            variables.insert(opening_constraint_variable(&opening.id, "height"));
        }
        variables
    }

    fn driving_dimension_constraint(
        &self,
        dimension: &DimensionConstraint,
    ) -> Option<LinearConstraint> {
        if dimension.kind != DimensionKind::Driving {
            return None;
        }

        let value = dimension.value?;
        let start = self.dimension_anchor_expression(&dimension.start, dimension.axis)?;
        let mut expression = self.dimension_anchor_expression(&dimension.end, dimension.axis)?;
        expression.add_expression(&start, -1);
        let target = match dimension.direction {
            DimensionDirection::Forward => value * 2,
            DimensionDirection::Backward => value * -2,
        };
        Some(LinearConstraint::new(
            dimension.id.0.clone(),
            expression,
            target,
        ))
    }

    fn dimension_anchor_expression(
        &self,
        anchor: &DimensionAnchor,
        axis: DimensionAxis,
    ) -> Option<LinearExpression> {
        let mut expression = LinearExpression::new();
        match axis {
            DimensionAxis::Horizontal => match anchor {
                DimensionAnchor::WallStart => {}
                DimensionAnchor::WallEnd => {
                    add_wall_horizontal_anchor_terms(
                        &mut expression,
                        &self.id,
                        DimensionHorizontalReference::Right,
                    );
                }
                DimensionAnchor::OpeningLeft { opening } => {
                    self.add_opening_horizontal_anchor_terms(
                        &mut expression,
                        opening,
                        DimensionHorizontalReference::Left,
                    )?;
                }
                DimensionAnchor::OpeningCenter { opening } => {
                    self.add_opening_horizontal_anchor_terms(
                        &mut expression,
                        opening,
                        DimensionHorizontalReference::Center,
                    )?;
                }
                DimensionAnchor::OpeningRight { opening } => {
                    self.add_opening_horizontal_anchor_terms(
                        &mut expression,
                        opening,
                        DimensionHorizontalReference::Right,
                    )?;
                }
                DimensionAnchor::WallPoint { horizontal, .. } => {
                    add_wall_horizontal_anchor_terms(&mut expression, &self.id, *horizontal);
                }
                DimensionAnchor::OpeningPoint {
                    opening,
                    horizontal,
                    ..
                } => {
                    self.add_opening_horizontal_anchor_terms(
                        &mut expression,
                        opening,
                        *horizontal,
                    )?;
                }
            },
            DimensionAxis::Vertical => match anchor {
                DimensionAnchor::WallStart | DimensionAnchor::WallEnd => {}
                DimensionAnchor::OpeningLeft { opening }
                | DimensionAnchor::OpeningCenter { opening }
                | DimensionAnchor::OpeningRight { opening } => {
                    self.add_opening_vertical_anchor_terms(
                        &mut expression,
                        opening,
                        DimensionVerticalReference::Center,
                    )?;
                }
                DimensionAnchor::WallPoint { vertical, .. } => {
                    add_wall_vertical_anchor_terms(&mut expression, &self.id, *vertical);
                }
                DimensionAnchor::OpeningPoint {
                    opening, vertical, ..
                } => {
                    self.add_opening_vertical_anchor_terms(&mut expression, opening, *vertical)?;
                }
            },
        }
        Some(expression)
    }

    fn add_opening_horizontal_anchor_terms(
        &self,
        expression: &mut LinearExpression,
        opening: &ElementId,
        horizontal: DimensionHorizontalReference,
    ) -> Option<()> {
        if !self
            .openings
            .iter()
            .any(|candidate| candidate.id == *opening)
        {
            return None;
        }

        expression.add_term(opening_constraint_variable(opening, "center-x"), 2);
        match horizontal {
            DimensionHorizontalReference::Left => {
                expression.add_term(opening_constraint_variable(opening, "width"), -1);
            }
            DimensionHorizontalReference::Center => {}
            DimensionHorizontalReference::Right => {
                expression.add_term(opening_constraint_variable(opening, "width"), 1);
            }
        }
        Some(())
    }

    fn add_opening_vertical_anchor_terms(
        &self,
        expression: &mut LinearExpression,
        opening: &ElementId,
        vertical: DimensionVerticalReference,
    ) -> Option<()> {
        if !self
            .openings
            .iter()
            .any(|candidate| candidate.id == *opening)
        {
            return None;
        }

        expression.add_term(opening_constraint_variable(opening, "bottom"), 2);
        match vertical {
            DimensionVerticalReference::Bottom => {}
            DimensionVerticalReference::Center => {
                expression.add_term(opening_constraint_variable(opening, "height"), 1);
            }
            DimensionVerticalReference::Top => {
                expression.add_term(opening_constraint_variable(opening, "height"), 2);
            }
        }
        Some(())
    }

    pub fn apply_driving_dimensions(&mut self) -> bool {
        let dimensions = self
            .dimensions
            .iter()
            .filter(|dimension| dimension.kind == DimensionKind::Driving)
            .cloned()
            .collect::<Vec<_>>();
        self.apply_driving_dimension_set(&dimensions)
    }

    pub fn apply_driving_dimension(&mut self, dimension: &DimensionConstraint) -> bool {
        if dimension.kind != DimensionKind::Driving {
            return false;
        }

        let dimensions = self
            .dimensions
            .iter()
            .filter(|candidate| candidate.kind == DimensionKind::Driving)
            .map(|candidate| {
                if candidate.id == dimension.id {
                    dimension.clone()
                } else {
                    candidate.clone()
                }
            })
            .chain(
                self.dimensions
                    .iter()
                    .all(|candidate| candidate.id != dimension.id)
                    .then(|| dimension.clone()),
            )
            .collect::<Vec<_>>();
        self.apply_driving_dimension_set(&dimensions)
    }

    fn apply_driving_dimension_set(&mut self, dimensions: &[DimensionConstraint]) -> bool {
        if dimensions.is_empty() {
            return false;
        }

        let system = self.driving_constraint_system(dimensions.iter());
        let Some(solution) = system.solve_with_defaults(&self.dimension_current_values()) else {
            return false;
        };
        self.apply_dimension_solution(&solution)
    }

    fn dimension_current_values(&self) -> BTreeMap<ConstraintVariable, Length> {
        let mut values = BTreeMap::new();
        values.insert(wall_constraint_variable(&self.id, "length"), self.length);
        values.insert(wall_constraint_variable(&self.id, "height"), self.height);
        for opening in &self.openings {
            values.insert(
                opening_constraint_variable(&opening.id, "center-x"),
                opening.center,
            );
            values.insert(
                opening_constraint_variable(&opening.id, "width"),
                opening.width,
            );
            values.insert(
                opening_constraint_variable(&opening.id, "bottom"),
                opening.sill_height,
            );
            values.insert(
                opening_constraint_variable(&opening.id, "height"),
                opening.height,
            );
        }
        values
    }

    fn apply_dimension_solution(&mut self, values: &BTreeMap<ConstraintVariable, Length>) -> bool {
        let next_length = values
            .get(&wall_constraint_variable(&self.id, "length"))
            .copied()
            .unwrap_or(self.length);
        if next_length <= Length::ZERO {
            return false;
        }
        let next_height = values
            .get(&wall_constraint_variable(&self.id, "height"))
            .copied()
            .unwrap_or(self.height);
        if next_height <= Length::ZERO {
            return false;
        }

        let mut next_openings = self.openings.clone();
        for opening in &mut next_openings {
            opening.center = values
                .get(&opening_constraint_variable(&opening.id, "center-x"))
                .copied()
                .unwrap_or(opening.center);
            opening.width = values
                .get(&opening_constraint_variable(&opening.id, "width"))
                .copied()
                .unwrap_or(opening.width);
            opening.sill_height = values
                .get(&opening_constraint_variable(&opening.id, "bottom"))
                .copied()
                .unwrap_or(opening.sill_height);
            opening.height = values
                .get(&opening_constraint_variable(&opening.id, "height"))
                .copied()
                .unwrap_or(opening.height);

            if opening.width <= Length::ZERO
                || opening.height <= Length::ZERO
                || opening.left() < Length::ZERO
                || opening.right() > next_length
                || opening.sill_height < Length::ZERO
                || opening.top() > next_height
            {
                return false;
            }
        }

        let changed = self.length != next_length
            || self.height != next_height
            || self.openings != next_openings;
        self.set_length_keep_direction(next_length);
        self.height = next_height;
        self.openings = next_openings;
        changed
    }

    fn set_length_keep_direction(&mut self, length: Length) {
        self.length = length;
        if self.start.y == self.end.y {
            let direction: i64 = if self.end.x >= self.start.x { 1 } else { -1 };
            self.end.x = self.start.x + length * direction;
        } else if self.start.x == self.end.x {
            let direction: i64 = if self.end.y >= self.start.y { 1 } else { -1 };
            self.end.y = self.start.y + length * direction;
        } else {
            self.end = Point2::new(self.start.x + length, self.start.y);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DimensionConstraint {
    pub id: ElementId,
    pub name: String,
    pub kind: DimensionKind,
    #[serde(default, skip_serializing_if = "DimensionAxis::is_horizontal")]
    pub axis: DimensionAxis,
    pub start: DimensionAnchor,
    pub end: DimensionAnchor,
    #[serde(default)]
    pub direction: DimensionDirection,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_offset: Option<Length>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Length>,
}

impl DimensionConstraint {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        kind: DimensionKind,
        start: DimensionAnchor,
        end: DimensionAnchor,
        direction: DimensionDirection,
        value: Option<Length>,
    ) -> Self {
        Self {
            id: ElementId::new(id),
            name: name.into(),
            kind,
            axis: DimensionAxis::Horizontal,
            start,
            end,
            direction,
            line_offset: None,
            value,
        }
    }

    pub fn with_axis(mut self, axis: DimensionAxis) -> Self {
        self.axis = axis;
        self
    }

    pub fn with_line_offset(mut self, line_offset: Length) -> Self {
        self.line_offset = Some(line_offset);
        self
    }

    fn validate(
        &self,
        opening_ids: &BTreeSet<ElementId>,
        wall_length: Length,
        wall_height: Length,
    ) -> Result<(), ModelError> {
        validate_element_id(&self.id)?;
        self.start.validate(opening_ids)?;
        self.end.validate(opening_ids)?;

        if self.start == self.end {
            return Err(ModelError::DimensionReferencesSameAnchor {
                dimension: self.id.clone(),
            });
        }

        match self.kind {
            DimensionKind::Driving => {
                let Some(value) = self.value else {
                    return Err(ModelError::DrivingDimensionMissingValue {
                        dimension: self.id.clone(),
                    });
                };
                let wall_bound = match self.axis {
                    DimensionAxis::Horizontal => wall_length,
                    DimensionAxis::Vertical => wall_height,
                };
                if value <= Length::ZERO || value > wall_bound {
                    return Err(ModelError::InvalidDimensionValue {
                        dimension: self.id.clone(),
                    });
                }
            }
            DimensionKind::Reference => {
                if self.value.is_some() {
                    return Err(ModelError::ReferenceDimensionHasValue {
                        dimension: self.id.clone(),
                    });
                }
            }
        }

        Ok(())
    }

    pub fn references_opening(&self, opening: &ElementId) -> bool {
        self.start.references_opening(opening) || self.end.references_opening(opening)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DimensionKind {
    Driving,
    Reference,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum DimensionAxis {
    #[default]
    Horizontal,
    Vertical,
}

impl DimensionAxis {
    pub fn is_horizontal(&self) -> bool {
        matches!(self, Self::Horizontal)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum DimensionDirection {
    #[default]
    Forward,
    Backward,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DimensionHorizontalReference {
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DimensionVerticalReference {
    Bottom,
    Center,
    Top,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DimensionAnchor {
    WallStart,
    WallEnd,
    OpeningLeft {
        opening: ElementId,
    },
    OpeningCenter {
        opening: ElementId,
    },
    OpeningRight {
        opening: ElementId,
    },
    WallPoint {
        horizontal: DimensionHorizontalReference,
        vertical: DimensionVerticalReference,
    },
    OpeningPoint {
        opening: ElementId,
        horizontal: DimensionHorizontalReference,
        vertical: DimensionVerticalReference,
    },
}

impl DimensionAnchor {
    pub fn coordinate(&self, wall: &Wall, axis: DimensionAxis) -> Option<Length> {
        match axis {
            DimensionAxis::Horizontal => self.local_x(wall),
            DimensionAxis::Vertical => self.local_y(wall),
        }
    }

    pub fn local_x(&self, wall: &Wall) -> Option<Length> {
        match self {
            Self::WallStart => Some(Length::ZERO),
            Self::WallEnd => Some(wall.length),
            Self::OpeningLeft { opening } => wall
                .openings
                .iter()
                .find(|candidate| candidate.id == *opening)
                .map(Opening::left),
            Self::OpeningCenter { opening } => wall
                .openings
                .iter()
                .find(|candidate| candidate.id == *opening)
                .map(|candidate| candidate.center),
            Self::OpeningRight { opening } => wall
                .openings
                .iter()
                .find(|candidate| candidate.id == *opening)
                .map(Opening::right),
            Self::WallPoint { horizontal, .. } => {
                Some(wall_horizontal_coordinate(wall, *horizontal))
            }
            Self::OpeningPoint {
                opening,
                horizontal,
                ..
            } => wall
                .openings
                .iter()
                .find(|candidate| candidate.id == *opening)
                .map(|opening| opening_horizontal_coordinate(opening, *horizontal)),
        }
    }

    pub fn local_y(&self, wall: &Wall) -> Option<Length> {
        match self {
            Self::WallStart | Self::WallEnd => Some(Length::ZERO),
            Self::OpeningLeft { opening }
            | Self::OpeningCenter { opening }
            | Self::OpeningRight { opening } => wall
                .openings
                .iter()
                .find(|candidate| candidate.id == *opening)
                .map(|opening| {
                    opening_vertical_coordinate(opening, DimensionVerticalReference::Center)
                }),
            Self::WallPoint { vertical, .. } => Some(wall_vertical_coordinate(wall, *vertical)),
            Self::OpeningPoint {
                opening, vertical, ..
            } => wall
                .openings
                .iter()
                .find(|candidate| candidate.id == *opening)
                .map(|opening| opening_vertical_coordinate(opening, *vertical)),
        }
    }

    pub fn point(&self, wall: &Wall) -> Option<(Length, Length)> {
        Some((self.local_x(wall)?, self.local_y(wall)?))
    }

    pub fn references_opening(&self, opening_id: &ElementId) -> bool {
        match self {
            Self::OpeningLeft { opening }
            | Self::OpeningCenter { opening }
            | Self::OpeningRight { opening }
            | Self::OpeningPoint { opening, .. } => opening == opening_id,
            Self::WallStart | Self::WallEnd | Self::WallPoint { .. } => false,
        }
    }

    fn validate(&self, opening_ids: &BTreeSet<ElementId>) -> Result<(), ModelError> {
        let opening = match self {
            Self::OpeningLeft { opening }
            | Self::OpeningCenter { opening }
            | Self::OpeningRight { opening }
            | Self::OpeningPoint { opening, .. } => Some(opening),
            Self::WallStart | Self::WallEnd | Self::WallPoint { .. } => None,
        };

        if let Some(opening) = opening
            && !opening_ids.contains(opening)
        {
            return Err(ModelError::DimensionReferencesUnknownOpening {
                opening: opening.clone(),
            });
        }

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WallJoin {
    pub id: ElementId,
    pub name: String,
    pub kind: WallJoinKind,
    pub first_wall: ElementId,
    pub second_wall: ElementId,
    pub point: Point2,
}

impl WallJoin {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        kind: WallJoinKind,
        first_wall: impl Into<String>,
        second_wall: impl Into<String>,
        point: Point2,
    ) -> Self {
        Self {
            id: ElementId::new(id),
            name: name.into(),
            kind,
            first_wall: ElementId::new(first_wall),
            second_wall: ElementId::new(second_wall),
            point,
        }
    }

    pub fn corner(
        id: impl Into<String>,
        name: impl Into<String>,
        first_wall: impl Into<String>,
        second_wall: impl Into<String>,
        point: Point2,
    ) -> Self {
        Self::new(
            id,
            name,
            WallJoinKind::Corner,
            first_wall,
            second_wall,
            point,
        )
    }

    pub fn validate(&self) -> Result<(), ModelError> {
        validate_element_id(&self.id)?;
        validate_element_id(&self.first_wall)?;
        validate_element_id(&self.second_wall)?;
        if self.first_wall == self.second_wall {
            return Err(ModelError::JoinReferencesSameWall {
                join: self.id.clone(),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WallJoinKind {
    Corner,
    EndToEnd,
    Tee,
    Cross,
}

/// Which endpoint of a wall an edit targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WallEnd {
    Start,
    End,
}

/// How a room is used. Drives labelling now and, later, room-type code rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum RoomUsage {
    #[default]
    Unspecified,
    Living,
    Bedroom,
    Bathroom,
    Kitchen,
    Dining,
    Office,
    Hallway,
    Closet,
    Utility,
    Garage,
    Other,
}

impl RoomUsage {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Unspecified => "Unspecified",
            Self::Living => "Living",
            Self::Bedroom => "Bedroom",
            Self::Bathroom => "Bathroom",
            Self::Kitchen => "Kitchen",
            Self::Dining => "Dining",
            Self::Office => "Office",
            Self::Hallway => "Hallway",
            Self::Closet => "Closet",
            Self::Utility => "Utility",
            Self::Garage => "Garage",
            Self::Other => "Other",
        }
    }

    pub const ALL: [Self; 12] = [
        Self::Unspecified,
        Self::Living,
        Self::Bedroom,
        Self::Bathroom,
        Self::Kitchen,
        Self::Dining,
        Self::Office,
        Self::Hallway,
        Self::Closet,
        Self::Utility,
        Self::Garage,
        Self::Other,
    ];
}

/// An authored room. Its identity (id, name, usage) and a `seed` point inside it
/// persist; the boundary, area, and perimeter are derived from the surrounding
/// walls at solve time and are never stored. See
/// `docs/plans/2026-06-18-walls-and-rooms-design.md`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Room {
    pub id: ElementId,
    pub name: String,
    #[serde(default)]
    pub usage: RoomUsage,
    pub level: ElementId,
    /// A point inside the room, used to locate its bounding wall loop.
    pub seed: Point2,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

impl Room {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        usage: RoomUsage,
        level: impl Into<String>,
        seed: Point2,
    ) -> Self {
        Self {
            id: ElementId::new(id),
            name: name.into(),
            usage,
            level: ElementId::new(level),
            seed,
            tags: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Opening {
    pub id: ElementId,
    pub name: String,
    pub kind: OpeningKind,
    pub center: Length,
    pub width: Length,
    pub height: Length,
    pub sill_height: Length,
}

impl Opening {
    pub fn door(
        id: impl Into<String>,
        name: impl Into<String>,
        center: Length,
        width: Length,
        height: Length,
    ) -> Self {
        Self {
            id: ElementId::new(id),
            name: name.into(),
            kind: OpeningKind::Door,
            center,
            width,
            height,
            sill_height: Length::ZERO,
        }
    }

    pub fn window(
        id: impl Into<String>,
        name: impl Into<String>,
        center: Length,
        width: Length,
        height: Length,
        sill_height: Length,
    ) -> Self {
        Self {
            id: ElementId::new(id),
            name: name.into(),
            kind: OpeningKind::Window,
            center,
            width,
            height,
            sill_height,
        }
    }

    pub fn with_kind(mut self, kind: OpeningKind) -> Self {
        self.kind = kind;
        self
    }

    pub fn left(&self) -> Length {
        self.center - self.width / 2
    }

    pub fn right(&self) -> Length {
        self.center + self.width / 2
    }

    pub fn top(&self) -> Length {
        self.sill_height + self.height
    }

    pub fn has_sill(&self) -> bool {
        !matches!(self.kind, OpeningKind::Door | OpeningKind::GarageDoor)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OpeningKind {
    Door,
    Window,
    GarageDoor,
    Skylight,
    Stair,
}

#[derive(Debug, Error)]
pub enum ModelError {
    #[error(
        "element id {id:?} must be non-empty and contain only lowercase letters, digits, or hyphens"
    )]
    InvalidElementId { id: ElementId },
    #[error("element id {id:?} is duplicated")]
    DuplicateElementId { id: ElementId },
    #[error("model must contain at least one level")]
    MissingLevel,
    #[error("wall {wall:?} references unknown level {level:?}")]
    WallReferencesUnknownLevel { wall: ElementId, level: ElementId },
    #[error("wall {wall:?} must have a positive length")]
    InvalidWallLength { wall: ElementId },
    #[error("wall {wall:?} must have a positive height")]
    InvalidWallHeight { wall: ElementId },
    #[error("wall {wall:?} must be an axis-aligned segment with distinct endpoints")]
    InvalidWallPlacement { wall: ElementId },
    #[error("wall {wall:?} length must match its authored segment placement")]
    WallLengthPlacementMismatch { wall: ElementId },
    #[error("opening {opening:?} must have a positive width and height")]
    InvalidOpeningSize { opening: ElementId },
    #[error("opening {opening:?} is outside wall {wall:?}")]
    OpeningOutOfBounds { wall: ElementId, opening: ElementId },
    #[error("opening {opening:?} is taller than wall {wall:?}")]
    OpeningTooTall { wall: ElementId, opening: ElementId },
    #[error("openings {first:?} and {second:?} overlap")]
    OverlappingOpenings { first: ElementId, second: ElementId },
    #[error(
        "driving dimension {dimension:?} is overconstrained: expected offset {expected} from its start anchor but current anchors measure {actual}"
    )]
    OverconstrainedDimension {
        dimension: ElementId,
        expected: Length,
        actual: Length,
    },
    #[error(
        "driving dimension {dimension:?} is unsatisfied: expected offset {expected} from its start anchor but current anchors measure {actual}"
    )]
    UnsatisfiedDrivingDimension {
        dimension: ElementId,
        expected: Length,
        actual: Length,
    },
    #[error("dimension {dimension:?} references the same anchor twice")]
    DimensionReferencesSameAnchor { dimension: ElementId },
    #[error("dimension references unknown opening {opening:?}")]
    DimensionReferencesUnknownOpening { opening: ElementId },
    #[error("driving dimension {dimension:?} must have a positive value within the wall")]
    InvalidDimensionValue { dimension: ElementId },
    #[error("driving dimension {dimension:?} must store its target value")]
    DrivingDimensionMissingValue { dimension: ElementId },
    #[error("reference dimension {dimension:?} must not store a target value")]
    ReferenceDimensionHasValue { dimension: ElementId },
    #[error("wall join {join:?} references unknown wall {wall:?}")]
    JoinReferencesUnknownWall { join: ElementId, wall: ElementId },
    #[error("wall join {join:?} must reference two different walls")]
    JoinReferencesSameWall { join: ElementId },
    #[error("wall join {join:?} point does not connect the referenced wall endpoints")]
    JoinPointDoesNotConnectWalls { join: ElementId },
    #[error("room {room:?} references unknown level {level:?}")]
    RoomReferencesUnknownLevel { room: ElementId, level: ElementId },
    #[error("construction system {system:?} must have at least one layer")]
    SystemHasNoLayers { system: ElementId },
    #[error("construction system {system:?} has a layer with non-positive thickness")]
    InvalidLayerThickness { system: ElementId },
    #[error(
        "construction system {system:?} has a layer whose framing spec presence does not match its Framing function"
    )]
    LayerFramingMismatch { system: ElementId },
    #[error("construction system {system:?} has a framing layer with non-positive spacing")]
    InvalidFramingSpacing { system: ElementId },
    #[error("construction system {system:?} references unknown material {material:?}")]
    LayerReferencesUnknownMaterial {
        system: ElementId,
        material: ElementId,
    },
    #[error("asset hash {hash:?} must be a full lowercase blake3:<hex> content hash")]
    InvalidAssetHash { hash: String },
    #[error("asset {hash:?} must have a non-empty media type")]
    InvalidAssetMediaType { hash: String },
    #[error("asset role mismatch: expected {expected:?}, found {found:?}")]
    AssetRoleMismatch {
        expected: TextureRole,
        found: TextureRole,
    },
    #[error("asset-backed material appearances must use a positive scale")]
    InvalidAssetScale,
    #[error("construction system {system:?} must have exactly one framing layer, found {found}")]
    SystemFramingLayerCount { system: ElementId, found: usize },
    #[error("wall {wall:?} references unknown construction system {system:?}")]
    WallReferencesUnknownSystem { wall: ElementId, system: ElementId },
    #[error("wall {wall:?} references construction system {system:?} which is not a Wall system")]
    WallSystemWrongKind { wall: ElementId, system: ElementId },
    #[error("object family {object:?} must have positive width, depth, and height")]
    InvalidObjectSize { object: ElementId },
    #[error("furnishing instance {instance:?} references unknown level {level:?}")]
    FurnishingInstanceReferencesUnknownLevel {
        instance: ElementId,
        level: ElementId,
    },
    #[error("furnishing instance {instance:?} references unknown family {family:?}")]
    FurnishingInstanceReferencesUnknownFamily {
        instance: ElementId,
        family: ElementId,
    },
    #[error("MEP instance {instance:?} references unknown level {level:?}")]
    MepInstanceReferencesUnknownLevel {
        instance: ElementId,
        level: ElementId,
    },
    #[error("MEP instance {instance:?} references unknown family {family:?}")]
    MepInstanceReferencesUnknownFamily {
        instance: ElementId,
        family: ElementId,
    },
    #[error("roof plane {roof_plane:?} references unknown level {level:?}")]
    RoofPlaneReferencesUnknownLevel {
        roof_plane: ElementId,
        level: ElementId,
    },
    #[error("roof plane {roof_plane:?} references unknown construction system {system:?}")]
    RoofPlaneReferencesUnknownSystem {
        roof_plane: ElementId,
        system: ElementId,
    },
    #[error(
        "roof plane {roof_plane:?} references construction system {system:?} which is not a Roof system"
    )]
    RoofPlaneSystemWrongKind {
        roof_plane: ElementId,
        system: ElementId,
    },
    #[error("roof plane {roof_plane:?} outline must have at least three points")]
    RoofPlaneOutlineTooFewPoints { roof_plane: ElementId },
    #[error("roof plane {roof_plane:?} outline must not be self-intersecting")]
    RoofPlaneOutlineSelfIntersecting { roof_plane: ElementId },
    #[error(
        "roof plane {roof_plane:?} outline must be an implicit ring without duplicate or collinear vertices"
    )]
    RoofPlaneOutlineHasRedundantVertices { roof_plane: ElementId },
    #[error("roof plane {roof_plane:?} eave-edge index is out of range for its outline")]
    RoofPlaneEaveEdgeOutOfRange { roof_plane: ElementId },
    #[error("roof plane {roof_plane:?} slope must have a positive run")]
    RoofPlaneInvalidSlope { roof_plane: ElementId },
    #[error("roof plane {roof_plane:?} eave and rake overhangs must be non-negative")]
    RoofPlaneInvalidOverhang { roof_plane: ElementId },
    #[error(
        "connected roof planes {first:?} and {second:?} must use matching eave and rake overhangs"
    )]
    RoofPlaneConnectedOverhangMismatch { first: ElementId, second: ElementId },
    #[error("ceiling {ceiling:?} references unknown level {level:?}")]
    CeilingReferencesUnknownLevel {
        ceiling: ElementId,
        level: ElementId,
    },
    #[error("ceiling {ceiling:?} references unknown construction system {system:?}")]
    CeilingReferencesUnknownSystem {
        ceiling: ElementId,
        system: ElementId,
    },
    #[error(
        "ceiling {ceiling:?} references construction system {system:?} which is not a Ceiling system"
    )]
    CeilingSystemWrongKind {
        ceiling: ElementId,
        system: ElementId,
    },
    #[error("ceiling {ceiling:?} is sloped, which requires an explicit polygon region")]
    CeilingSlopeRequiresPolygonRegion { ceiling: ElementId },
    #[error("ceiling {ceiling:?} slope low-edge index is out of range for its outline")]
    CeilingSlopeLowEdgeOutOfRange { ceiling: ElementId },
    #[error("ceiling {ceiling:?} slope must have a positive run")]
    CeilingInvalidSlope { ceiling: ElementId },
    #[error("floor deck {floor_deck:?} references unknown level {level:?}")]
    FloorDeckReferencesUnknownLevel {
        floor_deck: ElementId,
        level: ElementId,
    },
    #[error("floor deck {floor_deck:?} references unknown construction system {system:?}")]
    FloorDeckReferencesUnknownSystem {
        floor_deck: ElementId,
        system: ElementId,
    },
    #[error(
        "floor deck {floor_deck:?} references construction system {system:?} which is not a Floor system"
    )]
    FloorDeckSystemWrongKind {
        floor_deck: ElementId,
        system: ElementId,
    },
    #[error("surface region of {element:?} references unknown room {room:?}")]
    SurfaceRegionReferencesUnknownRoom { element: ElementId, room: ElementId },
    #[error("surface region polygon of {element:?} must have at least three points")]
    SurfaceRegionPolygonTooFewPoints { element: ElementId },
    #[error("surface region polygon of {element:?} must not be self-intersecting")]
    SurfaceRegionPolygonSelfIntersecting { element: ElementId },
    #[error(
        "standards rule id {rule:?} must start with a lowercase letter or digit and contain only lowercase letters, digits, hyphens, or dots"
    )]
    StandardsInvalidRuleId { rule: String },
    #[error("standards rule id {rule:?} is duplicated within one pack")]
    StandardsDuplicateRuleId { rule: String },
    #[error("standards stack references pack {pack:?} more than once")]
    StandardsStackDuplicatePack { pack: ElementId },
    #[error("standards stack references unknown pack {pack:?}")]
    StandardsStackReferencesUnknownPack { pack: ElementId },
    #[error("standards waive overlay for {target:?} must include a non-empty reason")]
    StandardsOverlayMissingReason { target: String },
    #[error("standards table rows for rule {rule:?} must be strictly ordered by their natural key")]
    StandardsTableRowsNotStrictlyOrdered { rule: String },
    #[error(
        "standards predicate for rule {rule:?} compares fact {fact} as {expected} but operand is {found}"
    )]
    StandardsPredicateTypeMismatch {
        rule: String,
        fact: String,
        expected: String,
        found: String,
    },
    #[error(
        "standards predicate for rule {rule:?} uses operator {op} with flag fact {fact}; flags only support equality"
    )]
    StandardsPredicateInvalidOperator {
        rule: String,
        fact: String,
        op: String,
    },
    #[error(
        "standards predicate for rule {rule:?} uses fact {fact} from {found_scope} in {expected_scope} scope"
    )]
    StandardsPredicateScopeMismatch {
        rule: String,
        fact: String,
        expected_scope: String,
        found_scope: String,
    },
    #[error(
        "standards check {rule:?} cannot filter opening scope by tags because openings do not carry tags"
    )]
    StandardsOpeningTagsUnsupported { rule: String },
    #[error("braced wall line {braced_wall_line:?} references unknown level {level:?}")]
    BracedWallLineReferencesUnknownLevel {
        braced_wall_line: ElementId,
        level: ElementId,
    },
    #[error("bracing panel {panel:?} on wall {wall:?} must have a positive length")]
    BracingPanelInvalidLength { wall: ElementId, panel: ElementId },
    #[error("bracing panel {panel:?} on wall {wall:?} must fit within the wall length")]
    BracingPanelOutOfBounds { wall: ElementId, panel: ElementId },
}

fn default_levels() -> Vec<Level> {
    vec![Level::new("level-1", "Level 1", Length::ZERO)]
}

fn default_standards_stack() -> (Vec<ElementId>, Vec<StandardsPack>) {
    let pack = StandardsPack::irc_2021_starter();
    (vec![pack.id.clone()], vec![pack])
}

fn default_level_id() -> ElementId {
    ElementId::new("level-1")
}

pub(crate) fn validate_element_id(id: &ElementId) -> Result<(), ModelError> {
    if id.is_valid() {
        Ok(())
    } else {
        Err(ModelError::InvalidElementId { id: id.clone() })
    }
}

pub(crate) fn insert_unique_id(
    ids: &mut BTreeSet<ElementId>,
    id: &ElementId,
) -> Result<(), ModelError> {
    if ids.insert(id.clone()) {
        Ok(())
    } else {
        Err(ModelError::DuplicateElementId { id: id.clone() })
    }
}

fn validate_asset_scale(scale: Length) -> Result<(), ModelError> {
    if scale > Length::ZERO {
        Ok(())
    } else {
        Err(ModelError::InvalidAssetScale)
    }
}

pub fn is_blake3_hash(hash: &str) -> bool {
    let Some(hex) = hash.strip_prefix("blake3:") else {
        return false;
    };
    hex.len() == 64
        && hex
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn is_id_start(value: char) -> bool {
    value.is_ascii_lowercase() || value.is_ascii_digit()
}

fn is_id_continue(value: char) -> bool {
    is_id_start(value) || value == '-'
}

fn wall_constraint_variable(wall: &ElementId, attribute: &str) -> ConstraintVariable {
    ConstraintVariable::new(wall.0.clone(), attribute)
}

fn opening_constraint_variable(opening: &ElementId, attribute: &str) -> ConstraintVariable {
    ConstraintVariable::new(opening.0.clone(), attribute)
}

fn wall_horizontal_coordinate(wall: &Wall, horizontal: DimensionHorizontalReference) -> Length {
    match horizontal {
        DimensionHorizontalReference::Left => Length::ZERO,
        DimensionHorizontalReference::Center => wall.length / 2,
        DimensionHorizontalReference::Right => wall.length,
    }
}

fn wall_vertical_coordinate(wall: &Wall, vertical: DimensionVerticalReference) -> Length {
    match vertical {
        DimensionVerticalReference::Bottom => Length::ZERO,
        DimensionVerticalReference::Center => wall.height / 2,
        DimensionVerticalReference::Top => wall.height,
    }
}

fn opening_horizontal_coordinate(
    opening: &Opening,
    horizontal: DimensionHorizontalReference,
) -> Length {
    match horizontal {
        DimensionHorizontalReference::Left => opening.left(),
        DimensionHorizontalReference::Center => opening.center,
        DimensionHorizontalReference::Right => opening.right(),
    }
}

fn opening_vertical_coordinate(opening: &Opening, vertical: DimensionVerticalReference) -> Length {
    match vertical {
        DimensionVerticalReference::Bottom => opening.sill_height,
        DimensionVerticalReference::Center => opening.sill_height + opening.height / 2,
        DimensionVerticalReference::Top => opening.top(),
    }
}

fn add_wall_horizontal_anchor_terms(
    expression: &mut LinearExpression,
    wall: &ElementId,
    horizontal: DimensionHorizontalReference,
) {
    match horizontal {
        DimensionHorizontalReference::Left => {}
        DimensionHorizontalReference::Center => {
            expression.add_term(wall_constraint_variable(wall, "length"), 1);
        }
        DimensionHorizontalReference::Right => {
            expression.add_term(wall_constraint_variable(wall, "length"), 2);
        }
    }
}

fn add_wall_vertical_anchor_terms(
    expression: &mut LinearExpression,
    wall: &ElementId,
    vertical: DimensionVerticalReference,
) {
    match vertical {
        DimensionVerticalReference::Bottom => {}
        DimensionVerticalReference::Center => {
            expression.add_term(wall_constraint_variable(wall, "height"), 1);
        }
        DimensionVerticalReference::Top => {
            expression.add_term(wall_constraint_variable(wall, "height"), 2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wall_with_window(center: Length, width: Length) -> Wall {
        let code = FramingDefaults::irc_2021_starter();
        let mut wall = Wall::new("wall", "Wall", Length::from_feet(12.0), &code);
        wall.openings.push(Opening::window(
            "window",
            "Window",
            center,
            width,
            Length::from_feet(3.0),
            Length::from_feet(3.0),
        ));
        wall
    }

    fn window_anchor(anchor: WindowAnchor) -> DimensionAnchor {
        let opening = ElementId::new("window");
        match anchor {
            WindowAnchor::Left => DimensionAnchor::OpeningLeft { opening },
            WindowAnchor::Center => DimensionAnchor::OpeningCenter { opening },
            WindowAnchor::Right => DimensionAnchor::OpeningRight { opening },
        }
    }

    fn driving_dimension(
        id: &str,
        start: DimensionAnchor,
        end: DimensionAnchor,
        direction: DimensionDirection,
        value: Length,
    ) -> DimensionConstraint {
        DimensionConstraint::new(
            id,
            id,
            DimensionKind::Driving,
            start,
            end,
            direction,
            Some(value),
        )
    }

    #[derive(Debug, Clone, Copy)]
    enum WindowAnchor {
        Left,
        Center,
        Right,
    }

    /// A model carrying one framing-correct system of each surface kind (roof,
    /// floor, ceiling), referencing the starter library's `mat-spf`.
    fn surface_systems_model() -> BuildingModel {
        let mut model = BuildingModel::new();
        for (id, kind, family) in [
            ("system-roof", SystemKind::Roof, MemberFamily::Rafter),
            ("system-floor", SystemKind::Floor, MemberFamily::FloorJoist),
            (
                "system-ceiling",
                SystemKind::Ceiling,
                MemberFamily::CeilingJoist,
            ),
        ] {
            model.systems.push(ConstructionSystem {
                id: ElementId::new(id),
                name: id.to_owned(),
                kind,
                source: None,
                layers: vec![
                    ConstructionLayer::new(
                        LayerFunction::Framing,
                        "mat-spf",
                        BoardProfile::TwoBySix.nominal_depth(),
                    )
                    .with_framing(FramingSpec {
                        member: BoardProfile::TwoBySix,
                        spacing: Length::from_whole_inches(16),
                        pattern: FramingPattern::Single,
                        member_family: family,
                        cavity_material: None,
                    }),
                ],
            });
        }
        model
    }

    fn rect_outline() -> Vec<Point2> {
        vec![
            Point2::new(Length::ZERO, Length::ZERO),
            Point2::new(Length::from_feet(20.0), Length::ZERO),
            Point2::new(Length::from_feet(20.0), Length::from_feet(12.0)),
            Point2::new(Length::ZERO, Length::from_feet(12.0)),
        ]
    }

    fn sample_roof_plane() -> RoofPlane {
        RoofPlane::new(
            "roof-1",
            "Roof",
            "level-1",
            "system-roof",
            rect_outline(),
            Slope::new(Length::from_whole_inches(4), Length::from_whole_inches(12)),
            0,
            Length::from_feet(8.0),
        )
    }

    #[test]
    fn roof_plane_frame_projects_eave_and_ridge_elevations() {
        // rect_outline is 20ft × 12ft with eave edge 0 (the y=0 side), so the
        // up-slope normal points toward the centroid (+y) and the plane rises 4:12
        // from the 8ft springing over the 12ft (144") run to 96 + 48 = 144".
        let frame = sample_roof_plane().frame().expect("a non-degenerate frame");
        assert!(frame.up_slope().1 > 0.99, "up-slope should point +y");
        assert!(
            (frame.elevation_at(120.0, 0.0) - 96.0).abs() < 1.0e-9,
            "eave at 96\""
        );
        assert!(
            (frame.elevation_at(120.0, 144.0) - 144.0).abs() < 1.0e-9,
            "ridge at 144\""
        );
        assert_eq!(
            frame.plan_point_at(
                Length::from_whole_inches(120),
                Length::from_whole_inches(144)
            ),
            Point2::new(
                Length::from_whole_inches(120),
                Length::from_whole_inches(144)
            )
        );
    }

    #[test]
    fn roof_surface_outline_applies_eave_and_exposed_rake_overhangs() {
        let mut model = BuildingModel::new();
        let plane = RoofPlane::new(
            "roof",
            "Roof",
            "level-1",
            "system-roof-1",
            vec![
                Point2::new(Length::ZERO, Length::ZERO),
                Point2::new(Length::from_feet(12.0), Length::ZERO),
                Point2::new(Length::from_feet(12.0), Length::from_feet(8.0)),
                Point2::new(Length::ZERO, Length::from_feet(8.0)),
            ],
            Slope::new(Length::from_whole_inches(6), Length::from_whole_inches(12)),
            0,
            Length::from_feet(8.0),
        )
        .with_eave_overhang(Length::from_whole_inches(12))
        .with_rake_overhang(Length::from_whole_inches(8));
        model.roof_planes.push(plane.clone());

        assert_eq!(
            model.roof_surface_outline(&plane),
            vec![
                Point2::new(
                    Length::from_whole_inches(-8),
                    Length::from_whole_inches(-12)
                ),
                Point2::new(
                    Length::from_whole_inches(152),
                    Length::from_whole_inches(-12)
                ),
                Point2::new(
                    Length::from_whole_inches(152),
                    Length::from_whole_inches(96)
                ),
                Point2::new(Length::from_whole_inches(-8), Length::from_whole_inches(96)),
            ]
        );

        let mut zero = plane;
        zero.eave_overhang = Length::ZERO;
        zero.rake_overhang = Length::ZERO;
        assert_eq!(model.roof_surface_outline(&zero), zero.outline);
    }

    #[test]
    fn roof_surface_outline_is_winding_independent() {
        let mut model = BuildingModel::new();
        let plane = RoofPlane::new(
            "roof-reversed",
            "Roof reversed",
            "level-1",
            "system-roof-1",
            vec![
                Point2::new(Length::ZERO, Length::from_feet(8.0)),
                Point2::new(Length::from_feet(12.0), Length::from_feet(8.0)),
                Point2::new(Length::from_feet(12.0), Length::ZERO),
                Point2::new(Length::ZERO, Length::ZERO),
            ],
            Slope::new(Length::from_whole_inches(6), Length::from_whole_inches(12)),
            2,
            Length::from_feet(8.0),
        )
        .with_eave_overhang(Length::from_whole_inches(12))
        .with_rake_overhang(Length::from_whole_inches(8));
        model.roof_planes.push(plane.clone());
        let outline = model.roof_surface_outline(&plane);
        let mut xs: Vec<Length> = outline.iter().map(|point| point.x).collect();
        let mut ys: Vec<Length> = outline.iter().map(|point| point.y).collect();
        xs.sort_unstable();
        ys.sort_unstable();
        assert_eq!(xs[0], Length::from_whole_inches(-8));
        assert_eq!(xs[3], Length::from_whole_inches(152));
        assert_eq!(ys[0], Length::from_whole_inches(-12));
        assert_eq!(ys[3], Length::from_whole_inches(96));
    }

    #[test]
    fn roof_surface_outline_keeps_hip_seams_coincident() {
        let p = |x, y| Point2::new(Length::from_feet(x), Length::from_feet(y));
        let slope = Slope::new(Length::from_whole_inches(6), Length::from_whole_inches(12));
        let south = RoofPlane::new(
            "roof-south",
            "South",
            "level-1",
            "system-roof-1",
            vec![p(0.0, 0.0), p(12.0, 0.0), p(8.0, 4.0), p(4.0, 4.0)],
            slope,
            0,
            Length::from_feet(8.0),
        )
        .with_eave_overhang(Length::from_feet(1.0))
        .with_rake_overhang(Length::from_whole_inches(8));
        let east = RoofPlane::new(
            "roof-east",
            "East",
            "level-1",
            "system-roof-1",
            vec![p(12.0, 0.0), p(12.0, 8.0), p(8.0, 4.0)],
            slope,
            0,
            Length::from_feet(8.0),
        )
        .with_eave_overhang(Length::from_feet(1.0))
        .with_rake_overhang(Length::from_whole_inches(8));
        let north = RoofPlane::new(
            "roof-north",
            "North",
            "level-1",
            "system-roof-1",
            vec![p(12.0, 8.0), p(0.0, 8.0), p(4.0, 4.0), p(8.0, 4.0)],
            slope,
            0,
            Length::from_feet(8.0),
        )
        .with_eave_overhang(Length::from_feet(1.0));
        let west = RoofPlane::new(
            "roof-west",
            "West",
            "level-1",
            "system-roof-1",
            vec![p(0.0, 8.0), p(0.0, 0.0), p(4.0, 4.0)],
            slope,
            0,
            Length::from_feet(8.0),
        )
        .with_eave_overhang(Length::from_feet(1.0));
        let mut model = BuildingModel::new();
        model.roof_planes = vec![south.clone(), east.clone(), north, west];

        let south_outline = model.roof_surface_outline(&south);
        let east_outline = model.roof_surface_outline(&east);
        assert_eq!(south_outline[1], east_outline[0]);
        assert_eq!(south_outline[2], east_outline[2]);
    }

    #[test]
    fn roof_surface_outline_keeps_valley_seams_coincident() {
        let p = |x, y| Point2::new(Length::from_feet(x), Length::from_feet(y));
        let slope = Slope::new(Length::from_whole_inches(6), Length::from_whole_inches(12));
        let south = RoofPlane::new(
            "roof-south",
            "South valley field",
            "level-1",
            "system-roof-1",
            vec![p(0.0, 0.0), p(24.0, 0.0), p(24.0, 12.0), p(12.0, 12.0)],
            slope,
            0,
            Length::from_feet(8.0),
        )
        .with_eave_overhang(Length::from_feet(1.0))
        .with_rake_overhang(Length::from_whole_inches(8));
        let west = RoofPlane::new(
            "roof-west",
            "West valley field",
            "level-1",
            "system-roof-1",
            vec![p(0.0, 0.0), p(0.0, 24.0), p(12.0, 24.0), p(12.0, 12.0)],
            slope,
            0,
            Length::from_feet(8.0),
        )
        .with_eave_overhang(Length::from_feet(1.0))
        .with_rake_overhang(Length::from_whole_inches(8));
        let mut model = BuildingModel::new();
        model.roof_planes = vec![south.clone(), west.clone()];

        let south_outline = model.roof_surface_outline(&south);
        let west_outline = model.roof_surface_outline(&west);
        assert_eq!(south_outline[0], west_outline[0]);
        assert_eq!(south_outline[3], west_outline[3]);
    }

    fn simple_gable_model() -> BuildingModel {
        let mut model = BuildingModel::new();
        let defaults = model.framing_defaults();
        let p = |x, y| Point2::new(Length::from_feet(x), Length::from_feet(y));
        let wall = |id: &str, start, end| {
            let mut wall = Wall::new(id, id, Length::from_feet(1.0), &defaults)
                .with_placement("level-1", start, end);
            wall.height = Length::from_feet(8.0);
            wall
        };
        model.walls = vec![
            wall("wall-south", p(0.0, 0.0), p(12.0, 0.0)),
            wall("wall-east", p(12.0, 0.0), p(12.0, 8.0)),
            wall("wall-north", p(12.0, 8.0), p(0.0, 8.0)),
            wall("wall-west", p(0.0, 8.0), p(0.0, 0.0)),
        ];
        let slope = Slope::new(Length::from_whole_inches(6), Length::from_whole_inches(12));
        model.roof_planes = vec![
            RoofPlane::new(
                "roof-south",
                "South",
                "level-1",
                "system-roof-1",
                vec![p(0.0, 0.0), p(12.0, 0.0), p(12.0, 4.0), p(0.0, 4.0)],
                slope,
                0,
                Length::from_feet(8.0),
            ),
            RoofPlane::new(
                "roof-north",
                "North",
                "level-1",
                "system-roof-1",
                vec![p(0.0, 4.0), p(12.0, 4.0), p(12.0, 8.0), p(0.0, 8.0)],
                slope,
                2,
                Length::from_feet(8.0),
            ),
        ];
        model
    }

    #[test]
    fn gable_wall_profiles_derive_only_the_two_exterior_end_walls() {
        let model = simple_gable_model();
        let profiles = model.gable_wall_profiles();
        assert_eq!(profiles.len(), 2);
        for id in ["wall-east", "wall-west"] {
            let profile = profiles.get(&ElementId::new(id)).unwrap();
            assert_eq!(profile.width, Length::from_feet(8.0));
            assert_eq!(profile.peak_x, Length::from_feet(4.0));
            assert_eq!(profile.peak_height(), Length::from_feet(2.0));
            assert_eq!(
                profile.height_at(Length::from_feet(2.0)),
                Length::from_feet(1.0)
            );
        }
        assert!(!profiles.contains_key(&ElementId::new("wall-south")));
        assert!(!profiles.contains_key(&ElementId::new("wall-north")));
    }

    #[test]
    fn gable_profile_uses_bearing_outline_not_rake_overhang() {
        let mut model = simple_gable_model();
        for plane in &mut model.roof_planes {
            plane.eave_overhang = Length::from_feet(1.0);
            plane.rake_overhang = Length::from_feet(2.0);
        }
        let profile = model
            .gable_wall_profiles()
            .remove(&ElementId::new("wall-east"))
            .unwrap();
        assert_eq!(profile.width, Length::from_feet(8.0));
        assert_eq!(profile.peak_height(), Length::from_feet(2.0));
    }

    #[test]
    fn connected_roof_fields_require_one_coherent_overhang_pair() {
        let mut model = simple_gable_model();
        assert_eq!(
            model.connected_roof_plane_ids(&ElementId::new("roof-south")),
            BTreeSet::from([ElementId::new("roof-north"), ElementId::new("roof-south")])
        );
        model.roof_planes[0].eave_overhang = Length::from_feet(1.0);
        assert!(matches!(
            model.validate(),
            Err(ModelError::RoofPlaneConnectedOverhangMismatch { .. })
        ));
    }

    #[test]
    fn incomplete_or_mismatched_roof_fields_do_not_derive_gables() {
        let mut shed = simple_gable_model();
        shed.roof_planes.pop();
        assert!(shed.gable_wall_profiles().is_empty());

        let mut mismatched = simple_gable_model();
        mismatched.roof_planes[1].reference_elevation += Length::from_whole_inches(1);
        assert!(mismatched.gable_wall_profiles().is_empty());
    }

    #[test]
    fn unrelated_degenerate_plane_does_not_suppress_a_valid_gable() {
        let mut model = simple_gable_model();
        let p = |x, y| Point2::new(Length::from_feet(x), Length::from_feet(y));
        model.roof_planes.push(RoofPlane::new(
            "roof-degenerate",
            "In-progress unrelated field",
            "level-1",
            "system-roof-1",
            vec![p(20.0, 20.0), p(20.0, 20.0), p(22.0, 22.0)],
            Slope::new(Length::from_whole_inches(6), Length::from_whole_inches(12)),
            0,
            Length::from_feet(8.0),
        ));
        assert_eq!(model.gable_wall_profiles().len(), 2);
    }

    #[test]
    fn hip_roof_does_not_derive_gable_wall_profiles() {
        let mut model = simple_gable_model();
        let p = |x, y| Point2::new(Length::from_feet(x), Length::from_feet(y));
        let slope = Slope::new(Length::from_whole_inches(6), Length::from_whole_inches(12));
        let roof = |id: &str, outline| {
            RoofPlane::new(
                id,
                id,
                "level-1",
                "system-roof-1",
                outline,
                slope,
                0,
                Length::from_feet(8.0),
            )
        };
        model.roof_planes = vec![
            roof(
                "roof-south",
                vec![p(0.0, 0.0), p(12.0, 0.0), p(8.0, 4.0), p(4.0, 4.0)],
            ),
            roof("roof-east", vec![p(12.0, 0.0), p(12.0, 8.0), p(8.0, 4.0)]),
            roof(
                "roof-north",
                vec![p(12.0, 8.0), p(0.0, 8.0), p(4.0, 4.0), p(8.0, 4.0)],
            ),
            roof("roof-west", vec![p(0.0, 8.0), p(0.0, 0.0), p(4.0, 4.0)]),
        ];

        assert!(model.gable_wall_profiles().is_empty());
    }

    #[test]
    fn ridge_partition_is_not_derived_as_a_gable_wall() {
        let mut model = simple_gable_model();
        let defaults = model.framing_defaults();
        model.walls.push(
            Wall::new(
                "wall-ridge",
                "Ridge partition",
                Length::from_feet(12.0),
                &defaults,
            )
            .with_placement(
                "level-1",
                Point2::new(Length::ZERO, Length::from_feet(4.0)),
                Point2::new(Length::from_feet(12.0), Length::from_feet(4.0)),
            ),
        );
        assert!(
            !model
                .gable_wall_profiles()
                .contains_key(&ElementId::new("wall-ridge"))
        );
    }

    #[test]
    fn roof_plane_frame_none_for_degenerate_outline() {
        let mut plane = sample_roof_plane();
        plane.outline = vec![Point2::new(Length::ZERO, Length::ZERO)];
        assert!(plane.frame().is_none());
    }

    #[test]
    fn ceiling_frame_lifts_a_sloped_polygon_and_is_none_otherwise() {
        // The shared lift A4's meshers will call directly. A sloped polygon ceiling
        // projects exactly like a roof plane; everything else has no frame.
        let sloped = |region| {
            let mut ceiling = Ceiling::new(
                "ceiling-1",
                "Ceiling",
                "level-1",
                "system-ceiling",
                region,
                Length::from_feet(8.0),
            );
            ceiling.slope = Some(CeilingSlope::new(
                Slope::new(Length::from_whole_inches(4), Length::from_whole_inches(12)),
                0,
            ));
            ceiling
        };

        // rect_outline is 20ft × 12ft, low edge 0 (the y=0 side): the surface springs
        // at the 8ft (96") reference and rises 4:12 over the 12ft (144") run to 144".
        let frame = sloped(SurfaceRegion::Polygon(rect_outline()))
            .frame(Length::from_feet(8.0))
            .expect("a sloped polygon ceiling has a frame");
        assert!(
            frame.up_slope().1 > 0.99,
            "up-slope points +y toward the centroid"
        );
        assert!(
            (frame.elevation_at(120.0, 0.0) - 96.0).abs() < 1.0e-9,
            "low edge at the 96\" reference"
        );
        assert!(
            (frame.elevation_at(120.0, 144.0) - 144.0).abs() < 1.0e-9,
            "high edge risen to 144\""
        );

        // A flat ceiling (slope None) has no frame.
        let mut flat = sloped(SurfaceRegion::Polygon(rect_outline()));
        flat.slope = None;
        assert!(flat.frame(Length::from_feet(8.0)).is_none());

        // A sloped ceiling over a Room region (no stable edge order) has no frame.
        assert!(
            sloped(SurfaceRegion::Room(ElementId::new("room-1")))
                .frame(Length::from_feet(8.0))
                .is_none()
        );

        // A degenerate (<3-point) outline has no frame.
        assert!(
            sloped(SurfaceRegion::Polygon(vec![Point2::new(
                Length::ZERO,
                Length::ZERO
            )]))
            .frame(Length::from_feet(8.0))
            .is_none()
        );
    }

    #[test]
    fn surface_finish_material_picks_the_viewer_facing_layer() {
        let spf = || {
            ConstructionLayer::new(
                LayerFunction::Framing,
                "mat-spf",
                BoardProfile::TwoBySix.nominal_depth(),
            )
        };
        let finish = |function, material: &str| {
            ConstructionLayer::new(function, material, Length::from_whole_inches(1))
        };
        let system = |kind, layers| ConstructionSystem {
            id: ElementId::new("s"),
            name: "s".to_owned(),
            kind,
            source: None,
            layers,
        };
        let material_of = |s: &ConstructionSystem, face| {
            s.surface_finish_material(face)
                .map(|id| id.0.clone())
                .unwrap()
        };
        let finished = |s: &ConstructionSystem| material_of(s, AssemblyFace::Finished);

        // Roof: the weather face (the outermost Roofing layer).
        let roof = system(
            SystemKind::Roof,
            vec![spf(), finish(LayerFunction::Roofing, "mat-shingle")],
        );
        assert_eq!(finished(&roof), "mat-shingle");
        // Ceiling: the conditioned-side finish (the innermost CeilingFinish).
        let ceiling = system(
            SystemKind::Ceiling,
            vec![finish(LayerFunction::CeilingFinish, "mat-gwb"), spf()],
        );
        assert_eq!(finished(&ceiling), "mat-gwb");
        // Floor: the conditioned-side finish (the innermost InteriorFinish).
        let floor = system(
            SystemKind::Floor,
            vec![finish(LayerFunction::InteriorFinish, "mat-oak"), spf()],
        );
        assert_eq!(finished(&floor), "mat-oak");

        // Cathedral underside: a roof's *conditioned-side* finish (the innermost
        // CeilingFinish/InteriorFinish), not its weather face. The same system
        // resolves the shingle outward and the drywall on the underside.
        let cathedral_roof = system(
            SystemKind::Roof,
            vec![
                finish(LayerFunction::CeilingFinish, "mat-gwb"),
                spf(),
                finish(LayerFunction::Roofing, "mat-shingle"),
            ],
        );
        assert_eq!(finished(&cathedral_roof), "mat-shingle");
        assert_eq!(
            material_of(&cathedral_roof, AssemblyFace::Underside),
            "mat-gwb"
        );
        // A ceiling/floor is viewed from the conditioned side, so its underside
        // resolves the same innermost finish as its finished face.
        assert_eq!(material_of(&ceiling, AssemblyFace::Underside), "mat-gwb");
        assert_eq!(material_of(&floor, AssemblyFace::Underside), "mat-oak");
        // A roof with no interior finish layer falls back to its innermost layer.
        assert_eq!(material_of(&roof, AssemblyFace::Underside), "mat-spf");

        // Roof without a Roofing layer falls back to the outermost weather/sheathing
        // layer before resorting to the last layer.
        let roof_sheathed = system(
            SystemKind::Roof,
            vec![spf(), finish(LayerFunction::Sheathing, "mat-osb")],
        );
        assert_eq!(finished(&roof_sheathed), "mat-osb");
        let roof_wrb = system(
            SystemKind::Roof,
            vec![spf(), finish(LayerFunction::WeatherBarrier, "mat-felt")],
        );
        assert_eq!(finished(&roof_wrb), "mat-felt");
        // A floor whose only finish is structural sheathing (a bare subfloor).
        let floor_deck = system(
            SystemKind::Floor,
            vec![finish(LayerFunction::Sheathing, "mat-ply"), spf()],
        );
        assert_eq!(finished(&floor_deck), "mat-ply");

        // Fallback: a roof with only framing falls back to its outermost layer.
        let bare = system(SystemKind::Roof, vec![spf()]);
        assert_eq!(finished(&bare), "mat-spf");
        // An empty system resolves to nothing (the caller applies its fallback).
        assert!(
            system(SystemKind::Roof, vec![])
                .surface_finish_material(AssemblyFace::Finished)
                .is_none()
        );
        // Walls pick their face by exposure, not this rule (either face).
        assert!(
            system(SystemKind::Wall, vec![spf()])
                .surface_finish_material(AssemblyFace::Finished)
                .is_none()
        );
        assert!(
            system(SystemKind::Wall, vec![spf()])
                .surface_finish_material(AssemblyFace::Underside)
                .is_none()
        );
    }

    #[test]
    fn roof_plane_is_cathedral_tracks_ceiling_coverage() {
        let plane = sample_roof_plane(); // level-1, 20×12 ft footprint, centroid (10,6)ft
        let ceiling = |region| {
            Ceiling::new(
                "ceiling-1",
                "Ceiling",
                "level-1",
                "system-ceiling",
                region,
                Length::from_whole_inches(12),
            )
        };

        // No ceiling anywhere → cathedral.
        let mut model = surface_systems_model();
        model.roof_planes.push(plane.clone());
        assert!(model.roof_plane_is_cathedral(&plane));

        // A flat ceiling whose region encloses the footprint → not cathedral.
        let mut covered = model.clone();
        covered
            .ceilings
            .push(ceiling(SurfaceRegion::Polygon(rect_outline())));
        assert!(!covered.roof_plane_is_cathedral(&plane));

        // A sloped ceiling that covers the footprint also hides the underside
        // (distinct from the structural tie check, which counts only flat ties).
        let mut sloped = model.clone();
        let mut scissor = ceiling(SurfaceRegion::Polygon(rect_outline()));
        scissor.slope = Some(CeilingSlope::new(
            Slope::new(Length::from_whole_inches(3), Length::from_whole_inches(12)),
            0,
        ));
        sloped.ceilings.push(scissor);
        assert!(!sloped.roof_plane_is_cathedral(&plane));

        // A ceiling whose region misses the footprint centroid → still cathedral.
        let mut elsewhere = model.clone();
        let far = vec![
            Point2::new(Length::from_feet(50.0), Length::from_feet(50.0)),
            Point2::new(Length::from_feet(60.0), Length::from_feet(50.0)),
            Point2::new(Length::from_feet(60.0), Length::from_feet(60.0)),
            Point2::new(Length::from_feet(50.0), Length::from_feet(60.0)),
        ];
        elsewhere
            .ceilings
            .push(ceiling(SurfaceRegion::Polygon(far)));
        assert!(elsewhere.roof_plane_is_cathedral(&plane));

        // A covering ceiling on a different level does not tie this plane's level.
        let mut other_level = model.clone();
        let mut wrong_level = ceiling(SurfaceRegion::Polygon(rect_outline()));
        wrong_level.level = ElementId::new("level-2");
        other_level.ceilings.push(wrong_level);
        assert!(other_level.roof_plane_is_cathedral(&plane));

        // A room-attached ceiling — the common authored path — resolves its outline
        // through the wall graph. Build the 20×12 ft footprint as four walls with a
        // room seeded inside, then cover it via SurfaceRegion::Room.
        let mut roomed = model.clone();
        let corners = rect_outline();
        for i in 0..corners.len() {
            let next = (i + 1) % corners.len();
            roomed.walls.push(
                Wall::new(
                    format!("w-{i}"),
                    "Wall",
                    Length::from_whole_inches(6),
                    &roomed.framing_defaults(),
                )
                .with_placement("level-1", corners[i], corners[next]),
            );
        }
        roomed.rooms.push(Room::new(
            "room-1",
            "Room",
            RoomUsage::default(),
            "level-1",
            Point2::new(Length::from_feet(10.0), Length::from_feet(6.0)),
        ));
        let mut covered_room = roomed.clone();
        covered_room
            .ceilings
            .push(ceiling(SurfaceRegion::Room(ElementId::new("room-1"))));
        assert!(!covered_room.roof_plane_is_cathedral(&plane));
        // The batched classifier agrees with the per-plane form.
        assert_eq!(covered_room.roof_cathedral_flags(), vec![false]);

        // A ceiling pointing at a non-existent room resolves to no outline, so it
        // covers nothing and the plane stays a cathedral.
        let mut dangling = roomed;
        dangling
            .ceilings
            .push(ceiling(SurfaceRegion::Room(ElementId::new("ghost-room"))));
        assert!(dangling.roof_plane_is_cathedral(&plane));
        assert_eq!(dangling.roof_cathedral_flags(), vec![true]);
    }

    #[test]
    fn roof_cathedral_flags_align_per_plane() {
        // Two roof planes with disjoint footprints, one covered by a ceiling and one
        // not, pin that `roof_cathedral_flags` is positional (aligned to
        // `roof_planes`) — the index contract both renderers depend on. A single
        // shared/aliased flag would pass the single-plane tests but fail here.
        let ft = Length::from_feet;
        let footprint = |x0: f64, x1: f64| {
            vec![
                Point2::new(ft(x0), Length::ZERO),
                Point2::new(ft(x1), Length::ZERO),
                Point2::new(ft(x1), ft(12.0)),
                Point2::new(ft(x0), ft(12.0)),
            ]
        };
        let plane = |id: &str, x0: f64, x1: f64| {
            RoofPlane::new(
                id,
                id,
                "level-1",
                "system-roof",
                footprint(x0, x1),
                Slope::new(Length::from_whole_inches(4), Length::from_whole_inches(12)),
                0,
                ft(8.0),
            )
        };

        let mut model = surface_systems_model();
        // Order matters: covered plane first, cathedral plane second.
        model.roof_planes.push(plane("roof-covered", 0.0, 20.0));
        model.roof_planes.push(plane("roof-cathedral", 30.0, 50.0));
        // A ceiling over only the first plane's footprint.
        model.ceilings.push(Ceiling::new(
            "ceiling-1",
            "Ceiling",
            "level-1",
            "system-ceiling",
            SurfaceRegion::Polygon(footprint(0.0, 20.0)),
            Length::from_whole_inches(12),
        ));

        assert_eq!(model.roof_cathedral_flags(), vec![false, true]);
        // The per-plane form agrees index-for-index with the batched form.
        for (plane, &flag) in model
            .roof_planes
            .iter()
            .zip(model.roof_cathedral_flags().iter())
        {
            assert_eq!(model.roof_plane_is_cathedral(plane), flag);
        }
    }

    #[test]
    fn surface_objects_validate_when_well_formed() {
        let mut model = surface_systems_model();
        model
            .roof_planes
            .push(sample_roof_plane().with_eave_overhang(Length::from_whole_inches(12)));
        model.ceilings.push(Ceiling::new(
            "ceiling-1",
            "Ceiling",
            "level-1",
            "system-ceiling",
            SurfaceRegion::Polygon(rect_outline()),
            Length::from_feet(8.0),
        ));
        model.floor_decks.push(FloorDeck::new(
            "deck-1",
            "Deck",
            "level-1",
            "system-floor",
            SurfaceRegion::Polygon(rect_outline()),
        ));

        assert!(model.validate().is_ok());
    }

    #[test]
    fn roof_plane_rejects_wrong_system_kind() {
        let mut model = surface_systems_model();
        let mut roof = sample_roof_plane();
        roof.system = ElementId::new("system-wall-exterior-1");
        model.roof_planes.push(roof);

        assert!(matches!(
            model.validate(),
            Err(ModelError::RoofPlaneSystemWrongKind { .. })
        ));
    }

    #[test]
    fn roof_plane_rejects_unknown_system() {
        let mut model = surface_systems_model();
        let mut roof = sample_roof_plane();
        roof.system = ElementId::new("system-nope");
        model.roof_planes.push(roof);

        assert!(matches!(
            model.validate(),
            Err(ModelError::RoofPlaneReferencesUnknownSystem { .. })
        ));
    }

    #[test]
    fn roof_plane_rejects_unknown_level() {
        let mut model = surface_systems_model();
        let mut roof = sample_roof_plane();
        roof.level = ElementId::new("level-nope");
        model.roof_planes.push(roof);

        assert!(matches!(
            model.validate(),
            Err(ModelError::RoofPlaneReferencesUnknownLevel { .. })
        ));
    }

    #[test]
    fn roof_plane_rejects_too_few_outline_points() {
        let mut model = surface_systems_model();
        let mut roof = sample_roof_plane();
        roof.outline.truncate(2);
        model.roof_planes.push(roof);

        assert!(matches!(
            model.validate(),
            Err(ModelError::RoofPlaneOutlineTooFewPoints { .. })
        ));
    }

    #[test]
    fn roof_plane_rejects_self_intersecting_outline() {
        let mut model = surface_systems_model();
        let mut roof = sample_roof_plane();
        // A bow-tie quad: swap the last two vertices so opposite edges cross.
        roof.outline = vec![
            Point2::new(Length::ZERO, Length::ZERO),
            Point2::new(Length::from_feet(20.0), Length::ZERO),
            Point2::new(Length::ZERO, Length::from_feet(12.0)),
            Point2::new(Length::from_feet(20.0), Length::from_feet(12.0)),
        ];
        model.roof_planes.push(roof);

        assert!(matches!(
            model.validate(),
            Err(ModelError::RoofPlaneOutlineSelfIntersecting { .. })
        ));
    }

    #[test]
    fn roof_plane_rejects_explicitly_closed_or_collinear_outline_vertices() {
        for outline in [
            vec![
                Point2::new(Length::ZERO, Length::ZERO),
                Point2::new(Length::from_feet(20.0), Length::ZERO),
                Point2::new(Length::from_feet(20.0), Length::from_feet(12.0)),
                Point2::new(Length::ZERO, Length::from_feet(12.0)),
                Point2::new(Length::ZERO, Length::ZERO),
            ],
            vec![
                Point2::new(Length::ZERO, Length::ZERO),
                Point2::new(Length::from_feet(10.0), Length::ZERO),
                Point2::new(Length::from_feet(20.0), Length::ZERO),
                Point2::new(Length::from_feet(20.0), Length::from_feet(12.0)),
                Point2::new(Length::ZERO, Length::from_feet(12.0)),
            ],
        ] {
            let mut model = surface_systems_model();
            let mut roof = sample_roof_plane();
            roof.outline = outline;
            model.roof_planes.push(roof);

            assert!(matches!(
                model.validate(),
                Err(ModelError::RoofPlaneOutlineHasRedundantVertices { .. })
            ));
        }
    }

    #[test]
    fn roof_plane_rejects_eave_edge_out_of_range() {
        let mut model = surface_systems_model();
        let mut roof = sample_roof_plane();
        roof.eave_edge = 4; // outline has 4 points → valid indices are 0..=3
        model.roof_planes.push(roof);

        assert!(matches!(
            model.validate(),
            Err(ModelError::RoofPlaneEaveEdgeOutOfRange { .. })
        ));
    }

    #[test]
    fn roof_plane_rejects_nonpositive_slope_run() {
        let mut model = surface_systems_model();
        let mut roof = sample_roof_plane();
        roof.slope = Slope::new(Length::from_whole_inches(4), Length::ZERO);
        model.roof_planes.push(roof);

        assert!(matches!(
            model.validate(),
            Err(ModelError::RoofPlaneInvalidSlope { .. })
        ));
    }

    #[test]
    fn roof_plane_rejects_negative_eave_or_rake_overhang() {
        for (eave_overhang, rake_overhang) in [
            (Length::from_ticks(-1), Length::ZERO),
            (Length::ZERO, Length::from_ticks(-1)),
        ] {
            let mut model = surface_systems_model();
            let mut roof = sample_roof_plane();
            roof.eave_overhang = eave_overhang;
            roof.rake_overhang = rake_overhang;
            model.roof_planes.push(roof);

            assert!(matches!(
                model.validate(),
                Err(ModelError::RoofPlaneInvalidOverhang { .. })
            ));
        }
    }

    /// A ceiling over the standard 20×12 ft polygon with a slope (`low_edge` + run).
    fn sloped_ceiling(region: SurfaceRegion, low_edge: u32, run: Length) -> Ceiling {
        let mut ceiling = Ceiling::new(
            "ceiling-1",
            "Ceiling",
            "level-1",
            "system-ceiling",
            region,
            Length::from_feet(8.0),
        );
        ceiling.slope = Some(CeilingSlope::new(
            Slope::new(Length::from_whole_inches(4), run),
            low_edge,
        ));
        ceiling
    }

    #[test]
    fn sloped_ceiling_validates_with_polygon_region_and_in_range_low_edge() {
        let mut model = surface_systems_model();
        model.ceilings.push(sloped_ceiling(
            SurfaceRegion::Polygon(rect_outline()),
            0,
            Length::from_whole_inches(12),
        ));
        assert!(model.validate().is_ok());
    }

    #[test]
    fn sloped_ceiling_rejects_room_region() {
        // A Room boundary has no stable edge order, so a sloped ceiling needs an
        // explicit polygon. The room exists (so the region reference resolves), and
        // the slope rule — not the unknown-room rule — is what rejects it.
        let mut model = surface_systems_model();
        model.rooms.push(Room::new(
            "room-1",
            "Room",
            RoomUsage::default(),
            "level-1",
            Point2::new(Length::from_feet(10.0), Length::from_feet(6.0)),
        ));
        model.ceilings.push(sloped_ceiling(
            SurfaceRegion::Room(ElementId::new("room-1")),
            0,
            Length::from_whole_inches(12),
        ));
        assert!(matches!(
            model.validate(),
            Err(ModelError::CeilingSlopeRequiresPolygonRegion { .. })
        ));
    }

    #[test]
    fn sloped_ceiling_rejects_low_edge_out_of_range() {
        // rect_outline has 4 points → valid low-edge indices are 0..=3.
        let mut model = surface_systems_model();
        model.ceilings.push(sloped_ceiling(
            SurfaceRegion::Polygon(rect_outline()),
            4,
            Length::from_whole_inches(12),
        ));
        assert!(matches!(
            model.validate(),
            Err(ModelError::CeilingSlopeLowEdgeOutOfRange { .. })
        ));
    }

    #[test]
    fn sloped_ceiling_rejects_nonpositive_run() {
        let mut model = surface_systems_model();
        model.ceilings.push(sloped_ceiling(
            SurfaceRegion::Polygon(rect_outline()),
            0,
            Length::ZERO,
        ));
        assert!(matches!(
            model.validate(),
            Err(ModelError::CeilingInvalidSlope { .. })
        ));
    }

    #[test]
    fn flat_ceiling_keeps_validating_with_a_room_region() {
        // A flat ceiling (slope == None) is unaffected by the new rules: a Room
        // region stays valid (the ceiling tool's common case).
        let mut model = surface_systems_model();
        model.rooms.push(Room::new(
            "room-1",
            "Room",
            RoomUsage::default(),
            "level-1",
            Point2::new(Length::from_feet(10.0), Length::from_feet(6.0)),
        ));
        model.ceilings.push(Ceiling::new(
            "ceiling-1",
            "Ceiling",
            "level-1",
            "system-ceiling",
            SurfaceRegion::Room(ElementId::new("room-1")),
            Length::from_feet(8.0),
        ));
        assert!(model.validate().is_ok());
    }

    #[test]
    fn roof_opening_rejects_nonpositive_size() {
        let mut model = surface_systems_model();
        let mut roof = sample_roof_plane();
        roof.openings.push(RoofOpening::new(
            "skylight-1",
            OpeningKind::Skylight,
            Point2::new(Length::from_feet(10.0), Length::from_feet(6.0)),
            Length::ZERO,
            Length::from_feet(2.0),
        ));
        model.roof_planes.push(roof);

        assert!(matches!(
            model.validate(),
            Err(ModelError::InvalidOpeningSize { .. })
        ));
    }

    #[test]
    fn ceiling_rejects_wrong_system_kind() {
        let mut model = surface_systems_model();
        model.ceilings.push(Ceiling::new(
            "ceiling-1",
            "Ceiling",
            "level-1",
            "system-roof",
            SurfaceRegion::Polygon(rect_outline()),
            Length::from_feet(8.0),
        ));

        assert!(matches!(
            model.validate(),
            Err(ModelError::CeilingSystemWrongKind { .. })
        ));
    }

    #[test]
    fn ceiling_with_room_region_validates_and_rejects_unknown_room() {
        let mut model = surface_systems_model();
        model.rooms.push(Room::new(
            "room-1",
            "Living",
            RoomUsage::Living,
            "level-1",
            Point2::new(Length::from_feet(6.0), Length::from_feet(6.0)),
        ));
        model.ceilings.push(Ceiling::new(
            "ceiling-1",
            "Ceiling",
            "level-1",
            "system-ceiling",
            SurfaceRegion::Room(ElementId::new("room-1")),
            Length::from_feet(8.0),
        ));
        assert!(model.validate().is_ok());

        model.ceilings[0].region = SurfaceRegion::Room(ElementId::new("room-nope"));
        assert!(matches!(
            model.validate(),
            Err(ModelError::SurfaceRegionReferencesUnknownRoom { .. })
        ));
    }

    #[test]
    fn floor_deck_rejects_wrong_system_kind() {
        let mut model = surface_systems_model();
        model.floor_decks.push(FloorDeck::new(
            "deck-1",
            "Deck",
            "level-1",
            "system-ceiling",
            SurfaceRegion::Polygon(rect_outline()),
        ));

        assert!(matches!(
            model.validate(),
            Err(ModelError::FloorDeckSystemWrongKind { .. })
        ));
    }

    #[test]
    fn floor_deck_polygon_region_rejects_too_few_points() {
        let mut model = surface_systems_model();
        model.floor_decks.push(FloorDeck::new(
            "deck-1",
            "Deck",
            "level-1",
            "system-floor",
            SurfaceRegion::Polygon(vec![Point2::new(Length::ZERO, Length::ZERO)]),
        ));

        assert!(matches!(
            model.validate(),
            Err(ModelError::SurfaceRegionPolygonTooFewPoints { .. })
        ));
    }

    #[test]
    fn ceiling_rejects_unknown_level() {
        let mut model = surface_systems_model();
        let mut ceiling = Ceiling::new(
            "ceiling-1",
            "Ceiling",
            "level-nope",
            "system-ceiling",
            SurfaceRegion::Polygon(rect_outline()),
            Length::from_feet(8.0),
        );
        ceiling.level = ElementId::new("level-nope");
        model.ceilings.push(ceiling);

        assert!(matches!(
            model.validate(),
            Err(ModelError::CeilingReferencesUnknownLevel { .. })
        ));
    }

    #[test]
    fn ceiling_rejects_unknown_system() {
        let mut model = surface_systems_model();
        model.ceilings.push(Ceiling::new(
            "ceiling-1",
            "Ceiling",
            "level-1",
            "system-nope",
            SurfaceRegion::Polygon(rect_outline()),
            Length::from_feet(8.0),
        ));

        assert!(matches!(
            model.validate(),
            Err(ModelError::CeilingReferencesUnknownSystem { .. })
        ));
    }

    #[test]
    fn floor_deck_rejects_unknown_level() {
        let mut model = surface_systems_model();
        let mut deck = FloorDeck::new(
            "deck-1",
            "Deck",
            "level-1",
            "system-floor",
            SurfaceRegion::Polygon(rect_outline()),
        );
        deck.level = ElementId::new("level-nope");
        model.floor_decks.push(deck);

        assert!(matches!(
            model.validate(),
            Err(ModelError::FloorDeckReferencesUnknownLevel { .. })
        ));
    }

    #[test]
    fn floor_deck_rejects_unknown_system() {
        let mut model = surface_systems_model();
        model.floor_decks.push(FloorDeck::new(
            "deck-1",
            "Deck",
            "level-1",
            "system-nope",
            SurfaceRegion::Polygon(rect_outline()),
        ));

        assert!(matches!(
            model.validate(),
            Err(ModelError::FloorDeckReferencesUnknownSystem { .. })
        ));
    }

    #[test]
    fn surface_region_polygon_rejects_self_intersection() {
        let mut model = surface_systems_model();
        let bowtie = vec![
            Point2::new(Length::ZERO, Length::ZERO),
            Point2::new(Length::from_feet(10.0), Length::ZERO),
            Point2::new(Length::ZERO, Length::from_feet(10.0)),
            Point2::new(Length::from_feet(10.0), Length::from_feet(10.0)),
        ];
        model.floor_decks.push(FloorDeck::new(
            "deck-1",
            "Deck",
            "level-1",
            "system-floor",
            SurfaceRegion::Polygon(bowtie),
        ));

        assert!(matches!(
            model.validate(),
            Err(ModelError::SurfaceRegionPolygonSelfIntersecting { .. })
        ));
    }

    #[test]
    fn framing_layer_count_rule_applies_to_every_system_kind() {
        let mut model = surface_systems_model();
        // Drop the roof system's only framing layer → zero framing layers.
        let roof_system = model
            .systems
            .iter_mut()
            .find(|system| system.id == ElementId::new("system-roof"))
            .unwrap();
        roof_system.layers = vec![ConstructionLayer::new(
            LayerFunction::Roofing,
            "mat-spf",
            Length::from_whole_inches(1),
        )];

        assert!(matches!(
            model.validate(),
            Err(ModelError::SystemFramingLayerCount { found: 0, .. })
        ));
    }

    #[test]
    fn surface_objects_enforce_global_id_uniqueness() {
        // A nested roof-opening id colliding with a top-level id is rejected
        // (openings share the one global id set).
        let mut model = surface_systems_model();
        let mut roof = sample_roof_plane();
        roof.openings.push(RoofOpening::new(
            "roof-1", // collides with the roof plane's own id
            OpeningKind::Skylight,
            Point2::new(Length::from_feet(10.0), Length::from_feet(6.0)),
            Length::from_feet(2.0),
            Length::from_feet(2.0),
        ));
        model.roof_planes.push(roof);
        assert!(matches!(
            model.validate(),
            Err(ModelError::DuplicateElementId { .. })
        ));

        // Two floor decks sharing an id is rejected.
        let mut model = surface_systems_model();
        for _ in 0..2 {
            model.floor_decks.push(FloorDeck::new(
                "deck-dup",
                "Deck",
                "level-1",
                "system-floor",
                SurfaceRegion::Polygon(rect_outline()),
            ));
        }
        assert!(matches!(
            model.validate(),
            Err(ModelError::DuplicateElementId { .. })
        ));
    }

    #[test]
    fn exposure_is_kind_aware() {
        fn system_with(kind: SystemKind, functions: &[LayerFunction]) -> ConstructionSystem {
            ConstructionSystem {
                id: ElementId::new("system-exposure"),
                name: "Exposure".to_owned(),
                kind,
                source: None,
                layers: functions
                    .iter()
                    .map(|&function| {
                        ConstructionLayer::new(function, "mat-x", Length::from_whole_inches(1))
                    })
                    .collect(),
            }
        }

        // A roof's weather face is Roofing/Underlayment → Exterior.
        assert_eq!(
            system_with(
                SystemKind::Roof,
                &[LayerFunction::Roofing, LayerFunction::Sheathing]
            )
            .exposure(),
            WallExposure::Exterior
        );
        // Floors and ceilings have no weather face in v1 → always Interior, even
        // carrying outboard roles a wall would treat as exterior.
        assert_eq!(
            system_with(
                SystemKind::Floor,
                &[LayerFunction::Roofing, LayerFunction::Cladding]
            )
            .exposure(),
            WallExposure::Interior
        );
        assert_eq!(
            system_with(
                SystemKind::Ceiling,
                &[LayerFunction::CeilingFinish, LayerFunction::Sheathing]
            )
            .exposure(),
            WallExposure::Interior
        );
        // Regression: the wall branch is unchanged — a cladding/barrier layer is
        // Exterior, an all-interior stack is Interior.
        assert_eq!(
            system_with(
                SystemKind::Wall,
                &[LayerFunction::InteriorFinish, LayerFunction::Cladding]
            )
            .exposure(),
            WallExposure::Exterior
        );
        assert_eq!(
            system_with(SystemKind::Wall, &[LayerFunction::InteriorFinish]).exposure(),
            WallExposure::Interior
        );
    }

    #[test]
    fn polygon_self_intersection_detects_bowtie_not_simple_rect() {
        assert!(!polygon_self_intersects(&rect_outline()));
        let bowtie = vec![
            Point2::new(Length::ZERO, Length::ZERO),
            Point2::new(Length::from_feet(10.0), Length::ZERO),
            Point2::new(Length::ZERO, Length::from_feet(10.0)),
            Point2::new(Length::from_feet(10.0), Length::from_feet(10.0)),
        ];
        assert!(polygon_self_intersects(&bowtie));
    }

    #[test]
    fn polygon_self_intersection_tolerates_explicitly_closed_ring() {
        // A ring whose last vertex repeats the first (explicit closure) is a
        // simple rectangle, not a self-intersection.
        let mut closed = rect_outline();
        closed.push(closed[0]);
        assert!(!polygon_self_intersects(&closed));
    }

    #[test]
    fn opening_validation_rejects_out_of_bounds() {
        let code = FramingDefaults::irc_2021_starter();
        let mut wall = Wall::new("wall", "Wall", Length::from_feet(8.0), &code);
        wall.openings.push(Opening::door(
            "door",
            "Door",
            Length::from_inches(8.0),
            Length::from_inches(36.0),
            Length::from_inches(80.0),
        ));

        assert!(matches!(
            wall.validate(),
            Err(ModelError::OpeningOutOfBounds { .. })
        ));
    }

    #[test]
    fn model_validation_rejects_duplicate_ids() {
        let code = FramingDefaults::irc_2021_starter();
        let mut model = BuildingModel::new();
        model.walls.push(Wall::new(
            "wall",
            "First wall",
            Length::from_feet(8.0),
            &code,
        ));
        model.walls.push(Wall::new(
            "wall",
            "Second wall",
            Length::from_feet(8.0),
            &code,
        ));

        assert!(matches!(
            model.validate(),
            Err(ModelError::DuplicateElementId { .. })
        ));
    }

    #[test]
    fn model_validation_rejects_duplicate_standards_stack_entry() {
        let mut model = BuildingModel::new();
        model.standards.push(model.standards[0].clone());

        assert!(matches!(
            model.validate(),
            Err(ModelError::StandardsStackDuplicatePack { pack })
                if pack == ElementId::new("std-irc-2021")
        ));
    }

    #[test]
    fn model_validation_rejects_unknown_standards_stack_entry() {
        let mut model = BuildingModel::new();
        model.standards.push(ElementId::new("std-missing"));

        assert!(matches!(
            model.validate(),
            Err(ModelError::StandardsStackReferencesUnknownPack { pack })
                if pack == ElementId::new("std-missing")
        ));
    }

    #[test]
    fn framing_defaults_use_last_resolvable_standards_pack() {
        let mut model = BuildingModel::new();
        let mut overlay = StandardsPack::irc_2021_starter();
        overlay.id = ElementId::new("std-local-overlay");
        overlay.name = "Local overlay".to_owned();
        overlay.tables.defaults.default_wall_height = Length::from_feet(9.0);
        overlay.tables.defaults.stud_profile = BoardProfile::TwoBySix;

        model.standards.push(overlay.id.clone());
        model.standards_packs.push(overlay);

        model.validate().unwrap();
        let defaults = model.framing_defaults();
        assert_eq!(defaults.default_wall_height, Length::from_feet(9.0));
        assert_eq!(defaults.stud_profile, BoardProfile::TwoBySix);
    }

    #[test]
    fn base_standards_name_uses_first_resolvable_standards_pack() {
        let mut model = BuildingModel::new();
        let base_name = model.standards_packs[0].name.clone();
        let mut overlay = StandardsPack::irc_2021_starter();
        overlay.id = ElementId::new("std-local-overlay");
        overlay.name = "Local overlay".to_owned();

        model.standards.push(overlay.id.clone());
        model.standards_packs.push(overlay);

        model.validate().unwrap();
        assert_eq!(model.base_standards_name(), Some(base_name.as_str()));
    }

    #[test]
    fn model_validation_treats_standards_pack_ids_as_global_ids() {
        let mut model = BuildingModel::new();
        model.standards[0] = ElementId::new("level-1");
        model.standards_packs[0].id = ElementId::new("level-1");

        assert!(matches!(
            model.validate(),
            Err(ModelError::DuplicateElementId { id }) if id == ElementId::new("level-1")
        ));
    }

    #[test]
    fn model_validation_rejects_braced_wall_line_unknown_level() {
        let mut model = BuildingModel::new();
        model.braced_wall_lines.push(BracedWallLine {
            id: ElementId::new("bwl-front"),
            name: "Front braced wall line".to_owned(),
            level: ElementId::new("level-missing"),
            start: Point2::new(Length::ZERO, Length::ZERO),
            end: Point2::new(Length::from_feet(10.0), Length::ZERO),
        });

        assert!(matches!(
            model.validate(),
            Err(ModelError::BracedWallLineReferencesUnknownLevel { braced_wall_line, level })
                if braced_wall_line == ElementId::new("bwl-front")
                    && level == ElementId::new("level-missing")
        ));
    }

    #[test]
    fn model_validation_rejects_invalid_bracing_panel_length() {
        let code = FramingDefaults::irc_2021_starter();
        let mut model = BuildingModel::new();
        let mut wall = Wall::new("wall", "Wall", Length::from_feet(8.0), &code);
        wall.bracing.push(BracedPanel {
            id: ElementId::new("panel-zero"),
            offset: Length::ZERO,
            length: Length::ZERO,
            method: crate::standards::BracingMethod::Wsp,
        });
        model.walls.push(wall);

        assert!(matches!(
            model.validate(),
            Err(ModelError::BracingPanelInvalidLength { wall, panel })
                if wall == ElementId::new("wall") && panel == ElementId::new("panel-zero")
        ));
    }

    #[test]
    fn model_validation_rejects_out_of_bounds_bracing_panel() {
        let code = FramingDefaults::irc_2021_starter();
        let mut model = BuildingModel::new();
        let mut wall = Wall::new("wall", "Wall", Length::from_feet(8.0), &code);
        wall.bracing.push(BracedPanel {
            id: ElementId::new("panel-long"),
            offset: Length::from_feet(7.0),
            length: Length::from_feet(2.0),
            method: crate::standards::BracingMethod::Wsp,
        });
        model.walls.push(wall);

        assert!(matches!(
            model.validate(),
            Err(ModelError::BracingPanelOutOfBounds { wall, panel })
                if wall == ElementId::new("wall") && panel == ElementId::new("panel-long")
        ));
    }

    #[test]
    fn deterministic_sort_uses_stable_ids() {
        let code = FramingDefaults::irc_2021_starter();
        let mut model = BuildingModel::new();
        let mut wall_b = Wall::new("wall-b", "B", Length::from_feet(8.0), &code);
        wall_b.openings.push(Opening::window(
            "opening-b",
            "B",
            Length::from_inches(72.0),
            Length::from_inches(24.0),
            Length::from_inches(24.0),
            Length::from_inches(36.0),
        ));
        wall_b.openings.push(Opening::window(
            "opening-a",
            "A",
            Length::from_inches(36.0),
            Length::from_inches(24.0),
            Length::from_inches(24.0),
            Length::from_inches(36.0),
        ));
        model.walls.push(wall_b);
        model
            .walls
            .push(Wall::new("wall-a", "A", Length::from_feet(8.0), &code));

        model.sort_deterministically();

        assert_eq!(model.walls[0].id.0, "wall-a");
        assert_eq!(model.walls[1].openings[0].id.0, "opening-a");
        assert_eq!(model.walls[1].openings[1].id.0, "opening-b");
    }

    #[test]
    fn wall_dimensions_validate_opening_anchors() {
        let code = FramingDefaults::irc_2021_starter();
        let mut wall = Wall::new("wall", "Wall", Length::from_feet(8.0), &code);
        wall.dimensions.push(DimensionConstraint::new(
            "dim",
            "Missing opening dimension",
            DimensionKind::Reference,
            DimensionAnchor::WallStart,
            DimensionAnchor::OpeningCenter {
                opening: ElementId::new("missing-opening"),
            },
            DimensionDirection::Forward,
            None,
        ));

        assert!(matches!(
            wall.validate(),
            Err(ModelError::DimensionReferencesUnknownOpening { .. })
        ));
    }

    #[test]
    fn removing_opening_cascades_dimensions_that_reference_it() {
        let mut wall = wall_with_window(Length::from_feet(4.0), Length::from_feet(3.0));
        wall.openings.push(Opening::window(
            "other-window",
            "Other window",
            Length::from_feet(9.0),
            Length::from_feet(2.0),
            Length::from_feet(3.0),
            Length::from_feet(3.0),
        ));
        wall.dimensions.push(DimensionConstraint::new(
            "window-offset",
            "Window offset",
            DimensionKind::Reference,
            DimensionAnchor::WallStart,
            window_anchor(WindowAnchor::Left),
            DimensionDirection::Forward,
            None,
        ));
        wall.dimensions.push(DimensionConstraint::new(
            "other-offset",
            "Other offset",
            DimensionKind::Reference,
            DimensionAnchor::WallStart,
            DimensionAnchor::OpeningLeft {
                opening: ElementId::new("other-window"),
            },
            DimensionDirection::Forward,
            None,
        ));
        wall.dimensions.push(DimensionConstraint::new(
            "wall-length",
            "Wall length",
            DimensionKind::Reference,
            DimensionAnchor::WallStart,
            DimensionAnchor::WallEnd,
            DimensionDirection::Forward,
            None,
        ));

        assert!(wall.remove_opening(&ElementId::new("window")));

        assert_eq!(
            wall.openings
                .iter()
                .map(|opening| opening.id.0.as_str())
                .collect::<Vec<_>>(),
            vec!["other-window"]
        );
        assert_eq!(
            wall.dimensions
                .iter()
                .map(|dimension| dimension.id.0.as_str())
                .collect::<Vec<_>>(),
            vec!["other-offset", "wall-length"]
        );
        wall.validate().unwrap();
    }

    #[test]
    fn room_validation_rejects_unknown_level() {
        let mut model = BuildingModel::new();
        model.rooms.push(Room::new(
            "room-1",
            "Room",
            RoomUsage::Unspecified,
            "no-such-level",
            Point2::new(Length::from_feet(1.0), Length::from_feet(1.0)),
        ));

        assert!(matches!(
            model.validate(),
            Err(ModelError::RoomReferencesUnknownLevel { .. })
        ));
    }

    fn placed_wall_on_level(
        id: &str,
        level: &str,
        start: Point2,
        end: Point2,
        code: &FramingDefaults,
    ) -> Wall {
        Wall::new(id, id, Length::from_feet(1.0), code).with_placement(level, start, end)
    }

    fn placed_wall(id: &str, start: Point2, end: Point2, code: &FramingDefaults) -> Wall {
        placed_wall_on_level(id, "level-1", start, end, code)
    }

    #[test]
    fn tee_join_validates_when_partition_meets_through_wall_midspan() {
        let code = FramingDefaults::irc_2021_starter();
        let mut model = BuildingModel::new();
        model.walls.push(placed_wall(
            "through",
            Point2::new(Length::ZERO, Length::ZERO),
            Point2::new(Length::from_feet(20.0), Length::ZERO),
            &code,
        ));
        model.walls.push(placed_wall(
            "partition",
            Point2::new(Length::from_feet(10.0), Length::ZERO),
            Point2::new(Length::from_feet(10.0), Length::from_feet(8.0)),
            &code,
        ));
        model.wall_joins.push(WallJoin::new(
            "join-tee",
            "Tee",
            WallJoinKind::Tee,
            "through",
            "partition",
            Point2::new(Length::from_feet(10.0), Length::ZERO),
        ));

        model.validate().unwrap();
    }

    #[test]
    fn cross_join_validates_when_point_interior_to_both() {
        let code = FramingDefaults::irc_2021_starter();
        let mut model = BuildingModel::new();
        model.walls.push(placed_wall(
            "horizontal",
            Point2::new(Length::ZERO, Length::from_feet(4.0)),
            Point2::new(Length::from_feet(20.0), Length::from_feet(4.0)),
            &code,
        ));
        model.walls.push(placed_wall(
            "vertical",
            Point2::new(Length::from_feet(10.0), Length::ZERO),
            Point2::new(Length::from_feet(10.0), Length::from_feet(8.0)),
            &code,
        ));
        model.wall_joins.push(WallJoin::new(
            "join-cross",
            "Cross",
            WallJoinKind::Cross,
            "horizontal",
            "vertical",
            Point2::new(Length::from_feet(10.0), Length::from_feet(4.0)),
        ));

        model.validate().unwrap();
    }

    fn rp(x: f64, y: f64) -> Point2 {
        Point2::new(Length::from_feet(x), Length::from_feet(y))
    }

    #[test]
    fn reconcile_creates_corner_for_shared_endpoint() {
        let code = FramingDefaults::irc_2021_starter();
        let mut model = BuildingModel::new();
        model
            .walls
            .push(placed_wall("a", rp(0.0, 0.0), rp(10.0, 0.0), &code));
        model
            .walls
            .push(placed_wall("b", rp(10.0, 0.0), rp(10.0, 8.0), &code));

        model.reconcile_joins();

        assert_eq!(model.wall_joins.len(), 1);
        assert_eq!(model.wall_joins[0].kind, WallJoinKind::Corner);
        assert_eq!(model.wall_joins[0].point, rp(10.0, 0.0));
        model.validate().unwrap();
    }

    #[test]
    fn extend_collinear_wall_moves_start_and_preserves_wall_local_content() {
        let code = FramingDefaults::irc_2021_starter();
        let mut model = BuildingModel::new();
        let mut wall = placed_wall("existing", rp(10.0, 0.0), rp(0.0, 0.0), &code);
        wall.openings.push(Opening::window(
            "window",
            "Window",
            Length::from_feet(3.0),
            Length::from_feet(2.0),
            Length::from_feet(3.0),
            Length::from_feet(3.0),
        ));
        wall.bracing.push(BracedPanel {
            id: ElementId::new("panel"),
            offset: Length::from_feet(2.0),
            length: Length::from_feet(4.0),
            method: crate::BracingMethod::Wsp,
        });
        let opening_world_point = wall.point_at_local_x(wall.openings[0].center);
        let bracing_world_point = wall.point_at_local_x(wall.bracing[0].offset);
        model.walls.push(wall);

        let extended =
            model.extend_collinear_wall(&ElementId::new("level-1"), rp(10.0, 0.0), rp(20.0, 0.0));

        assert_eq!(extended, Some(ElementId::new("existing")));
        let wall = &model.walls[0];
        assert_eq!(wall.start, rp(20.0, 0.0));
        assert_eq!(wall.end, rp(0.0, 0.0));
        assert_eq!(wall.length, Length::from_feet(20.0));
        assert_eq!(
            wall.point_at_local_x(wall.openings[0].center),
            opening_world_point,
            "moving the local start must not move an existing opening"
        );
        assert_eq!(
            wall.point_at_local_x(wall.bracing[0].offset),
            bracing_world_point,
            "moving the local start must not move an existing braced panel"
        );
        model.validate().unwrap();
    }

    #[test]
    fn extend_collinear_wall_moves_end_without_shifting_local_content() {
        let code = FramingDefaults::irc_2021_starter();
        let mut model = BuildingModel::new();
        let mut wall = placed_wall("existing", rp(0.0, 0.0), rp(0.0, 10.0), &code);
        wall.openings.push(Opening::window(
            "window",
            "Window",
            Length::from_feet(3.0),
            Length::from_feet(2.0),
            Length::from_feet(3.0),
            Length::from_feet(3.0),
        ));
        model.walls.push(wall);

        let extended =
            model.extend_collinear_wall(&ElementId::new("level-1"), rp(0.0, 20.0), rp(0.0, 10.0));

        assert_eq!(extended, Some(ElementId::new("existing")));
        let wall = &model.walls[0];
        assert_eq!(wall.start, rp(0.0, 0.0));
        assert_eq!(wall.end, rp(0.0, 20.0));
        assert_eq!(wall.length, Length::from_feet(20.0));
        assert_eq!(wall.openings[0].center, Length::from_feet(3.0));
        model.validate().unwrap();
    }

    #[test]
    fn extend_collinear_wall_rejects_overlap_ambiguity_and_driving_conflicts() {
        let code = FramingDefaults::irc_2021_starter();
        let mut model = BuildingModel::new();
        let mut constrained = placed_wall("constrained", rp(0.0, 0.0), rp(10.0, 0.0), &code);
        constrained.dimensions.push(driving_dimension(
            "wall-length",
            DimensionAnchor::WallStart,
            DimensionAnchor::WallEnd,
            DimensionDirection::Forward,
            Length::from_feet(10.0),
        ));
        model.walls.push(constrained);

        assert_eq!(
            model.extend_collinear_wall(&ElementId::new("level-1"), rp(10.0, 0.0), rp(10.0, 0.0),),
            None,
            "a zero-length segment is not a continuation"
        );
        assert_eq!(
            model.extend_collinear_wall(&ElementId::new("level-1"), rp(10.0, 0.0), rp(20.0, 1.0),),
            None,
            "a diagonal segment is not a continuation"
        );

        assert_eq!(
            model.extend_collinear_wall(&ElementId::new("level-1"), rp(10.0, 0.0), rp(20.0, 0.0),),
            None,
            "a draw gesture must not silently override a driving dimension"
        );
        assert_eq!(model.walls[0].length, Length::from_feet(10.0));
        assert_eq!(
            model.extend_collinear_wall(&ElementId::new("level-1"), rp(2.0, 0.0), rp(10.0, 0.0),),
            None,
            "an overlapping stroke is not a continuation"
        );

        model.walls[0].dimensions.clear();
        model.walls.push(placed_wall(
            "wrong-level-continuation",
            rp(10.0, 0.0),
            rp(20.0, 0.0),
            &code,
        ));
        model
            .levels
            .push(Level::new("level-2", "Level 2", Length::from_feet(10.0)));
        model.walls.push(placed_wall_on_level(
            "other-level",
            "level-2",
            rp(10.0, 0.0),
            rp(20.0, 0.0),
            &code,
        ));
        assert_eq!(
            model.extend_collinear_wall(&ElementId::new("level-2"), rp(20.0, 0.0), rp(30.0, 0.0),),
            Some(ElementId::new("other-level")),
            "only the requested level participates in continuation"
        );
        assert_eq!(model.walls[0].length, Length::from_feet(10.0));
        assert_eq!(model.walls[1].length, Length::from_feet(10.0));
    }

    #[test]
    fn extend_collinear_wall_rejects_multiple_candidates_and_other_wall_overlap() {
        let code = FramingDefaults::irc_2021_starter();
        let mut model = BuildingModel::new();
        model
            .walls
            .push(placed_wall("left", rp(0.0, 0.0), rp(10.0, 0.0), &code));
        model
            .walls
            .push(placed_wall("right", rp(20.0, 0.0), rp(30.0, 0.0), &code));

        assert_eq!(
            model.extend_collinear_wall(&ElementId::new("level-1"), rp(10.0, 0.0), rp(20.0, 0.0),),
            None,
            "a bridge between two possible host walls is ambiguous"
        );

        model.walls.clear();
        model
            .walls
            .push(placed_wall("lower", rp(0.0, 0.0), rp(0.0, 10.0), &code));
        model.walls.push(placed_wall(
            "vertical-overlap",
            rp(0.0, 15.0),
            rp(0.0, 25.0),
            &code,
        ));
        assert_eq!(
            model.extend_collinear_wall(&ElementId::new("level-1"), rp(0.0, 10.0), rp(0.0, 20.0),),
            None,
            "a continuation must not create coincident wall runs"
        );
        assert_eq!(model.walls[0].length, Length::from_feet(10.0));
    }

    #[test]
    fn wall_envelope_span_laps_open_corner_by_stable_wall_id() {
        let code = FramingDefaults::irc_2021_starter();
        let mut model = BuildingModel::new();
        model
            .walls
            .push(placed_wall("a", rp(0.0, 0.0), rp(10.0, 0.0), &code));
        model
            .walls
            .push(placed_wall("b", rp(10.0, 0.0), rp(10.0, 8.0), &code));
        model.reconcile_joins();

        let a_half = model
            .system_for(&model.walls[0])
            .unwrap()
            .total_thickness()
            .inches()
            / 2.0;
        let b_half = model
            .system_for(&model.walls[1])
            .unwrap()
            .total_thickness()
            .inches()
            / 2.0;

        assert_eq!(
            model.wall_envelope_span(&model.walls[0]),
            (0.0, Length::from_feet(10.0).inches() + b_half)
        );
        assert_eq!(
            model.wall_envelope_span(&model.walls[1]),
            (a_half, Length::from_feet(8.0).inches())
        );
    }

    #[test]
    fn wall_corner_laps_follow_room_winding_and_counter_lap_upper_plate() {
        let model = BuildingModel::demo_shell();
        let front = model
            .walls
            .iter()
            .find(|wall| wall.id.0 == "wall-front")
            .unwrap();
        let primary = model.wall_framing_span(front);
        let counter = model.wall_counter_lap_framing_span(front);

        assert!(
            primary.0 > Length::ZERO && primary.1 > front.length,
            "the outgoing start butts and the incoming end runs through"
        );
        assert!(
            counter.0 < Length::ZERO && counter.1 < front.length,
            "the upper plate reverses both corner seams"
        );
        assert_ne!(primary, counter);
    }

    #[test]
    fn wall_corner_lap_world_geometry_ignores_authored_wall_direction() {
        let model = BuildingModel::demo_shell();
        let mut reversed = model.clone();
        for wall in &mut reversed.walls {
            std::mem::swap(&mut wall.start, &mut wall.end);
        }

        for wall in &model.walls {
            let reversed_wall = reversed
                .walls
                .iter()
                .find(|candidate| candidate.id == wall.id)
                .unwrap();
            let (start, end) = model.wall_framing_span(wall);
            let (reversed_start, reversed_end) = reversed.wall_framing_span(reversed_wall);
            let mut points = [wall.point_at_local_x(start), wall.point_at_local_x(end)];
            let mut reversed_points = [
                reversed_wall.point_at_local_x(reversed_start),
                reversed_wall.point_at_local_x(reversed_end),
            ];
            points.sort_by_key(|point| (point.x, point.y));
            reversed_points.sort_by_key(|point| (point.x, point.y));
            assert_eq!(points, reversed_points, "{}", wall.id.0);
        }
    }

    #[test]
    fn wall_corner_lap_roles_ignore_vector_and_join_field_order() {
        let model = BuildingModel::demo_shell();
        let mut reordered = model.clone();
        reordered.walls.reverse();
        reordered.wall_joins.reverse();
        for join in &mut reordered.wall_joins {
            std::mem::swap(&mut join.first_wall, &mut join.second_wall);
        }

        for wall in &model.walls {
            let reordered_wall = reordered
                .walls
                .iter()
                .find(|candidate| candidate.id == wall.id)
                .unwrap();
            assert_eq!(
                model.wall_envelope_span(wall),
                reordered.wall_envelope_span(reordered_wall),
                "{} envelope",
                wall.id.0
            );
            assert_eq!(
                model.wall_framing_span(wall),
                reordered.wall_framing_span(reordered_wall),
                "{} framing",
                wall.id.0
            );
            assert_eq!(
                model.wall_counter_lap_framing_span(wall),
                reordered.wall_counter_lap_framing_span(reordered_wall),
                "{} counter lap",
                wall.id.0
            );
        }
    }

    #[test]
    fn wall_corner_lap_uses_the_adjoining_wall_thickness() {
        let code = FramingDefaults::irc_2021_starter();
        let mut model = BuildingModel::new();
        model
            .walls
            .push(placed_wall("a", rp(0.0, 0.0), rp(10.0, 0.0), &code));
        model
            .walls
            .push(placed_wall("b", rp(10.0, 0.0), rp(10.0, 8.0), &code));
        let mut thick = model.system_for(&model.walls[1]).unwrap().clone();
        thick.id = ElementId::new("system-wall-thick");
        thick.layers.last_mut().unwrap().thickness += Length::from_inches(2.0);
        model.walls[1].system = thick.id.clone();
        model.systems.push(thick);
        model.reconcile_joins();

        let a_half = model
            .system_for(&model.walls[0])
            .unwrap()
            .total_thickness()
            .inches()
            / 2.0;
        let b_half = model
            .system_for(&model.walls[1])
            .unwrap()
            .total_thickness()
            .inches()
            / 2.0;
        assert_eq!(
            model.wall_envelope_span(&model.walls[0]),
            (0.0, 120.0 + b_half)
        );
        assert_eq!(model.wall_envelope_span(&model.walls[1]), (a_half, 96.0));
    }

    #[test]
    fn wall_corner_lap_clamps_an_inverted_short_span() {
        let code = FramingDefaults::irc_2021_starter();
        let mut model = BuildingModel::new();
        model
            .walls
            .push(placed_wall("a", rp(0.0, 0.0), rp(10.0, 0.0), &code));
        model
            .walls
            .push(placed_wall("z", rp(10.0, 0.0), rp(10.0, 1.0 / 12.0), &code));
        model.reconcile_joins();

        assert_eq!(model.wall_envelope_span(&model.walls[1]), (0.5, 0.5));
    }

    #[test]
    fn wall_envelope_span_butts_partition_against_through_wall_face() {
        let code = FramingDefaults::irc_2021_starter();
        let mut model = BuildingModel::new();
        model
            .walls
            .push(placed_wall("through", rp(0.0, 0.0), rp(20.0, 0.0), &code));
        model.walls.push(placed_wall(
            "partition",
            rp(10.0, 0.0),
            rp(10.0, 8.0),
            &code,
        ));
        model.reconcile_joins();

        assert_eq!(
            model.wall_envelope_span(&model.walls[0]),
            (0.0, Length::from_feet(20.0).inches())
        );
        let through_half = model
            .system_for(&model.walls[0])
            .unwrap()
            .total_thickness()
            .inches()
            / 2.0;
        assert_eq!(
            model.wall_envelope_span(&model.walls[1]),
            (through_half, Length::from_feet(8.0).inches())
        );
    }

    #[test]
    fn reconcile_creates_tee_with_through_then_partition() {
        let code = FramingDefaults::irc_2021_starter();
        let mut model = BuildingModel::new();
        model
            .walls
            .push(placed_wall("through", rp(0.0, 0.0), rp(20.0, 0.0), &code));
        model.walls.push(placed_wall(
            "partition",
            rp(10.0, 0.0),
            rp(10.0, 8.0),
            &code,
        ));

        model.reconcile_joins();

        assert_eq!(model.wall_joins.len(), 1);
        let join = &model.wall_joins[0];
        assert_eq!(join.kind, WallJoinKind::Tee);
        assert_eq!(join.first_wall, ElementId::new("through"));
        assert_eq!(join.second_wall, ElementId::new("partition"));
        assert_eq!(join.point, rp(10.0, 0.0));
        model.validate().unwrap();
    }

    #[test]
    fn reconcile_detects_cross() {
        let code = FramingDefaults::irc_2021_starter();
        let mut model = BuildingModel::new();
        model.walls.push(placed_wall(
            "horizontal",
            rp(0.0, 4.0),
            rp(20.0, 4.0),
            &code,
        ));
        model
            .walls
            .push(placed_wall("vertical", rp(10.0, 0.0), rp(10.0, 8.0), &code));

        model.reconcile_joins();

        assert_eq!(model.wall_joins.len(), 1);
        assert_eq!(model.wall_joins[0].kind, WallJoinKind::Cross);
        assert_eq!(model.wall_joins[0].point, rp(10.0, 4.0));
        model.validate().unwrap();
    }

    #[test]
    fn reconcile_ignores_cross_level_wall_geometry() {
        let code = FramingDefaults::irc_2021_starter();
        let mut model = BuildingModel::new();
        model
            .levels
            .push(Level::new("level-2", "Level 2", Length::from_feet(10.0)));
        model
            .walls
            .push(placed_wall("through", rp(0.0, 0.0), rp(20.0, 0.0), &code));
        model.walls.push(placed_wall_on_level(
            "corner",
            "level-2",
            rp(20.0, 0.0),
            rp(20.0, 8.0),
            &code,
        ));
        model.walls.push(placed_wall_on_level(
            "tee-partition",
            "level-2",
            rp(5.0, 0.0),
            rp(5.0, 8.0),
            &code,
        ));
        model.walls.push(placed_wall_on_level(
            "crossing",
            "level-2",
            rp(10.0, -4.0),
            rp(10.0, 4.0),
            &code,
        ));

        model.reconcile_joins();

        assert!(model.wall_joins.is_empty());
        model.validate().unwrap();
    }

    #[test]
    fn reconcile_drops_stale_join_when_walls_separate() {
        let code = FramingDefaults::irc_2021_starter();
        let mut model = BuildingModel::new();
        model
            .walls
            .push(placed_wall("a", rp(0.0, 0.0), rp(10.0, 0.0), &code));
        model
            .walls
            .push(placed_wall("b", rp(10.0, 0.0), rp(10.0, 8.0), &code));
        model.reconcile_joins();
        assert_eq!(model.wall_joins.len(), 1);

        // Pull wall b's lower endpoint away so the two walls no longer meet.
        let b = model.walls.iter_mut().find(|w| w.id.0 == "b").unwrap();
        *b = placed_wall("b", rp(15.0, 4.0), rp(15.0, 8.0), &code);
        model.reconcile_joins();

        assert!(model.wall_joins.is_empty());
        model.validate().unwrap();
    }

    #[test]
    fn reconcile_preserves_join_id_across_point_move() {
        let code = FramingDefaults::irc_2021_starter();
        let mut model = BuildingModel::new();
        model
            .walls
            .push(placed_wall("a", rp(0.0, 0.0), rp(10.0, 0.0), &code));
        model
            .walls
            .push(placed_wall("b", rp(10.0, 0.0), rp(10.0, 8.0), &code));
        model.wall_joins.push(WallJoin::corner(
            "join-keep",
            "Hand-named corner",
            "a",
            "b",
            rp(10.0, 0.0),
        ));

        // Move the shared corner of both walls to a new point.
        model.walls[0] = placed_wall("a", rp(0.0, 0.0), rp(12.0, 0.0), &code);
        model.walls[1] = placed_wall("b", rp(12.0, 0.0), rp(12.0, 8.0), &code);
        model.reconcile_joins();

        assert_eq!(model.wall_joins.len(), 1);
        assert_eq!(model.wall_joins[0].id, ElementId::new("join-keep"));
        assert_eq!(model.wall_joins[0].name, "Hand-named corner");
        assert_eq!(model.wall_joins[0].point, rp(12.0, 0.0));
        model.validate().unwrap();
    }

    #[test]
    fn move_wall_endpoint_moves_free_end_and_resyncs_length() {
        let code = FramingDefaults::irc_2021_starter();
        let mut model = BuildingModel::new();
        model
            .walls
            .push(placed_wall("a", rp(0.0, 0.0), rp(10.0, 0.0), &code));

        let affected = model.move_wall_endpoint(&ElementId::new("a"), WallEnd::End, rp(8.0, 0.0));

        assert_eq!(affected, vec![ElementId::new("a")]);
        assert_eq!(model.walls[0].end, rp(8.0, 0.0));
        assert_eq!(model.walls[0].length, Length::from_feet(8.0));
        model.validate().unwrap();
    }

    #[test]
    fn move_wall_endpoint_drags_shared_node_along_collinear_run() {
        let code = FramingDefaults::irc_2021_starter();
        let mut model = BuildingModel::new();
        // A collinear run a—b sharing the node at (10,0); moving it stays ortho.
        model
            .walls
            .push(placed_wall("a", rp(0.0, 0.0), rp(10.0, 0.0), &code));
        model
            .walls
            .push(placed_wall("b", rp(10.0, 0.0), rp(20.0, 0.0), &code));

        let affected = model.move_wall_endpoint(&ElementId::new("a"), WallEnd::End, rp(12.0, 0.0));

        assert_eq!(affected.len(), 2);
        assert_eq!(model.walls[0].end, rp(12.0, 0.0));
        assert_eq!(model.walls[1].start, rp(12.0, 0.0));
        assert_eq!(model.walls[0].length, Length::from_feet(12.0));
        assert_eq!(model.walls[1].length, Length::from_feet(8.0));
        model.validate().unwrap();
    }

    #[test]
    fn translate_wall_moves_both_ends_and_stretches_neighbour() {
        let code = FramingDefaults::irc_2021_starter();
        let mut model = BuildingModel::new();
        // L-corner at (0,0): horizontal `a` and vertical `b`.
        model
            .walls
            .push(placed_wall("a", rp(0.0, 0.0), rp(10.0, 0.0), &code));
        model
            .walls
            .push(placed_wall("b", rp(0.0, 0.0), rp(0.0, 8.0), &code));

        // Slide `a` up by 2ft (perpendicular to itself).
        let affected =
            model.translate_wall(&ElementId::new("a"), Length::ZERO, Length::from_feet(2.0));

        assert_eq!(affected.len(), 2);
        assert_eq!(model.walls[0].start, rp(0.0, 2.0));
        assert_eq!(model.walls[0].end, rp(10.0, 2.0));
        assert_eq!(model.walls[0].length, Length::from_feet(10.0));
        // `b` follows the shared corner up, shortening but staying vertical.
        assert_eq!(model.walls[1].start, rp(0.0, 2.0));
        assert_eq!(model.walls[1].end, rp(0.0, 8.0));
        assert_eq!(model.walls[1].length, Length::from_feet(6.0));
        model.reconcile_joins();
        model.validate().unwrap();
    }

    #[test]
    fn move_wall_endpoint_unknown_wall_is_noop() {
        let code = FramingDefaults::irc_2021_starter();
        let mut model = BuildingModel::new();
        model
            .walls
            .push(placed_wall("a", rp(0.0, 0.0), rp(10.0, 0.0), &code));

        let affected =
            model.move_wall_endpoint(&ElementId::new("missing"), WallEnd::End, rp(8.0, 0.0));

        assert!(affected.is_empty());
        assert_eq!(model.walls[0].end, rp(10.0, 0.0));
    }

    #[test]
    fn tee_join_rejected_when_point_is_a_shared_endpoint() {
        let code = FramingDefaults::irc_2021_starter();
        let mut model = BuildingModel::new();
        // Two walls meeting at a shared endpoint (a corner) but mislabelled Tee.
        model.walls.push(placed_wall(
            "a",
            Point2::new(Length::ZERO, Length::ZERO),
            Point2::new(Length::from_feet(10.0), Length::ZERO),
            &code,
        ));
        model.walls.push(placed_wall(
            "b",
            Point2::new(Length::from_feet(10.0), Length::ZERO),
            Point2::new(Length::from_feet(10.0), Length::from_feet(8.0)),
            &code,
        ));
        model.wall_joins.push(WallJoin::new(
            "join",
            "Bad tee",
            WallJoinKind::Tee,
            "a",
            "b",
            Point2::new(Length::from_feet(10.0), Length::ZERO),
        ));

        assert!(matches!(
            model.validate(),
            Err(ModelError::JoinPointDoesNotConnectWalls { .. })
        ));
    }

    #[test]
    fn removing_wall_cascades_joins_that_reference_it() {
        let mut model = BuildingModel::demo_shell();

        assert!(model.remove_wall(&ElementId::new("wall-front")));

        // The wall and every join touching it are gone; unrelated walls/joins stay.
        assert!(model.walls.iter().all(|wall| wall.id.0 != "wall-front"));
        assert_eq!(model.walls.len(), 3);
        assert!(model.wall_joins.iter().all(|join| {
            join.first_wall.0 != "wall-front" && join.second_wall.0 != "wall-front"
        }));
        assert_eq!(
            model
                .wall_joins
                .iter()
                .map(|join| join.id.0.as_str())
                .collect::<Vec<_>>(),
            vec!["join-back-left", "join-right-back"]
        );
        // The remaining shell is still a valid model.
        model.validate().unwrap();
    }

    #[test]
    fn removing_unknown_wall_is_a_no_op() {
        let mut model = BuildingModel::demo_shell();
        let walls_before = model.walls.len();
        let joins_before = model.wall_joins.len();

        assert!(!model.remove_wall(&ElementId::new("does-not-exist")));

        assert_eq!(model.walls.len(), walls_before);
        assert_eq!(model.wall_joins.len(), joins_before);
    }

    #[test]
    fn driving_dimension_moves_opening_anchor() {
        let code = FramingDefaults::irc_2021_starter();
        let mut wall = Wall::new("wall", "Wall", Length::from_feet(12.0), &code);
        wall.openings.push(Opening::window(
            "window",
            "Window",
            Length::from_feet(4.0),
            Length::from_feet(3.0),
            Length::from_feet(3.0),
            Length::from_feet(3.0),
        ));
        wall.dimensions.push(DimensionConstraint::new(
            "dim",
            "Left offset",
            DimensionKind::Driving,
            DimensionAnchor::WallStart,
            DimensionAnchor::OpeningLeft {
                opening: ElementId::new("window"),
            },
            DimensionDirection::Forward,
            Some(Length::from_feet(5.0)),
        ));

        assert!(wall.apply_driving_dimensions());

        let window = wall
            .openings
            .iter()
            .find(|opening| opening.id.0 == "window")
            .unwrap();
        assert_eq!(window.left(), Length::from_feet(5.0));
        assert_eq!(window.width, Length::from_feet(3.0));
    }

    #[test]
    fn driving_dimension_between_opening_edges_resizes_opening() {
        let code = FramingDefaults::irc_2021_starter();
        let mut wall = Wall::new("wall", "Wall", Length::from_feet(12.0), &code);
        wall.openings.push(Opening::window(
            "window",
            "Window",
            Length::from_feet(4.0),
            Length::from_feet(3.0),
            Length::from_feet(3.0),
            Length::from_feet(3.0),
        ));
        wall.dimensions.push(DimensionConstraint::new(
            "dim",
            "Window width",
            DimensionKind::Driving,
            DimensionAnchor::OpeningLeft {
                opening: ElementId::new("window"),
            },
            DimensionAnchor::OpeningRight {
                opening: ElementId::new("window"),
            },
            DimensionDirection::Forward,
            Some(Length::from_feet(4.0)),
        ));

        assert!(wall.apply_driving_dimensions());

        let window = wall
            .openings
            .iter()
            .find(|opening| opening.id.0 == "window")
            .unwrap();
        assert_eq!(window.width, Length::from_feet(4.0));
        assert_eq!(window.center, Length::from_feet(4.0));
    }

    #[test]
    fn vertical_driving_dimension_moves_opening_top_anchor() {
        let mut wall = wall_with_window(Length::from_feet(4.0), Length::from_feet(3.0));
        wall.dimensions.push(
            DimensionConstraint::new(
                "top-offset",
                "Top offset",
                DimensionKind::Driving,
                DimensionAnchor::WallPoint {
                    horizontal: DimensionHorizontalReference::Left,
                    vertical: DimensionVerticalReference::Bottom,
                },
                DimensionAnchor::OpeningPoint {
                    opening: ElementId::new("window"),
                    horizontal: DimensionHorizontalReference::Center,
                    vertical: DimensionVerticalReference::Top,
                },
                DimensionDirection::Forward,
                Some(Length::from_feet(7.0)),
            )
            .with_axis(DimensionAxis::Vertical),
        );

        assert!(wall.apply_driving_dimensions());

        let window = &wall.openings[0];
        assert_eq!(window.top(), Length::from_feet(7.0));
        assert_eq!(window.height, Length::from_feet(3.0));
        assert_eq!(window.sill_height, Length::from_feet(4.0));
    }

    #[test]
    fn vertical_driving_dimension_between_opening_edges_resizes_opening() {
        let mut wall = wall_with_window(Length::from_feet(4.0), Length::from_feet(3.0));
        wall.dimensions.push(
            DimensionConstraint::new(
                "opening-height",
                "Opening height",
                DimensionKind::Driving,
                DimensionAnchor::OpeningPoint {
                    opening: ElementId::new("window"),
                    horizontal: DimensionHorizontalReference::Center,
                    vertical: DimensionVerticalReference::Bottom,
                },
                DimensionAnchor::OpeningPoint {
                    opening: ElementId::new("window"),
                    horizontal: DimensionHorizontalReference::Center,
                    vertical: DimensionVerticalReference::Top,
                },
                DimensionDirection::Forward,
                Some(Length::from_feet(4.0)),
            )
            .with_axis(DimensionAxis::Vertical),
        );

        assert!(wall.apply_driving_dimensions());

        let window = &wall.openings[0];
        assert_eq!(window.height, Length::from_feet(4.0));
        assert_eq!(window.sill_height, Length::from_feet(3.0));
    }

    #[test]
    fn driving_dimension_can_move_first_anchor_when_second_anchor_is_fixed() {
        let code = FramingDefaults::irc_2021_starter();
        let mut wall = Wall::new("wall", "Wall", Length::from_feet(12.0), &code);
        wall.openings.push(Opening::window(
            "window",
            "Window",
            Length::from_feet(4.0),
            Length::from_feet(3.0),
            Length::from_feet(3.0),
            Length::from_feet(3.0),
        ));
        wall.dimensions.push(DimensionConstraint::new(
            "dim",
            "Center from start",
            DimensionKind::Driving,
            window_anchor(WindowAnchor::Center),
            DimensionAnchor::WallStart,
            DimensionDirection::Backward,
            Some(Length::from_feet(6.0)),
        ));

        assert!(wall.apply_driving_dimensions());

        let window = wall
            .openings
            .iter()
            .find(|opening| opening.id.0 == "window")
            .unwrap();
        assert_eq!(window.center, Length::from_feet(6.0));
    }

    #[test]
    fn wall_validation_rejects_overconstrained_driving_dimensions() {
        let mut wall = wall_with_window(Length::from_feet(5.0), Length::from_feet(4.0));
        wall.dimensions.push(DimensionConstraint::new(
            "left-offset",
            "Left offset",
            DimensionKind::Driving,
            DimensionAnchor::WallStart,
            DimensionAnchor::OpeningLeft {
                opening: ElementId::new("window"),
            },
            DimensionDirection::Forward,
            Some(Length::from_feet(3.0)),
        ));
        wall.dimensions.push(DimensionConstraint::new(
            "right-offset",
            "Right offset",
            DimensionKind::Driving,
            DimensionAnchor::WallStart,
            DimensionAnchor::OpeningRight {
                opening: ElementId::new("window"),
            },
            DimensionDirection::Forward,
            Some(Length::from_feet(7.0)),
        ));
        wall.dimensions.push(DimensionConstraint::new(
            "width",
            "Width",
            DimensionKind::Driving,
            window_anchor(WindowAnchor::Left),
            window_anchor(WindowAnchor::Right),
            DimensionDirection::Forward,
            Some(Length::from_feet(4.0)),
        ));

        wall.apply_driving_dimensions();

        assert!(matches!(
            wall.validate(),
            Err(ModelError::OverconstrainedDimension { .. })
        ));
        assert!(
            wall.dimensions
                .iter()
                .all(|dimension| wall.is_driving_dimension_satisfied(dimension))
        );
    }

    #[test]
    fn paired_edge_offsets_solve_opening_width_and_position_together() {
        let mut wall = wall_with_window(Length::from_feet(6.0), Length::from_feet(3.0));
        wall.dimensions.push(driving_dimension(
            "left-offset",
            DimensionAnchor::WallStart,
            window_anchor(WindowAnchor::Left),
            DimensionDirection::Forward,
            Length::from_feet(5.0),
        ));
        wall.dimensions.push(driving_dimension(
            "right-offset",
            DimensionAnchor::WallStart,
            window_anchor(WindowAnchor::Right),
            DimensionDirection::Forward,
            Length::from_feet(10.0),
        ));

        assert!(wall.apply_driving_dimensions());
        wall.validate().unwrap();

        let window = &wall.openings[0];
        assert_eq!(window.left(), Length::from_feet(5.0));
        assert_eq!(window.right(), Length::from_feet(10.0));
        assert_eq!(window.width, Length::from_feet(5.0));
        assert_eq!(window.center, Length::from_feet(7.5));
    }

    #[test]
    fn paired_edge_offsets_are_valid_but_direct_width_dimension_overconstrains() {
        let mut wall = wall_with_window(Length::from_feet(5.0), Length::from_feet(4.0));
        wall.dimensions.push(driving_dimension(
            "left-offset",
            DimensionAnchor::WallStart,
            window_anchor(WindowAnchor::Left),
            DimensionDirection::Forward,
            Length::from_feet(3.0),
        ));
        wall.dimensions.push(driving_dimension(
            "right-offset",
            DimensionAnchor::WallStart,
            window_anchor(WindowAnchor::Right),
            DimensionDirection::Forward,
            Length::from_feet(7.0),
        ));
        let width = driving_dimension(
            "width",
            window_anchor(WindowAnchor::Left),
            window_anchor(WindowAnchor::Right),
            DimensionDirection::Forward,
            Length::from_feet(4.0),
        );

        wall.validate().unwrap();
        assert!(
            wall.dimensions
                .iter()
                .all(|dimension| wall.is_driving_dimension_satisfied(dimension))
        );
        assert!(wall.would_overconstrain_driving_dimension(&width));

        wall.dimensions.push(width);
        assert!(matches!(
            wall.validate(),
            Err(ModelError::OverconstrainedDimension { .. })
        ));
    }

    #[test]
    fn width_and_one_edge_offset_are_valid_but_second_edge_offset_overconstrains() {
        let mut wall = wall_with_window(Length::from_feet(5.0), Length::from_feet(4.0));
        wall.dimensions.push(driving_dimension(
            "width",
            window_anchor(WindowAnchor::Left),
            window_anchor(WindowAnchor::Right),
            DimensionDirection::Forward,
            Length::from_feet(4.0),
        ));
        wall.dimensions.push(driving_dimension(
            "left-offset",
            DimensionAnchor::WallStart,
            window_anchor(WindowAnchor::Left),
            DimensionDirection::Forward,
            Length::from_feet(3.0),
        ));
        let right_offset = driving_dimension(
            "right-offset",
            DimensionAnchor::WallStart,
            window_anchor(WindowAnchor::Right),
            DimensionDirection::Forward,
            Length::from_feet(7.0),
        );

        wall.validate().unwrap();
        assert!(wall.would_overconstrain_driving_dimension(&right_offset));
    }

    #[test]
    fn reference_dimensions_do_not_participate_in_overconstraint_checks() {
        let mut wall = wall_with_window(Length::from_feet(5.0), Length::from_feet(4.0));
        wall.dimensions.push(DimensionConstraint::new(
            "reference-width",
            "Reference width",
            DimensionKind::Reference,
            window_anchor(WindowAnchor::Left),
            window_anchor(WindowAnchor::Right),
            DimensionDirection::Forward,
            None,
        ));
        let width = driving_dimension(
            "width",
            window_anchor(WindowAnchor::Left),
            window_anchor(WindowAnchor::Right),
            DimensionDirection::Forward,
            Length::from_feet(4.0),
        );

        wall.validate().unwrap();
        assert!(!wall.would_overconstrain_driving_dimension(&width));
    }

    #[test]
    fn duplicate_wall_length_dimension_overconstrains() {
        let code = FramingDefaults::irc_2021_starter();
        let mut wall = Wall::new("wall", "Wall", Length::from_feet(12.0), &code);
        wall.dimensions.push(driving_dimension(
            "length",
            DimensionAnchor::WallStart,
            DimensionAnchor::WallEnd,
            DimensionDirection::Forward,
            Length::from_feet(12.0),
        ));
        let duplicate = driving_dimension(
            "length-copy",
            DimensionAnchor::WallStart,
            DimensionAnchor::WallEnd,
            DimensionDirection::Forward,
            Length::from_feet(12.0),
        );

        wall.validate().unwrap();
        assert!(wall.would_overconstrain_driving_dimension(&duplicate));
    }

    #[test]
    fn reversed_equivalent_dimension_overconstrains() {
        let mut wall = wall_with_window(Length::from_feet(5.0), Length::from_feet(4.0));
        wall.dimensions.push(driving_dimension(
            "left-offset",
            DimensionAnchor::WallStart,
            window_anchor(WindowAnchor::Left),
            DimensionDirection::Forward,
            Length::from_feet(3.0),
        ));
        let reversed = driving_dimension(
            "left-offset-reversed",
            window_anchor(WindowAnchor::Left),
            DimensionAnchor::WallStart,
            DimensionDirection::Backward,
            Length::from_feet(3.0),
        );

        wall.validate().unwrap();
        assert!(wall.would_overconstrain_driving_dimension(&reversed));
    }

    #[test]
    fn new_driving_dimension_can_be_overconstrained_even_when_measured_value_matches() {
        let code = FramingDefaults::irc_2021_starter();
        let mut wall = Wall::new("wall", "Wall", Length::from_feet(12.0), &code);
        wall.openings.push(Opening::window(
            "window",
            "Window",
            Length::from_feet(4.0),
            Length::from_feet(3.0),
            Length::from_feet(3.0),
            Length::from_feet(3.0),
        ));
        wall.dimensions.push(DimensionConstraint::new(
            "center",
            "Center",
            DimensionKind::Driving,
            DimensionAnchor::WallStart,
            window_anchor(WindowAnchor::Center),
            DimensionDirection::Forward,
            Some(Length::from_feet(4.0)),
        ));
        wall.dimensions.push(DimensionConstraint::new(
            "width",
            "Width",
            DimensionKind::Driving,
            DimensionAnchor::OpeningLeft {
                opening: ElementId::new("window"),
            },
            DimensionAnchor::OpeningRight {
                opening: ElementId::new("window"),
            },
            DimensionDirection::Forward,
            Some(Length::from_feet(3.0)),
        ));
        let candidate = DimensionConstraint::new(
            "left-offset",
            "Left offset",
            DimensionKind::Driving,
            DimensionAnchor::WallStart,
            DimensionAnchor::OpeningLeft {
                opening: ElementId::new("window"),
            },
            DimensionDirection::Forward,
            Some(Length::from_feet(2.5)),
        );

        wall.validate().unwrap();
        assert!(wall.would_overconstrain_driving_dimension(&candidate));

        wall.dimensions.push(candidate);
        assert!(matches!(
            wall.validate(),
            Err(ModelError::OverconstrainedDimension { .. })
        ));
    }

    #[test]
    fn layer_r_value_uses_exact_thickness_not_whole_inches() {
        // 5/8" (0.625") of an R/in = 900 material: 0.625 * 16 = 10 ticks; the
        // exact contribution is 900 * 10 / 16 = 562 milli-R, NOT 900 (which
        // whole-inch rounding would yield).
        let drywall = Material::solid_color("mat-drywall", "Drywall", [240, 240, 240])
            .with_r_per_inch_milli(900);
        let thickness = Length::from_inches(0.625);
        assert_eq!(thickness.ticks(), 10);
        assert_eq!(drywall.r_value_milli(thickness), 562);

        // A whole-inch system swatch must agree: one 5/8" drywall layer over a
        // single positive-thickness framing layer contributes the exact 562, not
        // a rounded 900.
        let stud = Material::solid_color("mat-stud", "Stud", [200, 170, 120]);
        let system = ConstructionSystem {
            id: ElementId::new("system-test"),
            name: "Test".to_owned(),
            kind: SystemKind::Wall,
            source: None,
            layers: vec![
                ConstructionLayer::new(LayerFunction::InteriorFinish, "mat-drywall", thickness),
                ConstructionLayer::new(
                    LayerFunction::Framing,
                    "mat-stud",
                    Length::from_whole_inches(4),
                )
                .with_framing(FramingSpec {
                    member: BoardProfile::TwoByFour,
                    spacing: Length::from_whole_inches(16),
                    pattern: FramingPattern::Single,
                    member_family: MemberFamily::Stud,
                    cavity_material: None,
                }),
            ],
        };
        assert_eq!(system.r_value_milli(&[drywall, stud]), 562);
    }
}
