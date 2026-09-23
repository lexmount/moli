use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags};
use tempfile::TempDir;

pub(crate) struct SqliteSnapshot {
    // Close the connection before removing its files (fields drop in order).
    pub connection: Connection,
    _directory: TempDir,
}

impl SqliteSnapshot {
    /// Copy a database and its WAL companions. The source files must stay
    /// stable during copying; this is not an online-backup transaction.
    pub fn open(source: &Path, label: &str) -> Result<Self> {
        let directory =
            tempfile::tempdir().with_context(|| format!("failed to create {label} snapshot"))?;
        let database = directory.path().join("database.sqlite");
        fs::copy(source, &database)
            .with_context(|| format!("failed to snapshot {label} `{}`", source.display()))?;
        for suffix in ["-wal", "-shm"] {
            let companion = companion_path(source, suffix);
            match fs::copy(
                &companion,
                directory.path().join(format!("database.sqlite{suffix}")),
            ) {
                Ok(_) => {}
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("failed to snapshot {label} `{}`", companion.display())
                    });
                }
            }
        }
        let connection = Connection::open_with_flags(&database, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .with_context(|| format!("failed to open {label} snapshot"))?;
        Ok(Self {
            connection,
            _directory: directory,
        })
    }
}

fn companion_path(source: &Path, suffix: &str) -> PathBuf {
    let mut companion = source.as_os_str().to_owned();
    companion.push(suffix);
    companion.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_reads_wal_without_changing_source_and_rejects_writes() -> Result<()> {
        check_wal_snapshot(Path::new("cookies.sqlite"))
    }

    #[test]
    fn snapshot_reads_wal_with_unicode_file_names() -> Result<()> {
        check_wal_snapshot(Path::new("cookies-浏览器.sqlite"))
    }

    #[cfg(unix)]
    #[test]
    fn companion_paths_preserve_non_utf8_file_names() {
        use std::os::unix::ffi::OsStrExt;

        // Check byte preservation without requiring the filesystem to accept
        // invalid UTF-8 names: APFS rejects them before SQLite can open a file.
        let source = Path::new(std::ffi::OsStr::from_bytes(b"cookies-\xff.sqlite"));
        for (suffix, expected) in [
            ("-wal", b"cookies-\xff.sqlite-wal"),
            ("-shm", b"cookies-\xff.sqlite-shm"),
        ] {
            assert_eq!(
                companion_path(source, suffix).as_os_str().as_bytes(),
                expected
            );
        }
    }

    // Apple's filesystems require UTF-8 names. Keep the real non-UTF-8 I/O
    // coverage on Unix platforms that support such names as well.
    #[cfg(all(unix, not(target_vendor = "apple")))]
    #[test]
    fn snapshot_preserves_non_utf8_paths_when_finding_wal_files() -> Result<()> {
        use std::os::unix::ffi::OsStrExt;

        check_wal_snapshot(Path::new(std::ffi::OsStr::from_bytes(
            b"cookies-\xff.sqlite",
        )))
    }

    fn check_wal_snapshot(name: &Path) -> Result<()> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join(name);
        let writer = Connection::open(&source)?;
        writer.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA wal_autocheckpoint=0;
             CREATE TABLE entries (value TEXT);
             PRAGMA wal_checkpoint(TRUNCATE);
             INSERT INTO entries VALUES ('only in WAL');",
        )?;
        let files = fs::read_dir(directory.path())?
            .map(|entry| {
                let path = entry?.path();
                let bytes = fs::read(&path)?;
                Ok((path, bytes))
            })
            .collect::<std::io::Result<Vec<_>>>()?;
        let snapshot = SqliteSnapshot::open(&source, "test database")?;
        let value: String =
            snapshot
                .connection
                .query_row("SELECT value FROM entries", [], |row| row.get(0))?;
        assert_eq!(value, "only in WAL");
        let error = snapshot
            .connection
            .execute("INSERT INTO entries VALUES ('forbidden')", [])
            .unwrap_err();
        assert_eq!(
            error.sqlite_error_code(),
            Some(rusqlite::ErrorCode::ReadOnly)
        );
        drop(snapshot);
        for (path, bytes) in files {
            assert_eq!(fs::read(path)?, bytes);
        }
        Ok(())
    }
}
