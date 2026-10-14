//! Create (if needed) and acquire lockfile
//! in order to prevent multiple instances to run at the same time

use fs2::FileExt;
use std::fs::{self, File, OpenOptions};
use std::io::ErrorKind;

use crate::error::{Error, Result};

const PACKAGE_NAME: &str = env!("CARGO_PKG_NAME");

pub fn acquire_lockfile() -> Result<File> {
    // Create oniri cachedir (if it doesn't exist)
    let cachedir = dirs::cache_dir()
        .ok_or(Error::NoDir("cache"))?
        .join(PACKAGE_NAME);

    fs::create_dir_all(&cachedir).map_err(|source| Error::CreateCacheDir {
        path: cachedir.clone(),
        source,
    })?;

    let path = cachedir.join(format!("{PACKAGE_NAME}.lock"));

    let lockfile = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&path)
        .map_err(|source| Error::OpenLockfile {
            path: path.clone(),
            source,
        })?;

    lockfile.try_lock_exclusive().map_err(|source| {
        if source.kind() == ErrorKind::WouldBlock {
            Error::AlreadyRunning(PACKAGE_NAME)
        } else {
            Error::AcquireLock { path, source }
        }
    })?;

    Ok(lockfile)
}
