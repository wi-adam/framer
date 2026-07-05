use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use framer_core::{
    Applicability, BoardProfile, BracedPanel, BracedWallLine, BracingMethod, BracingRow,
    BuildingModel, CheckScope, CheckSeverity, CompareOp, ElementId, Fact, FactOperand, HeaderRow,
    Length, Opening, Predicate, PropertyValue, ResolutionAction, ResolvedRule, ResolvedStandards,
    SiteContext, Wall, WallExposure,
};
use framer_solver::{
    DiagnosticSeverity, FrameMember, MemberKind, PlanDiagnostic, ProjectFramePlan, RuleRef,
};
use serde::{Deserialize, Serialize};

const BRACING_UNASSOCIATED_PANEL: &str = "standards.bracing.unassociated-panel";
const BRACING_OUT_OF_DOMAIN: &str = "standards.bracing.out-of-domain";
const BRACING_ASSOCIATION_TOLERANCE: Length = Length::from_whole_inches(48);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Tri {
    False,
    Unknown,
    True,
}

impl Tri {
    pub const fn not(self) -> Self {
        match self {
            Self::False => Self::True,
            Self::Unknown => Self::Unknown,
            Self::True => Self::False,
        }
    }

    pub fn all(values: impl IntoIterator<Item = Self>) -> Self {
        values.into_iter().min().unwrap_or(Self::True)
    }

    pub fn any(values: impl IntoIterator<Item = Self>) -> Self {
        values.into_iter().max().unwrap_or(Self::False)
    }
}

