use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufReader},
    path::{Path, PathBuf},
};

#[cfg(test)]
use std::io::Read;

use anyhow::{Context, Result, anyhow};
use url::Url;

use crate::{
    eviction::prune_cache_to_max_bytes,
    metadata::{
        HttpCacheEntryMetadata, META_FILE, ReadMetadataError, read_metadata_file,
        touch_metadata_last_used_if_body_matches, write_metadata_file,
    },
    path_safety::{safe_body_file_name, safe_entry_key},
    time::{stable_cache_hash, unique_suffix, unix_now_ms},
    writer::HttpCacheBodyWriter,
};

/// A completed cache entry loaded from disk.
#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpCachedEntry {
    pub metadata: HttpCacheEntryMetadata,
    pub body: Vec<u8>,
}

/// A completed cache entry whose body can be consumed incrementally.
#[derive(Debug)]
pub struct HttpCachedEntryReader {
    pub metadata: HttpCacheEntryMetadata,
    pub body: BufReader<File>,
}

/// Lightweight metadata for one readable cache entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpCacheEntryInfo {
    pub key: String,
    pub metadata: HttpCacheEntryMetadata,
    pub body_len: u64,
    pub entry_size_bytes: u64,
}

/// Size and readability summary for a cache root.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HttpCacheStats {
    pub entry_count: usize,
    pub unreadable_entry_count: usize,
    pub total_bytes: u64,
    pub readable_body_bytes: u64,
}

impl HttpCachedEntryReader {
    /// Returns the current on-disk body length without reading the body.
    pub fn body_len(&self) -> io::Result<u64> {
        self.body
            .get_ref()
            .metadata()
            .map(|metadata| metadata.len())
    }

    /// Materializes a cached body only up to an explicit caller-owned limit.
    #[cfg(test)]
    pub fn try_into_bytes(mut self, max_body_bytes: usize) -> Result<Option<HttpCachedEntry>> {
        if self
            .body_len()
            .ok()
            .is_some_and(|body_len| body_len > max_body_bytes as u64)
        {
            return Ok(None);
        }
        let mut body = Vec::new();
        let mut limited = (&mut self.body).take(max_body_bytes.saturating_add(1) as u64);
        limited
            .read_to_end(&mut body)
            .context("failed to read HTTP cache body")?;
        if body.len() > max_body_bytes {
            return Ok(None);
        }

        Ok(Some(HttpCachedEntry {
            metadata: self.metadata,
            body,
        }))
    }
}

/// Directory-backed cache store.
#[derive(Debug, Clone)]
pub struct HttpCacheStore {
    root: PathBuf,
    max_bytes: Option<u64>,
}

