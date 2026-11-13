use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

async fn append_timing_link(
    page: &mut crate::runtime::PageVmTaskExecutorTestHarness,
    loader: &ResourceRequestClient,
    url: &str,
    rel: &str,
    expected_entries: usize,
) -> serde_json::Value {
    let url = serde_json::to_string(url).unwrap();
    page.eval(&format!(
        "{{ const link = document.createElement('link');\n\
         link.rel = {rel:?}; link.as = 'fetch'; link.crossOrigin = 'anonymous';\n\
         link.href = {url}; document.head.appendChild(link); }}"
    ))
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        page,
        loader,
        &format!("performance.getEntriesByName({url}).length"),
        &expected_entries.to_string(),
        "connected link native timing",
    )
    .await;
    serde_json::from_str(
        &page
            .eval(&format!(
                "JSON.stringify(performance.getEntriesByName({url})[{}].toJSON())",
                expected_entries - 1
            ))
            .unwrap(),
    )
    .unwrap()
}

#[tokio::test]
async fn connected_link_timing_retains_http_and_cache_observations() {
    let server = timing_server().await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = new_storage_page_task_executor_test_vm_with_loader(
        &format!("{}/document.html", server.origin),
        &loader,
    );
    for rel in ["prefetch", "preload"] {
        let url = format!("{}/compressed?kind=gzip&cache=1&link={rel}", server.origin);
        let network = append_timing_link(&mut page, &loader, &url, rel, 1).await;
        assert_eq!(network["initiatorType"], "link");
        assert_eq!(network["responseStatus"], 200);
        assert_eq!(network["encodedBodySize"], 23);
        assert_eq!(network["decodedBodySize"], 3);
        assert_eq!(network["transferSize"], 323);
        assert_eq!(network["nextHopProtocol"], "http/1.1");
        let time = |name: &str| network[name].as_f64().unwrap();
        assert!(time("fetchStart") > 0.0, "{network}");
        assert!(time("requestStart") >= time("connectEnd"), "{network}");
        assert!(time("requestStart") >= time("fetchStart"), "{network}");
        assert!(time("responseStart") >= time("requestStart"), "{network}");
        assert!(time("responseEnd") >= time("responseStart"), "{network}");

        if rel == "prefetch" {
            let cached = append_timing_link(&mut page, &loader, &url, rel, 2).await;
            assert_eq!(cached["transferSize"], 0);
            assert_eq!(cached["deliveryType"], "cache");
            assert_eq!(cached["encodedBodySize"], 23);
            assert_eq!(cached["decodedBodySize"], 3);
            assert_eq!(cached["nextHopProtocol"], "");
            assert_eq!(cached["requestStart"], cached["fetchStart"]);
        }
    }
}

#[tokio::test]
async fn connected_link_timing_masks_cross_origin_network_observations() {
    let server = timing_server().await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = new_storage_page_task_executor_test_vm_with_loader(
        "http://127.0.0.1:9/document.html",
        &loader,
    );
    for rel in ["prefetch", "preload"] {
        for tao in [false, true] {
            let url = format!(
                "{}/compressed?kind=gzip&cors=*&tao={}&link={rel}",
                server.origin,
                if tao { "*" } else { "" }
            );
            let entry = append_timing_link(&mut page, &loader, &url, rel, 1).await;
            assert_eq!(entry["responseStatus"], 200);
            if tao {
                assert!(entry["requestStart"].as_f64().unwrap() > 0.0, "{entry}");
                assert_eq!(entry["nextHopProtocol"], "http/1.1");
                assert_eq!(entry["encodedBodySize"], 23);
                assert_eq!(entry["decodedBodySize"], 3);
                assert_eq!(entry["transferSize"], 323);
            } else {
                for name in [
                    "domainLookupStart",
                    "domainLookupEnd",
                    "connectStart",
                    "connectEnd",
                    "requestStart",
                    "responseStart",
                    "encodedBodySize",
                    "decodedBodySize",
                    "transferSize",
                ] {
                    assert_eq!(entry[name], 0, "{name}: {entry}");
                }
                assert_eq!(entry["nextHopProtocol"], "");
                assert_eq!(entry["deliveryType"], "");
            }
        }
    }
}