impl From<bool> for Tri {
    fn from(value: bool) -> Self {
        if value { Self::True } else { Self::False }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FactValue {
    Length(Length),
    Int(i64),
    Flag(bool),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum EntityRef {
    Wall(ElementId),
    Opening(ElementId),
    Room(ElementId),
    BracedWallLine(ElementId),
}

impl EntityRef {
    fn element(&self) -> &ElementId {
        match self {
            Self::Wall(id) | Self::Opening(id) | Self::Room(id) | Self::BracedWallLine(id) => id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Outcome {
    Pass,
    Violation,
    Advisory,
    NeedsReview,
    NotApplicable,
    Waived { reason: String },
}

impl Outcome {
    fn label(&self) -> &'static str {
        match self {
            Self::Pass => "Pass",
            Self::Violation => "Violation",
            Self::Advisory => "Advisory",
            Self::NeedsReview => "NeedsReview",
            Self::NotApplicable => "NotApplicable",
            Self::Waived { .. } => "Waived",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComplianceEntry {
    pub rule: String,
    pub citation: String,
    pub pack: ElementId,
    pub outcome: Outcome,
    pub element: Option<ElementId>,
    pub message: String,
    pub chain: Vec<(ElementId, ResolutionAction)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ComplianceReport {
    pub entries: Vec<ComplianceEntry>,
}

impl ComplianceReport {
    pub fn to_csv(&self) -> String {
        let mut csv = "rule,citation,pack,outcome,element,message,chain\n".to_owned();
        for entry in &self.entries {
            let fields = [
                entry.rule.clone(),
                entry.citation.clone(),
                entry.pack.0.clone(),
                entry.outcome.label().to_owned(),
                entry
                    .element
                    .as_ref()
                    .map(|id| id.0.clone())
                    .unwrap_or_default(),
                entry.message.clone(),
                entry
                    .chain
                    .iter()
                    .map(|(pack, action)| format!("{}:{action:?}", pack.0))
                    .collect::<Vec<_>>()
                    .join(";"),
            ];
            csv.push_str(
                &fields
                    .iter()
                    .map(|field| csv_field(field))
                    .collect::<Vec<_>>()
                    .join(","),
            );
            csv.push('\n');
        }
        csv
    }
}

pub fn evaluate(
    model: &BuildingModel,
    resolved: &ResolvedStandards,
    plan: &ProjectFramePlan,
) -> ComplianceReport {
    let active_checks = resolved
        .checks
        .iter()
        .map(|(pack, check)| (check.rule.as_str(), (pack, check)))
        .collect::<BTreeMap<_, _>>();
    let mut entries = Vec::new();
    entries.extend(bracing_diagnostic_entries(model, resolved));

    for rule in resolved.rules.iter().filter(|rule| rule.severity.is_some()) {
        if let Some(reason) = &rule.waived {
            entries.push(entry(
                rule,
                Outcome::Waived {
                    reason: reason.clone(),
                },
                None,
                format!("Waived: {reason}"),
            ));
            continue;
        }

        let Some((_, check)) = active_checks.get(rule.rule.as_str()) else {
            continue;
        };

        match applicability(check.applies.clone(), &model.site) {
            Tri::False => {
                entries.push(entry(
                    rule,
                    Outcome::NotApplicable,
                    None,
                    format!("{} is not applicable for this site context.", check.title),
                ));
            }
            Tri::Unknown => {
                entries.push(entry(
                    rule,
                    Outcome::NeedsReview,
                    None,
                    format!("{} applicability needs review.", check.title),
                ));
            }
            Tri::True => {
                for entity in scoped_entities(model, check.scope.clone()) {
                    let tri = predicate_value(&check.requirement, &entity, model, resolved, plan);
                    let outcome = match tri {
                        Tri::True => Outcome::Pass,
                        Tri::False => match check.severity {
                            CheckSeverity::Required => Outcome::Violation,
                            CheckSeverity::Advisory => Outcome::Advisory,
                        },
                        Tri::Unknown => Outcome::NeedsReview,
                    };
                    entries.push(entry(
                        rule,
                        outcome.clone(),
                        Some(entity.element().clone()),
                        outcome_message(&check.title, &outcome),
                    ));
                }
            }
        }
    }

    entries.sort_by(|left, right| {
        left.rule
            .cmp(&right.rule)
            .then_with(|| left.element.cmp(&right.element))
    });
    ComplianceReport { entries }
}

pub fn diagnostics(report: &ComplianceReport) -> Vec<PlanDiagnostic> {
    report
        .entries
        .iter()
        .filter_map(|entry| {
            let severity = match entry.outcome {
                Outcome::Violation => DiagnosticSeverity::Violation,
                Outcome::Advisory => DiagnosticSeverity::Warning,
                Outcome::NeedsReview if entry.rule == BRACING_OUT_OF_DOMAIN => {
                    DiagnosticSeverity::Unsupported
                }
                Outcome::NeedsReview => DiagnosticSeverity::NeedsReview,
                Outcome::Pass | Outcome::NotApplicable | Outcome::Waived { .. } => return None,
            };
            Some(PlanDiagnostic {
                severity,
                code: entry.rule.clone(),
                source: entry.element.clone(),
                message: entry.message.clone(),
                rule: Some(RuleRef {
                    pack: entry.pack.clone(),
                    rule: entry.rule.clone(),
                    citation: entry.citation.clone(),
                }),
            })
        })
        .collect()
}

pub fn fact_value(
    fact: Fact,
    entity: &EntityRef,
    model: &BuildingModel,
    resolved: &ResolvedStandards,
    plan: &ProjectFramePlan,
) -> Option<FactValue> {
    match (fact, entity) {
        (Fact::WallLength, EntityRef::Wall(wall)) => {
            Some(FactValue::Length(find_wall(model, wall)?.length))
        }
        (Fact::WallHeight, EntityRef::Wall(wall)) => {
            Some(FactValue::Length(find_wall(model, wall)?.height))
        }
        (Fact::WallIsExterior, EntityRef::Wall(wall)) => {
            let wall = find_wall(model, wall)?;
            let system = model.system_for(wall)?;
            Some(FactValue::Flag(system.exposure() == WallExposure::Exterior))
        }
        (Fact::WallStudSpacing, EntityRef::Wall(wall)) => {
            let wall = find_wall(model, wall)?;
            let system = model.system_for(wall)?;
            Some(FactValue::Length(
                system.framing_layer()?.framing.as_ref()?.spacing,
            ))
        }
        (Fact::WallSystemRValueMilli, EntityRef::Wall(wall)) => {
            let wall = find_wall(model, wall)?;
            let system = model.system_for(wall)?;
            Some(FactValue::Int(system.r_value_milli(&model.materials)))
        }
        (Fact::WallStudMaxHeight, EntityRef::Wall(wall)) => {
            let wall = find_wall(model, wall)?;
            Some(FactValue::Length(wall_stud_max_height(
                wall.id.clone(),
                model,
                resolved,
            )?))
        }
        (Fact::OpeningRoughWidth, EntityRef::Opening(opening)) => {
            Some(FactValue::Length(find_opening(model, opening)?.1.width))
        }
        (Fact::OpeningRoughHeight, EntityRef::Opening(opening)) => {
            Some(FactValue::Length(find_opening(model, opening)?.1.height))
        }
        (Fact::OpeningHeaderDepth, EntityRef::Opening(opening)) => {
            let header = opening_headers(plan, opening).into_iter().next()?;
            Some(FactValue::Length(header.cross_section_depth))
        }
        (Fact::OpeningJackStuds, EntityRef::Opening(opening)) => {
            let count = opening_members(plan, opening)
                .into_iter()
                .filter(|member| member.kind == MemberKind::JackStud)
                .count()
                / 2;
            i64::try_from(count).ok().map(FactValue::Int)
        }
        (Fact::OpeningHeaderMaxSpan, EntityRef::Opening(opening)) => {
            let (_, opening_model) = find_opening(model, opening)?;
            Some(FactValue::Length(opening_header_max_span(
                opening_model,
                opening,
                model,
                resolved,
                plan,
            )?))
        }
        (Fact::RoomAreaSquareInches, EntityRef::Room(room)) => plan
            .rooms
            .iter()
            .find(|schedule| schedule.room == *room)
            .map(|schedule| FactValue::Int(schedule.area_square_inches)),
        (Fact::RoomCeilingHeight, EntityRef::Room(room)) => {
            let room = model.rooms.iter().find(|candidate| candidate.id == *room)?;
            let level = model
                .levels
                .iter()
                .find(|candidate| candidate.id == room.level)?;
            (level.height > Length::ZERO).then_some(FactValue::Length(level.height))
        }
        (Fact::BracedLineLength, EntityRef::BracedWallLine(line)) => Some(FactValue::Length(
            braced_line_length(find_braced_line(model, line)?)?,
        )),
        (Fact::BracedLineProvidedLength, EntityRef::BracedWallLine(line)) => {
            let line = find_braced_line(model, line)?;
            Some(FactValue::Length(braced_line_provided_length(model, line)))
        }
        (Fact::BracedLineRequiredLength, EntityRef::BracedWallLine(line)) => {
            let line = find_braced_line(model, line)?;
            match braced_line_required_length(line, model, resolved) {
                BracingRequirement::Known(length) => Some(FactValue::Length(length)),
                BracingRequirement::Unknown | BracingRequirement::OutOfDomain => None,
            }
        }
        _ => None,
    }
}

fn bracing_diagnostic_entries(
    model: &BuildingModel,
    resolved: &ResolvedStandards,
) -> Vec<ComplianceEntry> {
    let mut entries = Vec::new();
    let (pack, citation) = bracing_context(model, resolved);

    for (wall, panel) in bracing_panel_refs(model) {
        if associated_line_for_panel(model, wall, panel).is_none() {
            entries.push(ComplianceEntry {
                rule: BRACING_UNASSOCIATED_PANEL.to_owned(),
                citation: citation.clone(),
                pack: pack.clone(),
                outcome: Outcome::Advisory,
                element: Some(panel.id.clone()),
                message: format!(
                    "Braced panel {} is not associated with a parallel braced wall line within 4 ft.",
                    panel.id.0
                ),
                chain: Vec::new(),
            });
        }
    }

    for line in &model.braced_wall_lines {
        if braced_line_required_length(line, model, resolved) == BracingRequirement::OutOfDomain {
            entries.push(ComplianceEntry {
                rule: BRACING_OUT_OF_DOMAIN.to_owned(),
                citation: citation.clone(),
                pack: pack.clone(),
                outcome: Outcome::NeedsReview,
                element: Some(line.id.clone()),
                message: format!(
                    "Braced wall line {} is outside the resolved bracing table domain.",
                    line.id.0
                ),
                chain: Vec::new(),
            });
        }
    }

    entries
}

fn bracing_context(model: &BuildingModel, resolved: &ResolvedStandards) -> (ElementId, String) {
    resolved
        .bracing
        .first()
        .map(|(pack, table)| (pack.clone(), table.citation.clone()))
        .or_else(|| {
            model
                .standards
                .first()
                .map(|pack| (pack.clone(), String::new()))
        })
        .unwrap_or_else(|| (ElementId::new("standards"), String::new()))
}

fn entry(
    rule: &ResolvedRule,
    outcome: Outcome,
    element: Option<ElementId>,
    message: String,
) -> ComplianceEntry {
    ComplianceEntry {
        rule: rule.rule.clone(),
        citation: rule.citation.clone(),
        pack: rule.pack.clone(),
        outcome,
        element,
        message,
        chain: rule.chain.clone(),
    }
}

fn outcome_message(title: &str, outcome: &Outcome) -> String {
    match outcome {
        Outcome::Pass => format!("{title} passed."),
        Outcome::Violation => format!("{title} failed."),
        Outcome::Advisory => format!("{title} advisory failed."),
        Outcome::NeedsReview => format!("{title} needs review; one or more facts are unavailable."),
        Outcome::NotApplicable => format!("{title} is not applicable."),
        Outcome::Waived { reason } => format!("Waived: {reason}"),
    }
}

fn scoped_entities(model: &BuildingModel, scope: CheckScope) -> Vec<EntityRef> {
    match scope {
        CheckScope::Walls {
            exterior_only,
            tags,
        } => model
            .walls
            .iter()
            .filter(|wall| tags.iter().all(|tag| wall.tags.contains(tag)))
            .filter(|wall| {
                exterior_only.is_none_or(|expected| {
                    model
                        .system_for(wall)
                        .map(|system| (system.exposure() == WallExposure::Exterior) == expected)
                        .unwrap_or(false)
                })
            })
            .map(|wall| EntityRef::Wall(wall.id.clone()))
            .collect(),
        CheckScope::Openings { tags } => {
            if !tags.is_empty() {
                return Vec::new();
            }
            model
                .walls
                .iter()
                .flat_map(|wall| wall.openings.iter())
                .map(|opening| EntityRef::Opening(opening.id.clone()))
                .collect()
        }
        CheckScope::Rooms { tags } => model
            .rooms
            .iter()
            .filter(|room| tags.iter().all(|tag| room.tags.contains(tag)))
            .map(|room| EntityRef::Room(room.id.clone()))
            .collect(),
        CheckScope::BracedWallLines => model
            .braced_wall_lines
            .iter()
            .map(|line| EntityRef::BracedWallLine(line.id.clone()))
            .collect(),
    }
}

fn applicability(applies: Applicability, site: &SiteContext) -> Tri {
    match applies {
        Applicability::Always => Tri::True,
        Applicability::All(children) => {
            Tri::all(children.into_iter().map(|child| applicability(child, site)))
        }
        Applicability::Any(children) => {
            Tri::any(children.into_iter().map(|child| applicability(child, site)))
        }
        Applicability::Not(child) => applicability(*child, site).not(),
        Applicability::SeismicAtLeast(category) => site
            .seismic
            .map(|site_category| site_category >= category)
            .map(Tri::from)
            .unwrap_or(Tri::Unknown),
        Applicability::SeismicAtMost(category) => site
            .seismic
            .map(|site_category| site_category <= category)
            .map(Tri::from)
            .unwrap_or(Tri::Unknown),
        Applicability::WindSpeedAtLeast(speed) => site
            .wind_speed_mph
            .map(|site_speed| site_speed >= speed)
            .map(Tri::from)
            .unwrap_or(Tri::Unknown),
        Applicability::SnowLoadAtLeast(load) => site
            .ground_snow_load_psf
            .map(|site_load| site_load >= load)
            .map(Tri::from)
            .unwrap_or(Tri::Unknown),
        Applicability::SiteFlag { key } => match site.properties.get(&key) {
            Some(PropertyValue::Flag(value)) => Tri::from(*value),
            Some(_) | None => Tri::Unknown,
        },
    }
}

fn predicate_value(
    predicate: &Predicate,
    entity: &EntityRef,
    model: &BuildingModel,
    resolved: &ResolvedStandards,
    plan: &ProjectFramePlan,
) -> Tri {
    match predicate {
        Predicate::All(children) => Tri::all(
            children
                .iter()
                .map(|child| predicate_value(child, entity, model, resolved, plan)),
        ),
        Predicate::Any(children) => Tri::any(
            children
                .iter()
                .map(|child| predicate_value(child, entity, model, resolved, plan)),
        ),
        Predicate::Not(child) => predicate_value(child, entity, model, resolved, plan).not(),
        Predicate::Compare { fact, op, value } => {
            let Some(left) = fact_value(*fact, entity, model, resolved, plan) else {
                return Tri::Unknown;
            };
            let Some(right) = operand_value(value, entity, model, resolved, plan) else {
                return Tri::Unknown;
            };
            compare_fact_values(left, *op, right)
        }
    }
}

fn operand_value(
    value: &FactOperand,
    entity: &EntityRef,
    model: &BuildingModel,
    resolved: &ResolvedStandards,
    plan: &ProjectFramePlan,
) -> Option<FactValue> {
    match value {
        FactOperand::LengthLiteral(length) => Some(FactValue::Length(*length)),
        FactOperand::IntLiteral(value) => Some(FactValue::Int(*value)),
        FactOperand::FlagLiteral(value) => Some(FactValue::Flag(*value)),
        FactOperand::Fact(fact) => fact_value(*fact, entity, model, resolved, plan),
    }
}

fn compare_fact_values(left: FactValue, op: CompareOp, right: FactValue) -> Tri {
    match (left, right) {
        (FactValue::Length(left), FactValue::Length(right)) => compare_ord(left, op, right),
        (FactValue::Int(left), FactValue::Int(right)) => compare_ord(left, op, right),
        (FactValue::Flag(left), FactValue::Flag(right)) => match op {
            CompareOp::Eq => Tri::from(left == right),
            CompareOp::Ne => Tri::from(left != right),
            CompareOp::Lt | CompareOp::Le | CompareOp::Ge | CompareOp::Gt => Tri::Unknown,
        },
        _ => Tri::Unknown,
    }
}

fn compare_ord<T: Ord>(left: T, op: CompareOp, right: T) -> Tri {
    Tri::from(match op {
        CompareOp::Lt => left < right,
        CompareOp::Le => left <= right,
        CompareOp::Eq => left == right,
        CompareOp::Ge => left >= right,
        CompareOp::Gt => left > right,
        CompareOp::Ne => left != right,
    })
}

fn find_wall<'a>(model: &'a BuildingModel, wall: &ElementId) -> Option<&'a framer_core::Wall> {
    model.walls.iter().find(|candidate| candidate.id == *wall)
}

fn find_opening<'a>(
    model: &'a BuildingModel,
    opening: &ElementId,
) -> Option<(&'a framer_core::Wall, &'a Opening)> {
    model.walls.iter().find_map(|wall| {
        wall.openings
            .iter()
            .find(|candidate| candidate.id == *opening)
            .map(|opening| (wall, opening))
    })
}

fn wall_stud_max_height(
    wall_id: ElementId,
    model: &BuildingModel,
    resolved: &ResolvedStandards,
) -> Option<Length> {
    let wall = find_wall(model, &wall_id)?;
    let system = model.system_for(wall)?;
    let framing = system.framing_layer()?.framing.as_ref()?;
    let exterior = system.exposure() == WallExposure::Exterior;
    resolved
        .studs
        .iter()
        .flat_map(|(_, table)| table.rows.iter())
        .find(|row| row.profile == framing.member && row.spacing == framing.spacing)
        .map(|row| {
            if exterior {
                row.max_height_bearing
            } else {
                row.max_height_nonbearing
            }
        })
}

fn opening_members<'a>(plan: &'a ProjectFramePlan, opening: &ElementId) -> Vec<&'a FrameMember> {
    plan.wall_plans
        .iter()
        .flat_map(|wall| wall.members.iter())
        .filter(|member| member.source == *opening)
        .collect()
}

fn opening_headers<'a>(plan: &'a ProjectFramePlan, opening: &ElementId) -> Vec<&'a FrameMember> {
    opening_members(plan, opening)
        .into_iter()
        .filter(|member| member.kind == MemberKind::Header)
        .collect()
}

fn opening_header_max_span(
    opening_model: &Opening,
    opening: &ElementId,
    model: &BuildingModel,
    resolved: &ResolvedStandards,
    plan: &ProjectFramePlan,
) -> Option<Length> {
    let headers = opening_headers(plan, opening);
    let first = headers.first()?;
    let profile = first.profile;
    let plies = u8::try_from(headers.len()).ok()?;
    select_header_row(resolved, &model.site, opening_model.width, profile, plies)
        .map(|row| row.max_span)
}

fn select_header_row(
    resolved: &ResolvedStandards,
    site: &SiteContext,
    span: Length,
    profile: BoardProfile,
    plies: u8,
) -> Option<HeaderRow> {
    let rows = resolved
        .headers
        .iter()
        .flat_map(|(_, table)| table.rows.iter())
        .filter(|row| row.profile == profile && row.plies == plies)
        .collect::<Vec<_>>();
    let widest_width = rows.iter().map(|row| row.max_building_width).max()?;
    let highest_snow = rows.iter().map(|row| row.max_ground_snow_psf).max()?;

    rows.into_iter()
        .filter(|row| row.max_building_width == widest_width)
        .filter(|row| row.max_span >= span)
        .filter(|row| match site.ground_snow_load_psf {
            Some(load) => row.max_ground_snow_psf >= load,
            None => row.max_ground_snow_psf == highest_snow,
        })
        .min_by_key(|row| (row.max_span, row.jack_studs))
        .cloned()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BracingRequirement {
    Known(Length),
    Unknown,
    OutOfDomain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DistanceKey {
    cross_sq: i128,
    line_len_sq: i128,
}

impl Ord for DistanceKey {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.cross_sq * other.line_len_sq).cmp(&(other.cross_sq * self.line_len_sq))
    }
}

impl PartialOrd for DistanceKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn bracing_panel_refs(model: &BuildingModel) -> Vec<(&Wall, &BracedPanel)> {
    model
        .walls
        .iter()
        .flat_map(|wall| wall.bracing.iter().map(move |panel| (wall, panel)))
        .collect()
}

fn find_braced_line<'a>(model: &'a BuildingModel, line: &ElementId) -> Option<&'a BracedWallLine> {
    model
        .braced_wall_lines
        .iter()
        .find(|candidate| candidate.id == *line)
}

