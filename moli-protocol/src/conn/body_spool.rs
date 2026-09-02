use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};

use anyhow::{Context, Result, anyhow};

const DEFAULT_MEMORY_LIMIT: usize = 1024 * 1024;
pub(crate) const DEFAULT_BODY_MATERIALIZE_LIMIT: usize = 64 * 1024 * 1024;

static NEXT_SPOOL_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone)]
pub struct CapturedBody {
    inner: Arc<CapturedBodyInner>,
}

#[derive(Debug)]
enum CapturedBodyInner {
    Memory(Vec<u8>),
    SharedMemory(Arc<[u8]>),
    File { path: PathBuf, len: usize },
    Subresource(moli_core::page::SubresourceResponseBody),
}

impl Drop for CapturedBodyInner {
    fn drop(&mut self) {
        if let Self::File { path, .. } = self {
            let _ = fs::remove_file(path);
        }
    }
}

impl CapturedBody {
    pub(crate) fn from_string(body: String) -> Self {
        Self::from_bytes(body.into_bytes())
    }

    pub(crate) fn from_bytes(body: Vec<u8>) -> Self {
        Self {
            inner: Arc::new(CapturedBodyInner::Memory(body)),
        }
    }

    pub(crate) fn from_shared_bytes(body: Arc<[u8]>) -> Self {
        Self {
            inner: Arc::new(CapturedBodyInner::SharedMemory(body)),
        }
    }

    pub(crate) fn from_bytes_spooled(body: Vec<u8>) -> Self {
        if body.len() <= DEFAULT_MEMORY_LIMIT {
            return Self::from_bytes(body);
        }
        let mut writer = CapturedBodyWriter::default();
        if writer.append(&body).is_ok()
            && let Ok(captured) = writer.finish()
        {
            return captured;
        }
        Self::from_bytes(body)
    }

    pub(crate) fn from_optional_renderer_synthetic_response_body(
        body: Option<moli_core::page::RendererSyntheticResponseBody>,
    ) -> Self {
        Self::from_bytes(body.map(|body| body.into_body_bytes()).unwrap_or_default())
    }

    pub(crate) fn from_subresource_response_body(
        body: &moli_core::page::SubresourceResponseBody,
    ) -> Self {
        Self {
            inner: Arc::new(CapturedBodyInner::Subresource(body.clone())),
        }
    }

    pub(crate) fn len(&self) -> usize {
        match self.inner.as_ref() {
            CapturedBodyInner::Memory(bytes) => bytes.len(),
            CapturedBodyInner::SharedMemory(bytes) => bytes.len(),
            CapturedBodyInner::File { len, .. } => *len,
            CapturedBodyInner::Subresource(body) => body.len(),
        }
    }

    pub(crate) fn materialize_bytes_limited(&self, limit: usize) -> Result<Vec<u8>> {
        ensure_materialize_limit(self.len(), limit)?;
        self.materialize_bytes()
    }

    pub(crate) fn materialize_bytes(&self) -> Result<Vec<u8>> {
        match self.inner.as_ref() {
            CapturedBodyInner::Memory(bytes) => Ok(bytes.clone()),
            CapturedBodyInner::SharedMemory(bytes) => Ok(bytes.to_vec()),
            CapturedBodyInner::File { path, .. } => {
                let mut file = File::open(path).with_context(|| {
                    anyhow!("failed to open captured response body `{}`", path.display())
                })?;
                let mut bytes = Vec::new();
                file.read_to_end(&mut bytes).with_context(|| {
                    anyhow!("failed to read captured response body `{}`", path.display())
                })?;
                Ok(bytes)
            }
            CapturedBodyInner::Subresource(body) => body
                .materialize_bytes()
                .with_context(|| "failed to materialize captured subresource response body"),
        }
    }

    pub(crate) fn read_range(&self, offset: usize, len: usize) -> Result<Vec<u8>> {
        if len == 0 || offset >= self.len() {
            return Ok(Vec::new());
        }
        let len = len.min(self.len().saturating_sub(offset));
        match self.inner.as_ref() {
            CapturedBodyInner::Memory(bytes) => {
                Ok(bytes[offset..offset.saturating_add(len)].to_vec())
            }
            CapturedBodyInner::SharedMemory(bytes) => {
                Ok(bytes[offset..offset.saturating_add(len)].to_vec())
            }
            CapturedBodyInner::File { path, .. } => {
                let mut file = File::open(path).with_context(|| {
                    anyhow!("failed to open captured response body `{}`", path.display())
                })?;
                file.seek(SeekFrom::Start(offset as u64)).with_context(|| {
                    anyhow!("failed to seek captured response body `{}`", path.display())
                })?;
                let mut bytes = vec![0; len];
                let read = file.read(&mut bytes).with_context(|| {
                    anyhow!("failed to read captured response body `{}`", path.display())
                })?;
                bytes.truncate(read);
                Ok(bytes)
            }
            CapturedBodyInner::Subresource(body) => body
                .read_chunk(offset, len)
                .with_context(|| "failed to read captured subresource response body"),
        }
    }

