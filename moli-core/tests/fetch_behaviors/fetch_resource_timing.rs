use super::*;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

#[tokio::test(flavor = "multi_thread")]
async fn fetch_resource_timing_keeps_script_attribution_and_iframe_timeline_isolation() -> Result<()>
{
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}", listener.local_addr()?);
    let (shutdown, mut shutting_down) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        let mut connections = tokio::task::JoinSet::new();
        loop {
            tokio::select! {
                _ = &mut shutting_down => break,
                accepted = listener.accept() => {
                    let (mut socket, _) = accepted?;
                    connections.spawn(async move {
                        let mut bytes = Vec::new();
                        loop {
                            let mut buffer = [0;4096];
                            let size = socket.read(&mut buffer).await?;
                            if size == 0 { return Ok::<_, anyhow::Error>(()); }
                            bytes.extend_from_slice(&buffer[..size]);
                            if bytes.windows(4).any(|part| part == b"\r\n\r\n") { break; }
                        }
                        let request = String::from_utf8(bytes)?;
                        let path = request.split_whitespace().nth(1).context("request path")?;
                        let (mime,body) = match path {
                            "/page" => ("text/html", "<!doctype html><script src='/caller.js'></script><iframe id='child' src='/frame'></iframe>"),
                            "/frame" => ("text/html", "<!doctype html><script src='/child.js'></script>"),
                            "/caller.js" | "/child.js" => ("text/javascript", include_str!("../../../moli-renderer-v8/tests/fixtures/fetch-resource-timing.js")),
                            _ => ("text/plain", "body"),
                        };
                        socket.write_all(format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",body.len()
                        ).as_bytes()).await?;
                        Ok(())
                    });
                }
                Some(result) = connections.join_next(), if !connections.is_empty() => { result??; }
            }
        }
        connections.shutdown().await;
        Ok::<_, anyhow::Error>(())
    });
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser.fetch(&format!("{base}/page")).await?;
    let result = tokio::time::timeout(Duration::from_secs(15), page.evaluate_runtime_expression_with_await_async(r#"
        (async () => {
            const top = await fetchTimingDone;
            const frame = document.getElementById('child');
            await new Promise(resolve => {
                frame.addEventListener('load', resolve, {once:true});
                if (frame.contentDocument?.URL.endsWith('/frame') && frame.contentDocument.readyState === 'complete') resolve();
            });
            const child = await frame.contentWindow.fetchTimingDone;
            const script = performance.getEntriesByName(new URL('/caller.js', location.href).href)[0];
            return JSON.stringify({top,child,parserSource:script.initiatorUrl});
        })()
    "#, true)).await??;
    let _ = shutdown.send(());
    server.await??;
    let observed: serde_json::Value = serde_json::from_str(
        result["value"]
            .as_str()
            .context("Fetch timing observations")?,
    )?;
    let row = |document: &str, script: &str| {
        let name = format!("{base}/asset?context={document}");
        serde_json::json!({
            "body":"body", "name":name, "type":"fetch", "source":format!("{base}/{script}"),
            "jsonSource":format!("{base}/{script}"), "status":200, "decoded":4,
            "native":true, "entries":[name]
        })
    };
    assert_eq!(
        observed,
        serde_json::json!({
            "top":row("/page","caller.js"), "child":row("/frame","child.js"),
            "parserSource":format!("{base}/page")
        })
    );
    Ok(())
}
