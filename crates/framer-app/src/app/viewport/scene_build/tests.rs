use super::*;
use eframe::egui::Rect;
use framer_core::{
    BoardProfile, Ceiling, ConstructionLayer, FloorDeck, FramingPattern, FramingSpec,
    LayerFunction, Level, MemberFamily, Point2, Room, RoomUsage, Slope, SystemKind,
};
use framer_solver::ProjectFramePlan;

use crate::app::viewport::camera_3d::View3dState;

#[test]
fn scene_3d_builds_depth_tested_wall_and_member_cuboids() {
    let model = BuildingModel::demo_shell();
    let plan = framer_solver::generate_project_plan(&model).unwrap();
    let scene = Scene3d::from_project(
        &model,
        &plan,
        0,
        &Selection::Wall,
        WorkspaceMode::Plan,
        crate::app::WallDisplay::Full,
    )
    .unwrap();
    // The wall cross-section spans its full system thickness, centered on the
    // centerline so it reaches +/- total/2 on the side axis (no longer the
    // bare stud depth) regardless of which side topology marks as interior.
    // Every demo wall shares one system, so any wall gives the section thickness.
    let total = model
        .system_for(&model.walls[0])
        .expect("wall resolves a system")
        .total_thickness()
        .inches() as f32;
    // The full system is thicker than the framing layer alone, so layering
    // genuinely deepens the wall in the side axis.
    let stud_depth = model
        .framing_defaults()
        .stud_profile
        .nominal_depth()
        .inches() as f32;
    assert!(total > stud_depth);

    assert!(!scene.vertices.is_empty());
    assert!(scene.opaque_index_count > 0);
    assert!(scene.transparent_index_count > 0);
    assert_eq!(scene.opaque_index_count % 36, 0);
    assert_eq!(scene.transparent_index_count % 36, 0);

    let min_y = scene
        .points
        .iter()
        .map(|point| point.y)
        .fold(f32::MAX, f32::min);
    assert!(
        min_y <= -total / 2.0,
        "front wall should have full system thickness in plan depth"
    );

    let front_wall = model
        .walls
        .iter()
        .find(|wall| wall.id.0 == "wall-front")
        .expect("demo shell has a front wall");
    let (_, front_x1) = model.wall_envelope_span(front_wall);
    let front_right_x = front_x1 as f32;
    let front_outer_y = -total / 2.0;
    assert!(
        scene.points.iter().any(|point| {
            (point.x - front_right_x).abs() < 1.0e-4 && (point.y - front_outer_y).abs() < 1.0e-4
        }),
        "the primary through wall should reach the adjoining wall's outside face"
    );
}

#[test]
fn scene_3d_contains_pickable_members_openings_and_walls() {
    let model = BuildingModel::demo_shell();
    let plan = framer_solver::generate_project_plan(&model).unwrap();
    let plan_scene = Scene3d::from_project(
        &model,
        &plan,
        0,
        &Selection::Wall,
        WorkspaceMode::Plan,
        crate::app::WallDisplay::Full,
    )
    .unwrap();

    assert!(
        plan_scene
            .picks
            .iter()
            .any(|pick| matches!(&pick.click, ViewClick::Wall(0)))
    );
    assert!(
        plan_scene
            .picks
            .iter()
            .any(|pick| matches!(&pick.click, ViewClick::Opening { .. }))
    );
    assert!(
        plan_scene
            .picks
            .iter()
            .any(|pick| matches!(&pick.click, ViewClick::Member { .. }))
    );

    let design_scene = Scene3d::from_project(
        &model,
        &plan,
        0,
        &Selection::Wall,
        WorkspaceMode::Design,
        crate::app::WallDisplay::Full,
    )
    .unwrap();
    assert!(
        design_scene
            .picks
            .iter()
            .any(|pick| matches!(&pick.click, ViewClick::Wall(0)))
    );
    assert!(
        design_scene
            .picks
            .iter()
            .any(|pick| matches!(&pick.click, ViewClick::Opening { .. }))
    );
    assert!(
        design_scene
            .picks
            .iter()
            .all(|pick| !matches!(&pick.click, ViewClick::Member { .. }))
    );
}

#[test]
fn scene_3d_width_mode_collapses_layers_to_a_single_band() {
    let model = BuildingModel::demo_shell();
    let plan = framer_solver::generate_project_plan(&model).unwrap();
    let full = Scene3d::from_project(
        &model,
        &plan,
        0,
        &Selection::Wall,
        WorkspaceMode::Plan,
        crate::app::WallDisplay::Full,
    )
    .unwrap();
    let width = Scene3d::from_project(
        &model,
        &plan,
        0,
        &Selection::Wall,
        WorkspaceMode::Plan,
        crate::app::WallDisplay::Width,
    )
    .unwrap();

    // Width still fills the wall (one monochrome band per wall, opening-cut),
    // so it has transparent geometry — but strictly less than the multi-layer
    // Full cross-section. It draws no outline edges.
    assert!(width.transparent_index_count > 0);
    assert!(
        width.transparent_index_count < full.transparent_index_count,
        "one band per wall must be lighter than the full layered section"
    );
    assert!(width.outline_edges.is_empty());
}

#[test]
fn scene_3d_outline_mode_emits_edges_not_wall_bands() {
    let model = BuildingModel::demo_shell();
    let plan = framer_solver::generate_project_plan(&model).unwrap();
    let outline = Scene3d::from_project(
        &model,
        &plan,
        0,
        &Selection::Wall,
        WorkspaceMode::Plan,
        crate::app::WallDisplay::Outline,
    )
    .unwrap();

    // No wall fill bands (the transparent pass is empty); walls are carried by
    // the envelope edges instead — 12 per wall — and their corners still feed
    // the orbit projector so the view can frame an all-outline scene.
    assert_eq!(outline.transparent_index_count, 0);
    assert_eq!(outline.outline_edges.len(), model.walls.len() * 12);
    assert!(!outline.points.is_empty());
}

#[test]
fn scene_3d_outline_highlights_only_the_selected_walls_edges() {
    let model = BuildingModel::demo_shell();
    let plan = framer_solver::generate_project_plan(&model).unwrap();
    // Select wall index 1 (not the default 0) so the test can't pass by accident.
    let selected_wall = 1;
    let outline = Scene3d::from_project(
        &model,
        &plan,
        selected_wall,
        &Selection::Wall,
        WorkspaceMode::Design,
        crate::app::WallDisplay::Outline,
    )
    .unwrap();

    // Exactly the selected wall's 12 envelope edges carry `selected` (the overlay
    // paints those blue); every other wall's edges stay unselected.
    let selected_edges = outline.outline_edges.iter().filter(|e| e.selected).count();
    assert_eq!(selected_edges, 12);
    assert_eq!(outline.outline_edges.len(), model.walls.len() * 12);
}

#[test]
fn scene_3d_width_band_is_cut_by_openings() {
    // Width draws one neutral band per wall, but it must still be cut by openings.
    // Build the demo (which has doors/windows) and the same model with openings
    // stripped, both in Design workspace (no members, so only wall fill counts).
    let with_openings = BuildingModel::demo_shell();
    let mut without_openings = with_openings.clone();
    for wall in &mut without_openings.walls {
        wall.openings.clear();
    }
    let plan = framer_solver::generate_project_plan(&with_openings).unwrap();
    let plan_plain = framer_solver::generate_project_plan(&without_openings).unwrap();

    let build = |model, plan| {
        Scene3d::from_project(
            model,
            plan,
            0,
            &Selection::Wall,
            WorkspaceMode::Design,
            crate::app::WallDisplay::Width,
        )
        .unwrap()
        .transparent_index_count
    };
    let cut = build(&with_openings, &plan);
    let uncut = build(&without_openings, &plan_plain);

    // Cutting the band around each opening splits it into more segments, so the
    // cut scene has strictly more triangles than the single uncut band per wall —
    // proving openings are not ignored in Width mode.
    assert!(uncut > 0);
    assert!(
        cut > uncut,
        "openings should split the Width band into more segments ({cut} vs {uncut})"
    );
}

fn empty_plan() -> ProjectFramePlan {
    ProjectFramePlan {
        wall_plans: Vec::new(),
        floor_plans: Vec::new(),
        ceiling_plans: Vec::new(),
        roof_plans: Vec::new(),
        diagnostics: Vec::new(),
        rooms: Vec::new(),
        layers: Vec::new(),
        fasteners: Vec::new(),
    }
}

