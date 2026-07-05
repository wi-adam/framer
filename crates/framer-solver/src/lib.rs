use std::collections::BTreeMap;
use std::fmt::Write;

use framer_core::{
    BoardProfile, BuildingModel, Ceiling, CeilingSlope, ConnectionKind, ConstructionSystem,
    ElementId, FastenerSchedule, FloorDeck, FramingDefaults, FramingSpec, HeaderRow,
    HeaderSpanTable, LayerFunction, Length, Material, ModelError, Opening, Point2,
    ResolvedStandards, RoofPlane, RoofPlaneFrame, Room, SiteContext, Slope, SpanDirection,
    SurfaceRegion, Wall, WallExposure, WallJoin, WallJoinKind, concave_polygon_corners,
    level_wall_loop_outline, point_in_polygon, polygon_area_square_inches,
    room_boundaries_for_rooms,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WallFramePlan {
    pub wall: ElementId,
    pub members: Vec<FrameMember>,
    #[serde(default)]
    pub layers: Vec<LayerBomItem>,
    pub diagnostics: Vec<PlanDiagnostic>,
}

impl WallFramePlan {
    pub fn bom(&self) -> Vec<BomItem> {
        bom_from_members(self.members.iter())
    }

    /// The per-layer material takeoff for this wall, aggregated by
    /// (material, function, thickness).
    pub fn layer_bom(&self) -> Vec<LayerBomItem> {
        layer_bom_from(self.layers.iter())
    }
}

/// The derived framing of one floor deck: its joists, rim/band members, and
/// blocking, plus the per-layer material takeoff and any diagnostics. The shape
/// mirrors [`WallFramePlan`] (a flat ceiling and a floor deck share one joisting
/// generator), differing only in which authored element it is keyed to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FloorFramePlan {
    pub floor: ElementId,
    pub members: Vec<FrameMember>,
    #[serde(default)]
    pub layers: Vec<LayerBomItem>,
    pub diagnostics: Vec<PlanDiagnostic>,
}

impl FloorFramePlan {
    pub fn bom(&self) -> Vec<BomItem> {
        bom_from_members(self.members.iter())
    }

    pub fn layer_bom(&self) -> Vec<LayerBomItem> {
        layer_bom_from(self.layers.iter())
    }
}

/// The derived framing of one flat ceiling: structurally a floor deck viewed
/// from below, so it shares the joisting generator and the [`FloorFramePlan`]
/// shape. Keyed to the authored [`Ceiling`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CeilingFramePlan {
    pub ceiling: ElementId,
    pub members: Vec<FrameMember>,
    #[serde(default)]
    pub layers: Vec<LayerBomItem>,
    pub diagnostics: Vec<PlanDiagnostic>,
}

impl CeilingFramePlan {
    pub fn bom(&self) -> Vec<BomItem> {
        bom_from_members(self.members.iter())
    }

    pub fn layer_bom(&self) -> Vec<LayerBomItem> {
        layer_bom_from(self.layers.iter())
    }
}

/// The derived framing of one roof plane: its common rafters, plate blocking,
/// and (for a gable plane that carries the shared ridge) a ridge board, plus the
/// per-layer material takeoff and any diagnostics. Mirrors [`WallFramePlan`],
/// keyed to the authored [`RoofPlane`]. Rafters carry a [`SlopedPlacement`]; the
/// plan stays float-free (true lengths are rounded to ticks).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoofFramePlan {
    pub roof: ElementId,
    pub members: Vec<FrameMember>,
    #[serde(default)]
    pub layers: Vec<LayerBomItem>,
    pub diagnostics: Vec<PlanDiagnostic>,
}

impl RoofFramePlan {
    pub fn bom(&self) -> Vec<BomItem> {
        bom_from_members(self.members.iter())
    }

    pub fn layer_bom(&self) -> Vec<LayerBomItem> {
        layer_bom_from(self.layers.iter())
    }
}

/// A per-layer material takeoff row: how much of one material a layer requires.
/// Area goods (finishes, sheathing, cladding, weather barriers, masonry) report
/// `area_sq_in` (square inches); volumetric goods (continuous insulation and the
/// framing layer's cavity material) report `volume_bd_in` (cubic inches = area ×
/// layer thickness). The unused measure is zero. `LayerFunction` — not the
/// material — decides which measure applies, keeping the takeoff logic on the
/// closed enum. Quantities are whole units so the plan stays `Eq`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayerBomItem {
    pub material: ElementId,
    pub material_name: String,
    pub function: LayerFunction,
    pub thickness: Length,
    pub area_sq_in: i64,
    pub volume_bd_in: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectFramePlan {
    pub wall_plans: Vec<WallFramePlan>,
    #[serde(default)]
    pub floor_plans: Vec<FloorFramePlan>,
    #[serde(default)]
    pub ceiling_plans: Vec<CeilingFramePlan>,
    #[serde(default)]
    pub roof_plans: Vec<RoofFramePlan>,
    pub diagnostics: Vec<PlanDiagnostic>,
    #[serde(default)]
    pub rooms: Vec<RoomSchedule>,
    #[serde(default)]
    pub layers: Vec<LayerBomItem>,
    #[serde(default)]
    pub fasteners: Vec<FastenerTakeoff>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FastenerTakeoff {
    pub fastener: String,
    pub connection: ConnectionKind,
    pub quantity: u32,
    pub rule: String,
    pub citation: String,
}

/// A derived room takeoff row: identity plus the area/perimeter computed from the
/// room's bounding wall loop. `closed` is false when the room is not enclosed
/// (in which case area/perimeter are zero and a diagnostic is emitted). Area is
/// stored in whole square inches to keep the plan `Eq`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomSchedule {
    pub room: ElementId,
    pub name: String,
    pub usage: String,
    pub closed: bool,
    pub area_square_inches: i64,
    pub perimeter: Length,
}

impl RoomSchedule {
    pub fn area_square_feet(&self) -> f64 {
        self.area_square_inches as f64 / 144.0
    }
}

impl ProjectFramePlan {
    /// Every framing member across every host: walls, floor decks, ceilings, and
    /// roof planes.
    fn all_members(&self) -> impl Iterator<Item = &FrameMember> {
        self.wall_plans
            .iter()
            .flat_map(|plan| plan.members.iter())
            .chain(self.floor_plans.iter().flat_map(|plan| plan.members.iter()))
            .chain(
                self.ceiling_plans
                    .iter()
                    .flat_map(|plan| plan.members.iter()),
            )
            .chain(self.roof_plans.iter().flat_map(|plan| plan.members.iter()))
    }

    pub fn bom(&self) -> Vec<BomItem> {
        bom_from_members(self.all_members())
    }

    /// The project-wide per-layer material takeoff, aggregated across every wall,
    /// floor deck, and ceiling by (material, function, thickness).
    pub fn layer_bom(&self) -> Vec<LayerBomItem> {
        layer_bom_from(self.layers.iter())
    }

    pub fn wall_plan(&self, wall: &ElementId) -> Option<&WallFramePlan> {
        self.wall_plans
            .iter()
            .find(|wall_plan| wall_plan.wall == *wall)
    }

    pub fn wall_plan_mut(&mut self, wall: &ElementId) -> Option<&mut WallFramePlan> {
        self.wall_plans
            .iter_mut()
            .find(|wall_plan| wall_plan.wall == *wall)
    }

    pub fn floor_plan(&self, floor: &ElementId) -> Option<&FloorFramePlan> {
        self.floor_plans
            .iter()
            .find(|floor_plan| floor_plan.floor == *floor)
    }

    pub fn ceiling_plan(&self, ceiling: &ElementId) -> Option<&CeilingFramePlan> {
        self.ceiling_plans
            .iter()
            .find(|ceiling_plan| ceiling_plan.ceiling == *ceiling)
    }

    pub fn roof_plan(&self, roof: &ElementId) -> Option<&RoofFramePlan> {
        self.roof_plans
            .iter()
            .find(|roof_plan| roof_plan.roof == *roof)
    }
}

fn bom_from_members<'a>(members: impl IntoIterator<Item = &'a FrameMember>) -> Vec<BomItem> {
    let mut grouped: BTreeMap<(BoardProfile, MemberKind, Length), u32> = BTreeMap::new();
    for member in members {
        *grouped
            .entry((member.profile, member.kind, member.cut_length))
            .or_default() += 1;
    }

    grouped
        .into_iter()
        .map(|((profile, kind, cut_length), quantity)| BomItem {
            profile,
            kind,
            cut_length,
            quantity,
            total_length: cut_length * quantity as i64,
        })
        .collect()
}

/// Aggregate per-layer takeoff rows by (material, function, thickness), summing
/// area and volume. The key keeps material name out of grouping (it is identity,
/// not a discriminator) so rows are deterministic and `BTreeMap`-ordered.
fn layer_bom_from<'a>(layers: impl IntoIterator<Item = &'a LayerBomItem>) -> Vec<LayerBomItem> {
    let mut grouped: BTreeMap<(ElementId, LayerFunction, Length), (String, i64, i64)> =
        BTreeMap::new();
    for layer in layers {
        let entry = grouped
            .entry((layer.material.clone(), layer.function, layer.thickness))
            .or_insert_with(|| (layer.material_name.clone(), 0, 0));
        entry.1 += layer.area_sq_in;
        entry.2 += layer.volume_bd_in;
    }

    grouped
        .into_iter()
        .map(
            |((material, function, thickness), (material_name, area_sq_in, volume_bd_in))| {
                LayerBomItem {
                    material,
                    material_name,
                    function,
                    thickness,
                    area_sq_in,
                    volume_bd_in,
                }
            },
        )
        .collect()
}

#[derive(Debug, Clone, Default)]
struct ConnectionCounts {
    stud_to_plate: u32,
    top_plate_lap: u32,
    header_to_king_stud: u32,
}

struct FastenerAccumulator {
    quantity: u32,
    rule: String,
    citation: String,
}

fn fastener_takeoff(
    model: &BuildingModel,
    plan: &ProjectFramePlan,
    standards: &ResolvedStandards,
    diagnostics: &mut Vec<PlanDiagnostic>,
) -> Vec<FastenerTakeoff> {
    let counts = connection_counts(model, plan);
    let mut grouped = BTreeMap::<(String, ConnectionKind), FastenerAccumulator>::new();
    let mut saw_sheathing_row = false;

    for (_, schedule) in &standards.fastening {
        for row in &schedule.rows {
            if matches!(
                row.connection,
                ConnectionKind::SheathingEdge | ConnectionKind::SheathingField
            ) {
                saw_sheathing_row = true;
                continue;
            }

            let quantity = fastener_quantity(
                row.connection,
                &row.schedule,
                model,
                &standards.defaults,
                &counts,
            );
            if quantity == 0 {
                continue;
            }

            let entry = grouped
                .entry((row.fastener.clone(), row.connection))
                .or_insert_with(|| FastenerAccumulator {
                    quantity: 0,
                    rule: schedule.rule.clone(),
                    citation: schedule.citation.clone(),
                });
            entry.quantity = entry.quantity.saturating_add(quantity);
        }
    }

    if saw_sheathing_row {
        diagnostics.push(PlanDiagnostic::new(
            DiagnosticSeverity::Info,
            "standards.fastening.sheathing-not-counted",
            None,
            "Fastening schedule rows for sheathing edges/fields were skipped because panel layout is not generated yet.",
        ));
    }

    grouped
        .into_iter()
        .map(
            |(
                (fastener, connection),
                FastenerAccumulator {
                    quantity,
                    rule,
                    citation,
                },
            )| FastenerTakeoff {
                fastener,
                connection,
                quantity,
                rule,
                citation,
            },
        )
        .collect()
}

fn connection_counts(model: &BuildingModel, plan: &ProjectFramePlan) -> ConnectionCounts {
    let mut counts = ConnectionCounts::default();

    for wall_plan in &plan.wall_plans {
        counts.stud_to_plate = counts.stud_to_plate.saturating_add(
            usize_to_u32(
                wall_plan
                    .members
                    .iter()
                    .filter(|member| is_wall_stud(member))
                    .count(),
            )
            .saturating_mul(2),
        );

        // Top plates are currently emitted as full-length pieces. This stays at
        // zero until stock-length plate splicing emits multiple pieces at the
        // same elevation.
        let mut top_plate_lines = BTreeMap::<Length, u32>::new();
        for member in wall_plan
            .members
            .iter()
            .filter(|member| member.kind == MemberKind::TopPlate)
        {
            *top_plate_lines.entry(member.elevation).or_default() += 1;
        }
        counts.top_plate_lap = counts.top_plate_lap.saturating_add(
            top_plate_lines
                .values()
                .map(|pieces| pieces.saturating_sub(1))
                .sum(),
        );
    }

    for wall in &model.walls {
        let Some(wall_plan) = plan.wall_plan(&wall.id) else {
            continue;
        };
        for opening in &wall.openings {
            let has_header = wall_plan
                .members
                .iter()
                .any(|member| member.kind == MemberKind::Header && member.source == opening.id);
            if has_header {
                counts.header_to_king_stud = counts.header_to_king_stud.saturating_add(2);
            }
        }
    }

    counts
}

fn is_wall_stud(member: &FrameMember) -> bool {
    member.orientation == MemberOrientation::Vertical
        && matches!(
            member.kind,
            MemberKind::CornerPost
                | MemberKind::PartitionStud
                | MemberKind::BackingStud
                | MemberKind::CommonStud
                | MemberKind::KingStud
                | MemberKind::JackStud
                | MemberKind::CrippleStud
        )
}

fn fastener_quantity(
    connection: ConnectionKind,
    schedule: &FastenerSchedule,
    model: &BuildingModel,
    defaults: &FramingDefaults,
    counts: &ConnectionCounts,
) -> u32 {
    match schedule {
        FastenerSchedule::Count(count) => {
            connection_count(connection, counts).saturating_mul(*count)
        }
        FastenerSchedule::Spacing { on_center } => {
            spacing_fastener_count(connection, *on_center, model, defaults)
        }
        FastenerSchedule::EdgeField { .. } => 0,
    }
}

fn connection_count(connection: ConnectionKind, counts: &ConnectionCounts) -> u32 {
    match connection {
        ConnectionKind::StudToPlateEnd | ConnectionKind::StudToPlateToe => counts.stud_to_plate,
        ConnectionKind::TopPlateLap => counts.top_plate_lap,
        ConnectionKind::HeaderToKingStud => counts.header_to_king_stud,
        ConnectionKind::DoubleTopPlate
        | ConnectionKind::SolePlateToJoist
        | ConnectionKind::SheathingEdge
        | ConnectionKind::SheathingField => 0,
    }
}

fn spacing_fastener_count(
    connection: ConnectionKind,
    on_center: Length,
    model: &BuildingModel,
    defaults: &FramingDefaults,
) -> u32 {
    match connection {
        ConnectionKind::DoubleTopPlate if defaults.double_top_plate => {
            model.walls.iter().fold(0u32, |total, wall| {
                total.saturating_add(ceil_length_div(wall.length, on_center))
            })
        }
        _ => 0,
    }
}

fn ceil_length_div(length: Length, divisor: Length) -> u32 {
    if length <= Length::ZERO || divisor <= Length::ZERO {
        return 0;
    }
    let length = length.ticks();
    let divisor = divisor.ticks();
    usize_to_u32(((length + divisor - 1) / divisor) as usize)
}

fn usize_to_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// Build the per-layer material takeoff for a single wall. Net face area is the
/// wall face minus its openings (clamped non-negative).
fn wall_layer_bom(
    wall: &Wall,
    system: &ConstructionSystem,
    materials: &[Material],
) -> Vec<LayerBomItem> {
    layers_takeoff(system, net_face_area_sq_in(wall), materials)
}

/// The per-layer material takeoff for one surface of `area_sq_in` square inches
/// framed by `system`. Area goods (finishes, sheathing/decking, cladding,
/// weather barriers, masonry, roofing, ceiling finish) report area; continuous
/// insulation and the framing layer's cavity material report area × thickness;
/// air gaps, the framing layer's lumber, structure, and other roles are skipped
/// (lumber is covered by the member BOM). The cavity material uses the framing
/// layer's depth as its thickness. Shared by walls, floor decks, and ceilings —
/// the layer-role classification is identical across [`SystemKind`]s.
fn layers_takeoff(
    system: &ConstructionSystem,
    area_sq_in: i64,
    materials: &[Material],
) -> Vec<LayerBomItem> {
    let material_name = |id: &ElementId| {
        materials
            .iter()
            .find(|material| material.id == *id)
            .map(|material| material.name.clone())
            .unwrap_or_else(|| id.0.clone())
    };

    let mut items = Vec::new();
    for layer in &system.layers {
        match layer.function {
            LayerFunction::InteriorFinish
            | LayerFunction::Sheathing
            | LayerFunction::Cladding
            | LayerFunction::WeatherBarrier
            | LayerFunction::Masonry
            | LayerFunction::Roofing
            | LayerFunction::Underlayment
            | LayerFunction::CeilingFinish => {
                items.push(LayerBomItem {
                    material: layer.material.clone(),
                    material_name: material_name(&layer.material),
                    function: layer.function,
                    thickness: layer.thickness,
                    area_sq_in,
                    volume_bd_in: 0,
                });
            }
            LayerFunction::ContinuousInsulation => {
                items.push(LayerBomItem {
                    material: layer.material.clone(),
                    material_name: material_name(&layer.material),
                    function: layer.function,
                    thickness: layer.thickness,
                    area_sq_in: 0,
                    volume_bd_in: volume_bd_in(area_sq_in, layer.thickness),
                });
            }
            LayerFunction::Framing => {
                // The framing lumber itself is counted in the member BOM; here we
                // only take off the between-studs cavity material (if any), filling
                // the framing band's depth.
                if let Some(framing) = &layer.framing
                    && let Some(cavity) = &framing.cavity_material
                {
                    items.push(LayerBomItem {
                        material: cavity.clone(),
                        material_name: material_name(cavity),
                        function: LayerFunction::ContinuousInsulation,
                        thickness: layer.thickness,
                        area_sq_in: 0,
                        volume_bd_in: volume_bd_in(area_sq_in, layer.thickness),
                    });
                }
            }
            LayerFunction::AirGap | LayerFunction::Structure | LayerFunction::Other => {}
        }
    }
    items
}

/// The wall's clear face area in whole square inches: `length × height` minus the
/// sum of opening areas, clamped to be non-negative.
fn net_face_area_sq_in(wall: &Wall) -> i64 {
    let gross = (wall.length.inches() * wall.height.inches()).round() as i64;
    let openings: i64 = wall
        .openings
        .iter()
        .map(|opening| (opening.width.inches() * opening.height.inches()).round() as i64)
        .sum();
    (gross - openings).max(0)
}

