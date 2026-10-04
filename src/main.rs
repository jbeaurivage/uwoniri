//! Oniri - A tool that automatically maximizes the only window of a niri workspace.
//!
//! It relies on the niri IPC to verify if a window is the only one of a workspace,
//! whether it's the first one opened or the last remaining ones after all the other windows got closed,
//! and maximize it if so.
//!
//! FORK NOTE: added `-C` / `--column-widths` mode, which repurposes the same
//! per-workspace window tracking to manage column widths for an
//! ultrawide-style layout instead of maximizing. See column_widths_mode.rs.
//! This mode is purely positional (re-derives left/center/right from
//! niri's own column data on every relevant event) rather than tracking
//! "master" identity across transitions, and can be paused/resumed by
//! sending SIGUSR1 to the process (e.g. from a niri keybind).

use clap::Parser;
use log::{debug, error, info};
use niri_ipc::socket::Socket;
use niri_ipc::{Event, Output, state::EventStreamState, state::EventStreamStatePart};
use signal_hook::consts::SIGUSR1;
use signal_hook::iterator::Signals;
use std::collections::HashMap;
use std::process;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

mod column_widths_mode;
mod help;
mod lockfile;
mod outputs_map; // https://github.com/Antiz96/oniri/issues/3
mod socket_connections;
mod version;
mod window;
mod windows_map;

// Arguments definition
#[derive(Parser)]
#[command(disable_help_flag = true, disable_version_flag = true)]
struct Args {
    /// Show this help message
    #[arg(short = 'h', long = "help")]
    help: bool,

    #[arg(short = 'V', long = "version")]
    version: bool,
}

/// Which output a workspace currently lives on, for config scoping.
fn workspace_output(state: &EventStreamState, ws: u64) -> Option<String> {
    state
        .workspaces
        .workspaces
        .get(&ws)
        .and_then(|w| w.output.clone())
}

/// A window's current column index, straight from niri -- not tracked by
/// us for this purpose (the tracking we DO keep, `last_known_columns`, is
/// only to detect *changes*, below).
fn window_column(state: &EventStreamState, id: u64) -> Option<usize> {
    state
        .windows
        .windows
        .get(&id)
        .and_then(|w| w.layout.pos_in_scrolling_layout)
        .map(|(col, _tile)| col)
}

/// Record each window's current column so the next WindowLayoutsChanged
/// event can tell whether anything actually moved. Column index is
/// unaffected by width changes, so it's safe to seed this right after we
/// issue our own resize actions -- it won't go stale from that.
fn seed_column_cache(state: &EventStreamState, windows: &[u64], cache: &mut HashMap<u64, usize>) {
    for &id in windows {
        if let Some(col) = window_column(state, id) {
            cache.insert(id, col);
        }
    }
}

/// Re-derive and apply the column-widths rule for whatever count is
/// currently on this workspace. Purely positional -- no history. Safe to
/// call repeatedly; re-applying the same width is a no-op.
fn reconcile(
    action_socket: &mut Socket,
    state: &EventStreamState,
    outputs: &HashMap<String, Output>,
    config: &column_widths_mode::ColumnWidthsConfig,
    paused: &Arc<AtomicBool>,
    ws: u64,
    windows: &[u64],
) {
    if paused.load(Ordering::SeqCst) {
        return;
    }
    if !config.applies_to(workspace_output(state, ws).as_deref(), outputs) {
        return;
    }

    let sorted = column_widths_mode::columns_by_position(state, windows);
    let num_windows = sorted.len();
    log::debug!("windows: {}, sorted: {}", windows.len(), sorted.len());
    if num_windows < 1 {
        return;
    }

    match num_windows {
        0 => {}
        1 => {
            column_widths_mode::set_sole(action_socket, sorted[0])
                .unwrap_or_else(|error| error!("{error:?}"));
        }
        2 => {
            column_widths_mode::set_duo(action_socket, sorted[0], sorted[1])
                .unwrap_or_else(|error| error!("{error:?}"));
        }
        3 => {
            column_widths_mode::set_trio(action_socket, sorted[0], sorted[1], sorted[2])
                .unwrap_or_else(|error| error!("{error:?}"));
        }
        _ => {
            column_widths_mode::set_new_window_default(
                action_socket,
                num_windows,
                *sorted.last().unwrap(),
            )
            .unwrap_or_else(|error| error!("{error:?}"));
        }
    }
}

