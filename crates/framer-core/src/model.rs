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
    #[serde(default = "default_levels")]
    pub levels: Vec<Level>,
    pub walls: Vec<Wall>,
    #[serde(default)]
    pub wall_joins: Vec<WallJoin>,
}

impl BuildingModel {
    pub fn new(code: CodeProfile) -> Self {
        Self {
            code,
            levels: default_levels(),
            walls: Vec::new(),
            wall_joins: Vec::new(),
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

        Self {
            code,
            levels: default_levels(),
            walls: vec![wall],
            wall_joins: Vec::new(),
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

        Self {
            code,
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

            if !first.has_endpoint(join.point) || !second.has_endpoint(join.point) {
                return Err(ModelError::JoinPointDoesNotConnectWalls {
                    join: join.id.clone(),
                });
            }
        }
        Ok(())
    }

    pub fn sort_deterministically(&mut self) {
        self.levels.sort_by(|left, right| left.id.cmp(&right.id));
        self.walls.sort_by(|left, right| left.id.cmp(&right.id));
        for wall in &mut self.walls {
            wall.sort_deterministically();
        }
        self.wall_joins
            .sort_by(|left, right| left.id.cmp(&right.id));
    }

    pub fn into_deterministic(mut self) -> Self {
        self.sort_deterministically();
        self
    }

    pub fn upgrade_legacy_wall_placements(&mut self) {
        if self.levels.is_empty() {
            self.levels = default_levels();
        }

        for wall in &mut self.walls {
            if wall.level.0.is_empty() {
                wall.level = ElementId::new("level-1");
            }

            if wall.start == wall.end {
                wall.end = Point2::new(wall.start.x + wall.length, wall.start.y);
            }
        }
    }

    pub fn apply_driving_dimensions(&mut self) -> bool {
        let mut changed = false;
        for wall in &mut self.walls {
            changed |= wall.apply_driving_dimensions();
        }
        changed
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
    pub stud_spacing: Length,
    pub openings: Vec<Opening>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dimensions: Vec<DimensionConstraint>,
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
            stud_spacing: code.default_stud_spacing,
            openings: Vec::new(),
            dimensions: Vec::new(),
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

        if self.stud_spacing <= Length::ZERO {
            return Err(ModelError::InvalidStudSpacing {
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
            dimension.validate(&opening_ids, self.length)?;
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

    pub fn dimension_measurement(&self, dimension: &DimensionConstraint) -> Option<Length> {
        let start = dimension.start.local_x(self)?;
        let end = dimension.end.local_x(self)?;
        Some((end - start).abs())
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
        let start = dimension.start.local_x(self)?;
        let end = dimension.end.local_x(self)?;
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
        for opening in &self.openings {
            variables.insert(opening_constraint_variable(&opening.id, "center-x"));
            variables.insert(opening_constraint_variable(&opening.id, "width"));
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
        let start = self.dimension_anchor_expression(&dimension.start)?;
        let mut expression = self.dimension_anchor_expression(&dimension.end)?;
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

    fn dimension_anchor_expression(&self, anchor: &DimensionAnchor) -> Option<LinearExpression> {
        let mut expression = LinearExpression::new();
        match anchor {
            DimensionAnchor::WallStart => {}
            DimensionAnchor::WallEnd => {
                expression.add_term(wall_constraint_variable(&self.id, "length"), 2);
            }
            DimensionAnchor::OpeningLeft { opening } => {
                self.add_opening_anchor_terms(&mut expression, opening, -1)?;
            }
            DimensionAnchor::OpeningCenter { opening } => {
                self.add_opening_anchor_terms(&mut expression, opening, 0)?;
            }
            DimensionAnchor::OpeningRight { opening } => {
                self.add_opening_anchor_terms(&mut expression, opening, 1)?;
            }
        }
        Some(expression)
    }

    fn add_opening_anchor_terms(
        &self,
        expression: &mut LinearExpression,
        opening: &ElementId,
        width_coefficient: i64,
    ) -> Option<()> {
        if !self
            .openings
            .iter()
            .any(|candidate| candidate.id == *opening)
        {
            return None;
        }

        expression.add_term(opening_constraint_variable(opening, "center-x"), 2);
        if width_coefficient != 0 {
            expression.add_term(
                opening_constraint_variable(opening, "width"),
                width_coefficient,
            );
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
        for opening in &self.openings {
            values.insert(
                opening_constraint_variable(&opening.id, "center-x"),
                opening.center,
            );
            values.insert(
                opening_constraint_variable(&opening.id, "width"),
                opening.width,
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

            if opening.width <= Length::ZERO
                || opening.left() < Length::ZERO
                || opening.right() > next_length
            {
                return false;
            }
        }

        let changed = self.length != next_length || self.openings != next_openings;
        self.set_length_keep_direction(next_length);
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
    pub start: DimensionAnchor,
    pub end: DimensionAnchor,
    #[serde(default)]
    pub direction: DimensionDirection,
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
            start,
            end,
            direction,
            value,
        }
    }

    fn validate(
        &self,
        opening_ids: &BTreeSet<ElementId>,
        wall_length: Length,
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
                if value <= Length::ZERO || value > wall_length {
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DimensionKind {
    Driving,
    Reference,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum DimensionDirection {
    #[default]
    Forward,
    Backward,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DimensionAnchor {
    WallStart,
    WallEnd,
    OpeningLeft { opening: ElementId },
    OpeningCenter { opening: ElementId },
    OpeningRight { opening: ElementId },
}

impl DimensionAnchor {
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
        }
    }

    fn validate(&self, opening_ids: &BTreeSet<ElementId>) -> Result<(), ModelError> {
        let opening = match self {
            Self::OpeningLeft { opening }
            | Self::OpeningCenter { opening }
            | Self::OpeningRight { opening } => Some(opening),
            Self::WallStart | Self::WallEnd => None,
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
    pub fn corner(
        id: impl Into<String>,
        name: impl Into<String>,
        first_wall: impl Into<String>,
        second_wall: impl Into<String>,
        point: Point2,
    ) -> Self {
        Self {
            id: ElementId::new(id),
            name: name.into(),
            kind: WallJoinKind::Corner,
            first_wall: ElementId::new(first_wall),
            second_wall: ElementId::new(second_wall),
            point,
        }
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
    #[error("wall {wall:?} must have a positive stud spacing")]
    InvalidStudSpacing { wall: ElementId },
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