fn rect() -> Vec<Point2> {
    vec![
        Point2::new(Length::ZERO, Length::ZERO),
        Point2::new(Length::from_feet(12.0), Length::ZERO),
        Point2::new(Length::from_feet(12.0), Length::from_feet(8.0)),
        Point2::new(Length::ZERO, Length::from_feet(8.0)),
    ]
}

fn cuboid_xy_bounds(pick: &PickSolid) -> (f32, f32, f32, f32) {
    pick_points(pick).iter().fold(
        (f32::MAX, f32::MIN, f32::MAX, f32::MIN),
        |(min_x, max_x, min_y, max_y), point| {
            (
                min_x.min(point.x),
                max_x.max(point.x),
                min_y.min(point.y),
                max_y.max(point.y),
            )
        },
    )
}

fn pick_points(pick: &PickSolid) -> &[Point3] {
    match &pick.shape {
        PickShape::Cuboid(corners) => corners,
        PickShape::Mesh { points, .. } => points,
        _ => panic!("expected indexed solid pick geometry"),
    }
}

fn axis_overlap(first_min: f32, first_max: f32, second_min: f32, second_max: f32) -> f32 {
    first_max.min(second_max) - first_min.max(second_min)
}

#[test]
fn lapped_corner_wall_and_post_solids_do_not_overlap() {
    let model = BuildingModel::demo_shell();
    let plan = framer_solver::generate_project_plan(&model).unwrap();
    let scene = Scene3d::from_project(
        &model,
        &plan,
        0,
        &Selection::Wall,
        WorkspaceMode::Plan,
        WallDisplay::Full,
    )
    .unwrap();
    let front_index = model
        .walls
        .iter()
        .position(|wall| wall.id.0 == "wall-front")
        .unwrap();
    let right_index = model
        .walls
        .iter()
        .position(|wall| wall.id.0 == "wall-right")
        .unwrap();
    let wall_pick = |index| {
        scene
            .picks
            .iter()
            .find(|pick| matches!(pick.click, ViewClick::Wall(found) if found == index))
            .unwrap()
    };
    let front = cuboid_xy_bounds(wall_pick(front_index));
    let right = cuboid_xy_bounds(wall_pick(right_index));
    assert!(axis_overlap(front.0, front.1, right.0, right.1) > 0.0);
    assert!(
        axis_overlap(front.2, front.3, right.2, right.3).abs() < 1.0e-4,
        "finished wall bodies must meet at one face without a gap or doubled volume"
    );

    let member_pick = |id: &str| {
        scene
            .picks
            .iter()
            .find(|pick| {
                matches!(
                    &pick.click,
                    ViewClick::Member { member_id, .. } if member_id == id
                )
            })
            .unwrap()
    };
    let front_post = cuboid_xy_bounds(member_pick("join-front-right-wall-front-corner-post"));
    let right_post = cuboid_xy_bounds(member_pick("join-front-right-wall-right-corner-post"));
    assert!(
        axis_overlap(front_post.2, front_post.3, right_post.2, right_post.3) <= 1.0e-4,
        "integer-tick corner posts may leave a sub-tick clearance but must never overlap"
    );
}

fn finish_system(
    id: &str,
    kind: SystemKind,
    finish: LayerFunction,
    finish_material: &str,
    finish_first: bool,
) -> ConstructionSystem {
    let framing = ConstructionLayer::new(
        LayerFunction::Framing,
        "mat-spf",
        BoardProfile::TwoBySix.nominal_depth(),
    )
    .with_framing(FramingSpec {
        member: BoardProfile::TwoBySix,
        spacing: Length::from_whole_inches(16),
        pattern: FramingPattern::Single,
        member_family: MemberFamily::Rafter,
        cavity_material: None,
    });
    let finish = ConstructionLayer::new(finish, finish_material, Length::from_whole_inches(1));
    ConstructionSystem {
        id: ElementId::new(id),
        name: id.to_owned(),
        kind,
        source: None,
        layers: if finish_first {
            vec![finish, framing]
        } else {
            vec![framing, finish]
        },
    }
}

/// A model with one sloped roof plane, one flat ceiling, and one floor deck
/// over a 12×8 footprint (Polygon regions, so no walls are needed).
fn surface_model() -> BuildingModel {
    let mut model = BuildingModel::new();
    for level in &mut model.levels {
        if level.id.0 == "level-1" {
            level.height = Length::from_whole_inches(108);
        }
    }
    model
        .materials
        .push(Material::solid_color("mat-roof", "Shingle", [44, 46, 52]));
    model.materials.push(Material::solid_color(
        "mat-ceil",
        "Ceiling",
        [232, 232, 228],
    ));
    model.materials.push(Material::solid_color(
        "mat-floor",
        "Subfloor",
        [150, 116, 78],
    ));
    model.systems.push(finish_system(
        "system-roof",
        SystemKind::Roof,
        LayerFunction::Roofing,
        "mat-roof",
        false,
    ));
    model.systems.push(finish_system(
        "system-ceiling",
        SystemKind::Ceiling,
        LayerFunction::CeilingFinish,
        "mat-ceil",
        true,
    ));
    model.systems.push(finish_system(
        "system-floor",
        SystemKind::Floor,
        LayerFunction::InteriorFinish,
        "mat-floor",
        true,
    ));
    model.roof_planes.push(RoofPlane::new(
        "roof-1",
        "Roof",
        "level-1",
        "system-roof",
        rect(),
        Slope::new(Length::from_whole_inches(6), Length::from_whole_inches(12)),
        0,
        Length::from_feet(8.0),
    ));
    model.ceilings.push(Ceiling::new(
        "ceiling-1",
        "Ceiling",
        "level-1",
        "system-ceiling",
        SurfaceRegion::Polygon(rect()),
        Length::from_whole_inches(12),
    ));
    model.floor_decks.push(FloorDeck::new(
        "deck-1",
        "Deck",
        "level-1",
        "system-floor",
        SurfaceRegion::Polygon(rect()),
    ));
    model
}

/// A 12×8 shell on a level elevated 10ft, capped by two opposing 6:12 roof
/// planes. The east/west walls therefore receive 2ft-tall gable profiles whose
/// absolute apex is 20ft, exercising both gable geometry and stacked-level z.
fn elevated_gable_model() -> BuildingModel {
    let mut model = BuildingModel::new();
    let level = model
        .levels
        .iter_mut()
        .find(|level| level.id.0 == "level-1")
        .unwrap();
    level.elevation = Length::from_feet(10.0);
    level.height = Length::from_feet(8.0);
    let defaults = model.framing_defaults();
    let p = |x, y| Point2::new(Length::from_feet(x), Length::from_feet(y));
    let wall = |id: &str, start, end| {
        let mut wall = Wall::new(id, id, Length::from_feet(1.0), &defaults)
            .with_placement("level-1", start, end);
        wall.height = Length::from_feet(8.0);
        wall
    };
    model.walls = vec![
        wall("wall-south", p(0.0, 0.0), p(12.0, 0.0)),
        wall("wall-east", p(12.0, 0.0), p(12.0, 8.0)),
        wall("wall-north", p(12.0, 8.0), p(0.0, 8.0)),
        wall("wall-west", p(0.0, 8.0), p(0.0, 0.0)),
    ];
    model
        .materials
        .push(Material::solid_color("mat-roof", "Shingle", [44, 46, 52]));
    model.systems.push(finish_system(
        "system-roof",
        SystemKind::Roof,
        LayerFunction::Roofing,
        "mat-roof",
        false,
    ));
    let slope = Slope::new(Length::from_whole_inches(6), Length::from_whole_inches(12));
    let springing = Length::from_feet(18.0);
    model.roof_planes = vec![
        RoofPlane::new(
            "roof-south",
            "South",
            "level-1",
            "system-roof",
            vec![p(0.0, 0.0), p(12.0, 0.0), p(12.0, 4.0), p(0.0, 4.0)],
            slope,
            0,
            springing,
        ),
        RoofPlane::new(
            "roof-north",
            "North",
            "level-1",
            "system-roof",
            vec![p(0.0, 4.0), p(12.0, 4.0), p(12.0, 8.0), p(0.0, 8.0)],
            slope,
            2,
            springing,
        ),
    ];
    model
}

