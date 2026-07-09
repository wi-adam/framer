mod app;
mod app_config;

use std::sync::Arc;

fn main() -> eframe::Result {
    env_logger::init();
    let app_config = match app_config::load() {
        Ok(config) => config,
        Err(app_config::AppConfigError::Cli(error)) => error.exit(),
        Err(error) => {
            eprintln!("failed to load Framer configuration: {error}");
            std::process::exit(2);
        }
    };

    let options = eframe::NativeOptions {
        depth_buffer: 24,
        renderer: eframe::Renderer::Wgpu,
        wgpu_options: wgpu_options(app_config.render.ray_query),
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1360.0, 860.0])
            .with_min_inner_size([1040.0, 680.0])
            .with_title("Framer"),
        ..Default::default()
    };

    eframe::run_native(
        "Framer",
        options,
        Box::new(move |cc| Ok(Box::new(app::FramerApp::new(cc, app_config.clone())))),
    )
}

fn wgpu_options(ray_query_enabled: bool) -> eframe::egui_wgpu::WgpuConfiguration {
    use eframe::egui_wgpu::WgpuSetup;
    use eframe::wgpu;

    let mut config = eframe::egui_wgpu::WgpuConfiguration::default();
    let WgpuSetup::CreateNew(setup) = &mut config.wgpu_setup else {
        return config;
    };

    setup.device_descriptor = Arc::new(move |adapter| {
        let base_limits = if adapter.get_info().backend == wgpu::Backend::Gl {
            wgpu::Limits::downlevel_webgl2_defaults()
        } else {
            wgpu::Limits::default()
        };

        let mut required_features = wgpu::Features::empty();
        let mut required_limits = wgpu::Limits {
            // Match egui-wgpu's default: large enough for 4k+ surfaces with depth.
            max_texture_dimension_2d: 8192,
            ..base_limits
        };
        let mut experimental_features = wgpu::ExperimentalFeatures::disabled();

        if ray_query_enabled
            && adapter
                .features()
                .contains(wgpu::Features::EXPERIMENTAL_RAY_QUERY)
        {
            required_features |= wgpu::Features::EXPERIMENTAL_RAY_QUERY;
            required_limits =
                required_limits.using_minimum_supported_acceleration_structure_values();
            // SAFETY: This is an opt-in research path guarded by feature probing and
            // the app still defaults to the existing compute BVH renderer.
            experimental_features = unsafe { wgpu::ExperimentalFeatures::enabled() };
        }

        wgpu::DeviceDescriptor {
            label: Some("egui wgpu device"),
            required_features,
            required_limits,
            experimental_features,
            ..Default::default()
        }
    });

    config
}
