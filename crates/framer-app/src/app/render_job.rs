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
use framer_core::BuildingModel;
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

/// A cheap signature of the geometry that affects the render, so the job
/// restarts when the model is edited.
pub(super) fn model_signature(model: &BuildingModel) -> u64 {
    let mut h: u64 = 1469598103934665603; // FNV offset basis
    let mut mix = |v: i64| {
        h ^= v as u64;
        h = h.wrapping_mul(1099511628211);
    };
    for level in &model.levels {
        mix(level.elevation.ticks());
    }
    for wall in &model.walls {
        mix(wall.start.x.ticks());
        mix(wall.start.y.ticks());
        mix(wall.end.x.ticks());
        mix(wall.end.y.ticks());
        mix(wall.height.ticks());
        mix(wall.length.ticks());
        // The wall's construction system drives its through-wall depth and
        // exposure, so fold the system id, total thickness, and exposure in.
        for byte in wall.system.0.bytes() {
            mix(byte as i64);
        }
        if let Some(system) = model.system_for(wall) {
            mix(system.total_thickness().ticks());
            mix(match system.exposure() {
                framer_core::WallExposure::Exterior => 1,
                framer_core::WallExposure::Interior => 2,
            });
        }
        for opening in &wall.openings {
            mix(opening.center.ticks());
            mix(opening.width.ticks());
            mix(opening.height.ticks());
            mix(opening.sill_height.ticks());
            mix(opening.kind as i64);
        }
    }
    h
}
