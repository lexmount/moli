use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufReader, BufWriter, Write},
    path::Path,
};

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use typed_num::Num;

use crate::{path_safety::safe_body_file_name, time::unique_suffix};

pub(crate) const META_FILE: &str = "meta.json";
pub(crate) type HttpCacheFormatVersion = Num<4>;

/// Request header value captured for a `Vary` response header.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HttpCacheVaryHeader {
    pub name: String,
    pub value: Option<String>,
}

/// Serializable metadata for one completed HTTP cache entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HttpCacheEntryMetadata {
    pub(crate) version: HttpCacheFormatVersion,
    pub request_url: String,
    pub final_url: String,
    pub status: u16,
    pub status_text: Option<String>,
    pub headers: Vec<(String, String)>,
    pub stored_at_unix_ms: u64,
    pub last_used_at_unix_ms: u64,
    pub expires_at_unix_ms: Option<u64>,
    pub vary_headers: Vec<HttpCacheVaryHeader>,
    pub(crate) body_file: String,
}

impl HttpCacheEntryMetadata {
    /// Builds metadata for a cache entry whose body filename is assigned later.
    pub fn new(
        request_url: String,
        final_url: String,
        status: u16,
        headers: Vec<(String, String)>,
        stored_at_unix_ms: u64,
        expires_at_unix_ms: Option<u64>,
        vary_headers: Vec<HttpCacheVaryHeader>,
    ) -> Self {
        Self {
            version: HttpCacheFormatVersion::default(),
            request_url,
            final_url,
            status,
            status_text: None,
            headers,
            stored_at_unix_ms,
            last_used_at_unix_ms: stored_at_unix_ms,
            expires_at_unix_ms,
            vary_headers,
            body_file: String::new(),
        }
    }

    pub(crate) fn with_body_file(mut self, body_file: String) -> Self {
        self.body_file = body_file;
        self
    }
}

pub(crate) fn write_metadata_file(
    entry_dir: &Path,
    metadata: &HttpCacheEntryMetadata,
) -> Result<()> {
    let tmp_meta_path = entry_dir.join(format!("meta.{}.tmp", unique_suffix()));
    let tmp_meta_file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&tmp_meta_path)
        .with_context(|| {
            anyhow!(
                "failed to create HTTP cache metadata temp file `{}`",
                tmp_meta_path.display()
            )
        })?;
    let mut tmp_meta_writer = BufWriter::new(tmp_meta_file);
    // Stream metadata directly to the temp file; even small cache metadata
    // should follow the same no-extra-buffering rule as response bodies.
    serde_json::to_writer(&mut tmp_meta_writer, metadata).with_context(|| {
        anyhow!(
            "failed to write HTTP cache metadata `{}`",
            tmp_meta_path.display()
        )
    })?;
    tmp_meta_writer.flush().with_context(|| {
        anyhow!(
            "failed to flush HTTP cache metadata `{}`",
            tmp_meta_path.display()
        )
    })?;
    // Close the temp file before rename. Unix allows renaming an open file, but
    // Windows usually does not, so keep the publish path portable.
    drop(tmp_meta_writer);

    let meta_path = entry_dir.join(META_FILE);
    fs::rename(&tmp_meta_path, &meta_path).with_context(|| {
        anyhow!(
            "failed to publish HTTP cache metadata `{}`",
            meta_path.display()
        )
    })?;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReadMetadataError {
    Missing,
    Invalid,
    Io,
}

pub(crate) fn read_metadata_file(
    meta_path: &Path,
) -> std::result::Result<HttpCacheEntryMetadata, ReadMetadataError> {
    let file = File::open(meta_path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            ReadMetadataError::Missing
        } else {
            ReadMetadataError::Io
        }
    })?;
    let reader = BufReader::new(file);
    serde_json::from_reader(reader).map_err(|_| ReadMetadataError::Invalid)
}

pub(crate) fn touch_metadata_last_used_if_body_matches(
    entry_dir: &Path,
    expected_body_file: &str,
    last_used_at_unix_ms: u64,
) -> Result<()> {
    let mut metadata =
        read_metadata_file(&entry_dir.join(META_FILE)).map_err(|error| match error {
            ReadMetadataError::Missing => anyhow!("HTTP cache metadata disappeared before touch"),
            ReadMetadataError::Invalid => {
                anyhow!("HTTP cache metadata became invalid before touch")
            }
            ReadMetadataError::Io => anyhow!("failed to read HTTP cache metadata before touch"),
        })?;
    if metadata.body_file != expected_body_file {
        return Ok(());
    }
    metadata.last_used_at_unix_ms = last_used_at_unix_ms;
    write_metadata_file(entry_dir, &metadata)
}

pub(crate) fn read_published_body_file(entry_dir: &Path) -> Option<String> {
    let metadata = read_metadata_file(&entry_dir.join(META_FILE)).ok()?;
    if safe_body_file_name(&metadata.body_file) {
        Some(metadata.body_file)
    } else {
        None
    }
}
