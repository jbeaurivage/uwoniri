use std::{io, path::PathBuf};

use log::{debug, error, info};
use serde::Deserialize;

use crate::error::{self, Error, Result};

/// ~/.config/uwoniri/config.toml (honors $XDG_CONFIG_HOME):
///
/// ```toml
/// [watch]
/// outputs = ["DP-2"]   # optional: connector names this applies to
/// min_width = 3440     # optional: OR apply on any output at least this many logical pixels wide
///
/// [widths]             # optional, percentages in (0, 100]
/// overflow = 33.0
/// layouts = [          # entry i is used for workspaces with i+1 windows
///   { main = 50.0 },
///   { main = 50.0, side = 25.0 },
///   { main = 40.0, side = 30.0 },
/// ]
/// ```
///
/// If both `outputs` and `min_width` are set, an output qualifies by matching
/// EITHER condition.
#[derive(Debug, Deserialize, Default)]
pub struct Config {
    pub watch: Option<Watch>,
    #[serde(default)]
    pub widths: Widths,
}

#[derive(Debug, Deserialize, Default)]
pub struct Watch {
    #[serde(default)]
    pub outputs: Vec<String>,
    pub min_width: Option<i32>,
}

#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(deny_unknown_fields)]
pub struct Layout {
    pub main: f64,
    /// Width of the other windows. Ignored for a single window.
    pub side: Option<f64>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(default, deny_unknown_fields)]
pub struct Widths {
    /// `layouts[i]` applies to a workspace with `i + 1` windows; the length is
    /// how many windows are managed before the overflow rules kick in.
    pub layouts: Vec<Layout>,
    pub overflow: f64,
}

impl Default for Widths {
    fn default() -> Self {
        Self {
            layouts: vec![
                Layout {
                    main: 50.0,
                    side: None,
                },
                Layout {
                    main: 50.0,
                    side: Some(25.0),
                },
                Layout {
                    main: 40.0,
                    side: Some(30.0),
                },
            ],
            overflow: 33.0,
        }
    }
}

impl Widths {
    /// How many windows get their own layout before overflow applies.
    pub fn max_managed(&self) -> usize {
        self.layouts.len()
    }

    /// Layout for a workspace with `n` windows. Beyond `max_managed` this is
    /// the last layout (used for the focused window's group).
    pub fn layout_for(&self, n: usize) -> Layout {
        self.layouts[n.clamp(1, self.layouts.len()) - 1]
    }

    fn validate(&self) -> std::result::Result<(), String> {
        if self.layouts.is_empty() {
            return Err("widths.layouts needs at least one entry".into());
        }
        let ok = |v: f64| v > 0.0 && v <= 100.0;
        for (i, l) in self.layouts.iter().enumerate() {
            let count = i + 1;
            if !ok(l.main) {
                return Err(format!(
                    "widths.layouts[{i}] ({count} windows): main = {} is outside (0, 100]",
                    l.main
                ));
            }
            match l.side {
                None if count > 1 => {
                    return Err(format!(
                        "widths.layouts[{i}] ({count} windows) needs a `side` width"
                    ));
                }
                Some(s) if !ok(s) => {
                    return Err(format!(
                        "widths.layouts[{i}] ({count} windows): side = {s} is outside (0, 100]"
                    ));
                }
                _ => {}
            }
        }
        if !ok(self.overflow) {
            return Err(format!(
                "widths.overflow = {} is outside (0, 100]",
                self.overflow
            ));
        }
        Ok(())
    }
}

impl Config {
    fn config_path() -> Result<PathBuf> {
        Ok(dirs::config_dir()
            .ok_or(Error::NoDir("config"))?
            .join("uwoniri")
            .join("config.toml"))
    }

    /// Read and parse the config file. `Ok(None)` means there is no config
    /// file.
    fn try_load() -> Result<Option<(PathBuf, Self)>> {
        let path = Self::config_path()?;

        let contents = match std::fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(source) if source.kind() == io::ErrorKind::NotFound => {
                debug!(
                    "[column-widths] no config at {} -- applying to all outputs",
                    path.display()
                );
                return Ok(None);
            }
            Err(source) => return Err(Error::ConfigRead { path, source }),
        };

        match toml::from_str::<Self>(&contents) {
            Ok(cfg) => {
                cfg.widths
                    .validate()
                    .map_err(|reason| Error::ConfigInvalid {
                        path: path.clone(),
                        reason,
                    })?;
                Ok(Some((path, cfg)))
            }
            Err(source) => Err(Error::ConfigParse { path, source }),
        }
    }

    /// Load the config, falling back to the unrestricted default (and logging
    /// why) if it is missing or unusable.
    pub fn load() -> Self {
        match Self::try_load() {
            Ok(Some((path, cfg))) => {
                info!("[column-widths] loaded config from {}", path.display());
                cfg
            }
            Ok(None) => Self::default(),
            Err(err) => {
                error!(
                    "[column-widths] {}; applying to all outputs",
                    error::report(&err)
                );
                Self::default()
            }
        }
    }

    /// Whether column-widths behavior should run for this output.
    /// `logical_width` is only called if the answer depends on the output's
    /// size, i.e. `min_width` is set and the output wasn't already matched
    /// by name.
    pub fn applies_to(
        &self,
        output_name: Option<&str>,
        mut logical_width: impl FnMut(&str) -> Result<Option<i32>>,
    ) -> Result<bool> {
        let Some(watch) = &self.watch else {
            // No restriction
            return Ok(true);
        };

        if watch.outputs.is_empty() && watch.min_width.is_none() {
            return Ok(true); // unrestricted
        }

        let Some(name) = output_name else {
            return Ok(false);
        };

        if watch.outputs.iter().any(|o| o == name) {
            return Ok(true);
        }

        let Some(min) = watch.min_width else {
            return Ok(false);
        };
        Ok(logical_width(name)?.is_some_and(|width| width >= min))
    }
}
