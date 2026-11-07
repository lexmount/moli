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
    if (event.request.url.includes('network-fallback')) return;
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
                        "/queued-network" => ("text/html", queued_navigation_source(false)),
                        "/queued-service-worker" => ("text/html", queued_navigation_source(true)),
                        "/beforeunload" => ("text/html", beforeunload_navigation_source()),
                        "/with-nested" => ("text/html", "<!doctype html><body><iframe src='/empty'></iframe>".to_owned()),
                        "/unload-cases" => ("text/html", unload_navigation_cases_source()),
                        "/unload-opaque-host" => ("text/html", opaque_unload_navigation_source().to_owned()),
                        path if path.starts_with("/unload-previous?") => ("text/html", r#"<!doctype html><body><iframe src='/unload-child'></iframe><script>
                            function busy(ms) {const end=performance.now()+ms;while(performance.now()<end){}}
                            addEventListener('pagehide',()=>busy(250));
                            addEventListener('unload',()=>busy(105));
                            addEventListener('load',()=> (opener||parent).postMessage({phase:'ready',token:new URL(location.href).searchParams.get('token')},'*'));
                        </script>"#.to_owned()),
                        "/unload-child" => ("text/html", r#"<!doctype html><script>
                            addEventListener('unload',()=>{const end=performance.now()+405;while(performance.now()<end){}});
                        </script>"#.to_owned()),
                        path if path.starts_with("/unload-report?") => ("text/html", format!(r#"<!doctype html><script>
                            {STATE_WAITER}
                            addEventListener('load',()=> (opener||parent).postMessage({{phase:'report',token:new URL(location.href).searchParams.get('token'),...snapshot(window),legacy:performance.timing.toJSON()}},'*'));
                        </script>"#)),
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

fn queued_navigation_source(service_worker: bool) -> String {
    format!(
        r#"<!doctype html><body><script>
        {STATE_WAITER}
        (async () => {{
            if ({service_worker}) {{
                const registration = await navigator.serviceWorker.register('/sw.js', {{scope:'/controlled/'}});
                await waitState(registration.installing || registration.waiting || registration.active, 'activated');
            }}
            const observations = [];
            for (const kind of ['location', 'src', 'replacement']) {{
                for (const fallback of ({service_worker} ? [false, true] : [false])) {{
                    const frame = document.createElement('iframe');
                    await new Promise(resolve => {{ frame.onload=resolve;frame.src='/empty';document.body.appendChild(frame); }});
                    if (kind === 'replacement') {{
                        frame.contentWindow.location='/slow?discarded';
                        const discardedEnd=performance.now()+100;
                        while(performance.now()<discardedEnd) {{}}
                    }}
                    const before=performance.timeOrigin+performance.now();
                    const loaded=new Promise(resolve => frame.addEventListener('load',resolve,{{once:true}}));
                    const target=({service_worker} ? '/controlled/queued' : '/empty')+'?kind='+kind+(fallback ? '&network-fallback' : '');
                    if (kind === 'src') frame.src=target;
                    else frame.contentWindow.location=target;
                    const afterAssignment=performance.timeOrigin+performance.now();
                    const busyEnd=performance.now()+150;
                    while(performance.now()<busyEnd) {{}}
                    const afterBusy=performance.timeOrigin+performance.now();
                    await loaded;
                    observations.push({{kind,fallback,before,afterAssignment,afterBusy,...snapshot(frame.contentWindow)}});
                    frame.remove();
                }}
            }}
            globalThis.observations=observations;
        }})().catch(error => globalThis.caseError=String(error));
        </script>"#,
    )
}

async fn check_queued_navigation_clock(service_worker: bool) -> Result<()> {
    let server = NavigationTimingServer::spawn().await;
    let browser = Browser::new(AppConfig::default())?;
    let path = if service_worker {
        "/queued-service-worker"
    } else {
        "/queued-network"
    };
    let mut page = browser.fetch(&format!("{}{path}", server.origin)).await?;
    browser
        .wait_for_script_truthy(
            &mut page,
            "Boolean(globalThis.observations || globalThis.caseError)",
            Duration::from_secs(15),
        )
        .await?;
    let result = read_json(
        &mut page,
        "JSON.stringify({observations:globalThis.observations,error:globalThis.caseError})",
    )
    .await?;
    assert!(result.get("error").is_none(), "{result}");
    let observations = result["observations"].as_array().unwrap();
    assert_eq!(observations.len(), if service_worker { 6 } else { 3 });
    for row in observations {
        let origin = row["timeOrigin"].as_f64().unwrap();
        let before = row["before"].as_f64().unwrap();
        let assignment = row["afterAssignment"].as_f64().unwrap();
        assert!(
            origin >= before - 1.0,
            "a replacement must get its own navigation start: {row}"
        );
        assert!(
            origin <= assignment + 20.0,
            "navigation must include the deferred task's queueing time: {row}"
        );
        assert!(
            row["afterBusy"].as_f64().unwrap() - origin >= 100.0,
            "{row}"
        );
        let nav = &row["nav"];
        assert!(
            nav["fetchStart"].as_f64().unwrap() >= 100.0,
            "fetchStart must reflect actual deferred fetching: {row}"
        );
        for [earlier, later] in [
            ["fetchStart", "requestStart"],
            ["requestStart", "responseStart"],
            ["responseStart", "responseEnd"],
            ["fetchStart", "domainLookupStart"],
            ["domainLookupStart", "domainLookupEnd"],
            ["domainLookupEnd", "connectStart"],
            ["connectStart", "connectEnd"],
        ] {
            assert!(
                nav[earlier].as_f64().unwrap() <= nav[later].as_f64().unwrap(),
                "{earlier} <= {later}: {row}"
            );
        }
        assert_eq!(
            nav["secureConnectionStart"], 0,
            "plain HTTP must not acquire a TLS timestamp: {row}"
        );
        if service_worker {
            assert!(
                nav["workerStart"].as_f64().unwrap() >= 100.0,
                "the actual SW dispatch follows the queued parent script: {row}"
            );
            assert!(
                nav["workerStart"].as_f64().unwrap() <= nav["fetchStart"].as_f64().unwrap(),
                "{row}"
            );
        } else {
            assert_eq!(nav["workerStart"], 0, "{row}");
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn queued_frame_navigation_keeps_the_accepted_attempt_clock() -> Result<()> {
    check_queued_navigation_clock(false).await
}

#[tokio::test(flavor = "multi_thread")]
async fn queued_service_worker_navigation_keeps_the_accepted_attempt_clock() -> Result<()> {
    check_queued_navigation_clock(true).await
}

fn beforeunload_navigation_source() -> String {
    format!(
        r#"<!doctype html><body><script>
        {STATE_WAITER}
        globalThis.runBeforeunloadCases = async () => {{
            const observations = [];
            for (const kind of ['listener', 'property', 'attribute', 'descendant']) {{
                globalThis.beforeunloadStage=kind+':initial-load';
                const frame = document.createElement('iframe');
                await new Promise(resolve => {{ frame.onload=resolve;frame.src=kind === 'descendant' ? '/with-nested' : '/empty';document.body.appendChild(frame); }});
                let unloadStart, unloadEnd, calls=0;
                const handler = () => {{
                    calls++;
                    unloadStart=performance.timeOrigin+performance.now();
                    const end=performance.now()+150;
                    while(performance.now()<end) {{}}
                    unloadEnd=performance.timeOrigin+performance.now();
                }};
                if (kind === 'property') frame.contentWindow.onbeforeunload=handler;
                else if (kind === 'attribute') {{
                    globalThis.recordBeforeunload=handler;
                    frame.contentDocument.body.setAttribute('onbeforeunload', 'parent.recordBeforeunload()');
                }} else if (kind === 'descendant') {{
                    const nested=frame.contentDocument.querySelector('iframe');
                    nested.contentWindow.addEventListener('beforeunload',handler);
                }} else frame.contentWindow.addEventListener('beforeunload',handler);
                globalThis.beforeunloadStage=kind+':navigate';
                const loaded=new Promise(resolve => frame.addEventListener('load',resolve,{{once:true}}));
                if (kind === 'listener') frame.contentWindow.location='/slow';
                else frame.src='/slow';
                const busyEnd=performance.now()+150;
                while(performance.now()<busyEnd) {{}}
                await loaded;
                observations.push({{kind,calls,unloadStart,unloadEnd,responseSent:frame.contentWindow.responseSent,...snapshot(frame.contentWindow)}});
                globalThis.beforeunloadRows=observations;
                frame.remove();
            }}
            globalThis.observations=observations;
        }};
        </script>"#,
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn frame_navigation_time_origin_follows_beforeunload_handlers() -> Result<()> {
    let server = NavigationTimingServer::spawn().await;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = tokio::time::timeout(
        Duration::from_secs(30),
        browser.fetch(&format!("{}/beforeunload", server.origin)),
    )
    .await??;
    page.evaluate_runtime_expression_by_value_async(
        "void runBeforeunloadCases().catch(error => globalThis.caseError=String(error))",
    )
    .await?;
    let waited = tokio::time::timeout(
        Duration::from_secs(20),
        browser.wait_for_script_truthy(
            &mut page,
            "Boolean(globalThis.observations || globalThis.caseError)",
            Duration::from_secs(15),
        ),
    )
    .await;
    let result = read_json(
        &mut page,
        "JSON.stringify({observations:globalThis.observations,error:globalThis.caseError,stage:globalThis.beforeunloadStage,rows:globalThis.beforeunloadRows})",
    )
    .await?;
    assert!(matches!(waited, Ok(Ok(()))), "{waited:?}: {result}");
    assert!(result.get("error").is_none(), "{result}");
    let observations = result["observations"].as_array().unwrap();
    assert_eq!(observations.len(), 4, "{result}");
    for row in observations {
        assert_eq!(row["calls"], 1, "{row}");
        let end = row["unloadEnd"].as_f64().unwrap();
        let origin = row["timeOrigin"].as_f64().unwrap();
        assert!(end - row["unloadStart"].as_f64().unwrap() >= 140.0, "{row}");
        assert!(
            origin >= end - 1.0,
            "navigation time origin must follow beforeunload: {row}"
        );
        assert!(
            origin <= row["responseSent"].as_f64().unwrap() + 1.0,
            "navigation time origin must precede the response: {row}"
        );
    }
    Ok(())
}

fn unload_navigation_cases_source() -> String {
    r#"<!doctype html><body><script>
    globalThis.runUnloadCases = async privacy => {
        const messages = new Map(), waiters = new Map();
        addEventListener('message', event => {
            const data=event.data;
            if (!data || !data.token || !data.phase) return;
            const key=data.token+':'+data.phase;
            messages.set(key,data);
            if(waiters.has(key)) waiters.get(key)(data);
        });
        const waitFor=(token,phase)=>new Promise(resolve=> {
            const key=token+':'+phase;
            if(messages.has(key)) resolve(messages.get(key));
            else waiters.set(key,resolve);
        });
        const remote=new URL(location.href);
        remote.hostname=location.hostname==='127.0.0.1'?'localhost':'127.0.0.1';
        const observations=[];
        for (const kind of ['iframe','popup']) {
            for (const mode of (privacy?['cross','redirect','opaque']:['same'])) {
                if(mode==='opaque' && kind==='popup') continue;
                const token=kind+'-'+mode;
                globalThis.unloadStage=token+':ready';
                const ready=waitFor(token,'ready'), report=waitFor(token,'report');
                const old=(mode==='cross'?remote.origin:location.origin)+'/unload-previous?token='+token;
                let win,frame;
                if(kind==='iframe') {
                    frame=document.createElement('iframe');
                    if(mode==='opaque') frame.sandbox='allow-scripts';
                    frame.src=old;
                    document.body.appendChild(frame);win=frame.contentWindow;
                } else win=window.open(old);
                await ready;
                globalThis.unloadStage=token+':navigate';
                const destination=location.origin+'/unload-report?token='+token;
                const target=mode==='redirect'?location.origin+'/redirect?to='+remote.origin+'/redirect?to='+destination:destination;
                win.location=target;
                const row={kind,mode,...await report};
                observations.push(row);globalThis.unloadRows=observations;
                if(frame) frame.remove();else win.close();
            }
        }
        globalThis.observations=observations;
    };
    globalThis.runOpaqueUnloadCases = async () => {
        globalThis.unloadStage='opaque-inherited:load';
        const done=new Promise(resolve => addEventListener('message',event => {
            if(event.data && event.data.phase==='report') resolve(event.data);
        }));
        const source=await (await fetch('/unload-opaque-host')).text();
        const frame=document.createElement('iframe');
        frame.src='data:text/html;charset=utf-8,'+encodeURIComponent(source);document.body.appendChild(frame);
        globalThis.observations=[await done];frame.remove();
    };
    </script>"#
        .to_owned()
}

fn opaque_unload_navigation_source() -> &'static str {
    r#"<!doctype html><body><script>
    function documentFor(fn) {return '<!doctype html><body><script>('+fn.toString()+')();<'+'/script>';}
    addEventListener('load',async()=>{
        const frame=document.createElement('iframe');
        let oldAccess;
        const ready=new Promise(resolve=>addEventListener('message',event=>{
            if(event.source!==frame.contentWindow) return;
            if(event.data.phase==='ready') {oldAccess=event.data.parentAccess;resolve();}
            if(event.data.phase==='report') parent.postMessage({kind:'iframe',mode:'opaque-inherited',oldAccess,...event.data},'*');
        }));
        frame.srcdoc=documentFor(function(){
            addEventListener('unload',()=>{const end=performance.now()+105;while(performance.now()<end){}});
            addEventListener('load',()=>{
                let parentAccess=false;try{parentAccess=Boolean(parent.document);}catch{}
                parent.postMessage({phase:'ready',parentAccess},'*');
            });
        });
        document.body.appendChild(frame);await ready;
        frame.srcdoc=documentFor(function(){
            addEventListener('load',()=>{
                let parentAccess=false;try{parentAccess=Boolean(parent.document);}catch{}
                parent.postMessage({phase:'report',parentAccess,timeOrigin:performance.timeOrigin,nav:performance.getEntriesByType('navigation')[0].toJSON(),legacy:performance.timing.toJSON()},'*');
            });
        });
    });
    </script>"#
}

async fn read_previous_document_unload_cases(script: &str) -> Result<serde_json::Value> {
    let server = NavigationTimingServer::spawn().await;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = tokio::time::timeout(
        Duration::from_secs(30),
        browser.fetch(&format!("{}/unload-cases", server.origin)),
    )
    .await??;
    page.evaluate_runtime_expression_by_value_async(&format!(
        "void ({script}).catch(error => globalThis.caseError=String(error))"
    ))
    .await?;
    let waited = tokio::time::timeout(
        Duration::from_secs(35),
        browser.wait_for_script_truthy(
            &mut page,
            "Boolean(globalThis.observations || globalThis.caseError)",
            Duration::from_secs(30),
        ),
    )
    .await;
    let result = read_json(
        &mut page,
        "JSON.stringify({observations:globalThis.observations,error:globalThis.caseError,stage:globalThis.unloadStage,rows:globalThis.unloadRows})",
    )
    .await?;
    assert!(matches!(waited, Ok(Ok(()))), "{waited:?}: {result}");
    assert!(result.get("error").is_none(), "{result}");
    Ok(result)
}

async fn check_previous_document_unload_timing(privacy: bool) -> Result<()> {
    let result = read_previous_document_unload_cases(&format!("runUnloadCases({privacy})")).await?;
    let rows = result["observations"].as_array().unwrap();
    assert_eq!(rows.len(), if privacy { 5 } else { 2 }, "{result}");
    for row in rows {
        let nav = &row["nav"];
        let legacy = &row["legacy"];
        if privacy {
            for field in ["unloadEventStart", "unloadEventEnd"] {
                assert_eq!(
                    nav[field], 0,
                    "cross-origin unload timing must be hidden: {row}"
                );
                assert_eq!(legacy[field], 0, "{row}");
            }
        } else {
            let start = nav["unloadEventStart"].as_f64().unwrap();
            let end = nav["unloadEventEnd"].as_f64().unwrap();
            assert!(
                start > 0.0,
                "previous document unload timing must be observed: {row}"
            );
            assert!(
                end - start >= 100.0,
                "the root unload handler must be measured: {row}"
            );
            assert!(
                end - start < 250.0,
                "pagehide and descendant unload must be excluded: {row}"
            );
            for (field, relative) in [("unloadEventStart", start), ("unloadEventEnd", end)] {
                let absolute = legacy[field].as_f64().unwrap();
                assert!(
                    absolute >= legacy["navigationStart"].as_f64().unwrap(),
                    "{row}"
                );
                assert!(
                    (absolute - row["timeOrigin"].as_f64().unwrap() - relative).abs() < 1.1,
                    "modern and legacy unload timing must describe the same event: {row}"
                );
            }
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn previous_document_unload_timing_measures_only_the_root_event() -> Result<()> {
    check_previous_document_unload_timing(false).await
}

#[tokio::test(flavor = "multi_thread")]
async fn previous_document_unload_timing_hides_cross_origin_and_redirects() -> Result<()> {
    check_previous_document_unload_timing(true).await
}

#[tokio::test(flavor = "multi_thread")]
async fn previous_document_unload_timing_preserves_inherited_opaque_origins() -> Result<()> {
    let result = read_previous_document_unload_cases("runOpaqueUnloadCases()").await?;
    let rows = result["observations"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "{result}");
    let row = &rows[0];
    assert_eq!(
        row["oldAccess"], true,
        "old srcdoc must inherit its parent's opaque origin: {row}"
    );
    assert_eq!(
        row["parentAccess"], true,
        "new srcdoc must inherit the same opaque origin: {row}"
    );
    let start = row["nav"]["unloadEventStart"].as_f64().unwrap();
    let end = row["nav"]["unloadEventEnd"].as_f64().unwrap();
    assert!(
        start > 0.0 && end - start >= 100.0,
        "inherited opaque origin must retain its real previous unload timing: {row}"
    );
    assert!(
        row["legacy"]["unloadEventStart"].as_f64().unwrap() > 0.0,
        "{row}"
    );
    for (field, relative) in [("unloadEventStart", start), ("unloadEventEnd", end)] {
        let absolute = row["legacy"][field].as_f64().unwrap();
        assert!(
            absolute >= row["legacy"]["navigationStart"].as_f64().unwrap(),
            "{row}"
        );
        assert!(
            (absolute - row["timeOrigin"].as_f64().unwrap() - relative).abs() < 1.1,
            "{row}"
        );
    }
    Ok(())
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
