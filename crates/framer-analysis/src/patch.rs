use framer_core::{
    AuthoredEntityRef, BuildingModel, ElementId, IntentOverrideId, ModelError, Point2, QuarterTurn,
};
use thiserror::Error;

/// The two authored placed-object families that the first resolution provider may edit.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PlacementTarget {
    FurnishingInstance(ElementId),
    MepInstance(ElementId),
}

impl PlacementTarget {
    pub const fn element(&self) -> &ElementId {
        match self {
            Self::FurnishingInstance(id) | Self::MepInstance(id) => id,
        }
    }

    pub fn authored(&self) -> AuthoredEntityRef {
        match self {
            Self::FurnishingInstance(id) => AuthoredEntityRef::FurnishingInstance(id.clone()),
            Self::MepInstance(id) => AuthoredEntityRef::MepInstance(id.clone()),
        }
    }

    pub const fn kind_label(&self) -> &'static str {
        match self {
            Self::FurnishingInstance(_) => "furnishing instance",
            Self::MepInstance(_) => "MEP instance",
        }
    }
}

/// Complete authored plan pose of a placed furnishing or MEP instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlacementPose {
    pub position: Point2,
    pub rotation: QuarterTurn,
}

impl PlacementPose {
    pub const fn new(position: Point2, rotation: QuarterTurn) -> Self {
        Self { position, rotation }
    }
}

/// One placement-only compare-and-swap operation over authored model state.
///
/// `expected` makes a patch fail closed if the target moved after the option was produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlacementPatch {
    pub target: PlacementTarget,
    pub expected: PlacementPose,
    pub replacement: PlacementPose,
}

impl PlacementPatch {
    pub const fn new(
        target: PlacementTarget,
        expected: PlacementPose,
        replacement: PlacementPose,
    ) -> Self {
        Self {
            target,
            expected,
            replacement,
        }
    }

    pub const fn before(&self) -> PlacementPose {
        self.expected
    }

    pub const fn after(&self) -> PlacementPose {
        self.replacement
    }
}

/// A sorted, validated candidate model. The source model is never borrowed mutably while a patch
/// is previewed; callers may inspect this value or consume it at their ordinary edit boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedPlacementPatch {
    model: BuildingModel,
}

impl StagedPlacementPatch {
    pub const fn model(&self) -> &BuildingModel {
        &self.model
    }

    pub fn into_model(self) -> BuildingModel {
        self.model
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PlacementPatchError {
    #[error("placement patch target id must not be empty")]
    EmptyTarget,
    #[error("placement patch does not change the target pose")]
    NoOp,
    #[error(
        "placement patch expected {expected_kind} {target:?}, but that id belongs to {actual_kind}"
    )]
    WrongKind {
        target: ElementId,
        expected_kind: &'static str,
        actual_kind: &'static str,
    },
    #[error("placement patch target {target:?} does not exist")]
    MissingTarget { target: PlacementTarget },
    #[error(
        "placement patch target {target:?} changed after staging (expected {expected:?}, current {current:?})"
    )]
    StaleExpectedPose {
        target: PlacementTarget,
        expected: PlacementPose,
        current: PlacementPose,
    },
    #[error("staged placement model failed validation: {0}")]
    InvalidModel(String),
}

impl From<ModelError> for PlacementPatchError {
    fn from(error: ModelError) -> Self {
        Self::InvalidModel(error.to_string())
    }
}

/// Read the current authored pose of a typed placement target.
pub fn current_placement_pose(
    model: &BuildingModel,
    target: &PlacementTarget,
) -> Result<PlacementPose, PlacementPatchError> {
    if target.element().0.trim().is_empty() {
        return Err(PlacementPatchError::EmptyTarget);
    }

    let pose = match target {
        PlacementTarget::FurnishingInstance(id) => model
            .furnishing_instances
            .iter()
            .find(|instance| instance.id == *id)
            .map(|instance| PlacementPose::new(instance.position, instance.rotation)),
        PlacementTarget::MepInstance(id) => model
            .mep_instances
            .iter()
            .find(|instance| instance.id == *id)
            .map(|instance| PlacementPose::new(instance.position, instance.rotation)),
    };
    if let Some(pose) = pose {
        return Ok(pose);
    }

    if let Some(actual_kind) = authored_kind_for_id(model, target.element()) {
        return Err(PlacementPatchError::WrongKind {
            target: target.element().clone(),
            expected_kind: target.kind_label(),
            actual_kind,
        });
    }
    Err(PlacementPatchError::MissingTarget {
        target: target.clone(),
    })
}

