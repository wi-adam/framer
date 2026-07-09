//! Background progressive path-trace rendering for the in-app Render view.
//!
//! Rendering runs on a worker thread so the UI stays responsive. The worker
//! accumulates samples in growing batches and publishes a tone-mapped RGBA frame
//! after each batch; the UI uploads the latest frame to a texture and repaints
//! until the target sample count is reached. When the camera or model changes,
//! the job is cancelled and a fresh one starts.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use eframe::egui::{ColorImage, Context, TextureHandle, TextureOptions};
use framer_core::{Appearance, BuildingModel, Length, Point2, SpanDirection, SurfaceRegion};
use framer_render::math::Vec3;
use framer_render::{RenderOptions, accumulate, scene_from_model, tonemap_accum};

/// Total samples per pixel the worker converges to before idling.
const TARGET_SPP: u32 = 256;
/// Fixed seed keeps renders reproducible across restarts.
const SEED: u64 = 1;

/// A published, tone-mapped frame.
struct Frame {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
    samples: u32,
}

#[derive(Default)]
struct Shared {
    frame: Option<Frame>,
}

/// A running (or finished) progressive render for one camera/model/size.
struct RenderJob {
    key: u64,
    target_spp: u32,
    shared: Arc<Mutex<Shared>>,
    cancel: Arc<AtomicBool>,
    _handle: JoinHandle<()>,
}

impl RenderJob {
    fn spawn(key: u64, model: BuildingModel, opts: RenderOptions, width: u32, height: u32) -> Self {
        let shared = Arc::new(Mutex::new(Shared::default()));
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_shared = Arc::clone(&shared);
        let worker_cancel = Arc::clone(&cancel);

        let handle = std::thread::spawn(move || {
            let scene = scene_from_model(&model, &opts);
            let mut accum = vec![Vec3::ZERO; (width as usize) * (height as usize)];
            let mut done = 0u32;
            while done < TARGET_SPP {
                if worker_cancel.load(Ordering::Relaxed) {
                    return;
                }
                // Small batches first (instant feedback), growing for efficiency.
                let batch = done.clamp(1, 16).min(TARGET_SPP - done);
                accumulate(&scene, width, height, batch, done, SEED, &mut accum);
                done += batch;
                let rgba = tonemap_accum(&accum, done, opts.exposure);
                if let Ok(mut guard) = worker_shared.lock() {
                    guard.frame = Some(Frame {
                        width,
                        height,
                        rgba,
                        samples: done,
                    });
                }
            }
        });

        Self {
            key,
            target_spp: TARGET_SPP,
            shared,
            cancel,
            _handle: handle,
        }
    }
}