/// Volume (cubic inches) of an area good of the given thickness.
fn volume_bd_in(area_sq_in: i64, thickness: Length) -> i64 {
    (area_sq_in as f64 * thickness.inches()).round() as i64
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameMember {
    pub id: String,
    pub source: ElementId,
    pub kind: MemberKind,
    pub profile: BoardProfile,
    pub orientation: MemberOrientation,
    pub x: Length,
    pub elevation: Length,
    pub cut_length: Length,
    pub cross_section_depth: Length,
    /// Through-wall offset (interior -> exterior) of the framing band this member
    /// sits in: the summed thickness of the layers inboard of the framing layer.
    /// Lets the renderer place studs inside the framing layer rather than centered
    /// across the full wall thickness.
    pub side_offset: Length,
    /// Through-wall depth of the framing band: the framing layer's thickness
    /// (== `member.nominal_depth()`). The member occupies `[side_offset,
    /// side_offset + side_depth]` across the wall section.
    pub side_depth: Length,
    /// Sloped placement for roof-plane members whose true building elevation
    /// varies along their in-plane run (a rafter rises from eave to ridge).
    /// `None` for plain 2-D members (walls, flat floors/ceilings), which sit at a
    /// single host elevation applied at render. When `Some`, `cut_length` is the
    /// true sloped length of the board; the in-plane plan run is recovered as
    /// `sqrt(cut_length² − (high − low)²)`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sloped: Option<SlopedPlacement>,
    pub provenance: RuleProvenance,
}

/// The vertical placement of a sloped roof-plane member: the true building
/// elevation at each end of its in-plane run. A rafter's `low` is its eave (or
/// overhang-tail) end and `high` is its ridge end; level roof members (a ridge
/// board, eave blocking) carry `low == high`. Integer-tick and `Eq` so the
/// derived plan stays deterministic; true (f64) geometry is computed transiently
/// in the solver and rounded to ticks here, never stored as a float.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlopedPlacement {
    /// True building elevation at the member's low (eave/tail) end.
    pub low_elevation: Length,
    /// True building elevation at the member's high (ridge) end.
    pub high_elevation: Length,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleProvenance {
    pub rule_id: String,
    pub summary: String,
}

impl RuleProvenance {
    fn new(rule_id: impl Into<String>, summary: impl Into<String>) -> Self {
        Self {
            rule_id: rule_id.into(),
            summary: summary.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum MemberKind {
    BottomPlate,
    TopPlate,
    CornerPost,
    PartitionStud,
    BackingStud,
    CommonStud,
    KingStud,
    JackStud,
    Header,
    RoughSill,
    CrippleStud,
    /// A floor-deck joist (the horizontal span member of a floor).
    FloorJoist,
    /// A flat-ceiling joist (a floor joist viewed from below).
    CeilingJoist,
    /// The rim/band member closing the joist ends at a bearing line.
    RimJoist,
    /// A short member set between joists (e.g. a mid-span blocking row).
    Blocking,
    /// A common rafter: the sloped span member of a roof plane, running up the
    /// slope from the eave (downslope) edge to the ridge/high edge.
    Rafter,
    /// The ridge board a gable's opposing rafters bear against, running level
    /// along the ridge (the shared high edge of two roof planes).
    RidgeBoard,
    /// A diagonal hip rafter shared by two adjacent hip-roof planes, running from
    /// a footprint corner up to the ridge end.
    HipRafter,
    /// A diagonal valley rafter shared by two inward roof planes.
    ValleyRafter,
    /// A shortened rafter that dies into a hip or valley. v2 B2 emits these
    /// against hips; valley jacks arrive with the valley phase.
    JackRafter,
}

impl MemberKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::BottomPlate => "bottom plate",
            Self::TopPlate => "top plate",
            Self::CornerPost => "corner post",
            Self::PartitionStud => "partition stud",
            Self::BackingStud => "backing stud",
            Self::CommonStud => "common stud",
            Self::KingStud => "king stud",
            Self::JackStud => "jack stud",
            Self::Header => "header",
            Self::RoughSill => "rough sill",
            Self::CrippleStud => "cripple stud",
            Self::FloorJoist => "floor joist",
            Self::CeilingJoist => "ceiling joist",
            Self::RimJoist => "rim joist",
            Self::Blocking => "blocking",
            Self::Rafter => "rafter",
            Self::RidgeBoard => "ridge board",
            Self::HipRafter => "hip rafter",
            Self::ValleyRafter => "valley rafter",
            Self::JackRafter => "jack rafter",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MemberOrientation {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BomItem {
    pub profile: BoardProfile,
    pub kind: MemberKind,
    pub cut_length: Length,
    pub quantity: u32,
    pub total_length: Length,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanDiagnostic {
    pub severity: DiagnosticSeverity,
    pub code: String,
    pub source: Option<ElementId>,
    pub message: String,
}

impl PlanDiagnostic {
    fn new(
        severity: DiagnosticSeverity,
        code: impl Into<String>,
        source: Option<ElementId>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            severity,
            code: code.into(),
            source,
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiagnosticSeverity {
    Info,
    Warning,
    Unsupported,
}

/// The framing detail of a construction system: the single `Framing` layer's
/// `FramingSpec`. Studs/plates/joists use `member`; `spacing` is the o.c. layout.
/// Generic over [`SystemKind`] — walls, floors, and ceilings all read it.
fn system_framing(system: &ConstructionSystem) -> Result<&FramingSpec, SolverError> {
    system
        .framing_layer()
        .and_then(|layer| layer.framing.as_ref())
        .ok_or_else(|| SolverError::SystemHasNoFramingLayer {
            system: system.id.clone(),
        })
}

/// The through-wall band a system's framing members occupy: `offset` is the
/// summed thickness of every layer inboard (interior) of the framing layer, and
/// `depth` is the framing layer's own thickness. Members are placed inside
/// `[offset, offset + depth]` so studs sit in the framing layer rather than
/// spanning the full wall section.
#[derive(Clone, Copy)]
struct FramingBand {
    offset: Length,
    depth: Length,
}

impl FramingBand {
    fn for_system(system: &ConstructionSystem) -> Result<Self, SolverError> {
        let mut offset = Length::ZERO;
        for layer in &system.layers {
            if layer.function == LayerFunction::Framing {
                return Ok(Self {
                    offset,
                    depth: layer.thickness,
                });
            }
            offset += layer.thickness;
        }
        Err(SolverError::SystemHasNoFramingLayer {
            system: system.id.clone(),
        })
    }
}

/// The per-wall framing facts a member generator needs, bundled so opening and
/// cripple helpers take one `Copy` value rather than several parallel params:
/// the framing `member` (studs/plates), its on-center `spacing`, and the
/// through-wall `band` the members sit in. Built once in `generate_wall_plan`
/// from the system's framing layer.
#[derive(Clone, Copy)]
struct WallFraming {
    member: BoardProfile,
    spacing: Length,
    band: FramingBand,
}

impl WallFraming {
    fn for_system(system: &ConstructionSystem) -> Result<Self, SolverError> {
        let framing = system_framing(system)?;
        Ok(Self {
            member: framing.member,
            spacing: framing.spacing,
            band: FramingBand::for_system(system)?,
        })
    }
}

pub fn generate_wall_plan(
    wall: &Wall,
    system: &ConstructionSystem,
    materials: &[Material],
    standards: &ResolvedStandards,
    standards_name: &str,
) -> Result<WallFramePlan, SolverError> {
    generate_wall_plan_with_site(
        wall,
        system,
        materials,
        standards,
        standards_name,
        &SiteContext::default(),
    )
}

fn generate_wall_plan_with_site(
    wall: &Wall,
    system: &ConstructionSystem,
    materials: &[Material],
    standards: &ResolvedStandards,
    standards_name: &str,
    site: &SiteContext,
) -> Result<WallFramePlan, SolverError> {
    wall.validate()?;
    let defaults = &standards.defaults;

    // Bundle the per-wall framing facts (member, spacing, through-wall band)
    // once; opening and cripple helpers take this rather than parallel params.
    // Members live inside the framing layer's through-wall band so studs render
    // in the framing layer rather than centered across the whole wall section.
    let framing = WallFraming::for_system(system)?;
    let band = framing.band;
    // The framing member is both the stud and the plate; spacing drives the
    // on-center layout. TODO: honor FramingPattern::Double/Staggered (extra
    // studs); every pattern is currently treated as Single.
    let wall_stud = framing.member;
    let wall_plate = framing.member;
    let stud_spacing = framing.spacing;

    let mut members = Vec::new();
    let mut diagnostics = starter_profile_diagnostics(wall, standards_name);
    let use_header_tables = wall_uses_header_tables(wall, system);
    let plate_thickness = wall_plate.thickness();
    let stud_thickness = wall_stud.thickness();
    let top_plate_count = if defaults.double_top_plate { 2 } else { 1 };
    let stud_top = wall.height - plate_thickness * top_plate_count as i64;
    let stud_length = stud_top - plate_thickness;

    if stud_length <= Length::ZERO {
        return Err(SolverError::WallTooShortForPlateStack {
            wall: wall.id.clone(),
        });
    }

    for opening in &wall.openings {
        if opening.left() < stud_thickness * 2 || opening.right() + stud_thickness * 2 > wall.length
        {
            return Err(SolverError::OpeningTooCloseToWallEnd {
                wall: wall.id.clone(),
                opening: opening.id.clone(),
            });
        }

        if opening.top() >= stud_top {
            return Err(SolverError::OpeningLeavesNoHeaderSpace {
                wall: wall.id.clone(),
                opening: opening.id.clone(),
            });
        }
    }

    members.push(frame_member(
        "bottom-plate-1",
        &wall.id,
        MemberKind::BottomPlate,
        wall_plate,
        FrameMemberPlacement::new(
            MemberOrientation::Horizontal,
            Length::ZERO,
            Length::ZERO,
            wall.length,
            plate_thickness,
        ),
        band,
        RuleProvenance::new(
            "wall.plate.continuous",
            "Bottom plate runs the authored wall length using the configured plate profile.",
        ),
    ));

    for index in 0..top_plate_count {
        members.push(frame_member(
            format!("top-plate-{}", index + 1),
            &wall.id,
            MemberKind::TopPlate,
            wall_plate,
            FrameMemberPlacement::new(
                MemberOrientation::Horizontal,
                Length::ZERO,
                wall.height - plate_thickness * (index as i64 + 1),
                wall.length,
                plate_thickness,
            ),
            band,
            RuleProvenance::new(
                "wall.plate.double-top",
                format!(
                    "Top plate {} of {} uses the wall length and configured double-top-plate setting.",
                    index + 1,
                    top_plate_count
                ),
            ),
        ));
    }

    let stud_base = plate_thickness;
    for x in stud_positions(wall.length, stud_spacing, stud_thickness) {
        if !is_inside_opening_framing_assembly(x, &wall.openings, stud_thickness) {
            members.push(frame_member(
                format!("stud-{}", x.ticks()),
                &wall.id,
                MemberKind::CommonStud,
                wall_stud,
                FrameMemberPlacement::new(
                    MemberOrientation::Vertical,
                    x,
                    stud_base,
                    stud_length,
                    stud_thickness,
                ),
                band,
                RuleProvenance::new(
                    "wall.studs.on-center",
                    format!(
                        "End studs align with wall faces, interior common studs are placed at {} layout marks, and authored opening framing assemblies are kept clear.",
                        stud_spacing
                    ),
                ),
            ));
        }
    }

    let mut openings = wall.openings.clone();
    openings.sort_by_key(Opening::left);
    for opening in openings {
        let context = OpeningMemberContext {
            defaults,
            framing,
            standards,
            site,
            use_header_tables,
            top_plate_count,
        };
        add_opening_members(&mut members, &mut diagnostics, wall, &opening, context);
    }

    let layers = wall_layer_bom(wall, system, materials);

    Ok(WallFramePlan {
        wall: wall.id.clone(),
        members,
        layers,
        diagnostics,
    })
}

/// The plan-local layout of a horizontal surface (floor deck / flat ceiling):
/// joists run the full `span_length` along the span axis and are arrayed along
/// the perpendicular layout axis over `layout_length`. v1 is axis-aligned — the
/// span/layout axes are the world X/Y axes chosen from the region's bounding box
/// and the authored [`SpanDirection`]. Non-rectangular regions are framed across
/// their bounding box; clipping to L/T outlines is a later phase.
struct SurfaceLayout {
    /// Each joist's length: the region extent along the span axis.
    span_length: Length,
    /// The extent the joists are arrayed across: the region's other axis.
    layout_length: Length,
}

fn surface_layout(outline: &[Point2], span: SpanDirection) -> SurfaceLayout {
    let Some(first) = outline.first() else {
        return SurfaceLayout {
            span_length: Length::ZERO,
            layout_length: Length::ZERO,
        };
    };
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (first.x, first.y, first.x, first.y);
    for point in &outline[1..] {
        min_x = min_x.min(point.x);
        min_y = min_y.min(point.y);
        max_x = max_x.max(point.x);
        max_y = max_y.max(point.y);
    }
    let width = max_x - min_x; // extent along world X
    let depth = max_y - min_y; // extent along world Y

    // Whether the joists span the world X axis (true) or the world Y axis.
    let span_along_x = match span {
        // Span the shorter clear dimension (the structural default), which also
        // reads as "across the principal/longer axis".
        SpanDirection::Shorter | SpanDirection::Across => width <= depth,
        // Span the longer (principal) axis.
        SpanDirection::Along => width >= depth,
        // Span the world axis nearest the authored direction vector.
        SpanDirection::Explicit(direction) => direction.x.abs() >= direction.y.abs(),
    };

    if span_along_x {
        SurfaceLayout {
            span_length: width,
            layout_length: depth,
        }
    } else {
        SurfaceLayout {
            span_length: depth,
            layout_length: width,
        }
    }
}

/// A human-readable name for the span dimension, for member provenance.
fn span_label(span: SpanDirection) -> &'static str {
    match span {
        SpanDirection::Shorter | SpanDirection::Across => "shorter",
        SpanDirection::Along => "longer",
        SpanDirection::Explicit(_) => "authored",
    }
}

/// The derived contents of one surface (floor deck / flat ceiling) joist plan,
/// before it is keyed to its host element by the public generators.
struct JoistPlan {
    members: Vec<FrameMember>,
    layers: Vec<LayerBomItem>,
    diagnostics: Vec<PlanDiagnostic>,
}

/// Lay out the joists, rim/band members, and blocking for one horizontal surface
/// region (a floor deck or a flat ceiling — they share this generator). Members
/// are placed in the surface's plan-local 2-D frame: a member's `x` is its
/// position along the layout (cross-joist) axis and its `elevation` is its
/// position along the span (along-joist) axis. The surface's true building
/// elevation is a property of the host element and is applied when the surface is
/// rendered, not stored on the member. `joist_kind` is `FloorJoist`/`CeilingJoist`
/// and `prefix` (`"floor"`/`"ceiling"`) namespaces member ids, provenance, and
/// diagnostics.
#[allow(clippy::too_many_arguments)]
fn generate_joist_plan(
    element: &ElementId,
    name: &str,
    system: &ConstructionSystem,
    outline: &[Point2],
    span: SpanDirection,
    joist_kind: MemberKind,
    prefix: &str,
    materials: &[Material],
) -> Result<JoistPlan, SolverError> {
    let framing = system_framing(system)?;
    let band = FramingBand::for_system(system)?;
    let joist = framing.member;
    let spacing = framing.spacing;
    let joist_thickness = joist.thickness();

    let layout = surface_layout(outline, span);
    let positions = stud_positions(layout.layout_length, spacing, joist_thickness);

    let mut members = Vec::new();

    // Common joists: arrayed across the layout axis at on-center spacing, each
    // running the full span; the end joists align with the region edges.
    for mark in &positions {
        members.push(frame_member(
            format!("{prefix}-joist-{}", mark.ticks()),
            element,
            joist_kind,
            joist,
            FrameMemberPlacement::new(
                MemberOrientation::Vertical,
                *mark,
                Length::ZERO,
                layout.span_length,
                joist_thickness,
            ),
            band,
            RuleProvenance::new(
                format!("{prefix}.joists.on-center"),
                format!(
                    "Joists span the {} clear dimension; end joists align with the region edges and interior joists fall on {} layout marks.",
                    span_label(span),
                    spacing
                ),
            ),
        ));
    }

    // Rim/band members close the joist ends at each bearing line, running the
    // full layout width perpendicular to the joists.
    let far_rim = (layout.span_length - joist_thickness).max(Length::ZERO);
    for (index, at) in [Length::ZERO, far_rim].into_iter().enumerate() {
        members.push(frame_member(
            format!("{prefix}-rim-{}", index + 1),
            element,
            MemberKind::RimJoist,
            joist,
            FrameMemberPlacement::new(
                MemberOrientation::Horizontal,
                Length::ZERO,
                at,
                layout.layout_length,
                joist_thickness,
            ),
            band,
            RuleProvenance::new(
                format!("{prefix}.rim.bearing-ends"),
                "A rim/band member closes the joist ends at each bearing line.",
            ),
        ));
    }

    // A single mid-span blocking row: one piece in each clear gap between
    // adjacent joists. A starter rule — real blocking/bridging spacing arrives
    // with code span tables.
    let block_at = ((layout.span_length - joist_thickness) / 2).max(Length::ZERO);
    for pair in positions.windows(2) {
        let start = pair[0] + joist_thickness / 2;
        let gap = pair[1] - pair[0] - joist_thickness;
        if gap <= Length::ZERO {
            continue;
        }
        members.push(frame_member(
            format!("{prefix}-blocking-{}", start.ticks()),
            element,
            MemberKind::Blocking,
            joist,
            FrameMemberPlacement::new(
                MemberOrientation::Horizontal,
                start,
                block_at,
                gap,
                joist_thickness,
            ),
            band,
            RuleProvenance::new(
                format!("{prefix}.blocking.mid-span"),
                "A single mid-span blocking row is generated between joists (starter rule).",
            ),
        ));
    }

    let layers = layers_takeoff(
        system,
        polygon_area_square_inches(outline).round() as i64,
        materials,
    );

    // v1 surfaces structural judgment as a diagnostic, never an enforced span
    // check (real span tables arrive with M4 code profiles).
    let diagnostics = vec![PlanDiagnostic::new(
        DiagnosticSeverity::Info,
        format!("{prefix}.span.not-checked"),
        Some(element.clone()),
        format!(
            "{name} joists are laid out geometrically; their span has not been checked against a code span table."
        ),
    )];

    Ok(JoistPlan {
        members,
        layers,
        diagnostics,
    })
}

/// Generate the framing plan for one floor deck from its resolved plan outline. A
/// sibling of [`generate_wall_plan`]; the open-region case (a `Room` whose wall
/// loop is not closed) is handled by the project pass, which emits a diagnostic
/// instead of calling this.
pub fn generate_floor_plan(
    deck: &FloorDeck,
    system: &ConstructionSystem,
    outline: &[Point2],
    materials: &[Material],
) -> Result<FloorFramePlan, SolverError> {
    let joists = generate_joist_plan(
        &deck.id,
        &deck.name,
        system,
        outline,
        deck.span,
        MemberKind::FloorJoist,
        "floor",
        materials,
    )?;
    Ok(FloorFramePlan {
        floor: deck.id.clone(),
        members: joists.members,
        layers: joists.layers,
        diagnostics: joists.diagnostics,
    })
}

/// Generate the framing plan for one ceiling from its resolved plan outline and the
/// building elevation at its low edge (`reference_elevation = level.elevation +
/// level.height − ceiling.height`). A **flat** ceiling is a floor deck viewed from
/// below, so it reuses the horizontal joisting generator (v1 spans the shorter clear
/// dimension); a **sloped** (scissor/vault) ceiling is framed like a roof plane —
/// its joists run up the slope at true sloped length.
pub fn generate_ceiling_plan(
    ceiling: &Ceiling,
    system: &ConstructionSystem,
    outline: &[Point2],
    reference_elevation: Length,
    materials: &[Material],
) -> Result<CeilingFramePlan, SolverError> {
    // A sloped (scissor/vault) ceiling frames like a roof plane when its low edge
    // yields a valid frame.
    if let Some(slope) = ceiling.slope
        && let Some(frame) = ceiling.frame(reference_elevation)
    {
        return generate_sloped_ceiling_plan(
            ceiling,
            system,
            outline,
            slope,
            frame,
            reference_elevation,
            materials,
        );
    }
    // Flat ceiling — or a sloped ceiling whose low edge is degenerate (a zero-length
    // edge passes validation but frames no slope): the horizontal joist plan, plus a
    // warning if a slope was dropped (mirroring the roof's degenerate-outline path).
    let mut plan = flat_ceiling_plan(ceiling, system, outline, materials)?;
    if ceiling.slope.is_some() {
        plan.diagnostics.push(PlanDiagnostic::new(
            DiagnosticSeverity::Warning,
            "ceiling.outline.degenerate",
            Some(ceiling.id.clone()),
            format!(
                "{} is sloped but its low edge is degenerate (zero length), so the slope was dropped and it was framed flat.",
                ceiling.name
            ),
        ));
    }
    Ok(plan)
}

/// The flat horizontal joist plan for a ceiling — a floor deck viewed from below, so
/// it reuses the joisting generator; v1 spans the shorter clear dimension.
fn flat_ceiling_plan(
    ceiling: &Ceiling,
    system: &ConstructionSystem,
    outline: &[Point2],
    materials: &[Material],
) -> Result<CeilingFramePlan, SolverError> {
    let joists = generate_joist_plan(
        &ceiling.id,
        &ceiling.name,
        system,
        outline,
        SpanDirection::Shorter,
        MemberKind::CeilingJoist,
        "ceiling",
        materials,
    )?;
    Ok(CeilingFramePlan {
        ceiling: ceiling.id.clone(),
        members: joists.members,
        layers: joists.layers,
        diagnostics: joists.diagnostics,
    })
}

/// Frame a sloped (scissor/vault) ceiling like a roof plane: ceiling joists arrayed
/// along the low (spring) edge at on-center spacing, each running up the slope to the
/// high edge cut to its **true sloped length** (plan run × the pitch factor); band
/// joists close the low and high edges; one mid-slope blocking row sits between
/// joists. Spacing and the per-layer takeoff use plan length. The joists carry a
/// [`SlopedPlacement`] (true building elevations) so the BOM and section read the
/// real geometry, mirroring `generate_roof_plan`'s rafters. `slope`/`frame` are the
/// caller's already-resolved `ceiling.slope` / `ceiling.frame(reference_elevation)`.
fn generate_sloped_ceiling_plan(
    ceiling: &Ceiling,
    system: &ConstructionSystem,
    outline: &[Point2],
    slope: CeilingSlope,
    frame: RoofPlaneFrame,
    reference_elevation: Length,
    materials: &[Material],
) -> Result<CeilingFramePlan, SolverError> {
    let framing = system_framing(system)?;
    let band = FramingBand::for_system(system)?;
    let joist = framing.member;
    let spacing = framing.spacing;
    let joist_thickness = joist.thickness();

    let geometry = surface_geometry(outline, slope.low_edge, &frame);
    let factor = slope_factor(slope.pitch);
    let ratio = slope_ratio(slope.pitch);
    let run_extent = geometry.run_extent;
    let high_elevation = reference_elevation + Length::from_inches(run_extent.inches() * ratio);
    // Each joist's true sloped cut and its low/high building elevations.
    let cut_length = Length::from_inches(run_extent.inches() * factor);
    let joist_slope = SlopedPlacement {
        low_elevation: reference_elevation,
        high_elevation,
    };

    let mut members = Vec::new();

    // Common ceiling joists: arrayed along the low edge at o.c., each running up the
    // slope to the high edge. End joists align with the rake edges.
    let positions = stud_positions(geometry.eave_length, spacing, joist_thickness);
    for mark in &positions {
        members.push(frame_member(
            format!("ceiling-joist-{}", mark.ticks()),
            &ceiling.id,
            MemberKind::CeilingJoist,
            joist,
            FrameMemberPlacement::new(
                MemberOrientation::Vertical,
                *mark,
                Length::ZERO,
                cut_length,
                joist_thickness,
            )
            .with_slope(joist_slope),
            band,
            RuleProvenance::new(
                "ceiling.joists.on-slope",
                format!(
                    "Sloped ceiling joists span the {} plan run perpendicular to the low edge; end joists align with the rake edges and interior joists fall on {} layout marks. The cut length is the true sloped length (plan run times the {}:{} pitch).",
                    run_extent, spacing, slope.pitch.rise, slope.pitch.run
                ),
            ),
        ));
    }

    // Band joists close the joist ends at the low and high edges, running the full
    // eave length perpendicular to the joists; each is level at its edge's elevation.
    for (index, v, at_elevation) in [
        (1usize, Length::ZERO, reference_elevation),
        (2, run_extent, high_elevation),
    ] {
        members.push(frame_member(
            format!("ceiling-rim-{index}"),
            &ceiling.id,
            MemberKind::RimJoist,
            joist,
            FrameMemberPlacement::new(
                MemberOrientation::Horizontal,
                Length::ZERO,
                v,
                geometry.eave_length,
                joist_thickness,
            )
            .with_slope(SlopedPlacement {
                low_elevation: at_elevation,
                high_elevation: at_elevation,
            }),
            band,
            RuleProvenance::new(
                "ceiling.rim.sloped-ends",
                "A band joist closes the joist ends at each bearing edge of the sloped ceiling.",
            ),
        ));
    }

    // A single mid-slope blocking row: one level piece in each clear gap between
    // adjacent joists, on the sloped surface (a starter rule).
    let mid_v = (run_extent / 2).max(Length::ZERO);
    let mid_elevation = reference_elevation + Length::from_inches(mid_v.inches() * ratio);
    let mid_slope = SlopedPlacement {
        low_elevation: mid_elevation,
        high_elevation: mid_elevation,
    };
    for pair in positions.windows(2) {
        let start = pair[0] + joist_thickness / 2;
        let gap = pair[1] - pair[0] - joist_thickness;
        if gap <= Length::ZERO {
            continue;
        }
        members.push(frame_member(
            format!("ceiling-blocking-{}", start.ticks()),
            &ceiling.id,
            MemberKind::Blocking,
            joist,
            FrameMemberPlacement::new(
                MemberOrientation::Horizontal,
                start,
                mid_v,
                gap,
                joist_thickness,
            )
            .with_slope(mid_slope),
            band,
            RuleProvenance::new(
                "ceiling.blocking.mid-slope",
                "A single mid-slope blocking row is generated between sloped ceiling joists (starter rule).",
            ),
        ));
    }

    // Plan footprint area for the per-layer takeoff (plan, not true surface area).
    let layers = layers_takeoff(
        system,
        polygon_area_square_inches(outline).round() as i64,
        materials,
    );

    // v2 surfaces structural judgment as a diagnostic: a scissor/vault ceiling is not
    // a full rafter tie, so the ridge fork (A1.1) flags a beam — see the project pass.
    let diagnostics = vec![
        PlanDiagnostic::new(
            DiagnosticSeverity::Info,
            "ceiling.span.not-checked",
            Some(ceiling.id.clone()),
            format!(
                "{} sloped joists are laid out geometrically; their span has not been checked against a code span table.",
                ceiling.name
            ),
        ),
        PlanDiagnostic::new(
            DiagnosticSeverity::Info,
            "ceiling.slope.scissor",
            Some(ceiling.id.clone()),
            format!(
                "{} is a sloped (scissor/vault) ceiling: it is not a flat rafter tie at the plate, so a roof over it relies on a structural ridge beam (see the roof's ridge diagnostic).",
                ceiling.name
            ),
        ),
    ];

    Ok(CeilingFramePlan {
        ceiling: ceiling.id.clone(),
        members,
        layers,
        diagnostics,
    })
}

/// The plan-projected geometry of a roof plane derived from its outline and
/// designated eave edge: the eave (layout) length the rafters are arrayed along,
/// the up-slope run extent they span in plan, and the high (ridge) edge used to
/// detect a shared gable ridge. All lengths are horizontal plan projections; the
/// generator scales the run by the pitch factor to get the true sloped cut.
struct RoofPlaneGeometry {
    /// Length of the eave edge — the layout axis the rafters array along.
    eave_length: Length,
    /// Greatest perpendicular (up-slope) plan distance from the eave line to the
    /// outline: how far the rafters run in plan from the eave to the ridge.
    run_extent: Length,
    /// The high (ridge) edge as an endpoint pair (exact outline points), compared
    /// unordered to find two planes that share a gable ridge.
    high_edge: (Point2, Point2),
    /// The high-edge plan length. Equal to the eave length for a rectangular
    /// gable, shorter for a hip trapezoid whose ridge stops at the hip rafters.
    high_edge_length: Length,
}

/// Derive a roof plane's plan geometry. Returns `None` for a degenerate outline
/// (no eave length) the model validator would normally reject first. The up-slope
/// direction is the eave-perpendicular oriented toward the outline centroid, so
/// it is robust to the polygon's winding.
fn roof_plane_geometry(plane: &RoofPlane) -> Option<RoofPlaneGeometry> {
    // The eave origin + up-slope unit normal (toward the outline centroid, so it is
    // winding-independent) come from the shared `framer-core` frame, so the framing
    // and the rendered surface cannot drift.
    let frame = plane.frame()?;
    Some(surface_geometry(&plane.outline, plane.eave_edge, &frame))
}

/// The plan-projected geometry of a sloped surface (a roof plane or a sloped
/// ceiling) from its outline, low (eave) edge, and shared [`RoofPlaneFrame`]: the
/// eave (layout) length, the up-slope plan run extent, and the high edge. Shared by
/// roof planes and sloped ceilings so they frame identically.
fn surface_geometry(
    outline: &[Point2],
    low_edge: u32,
    frame: &RoofPlaneFrame,
) -> RoofPlaneGeometry {
    let n = outline.len();

    // Run extent: the farthest up-slope perpendicular distance to any vertex.
    let run_extent = outline
        .iter()
        .map(|p| frame.up_slope_distance(p.x.inches(), p.y.inches()))
        .fold(0.0_f64, f64::max);

    // High (ridge) edge: the outline edge whose midpoint is farthest up-slope.
    let i = low_edge as usize % n;
    let mut high_edge = (outline[i], outline[(i + 1) % n]);
    let mut best = f64::MIN;
    for k in 0..n {
        let p = outline[k];
        let q = outline[(k + 1) % n];
        let d = frame.up_slope_distance(
            (p.x.inches() + q.x.inches()) / 2.0,
            (p.y.inches() + q.y.inches()) / 2.0,
        );
        if d > best {
            best = d;
            high_edge = (p, q);
        }
    }

    RoofPlaneGeometry {
        eave_length: Length::from_inches(frame.eave_length()),
        run_extent: Length::from_inches(run_extent),
        high_edge,
        high_edge_length: edge_length(high_edge),
    }
}

fn edge_length(edge: (Point2, Point2)) -> Length {
    let dx = edge.1.x.inches() - edge.0.x.inches();
    let dy = edge.1.y.inches() - edge.0.y.inches();
    Length::from_inches((dx * dx + dy * dy).sqrt())
}

/// The longest up-slope plan run available at one rafter layout mark before the
/// rafter hits the roof polygon boundary. Rectangular/gable planes return the
/// full run; hip trapezoids and triangles shorten near the hips so those marks
/// become jack rafters instead of common rafters overrunning the hip line.
fn rafter_run_at_mark(
    plane: &RoofPlane,
    frame: &RoofPlaneFrame,
    geometry: &RoofPlaneGeometry,
    mark: Length,
) -> Length {
    const EPS: f64 = 1.0e-7;

    let target = mark.inches();
    let local: Vec<(f64, f64)> = plane
        .outline
        .iter()
        .map(|point| {
            (
                eave_axis_position(plane, *point).inches(),
                frame.up_slope_distance(point.x.inches(), point.y.inches()),
            )
        })
        .collect();

    let mut runs = Vec::new();
    for index in 0..local.len() {
        let (x0, v0) = local[index];
        let (x1, v1) = local[(index + 1) % local.len()];

        if (x0 - target).abs() <= EPS {
            runs.push(v0);
        }
        if (x1 - target).abs() <= EPS {
            runs.push(v1);
        }
        if (x0 - x1).abs() <= EPS {
            if (x0 - target).abs() <= EPS {
                runs.push(v0);
                runs.push(v1);
            }
            continue;
        }

        let min_x = x0.min(x1);
        let max_x = x0.max(x1);
        if target > min_x + EPS && target < max_x - EPS {
            let t = (target - x0) / (x1 - x0);
            runs.push(v0 + t * (v1 - v0));
        }
    }

    let full_run = geometry.run_extent.inches();
    let run = runs.into_iter().fold(0.0_f64, f64::max);
    Length::from_inches(run.clamp(0.0, full_run))
}

/// True sloped length per unit plan run for a pitch (1 when flat, ≥ 1 otherwise).
fn slope_factor(slope: Slope) -> f64 {
    let rise = slope.rise.inches();
    let run = slope.run.inches();
    if run <= 0.0 {
        return 1.0;
    }
    ((run * run + rise * rise).sqrt() / run).max(1.0)
}

/// Rise (vertical) per unit plan run for a pitch.
fn slope_ratio(slope: Slope) -> f64 {
    let run = slope.run.inches();
    if run <= 0.0 {
        return 0.0;
    }
    slope.rise.inches() / run
}

/// Whether two edges (endpoint pairs) are the same segment, ignoring direction.
fn same_edge(a: (Point2, Point2), b: (Point2, Point2)) -> bool {
    (a.0 == b.0 && a.1 == b.1) || (a.0 == b.1 && a.1 == b.0)
}

/// The true building elevation of a plane's ridge (high edge): the eave springing
/// raised by the plan run times the pitch. Used both to place the ridge board and
/// to check that two planes sharing a ridge edge agree on its height.
fn ridge_elevation(plane: &RoofPlane, run_extent: Length) -> Length {
    plane.reference_elevation + Length::from_inches(run_extent.inches() * slope_ratio(plane.slope))
}

/// Whether a roof plane's ridge is restrained by a rafter-thrust tie. Encoding the
/// ridge-board-vs-beam decision as a type (rather than a bare `bool`) keeps it from
/// being silently transposed with the adjacent `carries_ridge` flag at a call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RidgeTie {
    /// A tie (a flat ceiling at the plate) resists thrust — a ridge board suffices.
    Tied,
    /// No tie (a cathedral / scissor condition) — a structural ridge beam is required.
    Untied,
}

/// Generate the framing plan for one roof plane: common rafters arrayed along the
/// eave at on-center spacing (running up the slope to the ridge, cut to their
/// true sloped length), level plate blocking between rafters at the eave, and —
/// when `carries_ridge` — a ridge board along the high edge plus the `ridge_tie`
/// structural judgment (ridge board adequate vs. ridge beam required). A sibling of
/// [`generate_wall_plan`]. Spacing and area use plan (horizontal) length; only
/// the rafter cut length is the true sloped length, rounded to integer ticks at
/// this f64 boundary.
pub fn generate_roof_plan(
    plane: &RoofPlane,
    system: &ConstructionSystem,
    materials: &[Material],
    carries_ridge: bool,
    ridge_tie: RidgeTie,
) -> Result<RoofFramePlan, SolverError> {
    let framing = system_framing(system)?;
    let band = FramingBand::for_system(system)?;
    let rafter = framing.member;
    let spacing = framing.spacing;
    let rafter_thickness = rafter.thickness();

    let mut members = Vec::new();
    let mut diagnostics = Vec::new();

    let Some(geometry) = roof_plane_geometry(plane) else {
        diagnostics.push(PlanDiagnostic::new(
            DiagnosticSeverity::Warning,
            "roof.outline.degenerate",
            Some(plane.id.clone()),
            format!(
                "{} has a degenerate outline, so its rafters cannot be laid out.",
                plane.name
            ),
        ));
        return Ok(RoofFramePlan {
            roof: plane.id.clone(),
            members,
            layers: Vec::new(),
            diagnostics,
        });
    };

    let frame = plane
        .frame()
        .expect("roof_plane_geometry returned Some only when the frame is valid");
    let factor = slope_factor(plane.slope);
    let ratio = slope_ratio(plane.slope);
    let run_extent = geometry.run_extent;
    let overhang = plane.eave_overhang;

    // Plane-local v = 0 is the eave bearing line; the tail sits at v = -overhang.
    let tail_v = Length::ZERO - overhang;
    let ridge_elevation = ridge_elevation(plane, run_extent);
    let tail_elevation = plane.reference_elevation - Length::from_inches(overhang.inches() * ratio);

    // Rafters are arrayed along the eave (layout) axis at o.c. A mark that can
    // reach the full high edge is a common rafter; a mark whose run is clipped by
    // a hip line becomes a jack rafter and dies into that hip.
    let positions = stud_positions(geometry.eave_length, spacing, rafter_thickness);
    for mark in &positions {
        let local_run = rafter_run_at_mark(plane, &frame, &geometry, *mark);
        let is_jack = run_extent - local_run > Length::from_ticks(1);
        let total_plan_run = local_run + overhang;
        let cut_length = Length::from_inches(total_plan_run.inches() * factor);
        let high_elevation =
            plane.reference_elevation + Length::from_inches(local_run.inches() * ratio);
        let member_kind = if is_jack {
            MemberKind::JackRafter
        } else {
            MemberKind::Rafter
        };
        let (id, rule_id, summary) = if is_jack {
            (
                format!("jack-rafter-{}", mark.ticks()),
                "roof.jack-rafters.intersection",
                format!(
                    "A jack rafter at layout mark {mark} dies into a hip or valley after a {} plan run; its cut length is the true sloped length.",
                    local_run
                ),
            )
        } else {
            (
                format!("rafter-{}", mark.ticks()),
                "roof.rafters.on-center",
                format!(
                    "Common rafters span the {} plan run perpendicular to the eave; end rafters align with the rake edges and interior rafters fall on {} layout marks. The cut length is the true sloped length (plan run times the {}:{} pitch).",
                    run_extent, spacing, plane.slope.rise, plane.slope.run
                ),
            )
        };

        members.push(frame_member(
            id,
            &plane.id,
            member_kind,
            rafter,
            FrameMemberPlacement::new(
                MemberOrientation::Vertical,
                *mark,
                tail_v,
                cut_length,
                rafter_thickness,
            )
            .with_slope(SlopedPlacement {
                low_elevation: tail_elevation,
                high_elevation,
            }),
            band,
            RuleProvenance::new(rule_id, summary),
        ));
    }

    // Plate blocking: one level piece in each clear gap between adjacent rafters
    // at the eave bearing line (a starter rule).
    let eave_slope = SlopedPlacement {
        low_elevation: plane.reference_elevation,
        high_elevation: plane.reference_elevation,
    };
    for pair in positions.windows(2) {
        let start = pair[0] + rafter_thickness / 2;
        let gap = pair[1] - pair[0] - rafter_thickness;
        if gap <= Length::ZERO {
            continue;
        }
        members.push(frame_member(
            format!("roof-blocking-{}", start.ticks()),
            &plane.id,
            MemberKind::Blocking,
            rafter,
            FrameMemberPlacement::new(
                MemberOrientation::Horizontal,
                start,
                Length::ZERO,
                gap,
                rafter_thickness,
            )
            .with_slope(eave_slope),
            band,
            RuleProvenance::new(
                "roof.blocking.eave",
                "Level blocking is generated between rafters at the eave bearing line (starter rule).",
            ),
        ));
    }

    // The ridge board runs level along the shared gable ridge; the opposing
    // planes' rafters bear against it. Only the ridge-carrying plane emits it so
    // a gable's single ridge is counted once.
    if carries_ridge {
        members.push(frame_member(
            "ridge-board",
            &plane.id,
            MemberKind::RidgeBoard,
            rafter,
            FrameMemberPlacement::new(
                MemberOrientation::Horizontal,
                Length::ZERO,
                run_extent,
                geometry.high_edge_length,
                rafter_thickness,
            )
            .with_slope(SlopedPlacement {
                low_elevation: ridge_elevation,
                high_elevation: ridge_elevation,
            }),
            band,
            RuleProvenance::new(
                "roof.ridge.gable",
                "A ridge board runs level along the shared gable ridge using the system framing member; the opposing rafters bear against it.",
            ),
        ));
        match ridge_tie {
            RidgeTie::Tied => diagnostics.push(PlanDiagnostic::new(
                DiagnosticSeverity::Info,
                "roof.ridge.tied",
                Some(plane.id.clone()),
                "A ridge board is adequate here: a flat ceiling at the plate ties the opposing rafters against outward thrust.",
            )),
            RidgeTie::Untied => diagnostics.push(PlanDiagnostic::new(
                DiagnosticSeverity::Unsupported,
                "roof.ridge.beam-required",
                Some(plane.id.clone()),
                "No ceiling tie resists rafter thrust at the plate (a cathedral or scissor condition), so a structural ridge beam is required. v1 frames a ridge board and does not size the beam.",
            )),
        }
    }

    // v1 surfaces structural judgment as a diagnostic, never an enforced span
    // check (real rafter span tables arrive with M4 code profiles).
    diagnostics.push(PlanDiagnostic::new(
        DiagnosticSeverity::Info,
        "roof.span.not-checked",
        Some(plane.id.clone()),
        format!(
            "{} rafters are laid out geometrically; their span has not been checked against a code span table.",
            plane.name
        ),
    ));

    // The roof's per-layer takeoff uses the plan footprint area (v1 does not yet
    // scale roofing goods by the slope to true surface area).
    let layers = layers_takeoff(
        system,
        polygon_area_square_inches(&plane.outline).round() as i64,
        materials,
    );

    Ok(RoofFramePlan {
        roof: plane.id.clone(),
        members,
        layers,
        diagnostics,
    })
}

/// How a roof plane relates to a shared gable ridge, deciding whether it emits a
/// ridge board.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RidgeCondition {
    /// No plane shares this plane's high edge (a shed / lone plane) — no ridge.
    None,
    /// This plane carries the single ridge board for a matched gable (it holds
    /// the smallest id among the planes sharing the ridge at the same elevation).
    Carries,
    /// Shares a matched ridge but defers the single ridge board to a smaller-id
    /// plane, so emits none itself.
    Deferred,
    /// Shares a high edge in plan but the sharing planes disagree on the ridge's
    /// true elevation (different pitch or springing), so a single shared ridge is
    /// geometrically invalid: each plane frames its own ridge and a diagnostic is
    /// emitted.
    Mismatched,
}

/// Classify the ridge condition of `planes[index]` by comparing its high (ridge)
/// edge — and the true elevation of that ridge — against every other plane. Two
/// planes that share the plan-projected edge but compute different ridge
/// elevations cannot meet at one ridge, so the shared edge alone is not enough. A
/// 1-tick tolerance absorbs rounding; matched gables produce identical ticks.
///
/// `geometries` holds each plane's precomputed [`RoofPlaneGeometry`] (aligned with
/// `planes`), so the per-plane outline pass runs once rather than re-deriving on
/// every comparison.
fn ridge_condition(
    index: usize,
    planes: &[RoofPlane],
    geometries: &[Option<RoofPlaneGeometry>],
) -> RidgeCondition {
    let plane = &planes[index];
    let Some(geometry) = geometries[index].as_ref() else {
        return RidgeCondition::None;
    };
    let my_ridge = ridge_elevation(plane, geometry.run_extent);
    let mut shares = false;
    let mut elevations_agree = true;
    let mut defer = false;
    for (other_index, other) in planes.iter().enumerate() {
        if other_index == index {
            continue;
        }
        let Some(other_geometry) = geometries[other_index].as_ref() else {
            continue;
        };
        if !same_edge(geometry.high_edge, other_geometry.high_edge) {
            continue;
        }
        shares = true;
        let other_ridge = ridge_elevation(other, other_geometry.run_extent);
        if (my_ridge - other_ridge).abs() > Length::from_ticks(1) {
            elevations_agree = false;
        } else if other.id < plane.id {
            // A smaller-id, elevation-matched sharer carries the single ridge.
            defer = true;
        }
    }

    match (shares, elevations_agree, defer) {
        (false, _, _) => RidgeCondition::None,
        (true, false, _) => RidgeCondition::Mismatched,
        (true, true, true) => RidgeCondition::Deferred,
        (true, true, false) => RidgeCondition::Carries,
    }
}

fn eave_axis_position(plane: &RoofPlane, point: Point2) -> Length {
    if plane.outline.len() < 2 {
        return Length::ZERO;
    }
    let i = plane.eave_edge as usize % plane.outline.len();
    let a = plane.outline[i];
    let b = plane.outline[(i + 1) % plane.outline.len()];
    let dx = b.x.inches() - a.x.inches();
    let dy = b.y.inches() - a.y.inches();
    let length = (dx * dx + dy * dy).sqrt();
    if length <= f64::EPSILON {
        return Length::ZERO;
    }
    let px = point.x.inches() - a.x.inches();
    let py = point.y.inches() - a.y.inches();
    Length::from_inches((px * dx + py * dy) / length)
}

fn matching_edge_elevations(
    first: &RoofPlane,
    second: &RoofPlane,
    edge: (Point2, Point2),
) -> Option<(Length, Length)> {
    let first_elevations = edge_elevations(first, edge)?;
    let second_elevations = edge_elevations(second, edge)?;
    if (first_elevations.0 - second_elevations.0).abs() > Length::from_ticks(1)
        || (first_elevations.1 - second_elevations.1).abs() > Length::from_ticks(1)
    {
        return None;
    }
    Some(first_elevations)
}

fn edge_elevations(plane: &RoofPlane, edge: (Point2, Point2)) -> Option<(Length, Length)> {
    let frame = plane.frame()?;
    Some((
        Length::from_inches(frame.elevation_at(edge.0.x.inches(), edge.0.y.inches())),
        Length::from_inches(frame.elevation_at(edge.1.x.inches(), edge.1.y.inches())),
    ))
}

fn shared_edge_elevations_match(
    first: &RoofPlane,
    second: &RoofPlane,
    edge: (Point2, Point2),
) -> bool {
    matching_edge_elevations(first, second, edge).is_some()
}

fn concave_footprint_corners(model: &BuildingModel, level: &ElementId) -> Vec<Point2> {
    level_wall_loop_outline(model, level)
        .map(|outline| concave_polygon_corners(&outline))
        .unwrap_or_default()
}

fn shared_edge_has_concave_endpoint(corners: &[Point2], edge: (Point2, Point2)) -> bool {
    corners
        .iter()
        .any(|corner| *corner == edge.0 || *corner == edge.1)
}

fn add_roof_intersection_members(
    plan: &mut ProjectFramePlan,
    model: &BuildingModel,
    geometries: &[Option<RoofPlaneGeometry>],
) -> Result<(), SolverError> {
    let mut concave_corners_by_level = BTreeMap::new();
    for plane in &model.roof_planes {
        concave_corners_by_level
            .entry(plane.level.clone())
            .or_insert_with(|| concave_footprint_corners(model, &plane.level));
    }

    for (first_index, first) in model.roof_planes.iter().enumerate() {
        let Some(first_geometry) = geometries[first_index].as_ref() else {
            continue;
        };
        for (second_index, second) in model.roof_planes.iter().enumerate().skip(first_index + 1) {
            let Some(second_geometry) = geometries[second_index].as_ref() else {
                continue;
            };
            for edge_index in 0..first.outline.len() {
                let edge = (
                    first.outline[edge_index],
                    first.outline[(edge_index + 1) % first.outline.len()],
                );
                let shared = (0..second.outline.len()).any(|other_edge_index| {
                    same_edge(
                        edge,
                        (
                            second.outline[other_edge_index],
                            second.outline[(other_edge_index + 1) % second.outline.len()],
                        ),
                    )
                });
                if !shared {
                    continue;
                }
                if same_edge(edge, first_geometry.high_edge)
                    && same_edge(edge, second_geometry.high_edge)
                {
                    // A shared high edge is the ridge board, not a hip.
                    continue;
                }
                let is_valley = first.level == second.level
                    && concave_corners_by_level
                        .get(&first.level)
                        .is_some_and(|corners| shared_edge_has_concave_endpoint(corners, edge));
                let elevations_match = shared_edge_elevations_match(first, second, edge);
                if is_valley && !elevations_match {
                    let owner = if first.id <= second.id { first } else { second };
                    if let Some(owner_plan) = plan
                        .roof_plans
                        .iter_mut()
                        .find(|roof_plan| roof_plan.roof == owner.id)
                    {
                        owner_plan.diagnostics.push(PlanDiagnostic::new(
                            DiagnosticSeverity::Unsupported,
                            "roof.valley.unequal-pitch",
                            Some(owner.id.clone()),
                            format!(
                                "The shared valley edge between {} and {} does not have matching elevations on both roof planes. v2 only frames equal-pitch right-angle valleys.",
                                first.name, second.name
                            ),
                        ));
                    }
                    continue;
                }
                let Some((a_elevation, b_elevation)) =
                    matching_edge_elevations(first, second, edge)
                else {
                    continue;
                };
                if (a_elevation - b_elevation).abs() <= Length::from_ticks(1) {
                    continue;
                }

                let (owner, peer) = if first.id <= second.id {
                    (first, second)
                } else {
                    (second, first)
                };
                let system = system_by_id(model, &owner.system).ok_or_else(|| {
                    SolverError::MissingSystemForElement {
                        element: owner.id.clone(),
                        system: owner.system.clone(),
                    }
                })?;
                let framing = system_framing(system)?;
                let band = FramingBand::for_system(system)?;
                let frame = owner.frame();
                let (low_point, low_elevation, high_elevation) = if a_elevation <= b_elevation {
                    (edge.0, a_elevation, b_elevation)
                } else {
                    (edge.1, b_elevation, a_elevation)
                };
                let rise = high_elevation - low_elevation;
                let plan_length = edge_length(edge);
                let cut_length = Length::from_inches(
                    (plan_length.inches().powi(2) + rise.inches().powi(2)).sqrt(),
                );
                let placement_run = frame
                    .map(|frame| {
                        Length::from_inches(
                            frame.up_slope_distance(low_point.x.inches(), low_point.y.inches()),
                        )
                    })
                    .unwrap_or(Length::ZERO);
                let placement_x = eave_axis_position(owner, low_point);
                let member = frame_member(
                    format!(
                        "{}-rafter-{}-edge-{edge_index}",
                        if is_valley { "valley" } else { "hip" },
                        peer.id.0
                    ),
                    &owner.id,
                    if is_valley {
                        MemberKind::ValleyRafter
                    } else {
                        MemberKind::HipRafter
                    },
                    framing.member,
                    FrameMemberPlacement::new(
                        MemberOrientation::Vertical,
                        placement_x,
                        placement_run,
                        cut_length,
                        framing.member.thickness(),
                    )
                    .with_slope(SlopedPlacement {
                        low_elevation,
                        high_elevation,
                    }),
                    band,
                    if is_valley {
                        RuleProvenance::new(
                            "roof.valley.equal-pitch",
                            format!(
                                "A valley rafter follows the shared sloped edge between {} and {}; v2 frames equal-pitch right-angle valleys and cuts the member to its true sloped length.",
                                owner.name, peer.name
                            ),
                        )
                    } else {
                        RuleProvenance::new(
                            "roof.hip.shared-edge",
                            format!(
                                "A hip rafter follows the shared sloped edge between {} and {}; its cut length is the true length from plate corner to ridge end.",
                                owner.name, peer.name
                            ),
                        )
                    },
                );
                if let Some(owner_plan) = plan
                    .roof_plans
                    .iter_mut()
                    .find(|roof_plan| roof_plan.roof == owner.id)
                {
                    owner_plan.members.push(member);
                }
            }
        }
    }
    Ok(())
}

/// A varying-plate-height message when the walls on a roof plane's level have
/// more than one distinct height. v1 assumes one plate height per roof and does
/// not read per-level bearing elevations, so this is surfaced as `Unsupported`.
fn varying_plate_height(model: &BuildingModel, plane: &RoofPlane) -> Option<String> {
    let mut heights: Vec<Length> = model
        .walls
        .iter()
        .filter(|wall| wall.level == plane.level)
        .map(|wall| wall.height)
        .collect();
    heights.sort_unstable();
    heights.dedup();
    (heights.len() > 1).then(|| {
        format!(
            "Walls on this roof's level have {} distinct heights; v1 assumes one plate height per roof and frames rafters to a single bearing line.",
            heights.len()
        )
    })
}

/// A horizontal surface that can tie rafter thrust at a plate: the level it bears
/// on, its true elevation, and its resolved plan outline. Both flat ceilings and
/// floor decks are gathered; whether one actually ties a given roof is an
/// *elevation* decision (see [`roof_has_thrust_tie`]), not a type decision — a
/// floor deck at the level base sits far below the plate, a flat ceiling at the
/// plate does not.
struct ThrustTie {
    level: ElementId,
    elevation: Length,
    outline: Vec<Point2>,
}

/// Collect the horizontal surfaces that can tie rafter thrust, resolving each
/// region outline once. A flat ceiling hangs `height` below the level top; a floor
/// deck bears at the level elevation. A sloped (scissor/vault) ceiling is excluded
/// — it is not a full tie. A floor deck is gathered (per spec Decision #10), but it
/// only ties when its elevation reaches a roof's plate; in v2's single-level models
/// a same-level deck is at the floor and is filtered out by the elevation gate, so
/// the cross-level floor-of-N+1 = ceiling-of-N case stays correctly deferred.
fn collect_thrust_ties(model: &BuildingModel) -> Vec<ThrustTie> {
    let level_of = |id: &ElementId| model.levels.iter().find(|level| level.id == *id);
    let mut ties = Vec::new();

    // Flat ceilings hang `height` below the level top (`elevation + height`).
    let flats: Vec<&Ceiling> = model
        .ceilings
        .iter()
        .filter(|ceiling| ceiling.slope.is_none())
        .collect();
    let ceiling_regions: Vec<&SurfaceRegion> =
        flats.iter().map(|ceiling| &ceiling.region).collect();
    for (ceiling, outline) in flats
        .iter()
        .zip(resolve_surface_regions(model, &ceiling_regions))
    {
        let (Some(outline), Some(level)) = (outline, level_of(&ceiling.level)) else {
            continue;
        };
        ties.push(ThrustTie {
            level: ceiling.level.clone(),
            elevation: level.elevation + level.height - ceiling.height,
            outline,
        });
    }

    // Floor decks bear at the level elevation.
    let deck_regions: Vec<&SurfaceRegion> =
        model.floor_decks.iter().map(|deck| &deck.region).collect();
    for (deck, outline) in model
        .floor_decks
        .iter()
        .zip(resolve_surface_regions(model, &deck_regions))
    {
        let (Some(outline), Some(level)) = (outline, level_of(&deck.level)) else {
            continue;
        };
        ties.push(ThrustTie {
            level: deck.level.clone(),
            elevation: level.elevation,
            outline,
        });
    }

    ties
}

/// Slack below the bearing line within which a flat ceiling still counts as a tie
/// — absorbing the top-plate thickness and rafter seat. A ceiling dropped further
/// than this no longer restrains the rafters' outward thrust.
const TIE_PLATE_SLACK: Length = Length::from_ticks(16 * 6); // 6 inches

/// Whether a rafter-thrust tie sits at this roof plane's bearing line: a gathered
/// flat ceiling or floor deck on the same level whose plan region encloses the
/// plane footprint (centroid test) and whose elevation is at or above the plane's
/// bearing line (`reference_elevation`, less the plate slack). The elevation gate
/// is what excludes a floor deck at the level base. Without such a tie a gable's
/// ridge needs a structural beam.
fn roof_has_thrust_tie(plane: &RoofPlane, ties: &[ThrustTie]) -> bool {
    let Some(sample) = polygon_centroid(&plane.outline) else {
        return false;
    };
    ties.iter().any(|tie| {
        tie.level == plane.level
            && tie.elevation + TIE_PLATE_SLACK >= plane.reference_elevation
            && point_in_polygon(sample, &tie.outline)
    })
}

/// The vertex centroid of a plan polygon — an interior sample point for a convex
/// roof footprint (gable halves, hip trapezoids/triangles). `None` for an empty
/// outline.
fn polygon_centroid(points: &[Point2]) -> Option<Point2> {
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

/// Generate the roof-plane rafter plans, one per authored plane. The plane that
/// carries a matched gable ridge emits the single shared ridge board; a gable
/// whose planes disagree on the ridge elevation frames a ridge per plane and is
/// flagged. Varying plate heights under a roof are flagged as unsupported.
fn generate_roof_plans(
    plan: &mut ProjectFramePlan,
    model: &BuildingModel,
) -> Result<(), SolverError> {
    // Derive each plane's plan geometry once; ridge classification then compares
    // these rather than re-deriving an outline pass per pairwise check.
    let geometries: Vec<Option<RoofPlaneGeometry>> =
        model.roof_planes.iter().map(roof_plane_geometry).collect();
    // Gather the flat-ceiling ties once so each ridge plane can ask whether a tie
    // resists its rafter thrust (ridge board vs. structural ridge beam).
    let ties = collect_thrust_ties(model);
    // Classify each region cathedral (no ceiling) vs. attic (a ceiling encloses it)
    // in one wall-graph pass — the same classifier the renderers use, so the
    // diagnostic agrees with the rendered cathedral underside.
    let cathedral_flags = model.roof_cathedral_flags();
    for (index, plane) in model.roof_planes.iter().enumerate() {
        let system = system_by_id(model, &plane.system).ok_or_else(|| {
            SolverError::MissingSystemForElement {
                element: plane.id.clone(),
                system: plane.system.clone(),
            }
        })?;
        // A matched gable's ridge is carried by one plane; a mismatched gable
        // frames a ridge on each plane (a single shared ridge would float away
        // from one side's rafters).
        let condition = ridge_condition(index, &model.roof_planes, &geometries);
        let carries_ridge = matches!(
            condition,
            RidgeCondition::Carries | RidgeCondition::Mismatched
        );
        let ridge_tie = if roof_has_thrust_tie(plane, &ties) {
            RidgeTie::Tied
        } else {
            RidgeTie::Untied
        };
        let mut roof_plan =
            generate_roof_plan(plane, system, &model.materials, carries_ridge, ridge_tie)?;
        if condition == RidgeCondition::Mismatched {
            roof_plan.diagnostics.push(PlanDiagnostic::new(
                DiagnosticSeverity::Unsupported,
                "roof.ridge.mismatched-elevation",
                Some(plane.id.clone()),
                "This plane shares a ridge edge with another whose pitch or springing puts the ridge at a different height; a single shared ridge cannot be framed, so each plane is given its own ridge. A matched gable (equal pitch and bearing) frames one shared ridge.",
            ));
        }
        if let Some(message) = varying_plate_height(model, plane) {
            roof_plan.diagnostics.push(PlanDiagnostic::new(
                DiagnosticSeverity::Unsupported,
                "roof.plate-height.varying",
                Some(plane.id.clone()),
                message,
            ));
        }
        // Cathedral vs. attic: a derived region classification (Info) so Plan Mode
        // can explain the space between the roof and any ceiling below, making
        // A1.1's ridge-board-vs-beam fork legible. Skipped for a degenerate plane
        // (it frames nothing). v2 Phase A treats any covering ceiling as an attic;
        // distinguishing a scissor/vaulted ceiling is deferred to the sloped-ceiling
        // slice (A3.2).
        if geometries[index].is_some() {
            let (code, message) = if cathedral_flags[index] {
                (
                    "roof.ceiling.cathedral",
                    format!(
                        "No ceiling encloses {}, so it is a cathedral region: the roof underside is the finished ceiling surface.",
                        plane.name
                    ),
                )
            } else {
                (
                    "roof.ceiling.attic",
                    format!(
                        "A ceiling encloses {}, so the space between it and the roof is an attic.",
                        plane.name
                    ),
                )
            };
            roof_plan.diagnostics.push(PlanDiagnostic::new(
                DiagnosticSeverity::Info,
                code,
                Some(plane.id.clone()),
                message,
            ));
        }
        plan.roof_plans.push(roof_plan);
    }
    add_roof_intersection_members(plan, model, &geometries)?;
    Ok(())
}

/// Resolve every surface region to its closed plan outline in one pass per level's
/// wall graph. `Polygon` regions are their own outline; `Room` regions are
/// resolved through [`room_boundaries_for_rooms`] so stacked levels with different
/// walls do not bleed into each other. Each entry lines up with `regions`; `None`
/// marks an open `Room` loop (a transient mid-edit condition, surfaced as a
/// diagnostic) or an unresolvable room reference.
fn resolve_surface_regions(
    model: &BuildingModel,
    regions: &[&SurfaceRegion],
) -> Vec<Option<Vec<Point2>>> {
    // The room backing each `Room` region, in order, plus where its boundary will
    // land in the batch result (`None` for `Polygon` regions and unknown rooms).
    let mut rooms = Vec::new();
    let mut slots = Vec::with_capacity(regions.len());
    for region in regions {
        match region {
            SurfaceRegion::Polygon(_) => slots.push(None),
            SurfaceRegion::Room(room) => {
                match model.rooms.iter().find(|candidate| candidate.id == *room) {
                    Some(found) => {
                        slots.push(Some(rooms.len()));
                        rooms.push(found);
                    }
                    None => slots.push(None),
                }
            }
        }
    }

    let boundaries = room_boundaries_for_rooms(model, &rooms);
    regions
        .iter()
        .zip(slots)
        .map(|(region, slot)| match region {
            SurfaceRegion::Polygon(points) => Some(points.clone()),
            SurfaceRegion::Room(_) => slot
                .and_then(|index| boundaries[index].as_ref())
                .map(|boundary| boundary.vertices.clone()),
        })
        .collect()
}

fn system_by_id<'a>(model: &'a BuildingModel, id: &ElementId) -> Option<&'a ConstructionSystem> {
    model.systems.iter().find(|system| system.id == *id)
}

fn open_region_diagnostic(element: &ElementId, name: &str, prefix: &str) -> PlanDiagnostic {
    PlanDiagnostic::new(
        DiagnosticSeverity::Warning,
        format!("{prefix}.boundary.open"),
        Some(element.clone()),
        format!("{name} is not enclosed by a closed wall loop, so its joists cannot be laid out."),
    )
}

/// Generate the floor-deck and flat-ceiling joist plans, pushing one plan per
/// authored element. A region whose `Room` loop is open contributes an empty plan
/// carrying a "boundary open" diagnostic and recovers once the loop closes. Every
/// `Room` region is resolved in one batch so the wall-graph faces are derived a
/// single time per project plan.
fn generate_surface_plans(
    plan: &mut ProjectFramePlan,
    model: &BuildingModel,
) -> Result<(), SolverError> {
    // Resolve all deck and ceiling outlines together (decks first, then ceilings)
    // so the wall-graph faces are computed once for the whole project.
    let regions: Vec<&SurfaceRegion> = model
        .floor_decks
        .iter()
        .map(|deck| &deck.region)
        .chain(model.ceilings.iter().map(|ceiling| &ceiling.region))
        .collect();
    let mut outlines = resolve_surface_regions(model, &regions).into_iter();

    for deck in &model.floor_decks {
        let system = system_by_id(model, &deck.system).ok_or_else(|| {
            SolverError::MissingSystemForElement {
                element: deck.id.clone(),
                system: deck.system.clone(),
            }
        })?;
        let floor_plan = match outlines.next().expect("one outline per floor deck") {
            Some(outline) => generate_floor_plan(deck, system, &outline, &model.materials)?,
            None => FloorFramePlan {
                floor: deck.id.clone(),
                members: Vec::new(),
                layers: Vec::new(),
                diagnostics: vec![open_region_diagnostic(&deck.id, &deck.name, "floor")],
            },
        };
        plan.floor_plans.push(floor_plan);
    }

    for ceiling in &model.ceilings {
        let system = system_by_id(model, &ceiling.system).ok_or_else(|| {
            SolverError::MissingSystemForElement {
                element: ceiling.id.clone(),
                system: ceiling.system.clone(),
            }
        })?;
        // The ceiling's low-edge building elevation: it hangs `height` below the
        // level top (`elevation + height`). Used by sloped ceilings to place their
        // joists in true elevation; ignored by flat ceilings. Fails loudly on a
        // dangling level (parallel to the system lookup above), rather than silently
        // framing at a wrong elevation.
        let level = model
            .levels
            .iter()
            .find(|level| level.id == ceiling.level)
            .ok_or_else(|| SolverError::MissingLevelForElement {
                element: ceiling.id.clone(),
                level: ceiling.level.clone(),
            })?;
        let reference_elevation = level.elevation + level.height - ceiling.height;
        let ceiling_plan = match outlines.next().expect("one outline per ceiling") {
            Some(outline) => generate_ceiling_plan(
                ceiling,
                system,
                &outline,
                reference_elevation,
                &model.materials,
            )?,
            None => CeilingFramePlan {
                ceiling: ceiling.id.clone(),
                members: Vec::new(),
                layers: Vec::new(),
                diagnostics: vec![open_region_diagnostic(
                    &ceiling.id,
                    &ceiling.name,
                    "ceiling",
                )],
            },
        };
        plan.ceiling_plans.push(ceiling_plan);
    }

    Ok(())
}

pub fn generate_project_plan(model: &BuildingModel) -> Result<ProjectFramePlan, SolverError> {
    model.validate()?;
    let standards = model.resolved_standards();
    let standards_name = model
        .base_standards_name()
        .unwrap_or("Standards starter pack");

    let mut plan = ProjectFramePlan {
        wall_plans: Vec::with_capacity(model.walls.len()),
        floor_plans: Vec::with_capacity(model.floor_decks.len()),
        ceiling_plans: Vec::with_capacity(model.ceilings.len()),
        roof_plans: Vec::with_capacity(model.roof_planes.len()),
        diagnostics: project_diagnostics(model),
        rooms: Vec::new(),
        layers: Vec::new(),
        fasteners: Vec::new(),
    };

    for wall in &model.walls {
        let system = model
            .system_for(wall)
            .ok_or_else(|| SolverError::MissingSystem {
                wall: wall.id.clone(),
                system: wall.system.clone(),
            })?;
        plan.wall_plans.push(generate_wall_plan_with_site(
            wall,
            system,
            &model.materials,
            &standards,
            standards_name,
            &model.site,
        )?);
    }

    add_join_members(&mut plan, model, &standards)?;
    generate_surface_plans(&mut plan, model)?;
    generate_roof_plans(&mut plan, model)?;
    plan.rooms = room_schedule(model, &mut plan.diagnostics);
    plan.layers = layer_bom_from(
        plan.wall_plans
            .iter()
            .flat_map(|wall_plan| wall_plan.layers.iter())
            .chain(
                plan.floor_plans
                    .iter()
                    .flat_map(|floor| floor.layers.iter()),
            )
            .chain(
                plan.ceiling_plans
                    .iter()
                    .flat_map(|ceiling| ceiling.layers.iter()),
            )
            .chain(plan.roof_plans.iter().flat_map(|roof| roof.layers.iter())),
    );
    let mut fastener_diagnostics = Vec::new();
    plan.fasteners = fastener_takeoff(model, &plan, &standards, &mut fastener_diagnostics);
    plan.diagnostics.append(&mut fastener_diagnostics);

    Ok(plan)
}

/// Derive a takeoff row for each authored room from its bounding wall loop. Rooms
/// that are not enclosed get a zeroed row plus a `Warning` diagnostic.
fn room_schedule(
    model: &BuildingModel,
    diagnostics: &mut Vec<PlanDiagnostic>,
) -> Vec<RoomSchedule> {
    let rooms: Vec<&Room> = model.rooms.iter().collect();
    let boundaries = room_boundaries_for_rooms(model, &rooms);
    let mut schedule = Vec::with_capacity(model.rooms.len());
    for (room, boundary) in model.rooms.iter().zip(boundaries) {
        match boundary {
            Some(boundary) => schedule.push(RoomSchedule {
                room: room.id.clone(),
                name: room.name.clone(),
                usage: room.usage.label().to_owned(),
                closed: true,
                area_square_inches: boundary.area_square_inches().round() as i64,
                perimeter: boundary.perimeter,
            }),
            None => {
                diagnostics.push(PlanDiagnostic::new(
                    DiagnosticSeverity::Warning,
                    "room.boundary.open",
                    Some(room.id.clone()),
                    format!(
                        "{} is not enclosed by a closed wall loop, so its area and perimeter cannot be computed.",
                        room.name
                    ),
                ));
                schedule.push(RoomSchedule {
                    room: room.id.clone(),
                    name: room.name.clone(),
                    usage: room.usage.label().to_owned(),
                    closed: false,
                    area_square_inches: 0,
                    perimeter: Length::ZERO,
                });
            }
        }
    }
    schedule
}

fn project_diagnostics(model: &BuildingModel) -> Vec<PlanDiagnostic> {
    let mut diagnostics = Vec::new();
    if model.walls.len() > 1 {
        diagnostics.push(PlanDiagnostic::new(
            DiagnosticSeverity::Info,
            "solver.scope.multi-wall-shell-alpha",
            None,
            format!(
                "Project framing is generated across {} connected wall segments and {} authored wall joins; floor, roof, shear, and engineered load-path design remain future work.",
                model.walls.len(),
                model.wall_joins.len()
            ),
        ));
    }

    diagnostics
}

fn add_join_members(
    plan: &mut ProjectFramePlan,
    model: &BuildingModel,
    standards: &ResolvedStandards,
) -> Result<(), SolverError> {
    let plate_thickness = standards.defaults.plate_profile.thickness();
    let top_plate_count = if standards.defaults.double_top_plate {
        2
    } else {
        1
    };

    let find_wall = |id: &ElementId| model.walls.iter().find(|candidate| candidate.id == *id);

    // The join stud uses the same framing member and through-wall band as the
    // wall it terminates in, so it renders inside that wall's framing layer.
    let wall_member = |wall: &Wall| -> Result<(BoardProfile, FramingBand), SolverError> {
        let system = model
            .system_for(wall)
            .ok_or_else(|| SolverError::MissingSystem {
                wall: wall.id.clone(),
                system: wall.system.clone(),
            })?;
        Ok((
            system_framing(system)?.member,
            FramingBand::for_system(system)?,
        ))
    };

    for join in &model.wall_joins {
        let first = find_wall(&join.first_wall).ok_or_else(|| SolverError::MissingWallForJoin {
            join: join.id.clone(),
            wall: join.first_wall.clone(),
        })?;
        let second =
            find_wall(&join.second_wall).ok_or_else(|| SolverError::MissingWallForJoin {
                join: join.id.clone(),
                wall: join.second_wall.clone(),
            })?;

        match join.kind {
            WallJoinKind::Corner | WallJoinKind::EndToEnd => {
                for wall in [first, second] {
                    let (member, band) = wall_member(wall)?;
                    push_join_stud(
                        plan,
                        join,
                        wall,
                        member,
                        band,
                        MemberKind::CornerPost,
                        "corner-post",
                        plate_thickness,
                        top_plate_count,
                        RuleProvenance::new(
                            "wall.join.corner-posts",
                            format!(
                                "A corner post is generated on {} with its faces inside the wall edge to make the authored {} join visible in the framing plan.",
                                wall.name, join.name
                            ),
                        ),
                    )?;
                }
            }
            WallJoinKind::Tee => {
                // The partition meets the through wall at the partition's endpoint;
                // the through wall owns the join on its interior. This derives the
                // roles from geometry, so it is correct regardless of which wall the
                // author stored as first/second (validation guarantees exactly one
                // endpoint owner).
                let (partition, through) = if first.has_endpoint(join.point) {
                    (first, second)
                } else {
                    (second, first)
                };
                let (partition_member, partition_band) = wall_member(partition)?;
                push_join_stud(
                    plan,
                    join,
                    partition,
                    partition_member,
                    partition_band,
                    MemberKind::PartitionStud,
                    "partition-stud",
                    plate_thickness,
                    top_plate_count,
                    RuleProvenance::new(
                        "wall.join.tee-partition-stud",
                        format!(
                            "A partition end stud terminates {} where it meets {} at the {} tee join.",
                            partition.name, through.name, join.name
                        ),
                    ),
                )?;
                let (through_member, through_band) = wall_member(through)?;
                push_join_stud(
                    plan,
                    join,
                    through,
                    through_member,
                    through_band,
                    MemberKind::BackingStud,
                    "backing-stud",
                    plate_thickness,
                    top_plate_count,
                    RuleProvenance::new(
                        "wall.join.tee-backing",
                        format!(
                            "A backing stud is added in {} to receive the {} partition and drywall at the {} tee join.",
                            through.name, partition.name, join.name
                        ),
                    ),
                )?;
            }
            WallJoinKind::Cross => {
                for wall in [first, second] {
                    let (member, band) = wall_member(wall)?;
                    push_join_stud(
                        plan,
                        join,
                        wall,
                        member,
                        band,
                        MemberKind::BackingStud,
                        "backing-stud",
                        plate_thickness,
                        top_plate_count,
                        RuleProvenance::new(
                            "wall.join.cross-backing",
                            format!(
                                "A backing stud is added in {} at the {} cross join.",
                                wall.name, join.name
                            ),
                        ),
                    )?;
                }
                plan.diagnostics.push(PlanDiagnostic::new(
                    DiagnosticSeverity::Info,
                    "wall.join.cross-simplified",
                    Some(join.id.clone()),
                    format!(
                        "{} is framed with backing studs on both walls; interrupting one wall for a true cross intersection is not yet modelled.",
                        join.name
                    ),
                ));
            }
        }
    }

    Ok(())
}

/// Push one vertical join stud (corner post / partition end stud / backing stud)
/// onto the given wall's plan, face-aligned at the join point.
#[allow(clippy::too_many_arguments)]
fn push_join_stud(
    plan: &mut ProjectFramePlan,
    join: &WallJoin,
    wall: &Wall,
    wall_stud: BoardProfile,
    band: FramingBand,
    kind: MemberKind,
    member_suffix: &str,
    plate_thickness: Length,
    top_plate_count: usize,
    provenance: RuleProvenance,
) -> Result<(), SolverError> {
    let join_x =
        wall.local_x_for_point(join.point)
            .ok_or_else(|| SolverError::JoinPointOutsideWall {
                join: join.id.clone(),
                wall: wall.id.clone(),
            })?;
    let post_x = face_aligned_center(join_x, wall.length, wall_stud.thickness());
    let stud_top = wall.height - plate_thickness * top_plate_count as i64;
    let stud_length = stud_top - plate_thickness;
    if stud_length <= Length::ZERO {
        return Err(SolverError::WallTooShortForPlateStack {
            wall: wall.id.clone(),
        });
    }

    let wall_plan = plan
        .wall_plan_mut(&wall.id)
        .ok_or_else(|| SolverError::MissingWallPlan {
            wall: wall.id.clone(),
        })?;
    wall_plan.members.push(frame_member(
        format!("{}-{}-{}", join.id.0, wall.id.0, member_suffix),
        &join.id,
        kind,
        wall_stud,
        FrameMemberPlacement::new(
            MemberOrientation::Vertical,
            post_x,
            plate_thickness,
            stud_length,
            wall_stud.thickness(),
        ),
        band,
        provenance,
    ));
    Ok(())
}

fn face_aligned_center(x: Length, length: Length, member_depth: Length) -> Length {
    if length <= member_depth {
        return length / 2;
    }

    let half_depth = member_depth / 2;
    x.max(half_depth).min(length - half_depth)
}

fn wall_uses_header_tables(wall: &Wall, system: &ConstructionSystem) -> bool {
    system.exposure() == WallExposure::Exterior
        || wall
            .tags
            .iter()
            .any(|tag| tag == "bearing" || tag == "load-bearing")
}

struct HeaderSpec {
    profile: BoardProfile,
    depth: Length,
    plies: u8,
    jack_studs: u8,
    rule_id: String,
    summary: String,
    fallback_diagnostic: Option<String>,
}

struct SelectedHeader<'a> {
    table: &'a HeaderSpanTable,
    row: &'a HeaderRow,
}

fn header_spec_for_opening(
    standards: &ResolvedStandards,
    site: &SiteContext,
    opening: &Opening,
    defaults: &FramingDefaults,
    use_header_tables: bool,
) -> HeaderSpec {
    if !use_header_tables {
        return fallback_header_spec(defaults, None);
    }

    if let Some(selection) = select_header_row(standards, site, opening.width) {
        let row = selection.row;
        let plies = row.plies.max(1);
        let jack_studs = row.jack_studs.max(1);
        return HeaderSpec {
            profile: row.profile,
            depth: row.profile.nominal_depth(),
            plies,
            jack_studs,
            rule_id: selection.table.rule.clone(),
            summary: format!(
                "{}-ply {} <= {} span - {}",
                plies,
                row.profile.label(),
                row.max_span,
                selection.table.citation
            ),
            fallback_diagnostic: None,
        };
    }

    let citations = standards
        .headers
        .iter()
        .map(|(_, table)| table.citation.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let mut message = format!(
        "No resolved header span table row covers {} rough span {}",
        opening.name, opening.width
    );
    if let Some(snow_load) = site.ground_snow_load_psf {
        let _ = write!(message, " at {snow_load} psf ground snow load");
    } else {
        message.push_str(" with unknown ground snow load using the highest snow band");
    }
    if citations.is_empty() {
        message.push_str("; no header span table is resolved");
    } else {
        let _ = write!(message, " in {citations}");
    }
    let _ = write!(
        message,
        "; falling back to {} at {}.",
        defaults.header_profile.label(),
        defaults.default_header_depth
    );

    fallback_header_spec(defaults, Some(message))
}

fn fallback_header_spec(
    defaults: &FramingDefaults,
    fallback_diagnostic: Option<String>,
) -> HeaderSpec {
    HeaderSpec {
        profile: defaults.header_profile,
        depth: defaults.default_header_depth,
        plies: 1,
        jack_studs: 1,
        rule_id: "opening.header.default-profile".to_owned(),
        summary: format!(
            "Header uses the configured starter profile {} with default depth {}; no span/load lookup is available.",
            defaults.header_profile.label(),
            defaults.default_header_depth
        ),
        fallback_diagnostic,
    }
}

fn select_header_row<'a>(
    standards: &'a ResolvedStandards,
    site: &SiteContext,
    span: Length,
) -> Option<SelectedHeader<'a>> {
    let mut best: Option<SelectedHeader<'a>> = None;

    for (_, table) in &standards.headers {
        let Some(widest_band) = table.rows.iter().map(|row| row.max_building_width).max() else {
            continue;
        };
        let highest_snow_band = site
            .ground_snow_load_psf
            .is_none()
            .then(|| table.rows.iter().map(|row| row.max_ground_snow_psf).max())
            .flatten();

        for row in &table.rows {
            // building width: conservative band
            if row.max_building_width != widest_band {
                continue;
            }
            if let Some(highest_snow_band) = highest_snow_band
                && row.max_ground_snow_psf != highest_snow_band
            {
                continue;
            }
            if let Some(snow_load) = site.ground_snow_load_psf
                && row.max_ground_snow_psf < snow_load
            {
                continue;
            }
            if row.max_span < span {
                continue;
            }
            let is_better = match &best {
                None => true,
                Some(best) => header_row_cmp(row, table, best.row, best.table).is_lt(),
            };
            if is_better {
                best = Some(SelectedHeader { table, row });
            }
        }
    }

    best
}

fn header_row_cmp(
    left: &HeaderRow,
    left_table: &HeaderSpanTable,
    right: &HeaderRow,
    right_table: &HeaderSpanTable,
) -> std::cmp::Ordering {
    left.max_span
        .cmp(&right.max_span)
        .then_with(|| left.plies.cmp(&right.plies))
        .then_with(|| {
            left.profile
                .nominal_depth()
                .cmp(&right.profile.nominal_depth())
        })
        .then_with(|| left.profile.cmp(&right.profile))
        .then_with(|| left_table.rule.cmp(&right_table.rule))
}

#[derive(Clone, Copy)]
struct OpeningMemberContext<'a> {
    defaults: &'a FramingDefaults,
    framing: WallFraming,
    standards: &'a ResolvedStandards,
    site: &'a SiteContext,
    use_header_tables: bool,
    top_plate_count: usize,
}

fn add_opening_members(
    members: &mut Vec<FrameMember>,
    diagnostics: &mut Vec<PlanDiagnostic>,
    wall: &Wall,
    opening: &Opening,
    context: OpeningMemberContext<'_>,
) {
    let code = context.defaults;
    let framing = context.framing;
    let wall_stud = framing.member;
    let band = framing.band;
    let plate_thickness = code.plate_profile.thickness();
    let stud_base = plate_thickness;
    let stud_top = wall.height - plate_thickness * context.top_plate_count as i64;
    let header_bottom = opening.top();
    let header_spec = header_spec_for_opening(
        context.standards,
        context.site,
        opening,
        code,
        context.use_header_tables,
    );
    let header_depth = header_spec.depth.min(stud_top - header_bottom);
    let header_top = header_bottom + header_depth;
    let left = opening.left();
    let right = opening.right();
    let stud_thickness = wall_stud.thickness();
    let side_positions =
        OpeningSidePositions::new(left, right, stud_thickness, header_spec.jack_studs);

    if let Some(message) = header_spec.fallback_diagnostic.as_ref() {
        diagnostics.push(PlanDiagnostic::new(
            DiagnosticSeverity::Unsupported,
            "standards.header.out-of-domain",
            Some(opening.id.clone()),
            message.clone(),
        ));
    }

    if header_depth < header_spec.depth {
        diagnostics.push(PlanDiagnostic::new(
            DiagnosticSeverity::Warning,
            "opening.header.depth-clipped",
            Some(opening.id.clone()),
            format!(
                "Header depth for {} is clipped to {} because the opening top is close to the top plates.",
                opening.name, header_depth
            ),
        ));
    }

    for side in [OpeningSide::Left, OpeningSide::Right] {
        members.push(frame_member(
            format!("{}-king-{}", opening.id.0, side.label()),
            &opening.id,
            MemberKind::KingStud,
            wall_stud,
            FrameMemberPlacement::new(
                MemberOrientation::Vertical,
                side_positions.king_x(side),
                stud_base,
                stud_top - stud_base,
                stud_thickness,
            ),
            band,
            RuleProvenance::new(
                "opening.king-studs.each-side",
                format!(
                    "A king stud is generated at the {} rough opening edge for {}.",
                    side.label(),
                    opening.name
                ),
            ),
        ));

        for jack_index in 0..header_spec.jack_studs {
            members.push(frame_member(
                jack_member_id(opening, side, jack_index),
                &opening.id,
                MemberKind::JackStud,
                wall_stud,
                FrameMemberPlacement::new(
                    MemberOrientation::Vertical,
                    side_positions.jack_x(side, jack_index),
                    stud_base,
                    header_bottom - stud_base,
                    stud_thickness,
                ),
                band,
                RuleProvenance::new(
                    "opening.jack-studs.header-bearing",
                    format!(
                        "{} jack stud(s) are generated at the {} rough opening edge to support the selected header.",
                        header_spec.jack_studs,
                        side.label()
                    ),
                ),
            ));
        }
    }

    for ply_index in 0..header_spec.plies {
        members.push(frame_member(
            header_member_id(opening, ply_index),
            &opening.id,
            MemberKind::Header,
            header_spec.profile,
            FrameMemberPlacement::new(
                MemberOrientation::Horizontal,
                side_positions.left_jack_left_face,
                header_bottom,
                opening.width + stud_thickness * i64::from(header_spec.jack_studs) * 2,
                header_depth,
            ),
            band,
            RuleProvenance::new(header_spec.rule_id.clone(), header_spec.summary.clone()),
        ));
    }

    if opening.has_sill() {
        members.push(frame_member(
            format!("{}-sill", opening.id.0),
            &opening.id,
            MemberKind::RoughSill,
            wall_stud,
            FrameMemberPlacement::new(
                MemberOrientation::Horizontal,
                left,
                opening.sill_height,
                opening.width,
                stud_thickness,
            ),
            band,
            RuleProvenance::new(
                "opening.window.rough-sill",
                format!(
                    "A rough sill is generated for {} because this opening type has a sill height.",
                    opening.name
                ),
            ),
        ));

        add_cripples(
            members,
            framing,
            opening,
            "lower",
            plate_thickness,
            opening.sill_height,
        );
    }

    add_cripples(members, framing, opening, "upper", header_top, stud_top);
}

#[derive(Clone, Copy)]
enum OpeningSide {
    Left,
    Right,
}

impl OpeningSide {
    const fn label(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
        }
    }
}

