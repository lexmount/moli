use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::metadata::{META_FILE, read_metadata_file};

#[derive(Debug)]
struct CacheEntryForEviction {
    entry_dir: PathBuf,
    last_used_at_unix_ms: u64,
    size: u64,
    protected: bool,
}

pub(crate) fn prune_cache_to_max_bytes(root: &Path, max_bytes: u64, protected_entry_dir: &Path) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    let mut cache_entries = entries
        .flatten()
        .filter_map(|entry| cache_entry_for_eviction(entry.path(), protected_entry_dir))
        .collect::<Vec<_>>();
    let mut total_size = cache_entries.iter().map(|entry| entry.size).sum::<u64>();
    if total_size <= max_bytes {
        return;
    }

    cache_entries.sort_by_key(|entry| (entry.protected, entry.last_used_at_unix_ms));
    for entry in cache_entries {
        if total_size <= max_bytes {
            break;
        }
        if entry.protected {
            continue;
        }
        if fs::remove_dir_all(&entry.entry_dir).is_ok() {
            total_size = total_size.saturating_sub(entry.size);
        }
    }
}

fn cache_entry_for_eviction(
    entry_dir: PathBuf,
    protected_entry_dir: &Path,
) -> Option<CacheEntryForEviction> {
    if !entry_dir.is_dir()
        || !entry_dir
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(".entry"))
    {
        return None;
    }
    let size = directory_size(&entry_dir);
    let last_used_at_unix_ms = read_metadata_file(&entry_dir.join(META_FILE))
        .ok()
        .map(|metadata| metadata.last_used_at_unix_ms)
        .unwrap_or(0);
    Some(CacheEntryForEviction {
        protected: entry_dir.as_path() == protected_entry_dir,
        entry_dir,
        last_used_at_unix_ms,
        size,
    })
}

use crate::store::directory_size;
