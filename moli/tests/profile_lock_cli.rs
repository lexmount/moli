use std::{
    path::PathBuf,
    process::{Command, Output},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Result;
use moli_browser_profile::{BrowserProfileLock, BrowserProfilePaths};
use strip_ansi_escapes::strip;

fn moli_cli_path() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_moli"))
}

fn unique_temp_dir(name: &str) -> Result<PathBuf> {
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let dir = std::env::temp_dir().join(format!("moli-{name}-{}-{nonce}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn run_fetch_with_profile_dir(profile_dir: &str) -> Result<Output> {
    Ok(Command::new(moli_cli_path())
        .arg("fetch")
        .arg("--log-level")
        .arg("error")
        .arg("--profile-dir")
        .arg(profile_dir)
        .arg("about:blank")
        .env_remove("HTTP_PROXY")
        .env_remove("HTTPS_PROXY")
        .env_remove("ALL_PROXY")
        .env_remove("http_proxy")
        .env_remove("https_proxy")
        .env_remove("all_proxy")
        .output()?)
}

fn run_serve_with_profile_dir(profile_dir: &str) -> Result<Output> {
    Ok(Command::new(moli_cli_path())
        .arg("serve")
        .arg("--log-level")
        .arg("error")
        .arg("--port")
        .arg("0")
        .arg("--profile-dir")
        .arg(profile_dir)
        .env_remove("HTTP_PROXY")
        .env_remove("HTTPS_PROXY")
        .env_remove("ALL_PROXY")
        .env_remove("http_proxy")
        .env_remove("https_proxy")
        .env_remove("all_proxy")
        .output()?)
}

fn clean_output(bytes: &[u8]) -> String {
    String::from_utf8_lossy(&strip(bytes)).into_owned()
}

#[test]
fn cli_profile_dir_refuses_when_profile_lock_is_held() -> Result<()> {
    let profile_dir = unique_temp_dir("profile-writer-lock")?;
    let profile_dir_arg = profile_dir.to_string_lossy().into_owned();
    let profile_paths = BrowserProfilePaths::new(&profile_dir);
    let _lock = BrowserProfileLock::acquire(&profile_paths)?;

    let output = run_fetch_with_profile_dir(&profile_dir_arg)?;

    assert!(
        !output.status.success(),
        "locked profile fetch should fail: stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = clean_output(&output.stderr);
    assert!(
        stderr.contains("already locked"),
        "stderr start: {stderr} stderr end"
    );
    assert!(
        stderr.contains("moli-profile.lock"),
        "stderr start: {stderr} stderr end"
    );
    assert!(
        stderr.contains(&profile_dir_arg),
        "stderr should identify the locked profile: {stderr}"
    );
    Ok(())
}

#[test]
fn cli_serve_profile_dir_refuses_when_profile_lock_is_held() -> Result<()> {
    let profile_dir = unique_temp_dir("serve-profile-writer-lock")?;
    let profile_dir_arg = profile_dir.to_string_lossy().into_owned();
    let profile_paths = BrowserProfilePaths::new(&profile_dir);
    let _lock = BrowserProfileLock::acquire(&profile_paths)?;

    let output = run_serve_with_profile_dir(&profile_dir_arg)?;

    assert!(
        !output.status.success(),
        "locked profile serve should fail: stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = clean_output(&output.stderr);
    assert!(
        stderr.contains("already locked"),
        "stderr start: {stderr} stderr end"
    );
    assert!(
        stderr.contains("moli-profile.lock"),
        "stderr start: {stderr} stderr end"
    );
    assert!(
        stderr.contains(&profile_dir_arg),
        "stderr should identify the locked profile: {stderr}"
    );
    Ok(())
}
