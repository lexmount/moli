//! The peer half of the browser harness for `web-transport-wasm`.
//!
//! The echo servers only ever answer; nothing in them opens a stream toward the
//! client or resets one, so a browser client cannot exercise accept, a peer reset,
//! or a session close against them. This one is driven by the client instead: it
//! reads a command off a bidirectional stream and does what it says, which is what
//! lets the harness set up each scenario deterministically rather than by timing.
//!
//! Run it with the certificate from `dev/setup`, then serve the harness page:
//!
//! ```text
//! just harness
//! ```
//!
//! Unidirectional payloads are echoed as datagrams, so the harness can verify
//! opening and finishing a uni stream after abandoned opens.
//!
//! Commands, one per bidirectional stream, as a UTF-8 line:
//!
//! - `echo <text>` — write `<text>` back and finish.
//! - `streams <n>` — open `n` unidirectional streams carrying `stream-0`, `stream-1`,
//!   … then finish. The harness uses this to exercise accept.
//! - `streams-after <millis> <n>` — finish the command stream, wait, then open the
//!   streams. The harness uses this to drop a pending accept before a value exists.
//! - `reset <code>` — reset this stream with `<code>`, so the client sees a peer
//!   RESET_STREAM.
//! - `stop <code>` — stop the client's sending half with `<code>` while keeping the
//!   response half available.
//! - `close <code> <reason>` — close the whole session.

use std::{fs, io, path, time::Duration};

use anyhow::Context;
use clap::Parser;
use rustls::pki_types::CertificateDer;
use web_transport_quinn::{proto::ConnectResponse, Session};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(short, long, default_value = "[::]:4443")]
    addr: std::net::SocketAddr,

    /// Use the certificates at this path, encoded as PEM.
    #[arg(long)]
    pub tls_cert: path::PathBuf,

    /// Use the private key at this path, encoded as PEM.
    #[arg(long)]
    pub tls_key: path::PathBuf,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();

    let chain = fs::File::open(args.tls_cert).context("failed to open cert file")?;
    let chain: Vec<CertificateDer> = rustls_pemfile::certs(&mut io::BufReader::new(chain))
        .collect::<Result<_, _>>()
        .context("failed to load certs")?;
    anyhow::ensure!(!chain.is_empty(), "could not find certificate");

    let keys = fs::File::open(args.tls_key).context("failed to open key file")?;
    let key = rustls_pemfile::private_key(&mut io::BufReader::new(keys))
        .context("failed to load private key")?
        .context("missing private key")?;

    let mut server = web_transport_quinn::ServerBuilder::new()
        .with_addr(args.addr)
        .with_certificate(chain, key)?;

    tracing::info!(addr = %args.addr, "harness server listening");

    while let Some(conn) = server.accept().await {
        tokio::spawn(async move {
            if let Err(err) = run_conn(conn).await {
                tracing::info!(?err, "connection ended");
            }
        });
    }

    Ok(())
}

async fn run_conn(request: web_transport_quinn::Request) -> anyhow::Result<()> {
    let session = request
        .respond(ConnectResponse::OK)
        .await
        .context("failed to accept session")?;
    tracing::info!("accepted session");

    run_session(session).await
}

async fn run_session(session: Session) -> anyhow::Result<()> {
    loop {
        tokio::select! {
            res = session.accept_bi() => {
                let (send, recv) = res?;

                let session = session.clone();
                tokio::spawn(async move {
                    if let Err(err) = read_command(session, send, recv).await {
                        tracing::warn!(?err, "command failed");
                    }
                });
            },
            res = session.accept_uni() => {
                let mut recv = res?;
                let session = session.clone();
                tokio::spawn(async move {
                    match recv.read_to_end(4096).await {
                        Ok(payload) => {
                            if let Err(err) = session.send_datagram(payload.into()) {
                                tracing::debug!(?err, "uni echo ended");
                            }
                        }
                        // Abandoned opens deliberately reset before sending a payload.
                        Err(err) => tracing::debug!(?err, "uni stream abandoned"),
                    }
                });
            },
            // Datagrams are echoed, so the harness can drive send and recv together.
            res = session.read_datagram() => {
                let msg = res?;
                session.send_datagram(msg)?;
            },
        }
    }
}

async fn read_command(
    session: Session,
    send: web_transport_quinn::SendStream,
    mut recv: web_transport_quinn::RecvStream,
) -> anyhow::Result<()> {
    let mut command = Vec::new();
    let mut buf = [0; 1024];
    loop {
        match recv.read(&mut buf).await {
            Ok(Some(size)) => {
                command.extend_from_slice(&buf[..size]);
                anyhow::ensure!(command.len() <= 4096, "command too long");
                if let Some(newline) = command.iter().position(|&byte| byte == b'\n') {
                    command.truncate(newline);
                    break;
                }
            }
            Ok(None) => break,
            // The dropped-open regression deliberately resets command streams
            // before writing anything. That abandons one command, not the session.
            Err(err) => {
                tracing::debug!(?err, "command stream abandoned");
                return Ok(());
            }
        }
    }
    let command = String::from_utf8_lossy(&command).trim().to_string();
    tracing::info!(%command, "command");
    run_command(session, send, recv, command).await
}

async fn run_command(
    session: Session,
    mut send: web_transport_quinn::SendStream,
    mut recv: web_transport_quinn::RecvStream,
    command: String,
) -> anyhow::Result<()> {
    let (verb, rest) = match command.split_once(' ') {
        Some((verb, rest)) => (verb, rest),
        None => (command.as_str(), ""),
    };

    match verb {
        "echo" => {
            send.write_all(rest.as_bytes()).await?;
            send.finish()?;
        }
        "streams" => {
            let count: usize = rest.trim().parse().context("bad stream count")?;
            open_streams(&session, count).await?;
            send.finish()?;
        }
        "streams-after" => {
            let (millis, count) = rest.split_once(' ').context("missing stream count")?;
            let millis: u64 = millis.trim().parse().context("bad delay")?;
            let count: usize = count.trim().parse().context("bad stream count")?;

            send.finish()?;
            tokio::time::sleep(Duration::from_millis(millis)).await;
            open_streams(&session, count).await?;
        }
        "reset" => {
            let code: u32 = rest.trim().parse().context("bad reset code")?;
            send.reset(code)?;
        }
        "stop" => {
            let code: u32 = rest.trim().parse().context("bad stop code")?;
            recv.stop(code)?;
        }
        "close" => {
            let (code, reason) = rest.split_once(' ').unwrap_or((rest, ""));
            let code: u32 = code.trim().parse().context("bad close code")?;
            session.close(code, reason.as_bytes());
        }
        _ => anyhow::bail!("unknown command: {command}"),
    }

    Ok(())
}

async fn open_streams(session: &Session, count: usize) -> anyhow::Result<()> {
    // Held until every stream is open, so none is dropped while the harness is
    // still working through the earlier ones.
    let mut streams = Vec::with_capacity(count);
    for i in 0..count {
        let mut stream = session.open_uni().await?;
        stream.write_all(format!("stream-{i}").as_bytes()).await?;
        stream.finish()?;
        streams.push(stream);
    }
    Ok(())
}
