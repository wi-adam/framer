use std::collections::BTreeMap;
use std::fmt::Write;

use framer_core::{
    BoardProfile, BuildingModel, Ceiling, CodeProfile, ConstructionSystem, ElementId, FloorDeck,
    FramingSpec, LayerFunction, Length, Material, ModelError, Opening, Point2, RoofPlane, Slope,
    SpanDirection, SurfaceRegion, Wall, WallJoin, WallJoinKind, point_in_polygon,
    polygon_area_square_inches, room_boundaries,
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
    code: &CodeProfile,
) -> Result<WallFramePlan, SolverError> {
    wall.validate()?;

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
    let mut diagnostics = starter_profile_diagnostics(wall, code);
    let plate_thickness = wall_plate.thickness();
    let stud_thickness = wall_stud.thickness();
    let top_plate_count = if code.double_top_plate { 2 } else { 1 };
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
        add_opening_members(
            &mut members,
            &mut diagnostics,
            wall,
            code,
            framing,
            &opening,
            top_plate_count,
        );
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

/// Generate the framing plan for one flat ceiling from its resolved plan outline.
/// A flat ceiling is a floor deck viewed from below, so it reuses the joisting
/// generator; v1 ceilings always span the shorter clear dimension.
pub fn generate_ceiling_plan(
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
    let n = plane.outline.len();

    // Run extent: the farthest up-slope perpendicular distance to any vertex.
    let run_extent = plane
        .outline
        .iter()
        .map(|p| frame.up_slope_distance(p.x.inches(), p.y.inches()))
        .fold(0.0_f64, f64::max);

    // High (ridge) edge: the outline edge whose midpoint is farthest up-slope.
    let i = plane.eave_edge as usize % n;
    let mut high_edge = (plane.outline[i], plane.outline[(i + 1) % n]);
    let mut best = f64::MIN;
    for k in 0..n {
        let p = plane.outline[k];
        let q = plane.outline[(k + 1) % n];
        let d = frame.up_slope_distance(
            (p.x.inches() + q.x.inches()) / 2.0,
            (p.y.inches() + q.y.inches()) / 2.0,
        );
        if d > best {
            best = d;
            high_edge = (p, q);
        }
    }

    Some(RoofPlaneGeometry {
        eave_length: Length::from_inches(frame.eave_length()),
        run_extent: Length::from_inches(run_extent),
        high_edge,
    })
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

    let factor = slope_factor(plane.slope);
    let ratio = slope_ratio(plane.slope);
    let run_extent = geometry.run_extent;
    let overhang = plane.eave_overhang;

    // The whole rafter board, eave overhang tail included, cut to true length.
    let total_plan_run = run_extent + overhang;
    let cut_length = Length::from_inches(total_plan_run.inches() * factor);
    // Plane-local v = 0 is the eave bearing line; the tail sits at v = -overhang.
    let tail_v = Length::ZERO - overhang;
    let ridge_elevation = ridge_elevation(plane, run_extent);
    let tail_elevation = plane.reference_elevation - Length::from_inches(overhang.inches() * ratio);
    let rafter_slope = SlopedPlacement {
        low_elevation: tail_elevation,
        high_elevation: ridge_elevation,
    };

    // Common rafters: arrayed along the eave (layout) axis at o.c., each running
    // up the slope. End rafters align with the rake edges.
    let positions = stud_positions(geometry.eave_length, spacing, rafter_thickness);
    for mark in &positions {
        members.push(frame_member(
            format!("rafter-{}", mark.ticks()),
            &plane.id,
            MemberKind::Rafter,
            rafter,
            FrameMemberPlacement::new(
                MemberOrientation::Vertical,
                *mark,
                tail_v,
                cut_length,
                rafter_thickness,
            )
            .with_slope(rafter_slope),
            band,
            RuleProvenance::new(
                "roof.rafters.on-center",
                format!(
                    "Common rafters span the {} plan run perpendicular to the eave; end rafters align with the rake edges and interior rafters fall on {} layout marks. The cut length is the true sloped length (plan run times the {}:{} pitch).",
                    run_extent, spacing, plane.slope.rise, plane.slope.run
                ),
            ),
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
                geometry.eave_length,
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
        plan.roof_plans.push(roof_plan);
    }
    Ok(())
}

/// Resolve every surface region to its closed plan outline in a single pass over
/// the wall graph. `Polygon` regions are their own outline; `Room` regions are
/// resolved through one [`room_boundaries`] call so the bounded faces are derived
/// just once for the whole project (rather than per element). Each entry lines up
/// with `regions`; `None` marks an open `Room` loop (a transient mid-edit
/// condition, surfaced as a diagnostic) or an unresolvable room reference.
fn resolve_surface_regions(
    model: &BuildingModel,
    regions: &[&SurfaceRegion],
) -> Vec<Option<Vec<Point2>>> {
    // The seed of each `Room` region, in order, plus where its boundary will land
    // in the batch result (`None` for `Polygon` regions and unknown rooms).
    let mut seeds = Vec::new();
    let mut slots = Vec::with_capacity(regions.len());
    for region in regions {
        match region {
            SurfaceRegion::Polygon(_) => slots.push(None),
            SurfaceRegion::Room(room) => {
                match model.rooms.iter().find(|candidate| candidate.id == *room) {
                    Some(found) => {
                        slots.push(Some(seeds.len()));
                        seeds.push(found.seed);
                    }
                    None => slots.push(None),
                }
            }
        }
    }

    let boundaries = room_boundaries(model, &seeds);
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
        let ceiling_plan = match outlines.next().expect("one outline per ceiling") {
            Some(outline) => generate_ceiling_plan(ceiling, system, &outline, &model.materials)?,
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

    let mut plan = ProjectFramePlan {
        wall_plans: Vec::with_capacity(model.walls.len()),
        floor_plans: Vec::with_capacity(model.floor_decks.len()),
        ceiling_plans: Vec::with_capacity(model.ceilings.len()),
        roof_plans: Vec::with_capacity(model.roof_planes.len()),
        diagnostics: project_diagnostics(model),
        rooms: Vec::new(),
        layers: Vec::new(),
    };

    for wall in &model.walls {
        let system = model
            .system_for(wall)
            .ok_or_else(|| SolverError::MissingSystem {
                wall: wall.id.clone(),
                system: wall.system.clone(),
            })?;
        plan.wall_plans.push(generate_wall_plan(
            wall,
            system,
            &model.materials,
            &model.code,
        )?);
    }

    add_join_members(&mut plan, model)?;
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

    Ok(plan)
}

/// Derive a takeoff row for each authored room from its bounding wall loop. Rooms
/// that are not enclosed get a zeroed row plus a `Warning` diagnostic.
fn room_schedule(
    model: &BuildingModel,
    diagnostics: &mut Vec<PlanDiagnostic>,
) -> Vec<RoomSchedule> {
    let seeds: Vec<Point2> = model.rooms.iter().map(|room| room.seed).collect();
    let boundaries = room_boundaries(model, &seeds);
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

fn add_join_members(plan: &mut ProjectFramePlan, model: &BuildingModel) -> Result<(), SolverError> {
    let plate_thickness = model.code.plate_profile.thickness();
    let top_plate_count = if model.code.double_top_plate { 2 } else { 1 };

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

fn add_opening_members(
    members: &mut Vec<FrameMember>,
    diagnostics: &mut Vec<PlanDiagnostic>,
    wall: &Wall,
    code: &CodeProfile,
    framing: WallFraming,
    opening: &Opening,
    top_plate_count: usize,
) {
    let wall_stud = framing.member;
    let band = framing.band;
    let plate_thickness = code.plate_profile.thickness();
    let stud_base = plate_thickness;
    let stud_top = wall.height - plate_thickness * top_plate_count as i64;
    let header_bottom = opening.top();
    let header_depth = code.default_header_depth.min(stud_top - header_bottom);
    let header_top = header_bottom + header_depth;
    let left = opening.left();
    let right = opening.right();
    let stud_thickness = wall_stud.thickness();
    let side_positions = OpeningSidePositions::new(left, right, stud_thickness);

    if header_depth < code.default_header_depth {
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

    for (side, king_x, jack_x) in [
        ("left", side_positions.left_king, side_positions.left_jack),
        (
            "right",
            side_positions.right_king,
            side_positions.right_jack,
        ),
    ] {
        members.push(frame_member(
            format!("{}-king-{}", opening.id.0, side),
            &opening.id,
            MemberKind::KingStud,
            wall_stud,
            FrameMemberPlacement::new(
                MemberOrientation::Vertical,
                king_x,
                stud_base,
                stud_top - stud_base,
                stud_thickness,
            ),
            band,
            RuleProvenance::new(
                "opening.king-studs.each-side",
                format!(
                    "A king stud is generated at the {side} rough opening edge for {}.",
                    opening.name
                ),
            ),
        ));

        members.push(frame_member(
            format!("{}-jack-{}", opening.id.0, side),
            &opening.id,
            MemberKind::JackStud,
            wall_stud,
            FrameMemberPlacement::new(
                MemberOrientation::Vertical,
                jack_x,
                stud_base,
                header_bottom - stud_base,
                stud_thickness,
            ),
            band,
            RuleProvenance::new(
                "opening.jack-studs.header-bearing",
                format!(
                    "A jack stud is generated at the {side} rough opening edge to support the starter-profile header.",
                ),
            ),
        ));
    }

    members.push(frame_member(
        format!("{}-header", opening.id.0),
        &opening.id,
        MemberKind::Header,
        code.header_profile,
        FrameMemberPlacement::new(
            MemberOrientation::Horizontal,
            side_positions.left_jack_left_face,
            header_bottom,
            opening.width + stud_thickness * 2,
            header_depth,
        ),
        band,
        RuleProvenance::new(
            "opening.header.default-profile",
            format!(
                "Header uses the configured starter profile {} with default depth {}; no span/load lookup is performed yet.",
                code.header_profile.label(),
                header_depth
            ),
        ),
    ));

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

struct OpeningSidePositions {
    left_king: Length,
    left_jack: Length,
    left_jack_left_face: Length,
    right_jack: Length,
    right_king: Length,
}

impl OpeningSidePositions {
    fn new(opening_left: Length, opening_right: Length, stud_thickness: Length) -> Self {
        let half_stud = stud_thickness / 2;
        Self {
            left_king: opening_left - stud_thickness - half_stud,
            left_jack: opening_left - half_stud,
            left_jack_left_face: opening_left - stud_thickness,
            right_jack: opening_right + half_stud,
            right_king: opening_right + stud_thickness + half_stud,
        }
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

fn starter_profile_diagnostics(wall: &Wall, code: &CodeProfile) -> Vec<PlanDiagnostic> {
    let mut diagnostics = vec![
        PlanDiagnostic::new(
            DiagnosticSeverity::Warning,
            "code-profile.starter-only",
            Some(wall.id.clone()),
            format!(
                "{} is starter rule data for deterministic framing defaults, not complete IRC compliance.",
                code.display_name
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
                    "{} is modeled as a wide rough opening with starter-profile king, jack, and header members; garage-door-specific structural design is unsupported.",
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

pub fn export_bom_csv(bom: &[BomItem]) -> String {
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
    #[error("construction system {system:?} has no framing layer")]
    SystemHasNoFramingLayer { system: ElementId },
}

#[cfg(test)]
mod tests {
    use framer_core::{
        BuildingModel, CodeProfile, ElementId, Opening, Wall, load_project, save_project,
    };

    use super::*;

    /// A closed 12ft × 8ft rectangle with one room seeded at its centre.
    fn rectangle_with_room() -> BuildingModel {
        use framer_core::{Point2, Room, RoomUsage};
        let code = CodeProfile::irc_2021_prescriptive();
        let mut model = BuildingModel::new(code.clone());
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

    fn placed(id: &str, a: Point2, b: Point2, code: &CodeProfile) -> Wall {
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
        let code = CodeProfile::irc_2021_prescriptive();
        let mut model = BuildingModel::new(code.clone());
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
        let code = CodeProfile::irc_2021_prescriptive();
        let mut model = BuildingModel::new(code.clone());
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
    fn open_room_emits_warning_diagnostic() {
        use framer_core::{Point2, Room, RoomUsage};
        let code = CodeProfile::irc_2021_prescriptive();
        let mut model = BuildingModel::new(code.clone());
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
        let code = CodeProfile::irc_2021_prescriptive();
        let mut wall = Wall::new("wall", "Wall", Length::from_feet(12.0), &code);
        wall.openings.push(Opening::door(
            "door",
            "Door",
            Length::from_feet(4.0),
            Length::from_inches(36.0),
            Length::from_inches(80.0),
        ));

        let plan = generate_wall_plan(&wall, &wall_system(), &materials(), &code).unwrap();

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
    fn members_sit_in_the_framing_layer_band() {
        let code = CodeProfile::irc_2021_prescriptive();
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

        let plan = generate_wall_plan(&wall, &system, &materials(), &code).unwrap();
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
        let code = CodeProfile::irc_2021_prescriptive();
        let mut wall = Wall::new("wall", "Wall", Length::from_feet(12.0), &code);
        wall.openings.push(Opening::door(
            "door",
            "Door",
            Length::from_feet(4.0),
            Length::from_inches(36.0),
            Length::from_inches(80.0),
        ));

        let plan = generate_wall_plan(&wall, &wall_system(), &materials(), &code).unwrap();
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
        let code = CodeProfile::irc_2021_prescriptive();
        let wall = Wall::new("wall", "Wall", Length::from_feet(8.0), &code);

        let plan = generate_wall_plan(&wall, &wall_system(), &materials(), &code).unwrap();
        let bom = plan.bom();

        assert!(bom.iter().any(|item| {
            item.kind == MemberKind::TopPlate
                && item.cut_length == Length::from_feet(8.0)
                && item.quantity == 2
        }));
    }

    #[test]
    fn framing_member_sizes_studs_and_plates() {
        let code = CodeProfile::irc_2021_prescriptive();
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

        let plan = generate_wall_plan(&wall, &system, &materials(), &code).unwrap();

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

        // The header still follows the code profile until span lookups exist.
        let header = plan
            .members
            .iter()
            .find(|member| member.kind == MemberKind::Header)
            .unwrap();
        assert_eq!(header.profile, code.header_profile);
    }

    #[test]
    fn end_studs_align_faces_with_wall_edges() {
        let code = CodeProfile::irc_2021_prescriptive();
        let wall = Wall::new("wall", "Wall", Length::from_feet(8.0), &code);

        let plan = generate_wall_plan(&wall, &wall_system(), &materials(), &code).unwrap();
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
        let code = CodeProfile::irc_2021_prescriptive();
        let wall = Wall::new("wall", "Wall", Length::from_feet(8.0), &code);

        let plan = generate_wall_plan(&wall, &wall_system(), &materials(), &code).unwrap();

        assert!(plan.members.iter().any(|member| {
            member.kind == MemberKind::CommonStud && member.cut_length == Length::from_inches(91.5)
        }));
    }

    #[test]
    fn project_round_trip_regenerates_same_wall_plan() {
        let model = BuildingModel::demo_wall();
        let system = model.system_for(&model.walls[0]).unwrap();
        let original =
            generate_wall_plan(&model.walls[0], system, &model.materials, &model.code).unwrap();

        let serialized = save_project(&model).unwrap();
        let loaded = load_project(&serialized).unwrap();
        let loaded_system = loaded.system_for(&loaded.walls[0]).unwrap();
        let regenerated = generate_wall_plan(
            &loaded.walls[0],
            loaded_system,
            &loaded.materials,
            &loaded.code,
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
        let half_stud = model.code.stud_profile.thickness() / 2;

        for join in &model.wall_joins {
            for wall_id in [&join.first_wall, &join.second_wall] {
                let wall = model
                    .walls
                    .iter()
                    .find(|candidate| candidate.id == *wall_id)
                    .unwrap();
                let join_x = wall.local_x_for_point(join.point).unwrap();
                let expected_x =
                    face_aligned_center(join_x, wall.length, model.code.stud_profile.thickness());
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
        let csv = export_bom_csv(&plan.bom());

        assert!(svg.contains("data-wall=\"wall-front\""));
        assert!(svg.contains("data-join=\"join-front-right\""));
        assert!(svg.contains("data-wall-elevation=\"wall-right\""));
        assert!(csv.contains("corner post"));
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
        let code = CodeProfile::irc_2021_prescriptive();
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

        let plan = generate_wall_plan(&wall, &system, &materials(), &code).unwrap();

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
        let code = CodeProfile::irc_2021_prescriptive();
        let mut wall = Wall::new("wall", "Wall", Length::from_feet(12.0), &code);
        wall.openings.push(Opening::window(
            "window",
            "Window",
            Length::from_feet(4.0),
            Length::from_inches(48.0),
            Length::from_inches(36.0),
            Length::from_inches(36.0),
        ));

        let plan = generate_wall_plan(&wall, &wall_system(), &materials(), &code).unwrap();

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
        let plan =
            generate_wall_plan(&model.walls[0], system, &model.materials, &model.code).unwrap();

        let header = plan
            .members
            .iter()
            .find(|member| member.id == "opening-door-1-header")
            .unwrap();

        assert_eq!(header.source.0, "opening-door-1");
        assert_eq!(header.provenance.rule_id, "opening.header.default-profile");
        assert!(header.provenance.summary.contains("no span/load lookup"));
        assert_eq!(header.cross_section_depth, model.code.default_header_depth);
    }

    #[test]
    fn garage_door_reports_unsupported_starter_assumption() {
        let model = BuildingModel::demo_wall();
        let system = model.system_for(&model.walls[0]).unwrap();
        let plan =
            generate_wall_plan(&model.walls[0], system, &model.materials, &model.code).unwrap();

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
        let plan = generate_wall_plan(wall, system, &model.materials, &model.code).unwrap();

        let first_svg = export_wall_elevation_svg(wall, &plan);
        let second_svg = export_wall_elevation_svg(wall, &plan);
        let csv = export_bom_csv(&plan.bom());

        assert_eq!(first_svg, second_svg);
        assert!(first_svg.contains("<svg"));
        assert!(first_svg.contains("opening-garage-1-header"));
        assert!(first_svg.contains(r#"data-opening="opening-garage-1""#));
        assert!(
            first_svg
                .lines()
                .any(|line| line.contains(r#"id="opening-door-1-header""#)
                    && line.contains(r#"height="9""#))
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
        let code = CodeProfile::irc_2021_prescriptive();
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

        let plan = generate_wall_plan(&wall, &wall_system(), &materials(), &code).unwrap();

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
        let code = CodeProfile::irc_2021_prescriptive();
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

        let plan = generate_wall_plan(&wall, &wall_system(), &materials(), &code).unwrap();

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
        assert!(export_bom_csv(&plan.bom()).starts_with("quantity,profile,kind"));
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
        let mut model = BuildingModel::new(CodeProfile::irc_2021_prescriptive());
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
        };

        let error = generate_surface_plans(&mut plan, &model).unwrap_err();
        assert!(matches!(
            error,
            SolverError::MissingSystemForElement { element, system }
                if element.0 == "deck-1" && system.0 == "system-absent"
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

    /// The shipped `demo-shell` example (capped with a gable roof, a flat ceiling,
    /// and a floor deck in Slice 6) frames end-to-end: both slopes rafter, the
    /// gable shares one ridge board, the ceiling and deck joist, and every new
    /// member family folds into the project BOM.
    #[test]
    fn roofed_demo_shell_example_frames_every_surface() {
        let example = include_str!("../../../examples/projects/demo-shell.framer");
        let model = load_project(example).unwrap();
        let plan = generate_project_plan(&model).unwrap();

        assert_eq!(plan.roof_plans.len(), 2);
        assert_eq!(plan.ceiling_plans.len(), 1);
        assert_eq!(plan.floor_plans.len(), 1);

        for id in ["roof-north", "roof-south"] {
            let roof = plan.roof_plan(&ElementId::new(id)).unwrap();
            assert!(
                roof.members
                    .iter()
                    .any(|member| member.kind == MemberKind::Rafter),
                "{id} frames rafters"
            );
        }
        let ridges = plan
            .roof_plans
            .iter()
            .flat_map(|roof| roof.members.iter())
            .filter(|member| member.kind == MemberKind::RidgeBoard)
            .count();
        assert_eq!(ridges, 1, "a gable shares one ridge board");

        assert!(
            plan.ceiling_plan(&ElementId::new("ceiling-1"))
                .unwrap()
                .members
                .iter()
                .any(|member| member.kind == MemberKind::CeilingJoist)
        );
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
        let code = CodeProfile::irc_2021_prescriptive();
        let mut model = BuildingModel::new(code.clone());
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
        let code = CodeProfile::irc_2021_prescriptive();
        let mut model = BuildingModel::new(code.clone());
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
        let mut model = BuildingModel::new(CodeProfile::irc_2021_prescriptive());
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
        model.ceilings[0].slope = Some(Slope::new(
            Length::from_whole_inches(3),
            Length::from_whole_inches(12),
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
        let mut model = BuildingModel::new(CodeProfile::irc_2021_prescriptive());
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
        let code = CodeProfile::irc_2021_prescriptive();
        let mut model = BuildingModel::new(code.clone());
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
