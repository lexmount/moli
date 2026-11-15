use super::*;
use tokio::sync::Notify;

fn fixture() -> Router {
    Router::new()
        .route(
            "/page",
            get(|| async { ([(CONTENT_TYPE, "text/html")], "<!doctype html><body>parent") }),
        )
        .route(
            "/initial",
            get(|| async { ([(CONTENT_TYPE, "text/html")], "<!doctype html><p>initial") }),
        )
        .route(
            "/second",
            get(|| async { ([(CONTENT_TYPE, "text/html")], "<!doctype html><p>second") }),
        )
        .route(
            "/cache",
            get(|| async {
                (
                    [
                        ("content-type", "text/html"),
                        ("cache-control", "public, max-age=3600"),
                    ],
                    "<!doctype html><p>cached",
                )
            }),
        )
        .route("/no-content", get(|| async { StatusCode::NO_CONTENT }))
        .route(
            "/script-child",
            get(|| async {
                (
                    [(CONTENT_TYPE, "text/html")],
                    "<!doctype html><script src='/same.js'></script>",
                )
            }),
        )
        .route(
            "/redirect-child",
            get(|| async {
                (
                    [(CONTENT_TYPE, "text/html")],
                    "<!doctype html><script src='/redirect.js'></script>",
                )
            }),
        )
        .route(
            "/redirect.js",
            get(|| async { (StatusCode::FOUND, [("location", "/final.js")]) }),
        )
        .route(
            "/final.js",
            get(|| async {
                (
                    [(CONTENT_TYPE, "text/javascript")],
                    "console.log('redirect executed')",
                )
            }),
        )
        .route(
            "/sw-child",
            get(|| async {
                (
                    [(CONTENT_TYPE, "text/html")],
                    "<!doctype html><script src='/held-script.js'></script>",
                )
            }),
        )
        .route(
            "/sw.js",
            get(|| async { ([(CONTENT_TYPE, "text/javascript")], WORKER) }),
        )
}

async fn serve(router: Router) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (base, task)
}

async fn context(base: &str) -> TestContext {
    let mut ctx = TestContext::new();
    with_loaded_http_document(&mut ctx, &format!("{base}/page"), "SID-1", "TID-1").await;
    enable_runtime_async(&mut ctx, "SID-1", 72000).await;
    ctx
}

async fn evaluate(ctx: &mut TestContext, id: u64, expression: &str) {
    ctx.process_async(json!({"id":id,"sessionId":"SID-1","method":"Runtime.evaluate","params":{"expression":expression}})).await;
    let response = take_response_by_id(ctx, id);
    assert!(response.get("error").is_none(), "{response}");
    assert!(
        response["result"].get("exceptionDetails").is_none(),
        "{response}"
    );
}

async fn console(ctx: &mut TestContext, message: &str) {
    ctx.wait_for_scheduler_message("child regression console signal", |m| {
        m["method"] == "Runtime.consoleAPICalled" && m["params"]["args"][0]["value"] == message
    })
    .await;
}

async fn append(ctx: &mut TestContext, base: &str, path: &str) -> Value {
    evaluate(ctx, 72001, &format!("globalThis.frame=document.createElement('iframe');frame.src={};document.body.appendChild(frame)", json!(path))).await;
    let committed = ctx
        .wait_for_scheduler_message("initial child commit", |m| {
            m["method"] == "Page.frameNavigated"
                && m["params"]["frame"]["url"] == format!("{base}{path}")
        })
        .await;
    let frame = committed["params"]["frame"].clone();
    wait_until_frame_stopped_loading(ctx, frame["id"].as_str().unwrap()).await;
    frame
}

