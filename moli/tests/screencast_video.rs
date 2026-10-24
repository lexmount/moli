use std::{
    io::{BufRead, BufReader},
    net::SocketAddr,
    path::Path,
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, ensure};
use clap::Parser;
use futures_util::{SinkExt, StreamExt};
use moli::{cli::Cli, config::AppConfig};

#[test]
fn video_path_cli_requires_layout_and_preserves_os_paths() {
    for args in [
        vec!["moli", "serve", "--screencast-video", "video.mp4"],
        vec!["moli", "serve", "--layout", "--screencast-video", ""],
    ] {
        assert!(Cli::try_parse_from(args).is_err());
    }
    let cli = Cli::try_parse_from([
        "moli",
        "serve",
        "--layout",
        "--screencast-video",
        "movie.mp4",
    ])
    .unwrap();
    assert_eq!(
        AppConfig::from_cli(&cli)
            .unwrap()
            .server
            .screencast_video
            .as_deref(),
        Some(Path::new("movie.mp4"))
    );
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        let path = std::ffi::OsString::from_vec(b"movie-\xff.mp4".to_vec());
        let cli = Cli::try_parse_from([
            "moli".into(),
            "serve".into(),
            "--layout".into(),
            "--screencast-video".into(),
            path.clone(),
        ])
        .unwrap();
        assert_eq!(
            AppConfig::from_cli(&cli)
                .unwrap()
                .server
                .screencast_video
                .unwrap()
                .into_os_string(),
            path
        );
    }
}

#[test]
fn video_environment_and_cli_precedence_are_validated_in_child_processes() {
    for (layout, explicit, expected) in [
        ("true", false, true),
        ("false", false, false),
        ("0", false, false),
        ("false", true, true),
    ] {
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "video_environment_child", "--nocapture"])
            .env(
                "MOLI_VIDEO_TEST_CHILD",
                if explicit { "cli" } else { "env" },
            )
            .env("MOLI_VIDEO_TEST_VALID", expected.to_string())
            .env("MOLI_LAYOUT", layout)
            .env("MOLI_SCREENCAST_VIDEO", "environment.mp4")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn video_environment_child() {
    let Ok(mode) = std::env::var("MOLI_VIDEO_TEST_CHILD") else {
        return;
    };
    let expected = std::env::var("MOLI_VIDEO_TEST_VALID").unwrap() == "true";
    let args = if mode == "cli" {
        vec![
            "moli",
            "serve",
            "--layout",
            "--screencast-video",
            "explicit.mp4",
        ]
    } else {
        vec!["moli", "serve"]
    };
    let result = Cli::try_parse_from(args)
        .map_err(anyhow::Error::from)
        .and_then(|cli| AppConfig::from_cli(&cli));
    assert_eq!(result.is_ok(), expected);
    if let Ok(config) = result {
        assert_eq!(
            config.server.screencast_video.as_deref(),
            Some(Path::new(if mode == "cli" {
                "explicit.mp4"
            } else {
                "environment.mp4"
            }))
        );
    }
}

struct Server {
    child: Child,
    address: SocketAddr,
}

