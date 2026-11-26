use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn font_face_binary_sources_validate_payloads_and_buffer_conversion_order() -> Result<()> {
    let fonts = serde_json::json!({
        "ttf": include_bytes!("../../../moli-layout/tests/fixtures/moli-ahem.ttf").as_slice(),
        "woff": include_bytes!("../../../moli-layout/tests/fixtures/moli-ahem.woff").as_slice(),
        "woff2": include_bytes!("../../../moli-layout/tests/fixtures/moli-ahem.woff2").as_slice(),
    });
    let server = FixtureServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser.fetch(&server.url("/static")).await?;
    let probe = include_str!("../fixtures/font-binary.js");
    let expression = format!("({probe})({fonts}).then(JSON.stringify)");
    let result = tokio::time::timeout(
        Duration::from_secs(30),
        page.evaluate_runtime_expression_with_await_async(&expression, true),
    )
    .await??;
    let value: serde_json::Value = serde_json::from_str(
        result["value"]
            .as_str()
            .context("binary FontFace results")?,
    )?;
    assert_eq!(value["failures"], serde_json::json!([]), "{value}");
    assert_eq!(value["rows"].as_array().unwrap().len(), 52, "{value}");
    server.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn stylesheet_web_font_timing_uses_its_document_url() -> Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let origin = format!("http://{}", listener.local_addr()?);
    let server = tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let Ok(byte) = socket.read_u8().await else {
                    break;
                };
                request.push(byte);
            }
            let (kind, body): (&str, &[u8]) = if request.starts_with(b"GET /style-font.woff2 ") {
                (
                    "font/woff2",
                    include_bytes!("../../../moli-layout/tests/fixtures/moli-ahem.woff2"),
                )
            } else {
                ("text/html", br#"<!doctype html><style>@font-face { font-family: Probe; src: url('/style-font.woff2') } body { font-family: Probe }</style><body>probe"#)
            };
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            if socket.write_all(head.as_bytes()).await.is_ok() {
                let _ = socket.write_all(body).await;
            }
        }
    });
    let mut config = AppConfig::default();
    config.set_optional_resource_fetch_mask(moli_page_types::OptionalResourceFetchMask::ALL);
    let browser = Browser::new(config)?;
    let document_url = format!("{origin}/page");
    let elapsed = async {
        let mut page = browser.fetch(&document_url).await?;
        let font_url = format!("{origin}/style-font.woff2");
        let expression = format!(
            "performance.getEntriesByName({font_url:?}).some(e => e.initiatorType === 'css')"
        );
        browser
            .wait_for_script_truthy(&mut page, &expression, Duration::from_secs(5))
            .await?;
        Ok::<(), anyhow::Error>(())
    }
    .await;
    server.abort();
    elapsed
}
