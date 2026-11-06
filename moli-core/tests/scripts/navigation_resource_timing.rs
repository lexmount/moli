use super::*;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::{JoinHandle, JoinSet},
};

struct NavigationTimingServer {
    origin: String,
    task: JoinHandle<()>,
    prefetched_requests: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl Drop for NavigationTimingServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn epoch_millis() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
        * 1000.0
}

const STATE_WAITER: &str = r#"
async function waitState(worker, state) {
    if (worker.state === state) return;
    await new Promise(resolve => {
        const changed = () => {
            if (worker.state === state) {
                worker.removeEventListener('statechange', changed); resolve();
            }
        };
        worker.addEventListener('statechange', changed); changed();
    });
}
function snapshot(win) {
    const nav = win.performance.getEntriesByType('navigation')[0];
    return {timeOrigin: win.performance.timeOrigin, nav: nav.toJSON(),
        swTimes: win.swTimes || null};
}
"#;

const SERVICE_WORKER: &str = r#"
const times = {};
const epoch = () => performance.timeOrigin + performance.now();
addEventListener('activate', event => {
    times.activateStart = epoch();
    event.waitUntil(new Promise(resolve => setTimeout(resolve, 500)).then(() => {
        times.activateEnd = epoch();
    }));
});
addEventListener('fetch', event => {
    times.fetchEvent = epoch();
    event.respondWith(new Response('<!doctype html><body>controlled<script>globalThis.swTimes=' +
        JSON.stringify(times) + '</script>', {headers:{'Content-Type':'text/html'}}));
});
"#;

