use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::TransactionDurability;

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

pub(super) fn prepare_directory(path: &Path) -> io::Result<()> {
    let mut parents = Vec::new();
    let mut missing = path;
    while !missing.try_exists()? {
        let parent = missing
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        parents.push(parent);
        missing = parent;
    }
    fs::create_dir_all(path)?;
    // A later strict file commit also depends on newly created directories
    // remaining reachable. Sync each new directory entry once at creation.
    for parent in parents.into_iter().rev() {
        sync_directory(parent)?;
    }
    Ok(())
}

/// Keep the previous snapshot until the replacement has reached its requested
/// durability. In particular, a directory-sync error must not publish a commit
/// which the renderer will report as aborted.
pub(super) fn replace(
    path: &Path,
    bytes: &[u8],
    durability: TransactionDurability,
) -> io::Result<()> {
    replace_with_directory_sync(path, bytes, durability, sync_directory)
}

fn replace_with_directory_sync(
    path: &Path,
    bytes: &[u8],
    durability: TransactionDurability,
    mut sync_parent: impl FnMut(&Path) -> io::Result<()>,
) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let id = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let name = format!(".moli-idb-{}-{nonce}-{id}", std::process::id());
    let next = parent.join(format!("{name}.next"));
    let previous = parent.join(format!("{name}.previous"));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&next)?;
    let mut replaced = false;
    let mut has_previous = false;
    let result = (|| {
        // A hard link preserves the complete old file without copying records.
        // Both names are in the same directory/filesystem as the new snapshot.
        match fs::hard_link(path, &previous) {
            Ok(()) => has_previous = true,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        file.write_all(bytes)?;
        if durability == TransactionDurability::Strict {
            file.sync_all()?;
        }
        drop(file);
        fs::rename(&next, path)?;
        replaced = true;
        if durability == TransactionDurability::Strict {
            sync_parent(parent)?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        if replaced {
            let rollback = if has_previous {
                fs::rename(&previous, path)
            } else {
                fs::remove_file(path)
            };
            if let Err(rollback) = rollback {
                // Leave the backup available if the filesystem cannot restore it.
                return Err(io::Error::new(
                    error.kind(),
                    format!("{error}; failed to restore previous IndexedDB snapshot: {rollback}"),
                ));
            }
            if durability == TransactionDurability::Strict {
                sync_parent(parent)?;
            }
        }
        let _ = fs::remove_file(&next);
        let _ = fs::remove_file(&previous);
        return Err(error);
    }
    // The published snapshot is already durable. Cleanup cannot turn an
    // acknowledged replacement into a failed transaction.
    let _ = fs::remove_file(&previous);
    Ok(())
}

#[cfg(not(windows))]
fn sync_directory(path: &Path) -> io::Result<()> {
    fs::File::open(path)?.sync_all()
}

#[cfg(windows)]
fn sync_directory(path: &Path) -> io::Result<()> {
    use std::os::windows::fs::OpenOptionsExt;

    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const GENERIC_WRITE: u32 = 0x4000_0000;

    OpenOptions::new()
        .access_mode(GENERIC_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)?
        .sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directory_sync_failure_restores_previous_or_absent_snapshot_and_allows_retry() {
        for previous in [false, true] {
            let id = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = std::env::temp_dir().join(format!(
                "moli-idb-sync-failure-{}-{nonce}-{id}",
                std::process::id()
            ));
            prepare_directory(&root).unwrap();
            let path = root.join("origin.json");
            if previous {
                fs::write(&path, b"old").unwrap();
            }
            let mut syncs = 0;
            let error = replace_with_directory_sync(
                &path,
                b"discarded",
                TransactionDurability::Strict,
                |parent| {
                    syncs += 1;
                    if syncs == 1 {
                        Err(io::Error::other("injected directory-sync failure"))
                    } else {
                        sync_directory(parent)
                    }
                },
            )
            .unwrap_err();
            assert_eq!(error.to_string(), "injected directory-sync failure");
            assert_eq!(syncs, 2);
            if previous {
                assert_eq!(fs::read(&path).unwrap(), b"old");
            } else {
                assert!(!path.exists());
            }
            assert_eq!(fs::read_dir(&root).unwrap().count(), usize::from(previous));
            replace(&path, b"next", TransactionDurability::Strict).unwrap();
            assert_eq!(fs::read(&path).unwrap(), b"next");
            assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
            fs::remove_dir_all(root).unwrap();
        }
    }
}
