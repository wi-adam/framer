use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::Length;

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
    pub walls: Vec<Wall>,
}

impl BuildingModel {
    pub fn new(code: CodeProfile) -> Self {
        Self {
            code,
            walls: Vec::new(),
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
            walls: vec![wall],
        }
    }

    pub fn validate(&self) -> Result<(), ModelError> {
        let mut ids = BTreeSet::new();
        for wall in &self.walls {
            wall.validate()?;
            insert_unique_id(&mut ids, &wall.id)?;
            for opening in &wall.openings {
                insert_unique_id(&mut ids, &opening.id)?;
            }
        }
        Ok(())
    }

    pub fn sort_deterministically(&mut self) {
        self.walls.sort_by(|left, right| left.id.cmp(&right.id));
        for wall in &mut self.walls {
            wall.sort_deterministically();
        }
    }

    pub fn into_deterministic(mut self) -> Self {
        self.sort_deterministically();
        self
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
    pub length: Length,
    pub height: Length,
    pub stud_spacing: Length,
    pub openings: Vec<Opening>,
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
            length,
            height: code.default_wall_height,
            stud_spacing: code.default_stud_spacing,
            openings: Vec::new(),
        }
    }

    pub fn validate(&self) -> Result<(), ModelError> {
        validate_element_id(&self.id)?;

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
    #[error("wall {wall:?} must have a positive length")]
    InvalidWallLength { wall: ElementId },
    #[error("wall {wall:?} must have a positive height")]
    InvalidWallHeight { wall: ElementId },
    #[error("wall {wall:?} must have a positive stud spacing")]
    InvalidStudSpacing { wall: ElementId },
    #[error("opening {opening:?} must have a positive width and height")]
    InvalidOpeningSize { opening: ElementId },
    #[error("opening {opening:?} is outside wall {wall:?}")]
    OpeningOutOfBounds { wall: ElementId, opening: ElementId },
    #[error("opening {opening:?} is taller than wall {wall:?}")]
    OpeningTooTall { wall: ElementId, opening: ElementId },
    #[error("openings {first:?} and {second:?} overlap")]
    OverlappingOpenings { first: ElementId, second: ElementId },
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