struct DomInitiatorServer {
    origin: String,
    requests: Arc<AtomicUsize>,
    _server: TimingServer,
}

async fn dom_initiator_server() -> DomInitiatorServer {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let redirect_url = format!(
        "http://localhost:{}/image.gif",
        listener.local_addr().unwrap().port()
    );
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(AtomicUsize::new(0));
    let request_count = requests.clone();
    let task = tokio::spawn(async move {
        let mut connections = tokio::task::JoinSet::new();
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request_count = request_count.clone();
            let redirect_url = redirect_url.clone();
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
                request_count.fetch_add(1, Ordering::SeqCst);
                if path.starts_with("/redirect-image") {
                    let head = format!("HTTP/1.1 302 Found\r\nLocation: {redirect_url}\r\nContent-Length: 0\r\nCache-Control: no-store\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n");
                    let _ = socket.write_all(head.as_bytes()).await;
                    return;
                }
                let (mime, body): (&str, &[u8]) = match path.split('?').next().unwrap() {
                    "/image.gif" => ("image/gif", b"GIF89a\x01\0\x01\0\x80\0\0\0\0\0\xff\xff\xff!\xf9\x04\x01\0\0\0\0,\0\0\0\0\x01\0\x01\0\0\x02\x02D\x01\0;"),
                    "/sheet.css" => ("text/css", b"p { color: green }") ,
                    "/script.js" => ("text/javascript", b"globalThis.__domScriptLoaded = true;"),
                    "/frame.html" => ("text/html", b"<!doctype html><title>resource initiator</title>"),
                    _ => panic!("unexpected native fixture request: {path}"),
                };
                let head = format!("HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nTiming-Allow-Origin: *\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n", body.len());
                let _ = socket.write_all(head.as_bytes()).await;
                let _ = socket.write_all(body).await;
            });
            while let Some(result) = connections.try_join_next() {
                result.unwrap();
            }
        }
    });
    DomInitiatorServer {
        _server: TimingServer {
            origin: origin.clone(),
            task,
        },
        origin,
        requests,
    }
}

async fn prefetch_with_policies(
    server: &DomInitiatorServer,
    policies: &[&str],
    path: &str,
    blocked: bool,
) -> serde_json::Value {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = new_storage_page_task_executor_test_vm_with_loader(
        &format!("{}/document.html", server.origin),
        &loader,
    );
    let url = serde_json::to_string(&format!("{}{path}", server.origin)).unwrap();
    let policies = serde_json::to_string(policies).unwrap();
    page.eval(&format!(
        r#"
        globalThis.prefetchEvents = [];
        globalThis.prefetchViolations = [];
        document.addEventListener('securitypolicyviolation', e =>
            prefetchViolations.push([e.effectiveDirective, e.disposition, e.blockedURI]));
        for (const policy of {policies}) {{
            const meta = document.createElement('meta');
            meta.httpEquiv = 'Content-Security-Policy'; meta.content = policy;
            document.head.appendChild(meta);
        }}
        const link = document.createElement('link');
        link.rel = 'prefetch'; link.crossOrigin = 'anonymous'; link.href = {url};
        link.onload = () => prefetchEvents.push('load');
        link.onerror = () => prefetchEvents.push('error');
        document.head.appendChild(link);
    "#
    ))
    .unwrap();
    advance_page_task_executor_until_eval_equals(
        &mut page,
        &loader,
        "prefetchEvents.join(',')",
        if blocked { "error" } else { "load" },
        "prefetch policy load result",
    )
    .await;
    advance_page_task_executor_until_eval_equals(
        &mut page,
        &loader,
        &format!("performance.getEntriesByName({url}).length"),
        "1",
        "prefetch policy timing entry",
    )
    .await;
    if blocked {
        advance_page_task_executor_until_eval_equals(
            &mut page,
            &loader,
            "prefetchViolations.length",
            "1",
            "prefetch CSP violation",
        )
        .await;
        assert_eq!(
            page.eval("prefetchViolations[0][0]+';'+prefetchViolations[0][1]")
                .unwrap(),
            "default-src;enforce"
        );
    }
    serde_json::from_str(
        &page
            .eval(&format!(
                "JSON.stringify(performance.getEntriesByName({url})[0].toJSON())"
            ))
            .unwrap(),
    )
    .unwrap()
}

