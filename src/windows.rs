use anyhow::Context;
use log::{debug, info};
use niri_ipc::{Action, Request, Response, SizeChange, Window, Workspace, socket::Socket};

fn query_windows(socket: &mut Socket) -> anyhow::Result<Vec<Window>> {
    match socket
        .send(Request::Windows)
        .context("Failed to send windows request")?
        .map_err(anyhow::Error::msg)
        .context("Niri failed to return windows")?
    {
        Response::Windows(windows) => Ok(windows),
        other => Err(anyhow::anyhow!(
            "Unexpected response to Windows request: {other:?}"
        )),
    }
}

fn query_workspaces(socket: &mut Socket) -> anyhow::Result<Vec<Workspace>> {
    match socket
        .send(Request::Workspaces)
        .context("Failed to send workspaces request")?
        .map_err(anyhow::Error::msg)
        .context("Niri failed to return workspaces")?
    {
        Response::Workspaces(workspaces) => Ok(workspaces),
        other => Err(anyhow::anyhow!(
            "Unexpected response to Workspaces request: {other:?}"
        )),
    }
}

/// Which output a workspace currently lives on
pub fn workspace_output(socket: &mut Socket, ws: u64) -> anyhow::Result<Option<String>> {
    Ok(query_workspaces(socket)?
        .into_iter()
        .find(|w| w.id == ws)
        .and_then(|w| w.output))
}

/// Given a workspace's tracked window ids, return them sorted left-to-right
/// by column position
pub fn columns_by_position(socket: &mut Socket, windows: &[u64]) -> anyhow::Result<Vec<u64>> {
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
/// Return the configuration of window widths.
/// First element in the returned tuple is the master window width, second is
/// the width of all other windows.
fn width_config(num_windows: usize) -> (f64, Option<f64>) {
    match num_windows {
        0 | 1 => (50.0, None),
        2 => (50.0, Some(25.0)),
        3 => (40.0, Some(30.0)),
        _ => (40.0, Some(33.0)),
    }
}

/// Set focus on window designated by `id`. This function will block and keep
/// polling niri until the window is confirmed to be under focus.
fn focus_window(socket: &mut Socket, id: u64) -> anyhow::Result<()> {
    info!("Sending focus to window {id}");
    socket
        .send(Request::Action(Action::FocusWindow { id }))
        .context("Failed to send focus-window action")?
        .map_err(anyhow::Error::msg)
        .context("Niri failed to focus window")?;

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

/// Set width of focused column
fn set_window_width(socket: &mut Socket, id: u64, percent: f64) -> anyhow::Result<()> {
    socket
        .send(Request::Action(Action::SetWindowWidth {
            id: Some(id),
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
        .send(Request::Action(Action::CenterWindow { id: None }))
        .context("Failed to send center-column action")?
        .map_err(anyhow::Error::msg)
        .context("Niri failed to center column")?;
    Ok(())
}

pub fn set_sole(socket: &mut Socket, window_id: u64) -> anyhow::Result<()> {
    let master_width = width_config(1).0;

    set_window_width(socket, window_id, master_width)?;
    center_column(socket)?;
    info!("[column-widths] sole window {window_id} set to {master_width}%");
    Ok(())
}

pub fn set_duo(socket: &mut Socket, left: u64, right: u64) -> anyhow::Result<()> {
    let width_config = width_config(2);
    let master_width = width_config.0;
    let side_width = width_config.1.unwrap();

    set_window_width(socket, right, side_width)?;

    set_window_width(socket, left, master_width)?;

    focus_window(socket, left)?;
    center_column(socket)?;

    info!("[column-widths] duo left={left} ({master_width}%) right={right} ({side_width}%)");
    Ok(())
}

pub fn set_trio(socket: &mut Socket, left: u64, center: u64, right: u64) -> anyhow::Result<()> {
    let width_config = width_config(3);
    let master_width = width_config.0;
    let side_width = width_config.1.unwrap();

    set_window_width(socket, left, side_width)?;
    set_window_width(socket, center, master_width)?;
    set_window_width(socket, right, side_width)?;

    focus_window(socket, center)?;
    center_column(socket)?;

    info!(
        "[column-widths] trio left={left} ({side_width}%) center={center} ({master_width}%) right={right} ({side_width}%)"
    );
    Ok(())
}

pub fn set_new_window_default(
    socket: &mut Socket,
    num_windows: usize,
    new_window_id: u64,
) -> anyhow::Result<()> {
    let default_width = width_config(num_windows).1.unwrap();

    set_window_width(socket, new_window_id, default_width)?;
    info!("[column-widths] new window {new_window_id} defaulted to {default_width}%");
    Ok(())
}
