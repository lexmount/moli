use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use crate::{OpfsError, OpfsResult, staging::sync_directory};

pub(crate) const SYNC_DIR_NAME: &str = "sync";

/// One live byte store shared by every compatible sync handle for a file.
///
/// The catalog owns the stable backing ID and observable version. This object
/// owns only the open host file or in-memory bytes, reference count, and the
/// recovery marker required while a disk backing can be modified in place.
#[derive(Debug)]
pub(crate) struct SyncBacking {
    backing_id: u64,
    open_handles: usize,
    writable: bool,
    dirty: bool,
    next_version_id: u64,
    version_id_limit: u64,
    marker_identity: Option<(PathBuf, u64, u64)>,
    marker_path: Option<PathBuf>,
    storage: SyncBackingStorage,
}

#[derive(Debug)]
enum SyncBackingStorage {
    Memory(Vec<u8>),
    Disk { path: PathBuf, file: File },
}

impl SyncBacking {
    pub(crate) fn memory(backing_id: u64, bytes: Vec<u8>, writable: bool) -> Self {
        Self {
            backing_id,
            open_handles: 1,
            writable,
            dirty: false,
            next_version_id: 0,
            version_id_limit: 0,
            marker_identity: None,
            marker_path: None,
            storage: SyncBackingStorage::Memory(bytes),
        }
    }

