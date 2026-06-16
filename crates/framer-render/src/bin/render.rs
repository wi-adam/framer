//! Headless renderer: path-traces a `.framer` project to a PNG.
//!
//! ```text
//! render <project.framer> <out.png> [--width W] [--height H] [--spp N]
//!        [--seed S] [--yaw DEG] [--pitch DEG] [--zoom Z] [--exposure E]
//! ```
//!
//! This is both a user-facing export feature and the development tool used to
//! visually verify renders (render to PNG, then inspect the image).

use std::process::ExitCode;

use framer_render::{RenderOptions, render, scene_from_model};

struct Args {
    input: String,
    output: String,
    width: u32,
    height: u32,
    spp: u32,
    seed: u64,
    yaw: Option<f32>,
    pitch: Option<f32>,
    zoom: Option<f32>,
    exposure: Option<f32>,
}

fn parse_args() -> Result<Args, String> {
    let mut positional: Vec<String> = Vec::new();
    let mut width = 1280u32;
    let mut height = 720u32;
    let mut spp = 128u32;
    let mut seed = 1u64;
    let mut yaw = None;
    let mut pitch = None;
    let mut zoom = None;
    let mut exposure = None;

    let mut iter = std::env::args().skip(1);
    while let Some(arg) = iter.next() {
        let mut next_val = |name: &str| -> Result<String, String> {
            iter.next()
                .ok_or_else(|| format!("{name} requires a value"))
        };
        let parse_f32 = |s: String, name: &str| -> Result<f32, String> {
            s.parse::<f32>().map_err(|_| format!("invalid {name}: {s}"))
        };
        match arg.as_str() {
            "--width" => {
                width = next_val("--width")?
                    .parse()
                    .map_err(|_| "invalid --width")?
            }
            "--height" => {
                height = next_val("--height")?
                    .parse()
                    .map_err(|_| "invalid --height")?
            }
            "--spp" => spp = next_val("--spp")?.parse().map_err(|_| "invalid --spp")?,
            "--seed" => seed = next_val("--seed")?.parse().map_err(|_| "invalid --seed")?,
            "--yaw" => yaw = Some(parse_f32(next_val("--yaw")?, "--yaw")?.to_radians()),
            "--pitch" => pitch = Some(parse_f32(next_val("--pitch")?, "--pitch")?.to_radians()),
            "--zoom" => zoom = Some(parse_f32(next_val("--zoom")?, "--zoom")?),
            "--exposure" => exposure = Some(parse_f32(next_val("--exposure")?, "--exposure")?),
            "-h" | "--help" => return Err("help".to_string()),
            other if other.starts_with("--") => return Err(format!("unknown flag: {other}")),
            other => positional.push(other.to_string()),
        }
    }

    if positional.len() != 2 {
        return Err("expected <project.framer> <out.png>".to_string());
    }
    if width == 0 || height == 0 {
        return Err("width and height must be positive".to_string());
    }
    Ok(Args {
        input: positional[0].clone(),
        output: positional[1].clone(),
        width,
        height,
        spp,
        seed,
        yaw,
        pitch,
        zoom,
        exposure,
    })
}

fn run() -> Result<(), String> {
    let args = parse_args()?;

    let source = std::fs::read_to_string(&args.input)
        .map_err(|e| format!("cannot read {}: {e}", args.input))?;
    let model = framer_core::load_project(&source)
        .map_err(|e| format!("cannot parse {}: {e}", args.input))?;

    let mut opts = RenderOptions {
        aspect: args.width as f32 / args.height as f32,
        ..RenderOptions::default()
    };
    if let Some(v) = args.yaw {
        opts.yaw = v;
    }
    if let Some(v) = args.pitch {
        opts.pitch = v;
    }
    if let Some(v) = args.zoom {
        opts.zoom = v;
    }
    if let Some(v) = args.exposure {
        opts.exposure = v;
    }

    let scene = scene_from_model(&model, &opts);
    eprintln!(
        "Rendering {} ({} triangles) at {}x{}, {} spp...",
        args.input,
        scene.triangles.len(),
        args.width,
        args.height,
        args.spp
    );
    let buffer = render(&scene, args.width, args.height, args.spp, args.seed);

    let image = image::RgbaImage::from_raw(args.width, args.height, buffer)
        .ok_or("internal error: render buffer has the wrong size")?;
    image
        .save(&args.output)
        .map_err(|e| format!("cannot write {}: {e}", args.output))?;
    eprintln!("Wrote {}", args.output);
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) if msg == "help" => {
            eprintln!(
                "Usage: render <project.framer> <out.png> [--width W] [--height H] \
                 [--spp N] [--seed S] [--yaw DEG] [--pitch DEG] [--zoom Z] [--exposure E]"
            );
            ExitCode::SUCCESS
        }
        Err(msg) => {
            eprintln!("error: {msg}");
            ExitCode::FAILURE
        }
    }
}