struct OpeningSidePositions {
    left_king: Length,
    left_jack_left_face: Length,
    right_king: Length,
    opening_left: Length,
    opening_right: Length,
    stud_thickness: Length,
}

impl OpeningSidePositions {
    fn new(
        opening_left: Length,
        opening_right: Length,
        stud_thickness: Length,
        jack_studs: u8,
    ) -> Self {
        let half_stud = stud_thickness / 2;
        let jack_pack = stud_thickness * i64::from(jack_studs);
        Self {
            left_king: opening_left - jack_pack - half_stud,
            left_jack_left_face: opening_left - jack_pack,
            right_king: opening_right + jack_pack + half_stud,
            opening_left,
            opening_right,
            stud_thickness,
        }
    }

    fn king_x(&self, side: OpeningSide) -> Length {
        match side {
            OpeningSide::Left => self.left_king,
            OpeningSide::Right => self.right_king,
        }
    }

    fn jack_x(&self, side: OpeningSide, jack_index: u8) -> Length {
        let half_stud = self.stud_thickness / 2;
        let offset = self.stud_thickness * i64::from(jack_index);
        match side {
            OpeningSide::Left => self.opening_left - half_stud - offset,
            OpeningSide::Right => self.opening_right + half_stud + offset,
        }
    }
}

