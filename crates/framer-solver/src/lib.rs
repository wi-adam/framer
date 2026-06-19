use std::collections::BTreeMap;
use std::fmt::Write;

use framer_core::{
    BoardProfile, BuildingModel, CodeProfile, ConstructionSystem, ElementId, FramingSpec,
    LayerFunction, Length, Material, ModelError, Opening, Point2, Wall, WallJoin, WallJoinKind,
    room_boundaries,
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
    pub fn bom(&self) -> Vec<BomItem> {
        bom_from_members(
            self.wall_plans
                .iter()
                .flat_map(|wall_plan| wall_plan.members.iter()),
        )
    }

    /// The project-wide per-layer material takeoff, aggregated across every wall
    /// by (material, function, thickness).
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
/// wall face minus its openings (clamped non-negative). Area goods report area;
/// continuous insulation and the framing layer's cavity material report
/// area × thickness; air gaps, the framing layer's lumber, structure, and other
/// roles are skipped (lumber is covered by the member BOM). The cavity material
/// uses the framing layer's depth as its thickness.
fn wall_layer_bom(
    wall: &Wall,
    system: &ConstructionSystem,
    materials: &[Material],
) -> Vec<LayerBomItem> {
    let net_area = net_face_area_sq_in(wall);
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
            | LayerFunction::Masonry => {
                items.push(LayerBomItem {
                    material: layer.material.clone(),
                    material_name: material_name(&layer.material),
                    function: layer.function,
                    thickness: layer.thickness,
                    area_sq_in: net_area,
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
                    volume_bd_in: volume_bd_in(net_area, layer.thickness),
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
                        volume_bd_in: volume_bd_in(net_area, layer.thickness),
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
    pub provenance: RuleProvenance,
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

/// The framing detail of a wall's construction system: the single `Framing`
/// layer's `FramingSpec`. Studs and plates use `member`; spacing is the o.c.
/// layout.
fn wall_framing(system: &ConstructionSystem) -> Result<&FramingSpec, SolverError> {
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
        let framing = wall_framing(system)?;
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

pub fn generate_project_plan(model: &BuildingModel) -> Result<ProjectFramePlan, SolverError> {
    model.validate()?;

    let mut plan = ProjectFramePlan {
        wall_plans: Vec::with_capacity(model.walls.len()),
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
    plan.rooms = room_schedule(model, &mut plan.diagnostics);
    plan.layers = layer_bom_from(
        plan.wall_plans
            .iter()
            .flat_map(|wall_plan| wall_plan.layers.iter()),
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
            wall_framing(system)?.member,
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
        }
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
}
