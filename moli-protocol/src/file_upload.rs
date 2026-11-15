//! Browser-admitted file snapshots. Selection reads metadata; byte-consuming
//! Blob operations read the captured path on its original blocking executor.
use std::{
    fs,
    io::{self, Read, Seek, SeekFrom},
    ops::Range,
    path::{Component, Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use moli_core::page::SelectedFile;
use moli_file_api::{BlobData, BlobReadCompletion, BlobSource};

#[derive(Debug, Clone)]
struct FileSource {
    path: PathBuf,
    directory_root: Option<Arc<Path>>,
    size: u64,
    modified: Option<SystemTime>,
    runtime: tokio::runtime::Handle,
}

impl FileSource {
    fn read_range(&self, range: Range<usize>) -> io::Result<Vec<u8>> {
        let metadata = if let Some(root) = &self.directory_root {
            directory_upload_metadata(root, &self.path)?
        } else {
            fs::metadata(&self.path)?
        };
        // A FIFO open can wait for a writer before fd-based checks can run.
        self.check_snapshot(&metadata)?;
        let mut file = fs::File::open(&self.path)?;
        self.check_snapshot(&file.metadata()?)?;
        file.seek(SeekFrom::Start(range.start as u64))?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(range.len())
            .map_err(io::Error::other)?;
        bytes.resize(range.len(), 0);
        file.read_exact(&mut bytes)?;
        self.check_snapshot(&file.metadata()?)?;
        Ok(bytes)
    }

    fn check_snapshot(&self, metadata: &fs::Metadata) -> io::Result<()> {
        if !metadata.is_file()
            || metadata.len() != self.size
            || metadata.modified().ok() != self.modified
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Selected file changed since selection",
            ));
        }
        Ok(())
    }
}

// A queued blocking task may be discarded during executor shutdown. The Blob
// consumer must still get one completion instead of retaining a pending read.
struct ReadReply(Option<BlobReadCompletion>);
impl ReadReply {
    fn complete(mut self, result: io::Result<Vec<u8>>) {
        self.0.take().expect("one file read completion")(result);
    }
}
impl Drop for ReadReply {
    fn drop(&mut self) {
        if let Some(complete) = self.0.take() {
            complete(Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "File reading backend stopped",
            )));
        }
    }
}

impl BlobSource for FileSource {
    fn same_snapshot(&self, other: &dyn BlobSource) -> bool {
        let other: &dyn std::any::Any = other;
        other.downcast_ref::<Self>().is_some_and(|other| {
            self.path == other.path
                && self.directory_root == other.directory_root
                && self.size == other.size
                && self.modified == other.modified
        })
    }

    fn read(&self, range: Range<usize>, complete: BlobReadCompletion) {
        let source = self.clone();
        let reply = ReadReply(Some(complete));
        drop(
            self.runtime
                .spawn_blocking(move || reply.complete(source.read_range(range))),
        );
    }
}

/// Resolve selected paths without opening or copying file contents. A single
/// directory is enumerated in stable order; links below it are not traversed.
/// An explicitly selected directory link retains its chosen display name.
pub fn selected_files_from_paths(paths: &[String]) -> io::Result<Vec<SelectedFile>> {
    select_files(paths, None)
}

struct SelectionCancellation(Arc<std::sync::atomic::AtomicBool>);
impl Drop for SelectionCancellation {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Browser filesystem admission runs outside protocol and renderer turns. A
/// dropped caller stops the enumeration at its next filesystem entry boundary.
pub async fn selected_files_from_paths_async(paths: Vec<String>) -> io::Result<Vec<SelectedFile>> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let _cancel = SelectionCancellation(cancelled.clone());
    tokio::task::spawn_blocking(move || select_files(&paths, Some(&cancelled)))
        .await
        .map_err(io::Error::other)?
}

fn check_selection_cancelled(cancelled: Option<&std::sync::atomic::AtomicBool>) -> io::Result<()> {
    if cancelled.is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Relaxed)) {
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "File selection cancelled",
        ))
    } else {
        Ok(())
    }
}