    #[cfg(test)]
    pub(crate) fn materialize_lossy_string(&self) -> Result<String> {
        self.materialize_bytes()
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
    }

    pub(crate) fn chunk_reader(&self, chunk_size: usize) -> Result<CapturedBodyChunkReader> {
        if chunk_size == 0 {
            return Err(anyhow!(
                "captured response body chunk size must be non-zero"
            ));
        }
        match self.inner.as_ref() {
            CapturedBodyInner::Memory(_) | CapturedBodyInner::SharedMemory(_) => {
                Ok(CapturedBodyChunkReader {
                    source: CapturedBodyChunkReaderSource::Memory {
                        body: self.clone(),
                        offset: 0,
                    },
                    chunk_size,
                })
            }
            CapturedBodyInner::File { path, .. } => {
                let file = File::open(path).with_context(|| {
                    anyhow!("failed to open captured response body `{}`", path.display())
                })?;
                Ok(CapturedBodyChunkReader {
                    source: CapturedBodyChunkReaderSource::File {
                        file,
                        path: path.clone(),
                    },
                    chunk_size,
                })
            }
            CapturedBodyInner::Subresource(_) => Ok(CapturedBodyChunkReader {
                source: CapturedBodyChunkReaderSource::Subresource {
                    body: self.clone(),
                    offset: 0,
                },
                chunk_size,
            }),
        }
    }
}

pub(crate) struct CapturedBodyChunkReader {
    source: CapturedBodyChunkReaderSource,
    chunk_size: usize,
}

enum CapturedBodyChunkReaderSource {
    Memory { body: CapturedBody, offset: usize },
    File { file: File, path: PathBuf },
    Subresource { body: CapturedBody, offset: usize },
}

impl CapturedBodyChunkReader {
    pub(crate) fn next_chunk(&mut self) -> Result<Option<Vec<u8>>> {
        match &mut self.source {
            CapturedBodyChunkReaderSource::Memory { body, offset } => {
                let bytes: &[u8] = match body.inner.as_ref() {
                    CapturedBodyInner::Memory(bytes) => bytes,
                    CapturedBodyInner::SharedMemory(bytes) => bytes,
                    _ => return Ok(None),
                };
                if *offset >= bytes.len() {
                    return Ok(None);
                }
                let next_offset = (*offset).saturating_add(self.chunk_size).min(bytes.len());
                let chunk = bytes[*offset..next_offset].to_vec();
                *offset = next_offset;
                Ok(Some(chunk))
            }
            CapturedBodyChunkReaderSource::File { file, path } => {
                let mut buffer = vec![0; self.chunk_size];
                let read = file.read(&mut buffer).with_context(|| {
                    anyhow!("failed to read captured response body `{}`", path.display())
                })?;
                if read == 0 {
                    return Ok(None);
                }
                buffer.truncate(read);
                Ok(Some(buffer))
            }
            CapturedBodyChunkReaderSource::Subresource { body, offset } => {
                let CapturedBodyInner::Subresource(source) = body.inner.as_ref() else {
                    return Ok(None);
                };
                let chunk = source
                    .read_chunk(*offset, self.chunk_size)
                    .with_context(|| "failed to read captured subresource response body")?;
                if chunk.is_empty() {
                    return Ok(None);
                }
                *offset = offset.saturating_add(chunk.len());
                Ok(Some(chunk))
            }
        }
    }
}

pub(crate) fn ensure_materialize_limit(len: usize, limit: usize) -> Result<()> {
    if len > limit {
        Err(anyhow!(
            "response body is {len} bytes, exceeds CDP materialization limit of {limit} bytes"
        ))
    } else {
        Ok(())
    }
}

impl PartialEq for CapturedBody {
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len() && self.materialize_bytes().ok() == other.materialize_bytes().ok()
    }
}

impl Eq for CapturedBody {}

#[derive(Debug)]
pub(crate) struct CapturedBodyWriter {
    memory_limit: usize,
    len: usize,
    memory: Vec<u8>,
    file: Option<File>,
    path: Option<PathBuf>,
}