fn jack_member_id(opening: &Opening, side: OpeningSide, jack_index: u8) -> String {
    if jack_index == 0 {
        format!("{}-jack-{}", opening.id.0, side.label())
    } else {
        format!("{}-jack-{}-{}", opening.id.0, side.label(), jack_index + 1)
    }
}

fn header_member_id(opening: &Opening, ply_index: u8) -> String {
    if ply_index == 0 {
        format!("{}-header", opening.id.0)
    } else {
        format!("{}-header-{}", opening.id.0, ply_index + 1)
    }
}

fn add_cripples(
    members: &mut Vec<FrameMember>,
    framing: WallFraming,
    opening: &Opening,
    label: &str,
    bottom: Length,
    top: Length,
) {
    let cut_length = top - bottom;
    if cut_length <= Length::ZERO {
        return;
    }

    let stud_profile = framing.member;
    // Cripples honor the construction system's on-center spacing, the same
    // layout the common studs use, rather than the code-profile default.
    for x in cripple_positions(opening.left(), opening.right(), framing.spacing) {
        members.push(frame_member(
            format!("{}-cripple-{}-{}", opening.id.0, label, x.ticks()),
            &opening.id,
            MemberKind::CrippleStud,
            stud_profile,
            FrameMemberPlacement::new(
                MemberOrientation::Vertical,
                x,
                bottom,
                cut_length,
                stud_profile.thickness(),
            ),
            framing.band,
            RuleProvenance::new(
                "opening.cripples.on-center",
                format!(
                    "{} cripple studs are generated across {} at the system {} spacing where clear span allows.",
                    title_case(label),
                    opening.name,
                    framing.spacing
                ),
            ),
        ));
    }
}

struct FrameMemberPlacement {
    orientation: MemberOrientation,
    x: Length,
    elevation: Length,
    cut_length: Length,
    cross_section_depth: Length,
    sloped: Option<SlopedPlacement>,
}

impl FrameMemberPlacement {
    fn new(
        orientation: MemberOrientation,
        x: Length,
        elevation: Length,
        cut_length: Length,
        cross_section_depth: Length,
    ) -> Self {
        Self {
            orientation,
            x,
            elevation,
            cut_length,
            cross_section_depth,
            sloped: None,
        }
    }

    /// Attach a sloped placement (the eave/ridge elevations) to a roof-plane
    /// member; `cut_length` is the member's true sloped length.
    fn with_slope(mut self, sloped: SlopedPlacement) -> Self {
        self.sloped = Some(sloped);
        self
    }
}

fn frame_member(
    id: impl Into<String>,
    source: &ElementId,
    kind: MemberKind,
    profile: BoardProfile,
    placement: FrameMemberPlacement,
    band: FramingBand,
    provenance: RuleProvenance,
) -> FrameMember {
    FrameMember {
        id: id.into(),
        source: source.clone(),
        kind,
        profile,
        orientation: placement.orientation,
        x: placement.x,
        elevation: placement.elevation,
        cut_length: placement.cut_length,
        cross_section_depth: placement.cross_section_depth,
        side_offset: band.offset,
        side_depth: band.depth,
        sloped: placement.sloped,
        provenance,
    }
}

fn starter_profile_diagnostics(wall: &Wall, standards_name: &str) -> Vec<PlanDiagnostic> {
    let mut diagnostics = vec![
        PlanDiagnostic::new(
            DiagnosticSeverity::Warning,
            "code-profile.starter-only",
            Some(wall.id.clone()),
            format!(
                "{} is starter rule data for deterministic framing defaults, not complete IRC compliance.",
                standards_name
            ),
        ),
        PlanDiagnostic::new(
            DiagnosticSeverity::Info,
            "solver.scope.wall-segment-alpha",
            Some(wall.id.clone()),
            "This wall segment is framed with deterministic starter rules. Project generation aggregates connected wall segments and authored joins; engineered load paths, hold-downs, floors, and roofs remain future work.",
        ),
    ];

    for opening in &wall.openings {
        if matches!(opening.kind, framer_core::OpeningKind::GarageDoor) {
            diagnostics.push(PlanDiagnostic::new(
                DiagnosticSeverity::Unsupported,
                "opening.garage-door.starter-header",
                Some(opening.id.clone()),
                format!(
                    "{} is modeled as a wide rough opening with starter-rule king, jack, and header members; garage-door-specific structural design is unsupported.",
                    opening.name
                ),
            ));
        }
    }

    diagnostics
}

fn stud_positions(length: Length, spacing: Length, stud_thickness: Length) -> Vec<Length> {
    if length <= stud_thickness {
        return vec![length / 2];
    }

    let mut positions = Vec::new();
    let first_center = stud_thickness / 2;
    let last_center = length - first_center;

    positions.push(first_center);

    let mut x = spacing;
    while x < last_center {
        positions.push(x);
        x += spacing;
    }

    if positions.last().copied() != Some(last_center) {
        positions.push(last_center);
    }
    positions
}

fn cripple_positions(left: Length, right: Length, spacing: Length) -> Vec<Length> {
    let mut positions = Vec::new();
    let mut x = left + spacing;
    while x < right {
        positions.push(x);
        x += spacing;
    }
    positions
}

fn is_inside_opening_framing_assembly(
    x: Length,
    openings: &[Opening],
    stud_thickness: Length,
) -> bool {
    openings.iter().any(|opening| {
        x >= opening.left() - stud_thickness * 2 && x <= opening.right() + stud_thickness * 2
    })
}

