//! Loopback-only interoperability probe for the browser-owned QUIC connector.
//! Usage: URL CA_FILE MODE [TLS_HOSTNAME]. A '-' CA uses the system trust store.
//! The original URL port is dialled on 127.0.0.1; TLS validates the URL hostname.

use std::{path::PathBuf, time::Duration};

use anyhow::{Context, Result, ensure};
use moli_fetch::{FetchClient, FetchConfig};
use moli_webtransport::{CloseInfo, CloseReason, ConnectOptions};
use serde_json::{Value, json};

async fn run() -> Result<Value> {
    let args: Vec<_> = std::env::args().collect();
    let mut url = url::Url::parse(args.get(1).context("URL argument")?)?;
    let ca = args.get(2).context("CA argument")?;
    let mode = args.get(3).map(String::as_str).unwrap_or("echo");
    if let Some(hostname) = args.get(4) {
        url.set_host(Some(hostname))?;
    }
    let mut config = FetchConfig::default();
    config.set_http_proxy(Some(String::new()));
    let ca_cert = if ca == "-" {
        None
    } else {
        Some(PathBuf::from(ca))
    };
    config.set_tls_credentials(ca_cert, None, None, None);
    config.set_http_host_resolve(vec![format!(
        "{}:{}:127.0.0.1",
        url.host_str().context("URL host")?,
        url.port().context("fixture port")?
    )]);
    let client = FetchClient::new(&config, moli_cookie_jar::new_shared_browser_cookie_store());
    let connector = client.webtransport_connector();
    let options = ConnectOptions::new(url.clone(), "https://web-platform.localhost".into());
    let session = connector.connect(options).await?;
    let mut observations = json!({
        "url": url.as_str(), "tls_verified": true, "mode": mode,
        "negotiated_protocol": session.protocol(), "connector": "FetchClient",
    });
    if mode == "echo" {
        let payload = b"moli-browser-owned-webtransport\0\xff";
        let (mut send, mut recv) = session.open_bi().await?;
        send.write_all(payload).await?;
        send.finish()?;
        ensure!(
            recv.read_to_end(1024).await? == payload,
            "bidirectional echo"
        );
        observations["bidirectional_echo"] = json!(true);
        let mut send = session.open_uni().await?;
        send.write_all(payload).await?;
        send.finish()?;
        let mut recv = session.accept_uni().await?;
        ensure!(
            recv.read_to_end(1024).await? == payload,
            "unidirectional echo"
        );
        observations["unidirectional_echo"] = json!(true);
        session.send_datagram(bytes::Bytes::from_static(payload))?;
        ensure!(
            session.read_datagram().await?.as_ref() == payload,
            "datagram echo"
        );
        observations["datagram_echo"] = json!(true);
    }
    session.close(CloseInfo {
        code: 42,
        reason: "moli-close-💡".into(),
    });
    let closed = session.closed().await;
    ensure!(
        matches!(&closed,
        CloseReason::Local(CloseInfo { code: 42, reason }) if reason == "moli-close-💡"),
        "unexpected close outcome: {closed:?}"
    );
    observations["local_close"] = json!({"code":42,"reason":"moli-close-💡"});
    drop(client);
    ensure!(
        connector.is_closed(),
        "network owner must retire its connector"
    );
    observations["network_owner_retired"] = json!(true);
    Ok(observations)
}

#[tokio::main(worker_threads = 32)]
async fn main() {
    match tokio::time::timeout(Duration::from_secs(15), run()).await {
        Ok(Ok(value)) => println!("{value}"),
        Ok(Err(error)) => {
            println!("{}", json!({"error": format!("{error:#}")}));
            std::process::exit(1);
        }
        Err(error) => {
            println!("{}", json!({"timeout": error.to_string()}));
            std::process::exit(2);
        }
    }
}
