use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn fetched_null_bodies_stay_unused_through_consumption_cloning_and_abort() -> Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let response_url = format!("http://{}/", listener.local_addr()?);
    let mut tasks = tokio::task::JoinSet::new();
    tasks.spawn(async move {
        let mut connections = tokio::task::JoinSet::new();
        while let Ok((mut stream, _)) = listener.accept().await {
            connections.spawn(async move {
                let mut request = Vec::new();
                let mut buffer = [0u8; 1024];
                while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                    let len = stream.read(&mut buffer).await.unwrap();
                    if len == 0 {
                        return;
                    }
                    request.extend_from_slice(&buffer[..len]);
                    assert!(request.len() < 8192, "unexpectedly large request headers");
                }
                let request = String::from_utf8(request).unwrap();
                let preflight = request.lines().any(|line| {
                    line.to_ascii_lowercase().starts_with("access-control-request-method:")
                });
                let mut fields = request.split_whitespace();
                let method = fields.next().unwrap();
                let url = url::Url::parse(&format!("http://fixture{}", fields.next().unwrap())).unwrap();
                let code = url.query_pairs().find(|(name, _)| name == "code")
                    .map(|(_, value)| value.into_owned()).unwrap();
                let code = if preflight { "204" } else { &code };
                let empty = url.query_pairs().any(|(name, value)| name == "empty" && value == "1");
                let body = if empty || preflight { "" } else { "name=value" };
                let head = format!(
                    "HTTP/1.1 {code} Test\r\nContent-Type: application/x-www-form-urlencoded\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, HEAD, POST, OPTIONS\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len(),
                );
                if stream.write_all(head.as_bytes()).await.is_ok() && method != "HEAD" {
                    // Intentionally send bytes for null-body statuses as WPT's
                    // status.py does. Delay 205 so abort can race the transport.
                    if code == "205" {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                    let _ = stream.write_all(body.as_bytes()).await;
                }
            });
        }
    });
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let source = format!(
        "{}\nrunFetchedNullBodyProbe({}).then(finish, error => finish({{error: String(error)}}));",
        include_str!("../fixtures/runtime/fetched_null_body.js"),
        serde_json::to_string(&response_url)?,
    );
    for target in ["window", "child", "worker"] {
        let observed = tokio::time::timeout(
            Duration::from_secs(20),
            super::event_dispatch::run_probe(&browser, &server, target, &source),
        )
        .await??;
        assert_eq!(observed, serde_json::json!({"errors": []}), "{target}");
    }
    server.shutdown().await;
    tasks.shutdown().await;
    Ok(())
}
