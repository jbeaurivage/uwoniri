use std::{io, path::PathBuf};

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

    #[error("failed to connect to niri IPC socket ({which})")]
    Connect {
        which: &'static str,
        #[source]
        source: io::Error,
    },

    #[error("failed to send {what} request to niri")]
    Send {
        what: &'static str,
        #[source]
        source: io::Error,
    },

    #[error("niri rejected {what} request: {message}")]
    Niri { what: &'static str, message: String },

    #[error("unexpected response to {what} request: {response}")]
    UnexpectedResponse {
        what: &'static str,
        response: String,
    },

    #[error("failed to read event from niri IPC event stream")]
    ReadEvent(#[source] io::Error),
}

/// Render an error together with its chain of sources on one line
pub fn report(err: &dyn StdError) -> String {
    let mut out = err.to_string();
    let mut source = err.source();
    while let Some(cause) = source {
        out.push_str(": ");
        out.push_str(&cause.to_string());
        source = cause.source();
    }
    out
}
