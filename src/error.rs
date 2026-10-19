use std::{io, path::PathBuf};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to determine the {0} directory")]
    NoDir(&'static str),

    #[error("failed to create the {} cache directory", path.display())]
    CreateCacheDir {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("failed to open lockfile: {}", path.display())]
    OpenLockfile {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("another instance of {0} is already running")]
    AlreadyRunning(&'static str),

    #[error("failed to bind control socket {}", path.display())]
    BindControlSocket {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("invalid config file {}: {reason}", path.display())]
    ConfigInvalid { path: PathBuf, reason: String },

    #[error("failed to acquire lock on the {} lockfile", path.display())]
    AcquireLock {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("failed to read config file {}", path.display())]
    ConfigRead {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("failed to parse config file {}", path.display())]
    ConfigParse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },

    #[error("failed to connect to niri IPC socket")]
    Connect(#[source] io::Error),

    #[error("failed to send request to niri")]
    Send(#[source] io::Error),

    #[error("niri rejected request: {0}")]
    Niri(String),

    #[error("unexpected response: {0}")]
    UnexpectedResponse(String),

    #[error("failed to read event from niri IPC event stream")]
    ReadEvent(#[source] io::Error),

    #[error("error occured")]
    Anyhow(#[from] anyhow::Error),

    #[error("failed to connect to control socket {} (is the daemon running?)", path.display())]
    ControlConnect {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("control socket I/O failed")]
    ControlIo(#[source] io::Error),

    #[error("daemon rejected the command: {0}")]
    ControlRejected(String),
}

/// Render an error together with its chain of sources on one line
pub fn report(err: &dyn std::error::Error) -> String {
    let mut out = err.to_string();
    let mut source = err.source();
    while let Some(cause) = source {
        out.push_str(": ");
        out.push_str(&cause.to_string());
        source = cause.source();
    }
    out
}
