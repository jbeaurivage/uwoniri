use std::{collections::HashMap, io, path::PathBuf};

use log::{debug, error, info};
use niri_ipc::Output;
use serde::Deserialize;

use crate::error::{self, Error, Result};

/// ~/.config/uwoniri/config.toml (honors $XDG_CONFIG_HOME):
///
/// ```toml
/// outputs = ["DP-2"]   # optional: connector names this applies to
/// min_width = 3440     # optional: OR apply on any output at least this many logical pixels wide
///
/// [widths]             # optional, percentages in (0, 100]
/// overflow = 33.0
/// layouts = [          # entry i is used for workspaces with i+1 windows
///   { master = 50.0 },
///   { master = 50.0, side = 25.0 },
///   { master = 40.0, side = 30.0 },
/// ]
/// ```
///
/// If both `outputs` and `min_width` are set, an output qualifies by matching
/// EITHER condition.
#[derive(Debug, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub outputs: Vec<String>,
    pub min_width: Option<i32>,
    #[serde(default)]
    pub widths: Widths,
}

#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(deny_unknown_fields)]
pub struct Layout {
    pub master: f64,
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
                    master: 50.0,
                    side: None,
                },
                Layout {
                    master: 50.0,
                    side: Some(25.0),
                },
                Layout {
                    master: 40.0,
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
            if !ok(l.master) {
                return Err(format!(
                    "widths.layouts[{i}] ({count} windows): master = {} is outside (0, 100]",
                    l.master
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

    /// Read and parse the config file. `Ok(None)` means there is no config file.
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
    pub fn applies_to(&self, output_name: Option<&str>, outputs: &HashMap<String, Output>) -> bool {
        if self.outputs.is_empty() && self.min_width.is_none() {
            return true; // unrestricted
        }

        let Some(name) = output_name else {
            return false;
        };

        if self.outputs.iter().any(|o| o == name) {
            return true;
        }

        if let Some(min_w) = self.min_width
            && let Some(width) = outputs
                .get(name)
                .and_then(|o| o.logical.as_ref())
                .map(|l| l.width as i32)
        {
            return width >= min_w;
        }

        false
    }
}