/// Clone, compare-and-swap, canonicalize, and validate one placement patch without mutating the
/// source model.
pub fn stage_placement_patch(
    model: &BuildingModel,
    patch: &PlacementPatch,
) -> Result<StagedPlacementPatch, PlacementPatchError> {
    if patch.expected == patch.replacement {
        return Err(PlacementPatchError::NoOp);
    }
    let current = current_placement_pose(model, &patch.target)?;
    if current != patch.expected {
        return Err(PlacementPatchError::StaleExpectedPose {
            target: patch.target.clone(),
            expected: patch.expected,
            current,
        });
    }

    let mut candidate = model.clone();
    match &patch.target {
        PlacementTarget::FurnishingInstance(id) => {
            let instance = candidate
                .furnishing_instances
                .iter_mut()
                .find(|instance| instance.id == *id)
                .expect("typed target was resolved before cloning");
            instance.position = patch.replacement.position;
            instance.rotation = patch.replacement.rotation;
        }
        PlacementTarget::MepInstance(id) => {
            let instance = candidate
                .mep_instances
                .iter_mut()
                .find(|instance| instance.id == *id)
                .expect("typed target was resolved before cloning");
            instance.position = patch.replacement.position;
            instance.rotation = patch.replacement.rotation;
        }
    }
    candidate.sort_deterministically();
    candidate.validate()?;
    Ok(StagedPlacementPatch { model: candidate })
}

/// Transactionally apply a validated patch. On every error, `model` is unchanged.
pub fn apply_placement_patch(
    model: &mut BuildingModel,
    patch: &PlacementPatch,
) -> Result<(), PlacementPatchError> {
    *model = stage_placement_patch(model, patch)?.into_model();
    Ok(())
}

fn authored_kind_for_id(model: &BuildingModel, id: &ElementId) -> Option<&'static str> {
    let candidates = [
        AuthoredEntityRef::StandardsPack(id.clone()),
        AuthoredEntityRef::Material(id.clone()),
        AuthoredEntityRef::ConstructionSystem(id.clone()),
        AuthoredEntityRef::Furnishing(id.clone()),
        AuthoredEntityRef::MepObject(id.clone()),
        AuthoredEntityRef::Level(id.clone()),
        AuthoredEntityRef::Wall(id.clone()),
        AuthoredEntityRef::Opening(id.clone()),
        AuthoredEntityRef::Dimension(id.clone()),
        AuthoredEntityRef::WallJoin(id.clone()),
        AuthoredEntityRef::Room(id.clone()),
        AuthoredEntityRef::FurnishingInstance(id.clone()),
        AuthoredEntityRef::MepInstance(id.clone()),
        AuthoredEntityRef::RoofPlane(id.clone()),
        AuthoredEntityRef::RoofOpening(id.clone()),
        AuthoredEntityRef::Ceiling(id.clone()),
        AuthoredEntityRef::FloorDeck(id.clone()),
        AuthoredEntityRef::BracedWallLine(id.clone()),
        AuthoredEntityRef::BracedPanel(id.clone()),
        AuthoredEntityRef::IntentOverride(IntentOverrideId(id.clone())),
    ];
    candidates
        .iter()
        .find(|candidate| model.authored_entity_exists(candidate))
        .map(AuthoredEntityRef::kind_label)
}

#[cfg(test)]
mod tests {
    use framer_core::{
        BuildingModel, Furnishing, FurnishingInstance, Length, MepInstance, QuarterTurn,
        load_project, save_project,
    };

    use super::*;