    pub(crate) fn disk(
        bucket_dir: &Path,
        backing_path: PathBuf,
        entry_id: u64,
        backing_id: u64,
        writable: bool,
    ) -> OpfsResult<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(writable)
            .open(&backing_path)
            .map_err(|source| OpfsError::io("open sync backing file", &backing_path, source))?;
        let marker_identity = if writable {
            Some((bucket_dir.to_owned(), entry_id, backing_id))
        } else {
            None
        };
        Ok(Self {
            backing_id,
            open_handles: 1,
            writable,
            dirty: false,
            next_version_id: 0,
            version_id_limit: 0,
            marker_identity,
            marker_path: None,
            storage: SyncBackingStorage::Disk {
                path: backing_path,
                file,
            },
        })
    }

    pub(crate) const fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub(crate) fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    pub(crate) fn prepare_mutation(&mut self) -> OpfsResult<()> {
        if !self.writable {
            return Err(OpfsError::InvalidState);
        }
        if self.marker_path.is_none()
            && let Some((bucket_dir, entry_id, backing_id)) = &self.marker_identity
        {
            self.marker_path = Some(create_sync_marker(bucket_dir, *entry_id, *backing_id)?);
        }
        Ok(())
    }

    pub(crate) fn complete_checkpoint(&mut self) -> OpfsResult<()> {
        self.remove_marker()?;
        self.dirty = false;
        Ok(())
    }

    pub(crate) fn take_reserved_version_id(&mut self) -> Option<u64> {
        if self.next_version_id >= self.version_id_limit {
            return None;
        }
        let version_id = self.next_version_id;
        self.next_version_id += 1;
        Some(version_id)
    }

    pub(crate) fn install_version_reservation(&mut self, start: u64, end: u64) -> OpfsResult<()> {
        if !self.writable
            || start == 0
            || start >= end
            || self.next_version_id < self.version_id_limit
        {
            return Err(OpfsError::InvalidState);
        }
        self.next_version_id = start;
        self.version_id_limit = end;
        Ok(())
    }

    pub(crate) fn add_handle(&mut self, writable: bool) -> OpfsResult<()> {
        if self.writable != writable {
            return Err(OpfsError::InvalidState);
        }
        self.open_handles = self
            .open_handles
            .checked_add(1)
            .ok_or(OpfsError::InvalidState)?;
        Ok(())
    }

    /// Release one handle and report whether the backing has no remaining users.
    pub(crate) fn release_handle(&mut self) -> OpfsResult<bool> {
        self.open_handles = self
            .open_handles
            .checked_sub(1)
            .ok_or(OpfsError::InvalidState)?;
        Ok(self.open_handles == 0)
    }

    pub(crate) fn len(&self) -> OpfsResult<u64> {
        match &self.storage {
            SyncBackingStorage::Memory(bytes) => Ok(u64::try_from(bytes.len()).unwrap_or(u64::MAX)),
            SyncBackingStorage::Disk { path, file } => file
                .metadata()
                .map(|metadata| metadata.len())
                .map_err(|source| OpfsError::io("stat sync backing file", path, source)),
        }
    }

    pub(crate) fn read(&mut self, offset: u64, length: usize) -> OpfsResult<Vec<u8>> {
        match &mut self.storage {
            SyncBackingStorage::Memory(bytes) => {
                let start = usize::try_from(offset).map_err(|_| {
                    OpfsError::InvalidModification("sync read offset exceeds usize".to_owned())
                })?;
                if start >= bytes.len() {
                    return Ok(Vec::new());
                }
                let end = start.saturating_add(length).min(bytes.len());
                Ok(bytes[start..end].to_vec())
            }
            SyncBackingStorage::Disk { path, file } => {
                let file_len = file
                    .metadata()
                    .map_err(|source| OpfsError::io("stat sync backing file", &*path, source))?
                    .len();
                if offset >= file_len {
                    return Ok(Vec::new());
                }
                let requested = u64::try_from(length).unwrap_or(u64::MAX);
                let read_len =
                    usize::try_from((file_len - offset).min(requested)).map_err(|_| {
                        OpfsError::InvalidModification("sync read length exceeds usize".to_owned())
                    })?;
                let mut bytes = vec![0; read_len];
                file.seek(SeekFrom::Start(offset)).map_err(|source| {
                    OpfsError::io("seek sync backing file for read", &*path, source)
                })?;
                file.read_exact(&mut bytes)
                    .map_err(|source| OpfsError::io("read sync backing file", &*path, source))?;
                Ok(bytes)
            }
        }
    }

    pub(crate) fn read_all(&mut self) -> OpfsResult<Vec<u8>> {
        let len = self.len()?;
        let len = usize::try_from(len).map_err(|_| {
            OpfsError::InvalidModification("sync backing size exceeds usize".to_owned())
        })?;
        self.read(0, len)
    }

    pub(crate) fn write(&mut self, offset: u64, data: &[u8]) -> OpfsResult<usize> {
        if !self.writable {
            return Err(OpfsError::InvalidState);
        }
        match &mut self.storage {
            SyncBackingStorage::Memory(bytes) => {
                let start = usize::try_from(offset).map_err(|_| {
                    OpfsError::InvalidModification("sync write offset exceeds usize".to_owned())
                })?;
                let end = start.checked_add(data.len()).ok_or_else(|| {
                    OpfsError::InvalidModification("sync write length overflow".to_owned())
                })?;
                if !data.is_empty() && bytes.len() < end {
                    bytes.resize(end, 0);
                }
                if !data.is_empty() {
                    bytes[start..end].copy_from_slice(data);
                }
                Ok(data.len())
            }
            SyncBackingStorage::Disk { path, file } => {
                file.seek(SeekFrom::Start(offset)).map_err(|source| {
                    OpfsError::io("seek sync backing file for write", &*path, source)
                })?;
                write_all_sync_backing_file(file, path, data)
            }
        }
    }

    pub(crate) fn truncate(&mut self, size: u64) -> OpfsResult<()> {
        if !self.writable {
            return Err(OpfsError::InvalidState);
        }
        match &mut self.storage {
            SyncBackingStorage::Memory(bytes) => {
                let size = usize::try_from(size).map_err(|_| {
                    OpfsError::InvalidModification("sync truncate size exceeds usize".to_owned())
                })?;
                bytes.resize(size, 0);
                Ok(())
            }
            SyncBackingStorage::Disk { path, file } => file
                .set_len(size)
                .map_err(|source| OpfsError::io("truncate sync backing file", &*path, source)),
        }
    }

    pub(crate) fn flush(&mut self) -> OpfsResult<()> {
        match &mut self.storage {
            SyncBackingStorage::Memory(_) => Ok(()),
            SyncBackingStorage::Disk { path, file } => file
                .sync_all()
                .map_err(|source| OpfsError::io("flush sync backing file", &*path, source)),
        }
    }

    /// Close the backing, remove its recovery marker, and return memory bytes.
    pub(crate) fn finish(mut self) -> (Option<(u64, Vec<u8>)>, OpfsResult<()>) {
        let memory =
            match std::mem::replace(&mut self.storage, SyncBackingStorage::Memory(Vec::new())) {
                SyncBackingStorage::Memory(bytes) => Some((self.backing_id, bytes)),
                SyncBackingStorage::Disk { .. } => None,
            };
        let cleanup = self.remove_marker();
        (memory, cleanup)
    }

    fn remove_marker(&mut self) -> OpfsResult<()> {
        let Some(path) = self.marker_path.take() else {
            return Ok(());
        };
        remove_file_if_exists(&path, "remove sync recovery marker")?;
        if let Some(parent) = path.parent() {
            sync_directory(parent)?;
        }
        Ok(())
    }
}

