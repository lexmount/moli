use std::{
    fs,
    io::{self, Read},
    path,
    sync::Arc,
};

use anyhow::Context;

use clap::Parser;
use rustls::pki_types::CertificateDer;
use web_transport_quinn::Session;

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
    let mut keys = fs::File::open(args.tls_key).context("failed to open key file")?;

    // Read the keys into a Vec so we can parse it twice.
    let mut buf = Vec::new();
    keys.read_to_end(&mut buf)?;

    // Try to parse a PKCS#8 key
    // -----BEGIN PRIVATE KEY-----
    let key = rustls_pemfile::private_key(&mut io::Cursor::new(&buf))
        .context("failed to load private key")?
        .context("missing private key")?;

    // Standard Quinn setup
    let mut config = rustls::ServerConfig::builder_with_provider(
        web_transport_quinn::crypto::default_provider(),
    )
    .with_protocol_versions(&[&rustls::version::TLS13])?
    .with_no_client_auth()
    .with_single_cert(chain, key)?;

    config.max_early_data_size = u32::MAX;
    config.alpn_protocols = vec![web_transport_quinn::ALPN.as_bytes().to_vec()]; // this one is important

    let config: quinn::crypto::rustls::QuicServerConfig = config.try_into()?;
    let config = quinn::ServerConfig::with_crypto(Arc::new(config));

    tracing::info!(addr = %args.addr, "listening");

    let server = quinn::Endpoint::server(config, args.addr)?;

    // Accept new connections.
    while let Some(conn) = server.accept().await {
        tokio::spawn(async move {
            let err = run_conn(conn).await;
            if let Err(err) = err {
                tracing::error!(?err, "connection failed")
            }
        });
    }

    // TODO simple echo server

    Ok(())
}

async fn run_conn(conn: quinn::Incoming) -> anyhow::Result<()> {
    tracing::info!("received new QUIC connection");

    // Wait for the QUIC handshake to complete.
    let conn = conn.await.context("failed to accept connection")?;
    tracing::info!("established QUIC connection");

    // Perform the WebTransport handshake.
    let request = web_transport_quinn::Request::accept(conn).await?;
    tracing::info!(url = %request.url, "received WebTransport request");

    // Log all HTTP3 headers
    tracing::info!("HTTP3 headers:");
    for (name, value) in request.headers.iter() {
        let value = value.to_str().context("invalid header value")?;
        tracing::info!("  {}: {}", name, value);
    }
    if request.headers.is_empty() {
        tracing::info!("  (empty)");
    }

    // Accept the session.
    let session = request.ok().await.context("failed to accept session")?;

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
