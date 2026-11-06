use super::*;
use parking_lot::Mutex;
use std::sync::Arc;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::{JoinHandle, JoinSet},
};

struct ModuleInitiatorServer {
    origin: String,
    requests: Arc<Mutex<Vec<String>>>,
    task: JoinHandle<()>,
}

impl Drop for ModuleInitiatorServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl ModuleInitiatorServer {
    async fn spawn() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = requests.clone();
        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let captured = captured.clone();
                connections.spawn(async move {
                    let mut request = Vec::new();
                    while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                        let mut buffer = [0; 2048];
                        let count = socket.read(&mut buffer).await.unwrap();
                        if count == 0 { return; }
                        request.extend_from_slice(&buffer[..count]);
                    }
                    let request = String::from_utf8(request).unwrap();
                    let path = request.split_whitespace().nth(1).unwrap();
                    captured.lock().push(path.to_owned());
                    let (mime, body) = match path {
                        "/parser" => ("text/html", r#"<!doctype html><head><base href='/wrong/'>
                            <script type='module' src='/root.js'></script>
                            <script type='module'>import '/inline-leaf.js'; globalThis.inlineReady=true;</script>
                            </head><body>module initiator"#),
                        "/root.js" => ("text/javascript", "import '/leaf.js'; globalThis.rootReady=true;"),
                        "/leaf.js" | "/inline-leaf.js" | "/nested.js" => ("text/javascript", "export const value=1;"),
                        "/dynamic" => ("text/html", "<!doctype html><body><script src='/caller.js'></script><script src='/later.js'></script>"),
                        "/caller.js" => ("text/javascript", r#"Promise.resolve().then(() => {
                            const script=document.createElement('script'); script.type='module'; script.src='/dynamic.js';
                            document.body.appendChild(script);
                            import('/imported.js').then(() => globalThis.importReady=true);
                        });"#),
                        "/later.js" => ("text/javascript", "globalThis.laterRan=true;"),
                        "/dynamic.js" => ("text/javascript", "globalThis.dynamicReady=true;"),
                        "/imported.js" => ("text/javascript", "import '/nested.js'; export const value=1;"),
                        "/frame" => ("text/html", "<!doctype html><body><iframe src='/dynamic'></iframe>"),
                        "/parser-frame" => ("text/html", "<!doctype html><body><iframe src='/parser'></iframe>"),
                        "/preload-frame" => ("text/html", "<!doctype html><body><iframe src='/preload'></iframe>"),
                        "/preload-dynamic-frame" => ("text/html", "<!doctype html><body><iframe src='/preload-dynamic'></iframe>"),
                        "/preload" => ("text/html", "<!doctype html><head><base href='/wrong/'><link rel='modulepreload' href='/preloaded.js'></head><body><script type='module' src='/preloaded.js'></script>"),
                        "/preload-dynamic" => ("text/html", "<!doctype html><head><base href='/wrong/'></head><body><script src='/preload-caller.js'></script><script src='/later.js'></script>"),
                        "/preload-caller.js" => ("text/javascript", r#"Promise.resolve().then(() => {
                            const link=document.createElement('link'); link.rel='modulepreload'; link.href='/preloaded.js';
                            link.onload=() => {const script=document.createElement('script'); script.type='module'; script.src=link.href; document.body.appendChild(script);};
                            document.head.appendChild(link);
                        });"#),
                        "/preloaded.js" => ("text/javascript", "globalThis.preloadedReady=true;"),
                        "/sw-setup" => ("text/html", r#"<!doctype html><body><script>
                            navigator.serviceWorker.register('/sw.js', {scope:'/'}).then(async registration => {
                                const worker=registration.installing||registration.waiting||registration.active;
                                if (worker.state!=='activated') await new Promise(resolve=>worker.addEventListener('statechange',()=>{if(worker.state==='activated')resolve();}));
                                globalThis.swReady=true;
                            });
                        </script>"#),
                        "/sw.js" => ("text/javascript", r#"addEventListener('fetch', event => {
                            if (new URL(event.request.url).pathname==='/sw-module.js') {
                                event.respondWith(new Response('globalThis.swModuleReady=true;', {headers:{'Content-Type':'text/javascript'}}));
                            }
                        });"#),
                        "/sw-page" => ("text/html", "<!doctype html><body><script type='module' src='/sw-module.js'></script>"),
                        "/sw-page-dynamic" => ("text/html", "<!doctype html><body><script>const script=document.createElement('script');script.type='module';script.src='/sw-module.js';document.body.appendChild(script);</script>"),
                        _ => panic!("unexpected module fixture request: {path}"),
                    };
                    if mime == "text/javascript" && path != "/caller.js" && path != "/later.js" && path != "/preload-caller.js" {
                        // Publication must use the captured source after another script runs.
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                    let response = format!("HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                    let _ = socket.write_all(response.as_bytes()).await;
                });
                while let Some(result) = connections.try_join_next() {
                    result.unwrap();
                }
            }
        });
        Self {
            origin,
            requests,
            task,
        }
    }
}

async fn initiators(page: &mut moli_core::page::Page, window: &str) -> Result<serde_json::Value> {
    let result = page.evaluate_runtime_expression_by_value_async(&format!(
        "JSON.stringify({window}.performance.getEntriesByType('resource').filter(e=>new URL(e.name).pathname.endsWith('.js')).map(e=>({{path:new URL(e.name).pathname,initiator:e.initiatorUrl,type:e.initiatorType,json:e.toJSON().initiatorUrl,transfer:e.transferSize,encoded:e.encodedBodySize,status:e.responseStatus,delivery:e.deliveryType}})))"
    )).await?;
    Ok(serde_json::from_str(result["value"].as_str().unwrap())?)
}

