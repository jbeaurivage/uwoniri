use clap::Parser;
use log::{debug, error};
use niri_ipc::{Event, Window};
use std::process;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::error::{Error, Result, report};
use crate::ipc::ActionSocket;
use crate::windows_map::WindowMap;

mod config;
mod error;
mod ipc;
mod layout;
mod listener;
mod lockfile;
mod windows_map;

use config::Config;

/// Dynamically resize windows on ultrawide monitors
#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Args {
    /// Dynamically retile as windows are added or removed in the workspade (disabled by default)
    #[arg(short, long, default_value_t = false)]
    dynamic: bool,
}

/// Log a failed operation without aborting the event loop.
fn log_failure(result: Result<()>) {
    if let Err(err) = result {
        error!("{}", report(&err));
    }
}

struct Daemon {
    socket: Arc<Mutex<ActionSocket>>,
    config: Config,
    paused: Arc<AtomicBool>,
    windows: WindowMap,
}

impl Daemon {
    /// Re-apply the layout for workspace `ws`. `new` is the window that was
    /// just added to it, if any: once there are more windows than the layouts
    /// cover, only that window gets the overflow width and the existing ones
    /// are left untouched.
    fn update(&self, ws: u64, new: Option<u64>) -> Result<()> {
        if self.paused.load(Ordering::SeqCst) {
            return Ok(());
        }

        // Held for the whole update so the listener thread (retile) can't
        // interleave its actions with ours.
        let mut guard = self.socket.lock().unwrap();
        let socket = &mut *guard;

        let output = layout::workspace_output(socket, ws)?;

        // Only ask niri for the output size if the config actually uses it.
        let width = match (&output, self.config.min_width) {
            (Some(name), Some(_)) => layout::output_width(socket, name)?,
            _ => None,
        };

        if !self.config.applies_to(output.as_deref(), width) {
            return Ok(());
        }

        let widths = &self.config.widths;
        let ids = self.windows.windows(ws);

        if ids.len() > widths.max_managed() + 1 {
            return match new {
                Some(id) => layout::set_new_window_default(socket, widths, id),
                None => Ok(()),
            };
        }

        let sorted = layout::columns_by_position(socket, ids)?;
        debug!("workspace {ws}: windows {ids:?}, sorted {sorted:?}");

        match sorted.len() {
            0 => Ok(()),
            n if n <= widths.max_managed() => layout::set_layout(socket, &sorted, widths),
            n => layout::set_new_window_default(socket, widths, sorted[n - 1]),
        }
    }

    fn on_window_opened_or_changed(&mut self, window: Window) {
        // Floating windows should not be width-managed
        if window.is_floating {
            if let Some(ws) = self.windows.remove(window.id) {
                log_failure(self.update(ws, None));
            }
            return;
        }

        let Some(ws) = window.workspace_id else {
            return;
        };
        let id = window.id;

        debug!("Trigger Event: Window Opened Or Changed");

        // The IPC doesn't distinguish "opened" from "changed": a window we
        // already track on this workspace is just a change, so ignore it.
        if self.windows.workspace_of(id) == Some(ws) {
            return;
        }

        let previous_ws = self.windows.place(id, ws);
        log_failure(self.update(ws, Some(id)));

        // Moved from another workspace: tidy up what it left behind (e.g. the
        // one remaining window there should now be the sole window).
        if let Some(old_ws) = previous_ws {
            log_failure(self.update(old_ws, None));
        }
    }

    fn on_window_closed(&mut self, id: u64) {
        debug!("Trigger Event: Window Closed");

        if let Some(ws) = self.windows.remove(id) {
            log_failure(self.update(ws, None));
        }
    }
}

fn run(start_paused: bool) -> Result<()> {
    // Exit if there's already an instance running or the lockfile can't be
    // created or acquired (e.g. permission issue).
    let _lockfile = lockfile::acquire_lockfile()?;

    let config = Config::load();
    let paused = Arc::new(AtomicBool::new(start_paused));

    // Connect to the niri IPC socket and start the event stream.
    let (event_socket, action_socket) = ipc::init_socket_connections()?;
    let socket = Arc::new(Mutex::new(action_socket));

    let windows = WindowMap::init(socket.clone())?;

    // Command listener (pause / unpause / retile).
    listener::spawn(paused.clone(), socket.clone(), config.widths.clone())?;

    let mut daemon = Daemon {
        socket,
        config,
        paused,
        windows,
    };

    let mut read_event = event_socket.read_events();
    loop {
        match read_event().map_err(Error::ReadEvent)? {
            Event::WindowOpenedOrChanged { window } => daemon.on_window_opened_or_changed(window),
            Event::WindowClosed { id } => daemon.on_window_closed(id),
            _ => {}
        }
    }
}

fn main() {
    let args = Args::parse();

    env_logger::init();

    let start_paused = !args.dynamic;

    if let Err(err) = run(start_paused) {
        error!("{}", report(&err));
        process::exit(1);
    }
}
