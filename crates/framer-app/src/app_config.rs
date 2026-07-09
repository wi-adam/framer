use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::path::PathBuf;

use clap::{Arg, ArgAction, Command, value_parser};
use config::{Config, Environment, File, FileFormat};
use serde::Deserialize;

const ENV_PREFIX: &str = "FRAMER";

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct AppConfig {
    pub(crate) render: RenderConfig,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct RenderConfig {
    pub(crate) ray_query: bool,
    pub(crate) smoke_frames: Option<u32>,
}

#[derive(Debug)]
pub(crate) enum AppConfigError {
    Cli(Box<clap::Error>),
    Config(config::ConfigError),
}

impl fmt::Display for AppConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cli(error) => write!(f, "{error}"),
            Self::Config(error) => write!(f, "{error}"),
        }
    }
}

impl Error for AppConfigError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Cli(error) => Some(error.as_ref()),
            Self::Config(error) => Some(error),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct CliOverrides {
    config_path: Option<PathBuf>,
    render_ray_query: Option<bool>,
    render_smoke_frames: Option<u32>,
}

pub(crate) fn load() -> Result<AppConfig, AppConfigError> {
    let cli = parse_cli_from(std::env::args_os())
        .map_err(|error| AppConfigError::Cli(Box::new(error)))?;
    load_from_parts(cli, None).map_err(AppConfigError::Config)
}

fn parse_cli_from<I, T>(args: I) -> Result<CliOverrides, clap::Error>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let matches = command().try_get_matches_from(args)?;
    let enable_ray_query = matches.get_flag("render_ray_query");
    let disable_ray_query = matches.get_flag("no_render_ray_query");

    Ok(CliOverrides {
        config_path: matches.get_one::<PathBuf>("config").cloned(),
        render_ray_query: if enable_ray_query {
            Some(true)
        } else if disable_ray_query {
            Some(false)
        } else {
            None
        },
        render_smoke_frames: matches.get_one::<u32>("render_smoke_frames").copied(),
    })
}

fn command() -> Command {
    Command::new("framer")
        .about("Framer desktop CAD shell")
        .arg(
            Arg::new("config")
                .long("config")
                .value_name("PATH")
                .value_parser(value_parser!(PathBuf))
                .help("Load runtime configuration from a TOML file"),
        )
        .arg(
            Arg::new("render_ray_query")
                .long("render-ray-query")
                .action(ArgAction::SetTrue)
                .conflicts_with("no_render_ray_query")
                .help("Enable the experimental hardware ray-query render backend when supported"),
        )
        .arg(
            Arg::new("no_render_ray_query")
                .long("no-render-ray-query")
                .action(ArgAction::SetTrue)
                .help("Disable the experimental hardware ray-query render backend"),
        )
        .arg(
            Arg::new("render_smoke_frames")
                .long("render-smoke-frames")
                .value_name("FRAMES")
                .value_parser(value_parser!(u32))
                .help("Drive the Render view for N frames, then close"),
        )
}

fn load_from_parts(
    cli: CliOverrides,
    env_source: Option<config::Map<String, String>>,
) -> Result<AppConfig, config::ConfigError> {
    let mut builder = Config::builder().set_default("render.ray_query", false)?;

    if let Some(path) = cli.config_path {
        builder = builder.add_source(File::from(path).format(FileFormat::Toml).required(true));
    }

    let mut environment = Environment::with_prefix(ENV_PREFIX)
        .prefix_separator("__")
        .separator("__")
        .try_parsing(true)
        .ignore_empty(true);
    if let Some(source) = env_source {
        environment = environment.source(Some(source));
    }

    builder = builder
        .add_source(environment)
        .set_override_option("render.ray_query", cli.render_ray_query)?
        .set_override_option("render.smoke_frames", cli.render_smoke_frames)?;

    builder.build()?.try_deserialize()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::process;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    static NEXT_CONFIG: AtomicUsize = AtomicUsize::new(0);

    fn env(entries: &[(&str, &str)]) -> config::Map<String, String> {
        entries
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    fn load_test_config(cli: CliOverrides, entries: &[(&str, &str)]) -> AppConfig {
        load_from_parts(cli, Some(env(entries))).expect("test config should load")
    }

    fn write_temp_config(contents: &str) -> PathBuf {
        let index = NEXT_CONFIG.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("framer-config-test-{}-{index}.toml", process::id()));
        fs::write(&path, contents).expect("write temp config");
        path
    }

    #[test]
    fn defaults_keep_experimental_render_backend_disabled() {
        let config = load_test_config(CliOverrides::default(), &[]);

        assert!(!config.render.ray_query);
        assert_eq!(config.render.smoke_frames, None);
    }

    #[test]
    fn config_file_env_and_cli_are_layered_in_order() {
        let path = write_temp_config(
            r#"
[render]
ray_query = false
smoke_frames = 10
"#,
        );
        let cli = CliOverrides {
            config_path: Some(path.clone()),
            render_ray_query: Some(false),
            render_smoke_frames: Some(30),
        };

        let config = load_test_config(
            cli,
            &[
                ("FRAMER__RENDER__RAY_QUERY", "true"),
                ("FRAMER__RENDER__SMOKE_FRAMES", "20"),
            ],
        );

        assert!(!config.render.ray_query);
        assert_eq!(config.render.smoke_frames, Some(30));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn env_overrides_config_file_when_cli_is_absent() {
        let path = write_temp_config(
            r#"
[render]
ray_query = false
"#,
        );
        let config = load_test_config(
            CliOverrides {
                config_path: Some(path.clone()),
                ..CliOverrides::default()
            },
            &[("FRAMER__RENDER__RAY_QUERY", "true")],
        );

        assert!(config.render.ray_query);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn single_underscore_env_names_are_not_part_of_the_config_contract() {
        let config = load_test_config(
            CliOverrides::default(),
            &[("FRAMER_RENDER_RAY_QUERY", "true")],
        );

        assert!(!config.render.ray_query);
    }

    #[test]
    fn explicit_config_path_is_required() {
        let missing =
            std::env::temp_dir().join(format!("framer-missing-config-{}.toml", process::id()));
        let error = load_from_parts(
            CliOverrides {
                config_path: Some(missing),
                ..CliOverrides::default()
            },
            Some(env(&[])),
        )
        .expect_err("missing explicit config file should fail");

        assert!(
            error.to_string().contains("configuration file"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn unknown_config_keys_are_rejected() {
        let path = write_temp_config(
            r#"
[render]
typo = true
"#,
        );
        let error = load_from_parts(
            CliOverrides {
                config_path: Some(path.clone()),
                ..CliOverrides::default()
            },
            Some(env(&[])),
        )
        .expect_err("unknown config keys should fail");

        assert!(
            error.to_string().contains("unknown field"),
            "unexpected error: {error}"
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn cli_parser_supports_runtime_config_flags() {
        let enable =
            parse_cli_from(["framer", "--render-ray-query"]).expect("enable flag should parse");
        let disable =
            parse_cli_from(["framer", "--no-render-ray-query"]).expect("disable flag should parse");
        let smoke = parse_cli_from(["framer", "--render-smoke-frames", "42"])
            .expect("smoke frames flag should parse");
        let config = parse_cli_from(["framer", "--config", "framer.toml"])
            .expect("config flag should parse");

        assert_eq!(enable.render_ray_query, Some(true));
        assert_eq!(disable.render_ray_query, Some(false));
        assert_eq!(smoke.render_smoke_frames, Some(42));
        assert_eq!(config.config_path, Some(PathBuf::from("framer.toml")));
    }
}
