//! A profile lock covers both legacy inputs and publication. The version-6
//! manifest remains authoritative until both closed, checkpointed databases
//! are verified and durable; JSON inputs remain untouched as backups.
use anyhow::{Context, Result, bail};
use moli_browser_profile::{BrowserProfile, write_profile_file_durable};
use moli_sqlite::RuntimeHandle;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{BufReader, Read},
    path::{Path, PathBuf},
};

const STATE: &str = ".storage-sqlite-v7.json";
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MigrationState {
    format: String,
    source_fingerprint: String,
    migration_id: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Boundary {
    Owned,
    PublishedCookies,
    PublishedLocalStorage,
    BeforeManifest,
    AfterManifest,
}

pub fn migrate_profile_storage(profile: &mut BrowserProfile, runtime: RuntimeHandle) -> Result<()> {
    migrate(profile, runtime, |_| Ok(()))
}
fn source_fingerprint(profile: &BrowserProfile) -> Result<String> {
    let mut digest = Sha256::new();
    digest.update(b"moli-cookie-localstorage-sqlite-v7\0");
    for (label, path) in [
        (b"cookies".as_slice(), &profile.paths().cookies_path),
        (
            b"localStorage".as_slice(),
            &profile.paths().local_storage_path,
        ),
    ] {
        digest.update(label);
        match fs::File::open(path) {
            Ok(file) => {
                digest.update(b"present\0");
                let mut file = BufReader::new(file);
                let mut buffer = [0u8; 16 * 1024];
                loop {
                    let bytes = file.read(&mut buffer).with_context(|| {
                        format!("failed to hash legacy storage `{}`", path.display())
                    })?;
                    if bytes == 0 {
                        break;
                    }
                    digest.update(&buffer[..bytes]);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                digest.update(b"absent\0")
            }
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("failed to open legacy storage `{}`", path.display())
                });
            }
        }
        digest.update(b"\0end\0");
    }
    Ok(format!("{:x}", digest.finalize()))
}
fn record_state(path: &Path, state: &MigrationState) -> Result<()> {
    write_profile_file_durable(
        path,
        &serde_json::to_vec(state)?,
        "storage migration ownership",
    )?;
    Ok(())
}
fn migrate(
    profile: &mut BrowserProfile,
    runtime: RuntimeHandle,
    mut boundary: impl FnMut(Boundary) -> Result<()>,
) -> Result<()> {
    if !profile.needs_sqlite_storage_migration() {
        return Ok(());
    }
    profile.default_partition().ensure_directory()?;
    let root = &profile.paths().partition_root;
    let state_path = root.join(STATE);
    let fingerprint = source_fingerprint(profile)?;
    let cookie_target = profile.paths().cookies_sqlite_path.clone();
    let local_target = profile.paths().local_storage_sqlite_path.clone();
    let cookie_stage = root.join("cookies.sqlite.migrating");
    let local_stage = root.join("localstorage.sqlite.migrating");
    let state = if state_path.exists() {
        let existing: MigrationState = serde_json::from_slice(&fs::read(&state_path)?)
            .context("unrecognized storage migration ownership record")?;
        if existing.format != "moli-cookie-localstorage-sqlite-v7"
            || existing.source_fingerprint != fingerprint
            || existing.migration_id.len() != 32
            || !existing
                .migration_id
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            bail!(
                "legacy storage sources or migration identity changed; preserve the sources and `{}` for recovery",
                state_path.display()
            );
        }
        existing
    } else {
        for path in [&cookie_target, &local_target, &cookie_stage, &local_stage] {
            for suffix in ["", "-wal", "-shm"] {
                if path_present(&with_suffix(path, suffix))? {
                    bail!(
                        "unidentified SQLite migration files exist in `{}`",
                        root.display()
                    );
                }
            }
        }
        let mut nonce = [0u8; 16];
        getrandom::fill(&mut nonce).map_err(|error| {
            anyhow::anyhow!("failed to allocate storage migration identity: {error}")
        })?;
        let state = MigrationState {
            format: "moli-cookie-localstorage-sqlite-v7".into(),
            source_fingerprint: fingerprint.clone(),
            migration_id: nonce.iter().map(|byte| format!("{byte:02x}")).collect(),
        };
        record_state(&state_path, &state)?;
        state
    };
    boundary(Boundary::Owned)?;
    if !cookie_target.exists() {
        remove_owned_stage(&cookie_stage)?;
        let cookies = moli_browser_profile::load_cookie_cache(&profile.paths().cookies_path)?;
        moli_cookie_db::create_from_legacy(
            cookie_stage.clone(),
            cookies,
            fingerprint.clone(),
            state.migration_id.clone(),
            runtime.clone(),
        )?;
        publish(&cookie_stage, &cookie_target)?;
        record_state(&state_path, &state)?;
    }
    require_regular_file(&cookie_target)?;
    let cookie_marker =
        moli_cookie_db::CookieDatabase::inspect_migration(cookie_target.clone(), runtime.clone())?;
    if cookie_marker != Some((fingerprint.clone(), state.migration_id.clone())) {
        bail!("Cookie database does not belong to this storage migration");
    }
    boundary(Boundary::PublishedCookies)?;
    if !local_target.exists() {
        remove_owned_stage(&local_stage)?;
        moli_web_storage::create_sqlite_from_legacy_json(
            local_stage.clone(),
            profile.paths().local_storage_path.clone(),
            fingerprint.clone(),
            state.migration_id.clone(),
            runtime.clone(),
        )?;
        publish(&local_stage, &local_target)?;
        record_state(&state_path, &state)?;
    }
    require_regular_file(&local_target)?;
    let local_marker = moli_web_storage::inspect_sqlite_migration(local_target.clone(), runtime)?;
    if local_marker != Some((fingerprint.clone(), state.migration_id.clone())) {
        bail!("localStorage database does not belong to this storage migration");
    }
    boundary(Boundary::PublishedLocalStorage)?;
    if source_fingerprint(profile)? != fingerprint {
        bail!("legacy storage changed while the SQLite databases were being prepared");
    }
    // This final durable record is also the directory barrier for both links.
    record_state(&state_path, &state)?;
    boundary(Boundary::BeforeManifest)?;
    profile.finish_sqlite_storage_migration()?;
    boundary(Boundary::AfterManifest)?;
    // The marker is retained for diagnostics. JSON backups are never dual-written.
    Ok(())
}
fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}
fn path_present(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error)
            .with_context(|| format!("failed to inspect storage path `{}`", path.display())),
    }
}
fn require_regular_file(path: &Path) -> Result<()> {
    if !fs::symlink_metadata(path)?.file_type().is_file() {
        bail!(
            "migration target is not a regular file: `{}`",
            path.display()
        );
    }
    Ok(())
}
fn remove_owned_stage(path: &Path) -> Result<()> {
    for path in [
        path.to_path_buf(),
        with_suffix(path, "-wal"),
        with_suffix(path, "-shm"),
    ] {
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_file() => {
                fs::remove_file(&path).with_context(|| {
                    format!(
                        "failed to discard incomplete owned staging file `{}`",
                        path.display()
                    )
                })?
            }
            Ok(_) => bail!(
                "owned migration path is not a regular file: `{}`",
                path.display()
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("failed to inspect migration stage `{}`", path.display())
                });
            }
        }
    }
    Ok(())
}
fn publish(stage: &Path, target: &Path) -> Result<()> {
    fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(stage)?
        .sync_all()?;
    // Same-filesystem link publication is atomic and never replaces an
    // existing file. All SQLite connections and WALs are already closed.
    fs::hard_link(stage, target)
        .with_context(|| format!("failed to publish storage `{}`", target.display()))?;
    fs::remove_file(stage)?;
    Ok(())
}

#[cfg(test)]
mod tests;
