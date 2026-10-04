use anyhow::Context;
use log::info;
use niri_ipc::state::EventStreamState;
use niri_ipc::{Action, Output, Request, SizeChange, socket::Socket};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;

/// Return the configuration of window widths.
/// First element in the returned tuple is the master window width, second is the width of all other windows.
fn width_config(num_windows: usize) -> (f64, Option<f64>) {
    match num_windows {
        0 | 1 => (50.0, None),
        2 => (50.0, Some(25.0)),
        3 => (40.0, Some(30.0)),
        _ => (40.0, Some(33.0)),
    }
}

/// Set focus on window designated by `id`.
fn focus_window(socket: &mut Socket, id: u64) -> anyhow::Result<()> {
    socket
        .send(Request::Action(Action::FocusWindow { id }))
        .context("Failed to send focus-window action")?
        .map_err(anyhow::Error::msg)
        .context("Niri failed to focus window")?;
    Ok(())
}

/// Set width of focused column
fn set_column_width(socket: &mut Socket, percent: f64) -> anyhow::Result<()> {
    socket
        .send(Request::Action(Action::SetColumnWidth {
            change: SizeChange::SetProportion(percent),
        }))
        .context("Failed to send set-column-width action")?
        .map_err(anyhow::Error::msg)
        .context("Niri failed to set column width")?;
    Ok(())
}

/// Center the focused column
fn center_column(socket: &mut Socket) -> anyhow::Result<()> {
    socket
        .send(Request::Action(Action::CenterColumn {}))
        .context("Failed to send center-column action")?
        .map_err(anyhow::Error::msg)
        .context("Niri failed to center column")?;
    Ok(())
}

/// Given a workspace's tracked window ids, return them sorted left-to-right
pub fn columns_by_position(state: &EventStreamState, windows: &[u64]) -> Vec<u64> {
    let mut with_col: Vec<(usize, u64)> = windows
        .iter()
        .filter_map(|&id| {
            state
                .windows
                .windows
                .get(&id)
                .and_then(|w| w.layout.pos_in_scrolling_layout)
                .map(|(col, _tile)| (col, id))
        })
        .collect();

    if with_col.len() != windows.len() {
        return Vec::new();
    }

    with_col.sort_by_key(|&(col, _)| col);
    with_col.into_iter().map(|(_, id)| id).collect()
}

/// Solo window: 50% wide, centered.
pub fn set_sole(socket: &mut Socket, window_id: u64) -> anyhow::Result<()> {
    let master_width = width_config(1).0;

    focus_window(socket, window_id)?;
    set_column_width(socket, width_config(1).0)?;
    center_column(socket)?;
    info!("[column-widths] solo window {window_id} set to {master_width}%");
    Ok(())
}

/// Two windows: `left` (by column position) is master at 50% + centered
/// (last action); `right` gets 25%, resized first.
pub fn set_duo(socket: &mut Socket, left: u64, right: u64) -> anyhow::Result<()> {
    let width_config = width_config(2);
    let master_width = width_config.0;
    let side_width = width_config.1.unwrap();

    focus_window(socket, right)?;
    set_column_width(socket, master_width)?;

    focus_window(socket, left)?;
    set_column_width(socket, side_width)?;
    center_column(socket)?;

    info!("[column-widths] duo left={left} ({master_width}%) right={right} ({side_width}%)");
    Ok(())
}

/// Three windows: `center` (the spatially middle column) gets 40% +
/// centered (last action); `left`/`right` get 30% each, resized first.
pub fn set_trio(socket: &mut Socket, left: u64, center: u64, right: u64) -> anyhow::Result<()> {
    let width_config = width_config(3);
    let master_width = width_config.0;
    let side_width = width_config.1.unwrap();

    focus_window(socket, left)?;
    set_column_width(socket, side_width)?;

    focus_window(socket, right)?;
    set_column_width(socket, side_width)?;

    focus_window(socket, center)?;
    set_column_width(socket, side_width)?;
    center_column(socket)?;

    info!(
        "[column-widths] trio left={left} center={center} ({master_width}%) right={right} ({side_width}% each)"
    );
    Ok(())
}

/// Beyond the baseline transition: only the brand-new window gets
/// defaulted to 33%; existing columns are left exactly as they are.
pub fn set_new_window_default(
    socket: &mut Socket,
    num_windows: usize,
    new_window_id: u64,
) -> anyhow::Result<()> {
    let default_width = width_config(num_windows).1.unwrap();

    focus_window(socket, new_window_id)?;
    set_column_width(socket, default_width)?;
    info!("[column-widths] new window {new_window_id} defaulted to {default_width}%");
    Ok(())
}

// ---------------------------------------------------------------------
// Config: scope column-widths mode to specific outputs and/or a minimum
// display width. Absent file, or a file with neither field set, means
// "apply everywhere" -- the original unscoped behavior.
// ---------------------------------------------------------------------

/// ~/.config/oniri/column-widths.toml (honors $XDG_CONFIG_HOME):
///
/// ```toml
/// outputs = ["DP-2"]   # optional: connector names this applies to
/// min_width = 3440     # optional: OR apply on any output at least this
///                       # many logical pixels wide
/// ```
///
/// If both are set, an output qualifies by matching EITHER condition.
#[derive(Debug, Deserialize, Default)]
pub struct ColumnWidthsConfig {
    #[serde(default)]
    pub outputs: Vec<String>,
    pub min_width: Option<i32>,
}

impl ColumnWidthsConfig {
    fn config_path() -> anyhow::Result<PathBuf> {
        Ok(dirs::config_dir()
            .ok_or_else(|| anyhow::anyhow!("Failed to determine the config directory"))?
            .join("oniri")
            .join("column-widths.toml"))
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