impl HttpCacheStore {
    /// Creates a store rooted at `root`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            max_bytes: None,
        }
    }

    /// Creates a store with a maximum on-disk size.
    pub fn with_max_bytes(root: impl Into<PathBuf>, max_bytes: Option<u64>) -> Self {
        Self {
            root: root.into(),
            max_bytes,
        }
    }

    /// Returns the stable cache key used for a request URL.
    pub fn key_for_url(url: &str) -> String {
        format!("{:016x}", stable_cache_hash(url))
    }

    /// Returns true when the store can address an entry for `key`.
    pub fn contains_entry_path(&self, key: &str) -> bool {
        self.try_entry_dir(key).is_some_and(|path| path.is_dir())
    }

    /// Loads a completed cache entry and opens its body for incremental reads.
    pub fn load_reader(&self, key: &str) -> Result<Option<HttpCachedEntryReader>> {
        let Some((_, metadata, body_path)) = self.load_metadata_and_body_path(key)? else {
            return Ok(None);
        };
        let body = match File::open(&body_path) {
            Ok(body) => BufReader::new(body),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let _ = fs::remove_file(body_path.with_file_name(META_FILE));
                return Ok(None);
            }
            Err(_) => return Ok(None),
        };

        Ok(Some(HttpCachedEntryReader { metadata, body }))
    }

    fn load_metadata_and_body_path(
        &self,
        key: &str,
    ) -> Result<Option<(PathBuf, HttpCacheEntryMetadata, PathBuf)>> {
        let Some(entry_dir) = self.try_entry_dir(key) else {
            return Ok(None);
        };
        let meta_path = entry_dir.join(META_FILE);
        if !meta_path.exists() {
            return Ok(None);
        }

        let metadata: HttpCacheEntryMetadata = match read_metadata_file(&meta_path) {
            Ok(metadata) => metadata,
            Err(ReadMetadataError::Missing) => return Ok(None),
            Err(ReadMetadataError::Invalid) => {
                let _ = fs::remove_file(&meta_path);
                return Ok(None);
            }
            Err(ReadMetadataError::Io) => return Ok(None),
        };
        if !safe_body_file_name(&metadata.body_file) {
            let _ = fs::remove_file(&meta_path);
            return Ok(None);
        }

        let body_path = entry_dir.join(&metadata.body_file);
        Ok(Some((entry_dir, metadata, body_path)))
    }

    /// Best-effort touch for an entry after the caller has accepted a cache hit.
    pub fn touch_loaded_entry(&self, key: &str, metadata: &HttpCacheEntryMetadata) -> Result<()> {
        let Some(entry_dir) = self.try_entry_dir(key) else {
            return Ok(());
        };
        touch_metadata_last_used_if_body_matches(&entry_dir, &metadata.body_file, unix_now_ms())
    }

    /// Replaces metadata for a loaded entry only if it still points at the same body file.
    ///
    /// This is used by 304 revalidation: the response metadata can become
    /// fresher while the cached body stream remains valid. The body-file guard
    /// prevents an old cache hit from overwriting metadata published by a newer
    /// writer for the same key.
    pub fn refresh_loaded_entry_metadata(
        &self,
        key: &str,
        metadata: &HttpCacheEntryMetadata,
    ) -> Result<()> {
        let Some(entry_dir) = self.try_entry_dir(key) else {
            return Ok(());
        };
        let current =
            read_metadata_file(&entry_dir.join(META_FILE)).map_err(|error| match error {
                ReadMetadataError::Missing => {
                    anyhow!("HTTP cache metadata disappeared before refresh")
                }
                ReadMetadataError::Invalid => {
                    anyhow!("HTTP cache metadata became invalid before refresh")
                }
                ReadMetadataError::Io => {
                    anyhow!("failed to read HTTP cache metadata before refresh")
                }
            })?;
        if current.body_file != metadata.body_file {
            return Ok(());
        }
        write_metadata_file(&entry_dir, metadata)
    }

    /// Removes one cache entry directory if `key` maps to a valid cache path.
    pub fn remove_entry(&self, key: &str) -> Result<()> {
        let Some(entry_dir) = self.try_entry_dir(key) else {
            return Ok(());
        };
        match fs::remove_dir_all(&entry_dir) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error).with_context(|| {
                anyhow!(
                    "failed to remove HTTP cache entry directory `{}`",
                    entry_dir.display()
                )
            }),
        }
    }

    /// Clears all cache entry directories managed by this store.
    pub fn clear(&self) -> Result<()> {
        let entries = match fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                return Err(error).with_context(|| {
                    anyhow!(
                        "failed to read HTTP cache root directory `{}`",
                        self.root.display()
                    )
                });
            }
        };

        for entry in entries {
            let entry = entry.with_context(|| {
                anyhow!(
                    "failed to read HTTP cache root entry under `{}`",
                    self.root.display()
                )
            })?;
            if !is_cache_entry_directory(&entry)? {
                continue;
            }
            let entry_path = entry.path();
            match fs::remove_dir_all(&entry_path) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(error).with_context(|| {
                        anyhow!(
                            "failed to remove HTTP cache entry directory `{}`",
                            entry_path.display()
                        )
                    });
                }
            }
        }
        Ok(())
    }

    /// Returns all readable cache entries currently published under this root.
    pub fn entries(&self) -> Result<Vec<HttpCacheEntryInfo>> {
        let mut out = Vec::new();
        for entry_dir in self.cache_entry_dirs()? {
            if let Some(info) = self.entry_info_from_dir(&entry_dir)? {
                out.push(info);
            }
        }
        Ok(out)
    }

    /// Returns cache size statistics, including unreadable entry directories.
    pub fn stats(&self) -> Result<HttpCacheStats> {
        let mut stats = HttpCacheStats::default();
        for entry_dir in self.cache_entry_dirs()? {
            let entry_size_bytes = directory_size(&entry_dir);
            stats.total_bytes = stats.total_bytes.saturating_add(entry_size_bytes);
            if let Some(info) = self.entry_info_from_dir(&entry_dir)? {
                stats.entry_count += 1;
                stats.readable_body_bytes = stats.readable_body_bytes.saturating_add(info.body_len);
            } else {
                stats.unreadable_entry_count += 1;
            }
        }
        Ok(stats)
    }

    /// Removes readable cache entries accepted by `predicate`.
    pub fn remove_entries_matching(
        &self,
        mut predicate: impl FnMut(&HttpCacheEntryInfo) -> bool,
    ) -> Result<usize> {
        let mut removed = 0usize;
        for info in self.entries()? {
            if !predicate(&info) {
                continue;
            }
            self.remove_entry(&info.key)?;
            removed += 1;
        }
        Ok(removed)
    }

    /// Removes readable cache entries whose request or final URL matches `url`'s origin.
    pub fn remove_entries_for_origin(&self, url: &Url) -> Result<usize> {
        let origin = url.origin().ascii_serialization();
        self.remove_entries_matching(|info| {
            metadata_url_origin_matches(&info.metadata.request_url, &origin)
                || metadata_url_origin_matches(&info.metadata.final_url, &origin)
        })
    }

    /// Stores a complete response body in one step.
    #[cfg(test)]
    pub fn store_body(
        &self,
        key: &str,
        metadata: HttpCacheEntryMetadata,
        body: &[u8],
    ) -> Result<()> {
        let mut writer = self.create_body_writer(key)?;
        writer.write_all(body)?;
        writer.finish(metadata)
    }

    /// Best-effort trim of existing entries to the configured cache size.
    pub fn trim_to_max_bytes(&self) {
        if let Some(max_bytes) = self.max_bytes {
            let no_protected_entry = self.root.join(".no-protected-entry");
            prune_cache_to_max_bytes(&self.root, max_bytes, &no_protected_entry);
        }
    }

    /// Starts a streaming write for an entry body.
    pub fn create_body_writer(&self, key: &str) -> Result<HttpCacheBodyWriter> {
        let entry_dir = self
            .try_entry_dir(key)
            .ok_or_else(|| anyhow!("invalid HTTP cache key `{key}`; keys must be lowercase hex"))?;
        fs::create_dir_all(&entry_dir).with_context(|| {
            anyhow!(
                "failed to create HTTP cache entry directory `{}`",
                entry_dir.display()
            )
        })?;

        let unique = unique_suffix();
        let body_name = format!("body.{unique}.bin");
        let body_path = entry_dir.join(&body_name);
        let file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&body_path)
            .with_context(|| {
                anyhow!(
                    "failed to create HTTP cache body file `{}`",
                    body_path.display()
                )
            })?;

        Ok(HttpCacheBodyWriter {
            entry_dir,
            root: self.root.clone(),
            body_path,
            body_name,
            file: Some(file),
            max_bytes: self.max_bytes,
            body_bytes_written: 0,
            finished: false,
        })
    }

    pub(crate) fn entry_dir(&self, key: &str) -> PathBuf {
        self.root.join(format!("{key}.entry"))
    }

    fn try_entry_dir(&self, key: &str) -> Option<PathBuf> {
        if safe_entry_key(key) {
            Some(self.entry_dir(key))
        } else {
            None
        }
    }

    fn cache_entry_dirs(&self) -> Result<Vec<PathBuf>> {
        let entries = match fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(error).with_context(|| {
                    anyhow!(
                        "failed to read HTTP cache root directory `{}`",
                        self.root.display()
                    )
                });
            }
        };

        let mut out = Vec::new();
        for entry in entries {
            let entry = entry.with_context(|| {
                anyhow!(
                    "failed to read HTTP cache root entry under `{}`",
                    self.root.display()
                )
            })?;
            if is_cache_entry_directory(&entry)? {
                out.push(entry.path());
            }
        }
        Ok(out)
    }

    fn entry_info_from_dir(&self, entry_dir: &Path) -> Result<Option<HttpCacheEntryInfo>> {
        let Some(key) = cache_key_from_entry_dir(entry_dir) else {
            return Ok(None);
        };
        let Some((_, metadata, body_path)) = self.load_metadata_and_body_path(&key)? else {
            return Ok(None);
        };
        let body_len = match fs::metadata(&body_path) {
            Ok(metadata) if metadata.is_file() => metadata.len(),
            Ok(_) => return Ok(None),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(error).with_context(|| {
                    anyhow!(
                        "failed to inspect HTTP cache body file `{}`",
                        body_path.display()
                    )
                });
            }
        };

        Ok(Some(HttpCacheEntryInfo {
            key,
            metadata,
            body_len,
            entry_size_bytes: directory_size(entry_dir),
        }))
    }
}

