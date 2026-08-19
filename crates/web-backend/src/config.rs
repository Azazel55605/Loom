//! Where the server keeps its state, resolved without requiring configuration.
//!
//! Per `docs/adr/0004-zero-config-startup.md` every runtime setting has a
//! working default, so `docker compose up` and `cargo run` both work with no
//! environment at all. Nothing here may become a required variable.

use std::path::{Path, PathBuf};

/// Data directory used when `LOOM_DATA_DIR` is unset.
///
/// Relative on purpose. In a container `LOOM_DATA_DIR=/data` is set by the
/// Compose files and backed by a volume; on a developer's machine an absolute
/// `/data` would need root to create, so the fallback is beside the working
/// directory instead.
const DEFAULT_DATA_DIR: &str = "./data";

/// Database filename inside the data directory.
const DATABASE_FILENAME: &str = "loom.db";

/// Resolves the data directory and makes sure it exists.
///
/// Creating it here rather than demanding the operator pre-create it is part of
/// the same zero-config requirement: a first run should not fail on a missing
/// directory it could have made itself.
pub fn data_dir() -> std::io::Result<PathBuf> {
    let dir = match std::env::var("LOOM_DATA_DIR") {
        // An unset Docker `ARG` or an empty Compose value arrives as `""`
        // rather than as absent, and `""` would resolve to the filesystem root.
        Ok(value) if !value.trim().is_empty() => PathBuf::from(value.trim()),
        _ => PathBuf::from(DEFAULT_DATA_DIR),
    };

    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Full path to the SQLite database file inside `dir`.
pub fn database_path(dir: &Path) -> PathBuf {
    dir.join(DATABASE_FILENAME)
}

/// Builds the SQLite connection URL for a database file.
///
/// `?mode=rwc` is what creates the file when it does not exist yet. Without it
/// sqlx opens read-write but will not create, so a first run fails with a bare
/// "unable to open database file" that says nothing about the cause.
pub fn database_url(path: &Path) -> String {
    format!("sqlite://{}?mode=rwc", path.display())
}
