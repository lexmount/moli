//! Local, repeatable owner workload. See moli-curl/README.md for commands.
use anyhow::{Context, Result, bail};
use moli_curl::websocket::{
    CurlWebSocketConnection, CurlWebSocketEvent, CurlWebSocketRequest, CurlWebSocketRuntime,
    CurlWebSocketSend, MAX_SEND_FRAME_BYTES, WsFlags,
};
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};
use tokio_tungstenite::tungstenite::{self, Message};

struct Options {
    scenario: String,
    idle: usize,
    duration: Duration,
}

fn options() -> Result<Options> {
    let mut options = Options {
        scenario: "active".into(),
        idle: 32,
        duration: Duration::from_secs(3),
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let value = args
            .next()
            .context("expected a value after --scenario, --idle or --seconds")?;
        match arg.as_str() {
            "--scenario" => options.scenario = value,
            "--idle" => options.idle = value.parse()?,
            "--seconds" => options.duration = Duration::try_from_secs_f64(value.parse()?)?,
            _ => bail!("unknown option: {arg}"),
        }
    }
    if !matches!(
        options.scenario.as_str(),
        "idle" | "active" | "slow-read" | "wss"
    ) {
        bail!("scenario must be idle, active, slow-read or wss");
    }
    let count = options
        .idle
        .checked_add(usize::from(options.scenario != "idle"))
        .context("too many connections")?;
    if !(1..=255).contains(&count) || options.duration.is_zero() {
        bail!("use 1..255 total connections and a positive duration");
    }
    Ok(options)
}

fn serve<S: Read + Write>(stream: S, scenario: &str) -> Result<()> {
    let mut socket = tungstenite::accept(stream)
        .map_err(|error| anyhow::anyhow!("server handshake: {error}"))?;
    if matches!(scenario, "active" | "wss") {
        let message = Message::Binary(vec![7; MAX_SEND_FRAME_BYTES].into());
        while socket.send(message.clone()).is_ok() {}
    } else {
        loop {
            if scenario == "slow-read" {
                // This delay defines the benchmark peer's receive rate.
                thread::sleep(Duration::from_millis(2));
            }
            match socket.read() {
                Ok(Message::Binary(data)) => {
                    if data.len() != MAX_SEND_FRAME_BYTES || data.iter().any(|byte| *byte != 7) {
                        bail!("unexpected outgoing payload");
                    }
                }
                Ok(Message::Close(_)) | Err(_) => break,
                Ok(_) => {}
            }
        }
    }
    Ok(())
}

fn peer(
    scenario: String,
    tls: Option<Arc<rustls::ServerConfig>>,
) -> Result<(String, thread::JoinHandle<Result<()>>)> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let scheme = if tls.is_some() { "wss" } else { "ws" };
    let url = format!("{scheme}://{}/probe", listener.local_addr()?);
    let task = thread::spawn(move || {
        let (stream, _) = listener.accept()?;
        stream.set_write_timeout(Some(Duration::from_secs(10)))?;
        if let Some(tls) = tls {
            serve(
                rustls::StreamOwned::new(rustls::ServerConnection::new(tls)?, stream),
                &scenario,
            )
        } else {
            serve(stream, &scenario)
        }
    });
    Ok((url, task))
}

fn tls_config() -> Result<Arc<rustls::ServerConfig>> {
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
    let certificate = rcgen::generate_simple_self_signed(vec!["localhost".into()])?;
    Ok(Arc::new(
        rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from(certificate.cert.der().to_vec())],
                PrivateKeyDer::from(PrivatePkcs8KeyDer::from(
                    certificate.key_pair.serialize_der(),
                )),
            )?,
    ))
}

// Linux exposes scheduled CPU nanoseconds per thread. Other platforms still
// report workload throughput; no process-wide CPU is substituted for the owner.
fn owner_cpu_ns() -> Option<u64> {
    for task in std::fs::read_dir("/proc/self/task").ok()? {
        let path = task.ok()?.path();
        if std::fs::read_to_string(path.join("comm"))
            .ok()?
            .starts_with("moli-curl-web")
        {
            return std::fs::read_to_string(path.join("schedstat"))
                .ok()?
                .split_whitespace()
                .next()?
                .parse()
                .ok();
        }
    }
    None
}