#[tokio::test]
async fn connected_prefetch_csp_blocks_physical_requests_and_retains_failure_timing() {
    let server = dom_initiator_server().await;
    let scenarios: &[(&[&str], bool)] = &[
        (&[], false),
        (&["default-src 'none'; script-src 'unsafe-inline'"], true),
        (&["default-src 'self'; script-src 'unsafe-inline'"], false),
        (
            &["default-src 'none'; img-src 'self'; script-src 'unsafe-inline'"],
            false,
        ),
        (&["img-src 'none'; script-src 'unsafe-inline'"], false),
        (
            &["default-src 'none'; base-uri 'self'; script-src 'unsafe-inline'"],
            true,
        ),
        (
            &[
                "default-src 'none'; img-src 'self'",
                "default-src 'none'; script-src 'unsafe-inline'",
            ],
            true,
        ),
    ];
    for (policies, blocked) in scenarios {
        let before = server.requests.load(Ordering::SeqCst);
        let entry = prefetch_with_policies(&server, policies, "/image.gif", *blocked).await;
        assert_eq!(
            server.requests.load(Ordering::SeqCst) - before,
            usize::from(!*blocked),
            "{policies:?}"
        );
        assert_eq!(entry["initiatorType"], "link");
        if *blocked {
            for name in [
                "requestStart",
                "responseStatus",
                "transferSize",
                "encodedBodySize",
                "decodedBodySize",
            ] {
                assert_eq!(entry[name], 0, "{name}: {entry}");
            }
        } else {
            assert_eq!(entry["responseStatus"], 200);
            assert!(entry["requestStart"].as_f64().unwrap() > 0.0, "{entry}");
            assert!(entry["decodedBodySize"].as_u64().unwrap() > 0, "{entry}");
        }
    }
}

#[tokio::test]
async fn connected_prefetch_csp_checks_redirects_before_contacting_the_next_origin() {
    let server = dom_initiator_server().await;
    prefetch_with_policies(
        &server,
        &["default-src 'self'; script-src 'unsafe-inline'"],
        "/redirect-image",
        true,
    )
    .await;
    assert_eq!(server.requests.load(Ordering::SeqCst), 1);
    let entry = prefetch_with_policies(
        &server,
        &["default-src 'self'; img-src http://localhost:*; script-src 'unsafe-inline'"],
        "/redirect-image",
        false,
    )
    .await;
    assert_eq!(server.requests.load(Ordering::SeqCst), 3);
    assert_eq!(entry["responseStatus"], 200);
}

async fn assert_dom_resource_initiator(path: &str, source: &str) {
    let server = dom_initiator_server().await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    loader.set_image_fetch_enabled(true);
    let mut page = new_storage_page_task_executor_test_vm_with_loader(
        &format!("{}/document.html", server.origin),
        &loader,
    );
    exec_resource_initiator_script(
        &mut page,
        &server.origin,
        "library",
        &format!("function issueResource() {{ {source} }}"),
    );
    exec_resource_initiator_script(&mut page, &server.origin, "caller", "issueResource()");
    exec_resource_initiator_script(
        &mut page,
        &server.origin,
        "later",
        "globalThis.laterScriptExecuted = true",
    );
    let url = format!("{}{path}", server.origin);
    let expression = format!(
        "performance.getEntriesByName({}).length",
        serde_json::to_string(&url).unwrap()
    );
    advance_page_task_executor_until_eval_equals(
        &mut page,
        &loader,
        &expression,
        "1",
        "DOM resource timing entry",
    )
    .await;
    let expression = format!(
        "performance.getEntriesByName({})[0].initiatorUrl",
        serde_json::to_string(&url).unwrap()
    );
    assert_eq!(
        page.eval(&expression).unwrap(),
        format!("{}/caller.js", server.origin),
        "{path}: capture the triggering caller before deferred resource loading, not the library or later script"
    );
}

