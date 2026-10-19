//! Control socket so external tools (e.g. niri keybinds) can send commands.
//!
//! Protocol: one newline-terminated command per connection:
//! `pause`, `unpause` or `retile`. The reply is `ok` or `error: <reason>`.

use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use clap::ValueEnum;
use log::{debug, error, info, warn};

use crate::config::Widths;
use crate::error::{Error, Result, report};
use crate::ipc::ActionSocket;
use crate::layout;

/// Commands the daemon understands. The wire format is the lowercase variant
/// name (`pause`, `unpause`, `retile`), one per line.
#[derive(Clone, Copy, clap::ValueEnum)]
pub enum Command {
    /// Stop adjusting layouts until unpaused
    Pause,
    /// Resume adjusting layouts
    Unpause,
    /// Make the focused window the master and re-width the others
    Retile,
}

impl Command {
    /// Client side: send `cmd` to the running daemon and wait for its reply.
    pub fn send(&self) -> Result<()> {
        let path = socket_path()?;
        let mut stream = UnixStream::connect(&path).map_err(|source| Error::ControlConnect {
            path: path.clone(),
            source,
        })?;
        // A retile waits on niri, so allow for some latency.
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .map_err(Error::ControlIo)?;

        let name = self
            .to_possible_value()
            .expect("no skipped variants")
            .get_name()
            .to_owned();
        writeln!(stream, "{name}").map_err(Error::ControlIo)?;

        let mut reply = String::new();
        BufReader::new(&stream)
            .read_line(&mut reply)
            .map_err(Error::ControlIo)?;

        match reply.trim() {
            "ok" => Ok(()),
            other => Err(Error::ControlRejected(
                other.strip_prefix("error: ").unwrap_or(other).to_owned(),
            )),
        }
    }
}

fn socket_path() -> Result<PathBuf> {
    Ok(dirs::runtime_dir()
        .ok_or(Error::NoDir("runtime"))?
        .join(concat!(env!("CARGO_PKG_NAME"), ".sock")))
}

/// Bind the control socket and serve it from a background thread.
pub fn spawn(
    paused: Arc<AtomicBool>,
    action_socket: Arc<Mutex<ActionSocket>>,
    width_config: Widths,
) -> Result<()> {
    let path = socket_path()?;

    // We hold the lockfile, so any socket file still lying around is stale.
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).map_err(|source| Error::BindControlSocket {
        path: path.clone(),
        source,
    })?;
    info!("listening for commands on {}", path.display());

    std::thread::spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => handle(stream, &paused, action_socket.clone(), &width_config),
                Err(err) => warn!("control socket: failed to accept connection: {err}"),
            }
        }
    });
    Ok(())
}

fn read_line(stream: &UnixStream) -> io::Result<String> {
    // Don't let a silent client wedge the listener.
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line)?;
    Ok(line)
}

fn handle(
    mut stream: UnixStream,
    paused: &AtomicBool,
    socket: Arc<Mutex<ActionSocket>>,
    width_config: &Widths,
) {
    let outcome = read_line(&stream)
        .map_err(|err| format!("failed to read command: {err}"))
        .and_then(|line| Command::from_str(line.trim(), false))
        .and_then(|cmd| run(cmd, paused, socket, width_config).map_err(|err| report(&err)));

    let reply = match outcome {
        Ok(()) => "ok\n".to_string(),
        Err(msg) => {
            error!("control socket: {msg}");
            format!("error: {msg}\n")
        }
    };
    if let Err(err) = stream.write_all(reply.as_bytes()) {
        debug!("control socket: failed to send reply: {err}");
    }
}

fn run(
    cmd: Command,
    paused: &AtomicBool,
    socket: Arc<Mutex<ActionSocket>>,
    width_config: &Widths,
) -> Result<()> {
    let mut socket = socket.lock().unwrap();
    match cmd {
        Command::Pause => {
            paused.store(true, Ordering::SeqCst);
            info!("paused");
            Ok(())
        }
        Command::Unpause => {
            paused.store(false, Ordering::SeqCst);
            info!("resumed");
            Ok(())
        }
        Command::Retile => layout::retile(&mut socket, width_config),
    }
}