fn select_files(
    paths: &[String],
    cancelled: Option<&std::sync::atomic::AtomicBool>,
) -> io::Result<Vec<SelectedFile>> {
    check_selection_cancelled(cancelled)?;
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    let runtime = tokio::runtime::Handle::try_current().map_err(|error| {
        io::Error::other(format!(
            "File selection requires its Browser executor: {error}"
        ))
    })?;
    if paths.len() == 1 && Path::new(&paths[0]).is_dir() {
        let selected_root = upload_directory_root(Path::new(&paths[0]))?;
        // Resolve the explicitly granted root once. Its chosen name still
        // appears in webkitRelativePath, but retargeting that alias must not
        // redirect existing File backings to another directory.
        let root: Arc<Path> = fs::canonicalize(&selected_root)?.into();
        let mut directories = vec![root.to_path_buf()];
        let mut files = Vec::new();
        while let Some(directory) = directories.pop() {
            for entry in read_upload_directory(&root, &directory)? {
                check_selection_cancelled(cancelled)?;
                let entry = entry?;
                let kind = entry.file_type()?;
                if kind.is_dir() {
                    directories.push(entry.path());
                } else if kind.is_file() {
                    files.push(entry.path());
                }
            }
        }
        files.sort();
        return files
            .into_iter()
            .map(|path| {
                check_selection_cancelled(cancelled)?;
                let mut file = selected_file_from_file_path(&path, &runtime, Some(root.clone()))?;
                let relative = path
                    .strip_prefix(&root)
                    .expect("enumerated files stay below the resolved directory");
                file.relative_path = upload_relative_path(selected_root.file_name(), relative);
                Ok(file)
            })
            .collect();
    }
    paths
        .iter()
        .map(|path| {
            check_selection_cancelled(cancelled)?;
            selected_file_from_file_path(Path::new(path), &runtime, None)
        })
        .collect()
}

fn selected_file_from_file_path(
    file_path: &Path,
    runtime: &tokio::runtime::Handle,
    directory_root: Option<Arc<Path>>,
) -> io::Result<SelectedFile> {
    let path = file_path.to_string_lossy();
    let metadata = if let Some(root) = &directory_root {
        directory_upload_metadata(root, file_path)?
    } else {
        fs::metadata(file_path).map_err(|_| {
            io::Error::new(io::ErrorKind::NotFound, format!("File not found : {path}"))
        })?
    };
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("File not found : {path}"),
        ));
    }
    let length = usize::try_from(metadata.len())
        .map_err(|_| io::Error::other("Selected file exceeds the supported Blob size"))?;
    let modified = metadata.modified().ok();
    let source = FileSource {
        path: std::path::absolute(file_path)?,
        directory_root,
        size: metadata.len(),
        modified,
        runtime: runtime.clone(),
    };
    let name = file_path
        .file_name()
        .map(|name| name.to_string_lossy())
        .filter(|name| !name.is_empty())
        .unwrap_or(path)
        .into_owned();
    let last_modified = modified
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .or_else(|| SystemTime::now().duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0);
    Ok(SelectedFile {
        data: BlobData::from_source(Arc::new(source), length),
        mime_type: moli_web_mime::selected_file_mime_type(&name).to_owned(),
        name,
        relative_path: String::new(),
        last_modified,
    })
}

fn upload_directory_root(root: &Path) -> io::Result<PathBuf> {
    if root.file_name().is_some() {
        std::path::absolute(root)
    } else {
        fs::canonicalize(root)
    }
}

// These pathname checks narrow the enumeration/open race but are not an
// atomic restricted open. The resolved root and every component are checked
// again before lazy reads; device/inode identity is not guaranteed here.
fn directory_upload_metadata(root: &Path, path: &Path) -> io::Result<fs::Metadata> {
    let rejected = || {
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Directory upload path contains a symbolic link or leaves its selected root",
        )
    };
    let relative = path.strip_prefix(root).map_err(|_| rejected())?;
    if !root.is_absolute()
        || relative
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(rejected());
    }
    let mut checked = PathBuf::new();
    let mut metadata = None;
    // canonicalize() removed authorized root aliases at selection. Checking
    // the entire resolved pathname also catches replacements above that root.
    for part in path.components() {
        checked.push(part.as_os_str());
        match part {
            Component::Prefix(_) => continue,
            Component::RootDir | Component::Normal(_) => {}
            _ => return Err(rejected()),
        }
        let current = fs::symlink_metadata(&checked)?;
        if current.file_type().is_symlink() {
            return Err(rejected());
        }
        metadata = Some(current);
    }
    metadata.ok_or_else(rejected)
}

fn read_upload_directory(root: &Path, path: &Path) -> io::Result<fs::ReadDir> {
    if !directory_upload_metadata(root, path)?.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotADirectory,
            "Selected upload directory changed",
        ));
    }
    fs::read_dir(path)
}

fn upload_relative_path(root_name: Option<&std::ffi::OsStr>, relative: &Path) -> String {
    root_name
        .into_iter()
        .chain(relative.components().map(|component| component.as_os_str()))
        .map(|component| component.to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests;