fn braced_line_length(line: &BracedWallLine) -> Option<Length> {
    if line.start.y == line.end.y && line.start.x != line.end.x {
        Some((line.end.x - line.start.x).abs())
    } else if line.start.x == line.end.x && line.start.y != line.end.y {
        Some((line.end.y - line.start.y).abs())
    } else {
        None
    }
}

fn braced_line_provided_length(model: &BuildingModel, line: &BracedWallLine) -> Length {
    associated_panels_for_line(model, line)
        .into_iter()
        .fold(Length::ZERO, |sum, (_, panel)| sum + panel.length)
}

fn braced_line_required_length(
    line: &BracedWallLine,
    model: &BuildingModel,
    resolved: &ResolvedStandards,
) -> BracingRequirement {
    let Some(line_length) = braced_line_length(line) else {
        return BracingRequirement::Unknown;
    };
    let methods = associated_panels_for_line(model, line)
        .into_iter()
        .map(|(_, panel)| panel.method)
        .collect::<BTreeSet<_>>();
    if methods.is_empty() {
        return BracingRequirement::Unknown;
    }

    let mut required = Length::ZERO;
    let mut unknown = false;
    let mut out_of_domain = false;
    for method in methods {
        match bracing_required_for_method(method, line_length, &model.site, resolved) {
            BracingRequirement::Known(length) => required = required.max(length),
            BracingRequirement::Unknown => unknown = true,
            BracingRequirement::OutOfDomain => out_of_domain = true,
        }
    }

    if out_of_domain {
        BracingRequirement::OutOfDomain
    } else if unknown {
        BracingRequirement::Unknown
    } else {
        BracingRequirement::Known(required)
    }
}

