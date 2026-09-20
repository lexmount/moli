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
            let companion = sqlite_companion_path(source, suffix);
            match fs::copy(
                &companion,
                directory.path().join(format!("database.sqlite{suffix}")),
            ) {
                Ok(_) => {}
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!(
                            "failed to snapshot {label} `{}`",
                            Path::new(&companion).display()
                        )
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

fn sqlite_companion_path(source: &Path, suffix: &str) -> PathBuf {
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

    #[cfg(target_os = "linux")]
    #[test]
    fn companion_paths_preserve_non_utf8_database_names() {
        use std::os::unix::ffi::OsStrExt;

        let source = Path::new(std::ffi::OsStr::from_bytes(b"cookies-\xff.sqlite"));
        assert_eq!(
            sqlite_companion_path(source, "-wal").as_os_str().as_bytes(),
            b"cookies-\xff.sqlite-wal",
        );
        assert_eq!(
            sqlite_companion_path(source, "-shm").as_os_str().as_bytes(),
            b"cookies-\xff.sqlite-shm",
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn snapshot_preserves_non_ascii_paths_when_finding_wal_files() -> Result<()> {
        check_wal_snapshot(Path::new("cookies-é.sqlite"))
    }

    #[cfg(target_os = "linux")]
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
        // SQLite's filename conversion differs by platform. Create the WAL
        // through an ordinary path, then rename its three files so this test
        // isolates the snapshot's handling of non-UTF-8 source paths.
        let writer_source = directory.path().join("cookies.sqlite");
        let writer = Connection::open(&writer_source)?;
        writer.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA wal_autocheckpoint=0;
             CREATE TABLE entries (value TEXT);
             PRAGMA wal_checkpoint(TRUNCATE);
             INSERT INTO entries VALUES ('only in WAL');",
        )?;
        if source != writer_source {
            for suffix in ["", "-wal", "-shm"] {
                let mut original = writer_source.as_os_str().to_owned();
                original.push(suffix);
                let mut renamed = source.as_os_str().to_owned();
                renamed.push(suffix);
                fs::rename(Path::new(&original), Path::new(&renamed))?;
            }
        }
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