fn assert_initiator(entries: &serde_json::Value, path: &str, source: &str) {
    let matches: Vec<_> = entries
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["path"] == path)
        .collect();
    assert_eq!(matches.len(), 1, "one producer entry for {path}: {entries}");
    assert_eq!(matches[0]["type"], "script", "{entries}");
    assert_eq!(matches[0]["initiator"], source, "{entries}");
    assert_eq!(matches[0]["json"], source, "{entries}");
}

#[tokio::test(flavor = "multi_thread")]
async fn parser_modules_attribute_roots_and_static_imports_to_their_sources() -> Result<()> {
    for child in [false, true] {
        let server = ModuleInitiatorServer::spawn().await;
        let browser = Browser::new(AppConfig::default())?;
        let document = if child { "/parser-frame" } else { "/parser" };
        let window = if child {
            "document.querySelector('iframe').contentWindow"
        } else {
            "window"
        };
        let mut page = browser
            .fetch(&format!("{}{document}", server.origin))
            .await?;
        browser
            .wait_for_script_truthy(
                &mut page,
                &format!("(()=>{{const w={window};return !!(w.rootReady&&w.inlineReady)}})()"),
                Duration::from_secs(10),
            )
            .await?;
        let entries = initiators(&mut page, window).await?;
        for (path, source) in [
            ("/root.js", "/parser"),
            ("/leaf.js", "/root.js"),
            ("/inline-leaf.js", "/wrong/"),
        ] {
            assert_initiator(&entries, path, &format!("{}{source}", server.origin));
        }
        if child {
            assert_eq!(
                initiators(&mut page, "window").await?,
                serde_json::json!([])
            );
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn dynamic_modules_keep_the_triggering_script_in_main_and_child_documents() -> Result<()> {
    for child in [false, true] {
        let server = ModuleInitiatorServer::spawn().await;
        let browser = Browser::new(AppConfig::default())?;
        let mut page = browser
            .fetch(&format!(
                "{}{}",
                server.origin,
                if child { "/frame" } else { "/dynamic" }
            ))
            .await?;
        let window = if child {
            "document.querySelector('iframe').contentWindow"
        } else {
            "window"
        };
        browser.wait_for_script_truthy(&mut page, &format!("(()=>{{const w={window};return !!(w.dynamicReady&&w.importReady&&w.laterRan)}})()"), Duration::from_secs(10)).await?;
        let entries = initiators(&mut page, window).await?;

        for (path, source) in [
            ("/dynamic.js", "/caller.js"),
            ("/imported.js", "/caller.js"),
            ("/nested.js", "/imported.js"),
        ] {
            assert_initiator(&entries, path, &format!("{}{source}", server.origin));
        }
        if child {
            let parent = initiators(&mut page, "window").await?;
            assert_eq!(
                parent,
                serde_json::json!([]),
                "child timings belong to the child Document"
            );
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn modulepreload_keeps_its_producer_initiator_when_consumed() -> Result<()> {
    for child in [false, true] {
        for dynamic in [false, true] {
            let server = ModuleInitiatorServer::spawn().await;
            let browser = Browser::new(AppConfig::default())?;
            let document = if dynamic {
                "/preload-dynamic"
            } else {
                "/preload"
            };
            let source = if dynamic {
                "/preload-caller.js"
            } else {
                document
            };
            let target = if child {
                if dynamic {
                    "/preload-dynamic-frame"
                } else {
                    "/preload-frame"
                }
            } else {
                document
            };
            let window = if child {
                "document.querySelector('iframe').contentWindow"
            } else {
                "window"
            };
            let mut page = browser.fetch(&format!("{}{target}", server.origin)).await?;
            browser
                .wait_for_script_truthy(
                    &mut page,
                    &format!("Boolean({window}.preloadedReady)"),
                    Duration::from_secs(10),
                )
                .await?;
            let entries = initiators(&mut page, window).await?;
            // modulepreload uses the script fetch algorithm; its consumer has no second fetch.
            assert_initiator(
                &entries,
                "/preloaded.js",
                &format!("{}{source}", server.origin),
            );
            assert_eq!(
                server
                    .requests
                    .lock()
                    .iter()
                    .filter(|p| *p == "/preloaded.js")
                    .count(),
                1
            );
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn service_worker_module_responses_do_not_report_a_network_initiator() -> Result<()> {
    let server = ModuleInitiatorServer::spawn().await;
    let browser = Browser::new(AppConfig::default())?;
    let mut setup = browser
        .fetch(&format!("{}/sw-setup", server.origin))
        .await?;
    browser
        .wait_for_script_truthy(
            &mut setup,
            "Boolean(globalThis.swReady)",
            Duration::from_secs(10),
        )
        .await?;
    for path in ["/sw-page", "/sw-page-dynamic"] {
        let mut page = browser.fetch(&format!("{}{path}", server.origin)).await?;
        browser
            .wait_for_script_truthy(
                &mut page,
                "Boolean(globalThis.swModuleReady)",
                Duration::from_secs(10),
            )
            .await?;
        let entries = initiators(&mut page, "window").await?;
        assert_initiator(&entries, "/sw-module.js", "");
    }
    assert!(!server.requests.lock().iter().any(|p| p == "/sw-module.js"));
    Ok(())
}