fn is_cache_entry_directory(entry: &fs::DirEntry) -> Result<bool> {
    let Some(file_name) = entry.file_name().to_str().map(str::to_owned) else {
        return Ok(false);
    };
    if !file_name.ends_with(".entry") {
        return Ok(false);
    }
    // DirEntry::file_type does not follow symlinks on supported platforms, so
    // a malicious `<name>.entry` symlink cannot make clear() recurse outside
    // the configured cache root.
    entry
        .file_type()
        .map(|file_type| file_type.is_dir())
        .with_context(|| {
            anyhow!(
                "failed to inspect HTTP cache root entry `{}`",
                entry.path().display()
            )
        })
}

fn cache_key_from_entry_dir(entry_dir: &Path) -> Option<String> {
    let file_name = entry_dir.file_name()?.to_str()?;
    let key = file_name.strip_suffix(".entry")?;
    safe_entry_key(key).then(|| key.to_owned())
}

pub(super) fn directory_size(path: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(path) else {
        return 0;
    };
    entries
        .flatten()
        .filter_map(|entry| entry.metadata().ok())
        .filter(|metadata| metadata.is_file())
        .map(|metadata| metadata.len())
        .sum()
}

fn metadata_url_origin_matches(raw_url: &str, origin: &str) -> bool {
    Url::parse(raw_url)
        .ok()
        .is_some_and(|url| url.origin().ascii_serialization() == origin)
}
