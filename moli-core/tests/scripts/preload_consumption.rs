use super::*;
use anyhow::Context;
use parking_lot::Mutex;
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::Notify,
    task::{JoinHandle, JoinSet},
};

const SCRIPT: &str = "globalThis.preloadExecuted=(globalThis.preloadExecuted||0)+1;";
const PROBE: &str = include_str!("../fixtures/preload-consumption.js");

#[derive(Default)]
struct Asset {
    count: AtomicUsize,
    started: Notify,
    released: AtomicBool,
    release: Notify,
}

impl Asset {
    async fn wait(&self, release: bool) {
        loop {
            let notified = if release {
                self.release.notified()
            } else {
                self.started.notified()
            };
            tokio::pin!(notified);
            notified.as_mut().enable();
            if if release {
                self.released.load(Ordering::Acquire)
            } else {
                self.count.load(Ordering::Acquire) != 0
            } {
                return;
            }
            notified.await;
        }
    }
}

struct PreloadServer {
    origin: String,
    task: JoinHandle<()>,
}

impl Drop for PreloadServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl PreloadServer {
    async fn spawn() -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let origin = format!("http://{}", listener.local_addr()?);
        let task = tokio::spawn(async move {
            let assets = Arc::new(Mutex::new(HashMap::<String, Arc<Asset>>::new()));
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let Ok((mut stream, _)) = accepted else { break };
                        let assets = assets.clone();
                        connections.spawn(async move {
                            let mut request = Vec::new();
                            while !request.ends_with(b"\r\n\r\n") && request.len() < 16384 {
                                let Ok(byte) = stream.read_u8().await else { return };
                                request.push(byte);
                            }
                            let request = String::from_utf8_lossy(&request);
                            let path = request.split_whitespace().nth(1).unwrap_or("/");
                            let url = url::Url::parse(&format!("http://test{path}")).unwrap();
                            let query: HashMap<_,_> = url.query_pairs().into_owned().collect();
                            let token = query.get("token").cloned().unwrap_or_default();
                            let asset = assets.lock().entry(token).or_default().clone();
                            let mut cache = "no-store".to_owned();
                            let (mime, body) = match url.path() {
                                "/probe-worker.js" => ("text/javascript", include_str!("../fixtures/preload-consumption-worker.js").to_owned()),
                                "/probe-asset" => {
                                    asset.count.fetch_add(1, Ordering::AcqRel);
                                    asset.started.notify_waiters();
                                    asset.wait(true).await;
                                    cache = query.get("cache").cloned().unwrap_or(cache);
                                    if query.get("as").is_some_and(|kind| kind == "image") {
                                        ("image/svg+xml", "<svg xmlns='http://www.w3.org/2000/svg' width='1' height='1'><rect width='1' height='1'/></svg>".to_owned())
                                    } else { ("text/javascript", SCRIPT.to_owned()) }
                                }
                                "/probe-started" | "/probe-release" | "/probe-stats" => {
                                    if url.path() == "/probe-started" { asset.wait(false).await; }
                                    if url.path() == "/probe-release" {
                                        asset.released.store(true, Ordering::Release);
                                        asset.release.notify_waiters();
                                    }
                                    ("application/json", serde_json::json!({"count":asset.count.load(Ordering::Acquire)}).to_string())
                                }
                                _ => ("text/html", "<!doctype html><body>preload consumption".to_owned()),
                            };
                            let origin = request.lines().find_map(|line| line.split_once(':').filter(|(name,_)| name.eq_ignore_ascii_case("Origin")).map(|(_,value)| value.trim())).unwrap_or("*");
                            let response = format!("HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nCache-Control: {cache}\r\nAccess-Control-Allow-Origin: {origin}\r\nAccess-Control-Allow-Credentials: true\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                            let _ = stream.write_all(response.as_bytes()).await;
                        });
                    }
                    _ = connections.join_next(), if !connections.is_empty() => {}
                }
            }
        });
        Ok(Self { origin, task })
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn document_preloads_supply_pending_and_completed_consumers_including_failed_integrity()
-> Result<()> {
    let server = PreloadServer::spawn().await?;
    let mut config = AppConfig::default();
    config.set_optional_resource_fetch_mask(moli_page_types::OptionalResourceFetchMask::ALL);
    let browser = Browser::new(config)?;
    let mut page = browser.fetch(&server.origin).await?;
    let result = tokio::time::timeout(
        Duration::from_secs(30),
        page.evaluate_runtime_expression_with_await_async(
            &format!("({PROBE})().then(JSON.stringify)"),
            true,
        ),
    )
    .await??;
    let value: serde_json::Value = serde_json::from_str(
        result["value"]
            .as_str()
            .context("preload consumption results")?,
    )?;
    assert_eq!(value["basic"]["rows"].as_array().unwrap().len(), 24);
    assert_eq!(value["integrity"]["rows"].as_array().unwrap().len(), 32);
    assert_eq!(value["basic"]["failures"], serde_json::json!([]));
    assert_eq!(value["integrity"]["failures"], serde_json::json!([]));
    assert_eq!(value["basic"]["executions"], 12);
    assert_eq!(value["oneShot"], serde_json::json!([1, 2, 2, 3]));
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn document_preload_consumption_preserves_service_worker_filters_without_redispatch()
-> Result<()> {
    let server = PreloadServer::spawn().await?;
    let mut config = AppConfig::default();
    config.set_optional_resource_fetch_mask(moli_page_types::OptionalResourceFetchMask::ALL);
    let browser = Browser::new(config)?;
    let mut page = browser.fetch(&server.origin).await?;
    let probe = include_str!("../fixtures/preload-consumption-sw.js");
    let result = tokio::time::timeout(
        Duration::from_secs(30),
        page.evaluate_runtime_expression_with_await_async(
            &format!("({probe})().then(JSON.stringify)"),
            true,
        ),
    )
    .await??;
    let value: serde_json::Value = serde_json::from_str(
        result["value"]
            .as_str()
            .context("Service Worker consumption results")?,
    )?;
    assert_eq!(value["rows"].as_array().unwrap().len(), 16);
    assert_eq!(value["failures"], serde_json::json!([]));
    Ok(())
}