#[tokio::test]
async fn image_timing_retains_the_triggering_script_initiator() {
    assert_dom_resource_initiator(
        "/image.gif",
        "const image = new Image(); image.src = '/image.gif'; document.body.appendChild(image);",
    )
    .await;
}

#[tokio::test]
async fn stylesheet_timing_retains_the_triggering_script_initiator() {
    assert_dom_resource_initiator("/sheet.css", "const link = document.createElement('link'); link.rel = 'stylesheet'; link.href = '/sheet.css'; document.head.appendChild(link);").await;
}

#[tokio::test]
async fn classic_script_timing_retains_the_triggering_script_initiator() {
    assert_dom_resource_initiator("/script.js", "const script = document.createElement('script'); script.src = '/script.js'; document.head.appendChild(script);").await;
}

#[tokio::test]
async fn frame_timing_retains_the_triggering_script_initiator() {
    assert_dom_resource_initiator("/frame.html", "const frame = document.createElement('iframe'); frame.src = '/frame.html'; document.body.appendChild(frame);").await;
}

fn dom_request_source(tag: &str, url: &str, cors: bool) -> String {
    let attribute = if tag == "link" { "href" } else { "src" };
    format!(
        "const element = document.createElement({tag:?}); globalThis.resource = element;\n\
        if ({tag:?} === 'link') element.rel = 'stylesheet';\n\
        if ({cors}) element.crossOrigin = 'anonymous';\n\
        element.{attribute} = {}; document.body.appendChild(element);",
        serde_json::to_string(url).unwrap()
    )
}

async fn wait_for_dom_entry(
    page: &mut crate::runtime::PageVmTaskExecutorTestHarness,
    loader: &ResourceRequestClient,
    url: &str,
) -> String {
    let url = serde_json::to_string(url).unwrap();
    advance_page_task_executor_until_eval_equals(
        page,
        loader,
        &format!("performance.getEntriesByName({url}).length"),
        "1",
        "DOM request attribution",
    )
    .await;
    page.eval(&format!(
        "performance.getEntriesByName({url})[0].initiatorUrl"
    ))
    .unwrap()
}

#[tokio::test]
async fn dom_resource_initiators_are_hidden_for_opaque_tao_responses() {
    for (tag, path) in [
        ("img", "/image.gif"),
        ("link", "/sheet.css"),
        ("script", "/script.js"),
        ("iframe", "/frame.html"),
    ] {
        let server = dom_initiator_server().await;
        let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
        loader.set_image_fetch_enabled(true);
        let mut page = new_storage_page_task_executor_test_vm_with_loader(
            "http://127.0.0.1:9/document.html",
            &loader,
        );
        let url = format!("{}{path}", server.origin);
        exec_resource_initiator_script(
            &mut page,
            "http://127.0.0.1:9",
            "caller",
            &dom_request_source(tag, &url, false),
        );
        assert_eq!(
            wait_for_dom_entry(&mut page, &loader, &url).await,
            "",
            "{tag}: TAO and ACAO headers do not make a no-cors response readable"
        );
        assert_eq!(
            page.eval(&format!(
                "performance.getEntriesByName({})[0].encodedBodySize > 0",
                serde_json::to_string(&url).unwrap()
            ))
            .unwrap(),
            "true",
            "{tag}: the response must complete with TAO-visible bytes"
        );
    }
}

#[tokio::test]
async fn dom_resource_initiators_are_exposed_for_cors_approved_responses() {
    for (tag, path) in [
        ("img", "/image.gif"),
        ("link", "/sheet.css"),
        ("script", "/script.js"),
    ] {
        let server = dom_initiator_server().await;
        let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
        loader.set_image_fetch_enabled(true);
        let mut page = new_storage_page_task_executor_test_vm_with_loader(
            "http://127.0.0.1:9/document.html",
            &loader,
        );
        let url = format!("{}{path}", server.origin);
        exec_resource_initiator_script(
            &mut page,
            "http://127.0.0.1:9",
            "caller",
            &dom_request_source(tag, &url, true),
        );
        assert_eq!(
            wait_for_dom_entry(&mut page, &loader, &url).await,
            "http://127.0.0.1:9/caller.js",
            "{tag}: successful anonymous CORS exposes the initiating script"
        );
    }
}

