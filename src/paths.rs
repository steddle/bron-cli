use std::path::PathBuf;

use anyhow::{Context, Result};

/// Overrides every location below. Set it to keep a corpus outside the user
/// root, for example a per-project one in CI.
const HOME_ENV: &str = "BRON_HOME";

fn home_override() -> Option<PathBuf> {
    std::env::var_os(HOME_ENV).map(PathBuf::from)
}

/// Directory holding the corpus database and the frozen documents behind it.
pub fn data_dir() -> Result<PathBuf> {
    match home_override() {
        Some(home) => Ok(home),
        None => Ok(dirs::data_dir()
            .context("no data directory for this user")?
            .join("bron")),
    }
}

/// Content-addressed store of raw upstream bytes, one file per digest.
///
/// This is provenance, not convenience: entries are what the engine freezes and
/// what every offset refers to, so it is never evicted on a schedule.
pub fn blob_dir() -> Result<PathBuf> {
    Ok(data_dir()?.join("blobs"))
}

pub fn db_path() -> Result<PathBuf> {
    Ok(data_dir()?.join("bron.db"))
}

/// Cross-process rate-limit buckets.
///
/// Deliberately a fixed `~/.bron/buckets` rather than the platform data
/// directory: other languages honour these files from the other side of the
/// same machine, and a path that moves between macOS and Linux would have to be
/// reimplemented per platform by every one of them.
pub fn bucket_dir() -> Result<PathBuf> {
    if let Some(home) = home_override() {
        return Ok(home.join("buckets"));
    }
    Ok(dirs::home_dir()
        .context("no home directory for this user")?
        .join(".bron")
        .join("buckets"))
}
