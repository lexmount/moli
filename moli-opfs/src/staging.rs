use std::{
    fs::{self, File, OpenOptions},
    io::{Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use crate::{OpfsError, OpfsResult, WritableCommand};

pub(crate) const STAGING_DIR_NAME: &str = "staging";

/// Per-writer payload which is never visible through the committed namespace.
///
/// Memory-backed buckets keep their staging bytes in memory. Profile-backed
/// buckets keep an opaque file beside the content store so close can promote it
/// without loading the whole payload into the renderer process heap.
#[derive(Debug)]
pub(crate) enum WritableStaging {
    Memory(Vec<u8>),
    Disk(DiskWritableStaging),
}

impl WritableStaging {
    pub(crate) fn memory(bytes: Vec<u8>) -> Self {
        Self::Memory(bytes)
    }

    pub(crate) fn disk(
        bucket_dir: &Path,
        owner_id: u64,
        source: Option<&Path>,
    ) -> OpfsResult<Self> {
        let path = staging_path(bucket_dir, owner_id);
        DiskWritableStaging::create(path, source).map(Self::Disk)
    }

    pub(crate) fn len(&self) -> usize {
        match self {
            Self::Memory(bytes) => bytes.len(),
            Self::Disk(staging) => staging.len,
        }
    }

    pub(crate) fn projected_length(
        &self,
        cursor: u64,
        command: &WritableCommand,
    ) -> OpfsResult<usize> {
        match command {
            WritableCommand::Write { data, position } => {
                let position = position.unwrap_or(cursor);
                let start = usize::try_from(position).map_err(|_| {
                    OpfsError::InvalidModification("writer position exceeds usize".to_owned())
                })?;
                Ok(self.len().max(start.checked_add(data.len()).ok_or_else(|| {
                    OpfsError::InvalidModification("writer length overflow".to_owned())
                })?))
            }
            WritableCommand::Seek(_) => Ok(self.len()),
            WritableCommand::Truncate(size) => usize::try_from(*size).map_err(|_| {
                OpfsError::InvalidModification("writer truncate size exceeds usize".to_owned())
            }),
        }
    }

    pub(crate) fn apply(&mut self, cursor: &mut u64, command: WritableCommand) -> OpfsResult<()> {
        match self {
            Self::Memory(bytes) => apply_memory_command(bytes, cursor, command),
            Self::Disk(staging) => staging.apply(cursor, command),
        }
    }

    pub(crate) fn discard(self) -> OpfsResult<()> {
        match self {
            Self::Memory(_) => Ok(()),
            Self::Disk(staging) => staging.discard(),
        }
    }
}

#[derive(Debug)]
pub(crate) struct DiskWritableStaging {
    path: Option<PathBuf>,
    file: Option<File>,
    len: usize,
}

impl DiskWritableStaging {
    fn create(path: PathBuf, source: Option<&Path>) -> OpfsResult<Self> {
        let file = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|source| OpfsError::io("create writable staging file", &path, source))?;
        let mut staging = Self {
            path: Some(path),
            file: Some(file),
            len: 0,
        };
        if let Some(source_path) = source {
            let mut source_file = File::open(source_path).map_err(|source| {
                OpfsError::io("open committed content for staging", source_path, source)
            })?;
            let staging_path = staging.path_for_error().to_path_buf();
            let copied =
                std::io::copy(&mut source_file, staging.file_mut()?).map_err(|source| {
                    OpfsError::io("copy committed content into staging", &staging_path, source)
                })?;
            staging.len = usize::try_from(copied).map_err(|_| {
                OpfsError::InvalidModification("staging file size exceeds usize".to_owned())
            })?;
        }
        Ok(staging)
    }

    fn apply(&mut self, cursor: &mut u64, command: WritableCommand) -> OpfsResult<()> {
        match command {
            WritableCommand::Write { data, position } => {
                let position = position.unwrap_or(*cursor);
                let start = usize::try_from(position).map_err(|_| {
                    OpfsError::InvalidModification("writer position exceeds usize".to_owned())
                })?;
                let end = start.checked_add(data.len()).ok_or_else(|| {
                    OpfsError::InvalidModification("writer length overflow".to_owned())
                })?;
                let path = self.path_for_error().to_path_buf();
                let file = self.file_mut()?;
                file.seek(SeekFrom::Start(position))
                    .map_err(|source| OpfsError::io("seek writable staging file", &path, source))?;
                file.write_all(&data).map_err(|source| {
                    OpfsError::io("write writable staging file", &path, source)
                })?;
                self.len = self.len.max(end);
                *cursor = u64::try_from(end).unwrap_or(u64::MAX);
            }
            WritableCommand::Seek(position) => *cursor = position,
            WritableCommand::Truncate(size) => {
                let size = usize::try_from(size).map_err(|_| {
                    OpfsError::InvalidModification("writer truncate size exceeds usize".to_owned())
                })?;
                let path = self.path_for_error().to_path_buf();
                self.file_mut()?
                    .set_len(u64::try_from(size).unwrap_or(u64::MAX))
                    .map_err(|source| {
                        OpfsError::io("truncate writable staging file", &path, source)
                    })?;
                self.len = size;
                *cursor = (*cursor).min(size as u64);
            }
        }
        Ok(())
    }

    pub(crate) fn promote(mut self, destination: &Path) -> OpfsResult<()> {
        let source = self.path_for_error().to_path_buf();
        self.file_mut()?
            .sync_all()
            .map_err(|error| OpfsError::io("sync writable staging file", &source, error))?;
        #[cfg(test)]
        crate::fault_injection::crash_if_armed(
            crate::fault_injection::CrashPoint::WritableStagingSynced,
            || {
                // A real process crash does not run `Drop`. Disarm the
                // staging destructor so reopen has to collect this file.
                self.file.take();
                self.path.take();
            },
        );
        self.file.take();
        if destination
            .try_exists()
            .map_err(|error| OpfsError::io("inspect content destination", destination, error))?
        {
            return Err(OpfsError::CorruptCatalog(format!(
                "content destination `{}` already exists",
                destination.display()
            )));
        }
        fs::rename(&source, destination)
            .map_err(|error| OpfsError::io("promote writable staging file", &source, error))?;
        self.path.take();
        #[cfg(test)]
        crate::fault_injection::crash_if_armed(
            crate::fault_injection::CrashPoint::WritableStagingPromoted,
            || {},
        );
        if let Some(parent) = destination.parent() {
            sync_directory(parent)?;
        }
        #[cfg(test)]
        crate::fault_injection::crash_if_armed(
            crate::fault_injection::CrashPoint::WritableContentDurable,
            || {},
        );
        Ok(())
    }

    fn discard(mut self) -> OpfsResult<()> {
        self.file.take();
        let Some(path) = self.path.take() else {
            return Ok(());
        };
        remove_file_if_exists(&path, "remove writable staging file")?;
        if let Some(parent) = path.parent() {
            sync_directory(parent)?;
        }
        Ok(())
    }

    fn file_mut(&mut self) -> OpfsResult<&mut File> {
        let path = self.path_for_error().to_path_buf();
        self.file
            .as_mut()
            .ok_or_else(|| OpfsError::io("access closed writable staging file", path, closed_io()))
    }

    fn path_for_error(&self) -> &Path {
        self.path
            .as_deref()
            .unwrap_or_else(|| Path::new("<promoted>"))
    }
}

impl Drop for DiskWritableStaging {
    fn drop(&mut self) {
        self.file.take();
        if let Some(path) = self.path.take() {
            let _ = fs::remove_file(path);
        }
    }
}

fn apply_memory_command(
    bytes: &mut Vec<u8>,
    cursor: &mut u64,
    command: WritableCommand,
) -> OpfsResult<()> {
    match command {
        WritableCommand::Write { data, position } => {
            let position = position.unwrap_or(*cursor);
            let start = usize::try_from(position).map_err(|_| {
                OpfsError::InvalidModification("writer position exceeds usize".to_owned())
            })?;
            let end = start.checked_add(data.len()).ok_or_else(|| {
                OpfsError::InvalidModification("writer length overflow".to_owned())
            })?;
            if bytes.len() < end {
                bytes.resize(end, 0);
            }
            bytes[start..end].copy_from_slice(&data);
            *cursor = u64::try_from(end).unwrap_or(u64::MAX);
        }
        WritableCommand::Seek(position) => *cursor = position,
        WritableCommand::Truncate(size) => {
            let size = usize::try_from(size).map_err(|_| {
                OpfsError::InvalidModification("writer truncate size exceeds usize".to_owned())
            })?;
            bytes.resize(size, 0);
            *cursor = (*cursor).min(size as u64);
        }
    }
    Ok(())
}

pub(crate) fn prepare_staging_directory(bucket_dir: &Path) -> OpfsResult<PathBuf> {
    let directory = bucket_dir.join(STAGING_DIR_NAME);
    fs::create_dir_all(&directory)
        .map_err(|source| OpfsError::io("create writable staging directory", &directory, source))?;
    Ok(directory)
}

pub(crate) fn recover_staging_root(root: &Path) -> OpfsResult<()> {
    for entry in fs::read_dir(root)
        .map_err(|source| OpfsError::io("scan OPFS root for staging files", root, source))?
    {
        let entry = entry.map_err(|source| OpfsError::io("read OPFS root entry", root, source))?;
        let file_type = entry
            .file_type()
            .map_err(|source| OpfsError::io("inspect OPFS root entry", entry.path(), source))?;
        if file_type.is_dir() {
            cleanup_staging_directory(&entry.path())?;
        }
    }
    Ok(())
}

pub(crate) fn cleanup_staging_directory(bucket_dir: &Path) -> OpfsResult<()> {
    let directory = bucket_dir.join(STAGING_DIR_NAME);
    if !directory
        .try_exists()
        .map_err(|source| OpfsError::io("inspect writable staging directory", &directory, source))?
    {
        return Ok(());
    }
    let mut changed = false;
    for entry in fs::read_dir(&directory)
        .map_err(|source| OpfsError::io("scan writable staging directory", &directory, source))?
    {
        let entry = entry.map_err(|source| {
            OpfsError::io("read writable staging directory entry", &directory, source)
        })?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|source| OpfsError::io("inspect writable staging entry", &path, source))?;
        if file_type.is_dir() {
            fs::remove_dir_all(&path).map_err(|source| {
                OpfsError::io("remove orphan writable staging directory", &path, source)
            })?;
        } else {
            fs::remove_file(&path).map_err(|source| {
                OpfsError::io("remove orphan writable staging file", &path, source)
            })?;
        }
        changed = true;
    }
    if changed {
        sync_directory(&directory)?;
    }
    Ok(())
}

pub(crate) fn sync_directory(path: &Path) -> OpfsResult<()> {
    sync_directory_handle(path)
        .map_err(|source| OpfsError::io("sync backend directory", path, source))
}

#[cfg(not(windows))]
fn sync_directory_handle(path: &Path) -> std::io::Result<()> {
    File::open(path)?.sync_all()
}

#[cfg(windows)]
fn sync_directory_handle(path: &Path) -> std::io::Result<()> {
    use std::os::windows::fs::OpenOptionsExt;

    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const GENERIC_WRITE: u32 = 0x4000_0000;

    OpenOptions::new()
        .access_mode(GENERIC_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)?
        .sync_all()
}

fn staging_path(bucket_dir: &Path, owner_id: u64) -> PathBuf {
    bucket_dir
        .join(STAGING_DIR_NAME)
        .join(format!("writer-{owner_id:016x}.stage"))
}

fn remove_file_if_exists(path: &Path, operation: &'static str) -> OpfsResult<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(OpfsError::io(operation, path, source)),
    }
}

fn closed_io() -> std::io::Error {
    std::io::Error::other("writable staging file is closed")
}