/// The same 12×8 surface model, but capped by a rectangular hip roof: two
/// trapezoid fields plus two triangular hip ends sharing a shortened ridge.
fn hip_surface_model() -> BuildingModel {
    let mut model = surface_model();
    model.roof_planes.clear();

    let ft = Length::from_feet;
    let slope = Slope::new(Length::from_whole_inches(6), Length::from_whole_inches(12));
    let springing = ft(8.0);
    let ridge_west = Point2::new(ft(4.0), ft(4.0));
    let ridge_east = Point2::new(ft(8.0), ft(4.0));

    model.roof_planes.push(RoofPlane::new(
        "roof-east",
        "East hip end",
        "level-1",
        "system-roof",
        vec![
            Point2::new(ft(12.0), Length::ZERO),
            Point2::new(ft(12.0), ft(8.0)),
            ridge_east,
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
            Point2::new(ft(12.0), ft(8.0)),
            Point2::new(Length::ZERO, ft(8.0)),
            ridge_west,
            ridge_east,
        ],
        slope,
        0,
        springing,
    ));
    model.roof_planes.push(RoofPlane::new(
        "roof-south",
        "South hip field",
        "level-1",
        "system-roof",
        vec![
            Point2::new(Length::ZERO, Length::ZERO),
            Point2::new(ft(12.0), Length::ZERO),
            ridge_east,
            ridge_west,
        ],
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
            Point2::new(Length::ZERO, ft(8.0)),
            Point2::new(Length::ZERO, Length::ZERO),
            ridge_west,
        ],
        slope,
        0,
        springing,
    ));
    model
}

fn build(model: &BuildingModel, selection: &Selection) -> Scene3d {
    Scene3d::from_project(
        model,
        &empty_plan(),
        0,
        selection,
        WorkspaceMode::Design,
        WallDisplay::Outline,
    )
    .expect("a model with surfaces builds a scene")
}

fn pick_clicks(scene: &Scene3d) -> Vec<ViewClick> {
    scene.picks.iter().map(|pick| pick.click.clone()).collect()
}

#[test]
fn surfaces_emit_geometry_and_pick_volumes() {
    let scene = build(&surface_model(), &Selection::Wall);
    // Each surface is a double-faced sheet (two triangles per side for a quad),
    // so geometry is present and the framing points include the surface corners.
    assert!(!scene.vertices.is_empty(), "no surface geometry emitted");
    assert!(
        scene.points.len() >= 12,
        "surfaces did not feed the framing"
    );

    let clicks = pick_clicks(&scene);
    assert!(
        clicks
            .iter()
            .any(|c| matches!(c, ViewClick::RoofPlane { id } if id == "roof-1")),
        "no roof-plane pick volume emitted"
    );
    assert!(
        clicks
            .iter()
            .any(|c| matches!(c, ViewClick::Ceiling { id } if id == "ceiling-1")),
    );
    assert!(
        clicks
            .iter()
            .any(|c| matches!(c, ViewClick::FloorDeck { id } if id == "deck-1")),
    );
}

#[test]
fn generated_floor_and_ceiling_members_emit_plan_meshes_and_picks() {
    let model = surface_model();
    let plan = framer_solver::generate_project_plan(&model).unwrap();
    assert!(plan.floor_plans.iter().any(|plan| !plan.members.is_empty()));
    assert!(
        plan.ceiling_plans
            .iter()
            .any(|plan| !plan.members.is_empty())
    );

    let scene = Scene3d::from_project(
        &model,
        &plan,
        0,
        &Selection::Wall,
        WorkspaceMode::Plan,
        WallDisplay::Outline,
    )
    .unwrap();

    for source_id in ["deck-1", "ceiling-1"] {
        let pick = scene
            .picks
            .iter()
            .find(|pick| {
                matches!(
                    &pick.click,
                    ViewClick::Member { source_id: source, .. } if source == source_id
                )
            })
            .unwrap_or_else(|| panic!("no generated member pick for {source_id}"));
        let PickShape::Mesh { points, triangles } = &pick.shape else {
            panic!("generated {source_id} member must use its shared indexed mesh");
        };
        assert!(!points.is_empty());
        assert!(!triangles.is_empty());
        assert!(
            points
                .iter()
                .all(|point| scene.points.iter().any(|scene_point| {
                    scene_point.x == point.x && scene_point.y == point.y && scene_point.z == point.z
                }))
        );
        let rendered = points[triangles[0][0]];
        assert!(
            scene
                .vertices
                .iter()
                .any(|vertex| { vertex.position == [rendered.x, rendered.y, rendered.z] })
        );
    }
}

#[test]
fn roof_surface_is_sloped_and_decks_sit_at_their_elevations() {
    let scene = build(&surface_model(), &Selection::Wall);
    let zs: Vec<f32> = scene.vertices.iter().map(|v| v.position[2]).collect();
    let lo = zs.iter().cloned().fold(f32::INFINITY, f32::min);
    let hi = zs.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    // Floor deck at level elevation 0; roof ridge weather face at
    // 96 + 96*(6/12) + 7" assembly lift = 151".
    assert!((lo - 0.0).abs() < 0.5, "lowest surface z {lo}, want ~0");
    assert!(
        (hi - 151.0).abs() < 0.5,
        "highest surface z {hi}, want ~151"
    );
    // Pin the flat ceiling at level top (108") − height (12") = 96". It shares
    // the roof's eave elevation, so check a fully-horizontal triangle (which the
    // sloped roof never produces) rather than the raw z range — otherwise a
    // regression in the ceiling formula anywhere in (0, 144) would slip through.
    let flat_zs: Vec<f32> = horizontal_triangle_elevations(&scene);
    assert!(
        flat_zs.iter().any(|z| (z - 96.0).abs() < 0.5),
        "no horizontal ceiling surface at ~96in: {flat_zs:?}"
    );
    assert!(
        flat_zs.iter().any(|z| z.abs() < 0.5),
        "no horizontal floor surface at ~0in: {flat_zs:?}"
    );
    // The geometry is finite (no NaN normals from a degenerate fan).
    for v in &scene.vertices {
        assert!(v.position.iter().all(|c| c.is_finite()));
        assert!(v.normal.iter().all(|c| c.is_finite()));
    }
}

#[test]
fn roof_surface_overhang_expands_bounds_lowers_tail_and_is_pickable() {
    let mut model = surface_model();
    model.roof_planes[0].eave_overhang = Length::from_whole_inches(12);
    model.roof_planes[0].rake_overhang = Length::from_whole_inches(8);
    let scene = build(&model, &Selection::Wall);
    let roof_pick = scene
        .picks
        .iter()
        .find(|pick| matches!(&pick.click, ViewClick::RoofPlane { id } if id == "roof-1"))
        .expect("roof pick surface");
    let PickShape::Mesh {
        points: outline, ..
    } = &roof_pick.shape
    else {
        panic!("a roof must pick from its derived surface mesh");
    };
    let min = |axis: fn(&Point3) -> f32| outline.iter().map(axis).fold(f32::INFINITY, f32::min);
    let max = |axis: fn(&Point3) -> f32| outline.iter().map(axis).fold(f32::NEG_INFINITY, f32::max);
    assert!((min(|p| p.x) + 8.0).abs() < 0.5);
    assert!((max(|p| p.x) - 152.0).abs() < 0.5);
    assert!((min(|p| p.y) + 12.0).abs() < 0.5);
    assert!((max(|p| p.y) - 96.0).abs() < 0.5);
    // 6:12 over a 12in plan tail drops the bearing face 6in; the 7in roof
    // assembly lift leaves the weather-face pick outline at 97in.
    assert!((min(|p| p.z) - 97.0).abs() < 0.5);

    let drawing = Rect::from_min_size((0.0, 0.0).into(), (600.0, 400.0).into());
    let projector = OrbitProjector::from_points(&scene.points, drawing, View3dState::default())
        .expect("projector");
    let overhang_only = Point3::vector(72.0, -6.0, 100.0);
    match scene.pick(projector.project_point(overhang_only).pos, &projector) {
        Some(ViewClick::RoofPlane { id }) => assert_eq!(id, "roof-1"),
        _ => panic!("expected to pick the roof in its overhang-only region"),
    }
}

