use std::{fs, io, path};

use anyhow::Context;
use clap::Parser;
use rustls::pki_types::CertificateDer;
use url::Url;
use web_transport_quinn::proto::ConnectRequest;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(short, long, default_value = "https://localhost:4443")]
    url: Url,

    /// Accept the certificates at this path, encoded as PEM.
    #[arg(long)]
    tls_cert: Option<path::PathBuf>,

    /// Dangerous: Disable TLS certificate verification.
    #[arg(long, default_value = "false")]
    tls_disable_verify: bool,

    /// Optional WebTransport subprotocol to negotiate.
    #[arg(long)]
    protocol: Option<String>,
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

    let client = web_transport_quinn::ClientBuilder::new();

    let client = if args.tls_disable_verify {
        tracing::warn!("disabling TLS certificate verification; a MITM attack is possible");

        // Accept any certificate.
        client.dangerous().with_no_certificate_verification()?
    } else if let Some(path) = &args.tls_cert {
        // Read the PEM certificate chain
        let chain = fs::File::open(path).context("failed to open cert file")?;
        let mut chain = io::BufReader::new(chain);

        let chain: Vec<CertificateDer> = rustls_pemfile::certs(&mut chain)
            .collect::<Result<_, _>>()
            .context("failed to load certs")?;

        anyhow::ensure!(!chain.is_empty(), "could not find certificate");

        // Only accept these certificates.
        // Also available: with_server_certificate_hashes
        client.with_server_certificates(chain)?
    } else {
        // Accept any certificate that matches a system root.
        client.with_system_roots()?
    };

    tracing::info!(url = %args.url, "connecting");

    // Connect to the given URL.
    let mut request = ConnectRequest::new(args.url);
    if let Some(protocol) = &args.protocol {
        request = request.with_protocol(protocol);
    }
    let session = client.connect(request).await?;

    tracing::info!("connected");

    match (&args.protocol, session.protocol()) {
        (Some(_), Some(protocol)) => {
            tracing::info!(%protocol, "negotiated protocol");
        }
        (Some(requested), None) => {
            tracing::warn!(%requested, "server did not negotiate protocol");
        }
        _ => {}
    }

    // Create a bidirectional stream.
    let (mut send, mut recv) = session.open_bi().await?;

    tracing::info!("created stream");

    // Send a message.
    let msg = "hello world".to_string();
    send.write_all(msg.as_bytes()).await?;
    tracing::info!(%msg, "sent");

    // Shut down the send stream.
    send.finish()?;

    // Read back the message.
    let msg = recv.read_to_end(1024).await?;
    tracing::info!(msg = %String::from_utf8_lossy(&msg), "recv");

    session.close(42069, b"bye");
    session.closed().await;

    Ok(())
}
