use std::{fs, io, path, sync::Arc};

use anyhow::Context;
use clap::Parser;
use rustls::pki_types::CertificateDer;
use url::Url;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(short, long, default_value = "https://localhost:4443")]
    url: Url,

    /// Accept the certificates at this path, encoded as PEM.
    #[arg(long)]
    tls_cert: path::PathBuf,
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

    let mut roots = rustls::RootCertStore::empty();
    roots.add_parsable_certificates(chain);

    // Standard quinn setup, accepting only the given certificate.
    // You should use system roots in production.
    let mut config = rustls::ClientConfig::builder_with_provider(
        web_transport_quinn::crypto::default_provider(),
    )
    .with_protocol_versions(&[&rustls::version::TLS13])?
    .with_root_certificates(roots)
    .with_no_client_auth();
    config.alpn_protocols = vec![web_transport_quinn::ALPN.as_bytes().to_vec()]; // this one is important

    let config: quinn::crypto::rustls::QuicClientConfig = config.try_into()?;
    let config = quinn::ClientConfig::new(Arc::new(config));

    let client = quinn::Endpoint::client("[::]:0".parse()?)?;
    let client = web_transport_quinn::Client::new(client, config);

    // Connect to the given URL.
    tracing::info!(url = %args.url, "connecting");
    let session = client.connect(args.url).await?;

    tracing::info!("connected");

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

    Ok(())
}
