use std::{
    fs::{File, OpenOptions, TryLockError},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow};

use crate::BrowserProfilePaths;

#[derive(Debug)]
pub struct BrowserProfileLock {
    path: PathBuf,
    /// Keeps the profile exclusively owned until this handle closes, including
    /// when the process exits without running Rust destructors.
    _file: File,
}

impl BrowserProfileLock {
    pub fn acquire(paths: &BrowserProfilePaths) -> Result<Self> {
        acquire_profile_lock(paths)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

pub fn acquire_profile_lock(paths: &BrowserProfilePaths) -> Result<BrowserProfileLock> {
    if !paths.root.as_os_str().is_empty() {
        std::fs::create_dir_all(&paths.root)
            .with_context(|| format!("failed to create profile dir `{}`", paths.root.display()))?;
    }

    // Keep this file in place after release so contenders always lock the same
    // file. Its contents do not participate in profile ownership.
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
    match file.try_lock() {
        Ok(()) => Ok(BrowserProfileLock {
            path: paths.lock_path.clone(),
            _file: file,
        }),
        Err(TryLockError::WouldBlock) => Err(anyhow!(
            "browser profile `{}` is already locked by `{}`",
            paths.root.display(),
            paths.lock_path.display()
        )),
        Err(TryLockError::Error(error)) => Err(error).with_context(|| {
            format!(
                "failed to acquire browser profile lock `{}`",
                paths.lock_path.display()
            )
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        process::{Command, Stdio},
        thread,
        time::{Duration, SystemTime, UNIX_EPOCH},
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
        assert_eq!(first.path(), paths.lock_path);

        let error =
            BrowserProfileLock::acquire(&paths).expect_err("second lock acquisition should fail");
        let error = error.to_string();
        assert!(error.contains("already locked"), "error: {error}");

        drop(first);
        assert!(
            paths.lock_path.exists(),
            "lock file should remain reusable after drop"
        );
        assert!(fs::read(&paths.lock_path)?.is_empty());

        let _second = BrowserProfileLock::acquire(&paths)?;
        Ok(())
    }

    const CHILD_PROFILE_ENV: &str = "MOLI_PROFILE_LOCK_TEST_CHILD_PROFILE";
    const CHILD_READY_ENV: &str = "MOLI_PROFILE_LOCK_TEST_CHILD_READY";

    #[test]
    fn profile_lock_is_released_when_owner_process_is_terminated() -> Result<()> {
        if let (Some(profile), Some(ready)) = (
            std::env::var_os(CHILD_PROFILE_ENV),
            std::env::var_os(CHILD_READY_ENV),
        ) {
            let profile = PathBuf::from(profile);
            let ready = PathBuf::from(ready);
            let paths = BrowserProfilePaths::new(&profile);
            let _lock = BrowserProfileLock::acquire(&paths)?;
            fs::write(&ready, b"ready")?;
            thread::sleep(Duration::from_secs(120));
            return Ok(());
        }

        let profile = TempProfileDir::new("terminated-owner");
        let ready = profile.path.join("child-ready");
        let test_name =
            "profile_lock::tests::profile_lock_is_released_when_owner_process_is_terminated";
        let mut child = Command::new(std::env::current_exe()?)
            .args(["--exact", test_name, "--nocapture"])
            .env(CHILD_PROFILE_ENV, &profile.path)
            .env(CHILD_READY_ENV, &ready)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;

        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !ready.exists() {
            if let Some(status) = child.try_wait()? {
                anyhow::bail!("profile lock child exited before readiness: {status}");
            }
            if std::time::Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                anyhow::bail!("timed out waiting for profile lock child readiness");
            }
            thread::sleep(Duration::from_millis(25));
        }

        let paths = BrowserProfilePaths::new(&profile.path);
        let contention = BrowserProfileLock::acquire(&paths);
        child.kill()?;
        let _ = child.wait()?;

        let error = contention.expect_err("live child must keep the profile exclusively locked");
        assert!(error.to_string().contains("already locked"));
        assert!(paths.lock_path.exists());

        let reopened = BrowserProfileLock::acquire(&paths)?;
        drop(reopened);
        assert!(fs::read(&paths.lock_path)?.is_empty());
        Ok(())
    }

    #[test]
    fn profile_lock_reuses_existing_unlocked_lock_file() -> Result<()> {
        let profile = TempProfileDir::new("stale-file");
        let paths = BrowserProfilePaths::new(&profile.path);
        fs::create_dir_all(&profile.path)?;
        fs::write(&paths.lock_path, "pid=1\ncreated_unix_ms=1\n")?;

        let lock = BrowserProfileLock::acquire(&paths)?;
        drop(lock);
        let _reopened = BrowserProfileLock::acquire(&paths)?;
        Ok(())
    }
}
