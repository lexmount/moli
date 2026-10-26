use std::{fs, io, path};

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

    /// Optional WebTransport subprotocol to support.
    #[arg(long)]
    pub protocol: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Enable info logging.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();

    // Read the PEM certificate chain
    let chain = fs::File::open(args.tls_cert).context("failed to open cert file")?;
    let mut chain = io::BufReader::new(chain);

    let chain: Vec<CertificateDer> = rustls_pemfile::certs(&mut chain)
        .collect::<Result<_, _>>()
        .context("failed to load certs")?;

    anyhow::ensure!(!chain.is_empty(), "could not find certificate");

    // Read the PEM private key
    let keys = fs::File::open(args.tls_key).context("failed to open key file")?;

    // Try to parse a PKCS#8 key
    // -----BEGIN PRIVATE KEY-----
    let key = rustls_pemfile::private_key(&mut io::BufReader::new(keys))
        .context("failed to load private key")?
        .context("missing private key")?;

    let mut server = web_transport_quinn::ServerBuilder::new()
        .with_addr(args.addr)
        .with_certificate(chain, key)?;

    tracing::info!(addr = %args.addr, "listening");

    // Accept new connections.
    while let Some(conn) = server.accept().await {
        let protocol = args.protocol.clone();
        tokio::spawn(async move {
            let err = run_conn(conn, protocol).await;
            if let Err(err) = err {
                tracing::error!(?err, "connection failed")
            }
        });
    }

    // TODO simple echo server

    Ok(())
}

async fn run_conn(
    request: web_transport_quinn::Request,
    protocol: Option<String>,
) -> anyhow::Result<()> {
    tracing::info!(url = %request.url, "received WebTransport request");

    // Negotiate protocol if both client and server support it.
    let negotiated = protocol.filter(|p| request.protocols.contains(p));
    if let Some(protocol) = &negotiated {
        tracing::info!(%protocol, "negotiated protocol");
    }

    // Accept the session.
    let mut response = ConnectResponse::OK;
    if let Some(protocol) = negotiated {
        response = response.with_protocol(protocol);
    }
    let session = request
        .respond(response)
        .await
        .context("failed to accept session")?;
    tracing::info!("accepted session");

    // Run the session
    if let Err(err) = run_session(session).await {
        tracing::info!(?err, "closing session");
    }

    Ok(())
}

async fn run_session(session: Session) -> anyhow::Result<()> {
    loop {
        // Wait for a bidirectional stream or datagram.
        tokio::select! {
            res = session.accept_bi() => {
                let (mut send, mut recv) = res?;
                tracing::info!("accepted stream");

                // Read the message and echo it back.
                let msg = recv.read_to_end(1024).await?;
                tracing::info!(msg = %String::from_utf8_lossy(&msg), "recv");

                send.write_all(&msg).await?;
                tracing::info!(msg = %String::from_utf8_lossy(&msg), "send");
            },
            res = session.read_datagram() => {
                let msg = res?;
                tracing::info!("accepted datagram");
                tracing::info!(msg = %String::from_utf8_lossy(&msg), "recv");

                session.send_datagram(msg.clone())?;
                tracing::info!(msg = %String::from_utf8_lossy(&msg), "send");
            },
        };

        tracing::info!("echo successful");
    }
}