impl Default for CapturedBodyWriter {
    fn default() -> Self {
        Self::new(DEFAULT_MEMORY_LIMIT)
    }
}

impl CapturedBodyWriter {
    pub(crate) fn new(memory_limit: usize) -> Self {
        Self {
            memory_limit,
            len: 0,
            memory: Vec::new(),
            file: None,
            path: None,
        }
    }

    pub(crate) fn append(&mut self, bytes: &[u8]) -> Result<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        if self.file.is_none() && self.len.saturating_add(bytes.len()) <= self.memory_limit {
            self.memory.extend_from_slice(bytes);
            self.len = self.len.saturating_add(bytes.len());
            return Ok(());
        }
        self.ensure_file()?;
        if let Some(file) = self.file.as_mut() {
            file.write_all(bytes)
                .context("failed to append captured response body spool")?;
        }
        self.len = self.len.saturating_add(bytes.len());
        Ok(())
    }

    pub(crate) fn finish(mut self) -> Result<CapturedBody> {
        self.finish_in_place()
    }

    pub(crate) fn finish_in_place(&mut self) -> Result<CapturedBody> {
        if let Some(file) = self.file.as_mut() {
            file.flush()
                .context("failed to flush captured response body spool")?;
            let _ = self.file.take();
            let path = self
                .path
                .take()
                .expect("captured response body file path should be set");
            Ok(CapturedBody {
                inner: Arc::new(CapturedBodyInner::File {
                    path,
                    len: self.len,
                }),
            })
        } else {
            Ok(CapturedBody::from_bytes(std::mem::take(&mut self.memory)))
        }
    }

    fn ensure_file(&mut self) -> Result<()> {
        if self.file.is_some() {
            return Ok(());
        }
        let path = unique_spool_path()?;
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        configure_secure_spool_file_options(&mut options);
        let file = options.open(&path).with_context(|| {
            anyhow!(
                "failed to create captured response body spool `{}`",
                path.display()
            )
        })?;
        // Store the file/path before seeding bytes so Drop can clean up the
        // already-created temp file if write_all fails.
        self.path = Some(path);
        self.file = Some(file);
        if !self.memory.is_empty() {
            self.file
                .as_mut()
                .expect("captured response body spool file should be set")
                .write_all(&self.memory)
                .context("failed to seed captured response body spool")?;
            self.memory.clear();
        }
        Ok(())
    }
}

impl Drop for CapturedBodyWriter {
    fn drop(&mut self) {
        let _ = self.file.take();
        if let Some(path) = self.path.take() {
            let _ = fs::remove_file(path);
        }
    }
}

impl Write for CapturedBodyWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.append(buf).map_err(io::Error::other)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if let Some(file) = self.file.as_mut() {
            file.flush()
        } else {
            Ok(())
        }
    }
}

fn unique_spool_path() -> Result<PathBuf> {
    let root = std::env::temp_dir().join("moli-cdp-body-spool");
    create_secure_spool_root(&root)?;
    let id = NEXT_SPOOL_ID.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    Ok(root.join(format!("body-{}-{id}-{nanos}.bin", std::process::id())))
}

#[cfg(unix)]
fn create_secure_spool_root(root: &Path) -> Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true).mode(0o700);
    builder.create(root).with_context(|| {
        anyhow!(
            "failed to create captured response body spool root `{}`",
            root.display()
        )
    })?;
    // Existing temp dirs may have been created before this code used a
    // restrictive mode. Tighten them every time before publishing spool files.
    fs::set_permissions(root, fs::Permissions::from_mode(0o700)).with_context(|| {
        anyhow!(
            "failed to restrict captured response body spool root `{}`",
            root.display()
        )
    })?;
    Ok(())
}

#[cfg(not(unix))]
fn create_secure_spool_root(root: &Path) -> Result<()> {
    fs::create_dir_all(root).with_context(|| {
        anyhow!(
            "failed to create captured response body spool root `{}`",
            root.display()
        )
    })
}

#[cfg(unix)]
fn configure_secure_spool_file_options(options: &mut OpenOptions) {
    options.mode(0o600);
}

