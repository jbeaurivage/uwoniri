use std::thread;
use std::time::{Duration, Instant};

use log::{debug, info};
use niri_ipc::{Action, Request, Response, SizeChange, Window, Workspace};

use crate::config::Widths;
use crate::error::Result;
use crate::ipc::{self, ActionSocket};

fn query_windows(socket: &mut ActionSocket) -> Result<Vec<Window>> {
    match socket.request(Request::Windows)? {
        Response::Windows(windows) => Ok(windows),
        other => Err(ipc::unexpected(other)),
    }
}

fn query_workspaces(socket: &mut ActionSocket) -> Result<Vec<Workspace>> {
    match socket.request(Request::Workspaces)? {
        Response::Workspaces(workspaces) => Ok(workspaces),
        other => Err(ipc::unexpected(other)),
    }
}

/// Which output a workspace currently lives on
pub fn workspace_output(socket: &mut ActionSocket, ws: u64) -> Result<Option<String>> {
    Ok(query_workspaces(socket)?
        .into_iter()
        .find(|w| w.id == ws)
        .and_then(|w| w.output))
}

/// Logical width of the named output, if niri reports one.
pub fn output_width(socket: &mut ActionSocket, name: &str) -> Result<Option<i32>> {
    match socket.request(Request::Outputs)? {
        Response::Outputs(outputs) => Ok(outputs
            .get(name)
            .and_then(|o| o.logical.as_ref())
            .map(|l| l.width as i32)),
        other => Err(ipc::unexpected(other)),
    }
}

/// Given a workspace's tracked window ids, return them sorted left-to-right
/// by column position
pub fn columns_by_position(socket: &mut ActionSocket, windows: &[u64]) -> Result<Vec<u64>> {
    let mut with_col: Vec<(usize, u64)> = query_windows(socket)?
        .into_iter()
        .filter(|w| windows.contains(&w.id))
        .filter_map(|w| {
            w.layout
                .pos_in_scrolling_layout
                .map(|(col, _tile)| (col, w.id))
        })
        .collect();

    if with_col.len() != windows.len() {
        return Ok(Vec::new());
    }

    with_col.sort_by_key(|&(col, _)| col);
    Ok(with_col.into_iter().map(|(_, id)| id).collect())
}

/// Set focus on window designated by `id`. This function will block and keep
/// polling niri until the window is confirmed to be under focus.
fn focus_window(socket: &mut ActionSocket, id: u64) -> Result<()> {
    info!("Sending focus to window {id}");
    socket.send(Action::FocusWindow { id })?;

    loop {
        let windows = query_windows(socket)?;
        debug!("wait...");
        if windows.iter().any(|w| w.id == id && w.is_focused) {
            debug!("focus achieved");
            break;
        }
    }
    Ok(())
}

/// Set width of the column containing window `id`
fn set_window_width(socket: &mut ActionSocket, id: u64, percent: f64) -> Result<()> {
    socket.send(Action::SetWindowWidth {
        id: Some(id),
        change: SizeChange::SetProportion(percent),
    })
}

/// The size window `id` has actually committed (as opposed to been asked for).
fn committed_size(socket: &mut ActionSocket, id: u64) -> Result<Option<(i32, i32)>> {
    Ok(query_windows(socket)?
        .into_iter()
        .find(|w| w.id == id)
        .map(|w| w.layout.window_size))
}

