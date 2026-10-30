use std::{
    error::Error,
    ffi::OsString,
    fmt, fs,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Writes a profile file via a temporary file without forcing a disk sync.
/// The completed write replaces the target; crash durability is not guaranteed.
pub fn write_profile_file(path: &Path, bytes: &[u8], label: &str) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {label} dir `{}`", parent.display()))?;
    }

    let tmp_path = unique_temp_path(path);
    let result = (|| {
        fs::write(&tmp_path, bytes)
            .with_context(|| format!("failed to write {label} `{}`", tmp_path.display()))?;
        fs::rename(&tmp_path, path).with_context(|| {
            format!(
                "failed to replace {label} `{}` from `{}`",
                path.display(),
                tmp_path.display()
            )
        })
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp_path);
    }
    result
}

/// Failure while publishing metadata that other durable files may reference.
#[derive(Debug)]
pub struct DurableProfileFileError {
    published: bool,
    source: anyhow::Error,
}

impl DurableProfileFileError {
    /// Replacement succeeded, but its directory could not be synchronized.
    /// Callers must reopen rather than assume the previous file is authoritative.
    pub fn publication_uncertain(&self) -> bool {
        self.published
    }
}

impl fmt::Display for DurableProfileFileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:#}", self.source)
    }
}

impl Error for DurableProfileFileError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.source.as_ref())
    }
}

/// Atomically publishes a file after synchronizing its bytes and directories.
/// Use off the renderer thread for identities referenced by another backend.
pub fn write_profile_file_durable(
    path: &Path,
    bytes: &[u8],
    label: &str,
) -> Result<(), DurableProfileFileError> {
    let tmp_path = unique_temp_path(path);
    let mut published = false;
    let result = (|| -> Result<()> {
        let parent = path.parent().filter(|path| !path.as_os_str().is_empty());
        let parent = parent.unwrap_or_else(|| Path::new("."));
        create_durable_profile_directory(parent)
            .with_context(|| format!("failed to create {label} dir `{}`", parent.display()))?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp_path)
            .with_context(|| format!("failed to create {label} `{}`", tmp_path.display()))?;
        file.write_all(bytes)
            .and_then(|()| file.sync_all())
            .with_context(|| format!("failed to sync {label} `{}`", tmp_path.display()))?;
        drop(file);
        fs::rename(&tmp_path, path)
            .with_context(|| format!("failed to replace {label} `{}`", path.display()))?;
        published = true;
        #[cfg(test)]
        if FAIL_DIRECTORY_SYNC.with(|fault| fault.replace(false)) {
            anyhow::bail!("injected directory sync failure after replacement");
        }
        sync_profile_directory(parent).with_context(|| {
            format!(
                "failed to sync published {label} dir `{}`",
                parent.display()
            )
        })
    })();
    if result.is_err() && !published {
        let _ = fs::remove_file(&tmp_path);
    }
    result.map_err(|source| DurableProfileFileError { published, source })
}

pub(crate) fn create_durable_profile_directory(path: &Path) -> std::io::Result<()> {
    if path.is_dir() {
        return Ok(());
    }
    let parent = path.parent().filter(|path| !path.as_os_str().is_empty());
    let parent = parent.unwrap_or_else(|| Path::new("."));
    create_durable_profile_directory(parent)?;
    if let Err(error) = fs::create_dir(path)
        && !(error.kind() == std::io::ErrorKind::AlreadyExists && path.is_dir())
    {
        return Err(error);
    }
    sync_profile_directory(parent)
}

#[cfg(test)]
thread_local! {
    static FAIL_DIRECTORY_SYNC: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(not(windows))]
fn sync_profile_directory(path: &Path) -> std::io::Result<()> {
    fs::File::open(path)?.sync_all()
}

#[cfg(windows)]
fn sync_profile_directory(path: &Path) -> std::io::Result<()> {
    use std::os::windows::fs::OpenOptionsExt;
    fs::OpenOptions::new()
        .access_mode(0x4000_0000) // GENERIC_WRITE
        .custom_flags(0x0200_0000) // FILE_FLAG_BACKUP_SEMANTICS
        .open(path)?
        .sync_all()
}