#[cfg(not(unix))]
fn configure_secure_spool_file_options(_options: &mut OpenOptions) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captured_body_writer_keeps_small_body_in_memory() -> Result<()> {
        let mut writer = CapturedBodyWriter::new(16);
        writer.append(b"hello")?;
        let body = writer.finish()?;

        assert_eq!(body.len(), 5);
        assert_eq!(body.materialize_bytes()?, b"hello");
        Ok(())
    }

    #[test]
    fn captured_body_writer_spills_large_body_to_file() -> Result<()> {
        let mut writer = CapturedBodyWriter::new(4);
        writer.append(b"hello")?;
        writer.append(b" world")?;
        let body = writer.finish()?;

        assert_eq!(body.len(), 11);
        assert_eq!(body.materialize_lossy_string()?, "hello world");
        #[cfg(unix)]
        if let CapturedBodyInner::File { path, .. } = body.inner.as_ref() {
            assert_eq!(fs::metadata(path)?.permissions().mode() & 0o777, 0o600);
            assert_eq!(
                fs::metadata(path.parent().expect("spool path should have parent"))?
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }
        Ok(())
    }

    #[test]
    fn captured_body_from_bytes_spooled_reads_ranges_without_materializing_all() -> Result<()> {
        let body = CapturedBody::from_bytes_spooled(vec![b'a'; DEFAULT_MEMORY_LIMIT + 8]);

        assert_eq!(body.len(), DEFAULT_MEMORY_LIMIT + 8);
        assert_eq!(body.read_range(DEFAULT_MEMORY_LIMIT - 2, 6)?, vec![b'a'; 6]);
        assert!(body.read_range(body.len(), 16)?.is_empty());
        #[cfg(unix)]
        if let CapturedBodyInner::File { path, .. } = body.inner.as_ref() {
            assert_eq!(fs::metadata(path)?.permissions().mode() & 0o777, 0o600);
        }
        Ok(())
    }

    #[test]
    fn captured_body_materialization_limit_rejects_large_body_before_read() -> Result<()> {
        let body = CapturedBody::from_bytes(b"hello".to_vec());
        assert_eq!(body.materialize_bytes_limited(5)?, b"hello");
        let error = body.materialize_bytes_limited(4).unwrap_err().to_string();
        assert_eq!(
            error,
            "response body is 5 bytes, exceeds CDP materialization limit of 4 bytes"
        );
        Ok(())
    }

    #[test]
    fn captured_body_memory_chunk_reader_reads_in_chunks() -> Result<()> {
        let body = CapturedBody::from_bytes(b"hello world".to_vec());

        let mut reader = body.chunk_reader(4)?;
        let mut chunks = Vec::new();
        while let Some(chunk) = reader.next_chunk()? {
            chunks.push(chunk);
        }

        assert_eq!(
            chunks,
            vec![b"hell".to_vec(), b"o wo".to_vec(), b"rld".to_vec()]
        );
        assert_eq!(body.materialize_bytes()?, b"hello world");
        Ok(())
    }

    #[test]
    fn captured_body_shared_memory_reads_without_copying_the_backing() -> Result<()> {
        let backing: Arc<[u8]> = Arc::from(&b"hello world"[..]);
        let body = CapturedBody::from_shared_bytes(backing.clone());

        let CapturedBodyInner::SharedMemory(stored) = body.inner.as_ref() else {
            panic!("shared bytes should retain the shared-memory body kind");
        };
        assert!(Arc::ptr_eq(stored, &backing));
        assert_eq!(body.read_range(6, 5)?, b"world");
        assert_eq!(body.materialize_bytes()?, b"hello world");
        Ok(())
    }

    #[test]
    fn captured_body_file_spool_chunk_reader_reads_in_chunks() -> Result<()> {
        let mut writer = CapturedBodyWriter::new(4);
        writer.append(b"hello world")?;
        let body = writer.finish()?;

        let mut reader = body.chunk_reader(5)?;
        let mut chunks = Vec::new();
        while let Some(chunk) = reader.next_chunk()? {
            chunks.push(chunk);
        }

        assert_eq!(
            chunks,
            vec![b"hello".to_vec(), b" worl".to_vec(), b"d".to_vec()]
        );
        Ok(())
    }

    #[test]
    fn captured_body_from_subresource_response_body_reads_shared_source() -> Result<()> {
        let subresource_body =
            moli_core::page::SubresourceResponseBody::from_bytes(b"hello world".to_vec());
        let body = CapturedBody::from_subresource_response_body(&subresource_body);

        assert!(matches!(
            body.inner.as_ref(),
            CapturedBodyInner::Subresource(_)
        ));
        assert_eq!(body.len(), 11);
        assert_eq!(body.materialize_bytes()?, b"hello world");
        assert_eq!(body.read_range(6, 5)?, b"world");

        let mut reader = body.chunk_reader(5)?;
        let mut chunks = Vec::new();
        while let Some(chunk) = reader.next_chunk()? {
            chunks.push(chunk);
        }

        assert_eq!(
            chunks,
            vec![b"hello".to_vec(), b" worl".to_vec(), b"d".to_vec()]
        );
        Ok(())
    }
}
