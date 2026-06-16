//! Flattens a [`Scene`] into `#[repr(C)]` POD structs laid out for GPU storage
//! buffers, so the app's WGSL compute path tracer can consume the *same* scene
//! the CPU tracer renders. The layouts mirror the WGSL structs exactly: every
//! `vec3<f32>` is padded to 16 bytes and scalars are packed into the spare lane,
//! matching std430/std140 alignment. The `size_of` asserts in the tests pin this
//! down — if the WGSL structs change, the asserts (and the parity test) catch it.

use bytemuck::{Pod, Zeroable};

use crate::material::Material;
use crate::scene::Scene;

/// A triangle for the GPU: precomputed edges + geometric normal, with the
/// material index packed into `v0`'s spare lane. 64 bytes, 16-byte aligned.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct GpuTriangle {
    pub v0: [f32; 3],
    pub material: u32,
    pub edge1: [f32; 3],
    pub _pad1: f32,
    pub edge2: [f32; 3],
    pub _pad2: f32,
    pub normal: [f32; 3],
    pub _pad3: f32,
}

/// A flattened BVH node mirroring [`crate::bvh::BvhNode`]: `count > 0` ⇒ leaf
/// whose triangles are `indices[left_first .. left_first + count]`; `count == 0`
/// ⇒ internal node with children at `left_first` and `left_first + 1`. 32 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct GpuBvhNode {
    pub aabb_min: [f32; 3],
    pub left_first: u32,
    pub aabb_max: [f32; 3],
    pub count: u32,
}

/// Material kind tags shared with the WGSL kernel.
pub const MAT_KIND_DIFFUSE: u32 = 0;
pub const MAT_KIND_METAL: u32 = 1;
pub const MAT_KIND_DIELECTRIC: u32 = 2;
pub const MAT_KIND_EMISSIVE: u32 = 3;

/// A material for the GPU. `color` is albedo (diffuse/metal) or radiance
/// (emissive); `tint` is the dielectric transmission tint; `param` is roughness
/// (metal) or IOR (dielectric). `kind` selects the interpretation. 32 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct GpuMaterial {
    pub color: [f32; 3],
    pub kind: u32,
    pub tint: [f32; 3],
    pub param: f32,
}

/// Per-frame uniforms: camera basis, sun (with precomputed `cos(angular_radius)`
/// and solid angle so the GPU uses the *same* f32 values the CPU does), sky,
/// exposure, image dimensions, the index of the first sample in this dispatch
/// (`frame`; 0 ⇒ reset/discard previous accumulation), the number of samples to
/// trace this dispatch (`samples_per_dispatch`), and the split RNG seed. 176
/// bytes, a multiple of 16.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct GpuUniforms {
    pub cam_eye: [f32; 3],
    pub half_w: f32,
    pub cam_forward: [f32; 3],
    pub half_h: f32,
    pub cam_right: [f32; 3],
    pub _pad_r: f32,
    pub cam_up: [f32; 3],
    pub _pad_u: f32,
    pub sun_dir: [f32; 3],
    pub sun_cos_angular: f32,
    pub sun_irradiance: [f32; 3],
    pub sun_solid_angle: f32,
    pub sky_zenith: [f32; 3],
    pub exposure: f32,
    pub sky_horizon: [f32; 3],
    pub _pad_h: f32,
    pub sky_ground: [f32; 3],
    pub _pad_g: f32,
    pub width: u32,
    pub height: u32,
    /// Index of the first sample traced in this dispatch (0 discards the previous
    /// accumulator). With one sample per dispatch this is just the frame index.
    pub frame: u32,
    pub seed_lo: u32,
    pub seed_hi: u32,
    pub max_bounces: u32,
    pub _pad0: u32,
    /// Samples per pixel to trace in this dispatch (progressive burst). The kernel
    /// adds this to the accumulator's sample-count lane.
    pub samples_per_dispatch: u32,
}