#[test]
fn roof_opening_is_absent_from_render_and_pick_triangles() {
    let mut model = surface_model();
    let hole_center = Point2::new(Length::from_feet(6.0), Length::from_feet(4.0));
    model.roof_planes[0].openings.push(RoofOpening::new(
        "skylight-test",
        framer_core::OpeningKind::Skylight,
        hole_center,
        Length::from_feet(2.0),
        Length::from_feet(2.0),
    ));
    let scene = build(&model, &Selection::Wall);
    let roof_pick = scene
        .picks
        .iter()
        .find(|pick| matches!(&pick.click, ViewClick::RoofPlane { id } if id == "roof-1"))
        .expect("roof pick mesh");
    let PickShape::Mesh { points, triangles } = &roof_pick.shape else {
        panic!("roof cavities require indexed pick triangles");
    };
    let center = (hole_center.x.inches() as f32, hole_center.y.inches() as f32);
    for triangle in triangles {
        let centroid_x = triangle.iter().map(|index| points[*index].x).sum::<f32>() / 3.0;
        let centroid_y = triangle.iter().map(|index| points[*index].y).sum::<f32>() / 3.0;
        assert!(
            (centroid_x - center.0).abs() >= 12.0 || (centroid_y - center.1).abs() >= 12.0,
            "a pick/render triangle filled the modeled skylight cavity"
        );
    }
}

#[test]
fn invalid_roof_cavities_keep_a_holeless_render_and_pick_fallback() {
    let mut model = surface_model();
    for (id, x) in [("skylight-a", 6.0), ("skylight-b", 6.5)] {
        model.roof_planes[0].openings.push(RoofOpening::new(
            id,
            framer_core::OpeningKind::Skylight,
            Point2::new(Length::from_feet(x), Length::from_feet(4.0)),
            Length::from_feet(2.0),
            Length::from_feet(2.0),
        ));
    }
    assert!(
        model
            .roof_surface_triangulation(&model.roof_planes[0])
            .is_none(),
        "overlapping cavity rings must fail closed in physical geometry"
    );

    let scene = build(&model, &Selection::Wall);
    let roof_pick = scene
        .picks
        .iter()
        .find(|pick| matches!(&pick.click, ViewClick::RoofPlane { id } if id == "roof-1"))
        .expect("invalid cavities must not hide the host roof");
    let PickShape::Mesh { points, triangles } = &roof_pick.shape else {
        panic!("the fallback roof must retain an indexed pick mesh");
    };
    assert_eq!(points.len(), 4);
    assert_eq!(triangles.len(), 2);
    assert!(
        triangles
            .iter()
            .flatten()
            .all(|index| *index < points.len())
    );
}

#[test]
fn wall_member_picks_use_the_owning_plan_even_when_provenance_is_an_opening() {
    let model = BuildingModel::demo_wall();
    let plan = framer_solver::generate_project_plan(&model).unwrap();
    let wall_plan = plan.wall_plans.first().expect("demo wall plan");
    let opening_member = wall_plan
        .members
        .iter()
        .find(|member| member.source != wall_plan.wall)
        .expect("opening-owned framing member");
    assert!(
        model.walls[0]
            .openings
            .iter()
            .any(|opening| opening.id == opening_member.source)
    );

    let scene = Scene3d::from_project(
        &model,
        &plan,
        0,
        &Selection::Wall,
        WorkspaceMode::Plan,
        WallDisplay::Outline,
    )
    .unwrap();
    assert!(scene.picks.iter().any(|pick| matches!(
        &pick.click,
        ViewClick::Member { source_id, member_id }
            if source_id == &wall_plan.wall.0 && member_id == &opening_member.id
    )));
    assert!(scene.picks.iter().all(|pick| !matches!(
        &pick.click,
        ViewClick::Member { source_id, member_id }
            if source_id == &opening_member.source.0 && member_id == &opening_member.id
    )));
}

#[test]
fn generated_roof_members_are_plan_only_and_roof_skin_is_transparent() {
    let model = surface_model();
    let plan = framer_solver::generate_project_plan(&model).unwrap();
    let design = Scene3d::from_project(
        &model,
        &plan,
        0,
        &Selection::Wall,
        WorkspaceMode::Design,
        WallDisplay::Outline,
    )
    .unwrap();
    let plan_scene = Scene3d::from_project(
        &model,
        &plan,
        0,
        &Selection::Wall,
        WorkspaceMode::Plan,
        WallDisplay::Outline,
    )
    .unwrap();

    assert!(
        design
            .picks
            .iter()
            .all(|pick| !matches!(pick.click, ViewClick::Member { .. })),
        "Design must remain authored-assembly only"
    );
    assert!(plan.roof_plans.iter().any(|roof| !roof.members.is_empty()));
    for roof in &plan.roof_plans {
        for member in &roof.members {
            assert!(
                member.sloped.is_some(),
                "{} lacks spatial endpoints",
                member.id
            );
            assert!(plan_scene.picks.iter().any(|pick| matches!(
                &pick.click,
                ViewClick::Member { source_id, member_id }
                    if source_id == &roof.roof.0 && member_id == &member.id
            )));
        }
    }
    // Roof framing depth starts at the assembly's conditioned-side offset and
    // extends outward; it must not be centered halfway below the bearing plane.
    let (rafter_host, rafter) = plan
        .roof_plans
        .iter()
        .flat_map(|roof| roof.members.iter().map(move |member| (&roof.roof, member)))
        .find(|(_, member)| member.kind == MemberKind::Rafter)
        .expect("a common rafter");
    let rafter_pick = plan_scene
        .picks
        .iter()
        .find(|pick| {
            matches!(
                &pick.click,
                ViewClick::Member { source_id, member_id }
                    if source_id == &rafter_host.0 && member_id == &rafter.id
            )
        })
        .unwrap();
    let corners: &[Point3] = match &rafter_pick.shape {
        PickShape::Cuboid(corners) => corners,
        PickShape::Mesh { points, .. } => points,
        _ => panic!("a spatial roof member must use solid pick geometry"),
    };
    let sloped = rafter.sloped.unwrap();
    let start = Point3::vector(
        sloped.start.x.inches() as f32,
        sloped.start.y.inches() as f32,
        sloped.low_elevation.inches() as f32,
    );
    let end = Point3::vector(
        sloped.end.x.inches() as f32,
        sloped.end.y.inches() as f32,
        sloped.high_elevation.inches() as f32,
    );
    let along = normalized(vector_between(start, end)).unwrap();
    let plan_length = (along.x * along.x + along.y * along.y).sqrt();
    let across = Point3::vector(-along.y / plan_length, along.x / plan_length, 0.0);
    let mut section = normalized(cross(along, across)).unwrap();
    if section.z < 0.0 {
        section = -section;
    }
    let section_positions: Vec<f32> = corners
        .iter()
        .map(|corner| vector_between(start, *corner).dot(section))
        .collect();
    let section_min = section_positions
        .iter()
        .copied()
        .fold(f32::INFINITY, f32::min);
    let section_max = section_positions
        .iter()
        .copied()
        .fold(f32::NEG_INFINITY, f32::max);
    assert!((section_min - rafter.side_offset.inches() as f32).abs() < 0.1);
    assert!((section_max - (rafter.side_offset + rafter.side_depth).inches() as f32).abs() < 0.1);
    assert_eq!(design.transparent_index_count, 0);
    let expected_transparent_indices = model
        .roof_planes
        .iter()
        .map(|plane| {
            framer_core::triangulate_simple_polygon(&model.roof_surface_outline(plane)).len() as u32
                * 3
        })
        .sum::<u32>();
    assert_eq!(
        plan_scene.transparent_index_count, expected_transparent_indices,
        "Plan uses one translucent weather face per roof field so alpha is not double-applied"
    );
    let transparent_start = plan_scene.opaque_index_count as usize;
    assert!(
        plan_scene.indices[transparent_start..]
            .iter()
            .all(|index| { plan_scene.vertices[*index as usize].color[3] < 1.0 }),
        "the Plan transparent pass should contain translucent roof sheets"
    );
}

