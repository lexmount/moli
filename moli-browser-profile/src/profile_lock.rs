use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
use std::os::unix::io::AsRawFd;
#[cfg(windows)]
use std::os::windows::io::AsRawHandle;
#[cfg(windows)]
use windows_sys::Win32::{
    Foundation::{ERROR_LOCK_VIOLATION, HANDLE},
    Storage::FileSystem::{LockFile, UnlockFile},
};

use anyhow::{Context, Result, anyhow};

use crate::BrowserProfilePaths;

const MAX_PROFILE_LOCK_METADATA_BYTES: usize = 1024;

#[derive(Debug)]
pub struct BrowserProfileLock {
    path: PathBuf,
    /// Held on platforms with process-owned advisory locks. Keeping this
    /// handle alive keeps the profile exclusively owned; the OS releases the
    /// lock if the process exits without running Rust destructors.
    #[cfg(any(unix, windows))]
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
        #[cfg(any(unix, windows))]
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
        #[cfg(any(unix, windows))]
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

#[cfg(windows)]
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

#[cfg(not(any(unix, windows)))]
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

#[cfg(windows)]
const WINDOWS_PROFILE_LOCK_OFFSET: u64 = 1_u64 << 32;
#[cfg(windows)]
const WINDOWS_PROFILE_LOCK_LENGTH: u64 = 1;

#[cfg(windows)]
fn try_lock_profile_file(file: &File) -> std::io::Result<()> {
    let (offset_low, offset_high) = split_windows_u64(WINDOWS_PROFILE_LOCK_OFFSET);
    let (length_low, length_high) = split_windows_u64(WINDOWS_PROFILE_LOCK_LENGTH);
    // SAFETY: the handle is valid for the lifetime of `file`; the locked byte
    // lies beyond the bounded metadata prefix so other contenders can still
    // read owner diagnostics while the exclusive process lock is held.
    let result = unsafe {
        LockFile(
            file.as_raw_handle() as HANDLE,
            offset_low,
            offset_high,
            length_low,
            length_high,
        )
    };
    if result != 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(windows)]
fn unlock_profile_file(file: &File) -> std::io::Result<()> {
    let (offset_low, offset_high) = split_windows_u64(WINDOWS_PROFILE_LOCK_OFFSET);
    let (length_low, length_high) = split_windows_u64(WINDOWS_PROFILE_LOCK_LENGTH);
    // SAFETY: this uses the same valid handle and byte range as acquisition.
    let result = unsafe {
        UnlockFile(
            file.as_raw_handle() as HANDLE,
            offset_low,
            offset_high,
            length_low,
            length_high,
        )
    };
    if result != 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(windows)]
const fn split_windows_u64(value: u64) -> (u32, u32) {
    (value as u32, (value >> 32) as u32)
}

#[cfg(windows)]
fn is_advisory_lock_contention(error: &std::io::Error) -> bool {
    error.raw_os_error() == Some(ERROR_LOCK_VIOLATION as i32)
}

fn lock_owner_description(path: &Path) -> String {
    let Ok(file) = File::open(path) else {
        return "lock owner metadata unavailable".to_owned();
    };
    let mut contents = String::new();
    let Ok(bytes_read) = file
        .take((MAX_PROFILE_LOCK_METADATA_BYTES + 1) as u64)
        .read_to_string(&mut contents)
    else {
        return "lock owner metadata unavailable".to_owned();
    };
    if bytes_read > MAX_PROFILE_LOCK_METADATA_BYTES {
        return "lock owner metadata unavailable".to_owned();
    }
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
    #[cfg(windows)]
    use std::{
        process::{Command, Stdio},
        thread,
        time::Duration,
    };

    use anyhow::Result;

    use super::{BrowserProfileLock, MAX_PROFILE_LOCK_METADATA_BYTES, lock_owner_description};
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
        #[cfg(any(unix, windows))]
        assert!(
            paths.lock_path.exists(),
            "advisory lock metadata file should remain reusable after drop"
        );
        #[cfg(not(any(unix, windows)))]
        assert!(
            !paths.lock_path.exists(),
            "sentinel lock file should be removed after drop"
        );

        let _second = BrowserProfileLock::acquire(&paths)?;
        Ok(())
    }

    #[cfg(windows)]
    const CHILD_PROFILE_ENV: &str = "MOLI_PROFILE_LOCK_TEST_CHILD_PROFILE";
    #[cfg(windows)]
    const CHILD_READY_ENV: &str = "MOLI_PROFILE_LOCK_TEST_CHILD_READY";

    #[cfg(windows)]
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
        let error = BrowserProfileLock::acquire(&paths)
            .expect_err("live child must keep the profile exclusively locked");
        assert!(error.to_string().contains("already locked"));
        let child_pid = child.id();

        child.kill()?;
        let _ = child.wait()?;

        let reopened = BrowserProfileLock::acquire(&paths)?;
        let metadata = fs::read_to_string(&paths.lock_path)?;
        assert!(metadata.contains(&format!("pid={}", std::process::id())));
        assert!(
            !metadata
                .lines()
                .any(|line| line == format!("pid={child_pid}")),
            "terminated owner metadata should be replaced: {metadata}"
        );
        drop(reopened);
        assert!(paths.lock_path.exists());
        Ok(())
    }

    #[test]
    fn profile_lock_owner_metadata_read_is_bounded() -> Result<()> {
        let profile = TempProfileDir::new("bounded-owner-metadata");
        let paths = BrowserProfilePaths::new(&profile.path);
        fs::create_dir_all(&profile.path)?;
        fs::write(
            &paths.lock_path,
            "pid=1\ncreated_unix_ms=1\n".repeat(MAX_PROFILE_LOCK_METADATA_BYTES),
        )?;

        assert_eq!(
            lock_owner_description(&paths.lock_path),
            "lock owner metadata unavailable"
        );
        Ok(())
    }

    #[cfg(any(unix, windows))]
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
