use super::*;
use std::{collections::HashMap, path::PathBuf, sync::Arc};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

struct CacheDirectory(PathBuf);

impl Drop for CacheDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn resource_timing_distinguishes_network_local_and_revalidated_responses() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let cache_dir = CacheDirectory(std::env::temp_dir().join(format!(
        "moli-resource-timing-{}-{}",
        std::process::id(),
        addr.port()
    )));
    let requests = Arc::new(parking_lot::Mutex::new(HashMap::<
        String,
        Vec<Option<String>>,
    >::new()));
    let (shutdown, mut shutting_down) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        let mut connections = tokio::task::JoinSet::new();
        loop {
            tokio::select! {
                _ = &mut shutting_down => break,
                accepted = listener.accept() => {
                    let (mut socket, _) = accepted?;
                    let requests = Arc::clone(&requests);
                    connections.spawn(async move {
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
                        let params: HashMap<_, _> = url.query_pairs().collect();
                        let mode = params.get("mode").map(|value| value.as_ref()).unwrap_or("");
                        let context = params.get("context").map(|value| value.as_ref()).unwrap_or("");
                        let key = format!("{mode}:{context}");
                        let conditional = request.lines().find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("if-none-match").then(|| value.trim().to_owned())
                        });
                        let mut headers = String::new();
                        let (status, mime, body) = match url.path() {
                            "/probe-cache" => {
                                requests.lock().entry(key).or_default().push(conditional.clone());
                                let validated = mode == "revalidate" && conditional.as_deref() == Some("\"v1\"");
                                let cache = if mode == "fresh" || validated { "max-age=600" }
                                    else if mode == "revalidate" { "no-cache" }
                                    else { "no-store" };
                                headers = format!("ETag: \"v1\"\r\nCache-Control: {cache}\r\n");
                                if mode == "padding" {
                                    let padding: usize = params.get("padding").context("padding")?.parse()?;
                                    headers.push_str(&format!("X-Padding: {}\r\n", "x".repeat(padding)));
                                }
                                if validated { ("304 Not Modified", "text/plain", String::new()) }
                                else { ("200 OK", "text/plain", "cached".to_owned()) }
                            }
                            "/probe-cache-stats" => {
                                headers.push_str("Cache-Control: no-store\r\n");
                                let records = requests.lock().get(&key).cloned().unwrap_or_default();
                                let records: Vec<_> = records.into_iter().map(|conditional| serde_json::json!({"conditional":conditional})).collect();
                                ("200 OK", "application/json", serde_json::to_string(&records)?)
                            }
                            _ => ("200 OK", "text/html", "<!doctype html><body>cache timing".to_owned()),
                        };
                        socket.write_all(format!(
                            "HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\n{headers}Connection: close\r\n\r\n{body}", body.len()
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
    let mut config = AppConfig::default();
    config
        .fetch_mut()
        .set_http_cache_dir(Some(cache_dir.0.display().to_string()));
    let browser = Browser::new(config)?;
    let mut page = browser.fetch(&format!("http://{addr}/")).await?;
    let fixture = include_str!("../fixtures/cache-resource-timing.js");
    for context in ["top", "child"] {
        let script = if context == "top" {
            fixture.to_owned()
        } else {
            format!(
                r#"(async () => {{
                const frame = document.createElement('iframe');
                frame.srcdoc = '<!doctype html><body>child';
                await new Promise(resolve => {{ frame.onload=resolve; document.body.append(frame); }});
                frame.contentWindow.cacheTimingContext = 'child';
                const result = await frame.contentWindow.eval({});
                frame.remove();
                return result;
            }})()"#,
                serde_json::to_string(fixture)?
            )
        };
        let result = tokio::time::timeout(
            Duration::from_secs(15),
            page.evaluate_runtime_expression_with_await_async(
                &format!("({script}).then(JSON.stringify)"),
                true,
            ),
        )
        .await??;
        let observed: serde_json::Value =
            serde_json::from_str(result["value"].as_str().context("timing observations")?)?;
        let expected = [
            ("fresh", [306, 0, 0]),
            ("revalidate", [306, 300, 0]),
            ("no-store", [306, 306, 306]),
        ];
        let mut expected_rows = Vec::new();
        for (mode, transfers) in expected {
            let observations: Vec<_> = transfers
                .into_iter()
                .enumerate()
                .map(|(index, transfer)| {
                    serde_json::json!({
                        "body":"cached", "count":index + 1, "transfer":transfer, "encoded":6, "decoded":6, "status":200
                    })
                })
                .collect();
            let stats = match mode {
                "fresh" => serde_json::json!([{"conditional":null}]),
                "revalidate" => serde_json::json!([{"conditional":null},{"conditional":"\"v1\""}]),
                _ => {
                    serde_json::json!([{"conditional":null},{"conditional":null},{"conditional":null}])
                }
            };
            expected_rows
                .push(serde_json::json!({"mode":mode,"observations":observations,"stats":stats}));
        }
        assert_eq!(
            observed,
            serde_json::json!({"result":expected_rows,"headers":[
                {"padding":0,"transfer":306,"encoded":6},
                {"padding":4096,"transfer":306,"encoded":6}
            ]}),
            "{context}"
        );
    }
    let _ = shutdown.send(());
    server.await??;
    Ok(())
}