#[test]
fn common_stick_rafter_has_plumb_ends_and_a_matched_wall_birdsmouth() {
    let mut model = elevated_gable_model();
    for plane in &mut model.roof_planes {
        plane.eave_overhang = Length::from_whole_inches(12);
    }
    let plan = framer_solver::generate_project_plan(&model).unwrap();
    let roof_plan = plan
        .roof_plans
        .iter()
        .find(|plan| plan.roof.0 == "roof-south")
        .unwrap();
    let member = roof_plan
        .members
        .iter()
        .find(|member| member.kind == MemberKind::Rafter)
        .unwrap();
    let plane = model
        .roof_planes
        .iter()
        .find(|plane| plane.id == roof_plan.roof)
        .unwrap();
    let sloped = member.sloped.unwrap();
    let ridge_boards: Vec<_> = plan
        .roof_plans
        .iter()
        .flat_map(|roof_plan| &roof_plan.members)
        .filter(|member| member.kind == MemberKind::RidgeBoard)
        .collect();
    let south_ridge_setback = ridge_face_setback(member, &ridge_boards).unwrap();
    let prism = build_common_rafter_solid(
        member,
        plane,
        matched_bearing_depth(&model, plane).map(|depth| depth.inches()),
        Some(south_ridge_setback),
    )
    .unwrap();

    assert_eq!(
        prism.profile.len(),
        7,
        "birdsmouth adds heel + seat vertices"
    );
    assert!((prism.profile[0][0] - prism.profile[6][0]).abs() < 1.0e-3);
    assert!((prism.profile[4][0] - prism.profile[5][0]).abs() < 1.0e-3);
    assert!((south_ridge_setback - 0.75).abs() < 1.0e-3);
    let south_plan_run = (sloped.end.y - sloped.start.y).abs().inches();
    assert!(
        (prism.profile[4][0] - (south_plan_run - south_ridge_setback)).abs() < 1.0e-3,
        "the ridge plumb cut terminates at the near ridge-board face"
    );
    assert!(
        (prism.profile[1][0] - prism.profile[2][0]).abs() < 1.0e-3,
        "heel cut must be vertical"
    );
    assert!(
        (prism.profile[2][1] - prism.profile[3][1]).abs() < 1.0e-3,
        "birdsmouth seat must be horizontal"
    );
    assert!(prism.profile[3][0] > prism.profile[2][0]);

    let north_plan = plan
        .roof_plans
        .iter()
        .find(|plan| plan.roof.0 == "roof-north")
        .unwrap();
    let north_member = north_plan
        .members
        .iter()
        .find(|member| member.kind == MemberKind::Rafter)
        .unwrap();
    let north_plane = model
        .roof_planes
        .iter()
        .find(|plane| plane.id == north_plan.roof)
        .unwrap();
    let north_sloped = north_member.sloped.unwrap();
    let north_ridge_setback = ridge_face_setback(north_member, &ridge_boards).unwrap();
    assert!(
        north_sloped.end.y < north_sloped.start.y,
        "north field runs toward -y"
    );
    let north_prism = build_common_rafter_solid(
        north_member,
        north_plane,
        matched_bearing_depth(&model, north_plane).map(|depth| depth.inches()),
        Some(north_ridge_setback),
    )
    .unwrap();
    assert_eq!(
        north_prism.profile.len(),
        7,
        "reverse field keeps the cut profile"
    );
    assert!((north_prism.profile[2][1] - north_prism.profile[3][1]).abs() < 1.0e-3);
    assert!((north_ridge_setback - 0.75).abs() < 1.0e-3);
    let north_plan_run = (north_sloped.end.y - north_sloped.start.y).abs().inches();
    assert!(
        (north_prism.profile[4][0] - (north_plan_run - north_ridge_setback)).abs() < 1.0e-3,
        "the reverse field also stops at its near ridge-board face"
    );

    let scene = Scene3d::from_project(
        &model,
        &plan,
        0,
        &Selection::Wall,
        WorkspaceMode::Plan,
        WallDisplay::Outline,
    )
    .unwrap();
    let pick = scene
        .picks
        .iter()
        .find(|pick| {
            matches!(
                &pick.click,
                ViewClick::Member { source_id, member_id }
                    if source_id == &roof_plan.roof.0 && member_id == &member.id
            )
        })
        .unwrap();
    let PickShape::Mesh { points, triangles } = &pick.shape else {
        panic!("a cut common rafter must pick from its rendered profile mesh");
    };
    assert_eq!(points.len(), prism.solid.surface.points.len());
    assert_eq!(triangles, &prism.solid.surface.triangles);
    for (pick_point, shared_point) in points.iter().zip(&prism.solid.surface.points) {
        assert!((pick_point.x as f64 - shared_point.x).abs() < 1.0e-4);
        assert!((pick_point.y as f64 - shared_point.y).abs() < 1.0e-4);
        assert!((pick_point.z as f64 - shared_point.z).abs() < 1.0e-4);
    }
    let triangle = triangles[0].map(|index| points[index]);
    let centroid = Point3::vector(
        triangle.iter().map(|point| point.x).sum::<f32>() / 3.0,
        triangle.iter().map(|point| point.y).sum::<f32>() / 3.0,
        triangle.iter().map(|point| point.z).sum::<f32>() / 3.0,
    );
    let drawing = Rect::from_min_size((0.0, 0.0).into(), (600.0, 400.0).into());
    let projector = OrbitProjector::from_points(&scene.points, drawing, View3dState::default())
        .expect("projector");
    assert!(
        pick.hit_depth(projector.project_point(centroid).pos, &projector)
            .is_some(),
        "the rendered cut mesh must remain pickable"
    );

    let (ridge_host, ridge) = plan
        .roof_plans
        .iter()
        .flat_map(|roof| roof.members.iter().map(move |member| (&roof.roof, member)))
        .find(|(_, member)| member.kind == MemberKind::RidgeBoard)
        .unwrap();
    let ridge_pick = scene
        .picks
        .iter()
        .find(|pick| {
            matches!(
                &pick.click,
                ViewClick::Member { source_id, member_id }
                    if source_id == &ridge_host.0 && member_id == &ridge.id
            )
        })
        .unwrap();
    assert!(matches!(ridge_pick.shape, PickShape::Mesh { .. }));

    let (blocking_host, blocking) = plan
        .roof_plans
        .iter()
        .flat_map(|roof| roof.members.iter().map(move |member| (&roof.roof, member)))
        .find(|(_, member)| member.kind == MemberKind::Blocking)
        .unwrap();
    let blocking_pick = scene
        .picks
        .iter()
        .find(|pick| {
            matches!(
                &pick.click,
                ViewClick::Member { source_id, member_id }
                    if source_id == &blocking_host.0 && member_id == &blocking.id
            )
        })
        .unwrap();
    assert!(matches!(blocking_pick.shape, PickShape::Mesh { .. }));
}

#[test]
fn ridge_face_setback_rejects_unrelated_ridge_boards() {
    let model = elevated_gable_model();
    let plan = framer_solver::generate_project_plan(&model).unwrap();
    let rafter = plan
        .roof_plans
        .iter()
        .find(|roof_plan| roof_plan.roof.0 == "roof-south")
        .unwrap()
        .members
        .iter()
        .find(|member| member.kind == MemberKind::Rafter)
        .unwrap();
    let ridge = plan
        .roof_plans
        .iter()
        .flat_map(|roof_plan| &roof_plan.members)
        .find(|member| member.kind == MemberKind::RidgeBoard)
        .unwrap();

    let mut wrong_elevation = ridge.clone();
    let placement = wrong_elevation.sloped.as_mut().unwrap();
    placement.low_elevation += Length::from_whole_inches(1);
    placement.high_elevation += Length::from_whole_inches(1);
    assert_eq!(
        ridge_face_setback(rafter, &[&wrong_elevation]),
        None,
        "a ridge at another elevation cannot shorten this rafter"
    );

    let mut off_span = ridge.clone();
    let placement = off_span.sloped.as_mut().unwrap();
    placement.start.y += Length::from_whole_inches(1);
    placement.end.y += Length::from_whole_inches(1);
    assert_eq!(
        ridge_face_setback(rafter, &[&off_span]),
        None,
        "a nearby ridge whose span misses the endpoint cannot shorten this rafter"
    );
    assert_eq!(
        ridge_face_setback(rafter, &[&wrong_elevation, &off_span, ridge]),
        Some(0.75),
        "decoy ridges are skipped before the actual bearing ridge is selected"
    );
}

