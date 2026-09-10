use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

async fn read_request(stream: &mut TcpStream) -> Result<()> {
    let mut bytes = Vec::new();
    while !bytes.ends_with(b"\r\n\r\n") {
        anyhow::ensure!(bytes.len() < 16 * 1024, "oversized fixture request");
        bytes.push(stream.read_u8().await?);
    }
    Ok(())
}

async fn assert_opaque_stream(content_type: &str, corp: bool) -> Result<()> {
    let fixtures = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    for target in ["window", "child", "worker"] {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/stream", listener.local_addr()?);
        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\n{}Connection: close\r\n\r\n",
            if corp {
                "Cross-Origin-Resource-Policy: same-origin\r\n"
            } else {
                ""
            },
        );
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await?;
            read_request(&mut stream).await?;
            let stream_task = tokio::spawn(async move {
                let transfer = async {
                    stream.write_all(header.as_bytes()).await?;
                    stream.write_all(&[b'.'; 2048]).await?;
                    let mut byte = [0];
                    loop {
                        tokio::select! {
                            read = stream.read(&mut byte) => {
                                if read? == 0 { return Ok::<_, std::io::Error>(()); }
                            }
                            () = tokio::time::sleep(Duration::from_millis(10)) => {
                                stream.write_all(b".").await?;
                            }
                        }
                    }
                };
                // EOF and a failed write both mean that the client closed it.
                tokio::time::timeout(Duration::from_secs(6), transfer)
                    .await
                    .is_ok()
            });
            let (mut control, _) = listener.accept().await?;
            read_request(&mut control).await?;
            let closed = stream_task.await?;
            let body = serde_json::to_string(&serde_json::json!({"closed": closed}))?;
            control.write_all(format!(
                "HTTP/1.1 200 OK\r\nAccess-Control-Allow-Origin: *\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len(),
            ).as_bytes()).await?;
            Ok::<_, anyhow::Error>(closed)
        });
        let source = format!(
            "{}\nrunOpaqueStreamProbe({}, {}).then(finish, error => finish({{error: String(error)}}));",
            include_str!("../fixtures/runtime/fetch_opaque_stream.js"),
            serde_json::to_string(&url)?,
            serde_json::to_string(if corp { "blocked" } else { "opaque" })?,
        );
        let observed = tokio::time::timeout(
            Duration::from_secs(15),
            super::event_dispatch::run_probe(&browser, &fixtures, target, &source),
        )
        .await??;
        let closed = tokio::time::timeout(Duration::from_secs(10), server).await???;
        assert!(closed, "{content_type}/{target} left the transfer open");
        assert_eq!(
            observed,
            serde_json::json!({"errors": []}),
            "{content_type}/{target}/corp={corp}"
        );
    }
    fixtures.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn opaque_fetch_resolves_before_orb_body_validation_and_can_abort() -> Result<()> {
    assert_opaque_stream("text/plain", false).await
}

#[tokio::test(flavor = "multi_thread")]
async fn opaque_fetch_resolves_allowed_mime_responses_before_eof() -> Result<()> {
    assert_opaque_stream("application/javascript", false).await?;
    assert_opaque_stream("image/png", false).await
}

#[tokio::test(flavor = "multi_thread")]
async fn opaque_fetch_applies_corp_before_resolving_unfinished_responses() -> Result<()> {
    assert_opaque_stream("application/javascript", true).await
}