impl Server {
    fn start(path: &Path) -> Result<Self> {
        let mut child = Command::new(env!("CARGO_BIN_EXE_moli"))
            .args([
                "serve",
                "--port",
                "0",
                "--layout",
                "--log-level",
                "info",
                "--screencast-interval-ms",
                "10",
                "--screencast-video",
            ])
            .arg(path)
            .env_remove("MOLI_SCREENCAST_VIDEO")
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()?;
        let stderr = child.stderr.take().context("server stderr")?;
        let log = std::sync::Arc::new(parking_lot::Mutex::new(String::new()));
        let stderr_log = log.clone();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                let mut log = stderr_log.lock();
                if log.len() < 64 * 1024 {
                    log.push_str(&line);
                    log.push('\n');
                }
                drop(log);
                let plain = strip_ansi_escapes::strip_str(&line);
                if plain.contains("protocol server listening")
                    && let Some(address) = plain
                        .split("addr=")
                        .nth(1)
                        .and_then(|tail| tail.split_whitespace().next())
                        .and_then(|value| value.parse::<SocketAddr>().ok())
                {
                    let _ = sender.send(address);
                }
            }
        });
        match receiver.recv_timeout(Duration::from_secs(15)) {
            Ok(address) => Ok(Self { child, address }),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                Err(anyhow::anyhow!(
                    "video server startup failed: {error}; stderr: {}",
                    log.lock()
                ))
            }
        }
    }

    fn wait(&mut self) -> Result<std::process::ExitStatus> {
        let deadline = Instant::now() + Duration::from_secs(7);
        loop {
            if let Some(status) = self.child.try_wait()? {
                return Ok(status);
            }
            ensure!(
                Instant::now() < deadline,
                "video server did not exit within its deadline"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn assert_video(path: &Path) -> Result<()> {
    let reader = mp4::Mp4Reader::read_header(std::fs::File::open(path)?, path.metadata()?.len())?;
    let track = reader.tracks().get(&1).context("video track")?;
    ensure!(
        track.media_type()? == mp4::MediaType::H264,
        "video must be H.264"
    );
    ensure!(
        track.sample_count() > 0 && track.duration() > Duration::ZERO,
        "video must contain timed samples"
    );
    Ok(())
}

type Client =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn send_command(
    client: &mut Client,
    id: u64,
    method: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value> {
    client
        .send(tokio_tungstenite::tungstenite::Message::Text(
            serde_json::json!({"id":id,"method":method,"params":params})
                .to_string()
                .into(),
        ))
        .await?;
    tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(message) = client.next().await {
            if let tokio_tungstenite::tungstenite::Message::Text(text) = message? {
                let response: serde_json::Value = serde_json::from_str(&text)?;
                if response["id"] == id {
                    ensure!(
                        response.get("error").is_none(),
                        "{method} failed: {response}"
                    );
                    return Ok(response);
                }
            }
        }
        anyhow::bail!("connection closed before {method} response")
    })
    .await?
}

async fn start_screencast(server: &Server) -> Result<Client> {
    let (mut client, _) = tokio_tungstenite::connect_async(format!(
        "ws://{}/devtools/page/moli-default",
        server.address
    ))
    .await?;
    send_command(&mut client, 10, "Page.enable", serde_json::json!({})).await?;
    start_screencast_on_client(&mut client, 11).await?;
    Ok(client)
}

async fn start_screencast_on_client(client: &mut Client, id: u64) -> Result<i64> {
    command_and_frame(
        client,
        id,
        "Page.startScreencast",
        serde_json::json!({
            "format":"jpeg","maxWidth":160,"maxHeight":90
        }),
    )
    .await
}

async fn command_and_frame(
    client: &mut Client,
    id: u64,
    method: &str,
    params: serde_json::Value,
) -> Result<i64> {
    client
        .send(tokio_tungstenite::tungstenite::Message::Text(
            serde_json::json!({
                "id":id, "method":method, "params":params
            })
            .to_string()
            .into(),
        ))
        .await?;
    tokio::time::timeout(Duration::from_secs(5), async {
        let (mut response_seen, mut frame_session_id) = (false, None);
        while let Some(message) = client.next().await {
            if let tokio_tungstenite::tungstenite::Message::Text(text) = message? {
                let value: serde_json::Value = serde_json::from_str(&text)?;
                if value["id"] == id {
                    ensure!(value.get("error").is_none(), "{method} failed: {value}");
                    response_seen = true;
                }
                if value["method"] == "Page.screencastFrame" {
                    frame_session_id = Some(
                        value["params"]["sessionId"]
                            .as_i64()
                            .context("screencast frame session id")?,
                    );
                }
                if response_seen && let Some(frame_session_id) = frame_session_id {
                    return Ok::<i64, anyhow::Error>(frame_session_id);
                }
            }
        }
        anyhow::bail!("{method} did not deliver both response and frame")
    })
    .await?
}

async fn wait_for_server_close(client: &mut Client) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(message) = client.next().await {
            if matches!(message?, tokio_tungstenite::tungstenite::Message::Close(_)) {
                break;
            }
        }
        Ok::<(), anyhow::Error>(())
    })
    .await?
}

#[test]
#[cfg(unix)]
fn configured_sink_does_not_record_without_a_screencast() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("main.mp4");
    let mut server = Server::start(&path)?;
    assert!(!path.exists());
    // SAFETY: the PID belongs to the live Moli child started above.
    assert_eq!(
        unsafe { libc::kill(server.child.id() as libc::pid_t, libc::SIGTERM) },
        0
    );
    assert_eq!(server.wait()?.code(), Some(143));
    assert!(!path.exists());
    ensure!(
        directory.path().read_dir()?.next().is_none(),
        "unused part file must be removed"
    );
    Ok(())
}