fn unique_temp_path(path: &Path) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file_name = path
        .file_name()
        .map(OsString::from)
        .unwrap_or_else(|| OsString::from("profile-write"));
    let mut tmp_name = OsString::from(".");
    tmp_name.push(file_name);
    tmp_name.push(format!(".{}.{}.{}.tmp", std::process::id(), nonce, counter));
    path.with_file_name(tmp_name)
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use anyhow::Result;

    use super::{FAIL_DIRECTORY_SYNC, write_profile_file, write_profile_file_durable};

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(name: &str) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock should be after epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "moli-profile-file-{name}-{}-{nonce}",
                std::process::id()
            ));
            Self { path }
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn profile_file_creates_and_overwrites_file_in_new_directory() -> Result<()> {
        let temp = TempDir::new("create-replace");
        let parent = temp.path.join("profile with spaces \u{914d}\u{7f6e}");
        let target = parent.join("profile.json");

        write_profile_file(&target, b"initial profile", "profile test")?;
        assert_eq!(fs::read(&target)?, b"initial profile");

        write_profile_file(&target, b"updated", "profile test")?;
        assert_eq!(fs::read(&target)?, b"updated");
        assert_eq!(fs::read_dir(&parent)?.count(), 1);
        Ok(())
    }

    #[test]
    fn profile_file_uses_unique_temp_name_instead_of_fixed_tmp_path() -> Result<()> {
        let temp = TempDir::new("unique");
        let target = temp.path.join("profile.json");
        let mut fixed_tmp = target.as_os_str().to_owned();
        fixed_tmp.push(".tmp");
        let fixed_tmp = PathBuf::from(fixed_tmp);
        fs::create_dir_all(&temp.path)?;
        fs::write(&target, b"old profile")?;
        fs::write(&fixed_tmp, b"stale fixed tmp")?;

        write_profile_file(&target, b"new profile", "profile test")?;

        assert_eq!(fs::read(&target)?, b"new profile");
        assert_eq!(fs::read(&fixed_tmp)?, b"stale fixed tmp");
        Ok(())
    }

    #[test]
    fn profile_file_removes_unique_temp_after_replace_error() -> Result<()> {
        let temp = TempDir::new("replace-error");
        let target = temp.path.join("profile.json");
        fs::create_dir_all(&target)?;

        let error = write_profile_file(&target, b"new profile", "profile test")
            .expect_err("replacing a directory should fail");

        assert!(error.to_string().contains("failed to replace profile test"));
        assert!(error.to_string().contains(&target.display().to_string()));
        assert!(error.downcast_ref::<std::io::Error>().is_some());
        assert_eq!(fs::read_dir(&temp.path)?.count(), 1);
        assert!(target.is_dir());
        Ok(())
    }

    #[test]
    fn durable_profile_file_publishes_replacements_in_a_new_directory() -> Result<()> {
        let temp = TempDir::new("durable");
        let target = temp.path.join("new/nested/metadata.json");
        write_profile_file_durable(&target, b"first", "identity")?;
        write_profile_file_durable(&target, b"second", "identity")?;
        assert_eq!(fs::read(&target)?, b"second");
        assert_eq!(fs::read_dir(target.parent().unwrap())?.count(), 1);
        Ok(())
    }

    #[test]
    fn durable_profile_file_distinguishes_unpublished_and_uncertain_failures() -> Result<()> {
        let temp = TempDir::new("durable-failure");
        let target = temp.path.join("metadata.json");
        fs::create_dir_all(&target)?;
        let error = write_profile_file_durable(&target, b"first", "identity").unwrap_err();
        assert!(!error.publication_uncertain());
        assert!(target.is_dir());
        assert_eq!(fs::read_dir(&temp.path)?.count(), 1);
        fs::remove_dir(&target)?;
        write_profile_file_durable(&target, b"old", "identity")?;
        FAIL_DIRECTORY_SYNC.with(|fault| fault.set(true));
        let error = write_profile_file_durable(&target, b"new", "identity").unwrap_err();
        assert!(error.publication_uncertain());
        assert_eq!(fs::read(&target)?, b"new");
        // Recovery reads the actually published value; a new publication works.
        write_profile_file_durable(&target, b"recovered", "identity")?;
        assert_eq!(fs::read(&target)?, b"recovered");
        Ok(())
    }
}