impl GpuUniforms {
    /// Builds the per-frame uniforms for `scene` at `width`×`height`, progressive
    /// `frame` index, and RNG `seed`. `max_bounces` should be [`crate::MAX_BOUNCES`].
    pub fn new(
        scene: &Scene,
        width: u32,
        height: u32,
        frame: u32,
        seed: u64,
        max_bounces: u32,
    ) -> Self {
        let cam = &scene.camera;
        let sun = &scene.sun;
        let sky = &scene.sky;
        Self {
            cam_eye: arr(cam.eye),
            half_w: cam.half_w,
            cam_forward: arr(cam.forward),
            half_h: cam.half_h,
            cam_right: arr(cam.right),
            _pad_r: 0.0,
            cam_up: arr(cam.up),
            _pad_u: 0.0,
            sun_dir: arr(sun.dir),
            // Precompute exactly as the CPU does, so the GPU consumes identical
            // f32 values (cone sampling + disk test must agree bit-for-bit).
            sun_cos_angular: sun.angular_radius.cos(),
            sun_irradiance: arr(sun.irradiance),
            sun_solid_angle: sun.solid_angle(),
            sky_zenith: arr(sky.zenith),
            exposure: scene.exposure,
            sky_horizon: arr(sky.horizon),
            _pad_h: 0.0,
            sky_ground: arr(sky.ground),
            _pad_g: 0.0,
            width,
            height,
            frame,
            seed_lo: seed as u32,
            seed_hi: (seed >> 32) as u32,
            max_bounces,
            _pad0: 0,
            // Default to a single sample so the headless parity test (which drives
            // one sample per dispatch with frame = sample index) is unaffected; the
            // in-app renderer overrides this to burst multiple samples per frame.
            samples_per_dispatch: 1,
        }
    }
}

/// The scene flattened to GPU storage-buffer payloads.
#[derive(Clone, Debug)]
pub struct GpuScene {
    pub triangles: Vec<GpuTriangle>,
    pub nodes: Vec<GpuBvhNode>,
    pub indices: Vec<u32>,
    pub materials: Vec<GpuMaterial>,
}

#[inline]
fn arr(v: crate::math::Vec3) -> [f32; 3] {
    [v.x, v.y, v.z]
}

impl From<&Material> for GpuMaterial {
    fn from(m: &Material) -> Self {
        match *m {
            Material::Diffuse { albedo } => GpuMaterial {
                color: arr(albedo),
                kind: MAT_KIND_DIFFUSE,
                tint: [0.0; 3],
                param: 0.0,
            },
            Material::Metal { albedo, roughness } => GpuMaterial {
                color: arr(albedo),
                kind: MAT_KIND_METAL,
                tint: [0.0; 3],
                param: roughness,
            },
            Material::Dielectric { ior, tint } => GpuMaterial {
                color: [0.0; 3],
                kind: MAT_KIND_DIELECTRIC,
                tint: arr(tint),
                param: ior,
            },
            Material::Emissive { radiance } => GpuMaterial {
                color: arr(radiance),
                kind: MAT_KIND_EMISSIVE,
                tint: [0.0; 3],
                param: 0.0,
            },
        }
    }
}

