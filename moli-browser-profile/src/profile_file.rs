use std::{
    ffi::OsString,
    fs,
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

    use super::write_profile_file;

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
}