/// Set a window's width, then block until the client has committed the new
/// size. niri centers using the committed size, so centering before this
/// lands off-center.
fn set_window_width_settled(socket: &mut ActionSocket, id: u64, percent: f64) -> Result<()> {
    const POLL: Duration = Duration::from_millis(15);
    const TIMEOUT: Duration = Duration::from_millis(300);

    let before = committed_size(socket, id)?;
    set_window_width(socket, id, percent)?;
    let Some(before) = before else {
        return Ok(());
    };

    let start = Instant::now();
    let mut last = before;
    let mut stable = 0;
    while start.elapsed() < TIMEOUT {
        thread::sleep(POLL);

        let Some(size) = committed_size(socket, id)? else {
            return Ok(());
        };
        if size == last {
            stable += 1;
        } else {
            last = size;
            stable = 0;
        }
        // Changed and stopped changing, or never changed (already at target).
        if (last != before && stable >= 2) || stable >= 6 {
            return Ok(());
        }
    }
    debug!("window {id} did not settle within {TIMEOUT:?}");
    Ok(())
}
/// Center the focused column
fn center_column(socket: &mut ActionSocket) -> Result<()> {
    socket.send(Action::CenterWindow { id: None })
}
/// Apply the layout for `sorted.len()` windows (at most `max_managed`), given
/// left to right. The master is the middle window (left of the two middle ones
/// for an even count).
pub fn set_layout(socket: &mut ActionSocket, sorted: &[u64], widths: &Widths) -> Result<()> {
    let n = sorted.len();
    if n == 0 {
        return Ok(());
    }
    let layout = widths.layout_for(n);
    let master_idx = (n - 1) / 2;
    let master = sorted[master_idx];

    if let Some(side) = layout.side {
        for (i, &id) in sorted.iter().enumerate() {
            if i != master_idx {
                set_window_width(socket, id, side)?;
            }
        }
    }
    set_window_width_settled(socket, master, layout.main)?;

    // Centering acts on the focused column; a lone window is already focused.
    if n > 1 {
        focus_window(socket, master)?;
    }
    center_column(socket)?;

    info!(
        "{n} windows: master {master} ({}%), others {:?}%",
        layout.main, layout.side
    );
    Ok(())
}
pub fn set_new_window_default(
    socket: &mut ActionSocket,
    width_config: &Widths,
    new_window_id: u64,
) -> Result<()> {
    let default_width = width_config.overflow;
    set_window_width(socket, new_window_id, default_width)?;
    info!("new window {new_window_id} defaulted to {default_width}%");
    Ok(())
}

/// The focused tiled window, plus every tiled window on its workspace.
/// `None` if nothing is focused or the focused window is floating.
fn focused_workspace_windows(socket: &mut ActionSocket) -> Result<Option<(u64, Vec<u64>)>> {
    let windows = query_windows(socket)?;

    let Some(focused) = windows.iter().find(|w| w.is_focused && !w.is_floating) else {
        return Ok(None);
    };
    let Some(ws) = focused.workspace_id else {
        return Ok(None);
    };

    let ids = windows
        .iter()
        .filter(|w| !w.is_floating && w.workspace_id == Some(ws))
        .map(|w| w.id)
        .collect();
    Ok(Some((focused.id, ids)))
}

/// Make the focused window the master (master width, centered) and give every
/// other tiled window on its workspace the "other" width.
pub fn retile(socket: &mut ActionSocket, width_config: &Widths) -> Result<()> {
    let Some((focused, windows)) = focused_workspace_windows(socket)? else {
        debug!("retile: no focused tiled window, nothing to do");
        return Ok(());
    };

    let sorted = columns_by_position(socket, &windows)?;
    let Some(pos) = sorted.iter().position(|&id| id == focused) else {
        debug!("retile: could not place window {focused} in the column order, skipping");
        return Ok(());
    };

    // The focused window plus enough neighbors to fill the largest managed
    // layout (everyone, if there are no more windows than that) follow that
    // layout; all other windows get the overflow width.
    let n = sorted.len();
    let max = width_config.max_managed();
    let layout = width_config.layout_for(n);
    let group_len = n.min(max);
    let group_start = pos.saturating_sub((group_len - 1) / 2).min(n - group_len);
    let in_group = |i: usize| (group_start..group_start + group_len).contains(&i);

    // Beyond the managed count, a focused window at either end isn't centered.
    let at_edge = n > max && (pos == 0 || pos == n - 1);

    set_window_width_settled(socket, focused, layout.main)?;

    for (i, &id) in sorted.iter().enumerate() {
        if id == focused {
            continue;
        }
        let width = if in_group(i) {
            layout.side
        } else {
            Some(width_config.overflow)
        };

        if let Some(width) = width {
            set_window_width(socket, id, width)?;
        }
    }

    if !at_edge {
        center_column(socket)?; // acts on the focused column
    }

    info!("retiled {} windows around {focused}", windows.len());
    Ok(())
}
