use super::preload_as::evaluate_preload_probe;
use super::*;
use std::sync::Arc;

#[tokio::test(flavor = "multi_thread")]
async fn child_parser_preloads_start_while_the_child_parser_is_blocked() -> Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    for destination in ["fetch", "style"] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/page", listener.local_addr()?);
        let requested = Arc::new(tokio::sync::Notify::new());
        let server = tokio::spawn(async move {
            let mut handlers = tokio::task::JoinSet::new();
            for _ in 0..3 {
                let (mut stream, _) =
                    tokio::time::timeout(Duration::from_secs(10), listener.accept()).await??;
                let requested = Arc::clone(&requested);
                handlers.spawn(async move {
                    let mut request = Vec::new();
                    let mut buffer = [0; 1024];
                    while !request.windows(4).any(|part| part == b"\r\n\r\n") {
                        let count = stream.read(&mut buffer).await?;
                        anyhow::ensure!(count != 0, "HTTP request ended before headers");
                        request.extend_from_slice(&buffer[..count]);
                    }
                    if request.starts_with(b"GET /gate.js ") {
                        // Hold the parser-blocking script until the already
                        // discovered link starts its preload. An EOF rescan
                        // cannot satisfy this request gate.
                        let before_unblock = tokio::time::timeout(
                            Duration::from_secs(5), requested.notified()
                        ).await.is_ok();
                        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/javascript\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await?;
                        return Ok::<_, anyhow::Error>(Some(before_unblock));
                    }
                    let markup = format!("<!doctype html><body><iframe srcdoc=\"<!doctype html><head><link rel=preload as={destination} href=/resource><script src=/gate.js></script></head><body>tail\"></iframe>");
                    let (mime, body) = if request.starts_with(b"GET /page ") {
                        ("text/html", markup.as_str())
                    } else {
                        anyhow::ensure!(request.starts_with(b"GET /resource "), "unexpected request");
                        requested.notify_one();
                        ("text/css", "body {}")
                    };
                    stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await?;
                    Ok(None)
                });
            }
            let mut before_unblock = None;
            while let Some(result) = handlers.join_next().await {
                if let Some(observed) = result?? {
                    before_unblock = Some(observed);
                }
            }
            Ok::<_, anyhow::Error>(before_unblock)
        });
        let browser = Browser::new(AppConfig::default())?;
        let page = browser.fetch(&url).await?;
        assert_eq!(server.await??, Some(true), "destination={destination}");
        let requests: Vec<_> = page
            .subresource_network_records()
            .iter()
            .filter(|record| record.url().path() == "/resource")
            .collect();
        assert_eq!(requests.len(), 1, "destination={destination}");
        assert!(
            requests[0].frame_id().is_some(),
            "preload belongs to the child frame"
        );
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn child_parser_preloads_obey_the_child_csp_instead_of_the_parent_policy() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    let mut config = AppConfig::default();
    config.set_optional_resource_fetch_mask(moli_page_types::OptionalResourceFetchMask::ALL);
    let browser = Browser::new(config)?;
    let mut page = browser.fetch(&server.url("/static")).await?;
    let fixture = format!(
        "(globalThis.preloadCspCounterPath='/compat/preload-count', globalThis.preloadCspDocumentPath='/compat/child-dynamic-markup-document', globalThis.preloadCspBlockParent=true, {})",
        include_str!("../fixtures/preload-child-csp.js")
    );
    let observed = evaluate_preload_probe(&mut page, &fixture).await?;
    assert_eq!(observed["observations"].as_array().unwrap().len(), 25);
    assert_eq!(observed["failures"], serde_json::json!([]));
    server.shutdown().await;
    Ok(())
}