impl NavigationTimingServer {
    async fn spawn() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let stream_observed = std::sync::Arc::new(tokio::sync::Notify::new());
        let prefetched_requests = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let request_count = prefetched_requests.clone();
        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let stream_observed = stream_observed.clone();
                let request_count = request_count.clone();
                connections.spawn(async move {
                    let mut request = Vec::new();
                    while !request.windows(4).any(|b| b == b"\r\n\r\n") {
                        let mut buffer = [0; 2048];
                        let count = socket.read(&mut buffer).await.unwrap();
                        if count == 0 { return; }
                        request.extend_from_slice(&buffer[..count]);
                    }
                    let request = String::from_utf8(request).unwrap();
                    let path = request.split_whitespace().nth(1).unwrap();
                    if let Some(target) = path.strip_prefix("/redirect?to=") {
                        let head = format!("HTTP/1.1 302 Found\r\nLocation: {target}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                        socket.write_all(head.as_bytes()).await.unwrap();
                        return;
                    }
                    if path == "/stream" {
                        let first = "<!doctype html><body><script>globalThis.savedNav=performance.getEntriesByType('navigation')[0];globalThis.earlyNav=savedNav.toJSON();fetch('/stream-observed');</script>";
                        let last = "<p>stream completed</p>";
                        let head = format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", first.len()+last.len());
                        socket.write_all(head.as_bytes()).await.unwrap();
                        socket.write_all(first.as_bytes()).await.unwrap();
                        tokio::time::timeout(Duration::from_secs(10), stream_observed.notified()).await.unwrap();
                        socket.write_all(last.as_bytes()).await.unwrap();
                        return;
                    }
                    if path == "/stream-observed" {
                        stream_observed.notify_one();
                    }
                    if path == "/prefetched" {
                        request_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    }
                    let (mime, body) = match path {
                        "/prefetch" => ("text/html", format!(r#"<!doctype html><body><script>
                            {STATE_WAITER}
                            (async () => {{
                                const frame=document.createElement('iframe');
                                await new Promise(resolve => {{frame.onload=resolve;frame.src='/empty';document.body.appendChild(frame);}});
                                const doc=frame.contentDocument;
                                await new Promise((resolve,reject) => {{
                                    const link=doc.createElement('link');link.rel='prefetch';link.as='document';link.href='/prefetched';
                                    link.onload=resolve;link.onerror=reject;doc.head.appendChild(link);
                                }});
                                await new Promise(resolve => {{frame.onload=resolve;frame.contentWindow.location='/prefetched';}});
                                globalThis.observation=snapshot(frame.contentWindow);
                            }})().catch(error => globalThis.caseError=String(error));
                        </script>"#)),
                        "/sw.js" => ("text/javascript", SERVICE_WORKER.to_owned()),
                        "/setup" => ("text/html", format!(r#"<!doctype html><body><script>
                            {STATE_WAITER}
                            (async () => {{
                                const registration = await navigator.serviceWorker.register('/sw.js', {{scope:'/controlled/'}});
                                await waitState(registration.installing || registration.waiting || registration.active, 'activated');
                                globalThis.swReady = true;
                            }})().catch(error => globalThis.caseError = String(error));
                        </script>"#)),
                        "/activation" => ("text/html", format!(r#"<!doctype html><body><script>
                            {STATE_WAITER}
                            (async () => {{
                                const registration = await navigator.serviceWorker.register('/sw.js', {{scope:'/controlled/'}});
                                await waitState(registration.installing || registration.waiting || registration.active, 'activating');
                                const frame = document.createElement('iframe');
                                await new Promise(resolve => {{ frame.onload = resolve; frame.src = '/empty'; document.body.appendChild(frame); }});
                                const beforeNavigate = performance.timeOrigin + performance.now();
                                await new Promise(resolve => {{ frame.onload = resolve; frame.contentWindow.location = '/controlled/frame'; }});
                                globalThis.observation = {{beforeNavigate, child:snapshot(frame.contentWindow)}};
                            }})().catch(error => globalThis.caseError = String(error));
                        </script>"#)),
                        "/ownership" => ("text/html", r#"<!doctype html><body><script>
                            (async () => {
                                const frame = document.createElement('iframe');
                                await new Promise(resolve => { frame.onload=resolve; frame.src='/slow'; document.body.appendChild(frame); });
                                const win=frame.contentWindow;
                                const old=win.performance.getEntriesByType('navigation')[0];
                                const before=old.toJSON(), origin=win.performance.timeOrigin;
                                const getter=Object.getOwnPropertyDescriptor(PerformanceResourceTiming.prototype,'encodedBodySize').get;
                                const borrowed=getter.call(old);
                                win.document.open(); win.document.write('<!doctype html><body>replacement stream'); win.document.close();
                                const opened=old.toJSON(), openOrigin=win.performance.timeOrigin;
                                await new Promise(resolve => { frame.onload=resolve; frame.src='/empty'; });
                                globalThis.observation={before,origin,borrowed,opened,openOrigin,retired:old.toJSON(),current:frame.contentWindow.performance.getEntriesByType('navigation')[0].toJSON()};
                            })().catch(error => globalThis.caseError=String(error));
                        </script>"#.to_owned()),
                        "/slow" => {
                            // Deliberately delay the response, so a context-creation
                            // time origin cannot masquerade as navigation start.
                            tokio::time::sleep(Duration::from_millis(100)).await;
                            ("text/html", format!("<!doctype html><body><script>globalThis.responseSent={};</script>", epoch_millis()))
                        }
                        _ => ("text/html", "<!doctype html><body>network".to_owned()),
                    };
                    let cache_control = if path == "/prefetched" { "max-age=300" } else { "no-store" };
                    let head = format!("HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nCache-Control: {cache_control}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                    let _ = socket.write_all(head.as_bytes()).await;
                    let _ = socket.write_all(body.as_bytes()).await;
                });
                while let Some(result) = connections.try_join_next() {
                    result.unwrap();
                }
            }
        });
        Self {
            origin,
            task,
            prefetched_requests,
        }
    }
}

async fn read_json(
    page: &mut moli_core::page::Page,
    expression: &str,
) -> Result<serde_json::Value> {
    let result = page
        .evaluate_runtime_expression_by_value_async(expression)
        .await?;
    Ok(serde_json::from_str(
        result["value"]
            .as_str()
            .unwrap_or_else(|| panic!("{result}")),
    )?)
}

fn navigation_snapshot() -> String {
    format!("(() => {{ {STATE_WAITER}; return JSON.stringify(snapshot(window)); }})()")
}

#[tokio::test(flavor = "multi_thread")]
async fn navigation_time_origin_precedes_the_http_response() -> Result<()> {
    let server = NavigationTimingServer::spawn().await;
    let browser = Browser::new(AppConfig::default())?;
    let before = epoch_millis();
    let mut page = browser.fetch(&format!("{}/slow", server.origin)).await?;
    let result = read_json(&mut page, &format!("(() => {{ {STATE_WAITER}; return JSON.stringify({{...snapshot(window), responseSent}}); }})()")).await?;
    let origin = result["timeOrigin"].as_f64().unwrap();
    assert!(
        origin >= before && origin < result["responseSent"].as_f64().unwrap(),
        "{result}"
    );
    assert_eq!(result["nav"]["responseStatus"], 200, "{result}");
    assert!(
        result["nav"]["encodedBodySize"].as_u64().unwrap() > 0,
        "{result}"
    );
    let nav = &result["nav"];
    assert_eq!(nav["workerStart"], 0);
    for names in [
        ["fetchStart", "requestStart"],
        ["requestStart", "responseStart"],
        ["responseStart", "responseEnd"],
    ] {
        assert!(
            nav[names[0]].as_f64().unwrap() <= nav[names[1]].as_f64().unwrap(),
            "{result}"
        );
    }
    assert!(nav["responseStart"].as_f64().unwrap() > 0.0, "{result}");
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn iframe_navigation_timing_spans_service_worker_activation() -> Result<()> {
    let server = NavigationTimingServer::spawn().await;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser
        .fetch(&format!("{}/activation", server.origin))
        .await?;
    browser
        .wait_for_script_truthy(
            &mut page,
            "Boolean(globalThis.observation || globalThis.caseError)",
            Duration::from_secs(10),
        )
        .await?;
    let result = read_json(
        &mut page,
        "JSON.stringify({observation:globalThis.observation,error:globalThis.caseError})",
    )
    .await?;
    assert!(result.get("error").is_none(), "{result}");
    let observation = &result["observation"];
    let child = &observation["child"];
    let origin = child["timeOrigin"].as_f64().unwrap();
    let activation_end = child["swTimes"]["activateEnd"].as_f64().unwrap();
    let fetch_event = child["swTimes"]["fetchEvent"].as_f64().unwrap();
    assert!(
        origin >= observation["beforeNavigate"].as_f64().unwrap() && origin < activation_end,
        "{result}"
    );
    assert!(
        origin + child["nav"]["workerStart"].as_f64().unwrap() < activation_end,
        "{result}"
    );
    assert!(
        activation_end < origin + child["nav"]["fetchStart"].as_f64().unwrap(),
        "{result}"
    );
    assert!(
        origin + child["nav"]["fetchStart"].as_f64().unwrap() < fetch_event,
        "{result}"
    );
    assert_eq!(child["nav"]["responseStatus"], 200, "{result}");
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn main_navigation_timing_records_the_running_service_worker() -> Result<()> {
    let server = NavigationTimingServer::spawn().await;
    let browser = Browser::new(AppConfig::default())?;
    let mut setup = browser.fetch(&format!("{}/setup", server.origin)).await?;
    browser
        .wait_for_script_truthy(
            &mut setup,
            "Boolean(globalThis.swReady || globalThis.caseError)",
            Duration::from_secs(10),
        )
        .await?;
    let mut page = browser
        .fetch(&format!("{}/controlled/main", server.origin))
        .await?;
    let result = read_json(&mut page, &navigation_snapshot()).await?;
    assert_eq!(result["nav"]["responseStatus"], 200, "{result}");
    let origin = result["timeOrigin"].as_f64().unwrap();
    assert!(
        origin + result["nav"]["workerStart"].as_f64().unwrap()
            <= result["swTimes"]["fetchEvent"].as_f64().unwrap(),
        "{result}"
    );
    assert!(
        result["nav"]["fetchStart"].as_f64().unwrap() > 0.0,
        "{result}"
    );
    assert!(
        result["nav"]["responseStart"].as_f64().unwrap()
            >= result["nav"]["fetchStart"].as_f64().unwrap(),
        "{result}"
    );
    assert!(
        result["nav"]["decodedBodySize"].as_u64().unwrap() > 0,
        "{result}"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn cached_navigation_entry_observes_stream_completion() -> Result<()> {
    let server = NavigationTimingServer::spawn().await;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser.fetch(&format!("{}/stream", server.origin)).await?;
    let result = read_json(&mut page, "JSON.stringify({early:earlyNav,late:savedNav.toJSON(),same:savedNav===performance.getEntriesByType('navigation')[0]})").await?;
    assert_eq!(result["same"], true, "{result}");
    assert_eq!(result["early"]["responseStatus"], 200, "{result}");
    assert_eq!(result["early"]["responseEnd"], 0, "{result}");
    assert!(
        result["early"]["responseStart"].as_f64().unwrap() > 0.0,
        "{result}"
    );
    assert!(
        result["late"]["responseEnd"].as_f64().unwrap()
            > result["early"]["responseStart"].as_f64().unwrap(),
        "{result}"
    );
    assert!(
        result["late"]["encodedBodySize"].as_u64().unwrap() > 0,
        "{result}"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn cross_origin_navigation_redirect_exposes_only_the_final_response() -> Result<()> {
    let first = NavigationTimingServer::spawn().await;
    let last = NavigationTimingServer::spawn().await;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser
        .fetch(&format!(
            "{}/redirect?to={}/slow",
            first.origin, last.origin
        ))
        .await?;
    let result = read_json(&mut page, &navigation_snapshot()).await?;
    let nav = &result["nav"];
    assert_eq!(nav["redirectStart"], 0, "{result}");
    assert_eq!(nav["redirectEnd"], 0, "{result}");
    assert_eq!(nav["responseStatus"], 200, "{result}");
    assert!(nav["fetchStart"].as_f64().unwrap() > 0.0, "{result}");
    assert!(
        nav["responseStart"].as_f64().unwrap() > nav["fetchStart"].as_f64().unwrap(),
        "{result}"
    );
    assert!(nav["encodedBodySize"].as_u64().unwrap() > 0, "{result}");
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn navigation_timing_stays_with_its_window_across_open_and_replacement() -> Result<()> {
    let server = NavigationTimingServer::spawn().await;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser
        .fetch(&format!("{}/ownership", server.origin))
        .await?;
    browser
        .wait_for_script_truthy(
            &mut page,
            "Boolean(globalThis.observation || globalThis.caseError)",
            Duration::from_secs(10),
        )
        .await?;
    let result = read_json(
        &mut page,
        "JSON.stringify({observation:globalThis.observation,error:globalThis.caseError})",
    )
    .await?;
    assert!(result.get("error").is_none(), "{result}");
    let observation = &result["observation"];
    assert_eq!(observation["origin"], observation["openOrigin"], "{result}");
    assert_eq!(
        observation["borrowed"], observation["before"]["encodedBodySize"],
        "{result}"
    );
    for name in [
        "fetchStart",
        "requestStart",
        "responseStart",
        "responseEnd",
        "encodedBodySize",
        "responseStatus",
    ] {
        assert_eq!(
            observation["before"][name], observation["opened"][name],
            "{name}: {result}"
        );
        assert_eq!(
            observation["before"][name], observation["retired"][name],
            "{name}: {result}"
        );
    }
    assert_ne!(
        observation["current"]["encodedBodySize"], observation["retired"]["encodedBodySize"],
        "{result}"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn prefetched_navigation_reuses_the_response_and_reports_a_local_transfer() -> Result<()> {
    let server = NavigationTimingServer::spawn().await;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser
        .fetch(&format!("{}/prefetch", server.origin))
        .await?;
    browser
        .wait_for_script_truthy(
            &mut page,
            "Boolean(globalThis.observation || globalThis.caseError)",
            Duration::from_secs(10),
        )
        .await?;
    let result = read_json(
        &mut page,
        "JSON.stringify({observation:globalThis.observation,error:globalThis.caseError})",
    )
    .await?;
    assert!(result.get("error").is_none(), "{result}");
    assert_eq!(
        server
            .prefetched_requests
            .load(std::sync::atomic::Ordering::SeqCst),
        1,
        "navigation must reuse the prefetched response: {result}"
    );
    let nav = &result["observation"]["nav"];
    assert_eq!(nav["responseStatus"], 200, "{result}");
    assert_eq!(nav["transferSize"], 0, "{result}");
    assert!(nav["encodedBodySize"].as_u64().unwrap() > 0, "{result}");
    assert!(nav["responseEnd"].as_f64().unwrap() > 0.0, "{result}");
    Ok(())
}
