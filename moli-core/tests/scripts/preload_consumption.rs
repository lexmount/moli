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

pub(super) struct PreloadServer {
    pub(super) origin: String,
    task: JoinHandle<()>,
    assets: Arc<Mutex<HashMap<String, Arc<Asset>>>>,
}

impl Drop for PreloadServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl PreloadServer {
    pub(super) async fn spawn() -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let origin = format!("http://{}", listener.local_addr()?);
        let assets = Arc::new(Mutex::new(HashMap::<String, Arc<Asset>>::new()));
        let server_assets = assets.clone();
        let task = tokio::spawn(async move {
            let assets = server_assets;
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
                            let asset_token = if url.path().starts_with("/child-import-") && url.path().ends_with(".css") {
                                format!("{token}:{}", url.path())
                            } else { token.clone() };
                            let asset = assets.lock().entry(asset_token).or_default().clone();
                            let mut cache = "no-store".to_owned();
                            let (mime, body) = match url.path() {
                                "/probe-worker.js" => ("text/javascript", include_str!("../fixtures/preload-consumption-worker.js").to_owned()),
                                "/probe-timing.js" => ("text/javascript", include_str!("../fixtures/preload-resource-timing.js").to_owned()),
                                "/timing-header" => ("text/html", "<!doctype html><body>Header preload timing".to_owned()),
                                "/child-import-parent" => {
                                    let kind = query.get("kind").map(String::as_str).unwrap_or("linked");
                                    let scenario = query.get("scenario").map(String::as_str).unwrap_or("nested");
                                    ("text/html", format!("<!doctype html><iframe src='/child-import-page?kind={kind}&amp;token={token}&amp;scenario={scenario}'></iframe>"))
                                }
                                "/child-import-page" => {
                                    let kind = query.get("kind").map(String::as_str).unwrap_or("linked");
                                    let scenario = query.get("scenario").map(String::as_str).unwrap_or("nested");
                                    let root = format!("/child-import-root.css?token={token}&scenario={scenario}");
                                    let style = if kind == "inline" { format!("<style>@import url('{root}');</style>") }
                                        else { format!("<link rel='stylesheet' href='{root}'>") };
                                    let probe = r#"try { const rootSheet = document.styleSheets[0];
                                        const root = rootSheet.href ? rootSheet : rootSheet.cssRules[0].styleSheet;
                                        const imports = root ? Array.from(root.cssRules).filter(rule => rule.type === 3) : [];
                                        globalThis.parserSheetSnapshot = {root: !!root, branches: imports.length,
                                            children: imports.map(rule => {
                                                const branch = rule.styleSheet;
                                                const leafRule = branch && branch.cssRules[0];
                                                const leaf = leafRule && leafRule.styleSheet;
                                                return {branch: !!branch, leaf: !!leaf,
                                                    owner: !!branch && branch.ownerRule === rule && branch.parentStyleSheet === root,
                                                    leafOwner: !!leaf && leaf.ownerRule === leafRule && leaf.parentStyleSheet === branch,
                                                    text: leaf ? Array.from(leaf.cssRules).find(rule => rule.type === 1)?.cssText ?? null : null};
                                            })}; } catch (error) { globalThis.parserSheetSnapshot = {error: String(error)}; } parent.postMessage({childImportProbe: true, href: location.href}, '*');"#;
                                    ("text/html", format!("<!doctype html>{style}<script>{probe}</script><body>Child imports"))
                                }
                                "/child-import-root.css" | "/child-import-branch.css" | "/child-import-leaf.css" => {
                                    asset.count.fetch_add(1, Ordering::AcqRel);
                                    asset.started.notify_waiters();
                                    let scenario = query.get("scenario").map(String::as_str).unwrap_or("nested");
                                    if scenario == "delayed" && url.path() == "/child-import-branch.css" { asset.wait(true).await; }
                                    let body = match url.path() {
                                        "/child-import-root.css" => format!("@import url('/child-import-branch.css?token={token}&scenario={scenario}'); @import url('/child-import-branch.css?token={token}&scenario={scenario}'); .root {{ color: rgb(1, 2, 3); }}"),
                                        "/child-import-branch.css" => format!("@import url('/child-import-leaf.css?token={token}&scenario={scenario}'); .branch {{ color: rgb(4, 5, 6); }}"),
                                        _ if scenario == "cycle" => format!("@import url('/child-import-root.css?token={token}&scenario={scenario}'); .leaf {{ color: rgb(7, 8, 9); }}"),
                                        _ => ".leaf { color: rgb(7, 8, 9); }".to_owned(),
                                    };
                                    ("text/css", body)
                                }

                                "/probe-asset" | "/probe-font-final" => {
                                    asset.count.fetch_add(1, Ordering::AcqRel);
                                    asset.started.notify_waiters();
                                    if !query.contains_key("auto") { asset.wait(true).await; }
                                    cache = query.get("cache").cloned().unwrap_or(cache);
                                    if query.get("as").is_some_and(|kind| kind == "image") {
                                        ("image/svg+xml", "<svg xmlns='http://www.w3.org/2000/svg' width='1' height='1'><rect width='1' height='1'/></svg>".to_owned())
                                    } else if query.get("as").is_some_and(|kind| kind == "style") {
                                        ("text/css", "body { --preload-value: loaded; }".to_owned())
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
                            let font = matches!(url.path(), "/probe-asset" | "/probe-font-final") && query.get("as").is_some_and(|kind| kind == "font");
                            let body = if font {
                                if query.contains_key("invalid") { b"\0\x01\0\0".to_vec() }
                                else { include_bytes!("../../../moli-layout/tests/fixtures/moli-ahem.ttf").to_vec() }
                            } else { body.into_bytes() };
                            let mime = if font { "font/ttf" } else { mime };
                            let status = if url.path() == "/child-import-leaf.css" && query.get("scenario").is_some_and(|value| value == "invalid") {
                                "404"
                            } else { query.get("status").map(String::as_str).unwrap_or("200") };
                            let location = query.get("redirect").map(|url| format!("Location: {url}\r\n")).unwrap_or_default();
                            let origin = request.lines().find_map(|line| line.split_once(':').filter(|(name,_)| name.eq_ignore_ascii_case("Origin")).map(|(_,value)| value.trim())).unwrap_or("*");
                            let cors = if query.get("cors").is_some_and(|value| value == "none") { String::new() }
                                else { format!("Access-Control-Allow-Origin: {origin}\r\nAccess-Control-Allow-Credentials: true\r\n") };
                            let policy = query.get("reportPolicy").map(|policy| format!("Content-Security-Policy-Report-Only: {policy}\r\n")).unwrap_or_default();
                            let link = if url.path() == "/timing-header" {
                                let kind = query.get("as").map(String::as_str).unwrap_or("script");
                                let source = query.get("source").map(String::as_str).unwrap_or("network");
                                let path = if source == "network" { "/probe-asset" } else { "/probe-worker-asset" };
                                format!("Link: <{path}?token={token}&as={kind}&source={source}&auto=1>; rel=preload; as={kind}\r\n")
                            } else { String::new() };
                            let response = format!("HTTP/1.1 {status} OK\r\nContent-Type: {mime}\r\nCache-Control: {cache}\r\n{cors}{location}{policy}{link}Content-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                            let _ = stream.write_all(response.as_bytes()).await;
                            let _ = stream.write_all(&body).await;
                        });
                    }
                    _ = connections.join_next(), if !connections.is_empty() => {}
                }
            }
        });
        Ok(Self {
            origin,
            task,
            assets,
        })
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn preload_resource_timings_retain_the_initiator_through_consumption() -> Result<()> {
    let server = PreloadServer::spawn().await?;
    let mut config = AppConfig::default();
    config.set_optional_resource_fetch_mask(moli_page_types::OptionalResourceFetchMask::ALL);
    let browser = Browser::new(config)?;
    for realm in ["main", "child"] {
        let mut page = browser.fetch(&server.origin).await?;
        let expression = format!(
            r#"(async () => {{
                let target = window;
                if ({realm:?} === 'child') {{
                    const frame = document.createElement('iframe');
                    frame.src = '/?preload-timing-child';
                    await new Promise(resolve => {{ frame.onload = resolve; document.body.append(frame); }});
                    target = frame.contentWindow;
                }}
                const script = target.document.createElement('script');
                script.src = '/probe-timing.js?realm=' + {realm:?};
                await new Promise((resolve, reject) => {{
                    script.onload = resolve; script.onerror = reject;
                    target.document.body.append(script);
                }});
                return JSON.stringify(await target.preloadResourceTimingResult);
            }})()"#,
        );
        let result = tokio::time::timeout(
            Duration::from_secs(30),
            page.evaluate_runtime_expression_with_await_async(&expression, true),
        )
        .await??;
        let value: serde_json::Value = serde_json::from_str(
            result["value"]
                .as_str()
                .context("preload resource timing results")?,
        )?;
        assert_eq!(
            value["rows"].as_array().unwrap().len(),
            20,
            "{realm}: {value}"
        );
        assert_eq!(
            value["failures"],
            serde_json::json!([]),
            "preloads must retain their own initiating source and one network timing: {realm}: {value}"
        );
        assert_eq!(
            value["swRows"].as_array().unwrap().len(),
            5,
            "{realm}: {value}"
        );
        assert_eq!(
            value["swFailures"],
            serde_json::json!([]),
            "SW responses must hide the source while network fallback retains it: {realm}: {value}"
        );
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn response_header_preload_timings_retain_the_request_document() -> Result<()> {
    let server = PreloadServer::spawn().await?;
    let mut config = AppConfig::default();
    config.set_optional_resource_fetch_mask(moli_page_types::OptionalResourceFetchMask::ALL);
    let browser = Browser::new(config)?;
    let mut setup = browser.fetch(&server.origin).await?;
    tokio::time::timeout(Duration::from_secs(30), setup.evaluate_runtime_expression_with_await_async(
        r#"(async () => {
            const controlled = navigator.serviceWorker.controller ? Promise.resolve() :
                new Promise(resolve => navigator.serviceWorker.addEventListener('controllerchange', resolve, {once: true}));
            await navigator.serviceWorker.register('/probe-worker.js');
            await navigator.serviceWorker.ready;
            await controlled;
        })()"#, true)).await??;
    for source in ["network", "default", "basic"] {
        for kind in ["script", "style"] {
            let document_url = format!(
                "{}/timing-header?token=header-{source}-{kind}&as={kind}&source={source}",
                server.origin
            );
            let mut page = browser.fetch(&document_url).await?;
            let result = tokio::time::timeout(
            Duration::from_secs(30),
            page.evaluate_runtime_expression_with_await_async(
                r#"new Promise(resolve => {
                    const read = entries => {
                        const entry = entries.find(entry => entry.name.includes('/probe-asset?') || entry.name.includes('/probe-worker-asset?'));
                        if (!entry) return false;
                        resolve(JSON.stringify({type: entry.initiatorType, source: entry.initiatorUrl,
                            encoded: entry.encodedBodySize, end: entry.responseEnd}));
                        return true;
                    };
                    if (!read(performance.getEntriesByType('resource'))) {
                        const observer = new PerformanceObserver(list => {
                            if (read(list.getEntries())) observer.disconnect();
                        });
                        observer.observe({type: 'resource', buffered: true});
                    }
                })"#,
                true,
            ),
        )
        .await??;
            let value: serde_json::Value = serde_json::from_str(
                result["value"]
                    .as_str()
                    .context("header preload resource timing")?,
            )?;
            assert_eq!(value["type"], "link", "{kind}: {value}");
            assert_eq!(
                value["source"],
                if source == "network" {
                    &document_url
                } else {
                    ""
                },
                "header preload must retain its Document only for network fallback: {source}/{kind}: {value}"
            );
            assert!(
                value["encoded"].as_u64().is_some_and(|bytes| bytes > 0),
                "{value}"
            );
            assert!(
                value["end"].as_f64().is_some_and(|end| end > 0.0),
                "{value}"
            );
            assert_eq!(
                server
                    .assets
                    .lock()
                    .get(&format!("header-{source}-{kind}"))
                    .map(|asset| asset.count.load(Ordering::Acquire)),
                Some(if source == "default" { 0 } else { 1 }),
                "header preload must count actual network requests: {source}/{kind}"
            );
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn child_parser_stylesheet_imports_finish_before_following_scripts() -> Result<()> {
    let server = PreloadServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut failures = Vec::new();
    for kind in ["linked", "inline"] {
        for scenario in ["nested", "cycle", "invalid"] {
            let token = format!("parser-{kind}-{scenario}");
            let mut page = tokio::time::timeout(
                Duration::from_secs(30),
                browser.fetch(&format!(
                    "{}/child-import-parent?kind={kind}&token={token}&scenario={scenario}",
                    server.origin
                )),
            )
            .await??;
            let result = page.evaluate_runtime_expression_with_await_async(
            "JSON.stringify(document.querySelector('iframe').contentWindow.parserSheetSnapshot ?? {error: 'parser script did not publish a snapshot'})", true).await?;
            let value: serde_json::Value = serde_json::from_str(
                result["value"]
                    .as_str()
                    .context("child parser import snapshot")?,
            )?;
            let requests = ["root", "branch", "leaf"].map(|name| {
                server
                    .assets
                    .lock()
                    .get(&format!("{token}:/child-import-{name}.css"))
                    .map(|asset| asset.count.load(Ordering::Acquire))
                    .unwrap_or(0)
            });
            let children = value["children"].as_array().cloned().unwrap_or_default();
            let expected_text = if scenario == "invalid" {
                serde_json::Value::Null
            } else {
                serde_json::json!(".leaf { color: rgb(7, 8, 9); }")
            };
            if requests != [1, 1, 1]
                || value["root"] != true
                || value["branches"] != 2
                || children.len() != 2
                || children.iter().any(|row| {
                    row["branch"] != true
                        || row["leaf"] != true
                        || row["owner"] != true
                        || row["leafOwner"] != true
                        || row["text"] != expected_text
                })
            {
                failures.push(serde_json::json!({"kind":kind,"scenario":scenario,"requests":requests,"snapshot":value}));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "parser imports must be fetched once and installed before child scripts: {}",
        serde_json::to_string(&failures)?
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn child_parser_imports_hold_scripts_and_frame_load_until_the_graph_finishes() -> Result<()> {
    let server = PreloadServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let probe = include_str!("../fixtures/child-stylesheet-imports-pending.js");
    for kind in ["linked", "inline"] {
        let mut page = browser.fetch(&server.origin).await?;
        let value = tokio::time::timeout(
            Duration::from_secs(30),
            page.evaluate_runtime_expression_with_await_async(
                &format!("({probe})({kind:?}).then(JSON.stringify)"),
                true,
            ),
        )
        .await??;
        let value: serde_json::Value =
            serde_json::from_str(value["value"].as_str().context("pending import result")?)?;
        assert_eq!(
            value["before"],
            serde_json::json!({"loaded":false,"parserMessages":0}),
            "{kind}: {value}"
        );
        assert_eq!(value["loaded"], true, "{kind}: {value}");
        assert_eq!(value["parserMessages"], 1, "{kind}: {value}");
        assert_eq!(value["snapshot"]["branches"], 2, "{kind}: {value}");
        assert!(
            value["snapshot"]["children"]
                .as_array()
                .context("loaded imports")?
                .iter()
                .all(|row| row["leaf"] == true && row["owner"] == true && row["leafOwner"] == true),
            "{kind}: {value}"
        );
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn stylesheet_preloads_supply_pending_and_completed_consumers_and_integrity_failures()
-> Result<()> {
    let server = PreloadServer::spawn().await?;
    let mut config = AppConfig::default();
    config.set_optional_resource_fetch_mask(moli_page_types::OptionalResourceFetchMask::ALL);
    let browser = Browser::new(config)?;
    let mut page = browser.fetch(&server.origin).await?;
    let probe = include_str!("../fixtures/preload-consumption-style.js");
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
            .context("stylesheet preload consumption results")?,
    )?;
    assert_eq!(value["rows"].as_array().unwrap().len(), 72);
    assert_eq!(value["failures"], serde_json::json!([]));
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn stylesheet_preload_consumption_preserves_service_worker_filters_without_redispatch()
-> Result<()> {
    let server = PreloadServer::spawn().await?;
    let mut config = AppConfig::default();
    config.set_optional_resource_fetch_mask(moli_page_types::OptionalResourceFetchMask::ALL);
    let browser = Browser::new(config)?;
    let mut page = browser.fetch(&server.origin).await?;
    let probe = include_str!("../fixtures/preload-consumption-style-sw.js");
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
            .context("Service Worker stylesheet preload results")?,
    )?;
    assert_eq!(value["rows"].as_array().unwrap().len(), 24);
    assert_eq!(value["failures"], serde_json::json!([]));
    Ok(())
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
    assert_eq!(value["integrity"]["rows"].as_array().unwrap().len(), 40);
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