    fn patch_model() -> BuildingModel {
        let mut model = BuildingModel::demo_shell();
        model.furnishings.push(Furnishing::new(
            "chair",
            "Chair",
            Length::from_whole_inches(24),
            Length::from_whole_inches(30),
            Length::from_whole_inches(36),
        ));
        model.furnishing_instances.push(FurnishingInstance::new(
            "chair-1",
            "Chair 1",
            "chair",
            "level-1",
            Point2::new(Length::from_whole_inches(48), Length::from_whole_inches(48)),
        ));
        model.sort_deterministically();
        model.validate().unwrap();
        model
    }

    fn chair_target() -> PlacementTarget {
        PlacementTarget::FurnishingInstance(ElementId::new("chair-1"))
    }

    #[test]
    fn patch_preview_is_valid_round_trips_and_leaves_source_unchanged() {
        let source = patch_model();
        let original = source.clone();
        let expected = current_placement_pose(&source, &chair_target()).unwrap();
        let replacement = PlacementPose::new(
            Point2::new(Length::from_whole_inches(72), Length::from_whole_inches(84)),
            QuarterTurn::Deg90,
        );
        let staged = stage_placement_patch(
            &source,
            &PlacementPatch::new(chair_target(), expected, replacement),
        )
        .unwrap();

        assert_eq!(source, original);
        assert_eq!(
            current_placement_pose(staged.model(), &chair_target()).unwrap(),
            replacement
        );
        staged.model().validate().unwrap();
        let encoded = save_project(staged.model()).unwrap();
        assert_eq!(load_project(&encoded).unwrap(), staged.into_model());
    }

    #[test]
    fn apply_is_transactional_and_rejects_empty_noop_missing_wrong_kind_and_stale_expected() {
        let source = patch_model();
        let pose = current_placement_pose(&source, &chair_target()).unwrap();
        let moved = PlacementPose::new(
            Point2::new(pose.position.x + Length::from_ticks(1), pose.position.y),
            pose.rotation,
        );

        assert_eq!(
            stage_placement_patch(
                &source,
                &PlacementPatch::new(
                    PlacementTarget::FurnishingInstance(ElementId::new("")),
                    pose,
                    moved,
                ),
            ),
            Err(PlacementPatchError::EmptyTarget)
        );
        assert_eq!(
            stage_placement_patch(&source, &PlacementPatch::new(chair_target(), pose, pose),),
            Err(PlacementPatchError::NoOp)
        );
        assert!(matches!(
            stage_placement_patch(
                &source,
                &PlacementPatch::new(
                    PlacementTarget::FurnishingInstance(ElementId::new("missing")),
                    pose,
                    moved,
                ),
            ),
            Err(PlacementPatchError::MissingTarget { .. })
        ));
        assert!(matches!(
            stage_placement_patch(
                &source,
                &PlacementPatch::new(
                    PlacementTarget::FurnishingInstance(ElementId::new("level-1")),
                    pose,
                    moved,
                ),
            ),
            Err(PlacementPatchError::WrongKind { .. })
        ));
        assert!(matches!(
            stage_placement_patch(
                &source,
                &PlacementPatch::new(
                    chair_target(),
                    PlacementPose::new(pose.position, QuarterTurn::Deg180),
                    moved,
                ),
            ),
            Err(PlacementPatchError::StaleExpectedPose { .. })
        ));

        let mut wrong_kind = source.clone();
        wrong_kind.mep_instances.push(MepInstance::new(
            "mep-1",
            "MEP 1",
            "missing-family",
            "level-1",
            pose.position,
        ));
        assert!(matches!(
            current_placement_pose(
                &wrong_kind,
                &PlacementTarget::FurnishingInstance(ElementId::new("mep-1")),
            ),
            Err(PlacementPatchError::WrongKind { .. })
        ));

        let mut transactional = source.clone();
        let stale = PlacementPatch::new(
            chair_target(),
            PlacementPose::new(pose.position, QuarterTurn::Deg270),
            moved,
        );
        assert!(apply_placement_patch(&mut transactional, &stale).is_err());
        assert_eq!(transactional, source);
    }
}
