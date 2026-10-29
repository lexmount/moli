use std::{
    ffi::OsString,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

pub(crate) fn write_file_atomically(path: &Path, bytes: &[u8], label: &str) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {label} dir `{}`", parent.display()))?;
    }

    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    match OpenOptions::new().write(true).open(path) {
        Ok(mut destination) => {
            // Opening the existing target enforces its write permissions even
            // when the containing directory would allow an atomic replacement.
            // Do not truncate regular files before the replacement is ready.
            let metadata = destination.metadata();
            if !metadata.as_ref().is_ok_and(|metadata| metadata.is_file()) {
                // Like curl, write pipes and devices through the opened handle
                // instead of replacing their paths with a regular file.
                return write_contents(&mut destination, bytes, path, label);
            }
            #[cfg(unix)]
            if let Ok(metadata) = metadata {
                use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

                // Retain the existing mode while still applying umask to the
                // new inode, as in curl's regular-file replacement path.
                options.mode(0o600 | metadata.permissions().mode());
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to open {label} `{}`", path.display()));
        }
    }

    let tmp_path = unique_temp_path(path);
    let mut file = options
        .open(&tmp_path)
        .with_context(|| format!("failed to create {label} `{}`", tmp_path.display()))?;
    let write_result = write_contents(&mut file, bytes, &tmp_path, label);
    drop(file);
    if let Err(error) = write_result {
        let _ = fs::remove_file(&tmp_path);
        return Err(error);
    }

    if let Err(error) = fs::rename(&tmp_path, path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(error).with_context(|| {
            format!(
                "failed to replace {label} `{}` with `{}`",
                path.display(),
                tmp_path.display()
            )
        });
    }
    Ok(())
}

fn write_contents(file: &mut fs::File, bytes: &[u8], path: &Path, label: &str) -> Result<()> {
    file.write_all(bytes)
        .with_context(|| format!("failed to write {label} `{}`", path.display()))?;
    file.flush()
        .with_context(|| format!("failed to flush {label} `{}`", path.display()))
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
        .unwrap_or_else(|| OsString::from("cookie-cache-write"));
    let mut tmp_name = OsString::from(".");
    tmp_name.push(file_name);
    tmp_name.push(format!(".{}.{}.{}.tmp", std::process::id(), nonce, counter));
    path.with_file_name(tmp_name)
}
