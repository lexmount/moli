use super::auxiliary_page_identity::{open_auxiliary, open_auxiliary_with_url, resume_auxiliary};
use super::*;

async fn wait_for_value(page: &mut TestCdpSocket, expression: &str, expected: serde_json::Value) {
    let mut actual = serde_json::Value::Null;
    timeout(Duration::from_secs(5), async {
        loop {
            actual = evaluate_window_name_probe(page, 900, expression).await;
            if actual == expected {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{expression} did not become {expected}; last value was {actual}"));
}

async fn check_sequential_form_popup_requests(method: &str) {
    let requests = Arc::new(parking_lot::Mutex::new(Vec::<serde_json::Value>::new()));
    let captured = Arc::clone(&requests);
    let (fixture_addr, _fixture) = spawn_dedicated_fixture_server(
        Router::new().fallback(axum::routing::any(
            move |uri: axum::http::Uri,
                  method: axum::http::Method,
                  headers: axum::http::HeaderMap,
                  body: axum::body::Bytes| {
                let captured = Arc::clone(&captured);
                async move {
                    if !uri.path().starts_with("/form/") {
                        return axum::response::Html("<!doctype html><body></body>".to_owned());
                    }
                    let request = json!({
                        "path": uri.path(),
                        "query": uri.query(),
                        "method": method.as_str(),
                        "body": String::from_utf8(body.to_vec()).unwrap(),
                        "referrer": headers.get("referer").and_then(|value| value.to_str().ok()),
                        "contentType": headers.get("content-type").and_then(|value| value.to_str().ok()),
                    });
                    captured.lock().push(request.clone());
                    axum::response::Html(format!(
                        "<!doctype html><script>const channel=new BroadcastChannel('form-navigation');channel.onmessage=()=>close();channel.postMessage({{request:{request},referrer:document.referrer,hasOpener:opener!==null}});</script>"
                    ))
                }
            },
        )),
        "sequential-form-popup-requests",
    );
    let base = format!("http://{fixture_addr}");
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .unwrap();
    let parent_id = create_dynamic_target(&mut browser, 1).await;
    let mut parent = connect_dynamic_page(addr, &parent_id).await;
    send_cdp_command(&mut parent, 1, "Page.enable", None, json!({})).await;
    navigate_dynamic_page_and_wait_for_load(
        &mut parent,
        2,
        &format!("{base}/source?accepted=true#fragment"),
    )
    .await;
    evaluate_window_name_probe(
        &mut parent,
        3,
        "window.results=[];window.channel=new BroadcastChannel('form-navigation');channel.onmessage=event=>results.push(event.data);window.form=document.createElement('form');document.body.append(form);true",
    )
    .await;
    let initial_target_count = fetch_server_json(addr, "/json/list")
        .await
        .as_array()
        .unwrap()
        .len();
    let mut sequence = 0;
    for submitter in ["form", "button", "input"] {
        for (rel, has_opener, has_referrer) in [
            ("", false, true),
            ("noopener", false, true),
            ("noreferrer", false, false),
            ("opener", true, true),
            ("noopener noreferrer", false, false),
            ("noreferrer opener", false, false),
            ("opener noopener", false, true),
        ] {
            let path = format!("/form/{sequence}");
            evaluate_window_name_probe(
                &mut parent,
                4,
                &format!(
                    "(()=>{{results.length=0;form.innerHTML='<input type=hidden name=value value=\"a b&c\">';form.action={};form.method={};form.rel={};form.target='_blank';if({}==='form'){{form.submit()}}else{{const control=document.createElement({});control.type='submit';control.formTarget='_blank';form.append(control);control.click()}}return true}})()",
                    json!(path), json!(method), json!(rel), json!(submitter), json!(submitter),
                ),
            )
            .await;
            wait_for_value(&mut parent, "results.length > 0", json!(true)).await;
            let result = evaluate_window_name_probe(&mut parent, 5, "results[0]").await;
            evaluate_window_name_probe(&mut parent, 6, "channel.postMessage(null);true").await;
            wait_for_target_list(addr, "submitted popup closed", |targets| {
                targets.len() == initial_target_count
            })
            .await;
            assert_eq!(
                evaluate_window_name_probe(&mut parent, 7, "results.length").await,
                json!(1),
                "{method}, submitter={submitter}, rel={rel:?}: execute the response once",
            );
            let expected_referrer = has_referrer.then(|| format!("{base}/source?accepted=true"));
            let expected_request = json!({
                "path": path,
                "query": (method == "GET").then_some("value=a+b%26c"),
                "method": method,
                "body": if method == "POST" { "value=a+b%26c" } else { "" },
                "referrer": expected_referrer,
                "contentType": (method == "POST").then_some("application/x-www-form-urlencoded"),
            });
            let actual_requests = requests
                .lock()
                .iter()
                .filter(|request| request["path"] == path)
                .cloned()
                .collect::<Vec<_>>();
            assert_eq!(
                actual_requests,
                vec![expected_request.clone()],
                "{method}, submitter={submitter}, rel={rel:?}: exactly one accepted form request",
            );
            assert_eq!(
                result,
                json!({
                    "request": expected_request,
                    "referrer": expected_referrer.as_deref().unwrap_or(""),
                    "hasOpener": has_opener,
                }),
                "{method}, submitter={submitter}, rel={rel:?}: committed popup relations",
            );
            sequence += 1;
        }
    }
    let initial_ids = fetch_server_json(addr, "/json/list")
        .await
        .as_array()
        .unwrap()
        .iter()
        .map(|target| target["id"].clone())
        .collect::<Vec<_>>();
    evaluate_window_name_probe(&mut parent, 8, &format!(
        "(()=>{{results.length=0;form.innerHTML='<input type=hidden name=value value=\"a b&c\"><button type=submit>submit</button>';form.method={};form.rel='opener';form.target='cancelled-form-target';form.action='/form/cancelled';form.submit();form.target='accepted-form-target';form.action='/form/accepted';form.querySelector('button').click();form.method='get';form.action='/wrong';form.rel='noreferrer';form.querySelector('input').value='wrong';return true}})()",
        json!(method),
    )).await;
    wait_for_value(&mut parent, "results.length > 0", json!(true)).await;
    let result = evaluate_window_name_probe(&mut parent, 9, "results[0]").await;
    assert_eq!(result["request"]["path"], "/form/accepted");
    assert_eq!(result["request"]["method"], method);
    assert_eq!(result["referrer"], format!("{base}/source?accepted=true"));
    assert_eq!(result["hasOpener"], true);
    if method == "POST" {
        assert_eq!(result["request"]["body"], "value=a+b%26c");
    } else {
        assert_eq!(result["request"]["query"], "value=a+b%26c");
    }
    evaluate_window_name_probe(&mut parent, 10, "channel.postMessage(null);true").await;
    let remaining =
        wait_for_target_list(addr, "only the cancelled empty popup remains", |targets| {
            targets.len() == initial_target_count + 1
        })
        .await;
    assert_eq!(
        requests
            .lock()
            .iter()
            .filter(|request| request["path"] == "/form/cancelled")
            .count(),
        0,
        "a superseded submission must not start a request on its selected Page",
    );
    let cancelled = remaining
        .iter()
        .find(|target| !initial_ids.contains(&target["id"]))
        .unwrap();
    assert_eq!(cancelled["url"], "about:blank");
    send_cdp_command(
        &mut browser,
        700,
        "Target.closeTarget",
        None,
        json!({"targetId": cancelled["id"]}),
    )
    .await;
    wait_for_target_list(addr, "cancelled form target closed", |targets| {
        targets.len() == initial_target_count
    })
    .await;
    assert_eq!(
        evaluate_window_name_probe(&mut parent, 11, "results.length").await,
        json!(1)
    );
    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sequential_form_popups_issue_one_get_with_source_referrer() {
    check_sequential_form_popup_requests("GET").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sequential_form_popups_preserve_post_body_and_source_referrer() {
    check_sequential_form_popup_requests("POST").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn popup_document_access_keeps_retained_functions_and_checks_borrowed_receivers() {
    let (fixture_addr, _fixture) = spawn_dedicated_fixture_server(
        Router::new().fallback(get(|uri: axum::http::Uri| async move {
            axum::response::Html(format!("<h1>{}</h1>", uri.path()))
        })),
        "popup-document-access",
    );
    let base = format!("http://{fixture_addr}");
    let cross = format!("http://localhost:{}", fixture_addr.port());
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .unwrap();
    let opener_id = create_dynamic_target(&mut browser, 1).await;
    let mut opener = connect_dynamic_page(addr, &opener_id).await;
    send_cdp_command(&mut opener, 1, "Page.enable", None, json!({})).await;
    navigate_dynamic_page_and_wait_for_load(&mut opener, 2, &format!("{base}/opener")).await;
    for destination in [&base, &cross] {
        let (child_id, mut child) = open_auxiliary(addr, &mut opener, "").await;
        navigate_dynamic_page_and_wait_for_load(&mut child, 2, &format!("{base}/one")).await;
        evaluate_window_name_probe(
            &mut opener,
            3,
            "window.oldDocument=p.document;window.readOldDocument=p.Function('return document');window.readOldBody=p.Function('return document.body.textContent');window.savedDocumentGetter=p.Object.getOwnPropertyDescriptor(p,'document').get;true",
        )
        .await;
        navigate_dynamic_page_and_wait_for_load(&mut child, 3, &format!("{destination}/two")).await;
        assert_eq!(
            evaluate_window_name_probe(
                &mut opener,
                4,
                "[readOldDocument()===oldDocument,readOldBody(),oldDocument.body.textContent]",
            )
            .await,
            json!([true, "/one", "/one"])
        );
        assert_eq!(
            evaluate_window_name_probe(
                &mut opener,
                5,
                "[Object.getOwnPropertyDescriptor(window,'document').get,savedDocumentGetter,Object.getOwnPropertyDescriptor(__moliNativeBridge.window,'document').get].map(getter=>{try{return getter.call(p).body.textContent}catch(e){return e.name}})",
            )
            .await,
            if destination == &base {
                json!(["/two", "/two", "/two"])
            } else {
                json!(["SecurityError", "SecurityError", "SecurityError"])
            }
        );
        assert_eq!(
            evaluate_window_name_probe(&mut child, 4, "document.body.textContent").await,
            "/two"
        );
        evaluate_window_name_probe(&mut child, 5, "window.close();true").await;
        wait_for_target_list(addr, "closed popup removed", |targets| {
            !targets.iter().any(|target| target["id"] == child_id)
        })
        .await;
    }
    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn initial_popup_retirement_rejects_frames_created_by_navigation_cancellation() {
    let release = Arc::new(tokio::sync::Notify::new());
    let response = Arc::clone(&release);
    let (fixture_addr, _fixture) = spawn_dedicated_fixture_server(
        Router::new().fallback(get(move |uri: axum::http::Uri| {
            let response = Arc::clone(&response);
            async move {
                if uri.path() == "/child" {
                    response.notified().await;
                }
                axum::response::Html("<p>loaded</p>")
            }
        })),
        "popup-retirement-reentry",
    );
    let base = format!("http://{fixture_addr}");
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .unwrap();
    let opener_id = create_dynamic_target(&mut browser, 1).await;
    let mut opener = connect_dynamic_page(addr, &opener_id).await;
    send_cdp_command(&mut opener, 1, "Page.enable", None, json!({})).await;
    navigate_dynamic_page_and_wait_for_load(&mut opener, 2, &format!("{base}/opener")).await;
    let (_child_id, mut child) = open_auxiliary_with_url(
        addr,
        &mut opener,
        "/child",
        "p.document.body.innerHTML='<iframe src=/frame></iframe>'",
    )
    .await;
    wait_for_value(
        &mut opener,
        "p.document.querySelector('iframe').contentDocument.URL.endsWith('/frame') && p.document.querySelector('iframe').contentDocument.readyState==='complete'",
        json!(true),
    )
    .await;
    assert_eq!(
        evaluate_window_name_probe(
            &mut opener,
            2,
            r#"
        p.eval(`(() => {
          const w = document.querySelector('iframe').contentWindow;
          w.navigation.addEventListener('navigate', event => {
            event.intercept({handler: () => new Promise(() => {})});
          }, {once: true});
          w.navigation.addEventListener('navigateerror', () => {
            const next = document.createElement('iframe');
            document.body.appendChild(next);
            window.canceled = true;
            window.saved = next.contentWindow;
          }, {once: true});
          w.navigation.navigate(w.location.href + '#pending').finished.catch(() => {});
          return true;
        })()`)
    "#,
        )
        .await,
        true
    );
    release.notify_one();
    wait_for_value(&mut child, "document.URL", json!(format!("{base}/child"))).await;
    assert_eq!(
        evaluate_window_name_probe(
            &mut child,
            3,
            "[window.canceled,window.saved===null,document.querySelectorAll('iframe').length]",
        )
        .await,
        json!([true, true, 0])
    );
    assert_eq!(
        evaluate_window_name_probe(&mut opener, 3, "p.closed").await,
        false
    );
    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn initial_popup_commit_keeps_window_timers_and_service_worker_completion() {
    let release = Arc::new(tokio::sync::Notify::new());
    let response = Arc::clone(&release);
    let (fixture_addr, _fixture) = spawn_dedicated_fixture_server(
        Router::new().fallback(get(move |uri: axum::http::Uri| {
            let response = Arc::clone(&response);
            async move {
                if uri.path() == "/worker.js" {
                    return (
                        [("content-type", "text/javascript")],
                        r#"
                        self.addEventListener('install', e => e.waitUntil(self.skipWaiting()));
                        self.addEventListener('activate', e => e.waitUntil(self.clients.claim()));
                        self.addEventListener('message', e => e.waitUntil(
                          self.clients.matchAll({includeUncontrolled:true, type:'window'})
                            .then(clients => e.source.postMessage(clients.map(c => c.url)))
                        ));
                    "#,
                    );
                }
                if uri.path() == "/child" {
                    response.notified().await;
                }
                ([("content-type", "text/html")], "<p>loaded</p>")
            }
        })),
        "initial-popup-window-tasks",
    );
    let base = format!("http://{fixture_addr}");
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .unwrap();
    let opener_id = create_dynamic_target(&mut browser, 1).await;
    let mut opener = connect_dynamic_page(addr, &opener_id).await;
    send_cdp_command(&mut opener, 1, "Page.enable", None, json!({})).await;
    navigate_dynamic_page_and_wait_for_load(&mut opener, 2, &format!("{base}/opener")).await;
    send_cdp_command(
        &mut browser,
        20,
        "Target.setAutoAttach",
        None,
        json!({"autoAttach":true,"waitForDebuggerOnStart":true,"flatten":true}),
    )
    .await;
    let (child_id, mut child) = open_auxiliary_with_url(addr, &mut opener, "/child", "").await;
    evaluate_window_name_probe(&mut child, 2, "window.ticks=0;window.timerFired=false;window.timeoutId=setTimeout(()=>timerFired=true,900);window.intervalId=setInterval(()=>ticks++,200);true").await;
    wait_for_value(&mut child, "ticks>0", json!(true)).await;
    let ticks = evaluate_window_name_probe(&mut child, 3, "ticks")
        .await
        .as_u64()
        .unwrap();
    release.notify_one();
    resume_auxiliary(&mut browser, &child_id).await;
    wait_for_value(&mut child, "document.URL", json!(format!("{base}/child"))).await;
    wait_for_value(&mut child, "timerFired", json!(true)).await;
    wait_for_value(&mut child, &format!("ticks>{ticks}"), json!(true)).await;
    send_cdp_command(
        &mut browser,
        22,
        "Target.setAutoAttach",
        None,
        json!({"autoAttach":false,"waitForDebuggerOnStart":false,"flatten":true}),
    )
    .await;
    evaluate_window_name_probe(&mut child, 4, "window.registered='pending';window.controllerChanges=0;navigator.serviceWorker.oncontrollerchange=()=>controllerChanges++;navigator.serviceWorker.register('/worker.js').then(()=>registered='success',e=>registered=e.name);true").await;
    wait_for_value(&mut child, "registered", json!("success")).await;
    wait_for_value(
        &mut child,
        "[navigator.serviceWorker.controller?.scriptURL,controllerChanges]",
        json!([format!("{base}/worker.js"), 1]),
    )
    .await;
    evaluate_window_name_probe(&mut child, 6, "window.clientURLs=null;navigator.serviceWorker.onmessage=e=>clientURLs=e.data;navigator.serviceWorker.controller.postMessage('clients');true").await;
    wait_for_value(
        &mut child,
        "clientURLs?.sort()",
        json!([format!("{base}/child"), format!("{base}/opener")]),
    )
    .await;
    assert_eq!(
        evaluate_window_name_probe(&mut child, 5, "clearInterval(intervalId);typeof timeoutId")
            .await,
        "number"
    );
    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn isolated_popup_navigation_releases_indexed_db_connections() {
    let release = Arc::new(tokio::sync::Notify::new());
    let response = Arc::clone(&release);
    let (fixture_addr, _fixture) = spawn_dedicated_fixture_server(
        Router::new().fallback(get(move |uri: axum::http::Uri| {
            let response = Arc::clone(&response);
            async move {
                if uri.path() == "/child" {
                    response.notified().await;
                }
                axum::response::Html("<p>loaded</p>")
            }
        })),
        "isolated-popup-idb-retirement",
    );
    let base = format!("http://{fixture_addr}");
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .unwrap();
    let opener_id = create_dynamic_target(&mut browser, 1).await;
    let mut opener = connect_dynamic_page(addr, &opener_id).await;
    send_cdp_command(&mut opener, 1, "Page.enable", None, json!({})).await;
    navigate_dynamic_page_and_wait_for_load(&mut opener, 2, &format!("{base}/opener")).await;
    send_cdp_command(
        &mut browser,
        20,
        "Target.setAutoAttach",
        None,
        json!({"autoAttach":true,"waitForDebuggerOnStart":true,"flatten":true}),
    )
    .await;
    let (child_id, mut child) = open_auxiliary_with_url(addr, &mut opener, "/child", "").await;
    let tree = send_cdp_command(&mut child, 2, "Page.getFrameTree", None, json!({})).await;
    let world = send_cdp_command(&mut child, 3, "Page.createIsolatedWorld", None, json!({"frameId":response_by_id(&tree,2)["result"]["frameTree"]["frame"]["id"],"worldName":"idb-retirement"})).await;
    let context_id = response_by_id(&world, 3)["result"]["executionContextId"].clone();
    let opened = send_cdp_command(&mut child, 4, "Runtime.evaluate", None, json!({"contextId":context_id,"expression":"new Promise((resolve,reject)=>{window.retained=new Array(1024*1024).fill(3);const r=indexedDB.open('isolated-retirement',1);r.onsuccess=()=>{window.db=r.result;resolve('opened')};r.onerror=()=>reject(r.error)})","awaitPromise":true,"returnByValue":true})).await;
    assert_eq!(
        response_by_id(&opened, 4)["result"]["result"]["value"],
        "opened"
    );
    release.notify_one();
    resume_auxiliary(&mut browser, &child_id).await;
    wait_for_value(&mut child, "document.URL", json!(format!("{base}/child"))).await;
    evaluate_window_name_probe(&mut child, 5, "window.upgrade='pending';const r=indexedDB.open('isolated-retirement',2);r.onblocked=()=>upgrade='blocked';r.onerror=()=>upgrade=r.error.name;r.onsuccess=()=>{upgrade='success';r.result.close()};true").await;
    wait_for_value(&mut child, "upgrade", json!("success")).await;
    abort_test_cdp_server(server).await;
}

#[tokio::test]
async fn idb_sqlite_popup_close_retires_its_connection_and_preserves_the_opener() {
    let (fixture_addr, _fixture) = spawn_dedicated_fixture_server(
        Router::new().fallback(get(|| async { axum::response::Html("<p>opener</p>") })),
        "popup-indexeddb-close-retirement",
    );
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .unwrap();
    let opener_id = create_dynamic_target(&mut browser, 1).await;
    let mut opener = connect_dynamic_page(addr, &opener_id).await;
    send_cdp_command(&mut opener, 1, "Page.enable", None, json!({})).await;
    navigate_dynamic_page_and_wait_for_load(
        &mut opener,
        2,
        &format!("http://{fixture_addr}/opener"),
    )
    .await;
    let seeded = send_cdp_command(&mut opener, 3, "Runtime.evaluate", None, json!({
        "awaitPromise":true,"returnByValue":true,"expression":r#"(async () => {
            const request = indexedDB.open('opener-retained', 1);
            request.onupgradeneeded = () => request.result.createObjectStore('records').put('opener',1);
            globalThis.keeper = await new Promise((resolve,reject) => {
                request.onsuccess = () => resolve(request.result);
                request.onerror = () => reject(request.error);
            });
            return 'opened';
        })()"#
    })).await;
    assert_eq!(
        response_by_id(&seeded, 3)["result"]["result"]["value"],
        "opened",
        "{seeded:?}"
    );
    let (child_id, mut child) = open_auxiliary(addr, &mut opener, "").await;
    let opened = send_cdp_command(&mut child, 4, "Runtime.evaluate", None, json!({
        "awaitPromise":true,"returnByValue":true,"expression":r#"(async () => {
            const request = indexedDB.open('popup-retired', 1);
            request.onupgradeneeded = () => request.result.createObjectStore('records').put('popup',1);
            globalThis.popupHeldDatabase = await new Promise((resolve,reject) => {
                request.onsuccess = () => resolve(request.result);
                request.onerror = () => reject(request.error);
            });
            return 'opened';
        })()"#
    })).await;
    assert_eq!(
        response_by_id(&opened, 4)["result"]["result"]["value"],
        "opened",
        "{opened:?}"
    );
    // Keeping the real foreign wrapper in the opener prevents GC from hiding
    // a missing connection retirement when the popup's native owner closes.
    assert_eq!(
        evaluate_window_name_probe(
            &mut opener,
            5,
            "globalThis.popupHeldDatabase=p.popupHeldDatabase;p.close();p.closed"
        )
        .await,
        true
    );
    wait_for_target_list(addr, "closed popup owner removed", |targets| {
        targets.iter().all(|target| target["id"] != child_id)
    })
    .await;
    let result = send_cdp_command(
        &mut opener,
        6,
        "Runtime.evaluate",
        None,
        json!({
            "awaitPromise":true,"returnByValue":true,"expression":r#"(async () => {
            const request = indexedDB.open('popup-retired', 2);
            const upgraded = await new Promise((resolve,reject) => {
                request.onblocked = () => reject(Error('closed popup still blocks upgrade'));
                request.onerror = () => reject(request.error);
                request.onsuccess = () => resolve(request.result);
            });
            const read = db => new Promise((resolve,reject) => {
                const request = db.transaction('records').objectStore('records').get(1);
                request.onsuccess = () => resolve(request.result);
                request.onerror = () => reject(request.error);
            });
            const value = await read(upgraded);
            const retained = await read(keeper);
            const result = {closed:p.closed,version:upgraded.version,value,retained,
                retainedName:popupHeldDatabase.name};
            upgraded.close();keeper.close();return result;
        })()"#
        }),
    )
    .await;
    assert_eq!(
        response_by_id(&result, 6)["result"]["result"]["value"],
        json!({
            "closed":true,"version":2,"value":"popup","retained":"opener",
            "retainedName":"popup-retired"
        }),
        "{result:?}"
    );
    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn iframe_popup_location_navigation_uses_incumbent_document_referrer() {
    let requests = Arc::new(parking_lot::Mutex::new(Vec::<(String, String)>::new()));
    let captured = Arc::clone(&requests);
    let (fixture_addr, _fixture) = spawn_dedicated_fixture_server(
        Router::new().fallback(get(
            move |uri: axum::http::Uri, headers: axum::http::HeaderMap| {
                let captured = Arc::clone(&captured);
                async move {
                    captured.lock().push((
                        uri.path().to_owned(),
                        headers
                            .get("referer")
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or("")
                            .to_owned(),
                    ));
                    axum::response::Html(if uri.path() == "/frame/inner" {
                        "<script>onmessage=()=>parent.p.location.href='next'</script>"
                    } else {
                        "<p>loaded</p>"
                    })
                }
            },
        )),
        "iframe-popup-location-referrer",
    );
    let base = format!("http://{fixture_addr}");
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .unwrap();
    let opener_id = create_dynamic_target(&mut browser, 1).await;
    let mut opener = connect_dynamic_page(addr, &opener_id).await;
    send_cdp_command(&mut opener, 1, "Page.enable", None, json!({})).await;
    navigate_dynamic_page_and_wait_for_load(&mut opener, 2, &format!("{base}/source/start")).await;
    let (_, mut child) = open_auxiliary(addr, &mut opener, "").await;
    evaluate_window_name_probe(
        &mut opener,
        3,
        "document.body.innerHTML='<iframe src=/frame/inner></iframe>';true",
    )
    .await;
    wait_for_value(&mut opener,"document.querySelector('iframe').contentDocument.URL.endsWith('/frame/inner') && document.querySelector('iframe').contentDocument.readyState==='complete'",json!(true)).await;
    evaluate_window_name_probe(
        &mut opener,
        4,
        "document.querySelector('iframe').contentWindow.postMessage('navigate','*');true",
    )
    .await;
    wait_for_value(
        &mut child,
        "document.URL",
        json!(format!("{base}/frame/next")),
    )
    .await;
    assert_eq!(
        evaluate_window_name_probe(&mut child, 3, "document.referrer").await,
        format!("{base}/frame/inner")
    );
    assert_eq!(
        requests
            .lock()
            .iter()
            .find(|(path, _)| path == "/frame/next")
            .unwrap()
            .1,
        format!("{base}/frame/inner")
    );
    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cross_window_blank_reload_keeps_target_base_and_referrer() {
    let (fixture_addr, _fixture) = spawn_dedicated_fixture_server(
        Router::new().fallback(get(|| async { axum::response::Html("<p>loaded</p>") })),
        "cross-window-blank-reload-environment",
    );
    let base = format!("http://{fixture_addr}");
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .unwrap();
    let opener_id = create_dynamic_target(&mut browser, 1).await;
    let mut opener = connect_dynamic_page(addr, &opener_id).await;
    send_cdp_command(&mut opener, 1, "Page.enable", None, json!({})).await;
    navigate_dynamic_page_and_wait_for_load(&mut opener, 2, &format!("{base}/source/start")).await;
    let (_,mut child) = open_auxiliary(addr,&mut opener, "p.document.head.innerHTML='<base href=\"https://popup-base.test/dir/\">';p.document.body.textContent='old'").await;
    evaluate_window_name_probe(&mut opener, 3, "p.location.reload();true").await;
    wait_for_value(&mut child, "document.body.textContent", json!("")).await;
    assert_eq!(
        evaluate_window_name_probe(&mut child, 3, "[document.baseURI,document.referrer,origin]")
            .await,
        json!(["https://popup-base.test/dir/", "", base])
    );
    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cross_origin_location_functions_accept_other_location_receivers() {
    let (fixture_addr, _fixture) = spawn_dedicated_fixture_server(
        Router::new().fallback(get(|| async { axum::response::Html("<p>loaded</p>") })),
        "borrowed-cross-origin-location-functions",
    );
    let base = format!("http://{fixture_addr}");
    let cross = format!("http://localhost:{}", fixture_addr.port());
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .unwrap();
    let opener_id = create_dynamic_target(&mut browser, 1).await;
    let mut opener = connect_dynamic_page(addr, &opener_id).await;
    send_cdp_command(&mut opener, 1, "Page.enable", None, json!({})).await;
    navigate_dynamic_page_and_wait_for_load(&mut opener, 2, &format!("{base}/opener")).await;
    let (_, mut child) = open_auxiliary(addr, &mut opener, "").await;
    navigate_dynamic_page_and_wait_for_load(&mut child, 2, &format!("{cross}/child")).await;
    assert_eq!(
        evaluate_window_name_probe(
            &mut opener,
            3,
            r#"
        (()=>{
          const setter=Object.getOwnPropertyDescriptor(p.location,'href').set;
          const replace=p.location.replace;
          const attempt=f=>{try{f();return 'allowed'}catch(e){return e.name}};
          const invalid=[attempt(()=>setter.call({},'#bad')),attempt(()=>replace.call({},'#bad'))];
          setter.call(location,'#setter');
          const first=location.hash;
          replace.call(location,'#replace');
          return [first,location.hash,...invalid];
        })()
    "#
        )
        .await,
        json!(["#setter", "#replace", "TypeError", "TypeError"])
    );
    assert_eq!(
        evaluate_window_name_probe(&mut child, 3, "location.href").await,
        format!("{cross}/child")
    );
    abort_test_cdp_server(server).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn saved_popup_locations_cannot_navigate_a_replacement_document() {
    let (fixture_addr, _fixture) = spawn_dedicated_fixture_server(
        Router::new().fallback(get(|| async { axum::response::Html("<p>loaded</p>") })),
        "saved-popup-location-retirement",
    );
    let base = format!("http://{fixture_addr}");
    let cross = format!("http://localhost:{}", fixture_addr.port());
    let (addr, server) = spawn_test_protocol_server().await;
    let (mut browser, _) =
        connect_async(format!("ws://{addr}/devtools/browser/{DEFAULT_BROWSER_ID}"))
            .await
            .unwrap();
    let opener_id = create_dynamic_target(&mut browser, 1).await;
    let mut opener = connect_dynamic_page(addr, &opener_id).await;
    send_cdp_command(&mut opener, 1, "Page.enable", None, json!({})).await;
    navigate_dynamic_page_and_wait_for_load(&mut opener, 2, &format!("{base}/opener")).await;
    for target in [&base, &cross] {
        let (child_id, mut child) = open_auxiliary(addr, &mut opener, "").await;
        navigate_dynamic_page_and_wait_for_load(&mut child, 2, &format!("{target}/one")).await;
        evaluate_window_name_probe(&mut opener, 3, "window.savedLocation=p.location;true").await;
        navigate_dynamic_page_and_wait_for_load(&mut child, 3, &format!("{target}/two")).await;
        assert_eq!(
            evaluate_window_name_probe(
                &mut opener,
                4,
                &format!(
                    "savedLocation.replace({});true",
                    json!(format!("{target}/stale"))
                )
            )
            .await,
            true
        );
        // A subsequent command observes any synchronously admitted navigation.
        evaluate_window_name_probe(&mut child, 4, "true").await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(
            evaluate_window_name_probe(&mut child, 5, "location.href").await,
            format!("{target}/two")
        );
        evaluate_window_name_probe(&mut child, 6, "window.close();true").await;
        wait_for_target_list(addr, "closed popup removed", |targets| {
            !targets.iter().any(|target| target["id"] == child_id)
        })
        .await;
    }
    abort_test_cdp_server(server).await;
}