#[tokio::test]
async fn dom_element_reloads_capture_each_initiating_script() {
    for (tag, path, attribute) in [
        ("img", "/image.gif", "src"),
        ("link", "/sheet.css", "href"),
        ("iframe", "/frame.html", "src"),
    ] {
        let server = dom_initiator_server().await;
        let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
        loader.set_image_fetch_enabled(true);
        let mut page = new_storage_page_task_executor_test_vm_with_loader(
            &format!("{}/document.html", server.origin),
            &loader,
        );
        let first = format!("{}{path}", server.origin);
        exec_resource_initiator_script(
            &mut page,
            &server.origin,
            "first",
            &dom_request_source(tag, &first, false),
        );
        assert_eq!(
            wait_for_dom_entry(&mut page, &loader, &first).await,
            format!("{}/first.js", server.origin)
        );
        page.drain_ready_page_task_executor_turns_for_setup(&loader, 128)
            .await
            .unwrap();
        let second = format!("{first}?second");
        exec_resource_initiator_script(
            &mut page,
            &server.origin,
            "second",
            &format!(
                "resource.{attribute} = {}",
                serde_json::to_string(&second).unwrap()
            ),
        );
        assert_eq!(
            wait_for_dom_entry(&mut page, &loader, &second).await,
            format!("{}/second.js", server.origin),
            "{tag}: replacing the request must replace its attribution"
        );
        if tag == "iframe" {
            page.drain_ready_page_task_executor_turns_for_setup(&loader, 128)
                .await
                .unwrap();
            exec_resource_initiator_script(
                &mut page,
                &server.origin,
                "third",
                &format!("resource.src = {}", serde_json::to_string(&second).unwrap()),
            );
            let entries = format!(
                "performance.getEntriesByName({})",
                serde_json::to_string(&second).unwrap()
            );
            advance_page_task_executor_until_eval_equals(
                &mut page,
                &loader,
                &format!("{entries}.length"),
                "2",
                "same-value frame navigation",
            )
            .await;
            assert_eq!(
                page.eval(&format!("{entries}[1].initiatorUrl")).unwrap(),
                format!("{}/third.js", server.origin)
            );
        }
    }
}

#[tokio::test]
async fn reused_stylesheet_fetch_keeps_its_original_initiator() {
    let server = dom_initiator_server().await;
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let mut page = new_storage_page_task_executor_test_vm_with_loader(
        &format!("{}/document.html", server.origin),
        &loader,
    );
    let url = format!("{}/sheet.css", server.origin);
    exec_resource_initiator_script(
        &mut page,
        &server.origin,
        "first",
        &dom_request_source("link", &url, false),
    );
    assert_eq!(
        wait_for_dom_entry(&mut page, &loader, &url).await,
        format!("{}/first.js", server.origin)
    );
    page.drain_ready_page_task_executor_turns_for_setup(&loader, 128)
        .await
        .unwrap();
    exec_resource_initiator_script(
        &mut page,
        &server.origin,
        "second",
        "globalThis.secondLoaded = false; const link = document.createElement('link'); link.rel = 'stylesheet'; link.href = '/sheet.css'; link.onload = () => secondLoaded = true; document.head.appendChild(link);",
    );
    advance_page_task_executor_until_eval_equals(
        &mut page,
        &loader,
        "secondLoaded",
        "true",
        "reused stylesheet link event",
    )
    .await;
    assert_eq!(
        wait_for_dom_entry(&mut page, &loader, &url).await,
        format!("{}/first.js", server.origin)
    );
    assert_eq!(
        server.requests.load(Ordering::SeqCst),
        1,
        "attribution must not split the fetch's reuse identity"
    );
}
