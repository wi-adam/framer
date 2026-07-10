//! Color and material policy shared by scene element emitters and viewport siblings.

use eframe::egui::{Color32, Rgba};
use framer_core::{AssemblyFace, BuildingModel, ElementId, Material};
use framer_solver::MemberKind;

use super::super::theme;

pub(in crate::app::viewport) fn color_to_rgba(color: Color32) -> [f32; 4] {
    [
        color.r() as f32 / 255.0,
        color.g() as f32 / 255.0,
        color.b() as f32 / 255.0,
        color.a() as f32 / 255.0,
    ]
}

/// Layer bands render in the transparent (non-depth-writing) pass so framing
/// members stay visible inside the wall; this alpha keeps the layered
/// cross-section legible while letting studs show through.
const LAYER_BAND_ALPHA: u8 = 168;

/// A material's representative appearance color as a translucent `Color32` for a
/// layer band, so the colored cross-section reads while framing members inside
/// the wall remain visible through it.
fn material_color_to_rgba(material: &Material) -> Color32 {
    let [r, g, b] = material.color();
    Color32::from(Rgba::from_srgba_unmultiplied(r, g, b, LAYER_BAND_ALPHA))
}

/// The fill color for a layer band: the resolved material color, brightened a
/// touch when the wall is selected.
pub(super) fn layer_band_color(base: Color32, selected: bool) -> Color32 {
    if selected { brighten(base, 24) } else { base }
}

/// Resolve a layer material's color, falling back to a neutral tone when the
/// material id is dangling.
pub(super) fn material_color(model: &BuildingModel, id: &framer_core::ElementId) -> Color32 {
    model
        .material(id)
        .map(material_color_to_rgba)
        .unwrap_or_else(neutral_band_color)
}

/// The neutral fallback band color (translucent) used when a layer or wall has no
/// resolvable material/system.
pub(super) fn neutral_band_color() -> Color32 {
    theme::with_alpha(theme::sheet_ruler(), LAYER_BAND_ALPHA)
}

pub(in crate::app::viewport) fn brighten(color: Color32, amount: u8) -> Color32 {
    Color32::from(Rgba::from_srgba_unmultiplied(
        color.r().saturating_add(amount),
        color.g().saturating_add(amount),
        color.b().saturating_add(amount),
        color.a(),
    ))
}

pub(in crate::app::viewport) fn member_color(kind: MemberKind) -> Color32 {
    match kind {
        MemberKind::BottomPlate
        | MemberKind::TopPlate
        | MemberKind::RakePlate
        | MemberKind::RimJoist => theme::framing_line_dark(),
        MemberKind::CornerPost | MemberKind::RoughSill | MemberKind::ValleyRafter => {
            theme::dimension_line()
        }
        MemberKind::PartitionStud | MemberKind::Header | MemberKind::CeilingJoist => {
            theme::success().gamma_multiply(0.72)
        }
        MemberKind::BackingStud | MemberKind::Blocking => theme::warning().gamma_multiply(0.72),
        MemberKind::CommonStud
        | MemberKind::GableStud
        | MemberKind::FloorJoist
        | MemberKind::Rafter => brighten(theme::framing_line(), 42),
        MemberKind::KingStud | MemberKind::JackStud | MemberKind::CrippleStud => {
            brighten(theme::warning(), 26)
        }
        MemberKind::RidgeBoard | MemberKind::HipRafter => theme::framing_line(),
        MemberKind::JackRafter => brighten(theme::framing_line(), 28),
    }
}

pub(super) fn highlighted_member_color(
    kind: MemberKind,
    source_selected: bool,
    member_selected: bool,
) -> Color32 {
    if member_selected {
        theme::active_blue()
    } else if source_selected {
        brighten(member_color(kind), 20)
    } else {
        member_color(kind)
    }
}

// === roof / ceiling / floor surfaces ===

/// Which finished face of a surface assembly is shown, so it picks the layer the
/// viewer sees and a sensible fallback color.
#[derive(Clone, Copy)]
pub(super) enum SurfaceFace {
    Roof,
    /// A cathedral roof plane's underside — the assembly's conditioned-side finish.
    RoofUnderside,
    Ceiling,
    Floor,
}

/// The fill color of a roof/ceiling/floor surface: the resolved color of its
/// system's representative finish face (the layer selection lives in `framer-core`
/// so this 3-D view and the path-traced render pick the same face), falling back to
/// a neutral tone so it stays visible when the system or material is missing.
pub(super) fn surface_color(
    model: &BuildingModel,
    system_id: &ElementId,
    face: SurfaceFace,
) -> Color32 {
    let (fallback, assembly_face) = match face {
        SurfaceFace::Roof => (theme::dimension_line(), AssemblyFace::Finished),
        SurfaceFace::RoofUnderside => (theme::sheet_ruler(), AssemblyFace::Underside),
        SurfaceFace::Ceiling => (theme::sheet_ruler(), AssemblyFace::Finished),
        SurfaceFace::Floor => (theme::framing_line(), AssemblyFace::Finished),
    };
    model
        .systems
        .iter()
        .find(|system| system.id == *system_id)
        .and_then(|system| system.surface_finish_material(assembly_face))
        .and_then(|material| model.material(material))
        .map(|material| {
            let [r, g, b] = material.color();
            Color32::from_rgb(r, g, b)
        })
        .unwrap_or(fallback)
}