#[tokio::test(flavor = "multi_thread")]
async fn detaching_shared_script_leader_does_not_strand_other_frames_or_cache() {
    let hits = Arc::new(AtomicUsize::new(0));
    let release = Arc::new(Notify::new());
    let started = Arc::new(Notify::new());
    let (base, server) = serve(fixture().route(
        "/same.js",
        get({
            let hits = hits.clone();
            let release = release.clone();
            let started = started.clone();
            move || {
                let hits = hits.clone();
                let release = release.clone();
                let started = started.clone();
                async move {
                    hits.fetch_add(1, Ordering::SeqCst);
                    started.notify_one();
                    release.notified().await;
                    (
                        [
                            ("content-type", "text/javascript"),
                            ("cache-control", "public, max-age=3600"),
                        ],
                        "parent.console.log('executed:'+frameElement.id)",
                    )
                }
            }
        }),
    ))
    .await;
    let mut ctx = context(&base).await;
    evaluate(&mut ctx, 72010, "globalThis.first=document.createElement('iframe');first.id='first';first.src='/script-child';document.body.appendChild(first)").await;
    let first_request = ctx
        .wait_for_scheduler_message("first script request", |m| {
            m["method"] == "Network.requestWillBeSent"
                && m["params"]["request"]["url"] == format!("{base}/same.js")
        })
        .await;
    started.notified().await;
    evaluate(&mut ctx, 72011, "globalThis.second=document.createElement('iframe');second.id='second';second.src='/script-child';document.body.appendChild(second)").await;
    let first_frame = first_request["params"]["frameId"].clone();
    ctx.wait_for_scheduler_message("second script consumer", |m| {
        m["method"] == "Network.requestWillBeSent"
            && m["params"]["request"]["url"] == format!("{base}/same.js")
            && m["params"]["frameId"] != first_frame
    })
    .await;
    evaluate(&mut ctx, 72012, "first.remove()").await;
    release.notify_one();
    console(&mut ctx, "executed:second").await;
    evaluate(&mut ctx, 72013, "globalThis.third=document.createElement('iframe');third.id='third';third.src='/script-child';document.body.appendChild(third)").await;
    console(&mut ctx, "executed:third").await;
    assert_eq!(
        hits.load(Ordering::SeqCst),
        1,
        "shared physical script load and completed cache must survive one consumer's detach"
    );
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn superseded_child_navigate_does_not_adopt_the_successor_loader() {
    check_superseded_child_navigation(false).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn superseded_child_navigate_aborts_while_successor_response_is_pending() {
    check_superseded_child_navigation(true).await;
}

async fn check_superseded_child_navigation(hold_successor: bool) {
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let second_entered = Arc::new(Notify::new());
    let second_release = Arc::new(Notify::new());
    let router = fixture()
        .route(
            "/slow",
            get({
                let entered = entered.clone();
                let release = release.clone();
                move || {
                    let entered = entered.clone();
                    let release = release.clone();
                    async move {
                        entered.notify_one();
                        release.notified().await;
                        ([(CONTENT_TYPE, "text/html")], "<!doctype html><p>slow")
                    }
                }
            }),
        )
        .route(
            "/held-second",
            get({
                let entered = second_entered.clone();
                let release = second_release.clone();
                move || {
                    let entered = entered.clone();
                    let release = release.clone();
                    async move {
                        entered.notify_one();
                        release.notified().await;
                        ([(CONTENT_TYPE, "text/html")], "<!doctype html><p>second")
                    }
                }
            }),
        );
    let (base, server) = serve(router).await;
    let mut ctx = context(&base).await;
    let frame = append(&mut ctx, &base, "/initial").await;
    let command = crate::ParsedCdpCommand::from_serializable(json!({"id":72100,"sessionId":"SID-1","method":"Page.navigate","params":{"frameId":frame["id"],"url":format!("{base}/slow")}})).unwrap();
    let mut first_context = crate::conn::CommandDispatchContext::default();
    let step = ctx.with_fixture(|conn| {
        conn.start_parsed_command_dispatch_with_context(&command, &mut first_context)
    });
    let crate::CdpCommandTaskStep::Pending(first_pending) = step else {
        panic!("slow navigation must be pending")
    };
    let mut first_wait = Box::pin(first_pending.wait());
    ctx.drive_owned_wait(async {
        tokio::select! {
            () = entered.notified() => {},
            _ = &mut first_wait => panic!("slow navigation completed before its gated response"),
        }
    })
    .await;
    let first_request = ctx
        .wait_for_scheduler_message("first navigation request identity", |m| {
            m["method"] == "Network.requestWillBeSent"
                && m["params"]["request"]["url"] == format!("{base}/slow")
        })
        .await;
    let second_path = if hold_successor {
        "/held-second"
    } else {
        "/second"
    };
    let command = crate::ParsedCdpCommand::from_serializable(json!({"id":72101,"sessionId":"SID-1","method":"Page.navigate","params":{"frameId":frame["id"],"url":format!("{base}{second_path}")}})).unwrap();
    let mut second_context = crate::conn::CommandDispatchContext::default();
    let step = ctx.with_fixture(|conn| {
        conn.start_parsed_command_dispatch_with_context(&command, &mut second_context)
    });
    let crate::CdpCommandTaskStep::Pending(second_pending) = step else {
        panic!("successor navigation must be dispatched")
    };
    let mut second_wait = Box::pin(second_pending.wait());
    let (first_completed, second_completed) = if hold_successor {
        ctx.drive_owned_wait(async {
            tokio::select! {
                () = second_entered.notified() => {},
                _ = &mut second_wait => panic!("successor completed before its gated response"),
            }
        })
        .await;
        let first = ctx.drive_owned_wait(async {
            tokio::select! {
                first = &mut first_wait => first,
                _ = &mut second_wait => panic!("cancellation waited for the successor response"),
            }
        }).await;
        (first, None)
    } else {
        let (first, second) = ctx
            .drive_owned_wait(async { tokio::join!(&mut first_wait, &mut second_wait) })
            .await;
        (first, Some(second))
    };
    let step = ctx.with_fixture(|conn| {
        conn.complete_command_dispatch_with_context(first_completed, &mut first_context)
    });
    let (messages, _) = ctx
        .complete_command_task_step_with_context_for_test(step, &mut first_context)
        .await;
    let first = messages.into_iter().find(|m| m["id"] == 72100).unwrap();
    assert_eq!(
        first["result"]["errorText"], "net::ERR_ABORTED",
        "the canceled navigation must finish before releasing its successor: {first}"
    );
    assert_eq!(
        first["result"]["loaderId"], first_request["params"]["loaderId"],
        "the canceled command retains its own native loader identity"
    );
    second_release.notify_one();
    let second_completed = match second_completed {
        Some(second) => second,
        None => ctx.drive_owned_wait(second_wait).await,
    };
    let step = ctx.with_fixture(|conn| {
        conn.complete_command_dispatch_with_context(second_completed, &mut second_context)
    });
    let (messages, _) = ctx
        .complete_command_task_step_with_context_for_test(step, &mut second_context)
        .await;
    let second = messages.into_iter().find(|m| m["id"] == 72101).unwrap();
    assert!(second["result"]["loaderId"].is_string(), "{second}");
    assert!(second["result"]["errorText"].is_null(), "{second}");
    assert_ne!(first["result"]["loaderId"], second["result"]["loaderId"]);
    release.notify_one();
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn committed_child_navigate_keeps_its_result_after_a_later_commit() {
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let (base, server) = serve(fixture().route(
        "/committing",
        get({
            let entered = entered.clone();
            let release = release.clone();
            move || {
                let entered = entered.clone();
                let release = release.clone();
                async move {
                    entered.notify_one();
                    release.notified().await;
                    (
                        [(CONTENT_TYPE, "text/html")],
                        "<!doctype html><p>first commit",
                    )
                }
            }
        }),
    ))
    .await;
    let mut ctx = context(&base).await;
    let frame = append(&mut ctx, &base, "/initial").await;
    let command = crate::ParsedCdpCommand::from_serializable(json!({"id":72110,"sessionId":"SID-1","method":"Page.navigate","params":{"frameId":frame["id"],"url":format!("{base}/committing")}})).unwrap();
    let mut first_context = crate::conn::CommandDispatchContext::default();
    let step = ctx.with_fixture(|conn| {
        conn.start_parsed_command_dispatch_with_context(&command, &mut first_context)
    });
    let crate::CdpCommandTaskStep::Pending(pending) = step else {
        panic!("first navigation must be pending")
    };
    let mut first_wait = Box::pin(pending.wait());
    ctx.drive_owned_wait(async {
        tokio::select! {
            () = entered.notified() => {},
            _ = &mut first_wait => panic!("navigation completed before its gated response"),
        }
    })
    .await;
    release.notify_one();
    let first_commit = ctx
        .wait_for_scheduler_message(
            "first native commit before reading its command result",
            |m| {
                m["method"] == "Page.frameNavigated"
                    && m["params"]["frame"]["id"] == frame["id"]
                    && m["params"]["frame"]["url"] == format!("{base}/committing")
            },
        )
        .await;
    ctx.process_async(json!({"id":72111,"sessionId":"SID-1","method":"Page.navigate","params":{"frameId":frame["id"],"url":format!("{base}/second")}})).await;
    let second = ctx.sent.iter().find(|m| m["id"] == 72111).cloned().unwrap();
    assert!(second["result"]["loaderId"].is_string(), "{second}");
    let completed = ctx.drive_owned_wait(first_wait).await;
    let step = ctx.with_fixture(|conn| {
        conn.complete_command_dispatch_with_context(completed, &mut first_context)
    });
    let (messages, _) = ctx
        .complete_command_task_step_with_context_for_test(step, &mut first_context)
        .await;
    let first = messages.into_iter().find(|m| m["id"] == 72110).unwrap();
    assert!(
        first["result"]["errorText"].is_null(),
        "an already committed navigation remains successful: {first}"
    );
    assert_eq!(
        first["result"]["loaderId"],
        first_commit["params"]["frame"]["loaderId"]
    );
    assert_ne!(first["result"]["loaderId"], second["result"]["loaderId"]);
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn child_navigate_completes_at_commit_while_its_script_response_is_pending() {
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let (base, server) = serve(fixture().route(
        "/same.js",
        get({
            let entered = entered.clone();
            let release = release.clone();
            move || {
                let entered = entered.clone();
                let release = release.clone();
                async move {
                    entered.notify_one();
                    release.notified().await;
                    (
                        [(CONTENT_TYPE, "text/javascript")],
                        "console.log('script released after navigation reply')",
                    )
                }
            }
        }),
    ))
    .await;
    let mut ctx = context(&base).await;
    let frame = append(&mut ctx, &base, "/initial").await;
    let command = crate::ParsedCdpCommand::from_serializable(json!({"id":72120,"sessionId":"SID-1","method":"Page.navigate","params":{"frameId":frame["id"],"url":format!("{base}/script-child")}})).unwrap();
    let mut command_context = crate::conn::CommandDispatchContext::default();
    let step = ctx.with_fixture(|conn| {
        conn.start_parsed_command_dispatch_with_context(&command, &mut command_context)
    });
    let crate::CdpCommandTaskStep::Pending(pending) = step else {
        panic!("child navigation must be dispatched")
    };
    let (_, completed) = ctx
        .drive_owned_wait(async { tokio::join!(entered.notified(), pending.wait()) })
        .await;
    let step = ctx.with_fixture(|conn| {
        conn.complete_command_dispatch_with_context(completed, &mut command_context)
    });
    let (messages, _) = ctx
        .complete_command_task_step_with_context_for_test(step, &mut command_context)
        .await;
    let result = messages.iter().find(|m| m["id"] == 72120).unwrap();
    assert!(
        result["result"]["loaderId"].is_string(),
        "navigation must complete while the script response is held: {result}"
    );
    assert!(result["result"]["errorText"].is_null(), "{result}");
    release.notify_one();
    console(&mut ctx, "script released after navigation reply").await;
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn ignored_child_navigation_balances_loading_without_committing() {
    let (base, server) = serve(fixture()).await;
    let mut ctx = context(&base).await;
    let frame = append(&mut ctx, &base, "/initial").await;
    ctx.sent.clear();
    evaluate(&mut ctx, 72200, "frame.src='/no-content'").await;
    let request = ctx
        .wait_for_scheduler_message("204 child request", |m| {
            m["method"] == "Network.requestWillBeSent"
                && m["params"]["request"]["url"] == format!("{base}/no-content")
        })
        .await;
    ctx.wait_for_scheduler_message("204 child network terminal", |m| {
        m["method"] == "Network.loadingFinished"
            && m["params"]["requestId"] == request["params"]["requestId"]
    })
    .await;
    wait_until_frame_stopped_loading(&mut ctx, frame["id"].as_str().unwrap()).await;
    assert_eq!(
        ctx.sent
            .iter()
            .filter(|m| m["method"] == "Page.frameStartedLoading"
                && m["params"]["frameId"] == frame["id"])
            .count(),
        1
    );
    assert_eq!(
        ctx.sent
            .iter()
            .filter(|m| m["method"] == "Page.frameStoppedLoading"
                && m["params"]["frameId"] == frame["id"])
            .count(),
        1
    );
    assert!(
        !ctx.sent
            .iter()
            .any(|m| m["method"] == "Page.frameNavigated")
    );
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn repeated_child_navigation_preserves_cache_attribution() {
    let (base, server) = serve(fixture()).await;
    let cache = std::env::temp_dir().join(format!(
        "moli-child-cache-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut config = moli_fetch::FetchConfig::default();
    config.set_http_cache_dir(Some(cache.display().to_string()));
    let mut ctx = TestContext::from_conn(crate::conn::CdpConnection::new_with_fetch_config(config));
    let mut browser = ctx.with_fixture(|conn| conn.new_browser_context("BID-1".to_owned()));
    browser.set_active_target_id("TID-1");
    browser.attach_active_session("SID-1");
    ctx.with_fixture(|conn| conn.install_browser_context_fixture_for_test(browser));
    ctx.enable_page_events_for_test(Some("SID-1"));
    ctx.install_navigation_fixture_for_session_owner(&format!("{base}/page"), Some("SID-1"))
        .await;
    ctx.with_fixture(|conn| {
        conn.runtime_session_owner_slot_mut(Some("SID-1"))
            .unwrap()
            .enable_primary_network_events()
    });
    enable_runtime_async(&mut ctx, "SID-1", 72000).await;
    let frame = append(&mut ctx, &base, "/cache").await;
    ctx.sent.clear();
    evaluate(&mut ctx, 72300, "frame.src='/cache'").await;
    let next = ctx
        .wait_for_scheduler_message("second cached child commit", |m| {
            m["method"] == "Page.frameNavigated"
                && m["params"]["frame"]["id"] == frame["id"]
                && m["params"]["frame"]["loaderId"] != frame["loaderId"]
        })
        .await;
    let loader = &next["params"]["frame"]["loaderId"];
    let response = ctx
        .wait_for_scheduler_message("cached child response", |m| {
            m["method"] == "Network.responseReceived" && m["params"]["requestId"] == *loader
        })
        .await;
    assert_eq!(response["params"]["response"]["fromDiskCache"], true);
    assert!(
        ctx.sent
            .iter()
            .any(|m| m["method"] == "Network.requestServedFromCache"
                && m["params"]["requestId"] == *loader)
    );
    server.abort();
    drop(ctx);
    let _ = std::fs::remove_dir_all(cache);
}

#[tokio::test(flavor = "multi_thread")]
async fn child_script_redirect_links_fetch_hops() {
    let (base, server) = serve(fixture()).await;
    let mut ctx = context(&base).await;
    ctx.process_async(json!({"id":72400,"sessionId":"SID-1","method":"Fetch.enable","params":{"patterns":[{"resourceType":"Script","requestStage":"Request"}]}})).await;
    evaluate(&mut ctx, 72401, "globalThis.frame=document.createElement('iframe');frame.src='/redirect-child';document.body.appendChild(frame)").await;
    let first = ctx
        .wait_for_scheduler_message("script redirect first pause", |m| {
            m["method"] == "Fetch.requestPaused"
                && m["params"]["request"]["url"] == format!("{base}/redirect.js")
        })
        .await;
    ctx.process_async(json!({"id":72402,"sessionId":"SID-1","method":"Fetch.continueRequest","params":{"requestId":first["params"]["requestId"]}})).await;
    let second = ctx
        .wait_for_scheduler_message("script redirect second pause", |m| {
            m["method"] == "Fetch.requestPaused"
                && m["params"]["request"]["url"] == format!("{base}/final.js")
        })
        .await;
    assert_eq!(
        second["params"]["redirectedRequestId"],
        first["params"]["requestId"]
    );
    assert_eq!(second["params"]["networkId"], first["params"]["networkId"]);
    ctx.process_async(json!({"id":72403,"sessionId":"SID-1","method":"Fetch.continueRequest","params":{"requestId":second["params"]["requestId"]}})).await;
    console(&mut ctx, "redirect executed").await;
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn resource_tree_does_not_list_child_navigations_as_subresources() {
    let (base, server) = serve(fixture()).await;
    let mut ctx = context(&base).await;
    let frame = append(&mut ctx, &base, "/initial").await;
    evaluate(&mut ctx, 72500, "frame.src='/second'").await;
    ctx.wait_for_scheduler_message("second child commit for resource tree", |m| {
        m["method"] == "Page.frameNavigated"
            && m["params"]["frame"]["url"] == format!("{base}/second")
    })
    .await;
    wait_until_frame_stopped_loading(&mut ctx, frame["id"].as_str().unwrap()).await;
    ctx.process_async(json!({"id":72501,"sessionId":"SID-1","method":"Page.getResourceTree"}))
        .await;
    let response = take_response_by_id(&mut ctx, 72501);
    let child = response["result"]["frameTree"]["childFrames"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tree| tree["frame"]["id"] == frame["id"])
        .unwrap();
    assert_eq!(
        child["resources"],
        json!([]),
        "Document navigations belong to the frame, not its subresources: {response}"
    );
    server.abort();
}

const WORKER: &str = r#"
self.addEventListener('install', event => event.waitUntil(self.skipWaiting()));
self.addEventListener('activate', event => event.waitUntil(self.clients.claim()));
let release;
const gate = new Promise(resolve => release = resolve);
self.addEventListener('message', event => { if (event.data === 'release') release(); });
self.addEventListener('fetch', event => {
  const path = new URL(event.request.url).pathname;
  if (path !== '/held-child' && path !== '/held-script.js') return;
  event.respondWith((async () => {
    for (const client of await self.clients.matchAll({includeUncontrolled:true, type:'window'})) {
      client.postMessage('handling:' + path);
    }
    await gate;
    return new Response(path === '/held-child' ? '<!doctype html><p>worker child' : "parent.console.log('worker script executed')", {
      headers: {'content-type': path === '/held-child' ? 'text/html' : 'text/javascript'}
    });
  })());
});
"#;

async fn install_worker(ctx: &mut TestContext) {
    ctx.process_and_wait_for_response_async(json!({"id":72600,"sessionId":"SID-1","method":"Runtime.evaluate","params":{"awaitPromise":true,"expression":"(async()=>{navigator.serviceWorker.addEventListener('message',e=>console.log(e.data));await navigator.serviceWorker.register('/sw.js');await navigator.serviceWorker.ready;if(!navigator.serviceWorker.controller)await new Promise(r=>navigator.serviceWorker.addEventListener('controllerchange',r,{once:true}));return true})()"}})).await;
    let reply = take_response_by_id(ctx, 72600);
    assert_eq!(reply["result"]["result"]["value"], true, "{reply}");
}

#[tokio::test(flavor = "multi_thread")]
async fn detaching_child_aborts_document_while_service_worker_response_is_pending() {
    let (base, server) = serve(fixture()).await;
    let mut ctx = context(&base).await;
    install_worker(&mut ctx).await;
    evaluate(&mut ctx, 72601, "globalThis.frame=document.createElement('iframe');frame.src='/held-child';document.body.appendChild(frame)").await;
    console(&mut ctx, "handling:/held-child").await;
    let request = ctx
        .wait_for_scheduler_message("worker child request", |m| {
            m["method"] == "Network.requestWillBeSent"
                && m["params"]["request"]["url"] == format!("{base}/held-child")
        })
        .await;
    evaluate(&mut ctx, 72602, "frame.remove()").await;
    ctx.wait_for_scheduler_message("detach must terminate before worker release", |m| {
        m["method"] == "Network.loadingFailed"
            && m["params"]["requestId"] == request["params"]["requestId"]
    })
    .await;
    evaluate(
        &mut ctx,
        72603,
        "navigator.serviceWorker.controller.postMessage('release')",
    )
    .await;
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn intercepted_child_script_is_announced_before_service_worker_responds() {
    let (base, server) = serve(fixture()).await;
    let mut ctx = context(&base).await;
    install_worker(&mut ctx).await;
    ctx.process_async(json!({"id":72700,"sessionId":"SID-1","method":"Fetch.enable","params":{"patterns":[{"resourceType":"Script","requestStage":"Request"}]}})).await;
    evaluate(&mut ctx, 72701, "globalThis.frame=document.createElement('iframe');frame.src='/sw-child';document.body.appendChild(frame)").await;
    console(&mut ctx, "handling:/held-script.js").await;
    let request = ctx
        .wait_for_scheduler_message("worker script must be visible before response", |m| {
            m["method"] == "Network.requestWillBeSent"
                && m["params"]["request"]["url"] == format!("{base}/held-script.js")
        })
        .await;
    evaluate(
        &mut ctx,
        72702,
        "navigator.serviceWorker.controller.postMessage('release')",
    )
    .await;
    console(&mut ctx, "worker script executed").await;
    assert_eq!(
        ctx.sent
            .iter()
            .filter(|m| m["method"] == "Network.requestWillBeSent"
                && m["params"]["requestId"] == request["params"]["requestId"])
            .count()
            + 1,
        1
    );
    server.abort();
}
