use super::*;

async fn assert_body_utf8_decoding(scenario: &str) -> Result<()> {
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
                let path = request.split_whitespace().nth(1).unwrap();
                let url = url::Url::parse(&format!("http://fixture{path}")).unwrap();
                let hex = url.query_pairs().find(|(name, _)| name == "hex")
                    .map(|(_, value)| value.into_owned()).unwrap_or_default();
                let body: Vec<u8> = hex.as_bytes().as_chunks::<2>().0.iter().map(|pair| {
                    u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap()
                }).collect();
                let mime = url.query_pairs().find(|(name, _)| name == "type")
                    .map(|(_, value)| value.into_owned()).unwrap();
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nAccess-Control-Allow-Origin: *\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n"
                );
                if stream.write_all(head.as_bytes()).await.is_err() {
                    return;
                }
                // Split the BOM over transport chunks before delivering the
                // remaining bytes. Consumers must decode after fully reading.
                let split = body.len().min(3);
                for chunk in body[..split].chunks(1).chain(std::iter::once(&body[split..])) {
                    if chunk.is_empty() {
                        continue;
                    }
                    let mut encoded = format!("{:X}\r\n", chunk.len()).into_bytes();
                    encoded.extend_from_slice(chunk);
                    encoded.extend_from_slice(b"\r\n");
                    if stream.write_all(&encoded).await.is_err() {
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                let _ = stream.write_all(b"0\r\n\r\n").await;
            });
        }
    });
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let source = format!(
        "{}\nrunBodyUtf8Probe({}, {}).then(finish, error => finish({{error: String(error)}}));",
        include_str!("../fixtures/runtime/body_utf8.js"),
        serde_json::to_string(scenario)?,
        serde_json::to_string(&response_url)?,
    );
    for target in ["window", "child", "worker"] {
        let observed = tokio::time::timeout(
            Duration::from_secs(20),
            super::event_dispatch::run_probe(&browser, &server, target, &source),
        )
        .await??;
        assert_eq!(
            observed,
            serde_json::json!({"errors": []}),
            "{scenario}/{target}"
        );
    }
    server.shutdown().await;
    tasks.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_body_text_decodes_utf8_and_removes_only_one_initial_bom() -> Result<()> {
    assert_body_utf8_decoding("text").await
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_body_json_decodes_utf8_before_parsing_and_preserves_syntax_errors() -> Result<()> {
    assert_body_utf8_decoding("json").await
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_body_binary_and_form_consumers_preserve_bom_bytes_and_values() -> Result<()> {
    assert_body_utf8_decoding("bytes").await
}
