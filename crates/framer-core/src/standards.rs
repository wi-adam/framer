use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::model::validate_element_id;
use crate::{BoardProfile, ElementId, Length, ModelError, Point2, PropertyValue, Provenance};

fn map_is_empty<K, V>(map: &BTreeMap<K, V>) -> bool {
    map.is_empty()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum SeismicDesignCategory {
    A,
    B,
    C,
    D0,
    D1,
    D2,
    E,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SiteContext {
    pub jurisdiction: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seismic: Option<SeismicDesignCategory>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wind_speed_mph: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ground_snow_load_psf: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frost_depth: Option<Length>,
    #[serde(default, skip_serializing_if = "map_is_empty")]
    pub properties: BTreeMap<String, PropertyValue>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum BracingMethod {
    Lib,
    Dwb,
    Wsp,
    Sfb,
    Gb,
    Pcp,
    Hps,
    CsWsp,
}

impl BracingMethod {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Lib => "LIB",
            Self::Dwb => "DWB",
            Self::Wsp => "WSP",
            Self::Sfb => "SFB",
            Self::Gb => "GB",
            Self::Pcp => "PCP",
            Self::Hps => "HPS",
            Self::CsWsp => "CS-WSP",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BracedWallLine {
    pub id: ElementId,
    pub name: String,
    pub level: ElementId,
    pub start: Point2,
    pub end: Point2,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BracedPanel {
    pub id: ElementId,
    pub offset: Length,
    pub length: Length,
    pub method: BracingMethod,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FramingDefaults {
    pub default_wall_height: Length,
    pub default_stud_spacing: Length,
    pub double_top_plate: bool,
    pub default_header_depth: Length,
    pub stud_profile: BoardProfile,
    pub plate_profile: BoardProfile,
    pub header_profile: BoardProfile,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StudTable {
    pub rule: String,
    pub citation: String,
    pub rows: Vec<StudRow>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StudRow {
    pub profile: BoardProfile,
    pub spacing: Length,
    pub max_height_bearing: Length,
    pub max_height_nonbearing: Length,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeaderSpanTable {
    pub rule: String,
    pub citation: String,
    pub rows: Vec<HeaderRow>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeaderRow {
    pub profile: BoardProfile,
    pub plies: u8,
    pub max_ground_snow_psf: u32,
    pub max_building_width: Length,
    pub max_span: Length,
    pub jack_studs: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum ConnectionKind {
    StudToPlateEnd,
    StudToPlateToe,
    TopPlateLap,
    DoubleTopPlate,
    SolePlateToJoist,
    HeaderToKingStud,
    SheathingEdge,
    SheathingField,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum FastenerSchedule {
    Count(u32),
    Spacing { on_center: Length },
    EdgeField { edge: Length, field: Length },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FasteningSchedule {
    pub rule: String,
    pub citation: String,
    pub rows: Vec<FasteningRow>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FasteningRow {
    pub connection: ConnectionKind,
    pub fastener: String,
    pub schedule: FastenerSchedule,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BracingTable {
    pub rule: String,
    pub citation: String,
    pub rows: Vec<BracingRow>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BracingRow {
    pub method: BracingMethod,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_seismic: Option<SeismicDesignCategory>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_wind_speed_mph: Option<u32>,
    pub line_length: Length,
    pub required_length: Length,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StandardsTables {
    pub defaults: FramingDefaults,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub studs: Vec<StudTable>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<HeaderSpanTable>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fastening: Vec<FasteningSchedule>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bracing: Vec<BracingTable>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum CheckSeverity {
    Required,
    Advisory,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Applicability {
    Always,
    All(Vec<Applicability>),
    Any(Vec<Applicability>),
    Not(Box<Applicability>),
    SeismicAtLeast(SeismicDesignCategory),
    SeismicAtMost(SeismicDesignCategory),
    WindSpeedAtLeast(u32),
    SnowLoadAtLeast(u32),
    SiteFlag { key: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Fact {
    WallLength,
    WallHeight,
    WallIsExterior,
    WallStudSpacing,
    WallSystemRValueMilli,
    WallStudMaxHeight,
    OpeningRoughWidth,
    OpeningRoughHeight,
    OpeningHeaderDepth,
    OpeningJackStuds,
    OpeningHeaderMaxSpan,
    RoomAreaSquareInches,
    RoomCeilingHeight,
    BracedLineLength,
    BracedLineRequiredLength,
    BracedLineProvidedLength,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum FactType {
    Length,
    Int,
    Flag,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FactScope {
    Wall,
    Opening,
    Room,
    BracedWallLine,
}

impl FactScope {
    const fn label(self) -> &'static str {
        match self {
            Self::Wall => "Walls",
            Self::Opening => "Openings",
            Self::Room => "Rooms",
            Self::BracedWallLine => "BracedWallLines",
        }
    }
}

impl Fact {
    pub const fn value_type(self) -> FactType {
        match self {
            Self::WallLength
            | Self::WallHeight
            | Self::WallStudSpacing
            | Self::WallStudMaxHeight
            | Self::OpeningRoughWidth
            | Self::OpeningRoughHeight
            | Self::OpeningHeaderDepth
            | Self::OpeningHeaderMaxSpan
            | Self::RoomCeilingHeight
            | Self::BracedLineLength
            | Self::BracedLineRequiredLength
            | Self::BracedLineProvidedLength => FactType::Length,
            Self::WallSystemRValueMilli | Self::OpeningJackStuds | Self::RoomAreaSquareInches => {
                FactType::Int
            }
            Self::WallIsExterior => FactType::Flag,
        }
    }

    const fn scope(self) -> FactScope {
        match self {
            Self::WallLength
            | Self::WallHeight
            | Self::WallIsExterior
            | Self::WallStudSpacing
            | Self::WallSystemRValueMilli
            | Self::WallStudMaxHeight => FactScope::Wall,
            Self::OpeningRoughWidth
            | Self::OpeningRoughHeight
            | Self::OpeningHeaderDepth
            | Self::OpeningJackStuds
            | Self::OpeningHeaderMaxSpan => FactScope::Opening,
            Self::RoomAreaSquareInches | Self::RoomCeilingHeight => FactScope::Room,
            Self::BracedLineLength
            | Self::BracedLineRequiredLength
            | Self::BracedLineProvidedLength => FactScope::BracedWallLine,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum CompareOp {
    Lt,
    Le,
    Eq,
    Ge,
    Gt,
    Ne,
}

impl CompareOp {
    const fn label(self) -> &'static str {
        match self {
            Self::Lt => "<",
            Self::Le => "<=",
            Self::Eq => "==",
            Self::Ge => ">=",
            Self::Gt => ">",
            Self::Ne => "!=",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum FactOperand {
    LengthLiteral(Length),
    IntLiteral(i64),
    FlagLiteral(bool),
    Fact(Fact),
}

impl FactOperand {
    const fn value_type(&self) -> FactType {
        match self {
            Self::LengthLiteral(_) => FactType::Length,
            Self::IntLiteral(_) => FactType::Int,
            Self::FlagLiteral(_) => FactType::Flag,
            Self::Fact(fact) => fact.value_type(),
        }
    }

    const fn fact(&self) -> Option<Fact> {
        match self {
            Self::Fact(fact) => Some(*fact),
            Self::LengthLiteral(_) | Self::IntLiteral(_) | Self::FlagLiteral(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Predicate {
    All(Vec<Predicate>),
    Any(Vec<Predicate>),
    Not(Box<Predicate>),
    Compare {
        fact: Fact,
        op: CompareOp,
        value: FactOperand,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum CheckScope {
    Walls {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exterior_only: Option<bool>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        tags: Vec<String>,
    },
    Openings {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        tags: Vec<String>,
    },
    Rooms {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        tags: Vec<String>,
    },
    BracedWallLines,
}

impl CheckScope {
    const fn fact_scope(&self) -> FactScope {
        match self {
            Self::Walls { .. } => FactScope::Wall,
            Self::Openings { .. } => FactScope::Opening,
            Self::Rooms { .. } => FactScope::Room,
            Self::BracedWallLines => FactScope::BracedWallLine,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComplianceCheck {
    pub rule: String,
    pub citation: String,
    pub title: String,
    pub severity: CheckSeverity,
    pub applies: Applicability,
    pub scope: CheckScope,
    pub requirement: Predicate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum RuleOverlay {
    Waive {
        target: String,
        reason: String,
    },
    Severity {
        target: String,
        severity: CheckSeverity,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StandardsPack {
    pub id: ElementId,
    pub name: String,
    pub edition: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Provenance>,
    pub tables: StandardsTables,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checks: Vec<ComplianceCheck>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub overlays: Vec<RuleOverlay>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "map_is_empty")]
    pub properties: BTreeMap<String, PropertyValue>,
}

impl StandardsPack {
    pub fn validate(&self) -> Result<(), ModelError> {
        validate_element_id(&self.id)?;
        let mut rules = BTreeSet::new();
        for table in &self.tables.studs {
            validate_rule_decl(&mut rules, &table.rule)?;
            validate_strict_rows(&table.rule, &table.rows, |row| (row.profile, row.spacing))?;
        }
        for table in &self.tables.headers {
            validate_rule_decl(&mut rules, &table.rule)?;
            validate_strict_rows(&table.rule, &table.rows, |row| {
                (
                    row.profile,
                    row.plies,
                    row.max_ground_snow_psf,
                    row.max_building_width,
                )
            })?;
        }
        for schedule in &self.tables.fastening {
            validate_rule_decl(&mut rules, &schedule.rule)?;
            validate_strict_rows(&schedule.rule, &schedule.rows, |row| row.connection)?;
        }
        for table in &self.tables.bracing {
            validate_rule_decl(&mut rules, &table.rule)?;
            validate_strict_rows(&table.rule, &table.rows, |row| {
                (
                    row.method,
                    row.max_seismic,
                    row.max_wind_speed_mph,
                    row.line_length,
                )
            })?;
        }
        for check in &self.checks {
            validate_rule_decl(&mut rules, &check.rule)?;
            validate_predicate(&check.rule, &check.requirement, check.scope.fact_scope())?;
        }
        for overlay in &self.overlays {
            if let RuleOverlay::Waive { target, reason } = overlay
                && reason.trim().is_empty()
            {
                return Err(ModelError::StandardsOverlayMissingReason {
                    target: target.clone(),
                });
            }
        }
        Ok(())
    }

    pub fn irc_2021_starter() -> Self {
        Self {
            id: ElementId::new("std-irc-2021"),
            name: "IRC 2021 Prescriptive (starter)".to_owned(),
            edition: "2021".to_owned(),
            source: None,
            tables: StandardsTables {
                defaults: FramingDefaults::irc_2021_starter(),
                studs: vec![StudTable {
                    rule: "irc2021.r602.3-5.studs".to_owned(),
                    citation: "IRC 2021 Table R602.3(5)".to_owned(),
                    rows: vec![
                        StudRow {
                            profile: BoardProfile::TwoByFour,
                            spacing: Length::from_whole_inches(16),
                            max_height_bearing: Length::from_feet(10.0),
                            max_height_nonbearing: Length::from_feet(14.0),
                        },
                        StudRow {
                            profile: BoardProfile::TwoByFour,
                            spacing: Length::from_whole_inches(24),
                            max_height_bearing: Length::from_feet(10.0),
                            max_height_nonbearing: Length::from_feet(12.0),
                        },
                        StudRow {
                            profile: BoardProfile::TwoBySix,
                            spacing: Length::from_whole_inches(16),
                            max_height_bearing: Length::from_feet(14.0),
                            max_height_nonbearing: Length::from_feet(20.0),
                        },
                        StudRow {
                            profile: BoardProfile::TwoBySix,
                            spacing: Length::from_whole_inches(24),
                            max_height_bearing: Length::from_feet(12.0),
                            max_height_nonbearing: Length::from_feet(18.0),
                        },
                    ],
                }],
                headers: vec![HeaderSpanTable {
                    rule: "irc2021.r602.7-1.headers".to_owned(),
                    citation: "IRC 2021 Table R602.7(1)".to_owned(),
                    rows: vec![
                        HeaderRow {
                            profile: BoardProfile::TwoByTen,
                            plies: 1,
                            max_ground_snow_psf: 30,
                            max_building_width: Length::from_feet(36.0),
                            max_span: Length::from_feet(4.0),
                            jack_studs: 1,
                        },
                        HeaderRow {
                            profile: BoardProfile::TwoByTen,
                            plies: 2,
                            max_ground_snow_psf: 30,
                            max_building_width: Length::from_feet(36.0),
                            max_span: Length::from_feet(6.0),
                            jack_studs: 1,
                        },
                        HeaderRow {
                            profile: BoardProfile::TwoByTwelve,
                            plies: 1,
                            max_ground_snow_psf: 30,
                            max_building_width: Length::from_feet(36.0),
                            max_span: Length::from_feet(5.0),
                            jack_studs: 1,
                        },
                        HeaderRow {
                            profile: BoardProfile::TwoByTwelve,
                            plies: 2,
                            max_ground_snow_psf: 30,
                            max_building_width: Length::from_feet(36.0),
                            max_span: Length::from_feet(8.0),
                            jack_studs: 2,
                        },
                    ],
                }],
                fastening: vec![FasteningSchedule {
                    rule: "irc2021.r602.3-1.fastening".to_owned(),
                    citation: "IRC 2021 Table R602.3(1)".to_owned(),
                    rows: vec![
                        FasteningRow {
                            connection: ConnectionKind::StudToPlateEnd,
                            fastener: "16d common nail".to_owned(),
                            schedule: FastenerSchedule::Count(2),
                        },
                        FasteningRow {
                            connection: ConnectionKind::StudToPlateToe,
                            fastener: "8d common nail".to_owned(),
                            schedule: FastenerSchedule::Count(4),
                        },
                        FasteningRow {
                            connection: ConnectionKind::TopPlateLap,
                            fastener: "16d common nail".to_owned(),
                            schedule: FastenerSchedule::Count(8),
                        },
                        FasteningRow {
                            connection: ConnectionKind::DoubleTopPlate,
                            fastener: "16d common nail".to_owned(),
                            schedule: FastenerSchedule::Spacing {
                                on_center: Length::from_whole_inches(16),
                            },
                        },
                    ],
                }],
                bracing: Vec::new(),
            },
            checks: Vec::new(),
            overlays: Vec::new(),
            tags: Vec::new(),
            properties: BTreeMap::new(),
        }
    }
}

impl FramingDefaults {
    pub fn irc_2021_starter() -> Self {
        Self {
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
#[serde(deny_unknown_fields)]
pub enum ResolutionAction {
    Introduced,
    Shadowed,
    Waived,
    Reseverity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedRule {
    pub pack: ElementId,
    pub rule: String,
    pub citation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub severity: Option<CheckSeverity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waived: Option<String>,
    pub chain: Vec<(ElementId, ResolutionAction)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedStandards {
    pub defaults: FramingDefaults,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub studs: Vec<(ElementId, StudTable)>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<(ElementId, HeaderSpanTable)>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fastening: Vec<(ElementId, FasteningSchedule)>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bracing: Vec<(ElementId, BracingTable)>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checks: Vec<(ElementId, ComplianceCheck)>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<ResolvedRule>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<(String, String)>,
}

pub fn resolve_standards(stack: &[&StandardsPack]) -> ResolvedStandards {
    let mut defaults = stack
        .first()
        .map(|pack| pack.tables.defaults.clone())
        .unwrap_or_else(FramingDefaults::irc_2021_starter);
    let mut rules = BTreeMap::<String, ResolvedEntry>::new();
    let mut warnings = Vec::new();

    for pack in stack {
        defaults = pack.tables.defaults.clone();
        for payload in pack.payloads() {
            let rule = payload.rule().to_owned();
            let chain = match rules.remove(&rule) {
                Some(mut prior) => {
                    prior
                        .chain
                        .push((pack.id.clone(), ResolutionAction::Shadowed));
                    prior.chain
                }
                None => vec![(pack.id.clone(), ResolutionAction::Introduced)],
            };
            rules.insert(
                rule,
                ResolvedEntry {
                    pack: pack.id.clone(),
                    payload,
                    waived: None,
                    chain,
                },
            );
        }
        for overlay in &pack.overlays {
            match overlay {
                RuleOverlay::Waive { target, reason } => match rules.get_mut(target) {
                    Some(entry) => {
                        entry.waived = Some(reason.clone());
                        entry
                            .chain
                            .push((pack.id.clone(), ResolutionAction::Waived));
                    }
                    None => warnings.push((
                        "standards.overlay.unmatched".to_owned(),
                        format!(
                            "standards pack {:?} waives unmatched rule {target:?}",
                            pack.id
                        ),
                    )),
                },
                RuleOverlay::Severity { target, severity } => match rules.get_mut(target) {
                    Some(entry) => {
                        if let RulePayload::Check(check) = &mut entry.payload {
                            check.severity = *severity;
                        }
                        entry
                            .chain
                            .push((pack.id.clone(), ResolutionAction::Reseverity));
                    }
                    None => warnings.push((
                        "standards.overlay.unmatched".to_owned(),
                        format!(
                            "standards pack {:?} re-severities unmatched rule {target:?}",
                            pack.id
                        ),
                    )),
                },
            }
        }
    }

    let mut studs = Vec::new();
    let mut headers = Vec::new();
    let mut fastening = Vec::new();
    let mut bracing = Vec::new();
    let mut checks = Vec::new();
    let mut resolved_rules = Vec::new();
    for (rule, entry) in rules {
        resolved_rules.push(entry.resolved_rule(&rule));
        if entry.waived.is_some() {
            continue;
        }
        match entry.payload {
            RulePayload::Stud(table) => studs.push((entry.pack, table)),
            RulePayload::Header(table) => headers.push((entry.pack, table)),
            RulePayload::Fastening(schedule) => fastening.push((entry.pack, schedule)),
            RulePayload::Bracing(table) => bracing.push((entry.pack, table)),
            RulePayload::Check(check) => checks.push((entry.pack, check)),
        }
    }

    ResolvedStandards {
        defaults,
        studs,
        headers,
        fastening,
        bracing,
        checks,
        rules: resolved_rules,
        warnings,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RulePayload {
    Stud(StudTable),
    Header(HeaderSpanTable),
    Fastening(FasteningSchedule),
    Bracing(BracingTable),
    Check(ComplianceCheck),
}

impl RulePayload {
    fn rule(&self) -> &str {
        match self {
            Self::Stud(table) => &table.rule,
            Self::Header(table) => &table.rule,
            Self::Fastening(schedule) => &schedule.rule,
            Self::Bracing(table) => &table.rule,
            Self::Check(check) => &check.rule,
        }
    }

    fn citation(&self) -> &str {
        match self {
            Self::Stud(table) => &table.citation,
            Self::Header(table) => &table.citation,
            Self::Fastening(schedule) => &schedule.citation,
            Self::Bracing(table) => &table.citation,
            Self::Check(check) => &check.citation,
        }
    }

    fn severity(&self) -> Option<CheckSeverity> {
        match self {
            Self::Check(check) => Some(check.severity),
            Self::Stud(_) | Self::Header(_) | Self::Fastening(_) | Self::Bracing(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedEntry {
    pack: ElementId,
    payload: RulePayload,
    waived: Option<String>,
    chain: Vec<(ElementId, ResolutionAction)>,
}

impl ResolvedEntry {
    fn resolved_rule(&self, rule: &str) -> ResolvedRule {
        ResolvedRule {
            pack: self.pack.clone(),
            rule: rule.to_owned(),
            citation: self.payload.citation().to_owned(),
            severity: self.payload.severity(),
            waived: self.waived.clone(),
            chain: self.chain.clone(),
        }
    }
}

impl StandardsPack {
    fn payloads(&self) -> Vec<RulePayload> {
        self.tables
            .studs
            .iter()
            .cloned()
            .map(RulePayload::Stud)
            .chain(self.tables.headers.iter().cloned().map(RulePayload::Header))
            .chain(
                self.tables
                    .fastening
                    .iter()
                    .cloned()
                    .map(RulePayload::Fastening),
            )
            .chain(
                self.tables
                    .bracing
                    .iter()
                    .cloned()
                    .map(RulePayload::Bracing),
            )
            .chain(self.checks.iter().cloned().map(RulePayload::Check))
            .collect()
    }
}

fn validate_rule_decl(rules: &mut BTreeSet<String>, rule: &str) -> Result<(), ModelError> {
    if !is_valid_rule_id(rule) {
        return Err(ModelError::StandardsInvalidRuleId {
            rule: rule.to_owned(),
        });
    }
    if !rules.insert(rule.to_owned()) {
        return Err(ModelError::StandardsDuplicateRuleId {
            rule: rule.to_owned(),
        });
    }
    Ok(())
}

fn is_valid_rule_id(rule: &str) -> bool {
    let mut chars = rule.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    matches!(first, 'a'..='z' | '0'..='9')
        && chars.all(|ch| matches!(ch, 'a'..='z' | '0'..='9' | '-' | '.'))
}

fn validate_strict_rows<T, K, F>(rule: &str, rows: &[T], key: F) -> Result<(), ModelError>
where
    K: Ord,
    F: Fn(&T) -> K,
{
    let mut previous = None;
    for row in rows {
        let current = key(row);
        if previous.as_ref().is_some_and(|prev| prev >= &current) {
            return Err(ModelError::StandardsTableRowsNotStrictlyOrdered {
                rule: rule.to_owned(),
            });
        }
        previous = Some(current);
    }
    Ok(())
}

fn validate_predicate(
    rule: &str,
    predicate: &Predicate,
    scope: FactScope,
) -> Result<(), ModelError> {
    match predicate {
        Predicate::All(children) | Predicate::Any(children) => {
            for child in children {
                validate_predicate(rule, child, scope)?;
            }
            Ok(())
        }
        Predicate::Not(child) => validate_predicate(rule, child, scope),
        Predicate::Compare { fact, op, value } => {
            validate_fact_scope(rule, *fact, scope)?;
            let expected = fact.value_type();
            let found = value.value_type();
            if expected != found {
                return Err(ModelError::StandardsPredicateTypeMismatch {
                    rule: rule.to_owned(),
                    fact: format!("{fact:?}"),
                    expected: format!("{expected:?}"),
                    found: format!("{found:?}"),
                });
            }
            if expected == FactType::Flag && !matches!(op, CompareOp::Eq | CompareOp::Ne) {
                return Err(ModelError::StandardsPredicateInvalidOperator {
                    rule: rule.to_owned(),
                    fact: format!("{fact:?}"),
                    op: op.label().to_owned(),
                });
            }
            if let Some(operand_fact) = value.fact() {
                validate_fact_scope(rule, operand_fact, scope)?;
            }
            Ok(())
        }
    }
}

fn validate_fact_scope(rule: &str, fact: Fact, scope: FactScope) -> Result<(), ModelError> {
    let fact_scope = fact.scope();
    if fact_scope == scope {
        Ok(())
    } else {
        Err(ModelError::StandardsPredicateScopeMismatch {
            rule: rule.to_owned(),
            fact: format!("{fact:?}"),
            expected_scope: scope.label().to_owned(),
            found_scope: fact_scope.label().to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn starter_pack() -> StandardsPack {
        StandardsPack::irc_2021_starter()
    }

    fn test_check(rule: &str, requirement: Predicate) -> ComplianceCheck {
        ComplianceCheck {
            rule: rule.to_owned(),
            citation: "Test citation".to_owned(),
            title: "Test check".to_owned(),
            severity: CheckSeverity::Required,
            applies: Applicability::Always,
            scope: CheckScope::Walls {
                exterior_only: None,
                tags: Vec::new(),
            },
            requirement,
        }
    }

    fn wall_height_check(rule: &str) -> ComplianceCheck {
        test_check(
            rule,
            Predicate::Compare {
                fact: Fact::WallHeight,
                op: CompareOp::Le,
                value: FactOperand::Fact(Fact::WallStudMaxHeight),
            },
        )
    }

    fn table_pack(id: &str, rule: &str, header_depth: Length) -> StandardsPack {
        let mut pack = starter_pack();
        pack.id = ElementId::new(id);
        pack.tables.defaults.default_header_depth = header_depth;
        pack.tables.studs = vec![StudTable {
            rule: rule.to_owned(),
            citation: format!("{rule} citation"),
            rows: vec![StudRow {
                profile: BoardProfile::TwoByFour,
                spacing: Length::from_whole_inches(16),
                max_height_bearing: Length::from_feet(9.0),
                max_height_nonbearing: Length::from_feet(10.0),
            }],
        }];
        pack.tables.headers.clear();
        pack.tables.fastening.clear();
        pack
    }

    #[test]
    fn fully_populated_pack_round_trips_through_json() {
        let mut pack = starter_pack();
        pack.source = Some(Provenance {
            library_uid: "lib".to_owned(),
            version_id: "version".to_owned(),
            source_id: ElementId::new("source-pack"),
            content_hash: "blake3:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                .to_owned(),
        });
        pack.tables.bracing.push(BracingTable {
            rule: "irc2021.r602.10-3.bracing".to_owned(),
            citation: "IRC 2021 R602.10.3".to_owned(),
            rows: vec![BracingRow {
                method: BracingMethod::Wsp,
                max_seismic: Some(SeismicDesignCategory::C),
                max_wind_speed_mph: Some(115),
                line_length: Length::from_feet(20.0),
                required_length: Length::from_feet(8.0),
            }],
        });
        pack.checks
            .push(wall_height_check("irc2021.r602.3-5.stud-height"));
        pack.overlays.push(RuleOverlay::Severity {
            target: "irc2021.r602.3-5.stud-height".to_owned(),
            severity: CheckSeverity::Advisory,
        });
        pack.tags.push("starter".to_owned());
        pack.properties
            .insert("jurisdictional".to_owned(), PropertyValue::Flag(false));

        let json = serde_json::to_string_pretty(&pack).unwrap();
        let restored: StandardsPack = serde_json::from_str(&json).unwrap();

        assert_eq!(restored, pack);
    }

    #[test]
    fn validation_rejects_invalid_rule_id() {
        let mut pack = starter_pack();
        pack.tables.studs[0].rule = "IRC 2021".to_owned();

        assert!(matches!(
            pack.validate(),
            Err(ModelError::StandardsInvalidRuleId { .. })
        ));
    }

    #[test]
    fn validation_rejects_duplicate_rule_ids_across_tables_and_checks() {
        let mut pack = starter_pack();
        pack.checks
            .push(wall_height_check(&pack.tables.studs[0].rule));

        assert!(matches!(
            pack.validate(),
            Err(ModelError::StandardsDuplicateRuleId { .. })
        ));
    }

    #[test]
    fn validation_rejects_waive_without_reason() {
        let mut pack = starter_pack();
        pack.overlays.push(RuleOverlay::Waive {
            target: "irc2021.r602.3-5.studs".to_owned(),
            reason: " ".to_owned(),
        });

        assert!(matches!(
            pack.validate(),
            Err(ModelError::StandardsOverlayMissingReason { .. })
        ));
    }

    #[test]
    fn validation_rejects_predicate_type_mismatch() {
        let mut pack = starter_pack();
        pack.checks.push(test_check(
            "test.bad-type",
            Predicate::Compare {
                fact: Fact::WallHeight,
                op: CompareOp::Le,
                value: FactOperand::IntLiteral(1),
            },
        ));

        assert!(matches!(
            pack.validate(),
            Err(ModelError::StandardsPredicateTypeMismatch { .. })
        ));
    }

    #[test]
    fn validation_rejects_flag_predicate_with_ordering_operator() {
        let mut pack = starter_pack();
        pack.checks.push(test_check(
            "test.bad-flag-op",
            Predicate::Compare {
                fact: Fact::WallIsExterior,
                op: CompareOp::Gt,
                value: FactOperand::FlagLiteral(false),
            },
        ));

        assert!(matches!(
            pack.validate(),
            Err(ModelError::StandardsPredicateInvalidOperator { .. })
        ));
    }

    #[test]
    fn validation_rejects_fact_scope_mismatch() {
        let mut pack = starter_pack();
        pack.checks.push(test_check(
            "test.bad-scope",
            Predicate::Compare {
                fact: Fact::RoomAreaSquareInches,
                op: CompareOp::Gt,
                value: FactOperand::IntLiteral(1),
            },
        ));

        assert!(matches!(
            pack.validate(),
            Err(ModelError::StandardsPredicateScopeMismatch { .. })
        ));
    }

    #[test]
    fn validation_rejects_unordered_rows() {
        let mut pack = starter_pack();
        pack.tables.studs[0].rows.swap(0, 1);

        assert!(matches!(
            pack.validate(),
            Err(ModelError::StandardsTableRowsNotStrictlyOrdered { .. })
        ));
    }

    #[test]
    fn validation_rejects_duplicate_fastening_keys() {
        let mut pack = starter_pack();
        let duplicate = pack.tables.fastening[0].rows[0].clone();
        pack.tables.fastening[0].rows.insert(1, duplicate);

        assert!(matches!(
            pack.validate(),
            Err(ModelError::StandardsTableRowsNotStrictlyOrdered { .. })
        ));
    }

    #[test]
    fn starter_pack_validates_and_has_stable_canonical_json() {
        let pack = starter_pack();

        pack.validate().unwrap();
        assert_eq!(
            serde_json::to_value(&pack).unwrap(),
            serde_json::json!({
                "id": "std-irc-2021",
                "name": "IRC 2021 Prescriptive (starter)",
                "edition": "2021",
                "tables": {
                    "defaults": {
                        "default_wall_height": {"ticks": 1536},
                        "default_stud_spacing": {"ticks": 256},
                        "double_top_plate": true,
                        "default_header_depth": {"ticks": 144},
                        "stud_profile": "TwoByFour",
                        "plate_profile": "TwoByFour",
                        "header_profile": "TwoByTen"
                    },
                    "studs": [{
                        "rule": "irc2021.r602.3-5.studs",
                        "citation": "IRC 2021 Table R602.3(5)",
                        "rows": [
                            {"profile": "TwoByFour", "spacing": {"ticks": 256}, "max_height_bearing": {"ticks": 1920}, "max_height_nonbearing": {"ticks": 2688}},
                            {"profile": "TwoByFour", "spacing": {"ticks": 384}, "max_height_bearing": {"ticks": 1920}, "max_height_nonbearing": {"ticks": 2304}},
                            {"profile": "TwoBySix", "spacing": {"ticks": 256}, "max_height_bearing": {"ticks": 2688}, "max_height_nonbearing": {"ticks": 3840}},
                            {"profile": "TwoBySix", "spacing": {"ticks": 384}, "max_height_bearing": {"ticks": 2304}, "max_height_nonbearing": {"ticks": 3456}}
                        ]
                    }],
                    "headers": [{
                        "rule": "irc2021.r602.7-1.headers",
                        "citation": "IRC 2021 Table R602.7(1)",
                        "rows": [
                            {"profile": "TwoByTen", "plies": 1, "max_ground_snow_psf": 30, "max_building_width": {"ticks": 6912}, "max_span": {"ticks": 768}, "jack_studs": 1},
                            {"profile": "TwoByTen", "plies": 2, "max_ground_snow_psf": 30, "max_building_width": {"ticks": 6912}, "max_span": {"ticks": 1152}, "jack_studs": 1},
                            {"profile": "TwoByTwelve", "plies": 1, "max_ground_snow_psf": 30, "max_building_width": {"ticks": 6912}, "max_span": {"ticks": 960}, "jack_studs": 1},
                            {"profile": "TwoByTwelve", "plies": 2, "max_ground_snow_psf": 30, "max_building_width": {"ticks": 6912}, "max_span": {"ticks": 1536}, "jack_studs": 2}
                        ]
                    }],
                    "fastening": [{
                        "rule": "irc2021.r602.3-1.fastening",
                        "citation": "IRC 2021 Table R602.3(1)",
                        "rows": [
                            {"connection": "StudToPlateEnd", "fastener": "16d common nail", "schedule": {"Count": 2}},
                            {"connection": "StudToPlateToe", "fastener": "8d common nail", "schedule": {"Count": 4}},
                            {"connection": "TopPlateLap", "fastener": "16d common nail", "schedule": {"Count": 8}},
                            {"connection": "DoubleTopPlate", "fastener": "16d common nail", "schedule": {"Spacing": {"on_center": {"ticks": 256}}}}
                        ]
                    }]
                }
            })
        );
    }

    #[test]
    fn resolution_shadows_waives_reseverities_and_warns_deterministically() {
        let mut base = table_pack(
            "base-pack",
            "irc2021.r602.3-5.studs",
            Length::from_whole_inches(9),
        );
        base.checks.push(wall_height_check("check.wall-height"));
        let mut overlay = table_pack(
            "overlay-pack",
            "irc2021.r602.3-5.studs",
            Length::from_whole_inches(11),
        );
        overlay.overlays.push(RuleOverlay::Severity {
            target: "check.wall-height".to_owned(),
            severity: CheckSeverity::Advisory,
        });
        overlay.overlays.push(RuleOverlay::Waive {
            target: "irc2021.r602.3-5.studs".to_owned(),
            reason: "local engineered design".to_owned(),
        });
        overlay.overlays.push(RuleOverlay::Waive {
            target: "missing.rule".to_owned(),
            reason: "legacy overlay".to_owned(),
        });

        let resolved = resolve_standards(&[&base, &overlay]);
        let second = resolve_standards(&[&base, &overlay]);

        assert_eq!(resolved, second);
        assert_eq!(
            resolved.defaults.default_header_depth,
            Length::from_whole_inches(11)
        );
        assert!(resolved.studs.is_empty(), "waived table excluded");
        assert_eq!(resolved.checks.len(), 1);
        assert_eq!(resolved.checks[0].1.severity, CheckSeverity::Advisory);
        assert_eq!(resolved.warnings.len(), 1);
        assert_eq!(resolved.warnings[0].0, "standards.overlay.unmatched");

        let stud_rule = resolved
            .rules
            .iter()
            .find(|rule| rule.rule == "irc2021.r602.3-5.studs")
            .unwrap();
        assert_eq!(stud_rule.pack, ElementId::new("overlay-pack"));
        assert_eq!(stud_rule.waived.as_deref(), Some("local engineered design"));
        assert_eq!(
            stud_rule.chain,
            vec![
                (ElementId::new("base-pack"), ResolutionAction::Introduced),
                (ElementId::new("overlay-pack"), ResolutionAction::Shadowed),
                (ElementId::new("overlay-pack"), ResolutionAction::Waived),
            ]
        );

        let check_rule = resolved
            .rules
            .iter()
            .find(|rule| rule.rule == "check.wall-height")
            .unwrap();
        assert_eq!(check_rule.severity, Some(CheckSeverity::Advisory));
        assert_eq!(
            check_rule.chain,
            vec![
                (ElementId::new("base-pack"), ResolutionAction::Introduced),
                (ElementId::new("overlay-pack"), ResolutionAction::Reseverity),
            ]
        );
    }
}