async fn incoming(
    connection: &mut CurlWebSocketConnection,
    end: tokio::time::Instant,
) -> Result<u64> {
    let mut bytes = 0;
    while tokio::time::Instant::now() < end {
        let Ok(event) = tokio::time::timeout_at(end, connection.recv()).await else {
            break;
        };
        match event {
            Some(CurlWebSocketEvent::Chunk { data, .. }) => {
                if data.iter().any(|byte| *byte != 7) {
                    bail!("unexpected incoming payload");
                }
                bytes += data.len() as u64;
            }
            unexpected => bail!("connection ended during probe: {unexpected:?}"),
        }
    }
    Ok(bytes)
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let options = options()?;
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_ansi(false)
        .with_writer(std::io::stderr)
        .init();
    let runtime = CurlWebSocketRuntime::new()?;
    let tls = (options.scenario == "wss").then(tls_config).transpose()?;
    let mut peers = Vec::new();
    let mut connections = Vec::new();
    let active = options.scenario != "idle";
    for index in 0..options.idle + usize::from(active) {
        let scenario = if index < options.idle {
            "idle"
        } else {
            &options.scenario
        };
        let (url, task) = peer(scenario.into(), tls.clone())?;
        peers.push(task);
        let mut request = CurlWebSocketRequest::new(url);
        // The WSS fixture owns this locally generated certificate.
        request.tls.verify = tls.is_none();
        let mut connection = runtime.connect(request)?;
        match tokio::time::timeout(Duration::from_secs(10), connection.recv()).await? {
            Some(CurlWebSocketEvent::Handshake { result, .. }) => {
                result.map_err(anyhow::Error::msg)?
            }
            unexpected => bail!("expected handshake, got {unexpected:?}"),
        }
        connection.sender().set_reading(true);
        connections.push(connection);
    }
    // A fixed warmup is part of the workload, not a correctness assertion.
    if active && matches!(options.scenario.as_str(), "active" | "wss") {
        incoming(
            connections.last_mut().unwrap(),
            tokio::time::Instant::now() + Duration::from_millis(200),
        )
        .await?;
    } else {
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let cpu_before = owner_cpu_ns();
    let started = Instant::now();
    let end = tokio::time::Instant::now() + options.duration;
    let bytes = match options.scenario.as_str() {
        "idle" => {
            tokio::time::sleep_until(end).await;
            0
        }
        "slow-read" => {
            let sender = connections.last().unwrap().sender();
            let mut bytes = 0;
            while tokio::time::Instant::now() < end {
                let frame = CurlWebSocketSend {
                    flags: WsFlags::BINARY,
                    data: vec![7; MAX_SEND_FRAME_BYTES],
                };
                match tokio::time::timeout_at(end, sender.send_frame(frame)).await {
                    Ok(result) => bytes += result? as u64,
                    Err(_) => break,
                }
            }
            bytes
        }
        _ => {
            let connection = connections.last_mut().unwrap();
            if options.scenario == "wss" {
                connection.sender().set_reading(false);
                tokio::time::sleep_until(
                    end.min(tokio::time::Instant::now() + Duration::from_millis(100)),
                )
                .await;
                connection.sender().set_reading(true);
            }
            incoming(connection, end).await?
        }
    };
    let elapsed = started.elapsed();
    let cpu_ms = cpu_before
        .zip(owner_cpu_ns())
        .map(|(before, after)| after.saturating_sub(before) as f64 / 1_000_000.0);
    println!(
        "scenario={} idle={} elapsed_s={:.3} bytes={} mib_per_s={:.3} owner_cpu_ms={cpu_ms:?}",
        options.scenario,
        options.idle,
        elapsed.as_secs_f64(),
        bytes,
        bytes as f64 / 1_048_576.0 / elapsed.as_secs_f64()
    );
    drop(connections);
    drop(runtime);
    for task in peers {
        task.join()
            .map_err(|_| anyhow::anyhow!("peer panicked"))??;
    }
    Ok(())
}