impl Scene {
    /// Flattens this scene into GPU storage-buffer payloads. Triangle order,
    /// BVH node order, and the `indices` permutation are preserved verbatim, so
    /// the WGSL traversal reproduces the CPU traversal exactly.
    pub fn to_gpu(&self) -> GpuScene {
        let triangles = self
            .triangles
            .iter()
            .map(|t| GpuTriangle {
                v0: arr(t.v0),
                material: t.material,
                edge1: arr(t.edge1),
                _pad1: 0.0,
                edge2: arr(t.edge2),
                _pad2: 0.0,
                normal: arr(t.geom_normal),
                _pad3: 0.0,
            })
            .collect();
        let nodes = self
            .bvh
            .nodes
            .iter()
            .map(|n| GpuBvhNode {
                aabb_min: arr(n.aabb.min),
                left_first: n.left_first,
                aabb_max: arr(n.aabb.max),
                count: n.count,
            })
            .collect();
        let materials = self.materials.iter().map(GpuMaterial::from).collect();
        GpuScene {
            triangles,
            nodes,
            indices: self.bvh.indices.clone(),
            materials,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::Camera;
    use crate::geom::Triangle;
    use crate::material::Material;
    use crate::math::Vec3;
    use crate::scene::{DirectionalSun, Scene, Sky};
    use std::mem::size_of;

    /// The struct sizes are the contract with the WGSL kernel. They must be the
    /// documented values *and* multiples of 16 (storage/uniform array stride).
    #[test]
    fn struct_sizes_match_wgsl_layout() {
        assert_eq!(size_of::<GpuTriangle>(), 64);
        assert_eq!(size_of::<GpuBvhNode>(), 32);
        assert_eq!(size_of::<GpuMaterial>(), 32);
        assert_eq!(size_of::<GpuUniforms>(), 176);
        for s in [
            size_of::<GpuTriangle>(),
            size_of::<GpuBvhNode>(),
            size_of::<GpuMaterial>(),
            size_of::<GpuUniforms>(),
        ] {
            assert_eq!(s % 16, 0, "size {s} not a multiple of 16");
        }
    }

    fn one_tri_scene() -> Scene {
        let tris = vec![Triangle::new(
            Vec3::new(1.0, 2.0, 3.0),
            Vec3::new(4.0, 2.0, 3.0),
            Vec3::new(1.0, 6.0, 3.0),
            0,
        )];
        let materials = vec![Material::Diffuse {
            albedo: Vec3::new(0.1, 0.2, 0.3),
        }];
        let camera = Camera::orbit(Vec3::ZERO, 2.0, -0.6, 0.4, 1.0, 1.5, 36.0);
        let sun = DirectionalSun {
            dir: Vec3::new(0.3, 0.2, 1.0).normalize(),
            irradiance: Vec3::splat(4.0),
            angular_radius: 0.03,
        };
        let sky = Sky {
            zenith: Vec3::new(0.2, 0.4, 0.9),
            horizon: Vec3::new(0.7, 0.8, 0.95),
            ground: Vec3::new(0.2, 0.18, 0.15),
        };
        Scene::new(tris, materials, sun, sky, camera, 1.3)
    }

    #[test]
    fn to_gpu_counts_match_scene() {
        let scene = one_tri_scene();
        let gpu = scene.to_gpu();
        assert_eq!(gpu.triangles.len(), scene.triangles.len());
        assert_eq!(gpu.nodes.len(), scene.bvh.nodes.len());
        assert_eq!(gpu.indices, scene.bvh.indices);
        assert_eq!(gpu.materials.len(), scene.materials.len());
    }

    #[test]
    fn triangle_fields_round_trip() {
        let scene = one_tri_scene();
        let gpu = scene.to_gpu();
        let t = &scene.triangles[0];
        let g = gpu.triangles[0];
        assert_eq!(g.v0, [t.v0.x, t.v0.y, t.v0.z]);
        assert_eq!(g.edge1, [t.edge1.x, t.edge1.y, t.edge1.z]);
        assert_eq!(g.edge2, [t.edge2.x, t.edge2.y, t.edge2.z]);
        assert_eq!(
            g.normal,
            [t.geom_normal.x, t.geom_normal.y, t.geom_normal.z]
        );
        assert_eq!(g.material, 0);
    }

    #[test]
    fn material_mapping_covers_every_variant() {
        let diffuse = GpuMaterial::from(&Material::Diffuse {
            albedo: Vec3::new(0.4, 0.5, 0.6),
        });
        assert_eq!(diffuse.kind, MAT_KIND_DIFFUSE);
        assert_eq!(diffuse.color, [0.4, 0.5, 0.6]);

        let metal = GpuMaterial::from(&Material::Metal {
            albedo: Vec3::new(0.9, 0.8, 0.7),
            roughness: 0.25,
        });
        assert_eq!(metal.kind, MAT_KIND_METAL);
        assert_eq!(metal.color, [0.9, 0.8, 0.7]);
        assert_eq!(metal.param, 0.25);

        let glass = GpuMaterial::from(&Material::Dielectric {
            ior: 1.5,
            tint: Vec3::new(0.9, 0.95, 0.93),
        });
        assert_eq!(glass.kind, MAT_KIND_DIELECTRIC);
        assert_eq!(glass.param, 1.5);
        assert_eq!(glass.tint, [0.9, 0.95, 0.93]);

        let emit = GpuMaterial::from(&Material::Emissive {
            radiance: Vec3::new(3.0, 2.0, 1.0),
        });
        assert_eq!(emit.kind, MAT_KIND_EMISSIVE);
        assert_eq!(emit.color, [3.0, 2.0, 1.0]);
    }

    #[test]
    fn uniforms_capture_camera_sun_sky_and_seed() {
        let scene = one_tri_scene();
        let u = GpuUniforms::new(&scene, 320, 240, 5, 0x1234_5678_9ABC_DEF0, 8);
        assert_eq!(u.width, 320);
        assert_eq!(u.height, 240);
        assert_eq!(u.frame, 5);
        assert_eq!(u.max_bounces, 8);
        assert_eq!(u.seed_lo, 0x9ABC_DEF0);
        assert_eq!(u.seed_hi, 0x1234_5678);
        assert_eq!(
            u.cam_eye,
            [scene.camera.eye.x, scene.camera.eye.y, scene.camera.eye.z]
        );
        assert_eq!(u.exposure, 1.3);
        // Sun derived quantities use the CPU's exact f32 math.
        assert_eq!(u.sun_cos_angular, scene.sun.angular_radius.cos());
        assert_eq!(u.sun_solid_angle, scene.sun.solid_angle());
    }
}
