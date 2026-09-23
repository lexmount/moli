use super::preload_as::evaluate_preload_probe;
use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[tokio::test(flavor = "multi_thread")]
async fn preload_crossorigin_controls_fetch_mode_in_top_and_child_documents() -> Result<()> {
    let server = FixtureServer::spawn().await?;
    // A separate port makes every asset cross-origin without relying on DNS.
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let resource_url = format!("http://{}/resource", listener.local_addr()?);
    let (shutdown, mut shutting_down) = tokio::sync::oneshot::channel::<()>();
    let assets = tokio::spawn(async move {
        let mut requests = tokio::task::JoinSet::new();
        loop {
            tokio::select! {
                _ = &mut shutting_down => break,
                accepted = listener.accept() => {
                    let (mut socket, _) = accepted?;
                    requests.spawn(async move {
                        let mut bytes = Vec::new();
                        loop {
                            let mut buffer = [0; 4096];
                            let size = socket.read(&mut buffer).await?;
                            if size == 0 { return Ok::<_, anyhow::Error>(()); }
                            bytes.extend_from_slice(&buffer[..size]);
                            if bytes.windows(4).any(|part| part == b"\r\n\r\n") { break; }
                        }
                        let request = String::from_utf8(bytes)?;
                        let path = request.split_whitespace().nth(1).context("request path")?;
                        let url = url::Url::parse(&format!("http://fixture{path}"))?;
                        let parameters: std::collections::HashMap<_, _> = url.query_pairs().collect();
                        let origin = request.lines().find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("origin").then(|| value.trim())
                        });
                        let cors = match parameters.get("allow").map(|value| value.as_ref()) {
                            Some("wildcard") => "Access-Control-Allow-Origin: *\r\n".to_owned(),
                            Some("credentials") => format!(
                                "Access-Control-Allow-Origin: {}\r\nAccess-Control-Allow-Credentials: true\r\n",
                                origin.unwrap_or("*")
                            ),
                            _ => String::new(),
                        };
                        let (mime, body) = match parameters.get("as").map(|value| value.as_ref()) {
                            Some("script") => ("text/javascript", "/* preloaded, never executed */"),
                            Some("style") => ("text/css", "body { color: green; }"),
                            Some("image") => ("image/svg+xml", "<svg xmlns='http://www.w3.org/2000/svg' width='1' height='1'/>") ,
                            Some("font") => ("font/ttf", "preload does not decode a font"),
                            Some("track") => ("text/vtt", "WEBVTT\n\n"),
                            _ => ("text/plain", "preload"),
                        };
                        socket.write_all(format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nCache-Control: no-store\r\n{cors}Connection: close\r\n\r\n{body}", body.len()
                        ).as_bytes()).await?;
                        Ok(())
                    });
                }
                Some(result) = requests.join_next(), if !requests.is_empty() => { result??; }
            }
        }
        requests.shutdown().await;
        Ok::<_, anyhow::Error>(())
    });
    let mut config = AppConfig::default();
    config.set_optional_resource_fetch_mask(moli_page_types::OptionalResourceFetchMask::ALL);
    let browser = Browser::new(config)?;
    let mut page = browser.fetch(&server.url("/static")).await?;
    let fixture = include_str!("../fixtures/preload-crossorigin.js");
    let observed = evaluate_preload_probe(
        &mut page,
        &format!(
            "(globalThis.preloadCrossoriginResource = {}, {fixture})",
            serde_json::to_string(&resource_url)?
        ),
    )
    .await?;
    assert_eq!(observed["observations"].as_array().unwrap().len(), 75);
    assert_eq!(observed["failures"], serde_json::json!([]), "top dynamic");
    let markup = observed["parserMarkup"].as_str().context("parser markup")?;
    let mut url = url::Url::parse(&server.url("/compat/child-dynamic-markup-document"))?;
    url.query_pairs_mut().append_pair("markup", markup);
    page = browser.fetch(url.as_str()).await?;
    let observed = evaluate_preload_probe(&mut page, fixture).await?;
    assert_eq!(observed["context"], "parser");
    assert_eq!(observed["observations"].as_array().unwrap().len(), 75);
    assert_eq!(observed["failures"], serde_json::json!([]), "top parser");
    let observed = evaluate_preload_probe(
        &mut page,
        &format!(
            r#"(async () => {{
        const frame = document.createElement('iframe');
        frame.srcdoc = {markup};
        await new Promise(resolve => {{ frame.onload = resolve; document.body.append(frame); }});
        const result = await frame.contentWindow.eval({fixture});
        frame.remove();
        return result;
    }})()"#,
            markup = serde_json::to_string(markup)?,
            fixture = serde_json::to_string(fixture)?
        ),
    )
    .await?;
    assert_eq!(observed["context"], "parser");
    assert_eq!(observed["observations"].as_array().unwrap().len(), 75);
    assert_eq!(observed["failures"], serde_json::json!([]), "child parser");
    let _ = shutdown.send(());
    assets.await??;
    server.shutdown().await;
    Ok(())
}