impl Drop for RenderJob {
    fn drop(&mut self) {
        // Signal the worker to stop at the next batch boundary; it detaches and
        // exits on its own (we never block the UI thread on a join).
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// Per-frame UI state owning the current render job and its display texture.
#[derive(Default)]
pub(super) struct RenderViewState {
    job: Option<RenderJob>,
    texture: Option<TextureHandle>,
    uploaded_samples: u32,
}

impl RenderViewState {
    /// Ensures a job matching `key` is running, then uploads its latest frame to
    /// the display texture. Restarting (camera/model changed) keeps showing the
    /// previous texture until the new job produces its first frame.
    pub(super) fn update(
        &mut self,
        ctx: &Context,
        model: &BuildingModel,
        opts: RenderOptions,
        width: u32,
        height: u32,
        key: u64,
    ) {
        if self.job.as_ref().map(|j| j.key) != Some(key) {
            self.job = Some(RenderJob::spawn(key, model.clone(), opts, width, height));
            self.uploaded_samples = 0;
        }

        let Some(job) = &self.job else { return };
        let newer = {
            let guard = job.shared.lock().ok();
            guard.and_then(|g| {
                g.frame
                    .as_ref()
                    .filter(|f| f.samples > self.uploaded_samples)
                    .map(|f| {
                        (
                            ColorImage::from_rgba_unmultiplied(
                                [f.width as usize, f.height as usize],
                                &f.rgba,
                            ),
                            f.samples,
                        )
                    })
            })
        };
        if let Some((image, samples)) = newer {
            match &mut self.texture {
                Some(tex) => tex.set(image, TextureOptions::LINEAR),
                None => {
                    self.texture =
                        Some(ctx.load_texture("framer-render", image, TextureOptions::LINEAR))
                }
            }
            self.uploaded_samples = samples;
        }
    }

    pub(super) fn texture(&self) -> Option<&TextureHandle> {
        self.texture.as_ref()
    }

    pub(super) fn samples(&self) -> u32 {
        self.uploaded_samples
    }

    pub(super) fn target_spp(&self) -> u32 {
        self.job.as_ref().map_or(TARGET_SPP, |j| j.target_spp)
    }

    pub(super) fn is_accumulating(&self) -> bool {
        self.uploaded_samples < self.target_spp()
    }
}

/// A cheap signature of the scene inputs that affect the render, so the job
/// restarts when the model is edited.
pub(super) fn model_signature(model: &BuildingModel) -> u64 {
    let mut sig = RenderSignature::new();

    sig.mix_len(model.materials.len());
    for material in &model.materials {
        sig.mix_str(&material.id.0);
        sig.mix_appearance(&material.appearance);
    }

    sig.mix_len(model.systems.len());
    for system in &model.systems {
        sig.mix_str(&system.id.0);
        sig.mix_i64(system.kind as i64);
        sig.mix_len(system.layers.len());
        for layer in &system.layers {
            sig.mix_i64(layer.function as i64);
            sig.mix_str(&layer.material.0);
            sig.mix_length(layer.thickness);
            match &layer.framing {
                Some(framing) => {
                    sig.mix_i64(1);
                    sig.mix_i64(framing.member as i64);
                    sig.mix_length(framing.spacing);
                    sig.mix_i64(framing.pattern as i64);
                    sig.mix_i64(framing.member_family as i64);
                    sig.mix_option_str(framing.cavity_material.as_ref().map(|id| id.0.as_str()));
                }
                None => sig.mix_i64(0),
            }
        }
    }

    sig.mix_len(model.levels.len());
    for level in &model.levels {
        sig.mix_str(&level.id.0);
        sig.mix_length(level.elevation);
        sig.mix_length(level.height);
    }

    sig.mix_len(model.walls.len());
    for wall in &model.walls {
        sig.mix_str(&wall.id.0);
        sig.mix_str(&wall.level.0);
        sig.mix_point(wall.start);
        sig.mix_point(wall.end);
        sig.mix_length(wall.height);
        sig.mix_length(wall.length);
        sig.mix_str(&wall.system.0);
        sig.mix_len(wall.openings.len());
        for opening in &wall.openings {
            sig.mix_str(&opening.id.0);
            sig.mix_i64(opening.kind as i64);
            sig.mix_length(opening.center);
            sig.mix_length(opening.width);
            sig.mix_length(opening.height);
            sig.mix_length(opening.sill_height);
        }
    }

    sig.mix_len(model.wall_joins.len());
    for join in &model.wall_joins {
        sig.mix_str(&join.id.0);
        sig.mix_i64(join.kind as i64);
        sig.mix_str(&join.first_wall.0);
        sig.mix_str(&join.second_wall.0);
        sig.mix_point(join.point);
    }

    sig.mix_len(model.rooms.len());
    for room in &model.rooms {
        sig.mix_str(&room.id.0);
        sig.mix_str(&room.level.0);
        sig.mix_point(room.seed);
    }

    sig.mix_len(model.roof_planes.len());
    for plane in &model.roof_planes {
        sig.mix_str(&plane.id.0);
        sig.mix_str(&plane.level.0);
        sig.mix_str(&plane.system.0);
        sig.mix_points(&plane.outline);
        sig.mix_slope(plane.slope);
        sig.mix_i64(plane.eave_edge as i64);
        sig.mix_length(plane.reference_elevation);
        sig.mix_length(plane.eave_overhang);
        sig.mix_length(plane.rake_overhang);
        sig.mix_len(plane.openings.len());
        for opening in &plane.openings {
            sig.mix_str(&opening.id.0);
            sig.mix_i64(opening.kind as i64);
            sig.mix_point(opening.center);
            sig.mix_length(opening.width);
            sig.mix_length(opening.height);
        }
    }

    sig.mix_len(model.ceilings.len());
    for ceiling in &model.ceilings {
        sig.mix_str(&ceiling.id.0);
        sig.mix_str(&ceiling.level.0);
        sig.mix_str(&ceiling.system.0);
        sig.mix_region(&ceiling.region);
        sig.mix_length(ceiling.height);
        match ceiling.slope {
            Some(slope) => {
                sig.mix_i64(1);
                sig.mix_slope(slope.pitch);
                sig.mix_i64(slope.low_edge as i64);
            }
            None => sig.mix_i64(0),
        }
    }

    sig.mix_len(model.floor_decks.len());
    for deck in &model.floor_decks {
        sig.mix_str(&deck.id.0);
        sig.mix_str(&deck.level.0);
        sig.mix_str(&deck.system.0);
        sig.mix_region(&deck.region);
        sig.mix_span(deck.span);
    }

    sig.finish()
}

struct RenderSignature {
    h: u64,
}

impl RenderSignature {
    fn new() -> Self {
        Self {
            h: 1469598103934665603, // FNV offset basis
        }
    }

    fn finish(self) -> u64 {
        self.h
    }

    fn mix_i64(&mut self, v: i64) {
        self.h ^= v as u64;
        self.h = self.h.wrapping_mul(1099511628211);
    }

    fn mix_len(&mut self, len: usize) {
        self.mix_i64(len as i64);
    }

    fn mix_str(&mut self, value: &str) {
        self.mix_len(value.len());
        for byte in value.bytes() {
            self.mix_i64(byte as i64);
        }
    }

    fn mix_option_str(&mut self, value: Option<&str>) {
        match value {
            Some(value) => {
                self.mix_i64(1);
                self.mix_str(value);
            }
            None => self.mix_i64(0),
        }
    }

    fn mix_length(&mut self, value: Length) {
        self.mix_i64(value.ticks());
    }

    fn mix_point(&mut self, point: Point2) {
        self.mix_length(point.x);
        self.mix_length(point.y);
    }

    fn mix_points(&mut self, points: &[Point2]) {
        self.mix_len(points.len());
        for point in points {
            self.mix_point(*point);
        }
    }

    fn mix_slope(&mut self, slope: framer_core::Slope) {
        self.mix_length(slope.rise);
        self.mix_length(slope.run);
    }

    fn mix_region(&mut self, region: &SurfaceRegion) {
        match region {
            SurfaceRegion::Room(id) => {
                self.mix_i64(1);
                self.mix_str(&id.0);
            }
            SurfaceRegion::Polygon(points) => {
                self.mix_i64(2);
                self.mix_points(points);
            }
        }
    }

    fn mix_span(&mut self, span: SpanDirection) {
        match span {
            SpanDirection::Shorter => self.mix_i64(1),
            SpanDirection::Along => self.mix_i64(2),
            SpanDirection::Across => self.mix_i64(3),
            SpanDirection::Explicit(point) => {
                self.mix_i64(4);
                self.mix_point(point);
            }
        }
    }

    fn mix_appearance(&mut self, appearance: &Appearance) {
        match appearance {
            Appearance::SolidColor(color) => {
                self.mix_i64(1);
                self.mix_color(*color);
            }
            Appearance::Textured {
                color,
                texture,
                scale,
            } => {
                self.mix_i64(2);
                self.mix_color(*color);
                self.mix_asset(texture);
                self.mix_length(*scale);
            }
            Appearance::DepthMapped {
                color,
                height,
                scale,
            } => {
                self.mix_i64(3);
                self.mix_color(*color);
                self.mix_asset(height);
                self.mix_length(*scale);
            }
        }
    }

    fn mix_color(&mut self, color: [u8; 3]) {
        for channel in color {
            self.mix_i64(channel as i64);
        }
    }

    fn mix_asset(&mut self, asset: &framer_core::AssetRef) {
        self.mix_str(&asset.hash);
        self.mix_str(&asset.media_type);
        self.mix_i64(asset.role as i64);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use framer_core::{Ceiling, FloorDeck, RoofPlane, Slope};

    fn rect() -> Vec<Point2> {
        vec![
            Point2::new(Length::ZERO, Length::ZERO),
            Point2::new(Length::from_feet(12.0), Length::ZERO),
            Point2::new(Length::from_feet(12.0), Length::from_feet(8.0)),
            Point2::new(Length::ZERO, Length::from_feet(8.0)),
        ]
    }

    #[test]
    fn model_signature_reacts_to_authored_surfaces() {
        let mut model = BuildingModel::new();
        let base = model_signature(&model);

        model.roof_planes.push(RoofPlane::new(
            "roof-1",
            "Roof",
            "level-1",
            "system-roof",
            rect(),
            Slope::new(Length::from_whole_inches(4), Length::from_whole_inches(12)),
            0,
            Length::from_feet(8.0),
        ));
        assert_ne!(
            base,
            model_signature(&model),
            "adding a roof must rebuild the render scene"
        );

        let mut model = BuildingModel::new();
        let base = model_signature(&model);
        model.ceilings.push(Ceiling::new(
            "ceiling-1",
            "Ceiling",
            "level-1",
            "system-ceiling",
            SurfaceRegion::Polygon(rect()),
            Length::from_whole_inches(12),
        ));
        assert_ne!(
            base,
            model_signature(&model),
            "adding a ceiling must rebuild the render scene"
        );

        let mut model = BuildingModel::new();
        let base = model_signature(&model);
        model.floor_decks.push(FloorDeck::new(
            "deck-1",
            "Deck",
            "level-1",
            "system-floor",
            SurfaceRegion::Polygon(rect()),
        ));
        assert_ne!(
            base,
            model_signature(&model),
            "adding a floor deck must rebuild the render scene"
        );
    }

    #[test]
    fn model_signature_reacts_to_surface_material_changes() {
        let mut model = BuildingModel::new();
        let base = model_signature(&model);
        model.materials[0].appearance = Appearance::SolidColor([1, 2, 3]);

        assert_ne!(
            base,
            model_signature(&model),
            "material changes must re-upload render scene materials"
        );
    }

    #[test]
    fn model_signature_reacts_to_system_thickness_changes() {
        let mut model = BuildingModel::new();
        let base = model_signature(&model);
        let layer = model
            .systems
            .iter_mut()
            .find_map(|system| system.layers.first_mut())
            .expect("starter model has at least one construction layer");
        layer.thickness += Length::from_whole_inches(1);

        assert_ne!(
            base,
            model_signature(&model),
            "system thickness changes must rebuild render scene geometry"
        );
    }
}