pub fn export_bom_csv(bom: &[BomItem], fasteners: &[FastenerTakeoff]) -> String {
    let mut items = bom.to_vec();
    items.sort_by_key(|item| (item.profile, item.kind, item.cut_length));

    let mut csv =
        "quantity,profile,kind,cut_length_inches,cut_length_display,total_length_inches,total_length_display\n"
            .to_owned();
    for item in items {
        let fields = [
            item.quantity.to_string(),
            item.profile.label().to_owned(),
            item.kind.label().to_owned(),
            decimal_inches(item.cut_length),
            item.cut_length.to_string(),
            decimal_inches(item.total_length),
            item.total_length.to_string(),
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

    if !fasteners.is_empty() {
        let mut fastener_rows = fasteners.to_vec();
        fastener_rows.sort_by(|a, b| (&a.fastener, a.connection).cmp(&(&b.fastener, b.connection)));

        csv.push('\n');
        csv.push_str("fastener_quantity,fastener,connection,rule,citation\n");
        for item in fastener_rows {
            let fields = [
                item.quantity.to_string(),
                item.fastener,
                connection_kind_label(item.connection).to_owned(),
                item.rule,
                item.citation,
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
    }
    csv
}

/// Export the per-layer material takeoff (area goods and volumetric goods) as its
/// own CSV, kept separate from the lumber `export_bom_csv` so each section stays a
/// clean, single-shape table. Square-foot and board-foot columns are rendered for
/// readability while the canonical integer square-inch / cubic-inch values remain.
pub fn export_layer_bom_csv(layers: &[LayerBomItem]) -> String {
    let mut items = layers.to_vec();
    items.sort_by(|a, b| {
        (&a.material, a.function, a.thickness).cmp(&(&b.material, b.function, b.thickness))
    });

    let mut csv =
        "material,name,function,thickness_inches,thickness_display,area_sq_in,area_sq_ft,volume_cu_in,volume_bd_ft\n"
            .to_owned();
    for item in items {
        let fields = [
            item.material.0.clone(),
            item.material_name.clone(),
            item.function.label().to_owned(),
            decimal_inches(item.thickness),
            item.thickness.to_string(),
            item.area_sq_in.to_string(),
            format!("{:.2}", item.area_sq_in as f64 / 144.0),
            item.volume_bd_in.to_string(),
            format!("{:.2}", item.volume_bd_in as f64 / 144.0),
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

pub fn export_room_schedule_csv(rooms: &[RoomSchedule]) -> String {
    let mut rows = rooms.to_vec();
    rows.sort_by(|a, b| a.room.0.cmp(&b.room.0));

    let mut csv = "room,name,usage,enclosed,area_sqft,perimeter_ft\n".to_owned();
    for room in rows {
        let fields = [
            room.room.0.clone(),
            room.name.clone(),
            room.usage.clone(),
            room.closed.to_string(),
            format!("{:.1}", room.area_square_feet()),
            format!("{:.1}", room.perimeter.feet()),
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

fn connection_kind_label(connection: ConnectionKind) -> &'static str {
    match connection {
        ConnectionKind::StudToPlateEnd => "stud to plate end",
        ConnectionKind::StudToPlateToe => "stud to plate toe",
        ConnectionKind::TopPlateLap => "top plate lap",
        ConnectionKind::DoubleTopPlate => "double top plate",
        ConnectionKind::SolePlateToJoist => "sole plate to joist",
        ConnectionKind::HeaderToKingStud => "header to king stud",
        ConnectionKind::SheathingEdge => "sheathing edge",
        ConnectionKind::SheathingField => "sheathing field",
    }
}

pub fn export_wall_elevation_svg(wall: &Wall, plan: &WallFramePlan) -> String {
    let width = wall.length.inches().max(1.0);
    let height = wall.height.inches().max(1.0);
    let margin = 12.0;
    let mut svg = String::new();

    writeln!(svg, r#"<?xml version="1.0" encoding="UTF-8"?>"#).unwrap();
    writeln!(
        svg,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="{:.4} {:.4} {:.4} {:.4}" width="{:.4}in" height="{:.4}in">"#,
        -margin,
        -margin,
        width + margin * 2.0,
        height + margin * 2.0,
        (width + margin * 2.0) / 12.0,
        (height + margin * 2.0) / 12.0
    )
    .unwrap();
    writeln!(
        svg,
        "  <title>{}</title>",
        escape_xml(&format!("{} framing elevation", wall.name))
    )
    .unwrap();
    writeln!(
        svg,
        "  <desc>Starter framing elevation generated from authored Framer intent. Not a permit drawing.</desc>"
    )
    .unwrap();
    writeln!(
        svg,
        r##"  <rect x="0" y="0" width="{:.4}" height="{:.4}" fill="#faf8f2" stroke="#bdb7aa" stroke-width="0.25"/>"##,
        width, height
    )
    .unwrap();

    for opening in &wall.openings {
        writeln!(
            svg,
            r##"  <rect data-opening="{}" x="{}" y="{}" width="{}" height="{}" fill="#ffffff" fill-opacity="0.28" stroke="#896634" stroke-width="0.25" stroke-dasharray="2 1">"##,
            escape_xml(&opening.id.0),
            svg_number(opening.left().inches()),
            svg_number(height - opening.top().inches()),
            svg_number(opening.width.inches()),
            svg_number(opening.height.inches())
        )
        .unwrap();
        writeln!(
            svg,
            "    <title>{} {}</title>",
            escape_xml(&format!("{:?}", opening.kind)),
            escape_xml(&opening.name)
        )
        .unwrap();
        writeln!(svg, "  </rect>").unwrap();
    }

    for member in &plan.members {
        let (x, y, member_width, member_height) = match member.orientation {
            MemberOrientation::Horizontal => (
                member.x.inches(),
                height - member.elevation.inches() - member.cross_section_depth.inches(),
                member.cut_length.inches(),
                member.cross_section_depth.inches(),
            ),
            MemberOrientation::Vertical => (
                member.x.inches() - member.cross_section_depth.inches() / 2.0,
                height - member.elevation.inches() - member.cut_length.inches(),
                member.cross_section_depth.inches(),
                member.cut_length.inches(),
            ),
        };

        writeln!(
            svg,
            r##"  <rect id="{}" x="{}" y="{}" width="{}" height="{}" fill="{}" stroke="#574634" stroke-width="0.18">"##,
            escape_xml(&member.id),
            svg_number(x),
            svg_number(y),
            svg_number(member_width),
            svg_number(member_height),
            member_svg_color(member.kind)
        )
        .unwrap();
        writeln!(
            svg,
            "    <title>{}: {} {}</title>",
            escape_xml(&member.id),
            escape_xml(member.profile.label()),
            escape_xml(member.kind.label())
        )
        .unwrap();
        writeln!(
            svg,
            "    <desc>{}</desc>",
            escape_xml(&member.provenance.summary)
        )
        .unwrap();
        writeln!(svg, "  </rect>").unwrap();
    }

    writeln!(
        svg,
        r##"  <text x="0" y="{:.4}" font-family="Arial, sans-serif" font-size="4" fill="#46433d">{} x {}</text>"##,
        height + 7.0,
        escape_xml(&wall.length.to_string()),
        escape_xml(&wall.height.to_string())
    )
    .unwrap();
    writeln!(svg, "</svg>").unwrap();
    svg
}

pub fn export_project_svg(model: &BuildingModel, plan: &ProjectFramePlan) -> String {
    let bounds = project_bounds(model);
    let width = (bounds.max.x - bounds.min.x).inches().max(1.0);
    let depth = (bounds.max.y - bounds.min.y).inches().max(1.0);
    let max_wall_height = model
        .walls
        .iter()
        .map(|wall| wall.height.inches())
        .fold(96.0, f64::max);
    let margin = 18.0;
    let plan_height = depth + margin * 2.0;
    let elevation_height = (max_wall_height + 26.0) * model.walls.len().max(1) as f64;
    let canvas_width = width.max(220.0) + margin * 2.0;
    let canvas_height = plan_height + elevation_height + margin;
    let mut svg = String::new();

    writeln!(svg, r#"<?xml version="1.0" encoding="UTF-8"?>"#).unwrap();
    writeln!(
        svg,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {} {}" width="{}in" height="{}in">"#,
        svg_number(canvas_width),
        svg_number(canvas_height),
        svg_number(canvas_width / 12.0),
        svg_number(canvas_height / 12.0)
    )
    .unwrap();
    writeln!(svg, "  <title>Framer project shell plan</title>").unwrap();
    writeln!(
        svg,
        "  <desc>Whole-project alpha export generated from authored Framer intent. Not a permit drawing.</desc>"
    )
    .unwrap();
    writeln!(
        svg,
        r##"  <rect x="0" y="0" width="{}" height="{}" fill="#f7f5ee"/>"##,
        svg_number(canvas_width),
        svg_number(canvas_height)
    )
    .unwrap();

    writeln!(
        svg,
        r##"  <text x="{}" y="12" font-family="Arial, sans-serif" font-size="5" fill="#46433d">Plan</text>"##,
        svg_number(margin)
    )
    .unwrap();

    for join in &model.wall_joins {
        let point = project_svg_point(join.point, bounds.min, margin, plan_height - margin);
        writeln!(
            svg,
            r##"  <circle data-join="{}" cx="{}" cy="{}" r="2.5" fill="#2f5f7f">"##,
            escape_xml(&join.id.0),
            svg_number(point.0),
            svg_number(point.1)
        )
        .unwrap();
        writeln!(svg, "    <title>{}</title>", escape_xml(&join.name)).unwrap();
        writeln!(svg, "  </circle>").unwrap();
    }

    for wall in &model.walls {
        let start = project_svg_point(wall.start, bounds.min, margin, plan_height - margin);
        let end = project_svg_point(wall.end, bounds.min, margin, plan_height - margin);
        writeln!(
            svg,
            r##"  <line data-wall="{}" x1="{}" y1="{}" x2="{}" y2="{}" stroke="#6f5b3f" stroke-width="3" stroke-linecap="round">"##,
            escape_xml(&wall.id.0),
            svg_number(start.0),
            svg_number(start.1),
            svg_number(end.0),
            svg_number(end.1)
        )
        .unwrap();
        writeln!(svg, "    <title>{}</title>", escape_xml(&wall.name)).unwrap();
        writeln!(svg, "  </line>").unwrap();

        for opening in &wall.openings {
            let left = wall.point_at_local_x(opening.left());
            let right = wall.point_at_local_x(opening.right());
            let left = project_svg_point(left, bounds.min, margin, plan_height - margin);
            let right = project_svg_point(right, bounds.min, margin, plan_height - margin);
            writeln!(
                svg,
                r##"  <line data-opening="{}" x1="{}" y1="{}" x2="{}" y2="{}" stroke="#f7f5ee" stroke-width="5" stroke-linecap="butt"/>"##,
                escape_xml(&opening.id.0),
                svg_number(left.0),
                svg_number(left.1),
                svg_number(right.0),
                svg_number(right.1)
            )
            .unwrap();
            writeln!(
                svg,
                r##"  <line data-opening-edge="{}" x1="{}" y1="{}" x2="{}" y2="{}" stroke="#896634" stroke-width="1.25" stroke-linecap="butt"/>"##,
                escape_xml(&opening.id.0),
                svg_number(left.0),
                svg_number(left.1),
                svg_number(right.0),
                svg_number(right.1)
            )
            .unwrap();
        }
    }

    for room in &model.rooms {
        let Some(schedule) = plan.rooms.iter().find(|entry| entry.room == room.id) else {
            continue;
        };
        let anchor = project_svg_point(room.seed, bounds.min, margin, plan_height - margin);
        let label = if schedule.closed {
            format!("{} ({:.0} sq ft)", room.name, schedule.area_square_feet())
        } else {
            format!("{} (open)", room.name)
        };
        writeln!(
            svg,
            r##"  <text data-room="{}" x="{}" y="{}" font-family="Arial, sans-serif" font-size="4" fill="#46433d" text-anchor="middle">{}</text>"##,
            escape_xml(&room.id.0),
            svg_number(anchor.0),
            svg_number(anchor.1),
            escape_xml(&label)
        )
        .unwrap();
    }

    let mut elevation_y = plan_height + margin;
    for wall in &model.walls {
        if let Some(wall_plan) = plan.wall_plan(&wall.id) {
            writeln!(
                svg,
                r##"  <g data-wall-elevation="{}" transform="translate({}, {})">"##,
                escape_xml(&wall.id.0),
                svg_number(margin),
                svg_number(elevation_y)
            )
            .unwrap();
            writeln!(
                svg,
                r##"    <text x="0" y="-5" font-family="Arial, sans-serif" font-size="4.5" fill="#46433d">{}</text>"##,
                escape_xml(&wall.name)
            )
            .unwrap();
            write_wall_elevation_contents(&mut svg, wall, wall_plan, 0.45, "    ");
            writeln!(svg, "  </g>").unwrap();
            elevation_y += max_wall_height + 26.0;
        }
    }

    writeln!(svg, "</svg>").unwrap();
    svg
}

fn write_wall_elevation_contents(
    svg: &mut String,
    wall: &Wall,
    plan: &WallFramePlan,
    scale: f64,
    indent: &str,
) {
    let width = wall.length.inches().max(1.0) * scale;
    let height = wall.height.inches().max(1.0) * scale;
    writeln!(
        svg,
        r##"{indent}<rect x="0" y="0" width="{}" height="{}" fill="#faf8f2" stroke="#bdb7aa" stroke-width="0.25"/>"##,
        svg_number(width),
        svg_number(height)
    )
    .unwrap();

    for member in &plan.members {
        let (x, y, member_width, member_height) = match member.orientation {
            MemberOrientation::Horizontal => (
                member.x.inches() * scale,
                height
                    - member.elevation.inches() * scale
                    - member.cross_section_depth.inches() * scale,
                member.cut_length.inches() * scale,
                member.cross_section_depth.inches() * scale,
            ),
            MemberOrientation::Vertical => (
                member.x.inches() * scale - member.cross_section_depth.inches() * scale / 2.0,
                height - member.elevation.inches() * scale - member.cut_length.inches() * scale,
                member.cross_section_depth.inches() * scale,
                member.cut_length.inches() * scale,
            ),
        };
        writeln!(
            svg,
            r##"{indent}<rect data-member="{}" x="{}" y="{}" width="{}" height="{}" fill="{}" stroke="#574634" stroke-width="0.16"/>"##,
            escape_xml(&member.id),
            svg_number(x),
            svg_number(y),
            svg_number(member_width.max(0.75)),
            svg_number(member_height.max(0.75)),
            member_svg_color(member.kind)
        )
        .unwrap();
    }
}

fn member_svg_color(kind: MemberKind) -> &'static str {
    match kind {
        MemberKind::BottomPlate | MemberKind::TopPlate => "#635543",
        MemberKind::CornerPost => "#345f7f",
        MemberKind::PartitionStud => "#4f7f5f",
        MemberKind::BackingStud => "#7f6f4f",
        MemberKind::CommonStud => "#ba915e",
        MemberKind::KingStud => "#97643d",
        MemberKind::JackStud => "#d3a85f",
        MemberKind::Header => "#738263",
        MemberKind::RoughSill => "#5c7990",
        MemberKind::CrippleStud => "#dabe8b",
        MemberKind::FloorJoist => "#9c7b4f",
        MemberKind::CeilingJoist => "#7f9c8f",
        MemberKind::RimJoist => "#6f5535",
        MemberKind::Blocking => "#b59a6a",
        MemberKind::Rafter => "#8a6f4a",
        MemberKind::RidgeBoard => "#5d4a32",
        MemberKind::HipRafter => "#7f6848",
        MemberKind::ValleyRafter => "#725f7f",
        MemberKind::JackRafter => "#a8844f",
    }
}

struct ProjectBounds {
    min: Point2,
    max: Point2,
}

fn project_bounds(model: &BuildingModel) -> ProjectBounds {
    let mut min_x = Length::ZERO;
    let mut min_y = Length::ZERO;
    let mut max_x = Length::ZERO;
    let mut max_y = Length::ZERO;
    let mut initialized = false;

    for point in model
        .walls
        .iter()
        .flat_map(|wall| [wall.start, wall.end])
        .chain(model.wall_joins.iter().map(|join| join.point))
    {
        if !initialized {
            min_x = point.x;
            min_y = point.y;
            max_x = point.x;
            max_y = point.y;
            initialized = true;
        } else {
            min_x = min_x.min(point.x);
            min_y = min_y.min(point.y);
            max_x = max_x.max(point.x);
            max_y = max_y.max(point.y);
        }
    }

    ProjectBounds {
        min: Point2::new(min_x, min_y),
        max: Point2::new(max_x, max_y),
    }
}

fn project_svg_point(point: Point2, min: Point2, margin: f64, baseline: f64) -> (f64, f64) {
    (
        margin + (point.x - min.x).inches(),
        baseline - (point.y - min.y).inches(),
    )
}

fn decimal_inches(length: Length) -> String {
    format!("{:.4}", length.inches())
}

fn svg_number(value: f64) -> String {
    let rounded = format!("{value:.4}");
    rounded
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_owned()
}

fn csv_field(value: &str) -> String {
    if value.contains([',', '"', '\n']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    }
}

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn title_case(value: &str) -> String {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    first.to_ascii_uppercase().to_string() + chars.as_str()
}

#[derive(Debug, Error)]
pub enum SolverError {
    #[error(transparent)]
    Model(#[from] ModelError),
    #[error("wall {wall:?} was not found while generating join {join:?}")]
    MissingWallForJoin { join: ElementId, wall: ElementId },
    #[error("wall {wall:?} has no generated plan")]
    MissingWallPlan { wall: ElementId },
    #[error("join {join:?} point is outside wall {wall:?}")]
    JoinPointOutsideWall { join: ElementId, wall: ElementId },
    #[error("wall {wall:?} is too short for its configured plate stack")]
    WallTooShortForPlateStack { wall: ElementId },
    #[error("opening {opening:?} in wall {wall:?} leaves no header space below the top plates")]
    OpeningLeavesNoHeaderSpace { wall: ElementId, opening: ElementId },
    #[error(
        "opening {opening:?} in wall {wall:?} is too close to a wall end for starter king/jack framing"
    )]
    OpeningTooCloseToWallEnd { wall: ElementId, opening: ElementId },
    #[error("wall {wall:?} references construction system {system:?} which was not found")]
    MissingSystem { wall: ElementId, system: ElementId },
    #[error("element {element:?} references construction system {system:?} which was not found")]
    MissingSystemForElement {
        element: ElementId,
        system: ElementId,
    },
    #[error("element {element:?} references level {level:?} which was not found")]
    MissingLevelForElement {
        element: ElementId,
        level: ElementId,
    },
    #[error("construction system {system:?} has no framing layer")]
    SystemHasNoFramingLayer { system: ElementId },
}

#[cfg(test)]
mod tests {
    use framer_core::{
        BuildingModel, ElementId, FasteningRow, FasteningSchedule, FramingDefaults, Opening,
        StandardsPack, StandardsTables, Wall, load_project, save_project,
    };

    use super::*;

    const STARTER_STANDARDS_NAME: &str = "IRC 2021 Prescriptive (starter)";

    fn starter_standards() -> ResolvedStandards {
        BuildingModel::new().resolved_standards()
    }

    fn standards_with_header_rows(rows: Vec<HeaderRow>) -> ResolvedStandards {
        let mut standards = starter_standards();
        standards.headers = vec![(
            ElementId::new("std-test"),
            HeaderSpanTable {
                rule: "test.headers".to_owned(),
                citation: "Test Header Table".to_owned(),
                rows,
            },
        )];
        standards
    }

    fn header_row(
        profile: BoardProfile,
        plies: u8,
        max_ground_snow_psf: u32,
        max_building_width: Length,
        max_span: Length,
        jack_studs: u8,
    ) -> HeaderRow {
        HeaderRow {
            profile,
            plies,
            max_ground_snow_psf,
            max_building_width,
            max_span,
            jack_studs,
        }
    }

    fn add_fastening_pack(model: &mut BuildingModel, rule: &str, rows: Vec<FasteningRow>) {
        let defaults = model.framing_defaults();
        let pack_id = ElementId::new(format!("std-{}", rule.replace('.', "-")));
        model.standards.push(pack_id.clone());
        model.standards_packs.push(StandardsPack {
            id: pack_id,
            name: "Test fastening pack".to_owned(),
            edition: "test".to_owned(),
            source: None,
            tables: StandardsTables {
                defaults,
                studs: Vec::new(),
                headers: Vec::new(),
                fastening: vec![FasteningSchedule {
                    rule: rule.to_owned(),
                    citation: "Test Fastening Schedule".to_owned(),
                    rows,
                }],
                bracing: Vec::new(),
            },
            checks: Vec::new(),
            overlays: Vec::new(),
            tags: Vec::new(),
            properties: std::collections::BTreeMap::new(),
        });
    }

    fn fastener_row<'a>(
        plan: &'a ProjectFramePlan,
        fastener: &str,
        connection: ConnectionKind,
    ) -> &'a FastenerTakeoff {
        plan.fasteners
            .iter()
            .find(|row| row.fastener == fastener && row.connection == connection)
            .unwrap_or_else(|| panic!("expected {fastener:?} takeoff for {connection:?}"))
    }

    /// A closed 12ft × 8ft rectangle with one room seeded at its centre.
    fn rectangle_with_room() -> BuildingModel {
        use framer_core::{Point2, Room, RoomUsage};
        let code = FramingDefaults::irc_2021_starter();
        let mut model = BuildingModel::new();
        let (w, h, z) = (
            Length::from_feet(12.0),
            Length::from_feet(8.0),
            Length::ZERO,
        );
        let mut wall = |id: &str, a: Point2, b: Point2| {
            model.walls.push(
                Wall::new(id, id, Length::from_feet(1.0), &code).with_placement("level-1", a, b),
            );
        };
        wall("w-b", Point2::new(z, z), Point2::new(w, z));
        wall("w-r", Point2::new(w, z), Point2::new(w, h));
        wall("w-t", Point2::new(w, h), Point2::new(z, h));
        wall("w-l", Point2::new(z, h), Point2::new(z, z));
        model.rooms.push(Room::new(
            "room-1",
            "Living",
            RoomUsage::Living,
            "level-1",
            Point2::new(Length::from_feet(6.0), Length::from_feet(4.0)),
        ));
        model
    }

    fn placed(id: &str, a: Point2, b: Point2, code: &FramingDefaults) -> Wall {
        Wall::new(id, id, Length::from_feet(1.0), code).with_placement("level-1", a, b)
    }

    /// The seeded exterior wall system (2x4 framing @ 16" o.c.) — the default a
    /// freshly built `Wall` references. Drives `generate_wall_plan` in tests.
    fn wall_system() -> ConstructionSystem {
        BuildingModel::starter_library()
            .1
            .into_iter()
            .find(|system| system.id == ElementId::new("system-wall-exterior-1"))
            .expect("seeded exterior wall system")
    }

    /// The seeded material library, used to resolve layer-BOM material names.
    fn materials() -> Vec<Material> {
        BuildingModel::starter_library().0
    }

    /// A minimal single-framing-layer wall system using `member` at 16" o.c.
    fn framing_system(id: &str, member: BoardProfile) -> ConstructionSystem {
        ConstructionSystem {
            id: ElementId::new(id),
            name: id.to_owned(),
            kind: framer_core::SystemKind::Wall,
            source: None,
            layers: vec![
                framer_core::ConstructionLayer::new(
                    framer_core::LayerFunction::Framing,
                    "mat-spf",
                    member.nominal_depth(),
                )
                .with_framing(FramingSpec {
                    member,
                    spacing: Length::from_whole_inches(16),
                    pattern: framer_core::FramingPattern::Single,
                    member_family: framer_core::MemberFamily::Stud,
                    cavity_material: None,
                }),
            ],
        }
    }

    #[test]
    fn two_bedroom_example_frames_three_rooms_via_tee_joins() {
        let plan = generate_project_plan(&BuildingModel::demo_two_bedroom()).unwrap();

        // Three enclosed rooms with the expected areas (96 + 96 + 192 sq ft).
        assert_eq!(plan.rooms.len(), 3);
        assert!(plan.rooms.iter().all(|room| room.closed));
        let mut areas: Vec<i64> = plan
            .rooms
            .iter()
            .map(|room| room.area_square_feet().round() as i64)
            .collect();
        areas.sort_unstable();
        assert_eq!(areas, vec![96, 96, 192]);

        // Interior partitions are framed, not diagnosed as unsupported.
        assert!(
            plan.diagnostics
                .iter()
                .all(|d| d.code != "wall.join.unsupported-kind")
        );
        assert!(plan.wall_plans.iter().any(|wall_plan| {
            wall_plan
                .members
                .iter()
                .any(|m| m.kind == MemberKind::PartitionStud)
        }));
    }

    #[test]
    fn tee_join_frames_partition_end_stud_and_backing_no_corner_post() {
        use framer_core::{WallJoin, WallJoinKind};
        let code = FramingDefaults::irc_2021_starter();
        let mut model = BuildingModel::new();
        model.walls.push(placed(
            "through",
            Point2::new(Length::ZERO, Length::ZERO),
            Point2::new(Length::from_feet(20.0), Length::ZERO),
            &code,
        ));
        model.walls.push(placed(
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

        let plan = generate_project_plan(&model).unwrap();
        let through = plan.wall_plan(&ElementId::new("through")).unwrap();
        let partition = plan.wall_plan(&ElementId::new("partition")).unwrap();

        assert!(
            through
                .members
                .iter()
                .any(|m| m.kind == MemberKind::BackingStud)
        );
        assert!(
            partition
                .members
                .iter()
                .any(|m| m.kind == MemberKind::PartitionStud)
        );
        assert!(
            through
                .members
                .iter()
                .chain(&partition.members)
                .all(|m| m.kind != MemberKind::CornerPost),
            "a Tee must not generate corner posts"
        );
        assert!(
            plan.diagnostics
                .iter()
                .all(|d| d.code != "wall.join.unsupported-kind")
        );
    }

    #[test]
    fn cross_join_frames_backing_on_both_walls() {
        use framer_core::{WallJoin, WallJoinKind};
        let code = FramingDefaults::irc_2021_starter();
        let mut model = BuildingModel::new();
        model.walls.push(placed(
            "horizontal",
            Point2::new(Length::ZERO, Length::from_feet(4.0)),
            Point2::new(Length::from_feet(20.0), Length::from_feet(4.0)),
            &code,
        ));
        model.walls.push(placed(
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

        let plan = generate_project_plan(&model).unwrap();
        let horizontal = plan.wall_plan(&ElementId::new("horizontal")).unwrap();
        let vertical = plan.wall_plan(&ElementId::new("vertical")).unwrap();

        assert!(
            horizontal
                .members
                .iter()
                .any(|m| m.kind == MemberKind::BackingStud)
        );
        assert!(
            vertical
                .members
                .iter()
                .any(|m| m.kind == MemberKind::BackingStud)
        );
        assert!(
            plan.diagnostics
                .iter()
                .all(|d| d.code != "wall.join.unsupported-kind")
        );
    }

    #[test]
    fn room_schedule_reports_area_for_enclosed_room() {
        let plan = generate_project_plan(&rectangle_with_room()).unwrap();

        assert_eq!(plan.rooms.len(), 1);
        let room = &plan.rooms[0];
        assert!(room.closed);
        assert!((room.area_square_feet() - 96.0).abs() < 0.01);
        assert_eq!(room.perimeter, Length::from_feet(40.0));
    }

    #[test]
    fn room_schedule_resolves_each_room_on_its_own_level() {
        use framer_core::{Level, Point2, Room, RoomUsage};
        let mut model = rectangle_with_room();
        model
            .levels
            .push(Level::new("level-2", "Level 2", Length::from_feet(10.0)));
        model.rooms.push(Room::new(
            "room-2",
            "Unenclosed upper room",
            RoomUsage::Living,
            "level-2",
            Point2::new(Length::from_feet(6.0), Length::from_feet(4.0)),
        ));

        let plan = generate_project_plan(&model).unwrap();

        let upper = plan
            .rooms
            .iter()
            .find(|room| room.room == ElementId::new("room-2"))
            .expect("room-2 schedule row");
        assert!(
            !upper.closed,
            "a level-2 room must not borrow the level-1 rectangle"
        );
        assert_eq!(upper.area_square_inches, 0);
        assert!(plan.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "room.boundary.open"
                && diagnostic.source.as_ref().map(|id| id.0.as_str()) == Some("room-2")
        }));
    }

    #[test]
    fn open_room_emits_warning_diagnostic() {
        use framer_core::{Point2, Room, RoomUsage};
        let code = FramingDefaults::irc_2021_starter();
        let mut model = BuildingModel::new();
        model
            .walls
            .push(Wall::new("w-1", "Wall", Length::from_feet(12.0), &code));
        model.rooms.push(Room::new(
            "room-1",
            "Room",
            RoomUsage::Unspecified,
            "level-1",
            Point2::new(Length::from_feet(2.0), Length::from_feet(2.0)),
        ));

        let plan = generate_project_plan(&model).unwrap();

        assert_eq!(plan.rooms.len(), 1);
        assert!(!plan.rooms[0].closed);
        assert!(plan.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "room.boundary.open"
                && diagnostic.source.as_ref().map(|id| id.0.as_str()) == Some("room-1")
                && matches!(diagnostic.severity, DiagnosticSeverity::Warning)
        }));
    }

    #[test]
    fn demo_wall_emits_starter_standards_diagnostic_name() {
        let plan = generate_project_plan(&BuildingModel::demo_wall()).unwrap();
        let wall_plan = plan
            .wall_plan(&ElementId::new("wall-1"))
            .expect("demo wall plan");

        assert!(wall_plan.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "code-profile.starter-only"
                && diagnostic.source.as_ref().map(|id| id.0.as_str()) == Some("wall-1")
                && matches!(diagnostic.severity, DiagnosticSeverity::Warning)
                && diagnostic.message.contains(STARTER_STANDARDS_NAME)
        }));
    }

    #[test]
    fn project_svg_labels_rooms() {
        let model = rectangle_with_room();
        let plan = generate_project_plan(&model).unwrap();
        let svg = export_project_svg(&model, &plan);

        assert!(svg.contains(r#"data-room="room-1""#));
        assert!(svg.contains("Living"));
    }

    #[test]
    fn room_schedule_csv_has_a_row_per_room() {
        let plan = generate_project_plan(&rectangle_with_room()).unwrap();
        let csv = export_room_schedule_csv(&plan.rooms);

        assert!(csv.starts_with("room,name,usage,enclosed,area_sqft,perimeter_ft\n"));
        assert!(csv.contains("room-1"));
        assert!(csv.contains("Living"));
        assert!(csv.contains("96.0"));
    }

    #[test]
    fn wall_with_door_generates_kings_jacks_and_header() {
        let code = FramingDefaults::irc_2021_starter();
        let mut wall = Wall::new("wall", "Wall", Length::from_feet(12.0), &code);
        wall.openings.push(Opening::door(
            "door",
            "Door",
            Length::from_feet(4.0),
            Length::from_inches(36.0),
            Length::from_inches(80.0),
        ));

        let plan = generate_wall_plan(
            &wall,
            &wall_system(),
            &materials(),
            &starter_standards(),
            STARTER_STANDARDS_NAME,
        )
        .unwrap();

        assert_eq!(
            plan.members
                .iter()
                .filter(|member| member.kind == MemberKind::KingStud)
                .count(),
            2
        );
        assert_eq!(
            plan.members
                .iter()
                .filter(|member| member.kind == MemberKind::JackStud)
                .count(),
            2
        );
        assert!(plan.members.iter().any(|member| {
            member.kind == MemberKind::Header && member.cut_length == Length::from_inches(39.0)
        }));
    }

    #[test]
    fn header_sizing_uses_widest_band_and_tie_breaks() {
        let code = FramingDefaults::irc_2021_starter();
        let mut wall = Wall::new("wall", "Wall", Length::from_feet(12.0), &code);
        wall.openings.push(Opening::door(
            "door",
            "Door",
            Length::from_feet(4.0),
            Length::from_inches(56.0),
            Length::from_inches(80.0),
        ));
        let standards = standards_with_header_rows(vec![
            header_row(
                BoardProfile::TwoByFour,
                1,
                30,
                Length::from_feet(24.0),
                Length::from_feet(5.0),
                1,
            ),
            header_row(
                BoardProfile::TwoByTwelve,
                2,
                30,
                Length::from_feet(36.0),
                Length::from_feet(5.0),
                1,
            ),
            header_row(
                BoardProfile::TwoByTen,
                1,
                30,
                Length::from_feet(36.0),
                Length::from_feet(5.0),
                1,
            ),
            header_row(
                BoardProfile::TwoByEight,
                1,
                30,
                Length::from_feet(36.0),
                Length::from_feet(5.0),
                1,
            ),
            header_row(
                BoardProfile::TwoBySix,
                1,
                30,
                Length::from_feet(36.0),
                Length::from_feet(8.0),
                1,
            ),
        ]);
        let site = SiteContext {
            ground_snow_load_psf: Some(20),
            ..SiteContext::default()
        };

        let plan = generate_wall_plan_with_site(
            &wall,
            &wall_system(),
            &materials(),
            &standards,
            STARTER_STANDARDS_NAME,
            &site,
        )
        .unwrap();
        let header = find_member(&plan, "door-header");

        assert_eq!(header.profile, BoardProfile::TwoByEight);
        assert_eq!(
            header.cross_section_depth,
            BoardProfile::TwoByEight.nominal_depth()
        );
        assert_eq!(header.provenance.rule_id, "test.headers");
        assert!(header.provenance.summary.contains("Test Header Table"));
    }

    #[test]
    fn header_sizing_unknown_snow_uses_highest_snow_band() {
        let code = FramingDefaults::irc_2021_starter();
        let mut wall = Wall::new("wall", "Wall", Length::from_feet(12.0), &code);
        wall.openings.push(Opening::door(
            "door",
            "Door",
            Length::from_feet(4.0),
            Length::from_inches(48.0),
            Length::from_inches(80.0),
        ));
        let standards = standards_with_header_rows(vec![
            header_row(
                BoardProfile::TwoByEight,
                1,
                30,
                Length::from_feet(36.0),
                Length::from_feet(6.0),
                1,
            ),
            header_row(
                BoardProfile::TwoByTwelve,
                1,
                70,
                Length::from_feet(36.0),
                Length::from_feet(6.0),
                1,
            ),
        ]);

        let plan = generate_wall_plan(
            &wall,
            &wall_system(),
            &materials(),
            &standards,
            STARTER_STANDARDS_NAME,
        )
        .unwrap();

        assert_eq!(
            find_member(&plan, "door-header").profile,
            BoardProfile::TwoByTwelve
        );
    }

    #[test]
    fn header_sizing_known_snow_rejects_underrated_rows() {
        let code = FramingDefaults::irc_2021_starter();
        let mut wall = Wall::new("wall", "Wall", Length::from_feet(12.0), &code);
        wall.openings.push(Opening::door(
            "door",
            "Door",
            Length::from_feet(4.0),
            Length::from_inches(48.0),
            Length::from_inches(80.0),
        ));
        let standards = standards_with_header_rows(vec![
            header_row(
                BoardProfile::TwoByEight,
                1,
                30,
                Length::from_feet(36.0),
                Length::from_feet(6.0),
                1,
            ),
            header_row(
                BoardProfile::TwoByTwelve,
                1,
                70,
                Length::from_feet(36.0),
                Length::from_feet(6.0),
                1,
            ),
        ]);
        let site = SiteContext {
            ground_snow_load_psf: Some(50),
            ..SiteContext::default()
        };

        let plan = generate_wall_plan_with_site(
            &wall,
            &wall_system(),
            &materials(),
            &standards,
            STARTER_STANDARDS_NAME,
            &site,
        )
        .unwrap();

        assert_eq!(
            find_member(&plan, "door-header").profile,
            BoardProfile::TwoByTwelve
        );
    }

    #[test]
    fn header_sizing_out_of_domain_falls_back_and_diagnoses() {
        let code = FramingDefaults::irc_2021_starter();
        let mut wall = Wall::new("wall", "Wall", Length::from_feet(16.0), &code);
        wall.openings.push(Opening::door(
            "door",
            "Door",
            Length::from_feet(4.0),
            Length::from_feet(7.0),
            Length::from_inches(80.0),
        ));
        let standards = standards_with_header_rows(vec![header_row(
            BoardProfile::TwoByEight,
            1,
            30,
            Length::from_feet(36.0),
            Length::from_feet(4.0),
            1,
        )]);
        let site = SiteContext {
            ground_snow_load_psf: Some(20),
            ..SiteContext::default()
        };

        let plan = generate_wall_plan_with_site(
            &wall,
            &wall_system(),
            &materials(),
            &standards,
            STARTER_STANDARDS_NAME,
            &site,
        )
        .unwrap();
        let header = find_member(&plan, "door-header");

        assert_eq!(header.profile, code.header_profile);
        assert_eq!(header.cross_section_depth, code.default_header_depth);
        assert_eq!(header.provenance.rule_id, "opening.header.default-profile");
        assert!(plan.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "standards.header.out-of-domain"
                && diagnostic.source.as_ref().map(|id| id.0.as_str()) == Some("door")
                && matches!(diagnostic.severity, DiagnosticSeverity::Unsupported)
                && diagnostic.message.contains("Test Header Table")
                && diagnostic.message.contains("7' 0\"")
        }));
    }

    #[test]
    fn bearing_interior_wall_uses_header_tables() {
        let code = FramingDefaults::irc_2021_starter();
        let mut wall = Wall::new("wall", "Wall", Length::from_feet(12.0), &code);
        let system = framing_system("system-interior", BoardProfile::TwoByFour);
        wall.system = system.id.clone();
        wall.tags = vec!["bearing".to_owned()];
        wall.openings.push(Opening::door(
            "door",
            "Door",
            Length::from_feet(4.0),
            Length::from_inches(48.0),
            Length::from_inches(80.0),
        ));
        let standards = standards_with_header_rows(vec![header_row(
            BoardProfile::TwoByEight,
            1,
            30,
            Length::from_feet(36.0),
            Length::from_feet(6.0),
            1,
        )]);

        let plan = generate_wall_plan(
            &wall,
            &system,
            &materials(),
            &standards,
            STARTER_STANDARDS_NAME,
        )
        .unwrap();
        let header = find_member(&plan, "door-header");

        assert_eq!(header.profile, BoardProfile::TwoByEight);
        assert_eq!(header.provenance.rule_id, "test.headers");
    }

    #[test]
    fn nonbearing_interior_wall_uses_fallback_header_profile() {
        let code = FramingDefaults::irc_2021_starter();
        let mut wall = Wall::new("wall", "Wall", Length::from_feet(12.0), &code);
        let system = framing_system("system-interior", BoardProfile::TwoByFour);
        wall.system = system.id.clone();
        wall.openings.push(Opening::door(
            "door",
            "Door",
            Length::from_feet(4.0),
            Length::from_inches(48.0),
            Length::from_inches(80.0),
        ));
        let standards = standards_with_header_rows(vec![header_row(
            BoardProfile::TwoByEight,
            1,
            30,
            Length::from_feet(36.0),
            Length::from_feet(6.0),
            1,
        )]);

        let plan = generate_wall_plan(
            &wall,
            &system,
            &materials(),
            &standards,
            STARTER_STANDARDS_NAME,
        )
        .unwrap();
        let header = find_member(&plan, "door-header");

        assert_eq!(header.profile, code.header_profile);
        assert_eq!(header.provenance.rule_id, "opening.header.default-profile");
    }

    #[test]
    fn starter_header_table_drives_plies_jacks_and_provenance() {
        let model = BuildingModel::demo_wall();
        let system = model.system_for(&model.walls[0]).unwrap();
        let plan = generate_wall_plan(
            &model.walls[0],
            system,
            &model.materials,
            &model.resolved_standards(),
            STARTER_STANDARDS_NAME,
        )
        .unwrap();
        let opening = model.walls[0]
            .openings
            .iter()
            .find(|opening| opening.id.0 == "opening-garage-1")
            .expect("garage opening");
        let stud_thickness = model.framing_defaults().stud_profile.thickness();
        let half_stud = stud_thickness / 2;
        let garage_headers = plan
            .members
            .iter()
            .filter(|member| {
                member.source.0 == "opening-garage-1" && member.kind == MemberKind::Header
            })
            .collect::<Vec<_>>();
        let garage_jacks = plan
            .members
            .iter()
            .filter(|member| {
                member.source.0 == "opening-garage-1" && member.kind == MemberKind::JackStud
            })
            .count();
        let header = find_member(&plan, "opening-garage-1-header");
        let left_jack = find_member(&plan, "opening-garage-1-jack-left");
        let left_jack_2 = find_member(&plan, "opening-garage-1-jack-left-2");
        let right_jack = find_member(&plan, "opening-garage-1-jack-right");
        let right_jack_2 = find_member(&plan, "opening-garage-1-jack-right-2");
        let left_king = find_member(&plan, "opening-garage-1-king-left");
        let right_king = find_member(&plan, "opening-garage-1-king-right");

        assert_eq!(garage_headers.len(), 2);
        assert!(
            garage_headers
                .iter()
                .any(|member| member.id == "opening-garage-1-header")
        );
        assert!(
            garage_headers
                .iter()
                .any(|member| member.id == "opening-garage-1-header-2")
        );
        for header in garage_headers {
            assert_eq!(header.profile, BoardProfile::TwoByTwelve);
            assert_eq!(header.provenance.rule_id, "irc2021.r602.7-1.headers");
            assert!(header.provenance.summary.contains("2-ply 2x12"));
            assert!(
                header
                    .provenance
                    .summary
                    .contains("IRC 2021 Table R602.7(1)")
            );
        }
        assert_eq!(garage_jacks, 4);
        assert_eq!(left_jack.x, opening.left() - half_stud);
        assert_eq!(left_jack_2.x, opening.left() - half_stud - stud_thickness);
        assert_eq!(right_jack.x, opening.right() + half_stud);
        assert_eq!(right_jack_2.x, opening.right() + half_stud + stud_thickness);
        assert_eq!(left_king.x, opening.left() - stud_thickness * 2 - half_stud);
        assert_eq!(
            right_king.x,
            opening.right() + stud_thickness * 2 + half_stud
        );
        assert_eq!(header.x, opening.left() - stud_thickness * 2);
        assert_eq!(header.cut_length, opening.width + stud_thickness * 4);
    }

    #[test]
    fn members_sit_in_the_framing_layer_band() {
        let code = FramingDefaults::irc_2021_starter();
        let mut wall = Wall::new("wall", "Wall", Length::from_feet(12.0), &code);
        wall.openings.push(Opening::window(
            "window",
            "Window",
            Length::from_feet(5.0),
            Length::from_inches(36.0),
            Length::from_inches(48.0),
            Length::from_inches(30.0),
        ));
        let system = wall_system();

        // The framing band starts after every inboard layer and is exactly the
        // framing layer's own thickness (== the framing member's nominal depth).
        let framing = system.framing_layer().expect("seeded framing layer");
        let expected_depth = framing.thickness;
        let expected_offset = system
            .layers
            .iter()
            .take_while(|layer| layer.function != LayerFunction::Framing)
            .fold(Length::ZERO, |total, layer| total + layer.thickness);
        assert!(
            expected_offset > Length::ZERO,
            "exterior system has interior layers"
        );
        assert_eq!(
            expected_depth,
            framing.framing.as_ref().unwrap().member.nominal_depth()
        );

        let plan = generate_wall_plan(
            &wall,
            &system,
            &materials(),
            &starter_standards(),
            STARTER_STANDARDS_NAME,
        )
        .unwrap();
        assert!(!plan.members.is_empty());
        for member in &plan.members {
            assert_eq!(
                member.side_offset, expected_offset,
                "{} should sit at the framing-layer offset",
                member.id
            );
            assert_eq!(
                member.side_depth, expected_depth,
                "{} should fill the framing-layer depth",
                member.id
            );
        }
    }

    #[test]
    fn king_and_jack_studs_are_adjacent_not_overlapping() {
        let code = FramingDefaults::irc_2021_starter();
        let mut wall = Wall::new("wall", "Wall", Length::from_feet(12.0), &code);
        wall.openings.push(Opening::door(
            "door",
            "Door",
            Length::from_feet(4.0),
            Length::from_inches(36.0),
            Length::from_inches(80.0),
        ));

        let plan = generate_wall_plan(
            &wall,
            &wall_system(),
            &materials(),
            &starter_standards(),
            STARTER_STANDARDS_NAME,
        )
        .unwrap();
        let left_king = find_member(&plan, "door-king-left");
        let left_jack = find_member(&plan, "door-jack-left");
        let right_jack = find_member(&plan, "door-jack-right");
        let right_king = find_member(&plan, "door-king-right");

        assert_eq!(left_jack.x - left_king.x, code.stud_profile.thickness());
        assert_eq!(right_king.x - right_jack.x, code.stud_profile.thickness());
        assert_eq!(
            left_jack.x + code.stud_profile.thickness() / 2,
            wall.openings[0].left()
        );
        assert_eq!(
            right_jack.x - code.stud_profile.thickness() / 2,
            wall.openings[0].right()
        );
        assert!(!plan.members.iter().any(|member| {
            member.kind == MemberKind::CommonStud
                && member.x > left_king.x - code.stud_profile.thickness() / 2
                && member.x < right_king.x + code.stud_profile.thickness() / 2
        }));
    }

    #[test]
    fn bom_groups_cut_lengths() {
        let code = FramingDefaults::irc_2021_starter();
        let wall = Wall::new("wall", "Wall", Length::from_feet(8.0), &code);

        let plan = generate_wall_plan(
            &wall,
            &wall_system(),
            &materials(),
            &starter_standards(),
            STARTER_STANDARDS_NAME,
        )
        .unwrap();
        let bom = plan.bom();

        assert!(bom.iter().any(|item| {
            item.kind == MemberKind::TopPlate
                && item.cut_length == Length::from_feet(8.0)
                && item.quantity == 2
        }));
    }

    #[test]
    fn framing_member_sizes_studs_and_plates() {
        let code = FramingDefaults::irc_2021_starter();
        let mut wall = Wall::new("wall", "Wall", Length::from_feet(12.0), &code);
        let system = framing_system("system-2x6", BoardProfile::TwoBySix);
        wall.system = system.id.clone();
        wall.openings.push(Opening::door(
            "opening-door",
            "Door",
            Length::from_inches(72.0),
            Length::from_inches(36.0),
            Length::from_inches(80.0),
        ));

        let plan = generate_wall_plan(
            &wall,
            &system,
            &materials(),
            &starter_standards(),
            STARTER_STANDARDS_NAME,
        )
        .unwrap();

        for kind in [
            MemberKind::CommonStud,
            MemberKind::BottomPlate,
            MemberKind::TopPlate,
            MemberKind::KingStud,
            MemberKind::JackStud,
        ] {
            let member = plan
                .members
                .iter()
                .find(|member| member.kind == kind)
                .unwrap_or_else(|| panic!("expected a {kind:?} member"));
            assert_eq!(member.profile, BoardProfile::TwoBySix, "{kind:?}");
        }

        // The default starter table still selects the same nominal header
        // profile for this small rough opening.
        let header = plan
            .members
            .iter()
            .find(|member| member.kind == MemberKind::Header)
            .unwrap();
        assert_eq!(header.profile, code.header_profile);
    }

    #[test]
    fn end_studs_align_faces_with_wall_edges() {
        let code = FramingDefaults::irc_2021_starter();
        let wall = Wall::new("wall", "Wall", Length::from_feet(8.0), &code);

        let plan = generate_wall_plan(
            &wall,
            &wall_system(),
            &materials(),
            &starter_standards(),
            STARTER_STANDARDS_NAME,
        )
        .unwrap();
        let mut common_studs = plan
            .members
            .iter()
            .filter(|member| member.kind == MemberKind::CommonStud)
            .collect::<Vec<_>>();
        common_studs.sort_by_key(|member| member.x);

        let half_stud = code.stud_profile.thickness() / 2;
        assert_eq!(common_studs.first().unwrap().x, half_stud);
        assert_eq!(common_studs.last().unwrap().x, wall.length - half_stud);
        assert!(
            !common_studs
                .iter()
                .any(|member| member.x == Length::ZERO || member.x == wall.length)
        );
    }

    #[test]
    fn eight_foot_wall_uses_actual_plate_thickness_for_stud_length() {
        let code = FramingDefaults::irc_2021_starter();
        let wall = Wall::new("wall", "Wall", Length::from_feet(8.0), &code);

        let plan = generate_wall_plan(
            &wall,
            &wall_system(),
            &materials(),
            &starter_standards(),
            STARTER_STANDARDS_NAME,
        )
        .unwrap();

        assert!(plan.members.iter().any(|member| {
            member.kind == MemberKind::CommonStud && member.cut_length == Length::from_inches(91.5)
        }));
    }

    #[test]
    fn project_round_trip_regenerates_same_wall_plan() {
        let model = BuildingModel::demo_wall();
        let system = model.system_for(&model.walls[0]).unwrap();
        let original = generate_wall_plan(
            &model.walls[0],
            system,
            &model.materials,
            &model.resolved_standards(),
            model.base_standards_name().unwrap(),
        )
        .unwrap();

        let serialized = save_project(&model).unwrap();
        let loaded = load_project(&serialized).unwrap();
        let loaded_system = loaded.system_for(&loaded.walls[0]).unwrap();
        let regenerated = generate_wall_plan(
            &loaded.walls[0],
            loaded_system,
            &loaded.materials,
            &loaded.resolved_standards(),
            loaded.base_standards_name().unwrap(),
        )
        .unwrap();

        assert_eq!(loaded, model);
        assert_eq!(regenerated, original);
    }

    #[test]
    fn project_plan_frames_connected_shell_and_groups_bom() {
        let model = BuildingModel::demo_shell();
        let plan = generate_project_plan(&model).unwrap();

        assert_eq!(plan.wall_plans.len(), 4);
        assert!(plan.wall_plan(&ElementId::new("wall-front")).is_some());
        assert!(
            plan.wall_plans
                .iter()
                .flat_map(|wall_plan| wall_plan.members.iter())
                .any(|member| member.kind == MemberKind::CornerPost
                    && member.source.0 == "join-front-right")
        );
        assert!(plan.bom().iter().any(|item| {
            item.kind == MemberKind::CornerPost && item.quantity >= model.wall_joins.len() as u32
        }));
    }

    #[test]
    fn corner_posts_align_faces_with_joined_wall_edges() {
        let model = BuildingModel::demo_shell();
        let plan = generate_project_plan(&model).unwrap();
        let half_stud = model.framing_defaults().stud_profile.thickness() / 2;

        for join in &model.wall_joins {
            for wall_id in [&join.first_wall, &join.second_wall] {
                let wall = model
                    .walls
                    .iter()
                    .find(|candidate| candidate.id == *wall_id)
                    .unwrap();
                let join_x = wall.local_x_for_point(join.point).unwrap();
                let expected_x = face_aligned_center(
                    join_x,
                    wall.length,
                    model.framing_defaults().stud_profile.thickness(),
                );
                let wall_plan = plan.wall_plan(&wall.id).unwrap();
                let member_id = format!("{}-{}-corner-post", join.id.0, wall.id.0);
                let member = find_member(wall_plan, &member_id);

                assert_eq!(member.x, expected_x, "{member_id}");
                assert!(member.x >= half_stud, "{member_id}");
                assert!(member.x <= wall.length - half_stud, "{member_id}");
            }
        }
    }

    #[test]
    fn project_exports_include_whole_shell_plan_and_elevations() {
        let model = BuildingModel::demo_shell();
        let plan = generate_project_plan(&model).unwrap();
        let svg = export_project_svg(&model, &plan);
        let csv = export_bom_csv(&plan.bom(), &plan.fasteners);

        assert!(svg.contains("data-wall=\"wall-front\""));
        assert!(svg.contains("data-join=\"join-front-right\""));
        assert!(svg.contains("data-wall-elevation=\"wall-right\""));
        assert!(csv.contains("corner post"));
    }

    #[test]
    fn starter_fastening_schedule_emits_expected_takeoff_for_known_wall() {
        let code = FramingDefaults::irc_2021_starter();
        let mut model = BuildingModel::new();
        model
            .walls
            .push(Wall::new("wall", "Wall", Length::from_feet(8.0), &code));

        let plan = generate_project_plan(&model).unwrap();
        let second = generate_project_plan(&model).unwrap();

        assert_eq!(
            plan.fasteners, second.fasteners,
            "fastener takeoff should be deterministic"
        );

        let end_nails = fastener_row(&plan, "16d common nail", ConnectionKind::StudToPlateEnd);
        let toe_nails = fastener_row(&plan, "8d common nail", ConnectionKind::StudToPlateToe);
        let double_top = fastener_row(&plan, "16d common nail", ConnectionKind::DoubleTopPlate);

        assert_eq!(end_nails.quantity, 28);
        assert_eq!(end_nails.rule, "irc2021.r602.3-1.fastening");
        assert_eq!(end_nails.citation, "IRC 2021 Table R602.3(1)");
        assert_eq!(toe_nails.quantity, 56);
        assert_eq!(double_top.quantity, 6);
        assert!(
            !plan
                .fasteners
                .iter()
                .any(|row| row.connection == ConnectionKind::TopPlateLap)
        );

        let csv = export_bom_csv(&plan.bom(), &plan.fasteners);
        assert!(csv.contains("\n\nfastener_quantity,fastener,connection,rule,citation\n"));
        assert!(csv.contains(
            "28,16d common nail,stud to plate end,irc2021.r602.3-1.fastening,IRC 2021 Table R602.3(1)"
        ));
        assert!(csv.contains(
            "6,16d common nail,double top plate,irc2021.r602.3-1.fastening,IRC 2021 Table R602.3(1)"
        ));
    }

    #[test]
    fn double_top_plate_spacing_rounds_up_partial_bays() {
        let code = FramingDefaults::irc_2021_starter();
        let mut model = BuildingModel::new();
        model
            .walls
            .push(Wall::new("wall", "Wall", Length::from_inches(102.0), &code));

        let plan = generate_project_plan(&model).unwrap();
        let double_top = fastener_row(&plan, "16d common nail", ConnectionKind::DoubleTopPlate);

        assert_eq!(double_top.quantity, 7);
    }

    #[test]
    fn fastening_schedule_counts_header_to_king_connections_from_generated_headers() {
        let code = FramingDefaults::irc_2021_starter();
        let mut model = BuildingModel::new();
        let mut wall = Wall::new("wall", "Wall", Length::from_feet(12.0), &code);
        wall.openings.push(Opening::door(
            "door",
            "Door",
            Length::from_feet(4.0),
            Length::from_inches(36.0),
            Length::from_inches(80.0),
        ));
        model.walls.push(wall);
        add_fastening_pack(
            &mut model,
            "test.fastening.header",
            vec![FasteningRow {
                connection: ConnectionKind::HeaderToKingStud,
                fastener: "10d common nail".to_owned(),
                schedule: FastenerSchedule::Count(2),
            }],
        );

        let plan = generate_project_plan(&model).unwrap();
        let row = fastener_row(&plan, "10d common nail", ConnectionKind::HeaderToKingStud);

        assert_eq!(row.quantity, 4);
        assert_eq!(row.rule, "test.fastening.header");
        assert_eq!(row.citation, "Test Fastening Schedule");
    }

    #[test]
    fn sheathing_fastening_rows_emit_single_not_counted_diagnostic() {
        let code = FramingDefaults::irc_2021_starter();
        let mut model = BuildingModel::new();
        model
            .walls
            .push(Wall::new("wall", "Wall", Length::from_feet(8.0), &code));
        add_fastening_pack(
            &mut model,
            "test.fastening.sheathing",
            vec![
                FasteningRow {
                    connection: ConnectionKind::SheathingEdge,
                    fastener: "8d sheathing nail".to_owned(),
                    schedule: FastenerSchedule::EdgeField {
                        edge: Length::from_whole_inches(6),
                        field: Length::from_whole_inches(12),
                    },
                },
                FasteningRow {
                    connection: ConnectionKind::SheathingField,
                    fastener: "8d sheathing nail".to_owned(),
                    schedule: FastenerSchedule::EdgeField {
                        edge: Length::from_whole_inches(6),
                        field: Length::from_whole_inches(12),
                    },
                },
            ],
        );

        let plan = generate_project_plan(&model).unwrap();

        assert_eq!(
            plan.diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == "standards.fastening.sheathing-not-counted")
                .count(),
            1
        );
        assert!(
            !plan
                .fasteners
                .iter()
                .any(|row| row.fastener == "8d sheathing nail")
        );
    }

    /// A minimal single-framing-layer wall system using `member` at the given
    /// on-center `spacing`. Lets tests vary the layout spacing independently of
    /// the code-profile default.
    fn framing_system_spaced(
        id: &str,
        member: BoardProfile,
        spacing: Length,
    ) -> ConstructionSystem {
        let mut system = framing_system(id, member);
        if let Some(layer) = system
            .layers
            .iter_mut()
            .find(|layer| layer.function == LayerFunction::Framing)
            && let Some(framing) = layer.framing.as_mut()
        {
            framing.spacing = spacing;
        }
        system
    }

    #[test]
    fn opening_cripples_honor_system_spacing_not_code_default() {
        // A wall system framed at 24" o.c. — distinct from the code profile's
        // 16" default — must lay out its opening cripples at 24" o.c.
        let code = FramingDefaults::irc_2021_starter();
        assert_eq!(
            code.default_stud_spacing,
            Length::from_whole_inches(16),
            "this test relies on the code default being 16in so a 24in system is distinguishable"
        );

        let system = framing_system_spaced(
            "system-2x6-24oc",
            BoardProfile::TwoBySix,
            Length::from_whole_inches(24),
        );
        let mut wall = Wall::new("wall", "Wall", Length::from_feet(16.0), &code);
        wall.system = system.id.clone();
        // A wide window (10ft), centred in the 16ft wall so several cripples fit
        // above and below it with room for end framing.
        wall.openings.push(Opening::window(
            "window",
            "Window",
            Length::from_feet(8.0),
            Length::from_feet(10.0),
            Length::from_inches(36.0),
            Length::from_inches(36.0),
        ));

        let plan = generate_wall_plan(
            &wall,
            &system,
            &materials(),
            &starter_standards(),
            STARTER_STANDARDS_NAME,
        )
        .unwrap();

        // Collect the distinct cripple x-positions for the upper band and assert
        // the on-center step is 24in (system spacing), not 16in (code default).
        let mut upper_x: Vec<Length> = plan
            .members
            .iter()
            .filter(|member| {
                member.kind == MemberKind::CrippleStud && member.id.contains("-cripple-upper-")
            })
            .map(|member| member.x)
            .collect();
        upper_x.sort_unstable();
        upper_x.dedup();

        assert!(
            upper_x.len() >= 2,
            "a 10ft opening at 24in o.c. must yield multiple upper cripples, got {upper_x:?}"
        );
        for window in upper_x.windows(2) {
            assert_eq!(
                window[1] - window[0],
                Length::from_whole_inches(24),
                "cripples must step at the system 24in spacing, got {upper_x:?}"
            );
        }

        // The first cripple sits one system-spacing in from the opening's left
        // edge — confirming the layout starts from the opening, at 24in.
        assert_eq!(
            upper_x[0] - wall.openings[0].left(),
            Length::from_whole_inches(24),
            "first cripple should be 24in from the opening edge"
        );
    }

    #[test]
    fn window_generates_sill_and_cripples() {
        let code = FramingDefaults::irc_2021_starter();
        let mut wall = Wall::new("wall", "Wall", Length::from_feet(12.0), &code);
        wall.openings.push(Opening::window(
            "window",
            "Window",
            Length::from_feet(4.0),
            Length::from_inches(48.0),
            Length::from_inches(36.0),
            Length::from_inches(36.0),
        ));

        let plan = generate_wall_plan(
            &wall,
            &wall_system(),
            &materials(),
            &starter_standards(),
            STARTER_STANDARDS_NAME,
        )
        .unwrap();

        assert!(
            plan.members
                .iter()
                .any(|member| member.kind == MemberKind::RoughSill)
        );
        assert!(
            plan.members
                .iter()
                .any(|member| member.kind == MemberKind::CrippleStud)
        );
    }

    #[test]
    fn generated_members_include_source_and_rule_provenance() {
        let model = BuildingModel::demo_wall();
        let system = model.system_for(&model.walls[0]).unwrap();
        let plan = generate_wall_plan(
            &model.walls[0],
            system,
            &model.materials,
            &model.resolved_standards(),
            model.base_standards_name().unwrap(),
        )
        .unwrap();

        let header = plan
            .members
            .iter()
            .find(|member| member.id == "opening-door-1-header")
            .unwrap();

        assert_eq!(header.source.0, "opening-door-1");
        assert_eq!(header.provenance.rule_id, "irc2021.r602.7-1.headers");
        assert!(
            header
                .provenance
                .summary
                .contains("IRC 2021 Table R602.7(1)")
        );
        assert_eq!(
            header.cross_section_depth,
            model.framing_defaults().header_profile.nominal_depth()
        );
    }

    #[test]
    fn garage_door_reports_unsupported_starter_assumption() {
        let model = BuildingModel::demo_wall();
        let system = model.system_for(&model.walls[0]).unwrap();
        let plan = generate_wall_plan(
            &model.walls[0],
            system,
            &model.materials,
            &model.resolved_standards(),
            model.base_standards_name().unwrap(),
        )
        .unwrap();

        assert!(plan.diagnostics.iter().any(|diagnostic| {
            diagnostic.severity == DiagnosticSeverity::Unsupported
                && diagnostic.code == "opening.garage-door.starter-header"
                && diagnostic
                    .source
                    .as_ref()
                    .is_some_and(|source| source.0 == "opening-garage-1")
        }));
    }

    #[test]
    fn exports_are_deterministic_and_useful() {
        let model = BuildingModel::demo_wall();
        let wall = &model.walls[0];
        let system = model.system_for(wall).unwrap();
        let plan = generate_wall_plan(
            wall,
            system,
            &model.materials,
            &model.resolved_standards(),
            model.base_standards_name().unwrap(),
        )
        .unwrap();

        let first_svg = export_wall_elevation_svg(wall, &plan);
        let second_svg = export_wall_elevation_svg(wall, &plan);
        let csv = export_bom_csv(&plan.bom(), &[]);

        assert_eq!(first_svg, second_svg);
        assert!(first_svg.contains("<svg"));
        assert!(first_svg.contains("opening-garage-1-header"));
        assert!(first_svg.contains(r#"data-opening="opening-garage-1""#));
        assert!(
            first_svg
                .lines()
                .any(|line| line.contains(r#"id="opening-door-1-header""#)
                    && line.contains(r#"height="10""#))
        );
        assert!(csv.starts_with("quantity,profile,kind"));
        assert!(csv.contains("total_length_inches"));
    }

    fn find_member<'a>(plan: &'a WallFramePlan, id: &str) -> &'a FrameMember {
        plan.members
            .iter()
            .find(|member| member.id == id)
            .unwrap_or_else(|| panic!("expected member {id}"))
    }

    fn find_layer<'a>(layers: &'a [LayerBomItem], material: &str) -> &'a LayerBomItem {
        layers
            .iter()
            .find(|layer| layer.material.0 == material)
            .unwrap_or_else(|| panic!("expected layer for material {material}"))
    }

    #[test]
    fn layer_bom_area_goods_equal_net_face_area() {
        // 12ft x 8ft = 144" x 96" = 13_824 sq in gross; a 36" x 80" door removes
        // 2_880 sq in, leaving a 10_944 sq in net face.
        let code = FramingDefaults::irc_2021_starter();
        let mut wall = Wall::new("wall", "Wall", Length::from_feet(12.0), &code);
        wall.height = Length::from_feet(8.0);
        wall.openings.push(Opening::door(
            "door",
            "Door",
            Length::from_feet(4.0),
            Length::from_inches(36.0),
            Length::from_inches(80.0),
        ));

        let net_area = 144 * 96 - 36 * 80;
        assert_eq!(net_area, 10_944);

        let plan = generate_wall_plan(
            &wall,
            &wall_system(),
            &materials(),
            &starter_standards(),
            STARTER_STANDARDS_NAME,
        )
        .unwrap();

        // Area goods (drywall, plywood sheathing, fiber-cement cladding) each report
        // exactly the net face area and carry no volume.
        for material in ["mat-drywall", "mat-plywood", "mat-fiber-cement"] {
            let layer = find_layer(&plan.layers, material);
            assert_eq!(layer.area_sq_in, net_area, "{material} area");
            assert_eq!(layer.volume_bd_in, 0, "{material} volume");
        }

        // The rain-screen air gap is skipped entirely.
        assert!(
            !plan
                .layers
                .iter()
                .any(|layer| layer.material.0 == "mat-rainscreen"),
            "air-gap layers must not appear in the takeoff"
        );

        // Material names are resolved from the library.
        assert_eq!(
            find_layer(&plan.layers, "mat-drywall").material_name,
            "5/8\" Gypsum"
        );
    }

    #[test]
    fn layer_bom_volumetric_goods_equal_area_times_thickness() {
        let code = FramingDefaults::irc_2021_starter();
        let mut wall = Wall::new("wall", "Wall", Length::from_feet(12.0), &code);
        wall.height = Length::from_feet(8.0);
        wall.openings.push(Opening::door(
            "door",
            "Door",
            Length::from_feet(4.0),
            Length::from_inches(36.0),
            Length::from_inches(80.0),
        ));
        let net_area = 144 * 96 - 36 * 80;

        let plan = generate_wall_plan(
            &wall,
            &wall_system(),
            &materials(),
            &starter_standards(),
            STARTER_STANDARDS_NAME,
        )
        .unwrap();

        // Continuous polyiso (2") reports area x thickness as cubic inches.
        let polyiso = find_layer(&plan.layers, "mat-polyiso");
        assert_eq!(polyiso.function, LayerFunction::ContinuousInsulation);
        assert_eq!(polyiso.area_sq_in, 0);
        assert_eq!(polyiso.volume_bd_in, net_area * 2);

        // The framing layer's cavity mineral wool fills the 4" framing depth and is
        // taken off volumetrically; the lumber itself stays in the member BOM.
        let cavity = find_layer(&plan.layers, "mat-mineral-wool");
        assert_eq!(cavity.area_sq_in, 0);
        assert_eq!(cavity.volume_bd_in, net_area * 4);
        assert!(
            !plan
                .layers
                .iter()
                .any(|layer| layer.material.0 == "mat-spf"),
            "framing lumber is counted in the member BOM, not the layer takeoff"
        );
    }

    #[test]
    fn project_layer_bom_aggregates_across_walls() {
        let model = BuildingModel::demo_shell();
        let plan = generate_project_plan(&model).unwrap();

        // Each wall contributes a drywall area row; the project takeoff aggregates
        // them into a single grouped row whose area is the sum of the per-wall areas.
        let expected: i64 = plan
            .wall_plans
            .iter()
            .flat_map(|wall_plan| wall_plan.layers.iter())
            .filter(|layer| layer.material.0 == "mat-drywall")
            .map(|layer| layer.area_sq_in)
            .sum();
        assert!(expected > 0);

        let aggregated: Vec<&LayerBomItem> = plan
            .layers
            .iter()
            .filter(|layer| layer.material.0 == "mat-drywall")
            .collect();
        // One thickness of drywall => one aggregated row across the whole shell.
        assert_eq!(aggregated.len(), 1);
        assert_eq!(aggregated[0].area_sq_in, expected);

        // layer_bom() on the project mirrors the stored aggregate.
        assert_eq!(plan.layer_bom(), plan.layers);
    }

    #[test]
    fn export_layer_bom_csv_has_header_and_rows() {
        let model = BuildingModel::demo_shell();
        let plan = generate_project_plan(&model).unwrap();
        let csv = export_layer_bom_csv(&plan.layers);

        assert!(csv.starts_with(
            "material,name,function,thickness_inches,thickness_display,area_sq_in,area_sq_ft,volume_cu_in,volume_bd_ft\n"
        ));
        assert!(csv.contains("mat-drywall"));
        assert!(csv.contains("Continuous insulation"));
        // The lumber CSV is left untouched.
        assert!(export_bom_csv(&plan.bom(), &[]).starts_with("quantity,profile,kind"));
    }

    // ---- Floor decks & flat ceilings (Slice 2) -----------------------------

    /// A `w_ft` × `d_ft` rectangular surface region anchored at the origin (X is
    /// the width, Y is the depth).
    fn rect_region(w_ft: f64, d_ft: f64) -> SurfaceRegion {
        let (w, d) = (Length::from_feet(w_ft), Length::from_feet(d_ft));
        SurfaceRegion::Polygon(vec![
            Point2::new(Length::ZERO, Length::ZERO),
            Point2::new(w, Length::ZERO),
            Point2::new(w, d),
            Point2::new(Length::ZERO, d),
        ])
    }

    /// The polygon outline behind a `Polygon` region (panics for a `Room` region).
    fn region_points(region: &SurfaceRegion) -> &[Point2] {
        match region {
            SurfaceRegion::Polygon(points) => points,
            SurfaceRegion::Room(_) => panic!("test region must be a polygon"),
        }
    }

    /// A floor system: a 3/4" plywood subfloor over 2x8 joists at 16" o.c.
    fn floor_system() -> ConstructionSystem {
        ConstructionSystem {
            id: ElementId::new("system-floor"),
            name: "Floor".to_owned(),
            kind: framer_core::SystemKind::Floor,
            source: None,
            layers: vec![
                framer_core::ConstructionLayer::new(
                    LayerFunction::Sheathing,
                    "mat-plywood",
                    Length::from_inches(0.75),
                ),
                framer_core::ConstructionLayer::new(
                    LayerFunction::Framing,
                    "mat-spf",
                    BoardProfile::TwoByEight.nominal_depth(),
                )
                .with_framing(FramingSpec {
                    member: BoardProfile::TwoByEight,
                    spacing: Length::from_whole_inches(16),
                    pattern: framer_core::FramingPattern::Single,
                    member_family: framer_core::MemberFamily::FloorJoist,
                    cavity_material: None,
                }),
            ],
        }
    }

    /// A ceiling system: a 5/8" drywall finish under 2x6 joists at 16" o.c.
    fn ceiling_system() -> ConstructionSystem {
        ConstructionSystem {
            id: ElementId::new("system-ceiling"),
            name: "Ceiling".to_owned(),
            kind: framer_core::SystemKind::Ceiling,
            source: None,
            layers: vec![
                framer_core::ConstructionLayer::new(
                    LayerFunction::CeilingFinish,
                    "mat-drywall",
                    Length::from_inches(0.625),
                ),
                framer_core::ConstructionLayer::new(
                    LayerFunction::Framing,
                    "mat-spf",
                    BoardProfile::TwoBySix.nominal_depth(),
                )
                .with_framing(FramingSpec {
                    member: BoardProfile::TwoBySix,
                    spacing: Length::from_whole_inches(16),
                    pattern: framer_core::FramingPattern::Single,
                    member_family: framer_core::MemberFamily::CeilingJoist,
                    cavity_material: None,
                }),
            ],
        }
    }

    #[test]
    fn floor_deck_arrays_joists_across_the_shorter_span() {
        // A 20ft (x) × 12ft (y) deck: joists span the shorter 12ft dimension and
        // are arrayed across the 20ft length at 16in o.c.
        let deck = FloorDeck::new(
            "deck",
            "Deck",
            "level-1",
            "system-floor",
            rect_region(20.0, 12.0),
        );
        let plan = generate_floor_plan(
            &deck,
            &floor_system(),
            region_points(&deck.region),
            &materials(),
        )
        .unwrap();

        // stud_positions(240in, 16in, 1.5in) => 0.75, 16, 32, .., 224, 239.25 = 16.
        let joists: Vec<_> = plan
            .members
            .iter()
            .filter(|member| member.kind == MemberKind::FloorJoist)
            .collect();
        assert_eq!(joists.len(), 16);
        assert!(
            joists
                .iter()
                .all(|member| member.cut_length == Length::from_feet(12.0)),
            "every joist spans the shorter 12ft dimension"
        );
        assert!(
            joists
                .iter()
                .all(|member| member.profile == BoardProfile::TwoByEight),
            "joists use the system framing member"
        );
        // End joists align with the region edges; interiors fall on 16in marks.
        let half = BoardProfile::TwoByEight.thickness() / 2;
        let mut marks: Vec<Length> = joists.iter().map(|member| member.x).collect();
        marks.sort_unstable();
        assert_eq!(*marks.first().unwrap(), half);
        assert_eq!(*marks.last().unwrap(), Length::from_feet(20.0) - half);

        // Two rim/band members run the full 20ft layout width.
        let rims: Vec<_> = plan
            .members
            .iter()
            .filter(|member| member.kind == MemberKind::RimJoist)
            .collect();
        assert_eq!(rims.len(), 2);
        assert!(
            rims.iter()
                .all(|member| member.cut_length == Length::from_feet(20.0))
        );

        // Plan-local frame: joists run along the span axis (Vertical), rim and
        // blocking run across it (Horizontal).
        assert!(
            joists
                .iter()
                .all(|member| member.orientation == MemberOrientation::Vertical)
        );
        assert!(
            rims.iter()
                .all(|member| member.orientation == MemberOrientation::Horizontal)
        );

        // The two rims sit at the two bearing ends of the 12ft span: one at the
        // origin and one a joist-thickness inboard of the far edge.
        let span = Length::from_feet(12.0);
        let thickness = BoardProfile::TwoByEight.thickness();
        let mut rim_elevations: Vec<Length> = rims.iter().map(|member| member.elevation).collect();
        rim_elevations.sort_unstable();
        assert_eq!(rim_elevations, vec![Length::ZERO, span - thickness]);

        // One mid-span blocking piece sits in each clear gap between adjacent
        // joists, all on the same mid-span line, each spanning the clear gap.
        let blocking: Vec<_> = plan
            .members
            .iter()
            .filter(|member| member.kind == MemberKind::Blocking)
            .collect();
        assert_eq!(blocking.len(), joists.len() - 1);
        let mid_span = (span - thickness) / 2;
        assert!(
            blocking
                .iter()
                .all(|member| member.orientation == MemberOrientation::Horizontal
                    && member.elevation == mid_span)
        );
        // An interior gap spans exactly the on-center spacing less a joist face.
        assert!(
            blocking
                .iter()
                .any(|member| member.cut_length == Length::from_whole_inches(16) - thickness),
            "an interior blocking piece spans the o.c. spacing minus a joist thickness"
        );
    }

    #[test]
    fn floor_deck_explicit_span_overrides_shorter() {
        // Spanning the longer (20ft) axis: joists are 20ft long, arrayed across 12ft.
        let deck = FloorDeck::new(
            "deck",
            "Deck",
            "level-1",
            "system-floor",
            rect_region(20.0, 12.0),
        )
        .with_span(SpanDirection::Along);
        let plan = generate_floor_plan(
            &deck,
            &floor_system(),
            region_points(&deck.region),
            &materials(),
        )
        .unwrap();

        let joists: Vec<_> = plan
            .members
            .iter()
            .filter(|member| member.kind == MemberKind::FloorJoist)
            .collect();
        // stud_positions(144in, 16in, 1.5in) => 0.75, 16, .., 128, 143.25 = 10.
        assert_eq!(joists.len(), 10);
        assert!(
            joists
                .iter()
                .all(|member| member.cut_length == Length::from_feet(20.0))
        );
    }

    #[test]
    fn floor_deck_explicit_direction_selects_the_nearest_world_axis() {
        let system = floor_system();
        let span_along = |direction: Point2| -> Length {
            let deck = FloorDeck::new(
                "deck",
                "Deck",
                "level-1",
                "system-floor",
                rect_region(20.0, 12.0),
            )
            .with_span(SpanDirection::Explicit(direction));
            let plan =
                generate_floor_plan(&deck, &system, region_points(&deck.region), &materials())
                    .unwrap();
            plan.members
                .iter()
                .find(|member| member.kind == MemberKind::FloorJoist)
                .expect("a floor joist")
                .cut_length
        };

        // A direction biased toward +x runs joists along the 20ft x axis; one
        // biased toward +y runs them along the 12ft y axis. Asserting both pins
        // the axis-selection comparison against an inverted-direction regression.
        assert_eq!(
            span_along(Point2::new(
                Length::from_whole_inches(10),
                Length::from_whole_inches(1)
            )),
            Length::from_feet(20.0)
        );
        assert_eq!(
            span_along(Point2::new(
                Length::from_whole_inches(1),
                Length::from_whole_inches(10)
            )),
            Length::from_feet(12.0)
        );
    }

    #[test]
    fn surface_plan_reports_missing_system_for_a_dangling_reference() {
        // generate_surface_plans is defensive against a deck whose system id does
        // not resolve (model validation normally catches this first).
        let mut model = BuildingModel::new();
        model.floor_decks.push(FloorDeck::new(
            "deck-1",
            "Deck",
            "level-1",
            "system-absent",
            rect_region(10.0, 8.0),
        ));
        let mut plan = ProjectFramePlan {
            wall_plans: Vec::new(),
            floor_plans: Vec::new(),
            ceiling_plans: Vec::new(),
            roof_plans: Vec::new(),
            diagnostics: Vec::new(),
            rooms: Vec::new(),
            layers: Vec::new(),
            fasteners: Vec::new(),
        };

        let error = generate_surface_plans(&mut plan, &model).unwrap_err();
        assert!(matches!(
            error,
            SolverError::MissingSystemForElement { element, system }
                if element.0 == "deck-1" && system.0 == "system-absent"
        ));
    }

    #[test]
    fn surface_plan_reports_missing_level_for_a_dangling_ceiling_level() {
        // generate_surface_plans fails loudly on a ceiling whose level does not
        // resolve (model validation normally catches this first), rather than framing
        // it at a silently-wrong elevation. The system resolves, so the level lookup
        // is reached.
        let mut model = BuildingModel::new();
        model.systems.push(ceiling_system());
        model.ceilings.push(Ceiling::new(
            "clg",
            "Ceiling",
            "level-absent",
            "system-ceiling",
            rect_region(10.0, 8.0),
            Length::from_feet(8.0),
        ));
        let mut plan = ProjectFramePlan {
            wall_plans: Vec::new(),
            floor_plans: Vec::new(),
            ceiling_plans: Vec::new(),
            roof_plans: Vec::new(),
            diagnostics: Vec::new(),
            rooms: Vec::new(),
            layers: Vec::new(),
            fasteners: Vec::new(),
        };

        let error = generate_surface_plans(&mut plan, &model).unwrap_err();
        assert!(matches!(
            error,
            SolverError::MissingLevelForElement { element, level }
                if element.0 == "clg" && level.0 == "level-absent"
        ));
    }

    #[test]
    fn ceiling_generates_ceiling_joists_spanning_the_shorter_dimension() {
        let ceiling = Ceiling::new(
            "clg",
            "Ceiling",
            "level-1",
            "system-ceiling",
            rect_region(20.0, 12.0),
            Length::from_feet(8.0),
        );
        let plan = generate_ceiling_plan(
            &ceiling,
            &ceiling_system(),
            region_points(&ceiling.region),
            Length::from_feet(8.0),
            &materials(),
        )
        .unwrap();

        let joists: Vec<_> = plan
            .members
            .iter()
            .filter(|member| member.kind == MemberKind::CeilingJoist)
            .collect();
        assert!(!joists.is_empty());
        assert!(
            plan.members
                .iter()
                .all(|member| member.kind != MemberKind::FloorJoist),
            "a ceiling frames ceiling joists, never floor joists"
        );
        assert!(
            joists
                .iter()
                .all(|member| member.cut_length == Length::from_feet(12.0))
        );
        assert!(
            joists
                .iter()
                .all(|member| member.profile == BoardProfile::TwoBySix)
        );
    }

    #[test]
    fn sloped_ceiling_frames_joists_at_true_sloped_length() {
        // A 20ft-eave × 12ft-run scissor ceiling at a 9:12 pitch (factor 1.25, ratio
        // 0.75), springing at 8ft. low_edge 0 is the y=0 side (the 20ft eave), so the
        // joists run the 12ft plan run up-slope.
        let mut ceiling = Ceiling::new(
            "clg",
            "Scissor",
            "level-1",
            "system-ceiling",
            rect_region(20.0, 12.0),
            Length::from_feet(8.0),
        );
        ceiling.slope = Some(framer_core::CeilingSlope::new(pitch_9_12(), 0));
        let plan = generate_ceiling_plan(
            &ceiling,
            &ceiling_system(),
            region_points(&ceiling.region),
            Length::from_feet(8.0),
            &materials(),
        )
        .unwrap();

        let joists: Vec<_> = plan
            .members
            .iter()
            .filter(|member| member.kind == MemberKind::CeilingJoist)
            .collect();
        // Joists array along the 20ft (240in) low edge at 16in o.c. → 16 (plan spacing).
        assert_eq!(joists.len(), 16, "joist count uses the plan eave length");
        // True sloped cut: 144in plan run × 1.25 = 180in = 15ft (plan run is only 12ft).
        assert!(
            joists
                .iter()
                .all(|member| member.cut_length == Length::from_feet(15.0)),
            "joists are cut to true sloped length, not the 12ft plan run"
        );
        // Building elevations: springs at 8ft (96in), rises 144×0.75 = 108in to 204in.
        for joist in &joists {
            let sloped = joist
                .sloped
                .expect("a sloped ceiling joist carries a sloped placement");
            assert_eq!(sloped.low_elevation, Length::from_feet(8.0));
            assert_eq!(sloped.high_elevation, Length::from_inches(204.0));
        }
        // Band joists close both sloped ends; mid-slope blocking sits between joists.
        assert_eq!(
            plan.members
                .iter()
                .filter(|member| member.kind == MemberKind::RimJoist)
                .count(),
            2,
            "a band joist at the low and high edge"
        );
        assert!(
            plan.members
                .iter()
                .any(|member| member.kind == MemberKind::Blocking)
        );
        // The per-layer takeoff uses the plan footprint area (20ft × 12ft), not the
        // larger true sloped surface area.
        let finish_area: i64 = plan
            .layers
            .iter()
            .filter(|layer| layer.function == LayerFunction::CeilingFinish)
            .map(|layer| layer.area_sq_in)
            .sum();
        assert_eq!(finish_area, 240 * 144);
    }

    #[test]
    fn scissor_ceiling_emits_a_scissor_diagnostic_and_leaves_the_ridge_untied() {
        // A scissor/vault ceiling under a gable is not a flat rafter tie: the ceiling
        // carries a scissor note, and (consistent with A1.1's fork) the roof reverts
        // to a structural ridge beam.
        let mut model = tied_gable_model();
        model.ceilings[0].slope = Some(framer_core::CeilingSlope::new(
            Slope::new(Length::from_whole_inches(3), Length::from_whole_inches(12)),
            0,
        ));
        let plan = generate_project_plan(&model).unwrap();

        let ceiling = plan.ceiling_plan(&ElementId::new("ceiling-1")).unwrap();
        assert!(
            ceiling.diagnostics.iter().any(|d| {
                d.code == "ceiling.slope.scissor" && d.severity == DiagnosticSeverity::Info
            }),
            "a sloped ceiling reports that it is a scissor/vault, not a flat tie"
        );
        // The A1.1 fork reads the sloped ceiling as not a flat tie → ridge beam.
        assert!(!roof_a_ridge_is_tied(&model));
    }

    #[test]
    fn degenerate_sloped_ceiling_falls_back_to_flat_with_a_warning() {
        // A sloped ceiling whose low edge is zero-length frames no slope: rather than
        // silently dropping it, the generator falls back to a flat joist plan and
        // warns (mirroring the roof's degenerate-outline path). Driven directly since
        // a zero-length edge frames nothing.
        let mut ceiling = Ceiling::new(
            "clg",
            "Bad scissor",
            "level-1",
            "system-ceiling",
            SurfaceRegion::Polygon(vec![
                Point2::new(Length::ZERO, Length::ZERO),
                Point2::new(Length::ZERO, Length::ZERO), // coincident → zero-length low edge
                Point2::new(Length::from_feet(20.0), Length::from_feet(12.0)),
                Point2::new(Length::ZERO, Length::from_feet(12.0)),
            ]),
            Length::from_feet(8.0),
        );
        ceiling.slope = Some(framer_core::CeilingSlope::new(pitch_9_12(), 0));
        let plan = generate_ceiling_plan(
            &ceiling,
            &ceiling_system(),
            region_points(&ceiling.region),
            Length::from_feet(8.0),
            &materials(),
        )
        .unwrap();

        assert!(
            plan.members
                .iter()
                .filter(|member| member.kind == MemberKind::CeilingJoist)
                .all(|member| member.sloped.is_none()),
            "the fallback frames flat ceiling joists (no sloped placement)"
        );
        assert!(
            plan.diagnostics.iter().any(|d| {
                d.code == "ceiling.outline.degenerate" && d.severity == DiagnosticSeverity::Warning
            }),
            "the dropped slope is surfaced as a warning, not silent"
        );
    }

    #[test]
    fn floor_bom_groups_joists_by_profile_kind_and_length() {
        let deck = FloorDeck::new(
            "deck",
            "Deck",
            "level-1",
            "system-floor",
            rect_region(20.0, 12.0),
        );
        let plan = generate_floor_plan(
            &deck,
            &floor_system(),
            region_points(&deck.region),
            &materials(),
        )
        .unwrap();

        let joist_row = plan
            .bom()
            .into_iter()
            .find(|item| item.kind == MemberKind::FloorJoist)
            .expect("a floor-joist BOM row");
        assert_eq!(joist_row.profile, BoardProfile::TwoByEight);
        assert_eq!(joist_row.cut_length, Length::from_feet(12.0));
        assert_eq!(joist_row.quantity, 16);
        assert_eq!(joist_row.total_length, Length::from_feet(12.0) * 16);
    }

    #[test]
    fn floor_layer_takeoff_matches_region_area() {
        let deck = FloorDeck::new(
            "deck",
            "Deck",
            "level-1",
            "system-floor",
            rect_region(20.0, 12.0),
        );
        let plan = generate_floor_plan(
            &deck,
            &floor_system(),
            region_points(&deck.region),
            &materials(),
        )
        .unwrap();

        // 20ft × 12ft = 240in × 144in = 34_560 sq in of subfloor, no volume.
        let subfloor = find_layer(&plan.layers, "mat-plywood");
        assert_eq!(subfloor.area_sq_in, 240 * 144);
        assert_eq!(subfloor.volume_bd_in, 0);
        // Joist lumber stays in the member BOM, not the layer takeoff.
        assert!(
            !plan
                .layers
                .iter()
                .any(|layer| layer.material.0 == "mat-spf")
        );
    }

    #[test]
    fn floor_deck_emits_span_not_checked_diagnostic() {
        let deck = FloorDeck::new(
            "deck",
            "Deck",
            "level-1",
            "system-floor",
            rect_region(20.0, 12.0),
        );
        let plan = generate_floor_plan(
            &deck,
            &floor_system(),
            region_points(&deck.region),
            &materials(),
        )
        .unwrap();

        assert!(plan.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "floor.span.not-checked"
                && diagnostic.severity == DiagnosticSeverity::Info
                && diagnostic.source.as_ref().map(|id| id.0.as_str()) == Some("deck")
        }));
    }

    #[test]
    fn project_plan_frames_floor_and_ceiling_over_a_room() {
        let mut model = rectangle_with_room();
        model.systems.push(floor_system());
        model.systems.push(ceiling_system());
        model.floor_decks.push(FloorDeck::new(
            "deck-1",
            "Deck",
            "level-1",
            "system-floor",
            SurfaceRegion::Room(ElementId::new("room-1")),
        ));
        // The ceiling covers a differently-proportioned region than the floor so
        // a deck<->ceiling outline mix-up in the batch resolver would be caught:
        // the floor (the 12ft x 8ft room) spans 8ft, the ceiling (a 20ft x 14ft
        // polygon) spans 14ft.
        model.ceilings.push(Ceiling::new(
            "ceiling-1",
            "Ceiling",
            "level-1",
            "system-ceiling",
            rect_region(20.0, 14.0),
            Length::from_feet(8.0),
        ));

        let plan = generate_project_plan(&model).unwrap();

        assert_eq!(plan.floor_plans.len(), 1);
        assert_eq!(plan.ceiling_plans.len(), 1);

        let floor = plan.floor_plan(&ElementId::new("deck-1")).unwrap();
        let floor_joists: Vec<_> = floor
            .members
            .iter()
            .filter(|member| member.kind == MemberKind::FloorJoist)
            .collect();
        assert!(!floor_joists.is_empty());
        // The room is 12ft × 8ft; floor joists span the shorter 8ft dimension.
        assert!(
            floor_joists
                .iter()
                .all(|member| member.cut_length == Length::from_feet(8.0))
        );

        // The ceiling joists span its own 14ft shorter dimension, not the floor's.
        let ceiling = plan.ceiling_plan(&ElementId::new("ceiling-1")).unwrap();
        let ceiling_joists: Vec<_> = ceiling
            .members
            .iter()
            .filter(|member| member.kind == MemberKind::CeilingJoist)
            .collect();
        assert!(!ceiling_joists.is_empty());
        assert!(
            ceiling_joists
                .iter()
                .all(|member| member.cut_length == Length::from_feet(14.0))
        );

        // The project BOM folds in the new surfaces' members.
        let bom = plan.bom();
        assert!(bom.iter().any(|item| item.kind == MemberKind::FloorJoist));
        assert!(bom.iter().any(|item| item.kind == MemberKind::CeilingJoist));

        // The deck's own subfloor takeoff covers the 96 sq ft room (144in × 96in
        // = 13_824 sq in); the project total also folds it into the shared
        // plywood row alongside the wall sheathing.
        assert_eq!(
            find_layer(&floor.layers, "mat-plywood").area_sq_in,
            144 * 96
        );
        let project_plywood: i64 = plan
            .layers
            .iter()
            .filter(|layer| layer.material.0 == "mat-plywood")
            .map(|layer| layer.area_sq_in)
            .sum();
        assert!(project_plywood >= 144 * 96);
    }

    #[test]
    fn room_surface_regions_resolve_against_the_room_level() {
        use framer_core::{Level, Point2, Room, RoomUsage};
        let mut model = rectangle_with_room();
        model
            .levels
            .push(Level::new("level-2", "Level 2", Length::from_feet(10.0)));
        model.systems.push(floor_system());
        model.rooms.push(Room::new(
            "room-2",
            "Unenclosed upper room",
            RoomUsage::Living,
            "level-2",
            Point2::new(Length::from_feet(6.0), Length::from_feet(4.0)),
        ));
        model.floor_decks.push(FloorDeck::new(
            "deck-2",
            "Upper deck",
            "level-2",
            "system-floor",
            SurfaceRegion::Room(ElementId::new("room-2")),
        ));

        let plan = generate_project_plan(&model).unwrap();

        let deck = plan.floor_plan(&ElementId::new("deck-2")).unwrap();
        assert!(
            deck.members.is_empty(),
            "a level-2 room region must not frame over the level-1 rectangle"
        );
        assert!(deck.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "floor.boundary.open"
                && diagnostic.source.as_ref().map(|id| id.0.as_str()) == Some("deck-2")
        }));
    }

    /// The shipped `demo-shell` example (capped with a hip roof, a scissor-vault
    /// ceiling, and a floor deck) frames end-to-end: all four roof planes rafter,
    /// the hip post-pass adds hips/jacks and the shortened ridge, the two sloped
    /// vault halves joist, the deck joists, and every new member family folds
    /// into the project BOM.
    #[test]
    fn roofed_demo_shell_example_frames_every_surface() {
        let example = include_str!("../../../examples/projects/demo-shell.framer");
        let model = load_project(example).unwrap();
        let plan = generate_project_plan(&model).unwrap();

        assert_eq!(plan.roof_plans.len(), 4);
        // A scissor vault is two opposing sloped ceilings.
        assert_eq!(plan.ceiling_plans.len(), 2);
        assert_eq!(plan.floor_plans.len(), 1);

        for id in ["roof-east", "roof-north", "roof-south", "roof-west"] {
            let roof = plan.roof_plan(&ElementId::new(id)).unwrap();
            assert!(
                roof.members.iter().any(|member| matches!(
                    member.kind,
                    MemberKind::Rafter | MemberKind::JackRafter
                )),
                "{id} frames rafters or clipped jack rafters"
            );
        }
        let ridges = plan
            .roof_plans
            .iter()
            .flat_map(|roof| roof.members.iter())
            .filter(|member| member.kind == MemberKind::RidgeBoard)
            .count();
        assert_eq!(ridges, 1, "a rectangular hip shares one shortened ridge");

        let hips = plan
            .roof_plans
            .iter()
            .flat_map(|roof| roof.members.iter())
            .filter(|member| member.kind == MemberKind::HipRafter)
            .count();
        assert_eq!(hips, 4, "a rectangular hip roof has one hip per corner");
        let jacks = plan
            .roof_plans
            .iter()
            .flat_map(|roof| roof.members.iter())
            .filter(|member| member.kind == MemberKind::JackRafter)
            .count();
        assert!(jacks > 0, "hip-bounded rafters clip into jacks");

        // The scissor vault is sloped, so it is NOT a flat rafter tie at the plate:
        // the hip roof's shortened ridge still needs structural-ridge judgment.
        // Pins the product-visible tie fork through the real load_project +
        // region-resolution + elevation path (the synthetic tests hand-set those
        // inputs).
        let roof_diagnostics = || {
            plan.roof_plans
                .iter()
                .flat_map(|roof| roof.diagnostics.iter())
        };
        assert!(
            roof_diagnostics().any(|diagnostic| diagnostic.code == "roof.ridge.beam-required"
                && diagnostic.severity == DiagnosticSeverity::Unsupported),
            "a vaulted shell has no flat tie, so the ridge needs a beam"
        );
        assert!(
            roof_diagnostics().all(|diagnostic| diagnostic.code != "roof.ridge.tied"),
            "a vaulted shell's ridge is not reported as tied"
        );

        // Each vault half frames sloped ceiling joists and carries the scissor note.
        for id in ["ceiling-1", "ceiling-2"] {
            let ceiling = plan.ceiling_plan(&ElementId::new(id)).unwrap();
            assert!(
                ceiling
                    .members
                    .iter()
                    .any(|member| member.kind == MemberKind::CeilingJoist),
                "{id} frames ceiling joists"
            );
            assert!(
                ceiling
                    .members
                    .iter()
                    .filter(|member| member.kind == MemberKind::CeilingJoist)
                    .all(|member| member.sloped.is_some()),
                "{id} is a sloped (vault) ceiling"
            );
            assert!(
                ceiling
                    .diagnostics
                    .iter()
                    .any(|d| d.code == "ceiling.slope.scissor"),
                "{id} reports the scissor condition"
            );
        }
        assert!(
            plan.floor_plan(&ElementId::new("deck-1"))
                .unwrap()
                .members
                .iter()
                .any(|member| member.kind == MemberKind::FloorJoist)
        );

        let bom = plan.bom();
        for kind in [
            MemberKind::Rafter,
            MemberKind::RidgeBoard,
            MemberKind::HipRafter,
            MemberKind::JackRafter,
            MemberKind::CeilingJoist,
            MemberKind::FloorJoist,
        ] {
            assert!(
                bom.iter().any(|item| item.kind == kind),
                "{kind:?} appears in the project BOM"
            );
        }
    }

    #[test]
    fn floor_deck_over_open_room_emits_boundary_open_diagnostic_and_no_members() {
        use framer_core::{Room, RoomUsage};
        let code = FramingDefaults::irc_2021_starter();
        let mut model = BuildingModel::new();
        // A single wall never encloses a loop, so the room stays open.
        model
            .walls
            .push(Wall::new("w-1", "Wall", Length::from_feet(12.0), &code));
        model.rooms.push(Room::new(
            "room-1",
            "Room",
            RoomUsage::Unspecified,
            "level-1",
            Point2::new(Length::from_feet(2.0), Length::from_feet(2.0)),
        ));
        model.systems.push(floor_system());
        model.floor_decks.push(FloorDeck::new(
            "deck-1",
            "Deck",
            "level-1",
            "system-floor",
            SurfaceRegion::Room(ElementId::new("room-1")),
        ));

        let plan = generate_project_plan(&model).unwrap();
        let floor = plan.floor_plan(&ElementId::new("deck-1")).unwrap();

        assert!(floor.members.is_empty(), "an open region frames no joists");
        assert!(floor.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "floor.boundary.open"
                && diagnostic.severity == DiagnosticSeverity::Warning
                && diagnostic.source.as_ref().map(|id| id.0.as_str()) == Some("deck-1")
        }));
    }

    #[test]
    fn ceiling_over_open_room_emits_boundary_open_diagnostic_and_no_members() {
        use framer_core::{Room, RoomUsage};
        let code = FramingDefaults::irc_2021_starter();
        let mut model = BuildingModel::new();
        // A single wall never encloses a loop, so the room stays open.
        model
            .walls
            .push(Wall::new("w-1", "Wall", Length::from_feet(12.0), &code));
        model.rooms.push(Room::new(
            "room-1",
            "Room",
            RoomUsage::Unspecified,
            "level-1",
            Point2::new(Length::from_feet(2.0), Length::from_feet(2.0)),
        ));
        model.systems.push(ceiling_system());
        model.ceilings.push(Ceiling::new(
            "ceiling-1",
            "Ceiling",
            "level-1",
            "system-ceiling",
            SurfaceRegion::Room(ElementId::new("room-1")),
            Length::from_feet(8.0),
        ));

        let plan = generate_project_plan(&model).unwrap();
        let ceiling = plan.ceiling_plan(&ElementId::new("ceiling-1")).unwrap();

        assert!(
            ceiling.members.is_empty(),
            "an open region frames no joists"
        );
        assert!(ceiling.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "ceiling.boundary.open"
                && diagnostic.severity == DiagnosticSeverity::Warning
                && diagnostic.source.as_ref().map(|id| id.0.as_str()) == Some("ceiling-1")
        }));
    }

    /// A roof system: fiber-cement roofing over plywood sheathing over 2x8
    /// rafters at 16" o.c.
    fn roof_system() -> ConstructionSystem {
        ConstructionSystem {
            id: ElementId::new("system-roof"),
            name: "Roof".to_owned(),
            kind: framer_core::SystemKind::Roof,
            source: None,
            layers: vec![
                framer_core::ConstructionLayer::new(
                    LayerFunction::Roofing,
                    "mat-fiber-cement",
                    Length::from_inches(0.25),
                ),
                framer_core::ConstructionLayer::new(
                    LayerFunction::Sheathing,
                    "mat-plywood",
                    Length::from_inches(0.5),
                ),
                framer_core::ConstructionLayer::new(
                    LayerFunction::Framing,
                    "mat-spf",
                    BoardProfile::TwoByEight.nominal_depth(),
                )
                .with_framing(FramingSpec {
                    member: BoardProfile::TwoByEight,
                    spacing: Length::from_whole_inches(16),
                    pattern: framer_core::FramingPattern::Single,
                    member_family: framer_core::MemberFamily::Rafter,
                    cavity_material: None,
                }),
            ],
        }
    }

    /// A counter-clockwise rectangular outline anchored at the origin (in feet).
    fn rect_outline(w_ft: f64, d_ft: f64) -> Vec<Point2> {
        let (w, d) = (Length::from_feet(w_ft), Length::from_feet(d_ft));
        vec![
            Point2::new(Length::ZERO, Length::ZERO),
            Point2::new(w, Length::ZERO),
            Point2::new(w, d),
            Point2::new(Length::ZERO, d),
        ]
    }

    /// A single shed roof plane: a 24ft (eave) × 12ft (run) rectangle on the
    /// world axes, eave along the bottom edge, springing at 8ft, `slope` pitch.
    fn shed_plane(slope: Slope) -> RoofPlane {
        RoofPlane::new(
            "roof-shed",
            "Shed",
            "level-1",
            "system-roof",
            rect_outline(24.0, 12.0),
            slope,
            0,
            Length::from_feet(8.0),
        )
    }

    /// The 9:12 pitch — a 3-4-5 triangle over a 12ft run (144in run, 108in rise,
    /// 180in hypotenuse), so true sloped lengths land on exact ticks.
    fn pitch_9_12() -> Slope {
        Slope::new(Length::from_whole_inches(9), Length::from_whole_inches(12))
    }

    fn pitch_6_12() -> Slope {
        Slope::new(Length::from_whole_inches(6), Length::from_whole_inches(12))
    }

    #[test]
    fn roof_plane_arrays_rafters_at_true_sloped_length() {
        let plane = shed_plane(pitch_9_12());
        let plan = generate_roof_plan(
            &plane,
            &roof_system(),
            &materials(),
            false,
            RidgeTie::Untied,
        )
        .unwrap();

        let rafters: Vec<_> = plan
            .members
            .iter()
            .filter(|member| member.kind == MemberKind::Rafter)
            .collect();

        // One rafter per layout mark along the 24ft eave at 16" o.c.
        let expected = stud_positions(
            Length::from_feet(24.0),
            Length::from_whole_inches(16),
            BoardProfile::TwoByEight.thickness(),
        )
        .len();
        assert_eq!(rafters.len(), expected);
        assert!(expected > 2, "an interior rafter array is laid out");

        // Cut length is the true sloped length: a 12ft plan run at 9:12 is the
        // 15ft hypotenuse of the 3-4-5 triangle (plan run < cut length).
        assert!(
            rafters
                .iter()
                .all(|member| member.cut_length == Length::from_feet(15.0)),
            "rafters are cut to the true sloped 15ft length"
        );
        assert!(
            rafters
                .iter()
                .all(|member| member.profile == BoardProfile::TwoByEight
                    && member.orientation == MemberOrientation::Vertical),
            "rafters use the system framing member and run up the slope"
        );

        // The sloped placement records the eave springing (8ft) and the ridge
        // (8ft + 108in rise = 17ft), and recovers the 12ft plan run by Pythagoras.
        let rafter = rafters[0];
        let slope = rafter.sloped.expect("a rafter carries a sloped placement");
        assert_eq!(slope.low_elevation, Length::from_feet(8.0));
        assert_eq!(slope.high_elevation, Length::from_whole_inches(96 + 108));
        assert_eq!(
            rafter.elevation,
            Length::ZERO,
            "no overhang: tail at the eave"
        );
        let rise = slope.high_elevation - slope.low_elevation;
        let plan_run = (rafter.cut_length.inches().powi(2) - rise.inches().powi(2)).sqrt();
        assert!(
            (plan_run - Length::from_feet(12.0).inches()).abs() < 0.01,
            "the plan run recovers the 12ft horizontal extent"
        );

        // No ridge board for a plane that does not carry one; plate blocking is
        // generated between rafters.
        assert!(
            plan.members
                .iter()
                .all(|member| member.kind != MemberKind::RidgeBoard)
        );
        assert!(
            plan.members
                .iter()
                .any(|member| member.kind == MemberKind::Blocking)
        );

        // The span is laid out geometrically, surfaced as an Info diagnostic.
        assert!(plan.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "roof.span.not-checked"
                && diagnostic.severity == DiagnosticSeverity::Info
                && diagnostic.source.as_ref().map(|id| id.0.as_str()) == Some("roof-shed")
        }));
    }

    #[test]
    fn flat_roof_plane_rafter_cut_equals_plan_run() {
        let plane = shed_plane(Slope::flat());
        let plan = generate_roof_plan(
            &plane,
            &roof_system(),
            &materials(),
            false,
            RidgeTie::Untied,
        )
        .unwrap();

        let rafter = plan
            .members
            .iter()
            .find(|member| member.kind == MemberKind::Rafter)
            .expect("a rafter");
        // A flat plane: cut length equals the 12ft plan run and the slope is level.
        assert_eq!(rafter.cut_length, Length::from_feet(12.0));
        let slope = rafter
            .sloped
            .expect("flat rafters still record their elevation");
        assert_eq!(slope.low_elevation, Length::from_feet(8.0));
        assert_eq!(slope.high_elevation, Length::from_feet(8.0));
    }

    #[test]
    fn roof_eave_overhang_lengthens_rafter_and_drops_the_tail() {
        let plane = shed_plane(pitch_9_12()).with_eave_overhang(Length::from_whole_inches(12));
        let plan = generate_roof_plan(
            &plane,
            &roof_system(),
            &materials(),
            false,
            RidgeTie::Untied,
        )
        .unwrap();

        let rafter = plan
            .members
            .iter()
            .find(|member| member.kind == MemberKind::Rafter)
            .expect("a rafter");
        // The 12in overhang adds a sloped tail: 156in plan run × 1.25 = 195in cut.
        assert_eq!(rafter.cut_length, Length::from_whole_inches(195));
        assert_eq!(
            rafter.elevation,
            Length::ZERO - Length::from_whole_inches(12)
        );
        let slope = rafter.sloped.expect("a sloped placement");
        // The tail drops a 12in run × 0.75 = 9in below the 8ft springing.
        assert_eq!(slope.low_elevation, Length::from_whole_inches(96 - 9));
        assert_eq!(slope.high_elevation, Length::from_whole_inches(96 + 108));
    }

    /// A gable over a 24ft × 24ft footprint: two opposing 12ft-run planes sharing
    /// a ridge at y = 12ft. `roof-a` (lower id) carries the shared ridge board.
    fn gable_model() -> BuildingModel {
        let mut model = BuildingModel::new();
        model.systems.push(roof_system());
        let (w, mid, far) = (
            Length::from_feet(24.0),
            Length::from_feet(12.0),
            Length::from_feet(24.0),
        );
        model.roof_planes.push(RoofPlane::new(
            "roof-a",
            "South slope",
            "level-1",
            "system-roof",
            vec![
                Point2::new(Length::ZERO, Length::ZERO),
                Point2::new(w, Length::ZERO),
                Point2::new(w, mid),
                Point2::new(Length::ZERO, mid),
            ],
            pitch_9_12(),
            0,
            Length::from_feet(8.0),
        ));
        model.roof_planes.push(RoofPlane::new(
            "roof-b",
            "North slope",
            "level-1",
            "system-roof",
            vec![
                Point2::new(Length::ZERO, far),
                Point2::new(w, far),
                Point2::new(w, mid),
                Point2::new(Length::ZERO, mid),
            ],
            pitch_9_12(),
            0,
            Length::from_feet(8.0),
        ));
        model
    }

    fn rectangular_hip_model() -> BuildingModel {
        let mut model = BuildingModel::new();
        model.systems.push(roof_system());
        let (w, d, mid_y, inset) = (
            Length::from_feet(24.0),
            Length::from_feet(12.0),
            Length::from_feet(6.0),
            Length::from_feet(6.0),
        );
        let ridge_west = Point2::new(inset, mid_y);
        let ridge_east = Point2::new(w - inset, mid_y);
        let slope = pitch_6_12();
        let springing = Length::from_feet(8.0);
        model.roof_planes.push(RoofPlane::new(
            "roof-south",
            "South hip field",
            "level-1",
            "system-roof",
            vec![
                Point2::new(Length::ZERO, Length::ZERO),
                Point2::new(w, Length::ZERO),
                ridge_east,
                ridge_west,
            ],
            slope,
            0,
            springing,
        ));
        model.roof_planes.push(RoofPlane::new(
            "roof-north",
            "North hip field",
            "level-1",
            "system-roof",
            vec![
                Point2::new(w, d),
                Point2::new(Length::ZERO, d),
                ridge_west,
                ridge_east,
            ],
            slope,
            0,
            springing,
        ));
        model.roof_planes.push(RoofPlane::new(
            "roof-east",
            "East hip end",
            "level-1",
            "system-roof",
            vec![Point2::new(w, Length::ZERO), Point2::new(w, d), ridge_east],
            slope,
            0,
            springing,
        ));
        model.roof_planes.push(RoofPlane::new(
            "roof-west",
            "West hip end",
            "level-1",
            "system-roof",
            vec![
                Point2::new(Length::ZERO, d),
                Point2::new(Length::ZERO, Length::ZERO),
                ridge_west,
            ],
            slope,
            0,
            springing,
        ));
        model
    }

    fn square_hip_model() -> BuildingModel {
        let mut model = BuildingModel::new();
        model.systems.push(roof_system());
        let (side, mid) = (Length::from_feet(20.0), Length::from_feet(10.0));
        let peak = Point2::new(mid, mid);
        let slope = pitch_6_12();
        let springing = Length::from_feet(8.0);
        model.roof_planes.push(RoofPlane::new(
            "roof-south",
            "South hip triangle",
            "level-1",
            "system-roof",
            vec![
                Point2::new(Length::ZERO, Length::ZERO),
                Point2::new(side, Length::ZERO),
                peak,
            ],
            slope,
            0,
            springing,
        ));
        model.roof_planes.push(RoofPlane::new(
            "roof-east",
            "East hip triangle",
            "level-1",
            "system-roof",
            vec![
                Point2::new(side, Length::ZERO),
                Point2::new(side, side),
                peak,
            ],
            slope,
            0,
            springing,
        ));
        model.roof_planes.push(RoofPlane::new(
            "roof-north",
            "North hip triangle",
            "level-1",
            "system-roof",
            vec![
                Point2::new(side, side),
                Point2::new(Length::ZERO, side),
                peak,
            ],
            slope,
            0,
            springing,
        ));
        model.roof_planes.push(RoofPlane::new(
            "roof-west",
            "West hip triangle",
            "level-1",
            "system-roof",
            vec![
                Point2::new(Length::ZERO, side),
                Point2::new(Length::ZERO, Length::ZERO),
                peak,
            ],
            slope,
            0,
            springing,
        ));
        model
    }

    /// An equal-pitch L-footprint valley: the wall loop has one reentrant corner
    /// at (12ft, 12ft), and two stored planes share the diagonal valley from the
    /// outside corner to that reentrant corner. Each plane clips its rafters
    /// against the diagonal, producing jack rafters that die into the valley.
    fn l_valley_model() -> BuildingModel {
        let code = FramingDefaults::irc_2021_starter();
        let mut model = BuildingModel::new();
        model.systems.push(roof_system());
        let p = |x: f64, y: f64| Point2::new(Length::from_feet(x), Length::from_feet(y));
        let footprint = [
            p(0.0, 0.0),
            p(24.0, 0.0),
            p(24.0, 12.0),
            p(12.0, 12.0),
            p(12.0, 24.0),
            p(0.0, 24.0),
            p(0.0, 0.0),
        ];
        for (index, pair) in footprint.windows(2).enumerate() {
            model.walls.push(
                Wall::new(
                    format!("wall-l-{index}"),
                    format!("L footprint wall {index}"),
                    Length::from_feet(1.0),
                    &code,
                )
                .with_placement("level-1", pair[0], pair[1]),
            );
        }
        let slope = pitch_6_12();
        let springing = Length::from_feet(8.0);
        model.roof_planes.push(RoofPlane::new(
            "roof-a",
            "Lower L-wing valley slope",
            "level-1",
            "system-roof",
            vec![p(0.0, 0.0), p(24.0, 0.0), p(24.0, 12.0), p(12.0, 12.0)],
            slope,
            0,
            springing,
        ));
        model.roof_planes.push(RoofPlane::new(
            "roof-b",
            "Upper L-wing valley slope",
            "level-1",
            "system-roof",
            vec![p(0.0, 0.0), p(0.0, 24.0), p(12.0, 24.0), p(12.0, 12.0)],
            slope,
            0,
            springing,
        ));
        model
    }

    #[test]
    fn gable_with_no_ceiling_tie_emits_ridge_beam_required() {
        let plan = generate_project_plan(&gable_model()).unwrap();
        assert_eq!(plan.roof_plans.len(), 2);

        // Both slopes frame rafters running the 12ft half-span (15ft true length).
        for id in ["roof-a", "roof-b"] {
            let roof = plan.roof_plan(&ElementId::new(id)).unwrap();
            assert!(
                roof.members
                    .iter()
                    .filter(|member| member.kind == MemberKind::Rafter)
                    .all(|member| member.cut_length == Length::from_feet(15.0)),
                "{id} rafters are the 15ft true sloped length"
            );
        }

        // Exactly one ridge board across the whole gable, on the lower-id plane,
        // running the full 24ft ridge.
        let ridges: Vec<_> = plan
            .roof_plans
            .iter()
            .flat_map(|roof| roof.members.iter())
            .filter(|member| member.kind == MemberKind::RidgeBoard)
            .collect();
        assert_eq!(ridges.len(), 1, "a gable's shared ridge is counted once");
        assert_eq!(ridges[0].cut_length, Length::from_feet(24.0));
        assert!(
            plan.roof_plan(&ElementId::new("roof-a"))
                .unwrap()
                .members
                .iter()
                .any(|member| member.kind == MemberKind::RidgeBoard),
            "the lower-id plane carries the ridge"
        );

        // With no flat ceiling tying the rafters, the ridge-carrying plane reports
        // that a structural ridge beam is required (an Unsupported judgment).
        let roof_a = plan.roof_plan(&ElementId::new("roof-a")).unwrap();
        assert!(
            roof_a
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "roof.ridge.beam-required"
                    && diagnostic.severity == DiagnosticSeverity::Unsupported)
        );
        assert!(
            roof_a
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.code != "roof.ridge.tied"),
            "an untied gable is not also reported as tied"
        );

        // The project BOM and layer takeoff fold in the roof members and roofing.
        let bom = plan.bom();
        assert!(bom.iter().any(|item| item.kind == MemberKind::Rafter));
        assert!(bom.iter().any(|item| item.kind == MemberKind::RidgeBoard));
        // Each 24ft × 12ft slope contributes 288in × 144in of plan roofing area.
        let roofing: i64 = plan
            .layers
            .iter()
            .filter(|layer| layer.function == LayerFunction::Roofing)
            .map(|layer| layer.area_sq_in)
            .sum();
        assert_eq!(roofing, 2 * 288 * 144);
    }

    #[test]
    fn rectangular_hip_roof_frames_hips_and_a_shortened_ridge() {
        let plan = generate_project_plan(&rectangular_hip_model()).unwrap();
        assert_eq!(plan.roof_plans.len(), 4);

        let hips: Vec<_> = plan
            .roof_plans
            .iter()
            .flat_map(|roof| roof.members.iter())
            .filter(|member| member.kind == MemberKind::HipRafter)
            .collect();
        assert_eq!(hips.len(), 4, "one hip rafter per corner-to-ridge edge");
        assert!(
            hips.iter()
                .all(|member| member.cut_length == Length::from_feet(9.0)),
            "a 6ft by 6ft plan hip at 6:12 has a true 9ft cut length"
        );
        assert!(
            hips.iter().all(|member| {
                member.sloped
                    == Some(SlopedPlacement {
                        low_elevation: Length::from_feet(8.0),
                        high_elevation: Length::from_feet(11.0),
                    })
            }),
            "hips run from the plate corner to the ridge end"
        );

        let ridges: Vec<_> = plan
            .roof_plans
            .iter()
            .flat_map(|roof| roof.members.iter())
            .filter(|member| member.kind == MemberKind::RidgeBoard)
            .collect();
        assert_eq!(
            ridges.len(),
            1,
            "the two trapezoids share one central ridge"
        );
        assert_eq!(
            ridges[0].cut_length,
            Length::from_feet(12.0),
            "ridge length is the footprint length minus the two hip insets"
        );
        assert_eq!(
            ridges[0].sloped,
            Some(SlopedPlacement {
                low_elevation: Length::from_feet(11.0),
                high_elevation: Length::from_feet(11.0),
            })
        );

        let hip_bom = plan
            .bom()
            .into_iter()
            .find(|item| {
                item.kind == MemberKind::HipRafter && item.cut_length == Length::from_feet(9.0)
            })
            .expect("hip rafters fold into the project BOM");
        assert_eq!(hip_bom.quantity, 4);
    }

    #[test]
    fn rectangular_hip_roof_replaces_overrunning_common_rafters_with_jacks() {
        let plan = generate_project_plan(&rectangular_hip_model()).unwrap();
        let south = plan.roof_plan(&ElementId::new("roof-south")).unwrap();

        let ridge_start = Length::from_feet(6.0);
        let ridge_end = Length::from_feet(18.0);
        let full_run = Length::from_feet(6.0);
        let full_cut = Length::from_inches(full_run.inches() * slope_factor(pitch_6_12()));
        let ridge_elevation = Length::from_feet(11.0);

        let common: Vec<_> = south
            .members
            .iter()
            .filter(|member| member.kind == MemberKind::Rafter)
            .collect();
        assert!(
            !common.is_empty(),
            "the central ridge span still frames common rafters"
        );
        assert!(
            common.iter().all(|member| {
                member.x >= ridge_start
                    && member.x <= ridge_end
                    && member.cut_length == full_cut
                    && member
                        .sloped
                        .is_some_and(|slope| slope.high_elevation == ridge_elevation)
            }),
            "only rafters under the central ridge run the full common length"
        );

        let jacks: Vec<_> = south
            .members
            .iter()
            .filter(|member| member.kind == MemberKind::JackRafter)
            .collect();
        assert!(!jacks.is_empty(), "hip-bounded marks become jack rafters");
        assert!(
            jacks.iter().all(|member| {
                member.cut_length < full_cut
                    && member
                        .sloped
                        .is_some_and(|slope| slope.high_elevation < ridge_elevation)
            }),
            "jack rafters stop at the hip instead of overrunning to the ridge"
        );

        let right_jacks: Vec<_> = jacks
            .iter()
            .copied()
            .filter(|member| member.x > ridge_end)
            .collect();
        assert!(right_jacks.len() >= 2);
        for pair in right_jacks.windows(2) {
            assert!(
                pair[0].cut_length > pair[1].cut_length,
                "jack cut lengths descend toward the hip corner"
            );
        }
        for jack in right_jacks {
            let expected_run = Length::from_feet(24.0) - jack.x;
            let expected_high =
                Length::from_feet(8.0) + Length::from_inches(expected_run.inches() * 0.5);
            let expected_cut =
                Length::from_inches(expected_run.inches() * slope_factor(pitch_6_12()));
            assert_eq!(jack.sloped.unwrap().high_elevation, expected_high);
            assert_eq!(jack.cut_length, expected_cut);
        }

        let jack_bom = plan
            .bom()
            .into_iter()
            .find(|item| item.kind == MemberKind::JackRafter)
            .expect("jack rafters fold into the project BOM");
        assert!(jack_bom.quantity > 0);
    }

    #[test]
    fn square_hip_roof_frames_hips_without_a_ridge() {
        let plan = generate_project_plan(&square_hip_model()).unwrap();
        assert_eq!(plan.roof_plans.len(), 4);

        let hips: Vec<_> = plan
            .roof_plans
            .iter()
            .flat_map(|roof| roof.members.iter())
            .filter(|member| member.kind == MemberKind::HipRafter)
            .collect();
        assert_eq!(hips.len(), 4, "one hip rafter per corner-to-peak edge");
        assert!(
            hips.iter()
                .all(|member| member.cut_length == Length::from_feet(15.0)),
            "a 10ft by 10ft plan hip at 6:12 has a true 15ft cut length"
        );
        assert!(
            hips.iter().all(|member| {
                member.sloped
                    == Some(SlopedPlacement {
                        low_elevation: Length::from_feet(8.0),
                        high_elevation: Length::from_feet(13.0),
                    })
            }),
            "hips run from the plate corner to the shared peak"
        );

        let ridges: Vec<_> = plan
            .roof_plans
            .iter()
            .flat_map(|roof| roof.members.iter())
            .filter(|member| member.kind == MemberKind::RidgeBoard)
            .collect();
        assert!(
            ridges.is_empty(),
            "a square pyramidal hip has no horizontal ridge board"
        );

        let hip_bom = plan
            .bom()
            .into_iter()
            .find(|item| {
                item.kind == MemberKind::HipRafter && item.cut_length == Length::from_feet(15.0)
            })
            .expect("square hip rafters fold into the project BOM");
        assert_eq!(hip_bom.quantity, 4);
    }

    #[test]
    fn square_hip_roof_frames_jacks_without_overrunning_common_rafters() {
        let plan = generate_project_plan(&square_hip_model()).unwrap();

        for roof in &plan.roof_plans {
            assert!(
                roof.members
                    .iter()
                    .all(|member| member.kind != MemberKind::Rafter),
                "a pyramidal hip has no central ridge span for common rafters"
            );
            let jacks: Vec<_> = roof
                .members
                .iter()
                .filter(|member| member.kind == MemberKind::JackRafter)
                .collect();
            assert!(
                !jacks.is_empty(),
                "each square hip plane frames jack rafters against its hips"
            );
            assert!(
                jacks.iter().all(|member| {
                    member
                        .sloped
                        .is_some_and(|slope| slope.high_elevation < Length::from_feet(13.0))
                }),
                "no jack overruns the hip to the shared peak"
            );
        }
    }

    #[test]
    fn equal_pitch_l_roof_frames_valley_and_symmetric_jacks() {
        let plan = generate_project_plan(&l_valley_model()).unwrap();
        assert_eq!(plan.roof_plans.len(), 2);

        let valleys: Vec<_> = plan
            .roof_plans
            .iter()
            .flat_map(|roof| roof.members.iter())
            .filter(|member| member.kind == MemberKind::ValleyRafter)
            .collect();
        assert_eq!(valleys.len(), 1, "the L footprint has one shared valley");
        assert_eq!(
            valleys[0].cut_length,
            Length::from_feet(18.0),
            "a 12ft by 12ft plan valley at 6:12 cuts to an 18ft true length"
        );
        assert_eq!(
            valleys[0].sloped,
            Some(SlopedPlacement {
                low_elevation: Length::from_feet(8.0),
                high_elevation: Length::from_feet(14.0),
            })
        );
        assert!(
            plan.roof_plans
                .iter()
                .flat_map(|roof| roof.members.iter())
                .all(|member| member.kind != MemberKind::HipRafter),
            "the reentrant shared edge is not misclassified as a hip"
        );

        for roof_id in ["roof-a", "roof-b"] {
            let roof = plan.roof_plan(&ElementId::new(roof_id)).unwrap();
            let jacks: Vec<_> = roof
                .members
                .iter()
                .filter(|member| member.kind == MemberKind::JackRafter)
                .collect();
            assert!(
                !jacks.is_empty(),
                "{roof_id} clips common rafters into valley jacks"
            );
            assert!(
                jacks.iter().all(|jack| jack
                    .sloped
                    .is_some_and(|slope| slope.high_elevation < Length::from_feet(14.0))),
                "{roof_id} jacks stop on the valley before the full ridge height"
            );
        }

        let valley_bom = plan
            .bom()
            .into_iter()
            .find(|item| item.kind == MemberKind::ValleyRafter)
            .expect("valley rafters fold into the project BOM");
        assert_eq!(valley_bom.quantity, 1);
        assert!(
            plan.roof_plans.iter().all(|roof| {
                roof.diagnostics
                    .iter()
                    .all(|diagnostic| diagnostic.code != "roof.valley.unequal-pitch")
            }),
            "equal-pitch valleys do not emit the unsupported unequal-pitch diagnostic"
        );
    }

    #[test]
    fn unequal_pitch_l_roof_valley_is_diagnosed_not_framed() {
        let mut model = l_valley_model();
        model.roof_planes[1].slope = pitch_9_12();
        let plan = generate_project_plan(&model).unwrap();

        assert!(
            plan.roof_plans
                .iter()
                .flat_map(|roof| roof.members.iter())
                .all(|member| member.kind != MemberKind::ValleyRafter),
            "unequal-pitch valley edges are diagnosed rather than framed"
        );
        assert!(plan.roof_plans.iter().any(|roof| {
            roof.diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "roof.valley.unequal-pitch"
                    && diagnostic.severity == DiagnosticSeverity::Unsupported
            })
        }));
    }

    /// The tied-gable fixture: `gable_model` with the level top raised to the roof
    /// springing (8ft) and a flat ceiling covering the whole 24ft × 24ft footprint
    /// at the plate, so `roof-a`/`roof-b` are tied. Tests vary one field off this.
    fn tied_gable_model() -> BuildingModel {
        let mut model = gable_model();
        model.levels[0].height = Length::from_feet(8.0);
        model.systems.push(ceiling_system());
        model.ceilings.push(Ceiling::new(
            "ceiling-1",
            "Flat ceiling",
            "level-1",
            "system-ceiling",
            SurfaceRegion::Polygon(rect_outline(24.0, 24.0)),
            Length::ZERO,
        ));
        model
    }

    /// Whether `roof-a` reports a ridge tie: `true` on `roof.ridge.tied` (Info),
    /// `false` on `roof.ridge.beam-required` (Unsupported). Asserts exactly one of
    /// the two judgments is present, so a silently dropped diagnostic fails loudly.
    fn roof_a_ridge_is_tied(model: &BuildingModel) -> bool {
        let plan = generate_project_plan(model).unwrap();
        let roof_a = plan.roof_plan(&ElementId::new("roof-a")).unwrap();
        let tied = roof_a
            .diagnostics
            .iter()
            .any(|d| d.code == "roof.ridge.tied" && d.severity == DiagnosticSeverity::Info);
        let beam = roof_a.diagnostics.iter().any(|d| {
            d.code == "roof.ridge.beam-required" && d.severity == DiagnosticSeverity::Unsupported
        });
        assert!(tied ^ beam, "exactly one ridge-tie judgment is emitted");
        tied
    }

    #[test]
    fn gable_with_a_flat_ceiling_tie_emits_ridge_tied() {
        // A flat ceiling enclosing the footprint at the plate ties the opposing
        // rafters, so a ridge board is adequate — and still framed.
        let model = tied_gable_model();
        assert!(roof_a_ridge_is_tied(&model));
        let plan = generate_project_plan(&model).unwrap();
        assert!(
            plan.roof_plan(&ElementId::new("roof-a"))
                .unwrap()
                .members
                .iter()
                .any(|member| member.kind == MemberKind::RidgeBoard)
        );
    }

    /// Whether `roof` carries the cathedral region diagnostic (`true`) or the attic
    /// one (`false`); asserts exactly one of the two Info diagnostics is present so a
    /// silently dropped or duplicated classification fails loudly.
    fn roof_region_is_cathedral(plan: &ProjectFramePlan, id: &str) -> bool {
        let roof = plan.roof_plan(&ElementId::new(id)).unwrap();
        let cathedral = roof
            .diagnostics
            .iter()
            .any(|d| d.code == "roof.ceiling.cathedral" && d.severity == DiagnosticSeverity::Info);
        let attic = roof
            .diagnostics
            .iter()
            .any(|d| d.code == "roof.ceiling.attic" && d.severity == DiagnosticSeverity::Info);
        assert!(
            cathedral ^ attic,
            "exactly one cathedral/attic region diagnostic is emitted for {id}"
        );
        cathedral
    }

    #[test]
    fn cathedral_roof_region_emits_cathedral_diagnostic() {
        // No ceiling anywhere → each gable plane is a cathedral region, and the
        // classification agrees with the renderer's cathedral flag.
        let model = gable_model();
        let plan = generate_project_plan(&model).unwrap();
        for (index, id) in ["roof-a", "roof-b"].iter().enumerate() {
            assert!(
                roof_region_is_cathedral(&plan, id),
                "{id} should be cathedral"
            );
            assert!(model.roof_cathedral_flags()[index], "{id} core flag agrees");
        }
    }

    #[test]
    fn attic_roof_region_emits_attic_diagnostic() {
        // A flat ceiling enclosing the footprint makes the space above each plane an
        // attic (the inverse classification of the cathedral case).
        let plan = generate_project_plan(&tied_gable_model()).unwrap();
        for id in ["roof-a", "roof-b"] {
            assert!(
                !roof_region_is_cathedral(&plan, id),
                "{id} should be an attic"
            );
        }
    }

    #[test]
    fn degenerate_roof_plane_is_not_classified_cathedral_or_attic() {
        // The classification is skipped for a plane that frames nothing, so a
        // degenerate plane carries only its `roof.outline.degenerate` warning, never a
        // misleading attic/cathedral Info. A degenerate outline is rejected by model
        // validation, so it reaches the classifier only by calling `generate_roof_plans`
        // directly (the guard is defensive). `roof_cathedral_flags` still returns a flag
        // for the plane (its centroid is testable), so the geometry guard is what
        // suppresses the diagnostic: without it this plane would emit
        // `roof.ceiling.cathedral` (the model has no ceiling), and an inverted guard
        // would instead drop the well-formed planes' classification.
        let mut model = gable_model();
        model.roof_planes.push(RoofPlane::new(
            "roof-degenerate",
            "Degenerate",
            "level-1",
            "system-roof",
            vec![
                Point2::new(Length::ZERO, Length::ZERO),
                Point2::new(Length::ZERO, Length::ZERO),
                Point2::new(Length::from_feet(24.0), Length::from_feet(12.0)),
                Point2::new(Length::ZERO, Length::from_feet(12.0)),
            ],
            pitch_9_12(),
            0,
            Length::from_feet(8.0),
        ));

        let mut plan = ProjectFramePlan {
            wall_plans: Vec::new(),
            floor_plans: Vec::new(),
            ceiling_plans: Vec::new(),
            roof_plans: Vec::new(),
            diagnostics: Vec::new(),
            rooms: Vec::new(),
            layers: Vec::new(),
            fasteners: Vec::new(),
        };
        generate_roof_plans(&mut plan, &model).unwrap();

        let degenerate = plan.roof_plan(&ElementId::new("roof-degenerate")).unwrap();
        assert!(
            degenerate
                .diagnostics
                .iter()
                .any(|d| d.code == "roof.outline.degenerate"),
            "the degenerate plane keeps its outline warning"
        );
        assert!(
            degenerate
                .diagnostics
                .iter()
                .all(|d| d.code != "roof.ceiling.cathedral" && d.code != "roof.ceiling.attic"),
            "a plane that frames nothing is not classified cathedral or attic"
        );
        // The guard did not over-suppress: the well-formed planes stay classified.
        assert!(roof_region_is_cathedral(&plan, "roof-a"));
    }

    #[test]
    fn ceiling_tie_respects_the_plate_slack_threshold() {
        // The ceiling sits `height` below the 96in level top, and the springing is
        // 96in, so it ties only while `height <= TIE_PLATE_SLACK` (6in). This pins
        // both the constant and the `>=` direction — a 0in or 60in slack would flip
        // one of these cases (the equal-elevation fixture alone cannot).
        let mut within = tied_gable_model();
        within.ceilings[0].height = Length::from_whole_inches(6); // at the slack
        assert!(
            roof_a_ridge_is_tied(&within),
            "a ceiling within the plate slack ties the rafters"
        );

        let mut beyond = tied_gable_model();
        beyond.ceilings[0].height = Length::from_whole_inches(7); // just past the slack
        assert!(
            !roof_a_ridge_is_tied(&beyond),
            "a ceiling dropped past the plate slack needs a ridge beam"
        );
    }

    #[test]
    fn a_sloped_ceiling_is_not_a_rafter_tie() {
        // A scissor/vault ceiling (sloped) is not a full thrust tie, so the ridge
        // still needs a beam — the `slope.is_none()` filter in `collect_ceiling_ties`.
        let mut model = tied_gable_model();
        model.ceilings[0].slope = Some(framer_core::CeilingSlope::new(
            Slope::new(Length::from_whole_inches(3), Length::from_whole_inches(12)),
            0,
        ));
        assert!(!roof_a_ridge_is_tied(&model));
    }

    #[test]
    fn a_ceiling_on_another_level_does_not_tie_the_roof() {
        // A flat ceiling on a different level is not a tie at this roof's plate.
        let mut model = tied_gable_model();
        model.levels.push(
            framer_core::Level::new("level-2", "Level 2", Length::from_feet(9.0))
                .with_height(Length::from_feet(8.0)),
        );
        model.ceilings[0].level = ElementId::new("level-2");
        assert!(!roof_a_ridge_is_tied(&model));
    }

    #[test]
    fn a_ceiling_not_covering_the_plane_does_not_tie() {
        // A flat ceiling whose region excludes the plane footprint (centroid at
        // 12ft, 6ft) does not tie the rafters — the `point_in_polygon` conjunct.
        let mut model = tied_gable_model();
        model.ceilings[0].region = SurfaceRegion::Polygon(rect_outline(4.0, 4.0));
        assert!(!roof_a_ridge_is_tied(&model));
    }

    #[test]
    fn a_floor_deck_below_the_plate_does_not_tie_a_cathedral() {
        // The PR #41 fix, by the *elevation gate* (not a type exclusion): a deck
        // bears at the floor (`level.elevation` = 0), 96in below the springing, so
        // even a deck spanning the whole footprint leaves a cathedral roof untied.
        let mut model = gable_model();
        model.levels[0].height = Length::from_feet(8.0);
        model.systems.push(floor_system());
        model.floor_decks.push(FloorDeck::new(
            "deck-1",
            "Floor deck",
            "level-1",
            "system-floor",
            SurfaceRegion::Polygon(rect_outline(24.0, 24.0)),
        ));
        assert!(!roof_a_ridge_is_tied(&model));
    }

    #[test]
    fn a_floor_deck_at_the_bearing_line_ties_like_a_ceiling() {
        // Spec Decision #10: a floor deck *does* tie when its elevation reaches the
        // plate (the floor-of-N+1 = ceiling-of-N case). Pinning that the exclusion is
        // by elevation, not by type: a deck whose level elevation equals the roof
        // springing, with no ceiling, ties the rafters. Reverting to a categorical
        // `FloorDeck` exclusion fails here.
        let mut model = gable_model();
        for plane in &mut model.roof_planes {
            plane.reference_elevation = Length::ZERO; // springing at the deck elevation
        }
        model.systems.push(floor_system());
        model.floor_decks.push(FloorDeck::new(
            "deck-1",
            "Floor deck",
            "level-1",
            "system-floor",
            SurfaceRegion::Polygon(rect_outline(24.0, 24.0)),
        ));
        assert!(roof_a_ridge_is_tied(&model));
    }

    #[test]
    fn mismatched_gable_frames_a_ridge_per_plane_and_is_flagged() {
        // Two planes share the plan ridge edge but disagree on the ridge height:
        // roof-a at 9:12 ridges 108in above the plate, roof-b at 18:12 ridges
        // 216in above. A single shared ridge would float away from one side, so
        // each plane frames its own ridge and both are flagged.
        let mut model = gable_model();
        let north = model
            .roof_planes
            .iter_mut()
            .find(|plane| plane.id.0 == "roof-b")
            .unwrap();
        north.slope = Slope::new(Length::from_whole_inches(18), Length::from_whole_inches(12));

        let plan = generate_project_plan(&model).unwrap();

        // One ridge board per plane (not a single shared ridge), each at its own
        // ridge elevation.
        for (id, ridge_in) in [("roof-a", 96 + 108), ("roof-b", 96 + 216)] {
            let roof = plan.roof_plan(&ElementId::new(id)).unwrap();
            let ridges: Vec<_> = roof
                .members
                .iter()
                .filter(|member| member.kind == MemberKind::RidgeBoard)
                .collect();
            assert_eq!(ridges.len(), 1, "{id} frames its own ridge");
            let slope = ridges[0].sloped.expect("a ridge carries an elevation");
            assert_eq!(slope.high_elevation, Length::from_whole_inches(ridge_in));
            assert!(
                roof.diagnostics.iter().any(|diagnostic| {
                    diagnostic.code == "roof.ridge.mismatched-elevation"
                        && diagnostic.severity == DiagnosticSeverity::Unsupported
                }),
                "{id} is flagged for the mismatched ridge"
            );
        }
    }

    #[test]
    fn shed_roof_through_project_plan_has_no_ridge_board() {
        let mut model = BuildingModel::new();
        model.systems.push(roof_system());
        model.roof_planes.push(shed_plane(pitch_9_12()));

        let plan = generate_project_plan(&model).unwrap();
        let roof = plan.roof_plan(&ElementId::new("roof-shed")).unwrap();
        assert!(
            roof.members
                .iter()
                .all(|member| member.kind != MemberKind::RidgeBoard),
            "a lone shed plane shares no ridge, so frames no ridge board"
        );
        // A shed frames no ridge board, so it makes no ridge-tie judgment at all.
        assert!(roof.diagnostics.iter().all(|diagnostic| {
            diagnostic.code != "roof.ridge.tied" && diagnostic.code != "roof.ridge.beam-required"
        }));
    }

    #[test]
    fn varying_plate_height_under_a_roof_is_flagged_unsupported() {
        let code = FramingDefaults::irc_2021_starter();
        let mut model = BuildingModel::new();
        model.systems.push(roof_system());
        // Two walls on the roof's level at different heights: the bearing plate
        // height is ambiguous for v1.
        let mut tall = placed(
            "w-tall",
            Point2::new(Length::ZERO, Length::ZERO),
            Point2::new(Length::from_feet(12.0), Length::ZERO),
            &code,
        );
        tall.height = Length::from_feet(9.0);
        let mut short = placed(
            "w-short",
            Point2::new(Length::ZERO, Length::from_feet(12.0)),
            Point2::new(Length::from_feet(12.0), Length::from_feet(12.0)),
            &code,
        );
        short.height = Length::from_feet(8.0);
        model.walls.push(tall);
        model.walls.push(short);
        model.roof_planes.push(shed_plane(pitch_9_12()));

        let plan = generate_project_plan(&model).unwrap();
        let roof = plan.roof_plan(&ElementId::new("roof-shed")).unwrap();
        assert!(roof.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "roof.plate-height.varying"
                && diagnostic.severity == DiagnosticSeverity::Unsupported
                && diagnostic.source.as_ref().map(|id| id.0.as_str()) == Some("roof-shed")
        }));
    }

    #[test]
    fn degenerate_roof_outline_frames_nothing_and_warns() {
        // A zero-length eave edge (coincident consecutive vertices) passes core
        // geometry validation — which only rejects <3 points, self-intersection,
        // an out-of-range eave edge, and slope.run <= 0 — but cannot lay out
        // rafters. generate_roof_plan recovers with an empty plan and a Warning
        // rather than dividing by a zero-length eave.
        let plane = RoofPlane::new(
            "roof-degenerate",
            "Degenerate",
            "level-1",
            "system-roof",
            vec![
                Point2::new(Length::ZERO, Length::ZERO),
                Point2::new(Length::ZERO, Length::ZERO),
                Point2::new(Length::from_feet(24.0), Length::from_feet(12.0)),
                Point2::new(Length::ZERO, Length::from_feet(12.0)),
            ],
            pitch_9_12(),
            0,
            Length::from_feet(8.0),
        );

        let plan = generate_roof_plan(
            &plane,
            &roof_system(),
            &materials(),
            false,
            RidgeTie::Untied,
        )
        .unwrap();
        assert!(
            plan.members.is_empty(),
            "a degenerate outline frames no members"
        );
        assert!(plan.layers.is_empty());
        assert_eq!(plan.diagnostics.len(), 1);
        assert!(plan.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "roof.outline.degenerate"
                && diagnostic.severity == DiagnosticSeverity::Warning
                && diagnostic.source.as_ref().map(|id| id.0.as_str()) == Some("roof-degenerate")
        }));
    }
}
