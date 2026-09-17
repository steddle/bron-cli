use std::path::PathBuf;

use anyhow::{Context, Result};

/// Relocates the one thing bron writes. A per-project value keeps a CI run's
/// pacing to itself.
const HOME_ENV: &str = "BRON_HOME";

/// Cross-process rate-limit buckets, and the only thing bron writes.
///
/// `~/.bron/buckets` rather than the platform data directory, because other
/// languages honour these files from the other side of the same machine and a
/// path that moves between macOS and Linux would have to be reimplemented per
/// platform by every one of them. `BRON_HOME` moves it anyway, for a CI run
/// that should pace itself alone; what must not move is where it lands when
/// nobody says.
pub fn bucket_dir() -> Result<PathBuf> {
    let home = match std::env::var_os(HOME_ENV) {
        Some(home) => PathBuf::from(home),
        None => dirs::home_dir()
            .context("no home directory for this user")?
            .join(".bron"),
    };

    Ok(home.join("buckets"))
}