fn bracing_required_for_method(
    method: BracingMethod,
    line_length: Length,
    site: &SiteContext,
    resolved: &ResolvedStandards,
) -> BracingRequirement {
    let rows = resolved
        .bracing
        .iter()
        .flat_map(|(_, table)| table.rows.iter())
        .filter(|row| row.method == method)
        .filter(|row| row.line_length >= line_length)
        .collect::<Vec<_>>();
    if rows.is_empty() {
        return BracingRequirement::OutOfDomain;
    }
    if site.seismic.is_none() && rows.iter().any(|row| row.max_seismic.is_some()) {
        return BracingRequirement::Unknown;
    }
    if site.wind_speed_mph.is_none() && rows.iter().any(|row| row.max_wind_speed_mph.is_some()) {
        return BracingRequirement::Unknown;
    }

    rows.into_iter()
        .filter(|row| bracing_row_matches_site(row, site))
        .min_by_key(|row| {
            (
                row.line_length,
                row.required_length,
                row.max_seismic,
                row.max_wind_speed_mph,
            )
        })
        .map(|row| BracingRequirement::Known(row.required_length))
        .unwrap_or(BracingRequirement::OutOfDomain)
}

fn bracing_row_matches_site(row: &BracingRow, site: &SiteContext) -> bool {
    let seismic_matches = row
        .max_seismic
        .is_none_or(|max| site.seismic.is_some_and(|site| max >= site));
    let wind_matches = row
        .max_wind_speed_mph
        .is_none_or(|max| site.wind_speed_mph.is_some_and(|site| max >= site));
    seismic_matches && wind_matches
}

