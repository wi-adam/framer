//! Session-only component selection, visibility, and isolation state.
//!
//! The legacy inspector selection remains a single [`Selection`] plus an active
//! wall index. This module supplies stable, id-backed component identity around
//! that editing context so the browser and 3-D viewport can select more than one
//! component without making vector indices part of presentation identity.

use std::collections::BTreeSet;

use framer_solver::FrameMember;

use super::Selection;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) enum AuthoredComponentKind {
    Wall,
    Opening,
    Dimension,
    Join,
    Room,
    RoofPlane,
    Ceiling,
    FloorDeck,
    FurnishingInstance,
    MepInstance,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) enum ComponentKey {
    Authored {
        kind: AuthoredComponentKind,
        id: String,
    },
    GeneratedMember {
        host_id: String,
        member_id: String,
    },
}

impl ComponentKey {
    pub(super) fn authored(kind: AuthoredComponentKind, id: impl Into<String>) -> Self {
        Self::Authored {
            kind,
            id: id.into(),
        }
    }

    pub(super) fn member(host_id: impl Into<String>, member_id: impl Into<String>) -> Self {
        Self::GeneratedMember {
            host_id: host_id.into(),
            member_id: member_id.into(),
        }
    }

    pub(super) fn semantic_source_id(&self) -> Option<&str> {
        match self {
            Self::Authored {
                kind: AuthoredComponentKind::Opening | AuthoredComponentKind::Join,
                id,
            } => Some(id),
            Self::GeneratedMember { .. } => None,
            Self::Authored { .. } => None,
        }
    }

    pub(super) fn is_renderable(&self) -> bool {
        matches!(
            self,
            Self::GeneratedMember { .. }
                | Self::Authored {
                    kind: AuthoredComponentKind::Wall
                        | AuthoredComponentKind::Opening
                        | AuthoredComponentKind::Join
                        | AuthoredComponentKind::RoofPlane
                        | AuthoredComponentKind::Ceiling
                        | AuthoredComponentKind::FloorDeck,
                    ..
                }
        )
    }