#[test]
fn unmatched_or_truss_roofs_do_not_receive_a_birdsmouth_profile() {
    let mut model = surface_model();
    let plan = framer_solver::generate_project_plan(&model).unwrap();
    let member = plan.roof_plans[0]
        .members
        .iter()
        .find(|member| member.kind == MemberKind::Rafter)
        .unwrap()
        .clone();
    let plane = model.roof_planes[0].clone();
    let ridge_boards: Vec<_> = plan
        .roof_plans
        .iter()
        .flat_map(|roof_plan| &roof_plan.members)
        .filter(|member| member.kind == MemberKind::RidgeBoard)
        .collect();
    let ridge_setback = ridge_face_setback(&member, &ridge_boards);
    assert_eq!(ridge_setback, None, "a shed roof has no ridge-board face");
    let prism = build_common_rafter_solid(
        &member,
        &plane,
        matched_bearing_depth(&model, &plane).map(|depth| depth.inches()),
        ridge_setback,
    )
    .unwrap();
    assert_eq!(
        prism.profile.len(),
        4,
        "an unmatched roof gets plumb ends only"
    );

    let roof_system = model
        .systems
        .iter_mut()
        .find(|system| system.id == plane.system)
        .unwrap();
    roof_system
        .layers
        .iter_mut()
        .find_map(|layer| layer.framing.as_mut())
        .unwrap()
        .member_family = MemberFamily::Truss;
    let truss_plan = framer_solver::generate_project_plan(&model).unwrap();
    let scene = Scene3d::from_project(
        &model,
        &truss_plan,
        0,
        &Selection::Wall,
        WorkspaceMode::Plan,
        WallDisplay::Outline,
    )
    .unwrap();
    let pick = scene
        .picks
        .iter()
        .find(|pick| {
            matches!(&pick.click, ViewClick::Member { member_id, .. } if member_id == &member.id)
        })
        .unwrap();
    assert!(matches!(pick.shape, PickShape::Mesh { .. }));
}

#[test]
fn gable_profiles_extend_all_wall_modes_at_absolute_level_elevation() {
    let model = elevated_gable_model();
    let profiles = model.gable_wall_profiles();
    assert_eq!(profiles.len(), 2);

    let outline = Scene3d::from_project(
        &model,
        &empty_plan(),
        0,
        &Selection::Wall,
        WorkspaceMode::Design,
        WallDisplay::Outline,
    )
    .unwrap();
    assert!(
        outline
            .outline_edges
            .iter()
            .all(|edge| edge.a.z >= 120.0 && edge.b.z >= 120.0),
        "stacked-level walls must not fall back to world z=0"
    );
    assert!(outline.outline_edges.iter().any(|edge| {
        [edge.a, edge.b]
            .iter()
            .any(|point| (point.y - 48.0).abs() < 0.5 && (point.z - 240.0).abs() < 0.5)
    }));
    assert_eq!(
        outline
            .picks
            .iter()
            .filter(|pick| matches!(pick.shape, PickShape::GablePrism(_)))
            .count(),
        2,
        "each gable end needs a triangular wall pick prism"
    );

    for display in [WallDisplay::Full, WallDisplay::Width] {
        let scene = Scene3d::from_project(
            &model,
            &empty_plan(),
            0,
            &Selection::Wall,
            WorkspaceMode::Design,
            display,
        )
        .unwrap();
        assert!(
            scene.vertices.iter().any(|vertex| {
                (vertex.position[1] - 48.0).abs() < 0.5
                    && (vertex.position[2] - 240.0).abs() < 0.5
                    && (vertex.position[0] - 144.0).abs() > 0.5
                    && (vertex.position[0] - 144.0).abs() < 12.0
            }),
            "{display:?} omitted the wall-system gable prism"
        );
    }
}

#[test]
fn full_height_opening_under_gable_closes_each_layer_base() {
    let model = elevated_gable_model();
    let mut wall = model
        .walls
        .iter()
        .find(|wall| wall.id.0 == "wall-east")
        .unwrap()
        .clone();
    wall.openings.push(framer_core::Opening::door(
        "opening-full-height",
        "Full-height opening",
        Length::from_feet(4.0),
        Length::from_feet(3.0),
        wall.height,
    ));
    let profile = model.gable_wall_profiles().get(&wall.id).copied().unwrap();
    let mut builder = SceneBuilder::default();
    builder.push_gable_layer(&wall, &profile, -3.0, 3.0, Color32::WHITE);

    // Two triangular faces + two rake quads = 18 indices. The uncovered
    // full-height opening adds one base quad (6 more indices).
    assert_eq!(builder.indices.len(), 24);
    assert!(builder.vertices[14..].iter().all(|vertex| {
        (vertex.position[2] - profile.base_elevation.inches() as f32).abs() < 0.01
            && vertex.normal[2] < -0.99
    }));
}

#[test]
fn gable_rake_plates_use_spatial_member_prisms_and_source_picks() {
    let model = elevated_gable_model();
    let plan = framer_solver::generate_project_plan(&model).unwrap();
    let scene = Scene3d::from_project(
        &model,
        &plan,
        0,
        &Selection::Wall,
        WorkspaceMode::Plan,
        WallDisplay::Outline,
    )
    .unwrap();
    let rake_plates: Vec<_> = plan
        .wall_plans
        .iter()
        .flat_map(|wall| wall.members.iter())
        .filter(|member| member.kind == MemberKind::RakePlate)
        .collect();
    assert_eq!(rake_plates.len(), 4);
    for member in rake_plates {
        let sloped = member.sloped.expect("rake plate endpoints");
        assert!(sloped.high_elevation > sloped.low_elevation);
        let pick = scene
            .picks
            .iter()
            .find(|pick| {
                matches!(
                    &pick.click,
                    ViewClick::Member { source_id, member_id }
                        if source_id == &member.source.0 && member_id == &member.id
                )
            })
            .expect("rake plate source pick");
        let corners = pick_points(pick);
        let wall = model
            .walls
            .iter()
            .find(|wall| wall.id == member.source)
            .unwrap();
        let basis = WallBasis::new(wall);
        let through_positions: Vec<f32> = corners
            .iter()
            .map(|corner| {
                (corner.x - basis.origin_x) * basis.side_x
                    + (corner.y - basis.origin_y) * basis.side_y
            })
            .collect();
        let through_span = through_positions
            .iter()
            .copied()
            .fold(f32::NEG_INFINITY, f32::max)
            - through_positions
                .iter()
                .copied()
                .fold(f32::INFINITY, f32::min);
        assert!(
            (through_span - member.side_depth.inches() as f32).abs() < 0.1,
            "rake plate must use the wall framing-band depth through the wall"
        );

        let start = Point3::vector(
            sloped.start.x.inches() as f32,
            sloped.start.y.inches() as f32,
            sloped.low_elevation.inches() as f32,
        );
        let end = Point3::vector(
            sloped.end.x.inches() as f32,
            sloped.end.y.inches() as f32,
            sloped.high_elevation.inches() as f32,
        );
        let along = normalized(vector_between(start, end)).unwrap();
        let across = Point3::vector(basis.side_x, basis.side_y, 0.0);
        let mut section = normalized(cross(along, across)).unwrap();
        if section.z < 0.0 {
            section = -section;
        }
        let section_positions: Vec<f32> = corners
            .iter()
            .map(|corner| vector_between(start, *corner).dot(section))
            .collect();
        let section_min = section_positions
            .iter()
            .copied()
            .fold(f32::INFINITY, f32::min);
        let section_max = section_positions
            .iter()
            .copied()
            .fold(f32::NEG_INFINITY, f32::max);
        assert!(section_max.abs() < 0.1);
        assert!(
            (section_min + member.cross_section_depth.inches() as f32).abs() < 0.1,
            "rake plate thickness must sit below the roof rake in the wall plane"
        );
    }
}

#[test]
fn diagonal_hip_member_uses_exact_plan_endpoints_and_host_pick() {
    let model = hip_surface_model();
    let plan = framer_solver::generate_project_plan(&model).unwrap();
    let (host, hip) = plan
        .roof_plans
        .iter()
        .flat_map(|host| host.members.iter().map(move |member| (&host.roof, member)))
        .find(|(_, member)| member.kind == MemberKind::HipRafter)
        .expect("rectangular hip fixture emits a hip rafter");
    let sloped = hip.sloped.expect("hip has exact spatial placement");
    assert_ne!(sloped.start.x, sloped.end.x);
    assert_ne!(sloped.start.y, sloped.end.y);
    assert_ne!(sloped.low_elevation, sloped.high_elevation);

    let scene = Scene3d::from_project(
        &model,
        &plan,
        0,
        &Selection::Wall,
        WorkspaceMode::Plan,
        WallDisplay::Outline,
    )
    .unwrap();
    let pick = scene
        .picks
        .iter()
        .find(|pick| {
            matches!(
                &pick.click,
                ViewClick::Member { source_id, member_id }
                    if source_id == &host.0 && member_id == &hip.id
            )
        })
        .expect("diagonal hip member has an owning roof-plan pick");
    let corners = pick_points(pick);
    for point in [sloped.start, sloped.end] {
        assert!(corners.iter().any(|corner| {
            (corner.x - point.x.inches() as f32).abs() < hip.cross_section_depth.inches() as f32
                && (corner.y - point.y.inches() as f32).abs()
                    < hip.cross_section_depth.inches() as f32
        }));
    }
}