fn associated_panels_for_line<'a>(
    model: &'a BuildingModel,
    line: &BracedWallLine,
) -> Vec<(&'a Wall, &'a BracedPanel)> {
    bracing_panel_refs(model)
        .into_iter()
        .filter(|(wall, panel)| {
            associated_line_for_panel(model, wall, panel)
                .is_some_and(|candidate| candidate.id == line.id)
        })
        .collect()
}

fn associated_line_for_panel<'a>(
    model: &'a BuildingModel,
    wall: &Wall,
    panel: &BracedPanel,
) -> Option<&'a BracedWallLine> {
    let wall_direction = direction(wall.start, wall.end);
    if wall_direction == (0, 0) {
        return None;
    }
    let midpoint = wall.point_at_local_x(panel.offset + panel.length / 2);

    model
        .braced_wall_lines
        .iter()
        .filter(|line| line.level == wall.level)
        .filter(|line| {
            let line_direction = direction(line.start, line.end);
            line_direction != (0, 0) && cross(wall_direction, line_direction) == 0
        })
        .filter_map(|line| {
            let distance = distance_to_line(midpoint, line)?;
            (distance.within(BRACING_ASSOCIATION_TOLERANCE)).then_some((distance, &line.id, line))
        })
        .min_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(right.1)))
        .map(|(_, _, line)| line)
}

