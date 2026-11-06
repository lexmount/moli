use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

struct DomInitiatorServer {
    origin: String,
    requests: Arc<AtomicUsize>,
    _server: TimingServer,
}

async fn dom_initiator_server() -> DomInitiatorServer {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(AtomicUsize::new(0));
    let request_count = requests.clone();
    let task = tokio::spawn(async move {
        let mut connections = tokio::task::JoinSet::new();
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request_count = request_count.clone();
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
