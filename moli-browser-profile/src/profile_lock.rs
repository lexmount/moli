use std::{
    fs::{File, OpenOptions},
    io::{Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
use std::os::unix::io::AsRawFd;

use anyhow::{Context, Result, anyhow};

use crate::BrowserProfilePaths;

#[derive(Debug)]
pub struct BrowserProfileLock {
    path: PathBuf,
    /// Held only on Unix, where the file descriptor keeps the advisory `flock`
    /// alive for the lifetime of the guard. On non-Unix platforms the lock is
    /// the file's existence on disk, so no handle needs to be retained.
    #[cfg(unix)]
    file: File,
    remove_on_drop: bool,
}

impl BrowserProfileLock {
    pub fn acquire(paths: &BrowserProfilePaths) -> Result<Self> {
        acquire_profile_lock(paths)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for BrowserProfileLock {
    fn drop(&mut self) {
        #[cfg(unix)]
        let _ = unlock_profile_file(&self.file);
        if self.remove_on_drop {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

pub fn acquire_profile_lock(paths: &BrowserProfilePaths) -> Result<BrowserProfileLock> {
    if !paths.root.as_os_str().is_empty() {
        std::fs::create_dir_all(&paths.root)
            .with_context(|| format!("failed to create profile dir `{}`", paths.root.display()))?;
    }

    let (mut file, remove_on_drop) = open_and_lock_profile_file(paths)?;
    if let Err(error) = write_lock_owner_metadata(&mut file, &paths.lock_path) {
        if remove_on_drop {
            let _ = std::fs::remove_file(&paths.lock_path);
        }
        return Err(error);
    }

    Ok(BrowserProfileLock {
        path: paths.lock_path.clone(),
        #[cfg(unix)]
        file,
        remove_on_drop,
    })
}

#[cfg(unix)]
fn open_and_lock_profile_file(paths: &BrowserProfilePaths) -> Result<(File, bool)> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&paths.lock_path)
        .with_context(|| {
            format!(
                "failed to open browser profile lock `{}`",
                paths.lock_path.display()
            )
        })?;
    match try_lock_profile_file(&file) {
        Ok(()) => Ok((file, false)),
        Err(error) if is_advisory_lock_contention(&error) => {
            let owner = lock_owner_description(&paths.lock_path);
            Err(anyhow!(
                "browser profile `{}` is already locked by `{}` ({owner})",
                paths.root.display(),
                paths.lock_path.display()
            ))
        }
        Err(error) => Err(error).with_context(|| {
            format!(
                "failed to acquire browser profile lock `{}`",
                paths.lock_path.display()
            )
        }),
    }
}

#[cfg(not(unix))]
fn open_and_lock_profile_file(paths: &BrowserProfilePaths) -> Result<(File, bool)> {
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&paths.lock_path)
    {
        Ok(file) => Ok((file, true)),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let owner = lock_owner_description(&paths.lock_path);
            Err(anyhow!(
                "browser profile `{}` is already locked by `{}` ({owner}); if no Moli process is using this profile, remove the stale lock file and retry",
                paths.root.display(),
                paths.lock_path.display()
            ))
        }
        Err(error) => Err(error).with_context(|| {
            format!(
                "failed to acquire browser profile lock `{}`",
                paths.lock_path.display()
            )
        }),
    }
}

fn write_lock_owner_metadata(file: &mut File, path: &Path) -> Result<()> {
    file.set_len(0).with_context(|| {
        format!(
            "failed to truncate browser profile lock `{}`",
            path.display()
        )
    })?;
    file.seek(SeekFrom::Start(0))
        .with_context(|| format!("failed to seek browser profile lock `{}`", path.display()))?;
    let created_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    writeln!(
        file,
        "pid={}\ncreated_unix_ms={created_unix_ms}",
        std::process::id()
    )
    .with_context(|| format!("failed to write browser profile lock `{}`", path.display()))?;
    file.flush()
        .with_context(|| format!("failed to flush browser profile lock `{}`", path.display()))?;
    Ok(())
}

#[cfg(unix)]
fn try_lock_profile_file(file: &File) -> std::io::Result<()> {
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(unix)]
fn unlock_profile_file(file: &File) -> std::io::Result<()> {
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) };
    if rc == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(unix)]
fn is_advisory_lock_contention(error: &std::io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(code) if code == libc::EWOULDBLOCK || code == libc::EAGAIN
    )
}

fn lock_owner_description(path: &Path) -> String {
    let Ok(contents) = std::fs::read_to_string(path) else {
        return "lock owner metadata unavailable".to_owned();
    };
    let mut fields = Vec::new();
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with("pid=") || line.starts_with("created_unix_ms=") {
            fields.push(line.to_owned());
        }
    }
    if fields.is_empty() {
        "lock owner metadata unavailable".to_owned()
    } else {
        fields.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use anyhow::Result;

    use super::BrowserProfileLock;
    use crate::BrowserProfilePaths;

    struct TempProfileDir {
        path: PathBuf,
    }

    impl TempProfileDir {
        fn new(name: &str) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock should be after epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "moli-profile-lock-{name}-{}-{nonce}",
                std::process::id()
            ));
            Self { path }
        }
    }

    impl Drop for TempProfileDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn profile_lock_refuses_second_writer_until_guard_drops() -> Result<()> {
        let profile = TempProfileDir::new("exclusive");
        let paths = BrowserProfilePaths::new(&profile.path);

        let first = BrowserProfileLock::acquire(&paths)?;
        assert!(paths.lock_path.exists());
        let lock_contents = fs::read_to_string(&paths.lock_path)?;
        assert!(
            lock_contents.contains("pid="),
            "lock contents: {lock_contents}"
        );

        let error =
            BrowserProfileLock::acquire(&paths).expect_err("second lock acquisition should fail");
        let error = error.to_string();
        assert!(error.contains("already locked"), "error: {error}");
        assert!(
            error.contains("pid=") && error.contains("created_unix_ms="),
            "error should include lock owner metadata: {error}"
        );

        drop(first);
        #[cfg(unix)]
        assert!(
            paths.lock_path.exists(),
            "Unix advisory lock metadata file should remain reusable after drop"
        );
        #[cfg(not(unix))]
        assert!(
            !paths.lock_path.exists(),
            "non-Unix sentinel lock file should be removed after drop"
        );

        let _second = BrowserProfileLock::acquire(&paths)?;
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn profile_lock_reuses_existing_unlocked_lock_file() -> Result<()> {
        let profile = TempProfileDir::new("stale-file");
        let paths = BrowserProfilePaths::new(&profile.path);
        fs::create_dir_all(&profile.path)?;
        fs::write(&paths.lock_path, "pid=1\ncreated_unix_ms=1\n")?;

        let lock = BrowserProfileLock::acquire(&paths)?;

        let lock_contents = fs::read_to_string(&paths.lock_path)?;
        assert!(
            lock_contents.contains(&format!("pid={}", std::process::id())),
            "lock contents should be rewritten for the current owner: {lock_contents}"
        );
        assert!(
            !lock_contents.lines().any(|line| line == "pid=1"),
            "old owner metadata should not survive successful acquisition: {lock_contents}"
        );

        drop(lock);
        let _reopened = BrowserProfileLock::acquire(&paths)?;
        Ok(())
    }
}
