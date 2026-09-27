use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn serve(mut socket: tokio::net::TcpStream) -> Result<()> {
    let mut request = Vec::new();
    while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
        let mut buffer = [0; 4096];
        let count = socket.read(&mut buffer).await?;
        if count == 0 {
            return Ok(());
        }
        request.extend_from_slice(&buffer[..count]);
    }
    let request = String::from_utf8(request)?;
    let path = request
        .split_whitespace()
        .nth(1)
        .context("request target")?;
    let url = url::Url::parse(&format!("http://fixture{path}"))?;
    const REPORT: &str = include_str!("../fixtures/location-origin-report.js");
    const STORAGE_REPORT: &str = include_str!("../fixtures/storage-origin-report.js");
    let (mime, body) = match url.path() {
        "/storage-report.js" => ("text/javascript", STORAGE_REPORT.to_owned()),
        "/storage-report" => (
            "text/html",
            format!(
                r#"<!doctype html><script>
            const inspectStorage = {STORAGE_REPORT};
            addEventListener("load", () => {{
                const url = new URL(location.href);
                if (url.searchParams.has("launch")) {{
                    url.searchParams.delete("launch");
                    open(url.href, "_blank");
                }} else {{
                    (opener || parent).top.postMessage(
                        inspectStorage(self, url.searchParams.get("token")), "*");
                }}
            }});
            </script>"#
            ),
        ),
        "/origin-report.js" => ("text/javascript", REPORT.to_owned()),
        "/origin-report" => (
            "text/html",
            format!("<!doctype html><script>{REPORT}</script>"),
        ),
        _ => ("text/html", "<!doctype html><body>root".to_owned()),
    };
    let csp = url
        .query_pairs()
        .find(|(key, _)| key == "csp")
        .map(|(_, value)| value.into_owned());
    let csp_header = csp.map_or_else(String::new, |value| {
        format!("Content-Security-Policy: {value}\r\n")
    });
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {mime};charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\n{csp_header}Connection: close\r\n\r\n{body}",
        body.len()
    );
    socket.write_all(response.as_bytes()).await?;
    Ok(())
}

async fn run_origin_surface_fixture(fixture: &str, expected_observations: usize) -> Result<()> {
    let local = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let remote = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let origins = [
        format!("http://{}", local.local_addr()?),
        format!("http://{}", remote.local_addr()?),
    ];
    let (shutdown, mut shutting_down) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        let mut requests = tokio::task::JoinSet::new();
        loop {
            tokio::select! {
                _ = &mut shutting_down => break,
                accepted = async { tokio::select! {
                    accepted = local.accept() => accepted,
                    accepted = remote.accept() => accepted,
                }} => {
                    let (socket, _) = accepted?;
                    requests.spawn(serve(socket));
                }
                Some(result) = requests.join_next(), if !requests.is_empty() => { result??; }
            }
        }
        requests.shutdown().await;
        Ok::<_, anyhow::Error>(())
    });
    let result = tokio::time::timeout(Duration::from_secs(30), async {
        let browser = Browser::new(AppConfig::default())?;
        let mut page = browser.fetch(&origins[0]).await?;
        let script = format!(
            "(globalThis.originTestOrigins={}, {})",
            serde_json::to_string(&origins)?,
            fixture
        );
        let result = page
            .evaluate_runtime_expression_with_await_async(&script, true)
            .await?;
        let observed: serde_json::Value =
            serde_json::from_str(result["value"].as_str().context("Location origin result")?)?;
        assert_eq!(observed["errors"], serde_json::json!([]));
        assert_eq!(
            observed["observations"].as_array().unwrap().len(),
            expected_observations
        );
        Ok::<_, anyhow::Error>(())
    })
    .await;
    let _ = shutdown.send(());
    server.await??;
    result?
}

#[tokio::test(flavor = "multi_thread")]
async fn location_origin_serializes_the_url_without_changing_document_security() -> Result<()> {
    run_origin_surface_fixture(include_str!("../fixtures/location-url-origin.js"), 15).await
}

#[tokio::test(flavor = "multi_thread")]
async fn popup_storage_getters_enforce_receiver_and_document_origin_checks() -> Result<()> {
    run_origin_surface_fixture(include_str!("../fixtures/popup-storage-origin.js"), 10).await
}