fn write_all_sync_backing_file(
    writer: &mut impl Write,
    path: &Path,
    data: &[u8],
) -> OpfsResult<usize> {
    writer
        .write_all(data)
        .map_err(|source| OpfsError::io("write sync backing file", path, source))?;
    Ok(data.len())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SyncRecoveryMarker {
    pub entry_id: u64,
    pub backing_id: u64,
}

pub(crate) fn prepare_sync_directory(bucket_dir: &Path) -> OpfsResult<PathBuf> {
    let directory = bucket_dir.join(SYNC_DIR_NAME);
    fs::create_dir_all(&directory)
        .map_err(|source| OpfsError::io("create sync marker directory", &directory, source))?;
    Ok(directory)
}

pub(crate) fn read_sync_recovery_markers(bucket_dir: &Path) -> OpfsResult<Vec<SyncRecoveryMarker>> {
    let directory = prepare_sync_directory(bucket_dir)?;
    let mut markers = Vec::new();
    for entry in fs::read_dir(&directory)
        .map_err(|source| OpfsError::io("scan sync marker directory", &directory, source))?
    {
        let entry = entry.map_err(|source| {
            OpfsError::io("read sync marker directory entry", &directory, source)
        })?;
        let file_type = entry
            .file_type()
            .map_err(|source| OpfsError::io("inspect sync marker", entry.path(), source))?;
        if !file_type.is_file() {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if let Some(marker) = parse_marker_name(&name) {
            markers.push(marker);
        }
    }
    markers.sort_by_key(|marker| (marker.entry_id, marker.backing_id));
    markers.dedup();
    Ok(markers)
}

pub(crate) fn cleanup_sync_directory(bucket_dir: &Path) -> OpfsResult<()> {
    let directory = prepare_sync_directory(bucket_dir)?;
    let mut changed = false;
    for entry in fs::read_dir(&directory)
        .map_err(|source| OpfsError::io("scan sync marker directory", &directory, source))?
    {
        let entry = entry.map_err(|source| {
            OpfsError::io("read sync marker directory entry", &directory, source)
        })?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|source| OpfsError::io("inspect sync marker", &path, source))?;
        if file_type.is_dir() {
            fs::remove_dir_all(&path).map_err(|source| {
                OpfsError::io("remove orphan sync marker directory", &path, source)
            })?;
        } else {
            fs::remove_file(&path)
                .map_err(|source| OpfsError::io("remove orphan sync marker", &path, source))?;
        }
        changed = true;
    }
    if changed {
        sync_directory(&directory)?;
    }
    Ok(())
}

fn create_sync_marker(bucket_dir: &Path, entry_id: u64, backing_id: u64) -> OpfsResult<PathBuf> {
    let directory = prepare_sync_directory(bucket_dir)?;
    let path = directory.join(marker_name(entry_id, backing_id));
    let file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)
        .map_err(|source| OpfsError::io("create sync recovery marker", &path, source))?;
    file.sync_all()
        .map_err(|source| OpfsError::io("sync recovery marker", &path, source))?;
    sync_directory(&directory)?;
    Ok(path)
}

fn marker_name(entry_id: u64, backing_id: u64) -> String {
    format!("entry-{entry_id:016x}-backing-{backing_id:016x}.active")
}

fn parse_marker_name(name: &str) -> Option<SyncRecoveryMarker> {
    let name = name.strip_prefix("entry-")?.strip_suffix(".active")?;
    let (entry_id, backing_id) = name.split_once("-backing-")?;
    if entry_id.len() != 16 || backing_id.len() != 16 {
        return None;
    }
    Some(SyncRecoveryMarker {
        entry_id: u64::from_str_radix(entry_id, 16).ok()?,
        backing_id: u64::from_str_radix(backing_id, 16).ok()?,
    })
}

use crate::store::remove_file_if_exists;

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct ShortWriter {
        bytes: Vec<u8>,
        calls: usize,
    }

    impl Write for ShortWriter {
        fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
            self.calls += 1;
            let written = data.len().min(2);
            self.bytes.extend_from_slice(&data[..written]);
            Ok(written)
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn sync_backing_write_retries_short_writes_until_all_bytes_are_written() {
        let mut writer = ShortWriter::default();
        let data = b"complete sync access handle write";

        let written =
            write_all_sync_backing_file(&mut writer, Path::new("synthetic-sync-backing"), data)
                .expect("short writes should be retried");

        assert_eq!(written, data.len());
        assert_eq!(writer.bytes, data);
        assert!(writer.calls > 1, "fixture must exercise a short write");
    }
}