fn main() {
    // Parse arguments
    let args = Args::parse();

    // Show help message if the -h / --help arg is passed
    if args.help {
        help::show_help();
        return;
    }

    // Show name and version if the -V / --version arg is passed
    if args.version {
        version::show_version();
        return;
    }

    // Initialize logger
    env_logger::init();

    // Create (if needed) and acquire lockfile
    // Exit if there's already an instance running
    // or if there was an issue creating or acquiring the lockfile (e.g. permission issue)
    let _lockfile = lockfile::acquire_lockfile().unwrap_or_else(|error| {
        error!("{error:?}");
        process::exit(1);
    });

    let config = column_widths_mode::ColumnWidthsConfig::load();
    let paused = Arc::new(AtomicBool::new(false));

    let paused_for_signal = Arc::clone(&paused);
    std::thread::spawn(move || {
        let mut signals = match Signals::new([SIGUSR1]) {
            Ok(s) => s,
            Err(error) => {
                error!("Failed to register SIGUSR1 handler: {error:?}");
                return;
            }
        };
        for _ in signals.forever() {
            let now_paused = !paused_for_signal.load(Ordering::SeqCst);
            paused_for_signal.store(now_paused, Ordering::SeqCst);
            if now_paused {
                info!("[column-widths] paused");
            } else {
                info!("[column-widths] resumed");
            }
        }
    });

    // Initialize connections to niri IPC socket, start the event stream and gather events
    let (event_socket, mut action_socket) = socket_connections::init_socket_connections()
        .unwrap_or_else(|error| {
            error!("{error:?}");
            process::exit(2);
        });

    // Gather state and create an outputs map
    // This can be dropped once https://github.com/Antiz96/oniri/issues/3 is resolved
    let mut state = EventStreamState::default();
    let outputs: HashMap<String, Output> =
        outputs_map::init_outputs_map(&mut action_socket, &config).unwrap_or_else(|error| {
            error!("{error:?}");
            process::exit(3);
        });

    // Create a workspace/window(s) map and initialize it
    let mut workspace_windows =
        windows_map::init_windows_map(&mut action_socket).unwrap_or_else(|error| {
            error!("{error:?}");
            process::exit(4);
        });

    // Track windows that have moved workspaces until their tile size is properly recalculated
    // in the "WindowLayoutsChanged" event
    // This avoids comparing a window's old layout size against its new workspace output's resolution
    // See https://github.com/Antiz96/oniri/issues/79 for details
    // This can be dropped once https://github.com/Antiz96/oniri/issues/3 is resolved
    let mut moved_windows_map = HashMap::<u64, u64>::new();

    // column-widths mode only: last-seen column index per window, used to
    // tell a genuine column move apart from a mere resize (including our
    // own) in WindowLayoutsChanged -- see reconcile_duo_trio's call site.
    let mut last_known_columns: HashMap<u64, usize> = HashMap::new();
    for windows in workspace_windows.values() {
        seed_column_cache(&state, windows, &mut last_known_columns);
    }

    // Read events gathered from the IPC socket
    let mut read_event = event_socket.read_events();

    // Loop over events
    while let Ok(event) = read_event() {
        // Update state
        // This can be dropped once https://github.com/Antiz96/oniri/issues/3 is resolved
        state.apply(event.clone());

        // Filter events
        match event {
            // Window being opened
            Event::WindowOpenedOrChanged { window } => {
                // Skip floating windows (they cannot/should not be maximized or width-managed)
                if window.is_floating {
                    continue;
                }

                debug!("Trigger Event: Window Opened Or Changed");

                let id = window.id;
                let Some(ws) = window.workspace_id else {
                    continue;
                };

                // Workaround IPC limitation by checking if the window that triggered the event is
                // in the same workspace.
                //
                // If not, track the previous workspace to act on it (if needed), for instance if the current
                // window is moved to another workspace and there's only one window remaining on the
                // previous one (which should therefore be maximized).
                //
                // This will differentiate between WindowOpened and WindowChanged.
                let previous_ws = workspace_windows
                    .iter()
                    .find_map(|(&tracked_ws, windows)| windows.contains(&id).then_some(tracked_ws));

                if previous_ws == Some(ws) {
                    continue;
                }

                // Update the workspace/window(s) map
                for windows in workspace_windows.values_mut() {
                    windows.retain(|&wid| wid != id);
                }

                let windows = workspace_windows.entry(ws).or_default();
                windows.push(id);
                let windows_snapshot = windows.clone();

                if windows_snapshot.len() > 4 {
                    if config.applies_to(workspace_output(&state, ws).as_deref(), &outputs)
                        && !paused.load(Ordering::SeqCst)
                    {
                        column_widths_mode::set_new_window_default(
                            &mut action_socket,
                            windows_snapshot.len(),
                            id,
                        )
                        .unwrap_or_else(|error| error!("{error:?}"));
                    }
                } else {
                    reconcile(
                        &mut action_socket,
                        &state,
                        &outputs,
                        &config,
                        &paused,
                        ws,
                        &windows_snapshot,
                    );
                    seed_column_cache(&state, &windows_snapshot, &mut last_known_columns);
                }

                // If the window that triggered the event has been moved to another workspace:
                if let Some(old_ws) = previous_ws
                    && old_ws != ws
                {
                    if let Some(old_windows) = workspace_windows.get(&old_ws) {
                        let old_windows = old_windows.clone();
                        if old_windows.len() <= 4 {
                            reconcile(
                                &mut action_socket,
                                &state,
                                &outputs,
                                &config,
                                &paused,
                                old_ws,
                                &old_windows,
                            );
                            seed_column_cache(&state, &old_windows, &mut last_known_columns);
                        }
                        // still >4 after losing one: nothing to do, matches the "leave existing untouched" rule
                    }

                    // Track windows that have moved workspaces until their tile size is properly recalculated
                    // in the "WindowLayoutsChanged" event
                    // This avoids comparing a window's old layout size against its new workspace output's resolution
                    // See https://github.com/Antiz96/oniri/issues/79 for details
                    // This can be dropped once https://github.com/Antiz96/oniri/issues/3 is resolved
                    moved_windows_map.insert(id, old_ws);
                }
            }

            // Window being closed
            Event::WindowClosed { id } => {
                debug!("Trigger Event: Window Closed");

                let Some((&ws, windows)) = workspace_windows
                    .iter_mut()
                    .find(|(_, windows)| windows.contains(&id))
                else {
                    continue;
                };

                // Update the workspace vector
                windows.retain(|&wid| wid != id);

                last_known_columns.remove(&id);
                let windows_snapshot = windows.clone();
                if windows_snapshot.len() <= 4 {
                    reconcile(
                        &mut action_socket,
                        &state,
                        &outputs,
                        &config,
                        &paused,
                        ws,
                        &windows_snapshot,
                    );
                    seed_column_cache(&state, &windows_snapshot, &mut last_known_columns);
                }
            }
            // Ignore other events
            _ => {}
        }
    }
}
