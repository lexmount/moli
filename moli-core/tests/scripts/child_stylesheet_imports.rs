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

struct ChildImportServer {
    origin: String,
    task: JoinHandle<()>,
    assets: Arc<Mutex<HashMap<String, Arc<Asset>>>>,
}

impl Drop for ChildImportServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl ChildImportServer {
    async fn spawn() -> Result<Self> {
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
                            let (mime, body) = match url.path() {
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
                            let status = if url.path() == "/child-import-leaf.css"
                                && query.get("scenario").is_some_and(|value| value == "invalid") {
                                "404 Not Found"
                            } else { "200 OK" };
                            let response = format!("HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                            let _ = stream.write_all(response.as_bytes()).await;
                            let _ = stream.write_all(body.as_bytes()).await;
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
async fn child_parser_stylesheet_imports_finish_before_following_scripts() -> Result<()> {
    let server = ChildImportServer::spawn().await?;
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
    let server = ChildImportServer::spawn().await?;
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