#[test]
fn hip_roof_surfaces_emit_four_lifted_pickable_planes() {
    let scene = build(&hip_surface_model(), &Selection::Wall);
    let clicks = pick_clicks(&scene);
    for id in ["roof-east", "roof-north", "roof-south", "roof-west"] {
        assert!(
            clicks
                .iter()
                .any(|c| matches!(c, ViewClick::RoofPlane { id: found } if found == id)),
            "no pick volume emitted for {id}"
        );
    }

    let mut tilted_zs: Vec<f32> = Vec::new();
    let mut up_facing_tilted = 0;
    for tri in scene.indices.chunks_exact(3) {
        let v = |i: u32| scene.vertices[i as usize];
        let (a, b, c) = (v(tri[0]), v(tri[1]), v(tri[2]));
        let tilted = a.normal[2] > 0.1 && a.normal[2] < 0.99;
        if tilted {
            up_facing_tilted += 1;
            tilted_zs.extend([a.position[2], b.position[2], c.position[2]]);
        }
    }
    assert_eq!(
        up_facing_tilted, 6,
        "two trapezoids plus two triangles should emit six up-facing roof triangles"
    );
    let lo = tilted_zs.iter().cloned().fold(f32::INFINITY, f32::min);
    let hi = tilted_zs.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    assert!((lo - 103.0).abs() < 0.5, "hip eaves at {lo}, want ~103in");
    assert!((hi - 127.0).abs() < 0.5, "hip ridge at {hi}, want ~127in");
}

/// The elevations of every fully-horizontal triangle (all three vertices at one
/// z) in the mesh — the flat ceiling/floor surfaces, never the sloped roof.
fn horizontal_triangle_elevations(scene: &Scene3d) -> Vec<f32> {
    scene
        .indices
        .chunks_exact(3)
        .filter_map(|tri| {
            let z = |i: u32| scene.vertices[i as usize].position[2];
            let (a, b, c) = (z(tri[0]), z(tri[1]), z(tri[2]));
            ((a - b).abs() < 1.0e-3 && (a - c).abs() < 1.0e-3).then_some(a)
        })
        .collect()
}

/// A model with one gable roof plane over a 12×8 footprint and **no ceiling** —
/// a cathedral. The roof system stacks a conditioned-side finish (soffit),
/// framing, and roofing, so the weather face and the underside read as distinct
/// colors.
fn cathedral_model() -> BuildingModel {
    let mut model = BuildingModel::new();
    for level in &mut model.levels {
        if level.id.0 == "level-1" {
            level.height = Length::from_whole_inches(108);
        }
    }
    model
        .materials
        .push(Material::solid_color("mat-roof", "Shingle", [44, 46, 52]));
    model.materials.push(Material::solid_color(
        "mat-soffit",
        "Soffit",
        [205, 180, 140],
    ));
    let framing = ConstructionLayer::new(
        LayerFunction::Framing,
        "mat-spf",
        BoardProfile::TwoBySix.nominal_depth(),
    )
    .with_framing(FramingSpec {
        member: BoardProfile::TwoBySix,
        spacing: Length::from_whole_inches(16),
        pattern: FramingPattern::Single,
        member_family: MemberFamily::Rafter,
        cavity_material: None,
    });
    model.systems.push(ConstructionSystem {
        id: ElementId::new("system-roof"),
        name: "Roof".to_owned(),
        kind: SystemKind::Roof,
        source: None,
        layers: vec![
            ConstructionLayer::new(
                LayerFunction::CeilingFinish,
                "mat-soffit",
                Length::from_whole_inches(1),
            ),
            framing,
            ConstructionLayer::new(
                LayerFunction::Roofing,
                "mat-roof",
                Length::from_whole_inches(1),
            ),
        ],
    });
    model.roof_planes.push(RoofPlane::new(
        "roof-1",
        "Roof",
        "level-1",
        "system-roof",
        rect(),
        Slope::new(Length::from_whole_inches(6), Length::from_whole_inches(12)),
        0,
        Length::from_feet(8.0),
    ));
    model
}

/// The (color, min-z) of each sloped triangle facing up vs. down — used to tell
/// the weather face (up) from the cathedral underside (down).
fn sloped_faces(scene: &Scene3d, facing_up: bool) -> Vec<([f32; 4], f32)> {
    scene
        .indices
        .chunks_exact(3)
        .filter_map(|tri| {
            let v = |i: u32| scene.vertices[i as usize];
            let (a, b, c) = (v(tri[0]), v(tri[1]), v(tri[2]));
            let zs = [a.position[2], b.position[2], c.position[2]];
            let sloped = zs.iter().cloned().fold(f32::NEG_INFINITY, f32::max)
                - zs.iter().cloned().fold(f32::INFINITY, f32::min)
                > 1.0;
            let up = a.normal[2] > 0.5;
            let down = a.normal[2] < -0.5;
            (sloped && (if facing_up { up } else { down }))
                .then_some((a.color, zs.iter().cloned().fold(f32::INFINITY, f32::min)))
        })
        .collect()
}

#[test]
fn cathedral_roof_underside_is_a_distinct_lowered_face() {
    let scene = build(&cathedral_model(), &Selection::Wall);
    let weather = sloped_faces(&scene, true);
    let underside = sloped_faces(&scene, false);
    assert!(!weather.is_empty(), "no up-facing weather roof triangles");
    assert!(
        !underside.is_empty(),
        "cathedral roof emitted no down-facing underside"
    );
    // The underside is a distinct (interior-finish) color, not the weather face.
    let weather_color = weather[0].0;
    let underside_color = underside[0].0;
    assert_ne!(
        weather_color, underside_color,
        "cathedral underside should differ from the weather face"
    );
    // ...and the weather face is lifted one assembly-thickness (1 + 6 + 1 =
    // 8in) above the underside.
    let weather_lo = weather.iter().map(|f| f.1).fold(f32::INFINITY, f32::min);
    let underside_lo = underside.iter().map(|f| f.1).fold(f32::INFINITY, f32::min);
    assert!(
        (underside_lo - 96.0).abs() < 0.5,
        "cathedral underside springs at {underside_lo}in, want ~96"
    );
    let lift = weather_lo - underside_lo;
    assert!(
        (lift - 8.0).abs() < 0.5,
        "weather face lifted {lift}in above the underside, want ~8"
    );
}

#[test]
fn roof_with_a_ceiling_below_has_no_distinct_underside() {
    // Cover the footprint with a flat ceiling: the plane is no longer a
    // cathedral, so both roof faces share the weather color.
    let mut model = cathedral_model();
    model.systems.push(finish_system(
        "system-ceiling",
        SystemKind::Ceiling,
        LayerFunction::CeilingFinish,
        "mat-soffit",
        true,
    ));
    model.ceilings.push(Ceiling::new(
        "ceiling-1",
        "Ceiling",
        "level-1",
        "system-ceiling",
        SurfaceRegion::Polygon(rect()),
        Length::from_whole_inches(12),
    ));
    let scene = build(&model, &Selection::Wall);
    let weather = sloped_faces(&scene, true);
    let underside = sloped_faces(&scene, false);
    assert!(!weather.is_empty() && !underside.is_empty());
    // Every sloped down-face matches the weather color (no cathedral underside).
    assert!(
        underside.iter().all(|f| f.0 == weather[0].0),
        "a roof with a ceiling below should not recolor its underside"
    );
}

