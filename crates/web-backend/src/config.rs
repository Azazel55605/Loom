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

/// Resolves the data directory, creates it if missing, and checks it is
/// writable.
///
/// Creating it here rather than demanding the operator pre-create it is part of
/// the same zero-config requirement: a first run should not fail on a missing
/// directory it could have made itself.
///
/// The writability probe exists because the failure it catches is otherwise
/// invisible. `create_dir_all` succeeds on a directory that already exists but
/// is not writable — the common case being a Docker named volume mounted at a
/// path absent from the image, which Docker creates as `root:root` while the
/// server runs unprivileged. Without this check the first sign of trouble is
/// SQLite's `code: 14, "unable to open database file"`, which names neither
/// permissions nor the directory, and sends you debugging the database layer
/// instead of the mount.
pub fn data_dir() -> std::io::Result<PathBuf> {
    let dir = match std::env::var("LOOM_DATA_DIR") {
        // An unset Docker `ARG` or an empty Compose value arrives as `""`
        // rather than as absent, and `""` would resolve to the filesystem root.
        Ok(value) if !value.trim().is_empty() => PathBuf::from(value.trim()),
        _ => PathBuf::from(DEFAULT_DATA_DIR),
    };

    std::fs::create_dir_all(&dir).map_err(|error| {
        std::io::Error::new(
            error.kind(),
            format!(
                "could not create the data directory {}: {error}. \
                 Set LOOM_DATA_DIR to a writable path.",
                dir.display()
            ),
        )
    })?;

    ensure_writable(&dir)?;
    Ok(dir)
}

/// Confirms the process can actually create files in `dir`.
///
/// Probing by writing rather than by reading the mode bits: permissions are not
/// the only thing that can stop a write — a read-only mount, a full filesystem,
/// or an ACL will all pass a mode check and fail the actual create. The probe
/// file is removed immediately; failing to remove it is not fatal, since the
/// question being asked has already been answered.
fn ensure_writable(dir: &Path) -> std::io::Result<()> {
    let probe = dir.join(".loom-write-probe");

    match std::fs::File::create(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            Ok(())
        }
        Err(error) => Err(std::io::Error::new(
            error.kind(),
            format!(
                "the data directory {} is not writable by this process: \
                 {error}. In Docker this usually means the volume mounted \
                 there is owned by root while the container runs \
                 unprivileged — recreate it with `docker compose down -v`, or \
                 point LOOM_DATA_DIR at a writable path.",
                dir.display(),
            ),
        )),
    }
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
