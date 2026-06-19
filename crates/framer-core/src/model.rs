use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    ConstraintSystem, ConstraintVariable, Length, LinearConstraint, LinearExpression, Point2,
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
pub struct BuildingModel {
    pub code: CodeProfile,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub materials: Vec<Material>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub systems: Vec<ConstructionSystem>,
    #[serde(default = "default_levels")]
    pub levels: Vec<Level>,
    pub walls: Vec<Wall>,
    #[serde(default)]
    pub wall_joins: Vec<WallJoin>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rooms: Vec<Room>,
}

impl BuildingModel {
    pub fn new(code: CodeProfile) -> Self {
        let (materials, systems) = Self::starter_library();
        Self {
            code,
            materials,
            systems,
            levels: default_levels(),
            walls: Vec::new(),
            wall_joins: Vec::new(),
            rooms: Vec::new(),
        }
    }

    pub fn demo_wall() -> Self {
        let code = CodeProfile::irc_2021_prescriptive();
        let mut wall = Wall::new("wall-1", "Demo wall", Length::from_feet(28.0), &code);
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
            code,
            materials,
            systems,
            levels: default_levels(),
            walls: vec![wall],
            wall_joins: Vec::new(),
            rooms: Vec::new(),
        }
    }

    pub fn demo_shell() -> Self {
        let code = CodeProfile::irc_2021_prescriptive();
        let mut front = Wall::new("wall-front", "Front wall", Length::from_feet(28.0), &code)
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

        let mut right = Wall::new("wall-right", "Right wall", Length::from_feet(20.0), &code)
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

        let mut back = Wall::new("wall-back", "Back wall", Length::from_feet(28.0), &code)
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

        let mut left = Wall::new("wall-left", "Left wall", Length::from_feet(20.0), &code)
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
            code,
            materials,
            systems,
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
        }
        .into_deterministic()
    }

    /// A 24ft × 16ft shell partitioned into two bedrooms and a living area by
    /// interior walls that meet the exterior (and each other) at tee joins. Used
    /// as the rooms/interior-walls example project.
    pub fn demo_two_bedroom() -> Self {
        let code = CodeProfile::irc_2021_prescriptive();
        let ft = Length::from_feet;
        let wall = |id: &str, name: &str, start: Point2, end: Point2| {
            Wall::new(id, name, ft(1.0), &code).with_placement("level-1", start, end)
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

        let walls = vec![
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
            code,
            materials,
            systems,
            levels: default_levels(),
            walls,
            wall_joins,
            rooms,
        }
        .into_deterministic()
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

        let mut material_lookup = BTreeMap::new();
        for material in &self.materials {
            validate_element_id(&material.id)?;
            insert_unique_id(&mut ids, &material.id)?;
            material_lookup.insert(material.id.clone(), material);
        }

        let mut system_lookup = BTreeMap::new();
        for system in &self.systems {
            system.validate(&material_lookup, &mut ids)?;
            system_lookup.insert(system.id.clone(), system);
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

        for room in &self.rooms {
            validate_element_id(&room.id)?;
            insert_unique_id(&mut ids, &room.id)?;
            if !level_ids.contains(&room.level) {
                return Err(ModelError::RoomReferencesUnknownLevel {
                    room: room.id.clone(),
                    level: room.level.clone(),
                });
            }
        }
        Ok(())
    }

    pub fn sort_deterministically(&mut self) {
        self.materials.sort_by(|left, right| left.id.cmp(&right.id));
        // Systems sort by id; layer ORDER is semantic (interior -> exterior) and
        // must never be reordered.
        self.systems.sort_by(|left, right| left.id.cmp(&right.id));
        self.levels.sort_by(|left, right| left.id.cmp(&right.id));
        self.walls.sort_by(|left, right| left.id.cmp(&right.id));
        for wall in &mut self.walls {
            wall.sort_deterministically();
        }
        self.wall_joins
            .sort_by(|left, right| left.id.cmp(&right.id));
        self.rooms.sort_by(|left, right| left.id.cmp(&right.id));
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

    /// Resolve a material by id from the project library.
    pub fn material(&self, id: &ElementId) -> Option<&Material> {
        self.materials.iter().find(|material| material.id == *id)
    }

    /// The seeded material catalog and construction systems for a new project.
    /// Deterministic (id-sorted on output); shared by `new`, the `demo_*`
    /// constructors, and the app's `new_project`.
    pub fn starter_library() -> (Vec<Material>, Vec<ConstructionSystem>) {
        let materials = vec![
            Material::solid_color("mat-drywall", "5/8\" Gypsum", [228, 226, 220])
                .with_tags(["finish"])
                .with_r_per_inch_milli(900),
            Material::solid_color("mat-spf", "SPF framing", [205, 170, 120])
                .with_tags(["framing"])
                .with_r_per_inch_milli(1250),
            Material::solid_color("mat-mineral-wool", "Mineral wool", [176, 188, 170])
                .with_tags(["insulation"])
                .with_r_per_inch_milli(4200),
            Material::solid_color("mat-plywood", "1/2\" Plywood", [200, 172, 128])
                .with_tags(["sheathing"])
                .with_r_per_inch_milli(1250),
            Material::solid_color("mat-polyiso", "2\" Polyiso", [240, 222, 128])
                .with_tags(["insulation"])
                .with_r_per_inch_milli(6000),
            Material::solid_color("mat-rainscreen", "Rain-screen battens", [150, 110, 76])
                .with_tags(["furring"])
                .with_r_per_inch_milli(0),
            Material::solid_color("mat-fiber-cement", "Fiber-cement lap", [183, 185, 190])
                .with_tags(["cladding"])
                .with_r_per_inch_milli(150),
        ];

        let exterior = ConstructionSystem {
            id: ElementId::new("system-wall-exterior-1"),
            name: "Exterior wall".to_owned(),
            kind: SystemKind::Wall,
            layers: vec![
                ConstructionLayer::new(
                    LayerFunction::InteriorFinish,
                    "mat-drywall",
                    Length::from_inches(0.625),
                ),
                ConstructionLayer::new(
                    LayerFunction::Framing,
                    "mat-spf",
                    Length::from_whole_inches(4),
                )
                .with_framing(FramingSpec {
                    member: BoardProfile::TwoByFour,
                    spacing: Length::from_whole_inches(16),
                    pattern: FramingPattern::Single,
                    cavity_material: Some(ElementId::new("mat-mineral-wool")),
                }),
                ConstructionLayer::new(
                    LayerFunction::Sheathing,
                    "mat-plywood",
                    Length::from_inches(0.5),
                ),
                ConstructionLayer::new(
                    LayerFunction::ContinuousInsulation,
                    "mat-polyiso",
                    Length::from_whole_inches(2),
                ),
                ConstructionLayer::new(
                    LayerFunction::AirGap,
                    "mat-rainscreen",
                    Length::from_inches(0.75),
                ),
                ConstructionLayer::new(
                    LayerFunction::Cladding,
                    "mat-fiber-cement",
                    Length::from_inches(0.3125),
                ),
            ],
        };

        let interior = ConstructionSystem {
            id: ElementId::new("system-wall-interior-1"),
            name: "Interior partition".to_owned(),
            kind: SystemKind::Wall,
            layers: vec![
                ConstructionLayer::new(
                    LayerFunction::InteriorFinish,
                    "mat-drywall",
                    Length::from_inches(0.625),
                ),
                ConstructionLayer::new(
                    LayerFunction::Framing,
                    "mat-spf",
                    Length::from_whole_inches(4),
                )
                .with_framing(FramingSpec {
                    member: BoardProfile::TwoByFour,
                    spacing: Length::from_whole_inches(16),
                    pattern: FramingPattern::Single,
                    cavity_material: None,
                }),
                ConstructionLayer::new(
                    LayerFunction::InteriorFinish,
                    "mat-drywall",
                    Length::from_inches(0.625),
                ),
            ],
        };

        let mut materials = materials;
        materials.sort_by(|left, right| left.id.cmp(&right.id));
        let mut systems = vec![exterior, interior];
        systems.sort_by(|left, right| left.id.cmp(&right.id));
        (materials, systems)
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

/// Every join implied by the walls' current geometry: a `Corner` where two walls
/// share an endpoint, a `Tee` where one wall's endpoint lands on another's
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

/// The single join relationship (if any) between two distinct walls, in priority
/// order: shared endpoint → `Corner`; endpoint-on-interior → `Tee` (through wall
/// first); interior crossing → `Cross`.
fn relate_walls(a: &Wall, b: &Wall) -> Option<DesiredJoin> {
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Level {
    pub id: ElementId,
    pub name: String,
    pub elevation: Length,
}

impl Level {
    pub fn new(id: impl Into<String>, name: impl Into<String>, elevation: Length) -> Self {
        Self {
            id: ElementId::new(id),
            name: name.into(),
            elevation,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeProfile {
    pub code: PrescriptiveCode,
    pub display_name: String,
    pub default_wall_height: Length,
    pub default_stud_spacing: Length,
    pub double_top_plate: bool,
    pub default_header_depth: Length,
    pub stud_profile: BoardProfile,
    pub plate_profile: BoardProfile,
    pub header_profile: BoardProfile,
}

impl CodeProfile {
    pub fn irc_2021_prescriptive() -> Self {
        Self {
            code: PrescriptiveCode::Irc2021,
            display_name: "IRC 2021 prescriptive starter profile".to_owned(),
            default_wall_height: Length::from_feet(8.0),
            default_stud_spacing: Length::from_whole_inches(16),
            double_top_plate: true,
            default_header_depth: Length::from_whole_inches(9),
            stud_profile: BoardProfile::TwoByFour,
            plate_profile: BoardProfile::TwoByFour,
            header_profile: BoardProfile::TwoByTen,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrescriptiveCode {
    Irc2021,
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

/// The structural class of a construction system. A closed enum: the app reasons
/// about each kind (only `Wall` is wired now; `Floor`/`Roof` extend later).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum SystemKind {
    Wall,
    Floor,
    Roof,
}

impl SystemKind {
    pub const ALL: [Self; 3] = [Self::Wall, Self::Floor, Self::Roof];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Wall => "Wall",
            Self::Floor => "Floor",
            Self::Roof => "Roof",
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
    Other,
}

impl LayerFunction {
    pub const ALL: [Self; 10] = [
        Self::InteriorFinish,
        Self::Framing,
        Self::ContinuousInsulation,
        Self::Sheathing,
        Self::WeatherBarrier,
        Self::AirGap,
        Self::Cladding,
        Self::Masonry,
        Self::Structure,
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
            Self::Other => "Other",
        }
    }
}

/// How the framing members in a framing layer are laid out across the cavity.
/// `Staggered`/`Double` are authored now but generation is deferred.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
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
    pub fn new(
        function: LayerFunction,
        material: impl Into<String>,
        thickness: Length,
    ) -> Self {
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

    /// Derived exposure: `Exterior` if any layer is an outboard envelope role
    /// (weather barrier, cladding, masonry, continuous insulation), else
    /// `Interior`.
    pub fn exposure(&self) -> WallExposure {
        let exterior = self.layers.iter().any(|layer| {
            matches!(
                layer.function,
                LayerFunction::WeatherBarrier
                    | LayerFunction::Cladding
                    | LayerFunction::Masonry
                    | LayerFunction::ContinuousInsulation
            )
        });
        if exterior {
            WallExposure::Exterior
        } else {
            WallExposure::Interior
        }
    }

    /// Clear-wall R-value in milli-R (R × 1000), integer math: the sum over layers
    /// of `round(thickness_in_inches) * material.r_per_inch_milli`. The framing
    /// layer additionally contributes its cavity material's R over the framing
    /// depth. This is a clear-wall approximation — it ignores the framing-factor
    /// (parallel-path) derate, which is deferred.
    pub fn r_value_milli(&self, materials: &[Material]) -> i64 {
        let lookup = |id: &ElementId| materials.iter().find(|material| material.id == *id);
        let mut total = 0i64;
        for layer in &self.layers {
            let inches = layer.thickness.inches().round() as i64;
            if let Some(material) = lookup(&layer.material) {
                total += inches * material.r_per_inch_milli();
            }
            if let Some(framing) = &layer.framing
                && let Some(cavity) = &framing.cavity_material
                && let Some(material) = lookup(cavity)
            {
                total += inches * material.r_per_inch_milli();
            }
        }
        total
    }

    fn validate(
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

        if self.kind == SystemKind::Wall && framing_layers != 1 {
            return Err(ModelError::WallSystemFramingLayerCount {
                system: self.id.clone(),
                found: framing_layers,
            });
        }

        Ok(())
    }
}

/// Where a material is defined. `Project` materials are embedded in the model;
/// `External` references a shared/imported library (resolver widens later).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum MaterialSource {
    #[default]
    Project,
    External {
        reference: String,
    },
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

/// Authored finish for a material. Starts as a flat color; the enum is the seam
/// for richer, possibly geometry-affecting finishes, lowered into framer-render's
/// path-tracer material later.
///
/// GROWTH PATH (not built now):
///   - `Textured { color, texture_ref, scale }`
///   - `LappedSiding { color, reveal: Length }` — parametric, may affect geometry
///   - `Masonry { unit, coursing, color }` — depth-mapped brick/block
///   - `DepthMapped { color, height_ref, scale }`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Appearance {
    SolidColor([u8; 3]),
}

impl Appearance {
    /// A representative color for this appearance.
    pub fn color(&self) -> [u8; 3] {
        match self {
            Self::SolidColor(color) => *color,
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
    pub fn solid_color(
        id: impl Into<String>,
        name: impl Into<String>,
        color: [u8; 3],
    ) -> Self {
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
    pub dimensions: Vec<DimensionConstraint>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

impl Wall {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        length: Length,
        code: &CodeProfile,
    ) -> Self {
        Self {
            id: ElementId::new(id),
            name: name.into(),
            level: default_level_id(),
            start: Point2::new(Length::ZERO, Length::ZERO),
            end: Point2::new(length, Length::ZERO),
            length,
            height: code.default_wall_height,
            system: ElementId::new("system-wall-exterior-1"),
            openings: Vec::new(),
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
    #[error("wall construction system {system:?} must have exactly one framing layer, found {found}")]
    WallSystemFramingLayerCount { system: ElementId, found: usize },
    #[error("wall {wall:?} references unknown construction system {system:?}")]
    WallReferencesUnknownSystem { wall: ElementId, system: ElementId },
    #[error("wall {wall:?} references construction system {system:?} which is not a Wall system")]
    WallSystemWrongKind { wall: ElementId, system: ElementId },
}

fn default_levels() -> Vec<Level> {
    vec![Level::new("level-1", "Level 1", Length::ZERO)]
}

fn default_level_id() -> ElementId {
    ElementId::new("level-1")
}

fn validate_element_id(id: &ElementId) -> Result<(), ModelError> {
    if id.is_valid() {
        Ok(())
    } else {
        Err(ModelError::InvalidElementId { id: id.clone() })
    }
}

fn insert_unique_id(ids: &mut BTreeSet<ElementId>, id: &ElementId) -> Result<(), ModelError> {
    if ids.insert(id.clone()) {
        Ok(())
    } else {
        Err(ModelError::DuplicateElementId { id: id.clone() })
    }
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
        let code = CodeProfile::irc_2021_prescriptive();
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

    #[test]
    fn opening_validation_rejects_out_of_bounds() {
        let code = CodeProfile::irc_2021_prescriptive();
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
        let code = CodeProfile::irc_2021_prescriptive();
        let mut model = BuildingModel::new(code.clone());
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
    fn deterministic_sort_uses_stable_ids() {
        let code = CodeProfile::irc_2021_prescriptive();
        let mut model = BuildingModel::new(code.clone());
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
        let code = CodeProfile::irc_2021_prescriptive();
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
        let code = CodeProfile::irc_2021_prescriptive();
        let mut model = BuildingModel::new(code);
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

    fn placed_wall(id: &str, start: Point2, end: Point2, code: &CodeProfile) -> Wall {
        Wall::new(id, id, Length::from_feet(1.0), code).with_placement("level-1", start, end)
    }

    #[test]
    fn tee_join_validates_when_partition_meets_through_wall_midspan() {
        let code = CodeProfile::irc_2021_prescriptive();
        let mut model = BuildingModel::new(code.clone());
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
        let code = CodeProfile::irc_2021_prescriptive();
        let mut model = BuildingModel::new(code.clone());
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
        let code = CodeProfile::irc_2021_prescriptive();
        let mut model = BuildingModel::new(code.clone());
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
    fn reconcile_creates_tee_with_through_then_partition() {
        let code = CodeProfile::irc_2021_prescriptive();
        let mut model = BuildingModel::new(code.clone());
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
        let code = CodeProfile::irc_2021_prescriptive();
        let mut model = BuildingModel::new(code.clone());
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
    fn reconcile_drops_stale_join_when_walls_separate() {
        let code = CodeProfile::irc_2021_prescriptive();
        let mut model = BuildingModel::new(code.clone());
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
        let code = CodeProfile::irc_2021_prescriptive();
        let mut model = BuildingModel::new(code.clone());
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
        let code = CodeProfile::irc_2021_prescriptive();
        let mut model = BuildingModel::new(code.clone());
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
        let code = CodeProfile::irc_2021_prescriptive();
        let mut model = BuildingModel::new(code.clone());
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
        let code = CodeProfile::irc_2021_prescriptive();
        let mut model = BuildingModel::new(code.clone());
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
        let code = CodeProfile::irc_2021_prescriptive();
        let mut model = BuildingModel::new(code.clone());
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
        let code = CodeProfile::irc_2021_prescriptive();
        let mut model = BuildingModel::new(code.clone());
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
        let code = CodeProfile::irc_2021_prescriptive();
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
        let code = CodeProfile::irc_2021_prescriptive();
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
        let code = CodeProfile::irc_2021_prescriptive();
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
        let code = CodeProfile::irc_2021_prescriptive();
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
        let code = CodeProfile::irc_2021_prescriptive();
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
}
