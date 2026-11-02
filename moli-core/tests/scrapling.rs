use moli_test_support as support;

use anyhow::{Context, Result, bail};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::Duration as StdDuration,
};
use strip_ansi_escapes::strip;
use support::FixtureServer;
use tokio::time::{Duration, Instant};

struct ChildGuard {
    child: Child,
}

impl ChildGuard {
    fn spawn(mut command: Command) -> Result<Self> {
        let child = command
            .spawn()
            .with_context(|| format!("failed to spawn {:?}", command))?;
        Ok(Self { child })
    }

    fn try_wait(&mut self) -> Result<Option<std::process::ExitStatus>> {
        self.child
            .try_wait()
            .context("failed to poll child process")
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn moli_cli_path() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("CARGO_BIN_EXE_moli") {
        return Ok(PathBuf::from(path));
    }

    let exe_name = format!("moli{}", std::env::consts::EXE_SUFFIX);
    let current_exe = std::env::current_exe()?;
    if let Some(debug_dir) = current_exe.parent().and_then(|dir| dir.parent()) {
        let candidate = debug_dir.join(&exe_name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }

    let workspace_target = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("target")
        .join("debug")
        .join(&exe_name);
    if workspace_target.is_file() {
        return Ok(workspace_target);
    }

    anyhow::bail!("failed to locate moli CLI binary")
}

fn clean_output(bytes: &[u8]) -> String {
    String::from_utf8_lossy(&strip(bytes)).into_owned()
}

fn local_scrapling_repo() -> Option<PathBuf> {
    std::env::var_os("MOLI_SCRAPLING_REPO")
        .map(PathBuf::from)
        .or_else(|| Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../Scrapling")))
        .filter(|path| path.exists())
}

fn local_scrapling_smoke_python() -> Option<PathBuf> {
    std::env::var_os("MOLI_SCRAPLING_SMOKE_PYTHON")
        .map(PathBuf::from)
        .or_else(|| {
            Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".venv-scrapling-smoke/bin/python"))
        })
        .filter(|path| path.exists())
}

fn python_supports_local_scrapling_smoke(python: &Path, scrapling_repo: &Path) -> Result<bool> {
    let output = Command::new(python)
        .arg("-c")
        .arg("import scrapling; from scrapling import DynamicFetcher; print(DynamicFetcher.__name__)")
        .env("PYTHONPATH", scrapling_repo)
        .output()
        .with_context(|| format!("failed to probe python interpreter `{}`", python.display()))?;
    if output.status.success() {
        return Ok(true);
    }
    eprintln!(
        "skipping local Scrapling smoke: python preflight failed: stdout={} stderr={}",
        clean_output(&output.stdout),
        clean_output(&output.stderr)
    );
    Ok(false)
}

fn pick_unused_local_port() -> Result<u16> {
    let listener =
        TcpListener::bind("127.0.0.1:0").context("failed to bind an ephemeral local port")?;
    let port = listener
        .local_addr()
        .context("failed to read local listener addr")?
        .port();
    drop(listener);
    Ok(port)
}

fn cdp_discovery_ready(port: u16) -> Result<bool> {
    let mut stream = match TcpStream::connect(("127.0.0.1", port)) {
        Ok(stream) => stream,
        Err(_) => return Ok(false),
    };
    stream
        .set_read_timeout(Some(StdDuration::from_millis(200)))
        .context("failed to set cdp discovery read timeout")?;
    stream
        .set_write_timeout(Some(StdDuration::from_millis(200)))
        .context("failed to set cdp discovery write timeout")?;
    stream
        .write_all(b"GET /json/version/ HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .context("failed to query cdp discovery endpoint")?;
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .context("failed to read cdp discovery response")?;
    Ok(response.starts_with("HTTP/1.1 200") || response.starts_with("HTTP/1.0 200"))
}

async fn wait_for_protocol_server_cdp_discovery_ready(
    server: &mut ChildGuard,
    port: u16,
) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if let Some(status) = server.try_wait()? {
            bail!("moli serve exited early with status {status}");
        }
        if cdp_discovery_ready(port)? {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    bail!("timed out waiting for moli cdp discovery on port {port}");
}

fn spawn_moli_protocol_server(port: u16) -> Result<ChildGuard> {
    let mut command = Command::new(moli_cli_path()?);
    command
        .arg("serve")
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(port.to_string())
        .arg("--log-level")
        .arg("error")
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .env_remove("HTTP_PROXY")
        .env_remove("HTTPS_PROXY")
        .env_remove("ALL_PROXY")
        .env_remove("http_proxy")
        .env_remove("https_proxy")
        .env_remove("all_proxy");
    ChildGuard::spawn(command)
}

fn run_local_scrapling_dynamic_fetcher_smoke(
    python: &Path,
    scrapling_repo: &Path,
    url: &str,
    cdp_url: &str,
) -> Result<std::process::Output> {
    let script = support::scrapling_dynamic_fetcher_smoke_script();
    Command::new(python)
        .arg(script)
        .arg(url)
        .arg(cdp_url)
        .env("MOLI_SCRAPLING_REPO", scrapling_repo)
        .env("PYTHONPATH", scrapling_repo)
        .env_remove("HTTP_PROXY")
        .env_remove("HTTPS_PROXY")
        .env_remove("ALL_PROXY")
        .env_remove("http_proxy")
        .env_remove("https_proxy")
        .env_remove("all_proxy")
        .output()
        .with_context(|| format!("failed to run Scrapling smoke with `{}`", python.display()))
}

async fn run_local_scrapling_dynamic_fetcher_smoke_async(
    python: &Path,
    scrapling_repo: &Path,
    url: &str,
    cdp_url: &str,
) -> Result<std::process::Output> {
    let python = python.to_path_buf();
    let scrapling_repo = scrapling_repo.to_path_buf();
    let url = url.to_owned();
    let cdp_url = cdp_url.to_owned();
    tokio::task::spawn_blocking(move || {
        run_local_scrapling_dynamic_fetcher_smoke(&python, &scrapling_repo, &url, &cdp_url)
    })
    .await
    .context("local Scrapling smoke task join failed")?
}

#[tokio::test]
async fn local_scrapling_dynamic_fetcher_over_cdp_smoke() -> Result<()> {
    if std::env::var("MOLI_RUN_LOCAL_SCRAPLING_SMOKE")
        .ok()
        .as_deref()
        != Some("1")
    {
        eprintln!("skipping local Scrapling smoke: set MOLI_RUN_LOCAL_SCRAPLING_SMOKE=1 to enable");
        return Ok(());
    }
    let Some(scrapling_repo) = local_scrapling_repo() else {
        eprintln!("skipping local Scrapling smoke: local Scrapling repo not found");
        return Ok(());
    };
    let Some(python) = local_scrapling_smoke_python() else {
        eprintln!("skipping local Scrapling smoke: python interpreter not found");
        return Ok(());
    };
    if !python_supports_local_scrapling_smoke(&python, &scrapling_repo)? {
        return Ok(());
    }

    let fixture_server = FixtureServer::spawn().await?;
    let cdp_port = pick_unused_local_port()?;
    let mut protocol_server = spawn_moli_protocol_server(cdp_port)?;
    wait_for_protocol_server_cdp_discovery_ready(&mut protocol_server, cdp_port).await?;

    let cdp_url =
        support::discover_browser_websocket_url((std::net::Ipv4Addr::LOCALHOST, cdp_port).into())
            .await?;
    let fixture_url = fixture_server.url("/static");
    let output = run_local_scrapling_dynamic_fetcher_smoke_async(
        &python,
        &scrapling_repo,
        &fixture_url,
        &cdp_url,
    )
    .await?;

    fixture_server.shutdown().await;

    assert!(
        output.status.success(),
        "Scrapling smoke failed: stdout={} stderr={}",
        clean_output(&output.stdout),
        clean_output(&output.stderr)
    );

    let payload: serde_json::Value = serde_json::from_slice(&output.stdout)
        .context("failed to decode Scrapling smoke json output")?;
    assert_eq!(payload["status"], 200);
    assert_eq!(payload["url"], fixture_url);
    assert_eq!(payload["body_contains_fixture_static"], true);
    assert_eq!(payload["main_text"], "fixture static");

    Ok(())
}