#[test]
fn sloped_ceiling_is_lifted_via_the_frame_in_the_mesher() {
    // Slice A4: the app mesher lifts a sloped ceiling onto its plane via the
    // shared frame, instead of drawing it at a constant elevation. Isolate the
    // ceiling as the only tilted geometry (the surface_model roof is removed; the
    // floor stays horizontal), then a 6:12 slope over the 8ft run rises the
    // ceiling from its 96" springing to 144".
    let mut model = surface_model();
    model.roof_planes.clear();
    model.ceilings[0].slope = Some(framer_core::CeilingSlope::new(
        Slope::new(Length::from_whole_inches(6), Length::from_whole_inches(12)),
        0,
    ));
    let scene = build(&model, &Selection::Wall);

    // Tilted triangles (normal has both a vertical and a horizontal component) are
    // the lifted ceiling; a flat ceiling would have purely vertical normals. Span
    // all three vertices of each tilted triangle so the elevation range does not
    // depend on the triangulator's vertex order.
    let mut tilted_zs: Vec<f32> = Vec::new();
    for tri in scene.indices.chunks_exact(3) {
        let nz = scene.vertices[tri[0] as usize].normal[2].abs();
        if nz > 0.1 && nz < 0.99 {
            tilted_zs.extend(tri.iter().map(|&i| scene.vertices[i as usize].position[2]));
        }
    }
    assert!(
        !tilted_zs.is_empty(),
        "the sloped ceiling is lifted via the frame, not drawn flat"
    );
    let lo = tilted_zs.iter().cloned().fold(f32::INFINITY, f32::min);
    let hi = tilted_zs.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    assert!((lo - 96.0).abs() < 0.5, "ceiling springs at 96in, got {lo}");
    assert!((hi - 144.0).abs() < 0.5, "ceiling rises to 144in, got {hi}");
}

#[test]
fn surface_is_two_faced_with_opposite_normals() {
    // push_surface deliberately emits the outline twice with opposite normals so
    // the un-culled axonometric pipeline lights it from both sides. Pin that: the
    // flat floor deck's horizontal triangles must include both an up- and a
    // down-facing normal at its elevation.
    let mut model = BuildingModel::new();
    model.systems.push(finish_system(
        "system-floor",
        SystemKind::Floor,
        LayerFunction::InteriorFinish,
        "mat-floor",
        true,
    ));
    model.floor_decks.push(FloorDeck::new(
        "deck-1",
        "Deck",
        "level-1",
        "system-floor",
        SurfaceRegion::Polygon(rect()),
    ));
    let scene = build(&model, &Selection::Wall);
    let normals_z: Vec<f32> = scene
        .indices
        .chunks_exact(3)
        .filter_map(|tri| {
            let v = |i: u32| scene.vertices[i as usize];
            let (a, b, c) = (v(tri[0]), v(tri[1]), v(tri[2]));
            let flat = (a.position[2] - b.position[2]).abs() < 1.0e-3
                && (a.position[2] - c.position[2]).abs() < 1.0e-3;
            flat.then_some(a.normal[2])
        })
        .collect();
    assert!(
        normals_z.iter().any(|n| *n > 0.5),
        "no up-facing floor triangle"
    );
    assert!(
        normals_z.iter().any(|n| *n < -0.5),
        "no down-facing floor triangle (surface is not two-faced)"
    );
}

#[test]
fn clicking_a_roof_surface_picks_it() {
    let scene = build(&surface_model(), &Selection::Wall);
    let drawing = Rect::from_min_size((0.0, 0.0).into(), (600.0, 400.0).into());
    let projector = OrbitProjector::from_points(&scene.points, drawing, View3dState::default())
        .expect("a projector for the surface points");
    // Aim at the roof's plan centroid (6ft, 4ft); the projected point must land
    // on the roof surface and pick it.
    let centroid = Point3::vector(
        Length::from_feet(6.0).inches() as f32,
        Length::from_feet(4.0).inches() as f32,
        // The weather-face elevation at the centroid: springing 96" + 48"
        // up-slope × 6/12 + 7" assembly lift = 127".
        127.0,
    );
    let screen = projector.project_point(centroid).pos;
    match scene.pick(screen, &projector) {
        Some(ViewClick::RoofPlane { id }) => assert_eq!(id, "roof-1"),
        _ => panic!("expected to pick the roof plane at its centroid"),
    }
}

/// Six corner-joined walls enclosing a concave L-shaped room with a floor deck
/// attached via `SurfaceRegion::Room`.
fn l_shaped_room_model() -> BuildingModel {
    let ft = Length::from_feet;
    let mut model = BuildingModel::new();
    let pts = [
        Point2::new(ft(0.0), ft(0.0)),
        Point2::new(ft(12.0), ft(0.0)),
        Point2::new(ft(12.0), ft(6.0)),
        Point2::new(ft(6.0), ft(6.0)),
        Point2::new(ft(6.0), ft(12.0)),
        Point2::new(ft(0.0), ft(12.0)),
    ];
    for i in 0..pts.len() {
        let next = (i + 1) % pts.len();
        model.walls.push(
            Wall::new(format!("w-{i}"), "Wall", ft(1.0), &model.framing_defaults())
                .with_placement("level-1", pts[i], pts[next]),
        );
    }
    model.rooms.push(Room::new(
        "room-1",
        "L room",
        RoomUsage::default(),
        "level-1",
        Point2::new(ft(3.0), ft(3.0)),
    ));
    model.systems.push(finish_system(
        "system-floor",
        SystemKind::Floor,
        LayerFunction::InteriorFinish,
        "mat-floor",
        true,
    ));
    model.floor_decks.push(FloorDeck::new(
        "deck-1",
        "Deck",
        "level-1",
        "system-floor",
        SurfaceRegion::Room(ElementId::new("room-1")),
    ));
    model
}

#[test]
fn room_region_surface_resolves_concave_loop_and_tiles_it() {
    // The 3D mesher's Room arm: SurfaceRegion::Room ->
    // room_boundary_on_level (a concave L) -> ear-clip -> surface. The deck's
    // pick volume must carry the full 6-vertex L outline (not a convex hull),
    // and its triangulation must tile the L's plan area (no fan spill into the
    // notch).
    let scene = build(&l_shaped_room_model(), &Selection::Wall);
    let deck = scene
        .picks
        .iter()
        .find(|p| matches!(&p.click, ViewClick::FloorDeck { id } if id == "deck-1"))
        .expect("a floor-deck pick volume");
    let PickShape::Mesh {
        points: outline, ..
    } = &deck.shape
    else {
        panic!("a floor deck must pick as a surface mesh, not a cuboid");
    };
    assert_eq!(outline.len(), 6, "the concave L room loop has six vertices");

    // Tile the resolved outline through the same ear-clip the mesher used.
    let plan: Vec<Point2> = outline
        .iter()
        .map(|p| {
            Point2::new(
                Length::from_inches(p.x as f64),
                Length::from_inches(p.y as f64),
            )
        })
        .collect();
    let tris = framer_core::triangulate_simple_polygon(&plan);
    assert_eq!(tris.len(), 4, "an L (6-gon) ear-clips to four triangles");
    let area: f64 = tris
        .iter()
        .map(|&[a, b, c]| framer_core::polygon_area_square_inches(&[plan[a], plan[b], plan[c]]))
        .sum();
    // 12×12 − 6×6 = 108 sq ft = 15552 sq in.
    assert!(
        (area - 15552.0).abs() < 5.0,
        "L deck triangles cover {area} sq in, expected 15552"
    );
}

fn stacked_unenclosed_room_deck_model() -> BuildingModel {
    let ft = Length::from_feet;
    let mut model = BuildingModel::new();
    model
        .levels
        .push(Level::new("level-2", "Level 2", ft(10.0)));
    let outline = rect();
    for i in 0..outline.len() {
        let next = (i + 1) % outline.len();
        model.walls.push(
            Wall::new(format!("w-{i}"), "Wall", ft(1.0), &model.framing_defaults()).with_placement(
                "level-1",
                outline[i],
                outline[next],
            ),
        );
    }
    model.rooms.push(Room::new(
        "room-2",
        "Upper room",
        RoomUsage::Living,
        "level-2",
        Point2::new(ft(6.0), ft(4.0)),
    ));
    model.systems.push(finish_system(
        "system-floor",
        SystemKind::Floor,
        LayerFunction::InteriorFinish,
        "mat-floor",
        true,
    ));
    model.floor_decks.push(FloorDeck::new(
        "deck-2",
        "Upper deck",
        "level-2",
        "system-floor",
        SurfaceRegion::Room(ElementId::new("room-2")),
    ));
    model
}

#[test]
fn room_region_mesh_resolves_against_the_room_level() {
    let model = stacked_unenclosed_room_deck_model();

    assert!(
        region_outline_plan(&model, &model.floor_decks[0].region).is_none(),
        "a level-2 room region must not borrow the level-1 enclosure"
    );

    let scene = build(&model, &Selection::Wall);
    assert!(
        !pick_clicks(&scene)
            .iter()
            .any(|click| matches!(click, ViewClick::FloorDeck { id } if id == "deck-2")),
        "no floor-deck pick volume should be emitted for the unresolved level-2 room"
    );
}