    pub(super) fn has_design_3d_geometry(&self) -> bool {
        matches!(
            self,
            Self::Authored {
                kind: AuthoredComponentKind::Wall
                    | AuthoredComponentKind::RoofPlane
                    | AuthoredComponentKind::Ceiling
                    | AuthoredComponentKind::FloorDeck,
                ..
            }
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SelectionOp {
    Replace,
    Toggle,
}

#[derive(Debug, Clone, Default)]
pub(super) struct ComponentSelection {
    /// Unique, interaction-ordered keys. The last item is the primary selection.
    items: Vec<ComponentKey>,
}

impl ComponentSelection {
    pub(super) fn active_items(&self, current_primary: Option<ComponentKey>) -> Vec<ComponentKey> {
        let Some(current_primary) = current_primary else {
            return Vec::new();
        };
        if self.items.last() == Some(&current_primary) {
            self.items.clone()
        } else {
            vec![current_primary]
        }
    }

    pub(super) fn replace(&mut self, key: Option<ComponentKey>) {
        self.items.clear();
        if let Some(key) = key {
            self.items.push(key);
        }
    }

    pub(super) fn set_items(&mut self, items: Vec<ComponentKey>) {
        self.items = dedupe(items);
    }

    /// Toggle `key`, preserving interaction order and returning the new primary.
    pub(super) fn toggle(
        &mut self,
        current_primary: Option<ComponentKey>,
        key: ComponentKey,
    ) -> Option<ComponentKey> {
        let mut items = self.active_items(current_primary);
        if let Some(index) = items.iter().position(|item| item == &key) {
            items.remove(index);
        } else {
            items.push(key);
        }
        self.items = items;
        self.items.last().cloned()
    }

    pub(super) fn retain(&mut self, mut keep: impl FnMut(&ComponentKey) -> bool) {
        self.items.retain(|item| keep(item));
    }

    pub(super) fn primary(&self) -> Option<&ComponentKey> {
        self.items.last()
    }

    #[cfg(test)]
    pub(super) fn items(&self) -> &[ComponentKey] {
        &self.items
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum IsolationMode {
    DimOthers,
    HideOthers,
}

impl IsolationMode {
    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::DimOthers => "Dim others",
            Self::HideOthers => "Hide others",
        }
    }
}

#[derive(Debug, Clone)]
struct IsolationState {
    mode: IsolationMode,
    targets: Vec<ComponentKey>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ComponentAppearance {
    Normal,
    Dimmed,
    Hidden,
}

#[derive(Debug, Clone, Default)]
pub(super) struct ComponentVisibility {
    hidden: BTreeSet<ComponentKey>,
    isolation: Option<IsolationState>,
}

impl ComponentVisibility {
    pub(super) fn is_explicitly_visible(&self, key: &ComponentKey) -> bool {
        !self.hidden.contains(key)
    }

    pub(super) fn toggle(&mut self, key: ComponentKey) {
        self.exit_isolation();
        if !self.hidden.remove(&key) {
            self.hidden.insert(key);
        }
    }

    pub(super) fn hide(&mut self, keys: impl IntoIterator<Item = ComponentKey>) {
        self.exit_isolation();
        self.hidden
            .extend(keys.into_iter().filter(ComponentKey::is_renderable));
    }

    pub(super) fn show_all(&mut self) {
        self.hidden.clear();
    }

    pub(super) fn has_hidden(&self) -> bool {
        !self.hidden.is_empty()
    }

    pub(super) fn isolate(&mut self, mode: IsolationMode, targets: Vec<ComponentKey>) {
        let targets = dedupe(targets.into_iter().filter(ComponentKey::is_renderable));
        self.isolation = (!targets.is_empty()).then_some(IsolationState { mode, targets });
    }

    pub(super) fn exit_isolation(&mut self) {
        self.isolation = None;
    }

    pub(super) fn isolation_mode(&self) -> Option<IsolationMode> {
        self.isolation.as_ref().map(|isolation| isolation.mode)
    }

    pub(super) fn isolation_targets(&self) -> &[ComponentKey] {
        self.isolation
            .as_ref()
            .map(|isolation| isolation.targets.as_slice())
            .unwrap_or_default()
    }

    pub(super) fn retain(&mut self, mut keep: impl FnMut(&ComponentKey) -> bool) {
        self.hidden.retain(|item| keep(item));
        if let Some(isolation) = self.isolation.as_mut() {
            isolation.targets.retain(&mut keep);
            if isolation.targets.is_empty() {
                self.isolation = None;
            }
        }
    }

    pub(super) fn authored_appearance(&self, key: &ComponentKey) -> ComponentAppearance {
        if self.isolation_target_related(|target| target == key) {
            return ComponentAppearance::Normal;
        }
        if self.hidden.contains(key) {
            return ComponentAppearance::Hidden;
        }
        self.unrelated_isolation_appearance()
    }

    pub(super) fn opening_appearance(
        &self,
        wall: &ComponentKey,
        opening: &ComponentKey,
    ) -> ComponentAppearance {
        if self.isolation_target_related(|target| target == wall || target == opening) {
            return ComponentAppearance::Normal;
        }
        if self.hidden.contains(wall) || self.hidden.contains(opening) {
            return ComponentAppearance::Hidden;
        }
        self.unrelated_isolation_appearance()
    }

    pub(super) fn member_appearance(
        &self,
        host: &ComponentKey,
        member_key: &ComponentKey,
        member: &FrameMember,
    ) -> ComponentAppearance {
        let related = |target: &ComponentKey| {
            target == host
                || target == member_key
                || target.semantic_source_id() == Some(member.source.0.as_str())
        };
        if self.isolation_target_related(related) {
            return ComponentAppearance::Normal;
        }
        let semantic_source_hidden = self
            .hidden
            .iter()
            .any(|key| key.semantic_source_id() == Some(member.source.0.as_str()));
        if self.hidden.contains(host) || self.hidden.contains(member_key) || semantic_source_hidden
        {
            return ComponentAppearance::Hidden;
        }

        self.unrelated_isolation_appearance()
    }

    fn isolation_target_related(&self, related: impl Fn(&ComponentKey) -> bool) -> bool {
        self.isolation
            .as_ref()
            .is_some_and(|isolation| isolation.targets.iter().any(related))
    }

    fn unrelated_isolation_appearance(&self) -> ComponentAppearance {
        match self.isolation.as_ref().map(|isolation| isolation.mode) {
            None => ComponentAppearance::Normal,
            Some(IsolationMode::DimOthers) => ComponentAppearance::Dimmed,
            Some(IsolationMode::HideOthers) => ComponentAppearance::Hidden,
        }
    }
}

fn dedupe(items: impl IntoIterator<Item = ComponentKey>) -> Vec<ComponentKey> {
    let mut unique = Vec::new();
    for item in items {
        if !unique.contains(&item) {
            unique.push(item);
        }
    }
    unique
}

pub(super) fn key_for_selection(
    selected: &Selection,
    selected_wall_id: Option<&str>,
) -> Option<ComponentKey> {
    let authored = ComponentKey::authored;
    match selected {
        Selection::None
        | Selection::Site
        | Selection::Level(_)
        | Selection::System(_)
        | Selection::Material(_)
        | Selection::Furnishing(_)
        | Selection::MepObject(_)
        | Selection::StandardsPack(_) => None,
        Selection::Wall => selected_wall_id.map(|id| authored(AuthoredComponentKind::Wall, id)),
        Selection::Opening(id) => Some(authored(AuthoredComponentKind::Opening, id)),
        Selection::Dimension(id) => Some(authored(AuthoredComponentKind::Dimension, id)),
        Selection::Join(id) => Some(authored(AuthoredComponentKind::Join, id)),
        Selection::Room(id) => Some(authored(AuthoredComponentKind::Room, id)),
        Selection::Member {
            source_id,
            member_id,
        } => Some(ComponentKey::member(source_id, member_id)),
        Selection::RoofPlane(id) => Some(authored(AuthoredComponentKind::RoofPlane, id)),
        Selection::Ceiling(id) => Some(authored(AuthoredComponentKind::Ceiling, id)),
        Selection::FloorDeck(id) => Some(authored(AuthoredComponentKind::FloorDeck, id)),
        Selection::FurnishingInstance(id) => {
            Some(authored(AuthoredComponentKind::FurnishingInstance, id))
        }
        Selection::MepInstance(id) => Some(authored(AuthoredComponentKind::MepInstance, id)),
    }
}

#[cfg(test)]
mod tests {
    use framer_core::{BoardProfile, ElementId, Length};
    use framer_solver::{MemberKind, MemberOrientation, RuleProvenance};

    use super::*;

    fn member(source: &str) -> FrameMember {
        FrameMember {
            id: "member-1".to_owned(),
            source: ElementId::new(source),
            kind: MemberKind::JackStud,
            profile: BoardProfile::TwoByFour,
            orientation: MemberOrientation::Vertical,
            x: Length::ZERO,
            elevation: Length::ZERO,
            cut_length: Length::from_whole_inches(80),
            cross_section_depth: Length::from_whole_inches(4),
            side_offset: Length::ZERO,
            side_depth: Length::from_whole_inches(4),
            sloped: None,
            provenance: RuleProvenance {
                rule_id: "test".to_owned(),
                summary: "test".to_owned(),
            },
        }
    }

    #[test]
    fn ordered_selection_toggle_promotes_and_removes_primary() {
        let wall_a = ComponentKey::authored(AuthoredComponentKind::Wall, "wall-a");
        let wall_b = ComponentKey::authored(AuthoredComponentKind::Wall, "wall-b");
        let mut selection = ComponentSelection::default();
        selection.replace(Some(wall_a.clone()));

        assert_eq!(
            selection.toggle(Some(wall_a.clone()), wall_b.clone()),
            Some(wall_b.clone())
        );
        assert_eq!(selection.items(), &[wall_a.clone(), wall_b.clone()]);
        assert_eq!(
            selection.toggle(Some(wall_b.clone()), wall_a.clone()),
            Some(wall_b.clone())
        );
        assert_eq!(selection.items(), &[wall_b]);
    }

    #[test]
    fn opening_visibility_groups_members_by_semantic_source_not_host() {
        let host = ComponentKey::authored(AuthoredComponentKind::Wall, "wall-1");
        let opening = ComponentKey::authored(AuthoredComponentKind::Opening, "door-1");
        let exact = ComponentKey::member("wall-1", "door-1-jack-left");
        let mut visibility = ComponentVisibility::default();
        visibility.isolate(IsolationMode::HideOthers, vec![opening.clone()]);

        assert_eq!(
            visibility.member_appearance(&host, &exact, &member("door-1")),
            ComponentAppearance::Normal
        );
        assert_eq!(
            visibility.member_appearance(
                &host,
                &ComponentKey::member("wall-1", "common-stud-1"),
                &member("wall-1"),
            ),
            ComponentAppearance::Hidden
        );

        visibility.exit_isolation();
        visibility.toggle(opening);
        assert_eq!(
            visibility.member_appearance(&host, &exact, &member("door-1")),
            ComponentAppearance::Hidden
        );
    }

    #[test]
    fn corner_visibility_groups_only_matching_semantic_source_members() {
        let host = ComponentKey::authored(AuthoredComponentKind::Wall, "wall-1");
        let corner = ComponentKey::authored(AuthoredComponentKind::Join, "corner-1");
        let unrelated_authored =
            ComponentKey::authored(AuthoredComponentKind::RoofPlane, "corner-1");
        let exact = ComponentKey::member("wall-1", "corner-post");
        let corner_member = member("corner-1");
        let mut visibility = ComponentVisibility::default();
        visibility.isolate(IsolationMode::HideOthers, vec![corner]);

        assert_eq!(
            visibility.member_appearance(&host, &exact, &corner_member),
            ComponentAppearance::Normal
        );

        visibility.isolate(IsolationMode::HideOthers, vec![unrelated_authored]);
        assert_eq!(
            visibility.member_appearance(&host, &exact, &corner_member),
            ComponentAppearance::Hidden,
            "only Opening and Join identities expand through FrameMember::source"
        );
    }

    #[test]
    fn isolation_temporarily_reveals_hidden_target_then_restores_override() {
        let wall = ComponentKey::authored(AuthoredComponentKind::Wall, "wall-1");
        let mut visibility = ComponentVisibility::default();
        visibility.hide([wall.clone()]);
        assert_eq!(
            visibility.authored_appearance(&wall),
            ComponentAppearance::Hidden
        );

        visibility.isolate(IsolationMode::HideOthers, vec![wall.clone()]);
        assert_eq!(
            visibility.authored_appearance(&wall),
            ComponentAppearance::Normal
        );

        visibility.exit_isolation();
        assert_eq!(
            visibility.authored_appearance(&wall),
            ComponentAppearance::Hidden
        );
    }

    #[test]
    fn explicit_visibility_change_exits_isolation_so_it_is_immediately_visible() {
        let wall = ComponentKey::authored(AuthoredComponentKind::Wall, "wall-1");
        let mut visibility = ComponentVisibility::default();
        visibility.isolate(IsolationMode::DimOthers, vec![wall.clone()]);

        visibility.toggle(wall.clone());

        assert_eq!(visibility.isolation_mode(), None);
        assert_eq!(
            visibility.authored_appearance(&wall),
            ComponentAppearance::Hidden
        );
    }

    #[test]
    fn ordered_selection_supports_multiple_generated_member_leaves() {
        let first = ComponentKey::member("wall-1", "stud-1");
        let second = ComponentKey::member("wall-1", "stud-2");
        let mut selection = ComponentSelection::default();
        selection.replace(Some(first.clone()));

        assert_eq!(
            selection.toggle(Some(first.clone()), second.clone()),
            Some(second.clone())
        );
        assert_eq!(selection.items(), &[first, second]);
    }

    #[test]
    fn dim_isolation_keeps_unrelated_members_ghosted() {
        let host = ComponentKey::authored(AuthoredComponentKind::Wall, "wall-1");
        let selected = ComponentKey::member("wall-1", "member-1");
        let other = ComponentKey::member("wall-1", "member-2");
        let mut visibility = ComponentVisibility::default();
        visibility.isolate(IsolationMode::DimOthers, vec![selected.clone()]);

        assert_eq!(
            visibility.member_appearance(&host, &selected, &member("wall-1")),
            ComponentAppearance::Normal
        );
        assert_eq!(
            visibility.member_appearance(&host, &other, &member("other-source")),
            ComponentAppearance::Dimmed
        );
    }
}