impl DistanceKey {
    fn within(self, tolerance: Length) -> bool {
        let tolerance = i128::from(tolerance.ticks());
        self.cross_sq <= tolerance * tolerance * self.line_len_sq
    }
}

fn distance_to_line(point: framer_core::Point2, line: &BracedWallLine) -> Option<DistanceKey> {
    let line_direction = direction(line.start, line.end);
    let line_len_sq = dot(line_direction, line_direction);
    if line_len_sq == 0 {
        return None;
    }
    let offset = direction(line.start, point);
    let cross = cross(line_direction, offset);
    Some(DistanceKey {
        cross_sq: cross * cross,
        line_len_sq,
    })
}

fn direction(start: framer_core::Point2, end: framer_core::Point2) -> (i128, i128) {
    (
        i128::from((end.x - start.x).ticks()),
        i128::from((end.y - start.y).ticks()),
    )
}

fn cross(left: (i128, i128), right: (i128, i128)) -> i128 {
    left.0 * right.1 - left.1 * right.0
}

fn dot(left: (i128, i128), right: (i128, i128)) -> i128 {
    left.0 * right.0 + left.1 * right.1
}

fn csv_field(value: &str) -> String {
    if value.contains([',', '"', '\n']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use framer_core::{
        BracedPanel, BracedWallLine, BracingMethod, ComplianceCheck, FramingDefaults, Point2, Room,
        RoomUsage, SeismicDesignCategory, StandardsPack, Wall,
    };
    use framer_solver::generate_project_plan;

    use super::*;

    #[test]
    fn tri_uses_kleene_truth_tables() {
        assert_eq!(Tri::False.not(), Tri::True);
        assert_eq!(Tri::Unknown.not(), Tri::Unknown);
        assert_eq!(Tri::True.not(), Tri::False);
        assert_eq!(Tri::all([]), Tri::True);
        assert_eq!(Tri::all([Tri::True, Tri::Unknown]), Tri::Unknown);
        assert_eq!(Tri::all([Tri::True, Tri::False]), Tri::False);
        assert_eq!(Tri::any([]), Tri::False);
        assert_eq!(Tri::any([Tri::False, Tri::Unknown]), Tri::Unknown);
        assert_eq!(Tri::any([Tri::False, Tri::True]), Tri::True);
    }

    #[test]
    fn applicability_unknown_site_values_need_review() {
        let site = SiteContext::default();

        assert_eq!(
            applicability(
                Applicability::SeismicAtLeast(SeismicDesignCategory::D0),
                &site
            ),
            Tri::Unknown
        );
        assert_eq!(
            applicability(
                Applicability::SiteFlag {
                    key: "sprinklers".to_owned()
                },
                &site
            ),
            Tri::Unknown
        );
    }

    #[test]
    fn wall_facts_report_known_values_and_unknown_table_misses() {
        let model = one_wall_model(Length::from_feet(8.0));
        let plan = generate_project_plan(&model).unwrap();
        let mut resolved = model.resolved_standards();
        let wall = EntityRef::Wall(ElementId::new("wall"));

        assert_eq!(
            fact_value(Fact::WallLength, &wall, &model, &resolved, &plan),
            Some(FactValue::Length(Length::from_feet(8.0)))
        );
        assert_eq!(
            fact_value(Fact::WallIsExterior, &wall, &model, &resolved, &plan),
            Some(FactValue::Flag(true))
        );

        resolved.studs.clear();
        assert_eq!(
            fact_value(Fact::WallStudMaxHeight, &wall, &model, &resolved, &plan),
            None
        );
    }

    #[test]
    fn bracing_association_uses_parallel_tolerance_and_tie_break() {
        let mut model = braced_line_model(Length::from_feet(20.0));
        model.walls[0].bracing = vec![braced_panel(
            "panel",
            Length::from_feet(4.0),
            Length::from_feet(4.0),
            BracingMethod::Wsp,
        )];
        model.braced_wall_lines = vec![
            braced_line("bwl-b", Length::from_feet(20.0), Length::from_feet(2.0)),
            braced_line(
                "bwl-far",
                Length::from_feet(20.0),
                Length::from_whole_inches(49),
            ),
            BracedWallLine {
                id: ElementId::new("bwl-cross"),
                name: "Cross line".to_owned(),
                level: ElementId::new("level-1"),
                start: Point2::new(Length::from_feet(4.0), Length::ZERO),
                end: Point2::new(Length::from_feet(4.0), Length::from_feet(20.0)),
            },
            braced_line("bwl-a", Length::from_feet(20.0), Length::from_feet(-2.0)),
        ];

        let associated =
            associated_line_for_panel(&model, &model.walls[0], &model.walls[0].bracing[0])
                .expect("associated braced wall line");
        assert_eq!(associated.id, ElementId::new("bwl-a"));

        model.braced_wall_lines = vec![braced_line(
            "bwl-too-far",
            Length::from_feet(20.0),
            Length::from_whole_inches(49),
        )];
        assert!(
            associated_line_for_panel(&model, &model.walls[0], &model.walls[0].bracing[0])
                .is_none()
        );
    }

    #[test]
    fn braced_line_facts_use_associated_panels_and_sdc_bands() {
        let mut model = braced_line_model(Length::from_feet(20.0));
        model.site.seismic = Some(SeismicDesignCategory::C);
        model.walls[0].bracing = vec![braced_panel(
            "panel",
            Length::from_feet(4.0),
            Length::from_feet(4.0),
            BracingMethod::Wsp,
        )];
        let plan = generate_project_plan(&model).unwrap();
        let resolved = model.resolved_standards();
        let line = EntityRef::BracedWallLine(ElementId::new("bwl"));

        assert_eq!(
            fact_value(Fact::BracedLineLength, &line, &model, &resolved, &plan),
            Some(FactValue::Length(Length::from_feet(20.0)))
        );
        assert_eq!(
            fact_value(
                Fact::BracedLineProvidedLength,
                &line,
                &model,
                &resolved,
                &plan
            ),
            Some(FactValue::Length(Length::from_feet(4.0)))
        );
        assert_eq!(
            fact_value(
                Fact::BracedLineRequiredLength,
                &line,
                &model,
                &resolved,
                &plan
            ),
            Some(FactValue::Length(Length::from_feet(4.0)))
        );

        model.site.seismic = Some(SeismicDesignCategory::D2);
        let resolved = model.resolved_standards();
        assert_eq!(
            fact_value(
                Fact::BracedLineRequiredLength,
                &line,
                &model,
                &resolved,
                &plan
            ),
            Some(FactValue::Length(Length::from_feet(6.0)))
        );
    }

    #[test]
    fn braced_line_required_length_uses_multi_method_max() {
        let mut model = braced_line_model(Length::from_feet(20.0));
        model.site.seismic = Some(SeismicDesignCategory::D2);
        model.walls[0].bracing = vec![
            braced_panel(
                "panel-wsp",
                Length::from_feet(2.0),
                Length::from_feet(4.0),
                BracingMethod::Wsp,
            ),
            braced_panel(
                "panel-gb",
                Length::from_feet(8.0),
                Length::from_feet(4.0),
                BracingMethod::Gb,
            ),
        ];
        let plan = generate_project_plan(&model).unwrap();
        let resolved = model.resolved_standards();
        let line = EntityRef::BracedWallLine(ElementId::new("bwl"));

        assert_eq!(
            fact_value(
                Fact::BracedLineRequiredLength,
                &line,
                &model,
                &resolved,
                &plan
            ),
            Some(FactValue::Length(Length::from_feet(8.0)))
        );
    }

    #[test]
    fn unknown_sdc_turns_bracing_checks_into_needs_review() {
        let mut model = braced_line_model(Length::from_feet(20.0));
        model.site.seismic = None;
        model.walls[0].bracing = vec![braced_panel(
            "panel",
            Length::from_feet(4.0),
            Length::from_feet(4.0),
            BracingMethod::Wsp,
        )];
        let resolved = model.resolved_standards();
        let plan = generate_project_plan(&model).unwrap();
        let report = evaluate(&model, &resolved, &plan);

        assert!(has_outcome(
            &report,
            "irc2021.r602.10.braced-length",
            &Outcome::NeedsReview
        ));
        assert!(diagnostics(&report).iter().any(|diagnostic| {
            diagnostic.code == "irc2021.r602.10.braced-length"
                && diagnostic.severity == DiagnosticSeverity::NeedsReview
                && diagnostic.source.as_ref().map(|id| id.0.as_str()) == Some("bwl")
        }));
    }

    #[test]
    fn bracing_out_of_domain_lowers_to_unsupported_diagnostic() {
        let mut model = braced_line_model(Length::from_feet(50.0));
        model.site.seismic = Some(SeismicDesignCategory::C);
        model.walls[0].bracing = vec![braced_panel(
            "panel",
            Length::from_feet(4.0),
            Length::from_feet(4.0),
            BracingMethod::Wsp,
        )];
        let resolved = model.resolved_standards();
        let plan = generate_project_plan(&model).unwrap();
        let report = evaluate(&model, &resolved, &plan);

        assert!(diagnostics(&report).iter().any(|diagnostic| {
            diagnostic.code == BRACING_OUT_OF_DOMAIN
                && diagnostic.severity == DiagnosticSeverity::Unsupported
                && diagnostic.source.as_ref().map(|id| id.0.as_str()) == Some("bwl")
        }));
    }

    #[test]
    fn unassociated_bracing_panels_emit_advisory_diagnostics() {
        let mut model = one_wall_model(Length::from_feet(20.0));
        model.walls[0].bracing = vec![braced_panel(
            "panel",
            Length::from_feet(4.0),
            Length::from_feet(4.0),
            BracingMethod::Wsp,
        )];
        let resolved = model.resolved_standards();
        let plan = generate_project_plan(&model).unwrap();
        let report = evaluate(&model, &resolved, &plan);

        assert_eq!(evaluate(&model, &resolved, &plan), report);
        assert!(diagnostics(&report).iter().any(|diagnostic| {
            diagnostic.code == BRACING_UNASSOCIATED_PANEL
                && diagnostic.severity == DiagnosticSeverity::Warning
                && diagnostic.source.as_ref().map(|id| id.0.as_str()) == Some("panel")
        }));
    }

    #[test]
    fn evaluate_maps_required_advisory_unknown_and_waived_outcomes() {
        let mut model = one_wall_model(Length::from_feet(8.0));
        model.rooms.push(Room::new(
            "room",
            "Room",
            RoomUsage::Living,
            "level-1",
            Point2::new(Length::from_feet(1.0), Length::from_feet(1.0)),
        ));
        model.rooms[0].tags.push("habitable".to_owned());
        let mut pack = StandardsPack::irc_2021_starter();
        pack.checks = vec![
            wall_check(
                "test.wall.pass",
                CheckSeverity::Required,
                CompareOp::Le,
                FactOperand::LengthLiteral(Length::from_feet(12.0)),
            ),
            wall_check(
                "test.wall.violation",
                CheckSeverity::Required,
                CompareOp::Gt,
                FactOperand::LengthLiteral(Length::from_feet(20.0)),
            ),
            wall_check(
                "test.wall.advisory",
                CheckSeverity::Advisory,
                CompareOp::Gt,
                FactOperand::LengthLiteral(Length::from_feet(20.0)),
            ),
            ComplianceCheck {
                rule: "test.room.unknown".to_owned(),
                citation: "Test".to_owned(),
                title: "Room unknown".to_owned(),
                severity: CheckSeverity::Required,
                applies: Applicability::Always,
                scope: CheckScope::Rooms {
                    tags: vec!["habitable".to_owned()],
                },
                requirement: Predicate::Compare {
                    fact: Fact::RoomCeilingHeight,
                    op: CompareOp::Ge,
                    value: FactOperand::LengthLiteral(Length::from_feet(7.0)),
                },
            },
            wall_check(
                "test.wall.waived",
                CheckSeverity::Required,
                CompareOp::Gt,
                FactOperand::LengthLiteral(Length::from_feet(20.0)),
            ),
        ];
        pack.overlays.push(framer_core::RuleOverlay::Waive {
            target: "test.wall.waived".to_owned(),
            reason: "accepted by AHJ".to_owned(),
        });
        model.standards = vec![pack.id.clone()];
        model.standards_packs = vec![pack];
        let resolved = model.resolved_standards();
        let plan = generate_project_plan(&model).unwrap();
        let report = evaluate(&model, &resolved, &plan);

        assert!(has_outcome(&report, "test.wall.pass", &Outcome::Pass));
        assert!(has_outcome(
            &report,
            "test.wall.violation",
            &Outcome::Violation
        ));
        assert!(has_outcome(
            &report,
            "test.wall.advisory",
            &Outcome::Advisory
        ));
        assert!(has_outcome(
            &report,
            "test.room.unknown",
            &Outcome::NeedsReview
        ));
        assert!(report.entries.iter().any(|entry| matches!(
            &entry.outcome,
            Outcome::Waived { reason } if reason == "accepted by AHJ"
        )));

        let diagnostics = diagnostics(&report);
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.severity == DiagnosticSeverity::Violation
                && diagnostic.rule.as_ref().map(|rule| rule.rule.as_str())
                    == Some("test.wall.violation")
        }));
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.severity == DiagnosticSeverity::NeedsReview
                && diagnostic.code == "test.room.unknown"
        }));
    }

    #[test]
    fn report_csv_is_deterministic_and_escaped() {
        let model = one_wall_model(Length::from_feet(8.0));
        let mut pack = StandardsPack::irc_2021_starter();
        pack.checks = vec![wall_check(
            "test.wall.pass",
            CheckSeverity::Required,
            CompareOp::Le,
            FactOperand::LengthLiteral(Length::from_feet(12.0)),
        )];
        let mut model = model;
        model.standards = vec![pack.id.clone()];
        model.standards_packs = vec![pack];
        let resolved = model.resolved_standards();
        let plan = generate_project_plan(&model).unwrap();

        let first = evaluate(&model, &resolved, &plan).to_csv();
        let second = evaluate(&model, &resolved, &plan).to_csv();

        assert_eq!(first, second);
        assert!(first.starts_with("rule,citation,pack,outcome,element,message,chain\n"));
        assert!(first.contains("test.wall.pass,Test,std-irc-2021,Pass,wall"));
    }

    fn one_wall_model(length: Length) -> BuildingModel {
        let defaults = FramingDefaults::irc_2021_starter();
        let mut model = BuildingModel::new();
        model.walls = vec![Wall::new("wall", "Wall", length, &defaults)];
        model
    }

    fn braced_line_model(length: Length) -> BuildingModel {
        let mut model = one_wall_model(length);
        model.braced_wall_lines = vec![braced_line("bwl", length, Length::ZERO)];
        model
    }

    fn braced_line(id: &str, length: Length, y: Length) -> BracedWallLine {
        BracedWallLine {
            id: ElementId::new(id),
            name: id.to_owned(),
            level: ElementId::new("level-1"),
            start: Point2::new(Length::ZERO, y),
            end: Point2::new(length, y),
        }
    }

    fn braced_panel(
        id: &str,
        offset: Length,
        length: Length,
        method: BracingMethod,
    ) -> BracedPanel {
        BracedPanel {
            id: ElementId::new(id),
            offset,
            length,
            method,
        }
    }

    fn wall_check(
        rule: &str,
        severity: CheckSeverity,
        op: CompareOp,
        value: FactOperand,
    ) -> ComplianceCheck {
        ComplianceCheck {
            rule: rule.to_owned(),
            citation: "Test".to_owned(),
            title: rule.to_owned(),
            severity,
            applies: Applicability::Always,
            scope: CheckScope::Walls {
                exterior_only: None,
                tags: Vec::new(),
            },
            requirement: Predicate::Compare {
                fact: Fact::WallLength,
                op,
                value,
            },
        }
    }

    fn has_outcome(report: &ComplianceReport, rule: &str, outcome: &Outcome) -> bool {
        report
            .entries
            .iter()
            .any(|entry| entry.rule == rule && &entry.outcome == outcome)
    }
}
