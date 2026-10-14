use std::{collections::HashMap, path::PathBuf};

use log::info;
use niri_ipc::Output;
use serde::Deserialize;

/// ~/.config/uwoniri/config.toml (honors $XDG_CONFIG_HOME):
///
/// ```toml
/// outputs = ["DP-2"]   # optional: connector names this applies to
/// min_width = 3440     # optional: OR apply on any output at least this many logical pixels wide
/// ```
///
/// If both are set, an output qualifies by matching EITHER condition.
#[derive(Debug, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub outputs: Vec<String>,
    pub min_width: Option<i32>,
}

impl Config {
    fn config_path() -> anyhow::Result<PathBuf> {
        Ok(dirs::config_dir()
            .ok_or_else(|| anyhow::anyhow!("Failed to determine the config directory"))?
            .join("uwoniri")
            .join("config.toml"))
    }

    pub fn load() -> Self {
        let path = Self::config_path().unwrap();
        match std::fs::read_to_string(&path) {
            Ok(contents) => match toml::from_str::<Self>(&contents) {
                Ok(cfg) => {
                    info!("[column-widths] loaded config from {}", path.display());
                    cfg
                }
                Err(error) => {
                    log::error!(
                        "[column-widths] failed to parse {}: {error}; applying to all outputs",
                        path.display()
                    );
                    Self::default()
                }
            },
            Err(_) => {
                log::debug!(
                    "[column-widths] no config at {} -- applying to all outputs",
                    path.display()
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