#[tokio::test]
#[cfg(unix)]
async fn screencast_video_is_finalized_on_sigterm() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("main.mp4");
    let mut server = Server::start(&path)?;
    let mut client = start_screencast(&server).await?;
    assert!(!path.exists());
    // SAFETY: the PID belongs to the live Moli child started above.
    assert_eq!(
        unsafe { libc::kill(server.child.id() as libc::pid_t, libc::SIGTERM) },
        0
    );
    let _ = tokio::time::timeout(Duration::from_secs(3), client.close(None)).await;
    assert_eq!(server.wait()?.code(), Some(143));
    assert_video(&path)
}

#[tokio::test]
async fn repeated_screencasts_append_to_one_video_until_the_page_closes() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("main.mp4");
    let mut server = Server::start(&path)?;
    let mut client = start_screencast(&server).await?;
    for cycle in 0..3 {
        send_command(
            &mut client,
            20,
            "Page.stopScreencast",
            serde_json::json!({}),
        )
        .await?;
        assert!(
            !path.exists(),
            "stop must keep the MP4 open for a later start"
        );
        let response = send_command(
            &mut client,
            21,
            "Runtime.evaluate",
            serde_json::json!({"expression":"1+1","returnByValue":true}),
        )
        .await?;
        assert_eq!(response["result"]["result"]["value"], 2);
        if cycle < 2 {
            start_screencast_on_client(&mut client, 22).await?;
        }
    }
    assert!(server.child.try_wait()?.is_none());
    let (mut root, _) = tokio_tungstenite::connect_async(format!(
        "ws://{}/devtools/browser/moli-browser",
        server.address
    ))
    .await?;
    send_command(
        &mut root,
        1,
        "Target.closeTarget",
        serde_json::json!({"targetId":"moli-default"}),
    )
    .await?;
    let deadline = Instant::now() + Duration::from_secs(5);
    while !path.exists() {
        ensure!(
            Instant::now() < deadline,
            "main Page close did not finish MP4"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(server.child.try_wait()?.is_none());
    send_command(&mut root, 2, "Browser.close", serde_json::json!({})).await?;
    wait_for_server_close(&mut root).await?;
    drop(root);
    drop(client);
    assert!(server.wait()?.success());
    assert_video(&path)
}

#[tokio::test]
async fn video_is_finalized_on_browser_close() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("main.mp4");
    let mut server = Server::start(&path)?;
    let page_client = start_screencast(&server).await?;
    let (mut client, _) = tokio_tungstenite::connect_async(format!(
        "ws://{}/devtools/browser/moli-browser",
        server.address
    ))
    .await?;
    client
        .send(tokio_tungstenite::tungstenite::Message::Text(
            r#"{"id":1,"method":"Browser.close"}"#.into(),
        ))
        .await?;
    // Keep the frontend alive until this exact command's response is flushed.
    tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(message) = client.next().await {
            if let tokio_tungstenite::tungstenite::Message::Text(text) = message? {
                let response: serde_json::Value = serde_json::from_str(&text)?;
                if response["id"] == 1 {
                    ensure!(
                        response.get("error").is_none(),
                        "Browser.close failed: {response}"
                    );
                    return Ok::<(), anyhow::Error>(());
                }
            }
        }
        anyhow::bail!("browser closed before acknowledging Browser.close")
    })
    .await??;
    // Keep the transport alive until the server has confirmed its writer
    // flush and initiated shutdown. Disconnecting immediately after reading
    // the response races the existing failed-flush cancellation policy.
    wait_for_server_close(&mut client).await?;
    drop(client);
    drop(page_client);
    assert!(server.wait()?.success());
    assert_video(&path)
}

#[tokio::test]
async fn video_source_handover_refreshes_an_unchanged_screencast() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("handover.mp4");
    let mut server = Server::start(&path)?;
    let mut first = start_screencast(&server).await?;
    let (mut second, _) = tokio_tungstenite::connect_async(format!(
        "ws://{}/devtools/page/moli-default",
        server.address
    ))
    .await?;
    send_command(&mut second, 10, "Page.enable", serde_json::json!({})).await?;
    let frame_session_id = start_screencast_on_client(&mut second, 11).await?;
    send_command(&mut first, 12, "Page.stopScreencast", serde_json::json!({})).await?;
    // The second subscriber has already cached this exact picture. Taking over
    // video must still produce one raw frame before clean polling can resume.
    command_and_frame(
        &mut second,
        13,
        "Page.screencastFrameAck",
        serde_json::json!({
            "sessionId":frame_session_id
        }),
    )
    .await?;
    assert!(!path.exists());
    send_command(&mut second, 14, "Browser.close", serde_json::json!({})).await?;
    wait_for_server_close(&mut second).await?;
    drop(first);
    drop(second);
    assert!(server.wait()?.success());
    assert_video(&path)
}
