use super::*;
use crate::browser::{NetworkOwner, NetworkRequestState};
use crate::page::{
    RendererNetworkOutputItem, ScriptNetworkOutputItem, SubresourceBodyFinishedResult,
    SubresourceResourceType,
};

#[derive(Clone, Copy, Debug)]
enum Finish {
    Complete,
    PartialFailure,
    CancelledAfterChunk,
    PageClosed,
    ChildRemoved,
    DocumentOpened,
}

macro_rules! csp_stage_tests {
    ($($name:ident: $child:literal, $finish:ident, $controlled:literal;)*) => {
        $(#[tokio::test]
        async fn $name() {
            document_resource_stages($child, Finish::$finish, RequestKind::CspReport, $controlled).await;
        })*
    };
}

csp_stage_tests! {
    native_document_csp_stages_top: false, Complete, false;
    native_document_csp_stages_top_partial: false, PartialFailure, false;
    native_document_csp_stages_top_closed: false, PageClosed, false;
    native_document_csp_stages_child: true, Complete, false;
    native_document_csp_stages_child_partial: true, PartialFailure, false;
    native_document_csp_stages_child_page_closed: true, PageClosed, false;
    native_document_csp_stages_child_removed: true, ChildRemoved, false;
    native_document_csp_stages_top_opened: false, DocumentOpened, false;
    native_document_csp_stages_child_opened: true, DocumentOpened, false;
    native_document_csp_stages_service_response: false, Complete, true;
    native_document_csp_stages_service_partial: false, PartialFailure, true;
    native_document_csp_stages_service_page_closed: false, PageClosed, true;
    native_document_csp_stages_service_document_opened: false, DocumentOpened, true;
    native_document_csp_stages_service_child_removed: true, ChildRemoved, true;
    native_document_csp_stages_service_child_page_closed: true, PageClosed, true;
}

#[derive(Clone, Copy, Debug)]
enum RequestKind {
    CspReport,
    Beacon,
    Ping,
    Fetch,
    NoCorsFetch,
}

macro_rules! keepalive_stage_tests {
    ($($name:ident: $child:literal, $finish:ident, $kind:ident;)*) => {
        keepalive_stage_tests! { $($name: $child, $finish, $kind, false;)* }
    };
    ($($name:ident: $child:literal, $finish:ident, $kind:ident, $controlled:literal;)*) => {
        $(#[tokio::test]
        async fn $name() {
            document_resource_stages($child, Finish::$finish, RequestKind::$kind, $controlled).await;
        })*
    };
}

keepalive_stage_tests! {
    native_document_ping_stages_beacon: false, Complete, Beacon;
    native_document_ping_stages_beacon_partial: false, PartialFailure, Beacon;
    native_document_ping_stages_beacon_closed: false, PageClosed, Beacon;
    native_document_ping_stages_beacon_child_removed: true, ChildRemoved, Beacon;
    native_document_ping_stages_beacon_child_opened: true, DocumentOpened, Beacon;
    native_document_ping_stages_link: false, Complete, Ping;
    native_document_ping_stages_link_partial: false, PartialFailure, Ping;
    native_document_ping_stages_link_closed: false, PageClosed, Ping;
}

keepalive_stage_tests! {
    native_document_fetch_stages_live: false, Complete, Fetch;
    native_document_fetch_stages_partial: false, PartialFailure, Fetch;
    native_document_fetch_stages_cancelled_after_chunk: false, CancelledAfterChunk, Fetch;
    native_document_fetch_stages_closed: false, PageClosed, Fetch;
    native_document_fetch_stages_child_removed: true, ChildRemoved, Fetch;
    native_document_fetch_stages_no_cors: false, Complete, NoCorsFetch;
    native_document_fetch_stages_no_cors_cancelled_after_chunk: false, CancelledAfterChunk, NoCorsFetch;
    native_document_fetch_stages_no_cors_closed: false, PageClosed, NoCorsFetch;
}

keepalive_stage_tests! {
    native_document_fetch_stages_service: false, Complete, Fetch, true;
    native_document_fetch_stages_service_partial: false, PartialFailure, Fetch, true;
    native_document_fetch_stages_service_closed: false, PageClosed, Fetch, true;
    native_document_fetch_stages_service_no_cors: false, Complete, NoCorsFetch, true;
    native_document_fetch_stages_service_no_cors_partial: false, PartialFailure, NoCorsFetch, true;
    native_document_fetch_stages_service_no_cors_closed: false, PageClosed, NoCorsFetch, true;
}

async fn document_resource_stages(
    child: bool,
    finish: Finish,
    kind: RequestKind,
    controlled: bool,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let report_url = format!("{origin}/report");
    let (requested, request_arrived) = tokio::sync::oneshot::channel();
    let (headers, release_headers) = tokio::sync::oneshot::channel();
    let (chunk, release_chunk) = tokio::sync::oneshot::channel();
    let (tail, release_tail) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let mut requested = Some(requested);
        let mut release_headers = Some(release_headers);
        let mut release_chunk = Some(release_chunk);
        let mut release_tail = Some(release_tail);
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut byte = [0];
            while !request.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).await.unwrap();
                request.push(byte[0]);
            }
            let request = String::from_utf8(request).unwrap();
            let path = request.split_whitespace().nth(1).unwrap();
            if controlled && path == "/sw.js" {
                let validate_body = match kind {
                    RequestKind::CspReport => {
                        "const report = await event.request.json(); if (report['csp-report']['effective-directive'] !== 'connect-src') throw new Error('invalid report body');"
                    }
                    RequestKind::Fetch | RequestKind::NoCorsFetch => {
                        "const bytes = Array.from(new Uint8Array(await event.request.arrayBuffer())); if (JSON.stringify(bytes) !== '[0,128,255,65]') throw new Error('request bytes changed');"
                    }
                    _ => unreachable!("controlled fixture supports CSP reports and Fetch"),
                };
                let finish_stream = if matches!(finish, Finish::PartialFailure) {
                    "controller.error(new Error('truncated'))"
                } else {
                    "controller.enqueue(new Uint8Array([100,121]));controller.close()"
                };
                let source = format!(
                    r#"
                    self.addEventListener('install', event => event.waitUntil(self.skipWaiting()));
                    self.addEventListener('activate', event => event.waitUntil(self.clients.claim()));
                    self.addEventListener('fetch', event => {{
                        if (new URL(event.request.url).pathname !== '/report') return;
                        event.respondWith((async () => {{
                            if (!event.request.keepalive) throw new Error('report must be keepalive');
                            {validate_body}
                            await fetch('/report-headers');
                            return new Response(new ReadableStream({{ start(controller) {{
                                (async () => {{
                                    await fetch('/report-chunk');
                                    controller.enqueue(new Uint8Array([98,111]));
                                    await fetch('/report-tail');
                                    {finish_stream};
                                }})().catch(error => controller.error(error));
                            }} }}), {{statusText:'Controlled response',headers: {{'Content-Type':'text/plain'}}}});
                        }})());
                    }});
                "#
                );
                stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{source}", source.len()).as_bytes()).await.unwrap();
                continue;
            }
            if controlled && path.starts_with("/report-") {
                let release = match path {
                    "/report-headers" => {
                        requested.take().unwrap().send(()).unwrap();
                        release_headers.take().unwrap()
                    }
                    "/report-chunk" => release_chunk.take().unwrap(),
                    "/report-tail" => release_tail.take().unwrap(),
                    _ => panic!("unexpected ServiceWorker gate: {path}"),
                };
                if release.await.is_err() {
                    return;
                }
                stream
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    .await
                    .unwrap();
                if path == "/report-tail" {
                    break;
                }
                continue;
            }
            if path == "/report" {
                assert!(!controlled, "the ServiceWorker must handle the report");
                assert!(request.starts_with("POST /report HTTP/1.1"));
                let length = request
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().unwrap())
                    })
                    .expect("CSP report body length");
                let mut body = vec![0; length];
                stream.read_exact(&mut body).await.unwrap();
                match kind {
                    RequestKind::CspReport => {
                        let report: serde_json::Value = serde_json::from_slice(&body).unwrap();
                        assert_eq!(report["csp-report"]["effective-directive"], "connect-src");
                    }
                    RequestKind::Beacon | RequestKind::Fetch | RequestKind::NoCorsFetch => {
                        assert_eq!(body, [0, 128, 255, 65])
                    }
                    RequestKind::Ping => assert_eq!(body, b"PING"),
                }
                requested.take().unwrap().send(()).unwrap();
                if release_headers.take().unwrap().await.is_err() {
                    return;
                }
                stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 4\r\nConnection: close\r\n\r\n").await.unwrap();
                if release_chunk.take().unwrap().await.is_err() {
                    return;
                }
                stream.write_all(b"bo").await.unwrap();
                if release_tail.take().unwrap().await.is_err() {
                    return;
                }
                if matches!(finish, Finish::CancelledAfterChunk) {
                    assert_eq!(
                        tokio::time::timeout(
                            std::time::Duration::from_secs(5),
                            stream.read(&mut byte)
                        )
                        .await
                        .expect("retirement must cancel the physical response")
                        .unwrap(),
                        0,
                        "ordinary Fetch cancellation must close its transport",
                    );
                } else if !matches!(finish, Finish::PartialFailure) {
                    stream.write_all(b"dy").await.unwrap();
                }
                break;
            }
            let report_document = path == "/child" || (path == "/page" && !child);
            assert!(
                path == "/page" || (path == "/child" && child),
                "unexpected request: {path}"
            );
            let html = if report_document {
                let (body, script) = match kind {
                    RequestKind::CspReport => ("", "fetch('/blocked').catch(()=>{})"),
                    RequestKind::Beacon => (
                        "",
                        "navigator.sendBeacon('/report',new Uint8Array([0,128,255,65]))",
                    ),
                    RequestKind::Fetch => (
                        "",
                        "fetch('/report',{method:'POST',body:new Uint8Array([0,128,255,65]),keepalive}).then(r=>r.text()).catch(()=>{})",
                    ),
                    RequestKind::NoCorsFetch => (
                        "",
                        "fetch('/report',{method:'POST',mode:'no-cors',body:new Uint8Array([0,128,255,65]),keepalive}).then(r=>r.text()).catch(()=>{})",
                    ),
                    RequestKind::Ping => (
                        "<a href='#pinged' ping='/report'>ping</a>",
                        "document.querySelector('a').click()",
                    ),
                };
                let setup = if controlled {
                    "await navigator.serviceWorker.register('/sw.js');await navigator.serviceWorker.ready;if(!navigator.serviceWorker.controller)await new Promise(resolve=>navigator.serviceWorker.addEventListener('controllerchange',resolve,{once:true}));"
                } else {
                    ""
                };
                let keepalive = !matches!(finish, Finish::CancelledAfterChunk);
                format!(
                    "<!doctype html>{body}<script>(async()=>{{const keepalive={keepalive};{setup}{script}}})()</script>"
                )
            } else {
                "<!doctype html><iframe src='/child'></iframe>".to_owned()
            };
            let csp = if report_document && matches!(kind, RequestKind::CspReport) {
                "Content-Security-Policy: connect-src 'none'; report-uri /report\r\n"
            } else {
                ""
            };
            stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n{csp}Content-Length: {}\r\nConnection: close\r\n\r\n{html}", html.len()).as_bytes()).await.unwrap();
        }
    });
    let service = BrowserService::start().unwrap();
    let browser = service.handle();
    let (context, contents) = context_with_contents(&service);
    let (_, mut events) = browser.subscribe().unwrap();
    let document = navigate(&context, contents, &format!("{origin}/page")).await;
    tokio::time::timeout(std::time::Duration::from_secs(5), request_arrived)
        .await
        .expect("the real CSP report must reach HTTP with headers held")
        .unwrap();
    let (source, handle, mut sequence) =
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let event = events.recv().await.unwrap();
                if let BrowserEvent::NetworkRequestStarted(occurrence) = event.event
                    && occurrence.owner == NetworkOwner::Document(document)
                    && let RendererNetworkOutputItem::Resource(item) = &occurrence.renderer.item
                    && let ScriptNetworkOutputItem::SubresourceRequestStarted(request) =
                        item.as_ref()
                    && request.url().as_str() == report_url
                {
                    assert_eq!(request.method(), "POST");
                    assert_eq!(
                        request.resource_type(),
                        match kind {
                            RequestKind::CspReport => SubresourceResourceType::CspReport,
                            RequestKind::Fetch | RequestKind::NoCorsFetch =>
                                SubresourceResourceType::Fetch,
                            RequestKind::Beacon | RequestKind::Ping =>
                                SubresourceResourceType::Ping,
                        }
                    );
                    assert_eq!(
                        request.keepalive(),
                        !matches!(finish, Finish::CancelledAfterChunk)
                    );
                    break (
                        occurrence.renderer.source.clone(),
                        request.handle(),
                        event.sequence,
                    );
                }
            }
        })
        .await
        .expect("CSP native admission must precede held response headers without DevTools");
    if matches!(finish, Finish::PageClosed) {
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            context.close_web_contents(contents).unwrap().close_async(),
        )
        .await
        .expect("Page closes while its CSP report is pending");
        assert!(
            !browser
                .subscribe()
                .unwrap()
                .0
                .web_contents
                .contains(&contents)
        );
    } else if matches!(finish, Finish::DocumentOpened) {
        assert_eq!(context.evaluate_document_expression_for_test(
            document, "document.open();document.write('<!doctype html><title>replacement</title>');document.close();document.title", false,
        ).await.unwrap()["value"], "replacement");
    } else if matches!(finish, Finish::ChildRemoved) {
        let before = context
            .start_child_frame_tree_snapshot(document)
            .unwrap()
            .wait()
            .await;
        assert_eq!(
            context
                .finish_child_frame_tree_snapshot(before)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(context.evaluate_document_expression_for_test(
            document, "document.querySelector('iframe').remove();document.querySelector('iframe')===null", false,
        ).await.unwrap()["value"], true);
        let after = context
            .start_child_frame_tree_snapshot(document)
            .unwrap()
            .wait()
            .await;
        assert!(
            context
                .finish_child_frame_tree_snapshot(after)
                .unwrap()
                .is_empty(),
            "the original child must actually leave the frame tree"
        );
    }
    headers.send(()).unwrap();
    for (stage, release) in [(0, Some(chunk)), (1, Some(tail)), (2, None)] {
        let event = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let event = events.recv().await.unwrap();
                if let BrowserEvent::NetworkSourceClosed { source: closed, .. } = &event.event {
                    assert_ne!(*closed, source.identity(), "source closure cannot overtake an admitted CSP report");
                }
                let occurrence = match &event.event {
                    BrowserEvent::NetworkActivity(occurrence)
                    | BrowserEvent::NetworkRequestCompleted(occurrence)
                        if occurrence.owner == NetworkOwner::Document(document) => occurrence,
                    _ => continue,
                };
                if occurrence.renderer.source != source { continue; }
                let RendererNetworkOutputItem::Resource(item) = &occurrence.renderer.item else { continue; };
                if let ScriptNetworkOutputItem::SubresourceBodyFinished(body) = item.as_ref()
                    && body.handle() == handle && stage < 2 {
                    panic!("CSP terminal overtook held stage {stage}: {:?}", body.result());
                }
                let matched = match (stage, item.as_ref()) {
                    (0, ScriptNetworkOutputItem::SubresourceResponseStarted(head)) => head.handle() == handle,
                    (1, ScriptNetworkOutputItem::SubresourceDataReceived(data)) => data.handle() == handle,
                    (2, ScriptNetworkOutputItem::SubresourceBodyFinished(body)) => body.handle() == handle,
                    _ => false,
                };
                if matched { break event; }
            }
        }).await.unwrap_or_else(|_| panic!("{kind:?} child={child} {finish:?}: native stage {stage} must precede the next transport gate"));
        assert!(event.sequence > sequence);
        sequence = event.sequence;
        let occurrence = match event.event {
            BrowserEvent::NetworkActivity(occurrence) if stage < 2 => occurrence,
            BrowserEvent::NetworkRequestCompleted(occurrence) if stage == 2 => occurrence,
            other => panic!("wrong native phase event: {other:?}"),
        };
        let RendererNetworkOutputItem::Resource(item) = &occurrence.renderer.item else {
            unreachable!()
        };
        match item.as_ref() {
            ScriptNetworkOutputItem::SubresourceResponseStarted(head) => {
                assert_eq!(head.status(), 200);
                assert_eq!(
                    head.status_text(),
                    controlled.then_some("Controlled response")
                );
                assert!(browser.subscribe().unwrap().0.network_requests.iter().any(|request|
                    request.owner == NetworkOwner::Document(document) && request.renderer_source == source
                    && matches!(&request.state, NetworkRequestState::Responding { response, .. } if response.handle() == handle)));
            }
            ScriptNetworkOutputItem::SubresourceDataReceived(data) => {
                assert_eq!(data.data_length(), 2)
            }
            ScriptNetworkOutputItem::SubresourceBodyFinished(body) => match (finish, body.result())
            {
                (
                    Finish::PartialFailure | Finish::CancelledAfterChunk,
                    SubresourceBodyFinishedResult::FailedWithPartialBody {
                        error_text,
                        partial_body,
                    },
                ) => {
                    assert!(!error_text.is_empty());
                    assert_eq!(partial_body.clone_body_bytes(), b"bo");
                }
                (
                    Finish::Complete
                    | Finish::PageClosed
                    | Finish::ChildRemoved
                    | Finish::DocumentOpened,
                    SubresourceBodyFinishedResult::Ready(body),
                ) => assert_eq!(body.clone_body_bytes(), b"body"),
                other => panic!("native terminal must retain the physical CSP result: {other:?}"),
            },
            _ => unreachable!(),
        }
        if stage == 1 && matches!(finish, Finish::CancelledAfterChunk) {
            tokio::time::timeout(
                std::time::Duration::from_secs(5),
                context.close_web_contents(contents).unwrap().close_async(),
            )
            .await
            .expect("ordinary Fetch must retire after its first physical chunk");
        }
        if let Some(release) = release {
            release.send(()).unwrap();
        }
    }
    server.await.unwrap();
    if !matches!(finish, Finish::PageClosed | Finish::CancelledAfterChunk) {
        context
            .close_web_contents(contents)
            .unwrap()
            .close_async()
            .await;
    }
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if browser
                .subscribe()
                .unwrap()
                .0
                .network_requests
                .iter()
                .all(|request| request.renderer_source != source)
            {
                break;
            }
            events.recv().await.unwrap();
        }
    })
    .await
    .expect("retired Page source releases the completed CSP tail");
    service.shutdown();
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum PreflightFinish {
    Complete,
    CloseBeforeHead,
    CloseAfterPrefix,
    CloseBeforeOptions,
}

#[tokio::test]
async fn native_document_preflight_emits_native_stages() {
    document_preflight_stages(PreflightFinish::Complete).await;
}

#[tokio::test]
async fn native_document_preflight_close_before_head_cancels() {
    document_preflight_stages(PreflightFinish::CloseBeforeHead).await;
}

#[tokio::test]
async fn native_document_preflight_close_retains_exact_partial_body() {
    document_preflight_stages(PreflightFinish::CloseAfterPrefix).await;
}

#[tokio::test]
async fn native_document_preflight_keepalive_admitted_after_page_close() {
    document_preflight_stages(PreflightFinish::CloseBeforeOptions).await;
}

async fn document_preflight_stages(finish: PreflightFinish) {
    let page_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let target_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let page_origin = format!("http://{}", page_listener.local_addr().unwrap());
    let target_origin = format!("http://{}", target_listener.local_addr().unwrap());
    let page_origin_for_server = page_origin.clone();
    let (redirect_arrived_tx, redirect_arrived_rx) = tokio::sync::oneshot::channel();
    let (redirect_release_tx, redirect_release_rx) = tokio::sync::oneshot::channel();
    let (options_arrived_tx, options_arrived_rx) = tokio::sync::oneshot::channel();
    let (head_tx, head_rx) = tokio::sync::oneshot::channel();
    let (chunk_tx, chunk_rx) = tokio::sync::oneshot::channel();
    let (tail_tx, tail_rx) = tokio::sync::oneshot::channel();
    let keepalive = finish == PreflightFinish::CloseBeforeOptions;
    let server = tokio::spawn(async move {
        let (mut page, _) = page_listener.accept().await.unwrap();
        read_preflight_request(&mut page).await;
        let request_url = if keepalive {
            "/redirect".into()
        } else {
            format!("{target_origin}/preflight")
        };
        let html = format!(
            "<!doctype html><script>fetch('{request_url}',{{method:'PUT',headers:{{'X-Test':'1'}},body:'payload',keepalive:{keepalive}}}).catch(()=>{{}})</script>"
        );
        page.write_all(format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{html}", html.len()
        ).as_bytes()).await.unwrap();
        if keepalive {
            let (mut redirect, _) = page_listener.accept().await.unwrap();
            assert!(
                read_preflight_request(&mut redirect)
                    .await
                    .starts_with("PUT /redirect HTTP/1.1")
            );
            redirect_arrived_tx.send(()).unwrap();
            redirect_release_rx.await.unwrap();
            redirect.write_all(format!(
                "HTTP/1.1 307 Temporary Redirect\r\nLocation: {target_origin}/preflight\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            ).as_bytes()).await.unwrap();
        }
        let (mut options, _) = target_listener.accept().await.unwrap();
        let request = read_preflight_request(&mut options).await;
        assert!(request.starts_with("OPTIONS /preflight HTTP/1.1"));
        assert!(
            request
                .lines()
                .any(|line| line.eq_ignore_ascii_case("Access-Control-Request-Method: PUT"))
        );
        options_arrived_tx.send(()).unwrap();
        if head_rx.await.is_err() {
            return;
        }
        let (status, length) = if finish == PreflightFinish::Complete {
            ("204 No Content", 0)
        } else {
            ("200 OK", 4)
        };
        options.write_all(format!(
            "HTTP/1.1 {status}\r\nAccess-Control-Allow-Origin: {page_origin_for_server}\r\nAccess-Control-Allow-Methods: PUT\r\nAccess-Control-Allow-Headers: X-Test\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n"
        ).as_bytes()).await.unwrap();
        if length > 0 {
            chunk_rx.await.unwrap();
            options.write_all(b"bo").await.unwrap();
            if tail_rx.await.is_err() {
                return;
            }
            options.write_all(b"dy").await.unwrap();
        }
        let (mut put, _) = target_listener.accept().await.unwrap();
        assert!(
            read_preflight_request(&mut put)
                .await
                .starts_with("PUT /preflight HTTP/1.1")
        );
        let mut body = [0; 7];
        put.read_exact(&mut body).await.unwrap();
        assert_eq!(&body, b"payload");
        put.write_all(format!(
            "HTTP/1.1 200 OK\r\nAccess-Control-Allow-Origin: {page_origin_for_server}\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok"
        ).as_bytes()).await.unwrap();
    });

    let service = BrowserService::start().unwrap();
    let browser = service.handle();
    let (context, contents) = context_with_contents(&service);
    let (_, mut events) = browser.subscribe().unwrap();
    let document = navigate(&context, contents, &format!("{page_origin}/page")).await;
    if keepalive {
        tokio::time::timeout(std::time::Duration::from_secs(5), redirect_arrived_rx)
            .await
            .expect("original PUT must precede Page retirement")
            .unwrap();
        context
            .close_web_contents(contents)
            .unwrap()
            .close_async()
            .await;
        redirect_release_tx.send(()).unwrap();
    }
    tokio::time::timeout(std::time::Duration::from_secs(5), options_arrived_rx)
        .await
        .expect("the physical CORS preflight must reach the server")
        .unwrap();
    let (source, handle) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let event = events.recv().await.unwrap();
            if let BrowserEvent::NetworkRequestStarted(occurrence) = event.event
                && occurrence.owner == NetworkOwner::Document(document)
                && let RendererNetworkOutputItem::Resource(item) = &occurrence.renderer.item
                && let ScriptNetworkOutputItem::SubresourceRequestStarted(request) = item.as_ref()
                && request.method() == "OPTIONS"
            {
                assert_eq!(request.keepalive(), keepalive);
                return (occurrence.renderer.source.clone(), request.handle());
            }
        }
    })
    .await
    .expect("native preflight admission must precede the held response");

    if finish == PreflightFinish::CloseBeforeHead {
        context
            .close_web_contents(contents)
            .unwrap()
            .close_async()
            .await;
        drop(head_tx);
    } else {
        head_tx.send(()).unwrap();
        let (completed, head) = next_preflight_stage(&mut events, document, &source, handle).await;
        assert!(!completed);
        let ScriptNetworkOutputItem::SubresourceResponseStarted(head) = head.as_ref() else {
            panic!("head before body: {head:?}")
        };
        assert_eq!(
            head.status(),
            if finish == PreflightFinish::Complete {
                204
            } else {
                200
            }
        );
        if finish != PreflightFinish::Complete {
            chunk_tx.send(()).unwrap();
            let (completed, chunk) =
                next_preflight_stage(&mut events, document, &source, handle).await;
            assert!(!completed);
            let ScriptNetworkOutputItem::SubresourceDataReceived(chunk) = chunk.as_ref() else {
                panic!("body prefix before terminal: {chunk:?}")
            };
            assert_eq!(chunk.data_length(), 2);
            if finish == PreflightFinish::CloseAfterPrefix {
                context
                    .close_web_contents(contents)
                    .unwrap()
                    .close_async()
                    .await;
                drop(tail_tx);
            } else {
                tail_tx.send(()).unwrap();
                let (completed, tail) =
                    next_preflight_stage(&mut events, document, &source, handle).await;
                assert!(!completed);
                let ScriptNetworkOutputItem::SubresourceDataReceived(tail) = tail.as_ref() else {
                    panic!("tail before terminal: {tail:?}")
                };
                assert_eq!(tail.data_length(), 2);
            }
        }
    }
    let (completed, terminal) = next_preflight_stage(&mut events, document, &source, handle).await;
    assert!(completed, "native terminal must be committed as completed");
    let ScriptNetworkOutputItem::SubresourceBodyFinished(terminal) = terminal.as_ref() else {
        panic!("terminal after transport: {terminal:?}")
    };
    match (finish, terminal.result()) {
        (PreflightFinish::Complete, SubresourceBodyFinishedResult::Ready(body)) => {
            assert!(body.clone_body_bytes().is_empty())
        }
        (PreflightFinish::CloseBeforeOptions, SubresourceBodyFinishedResult::Ready(body)) => {
            assert_eq!(body.clone_body_bytes(), b"body")
        }
        (PreflightFinish::CloseBeforeHead, SubresourceBodyFinishedResult::Failed(error)) => {
            assert!(!error.is_empty())
        }
        (
            PreflightFinish::CloseAfterPrefix,
            SubresourceBodyFinishedResult::FailedWithPartialBody {
                error_text,
                partial_body,
            },
        ) => {
            assert!(!error_text.is_empty());
            assert_eq!(partial_body.clone_body_bytes(), b"bo");
        }
        other => panic!("native preflight must retain its actual outcome: {other:?}"),
    }
    server.await.unwrap();
    if finish == PreflightFinish::Complete {
        context
            .close_web_contents(contents)
            .unwrap()
            .close_async()
            .await;
    }
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let event = events.recv().await.unwrap();
            if let BrowserEvent::NetworkSourceClosed { source: closed, .. } = &event.event
                && *closed == source.identity()
            {
                break;
            }
            if let BrowserEvent::NetworkRequestStarted(ref occurrence)
            | BrowserEvent::NetworkRequestCompleted(ref occurrence) = event.event
            {
                assert_ne!(
                    crate::browser::network::request_key(&occurrence.renderer),
                    Some((
                        source.identity(),
                        crate::browser::network::NetworkRequestIdentity::Resource(handle.get())
                    )),
                    "one admission and terminal per physical preflight"
                );
            }
        }
    })
    .await
    .expect("retired source must release its admitted preflight");
    service.shutdown();
}

async fn read_preflight_request(stream: &mut tokio::net::TcpStream) -> String {
    let mut request = Vec::new();
    let mut byte = [0];
    while !request.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte).await.unwrap();
        request.push(byte[0]);
    }
    String::from_utf8(request).unwrap()
}

async fn next_preflight_stage(
    events: &mut crate::browser::BrowserEventReceiver,
    document: DocumentHandle,
    source: &crate::page::RendererNetworkSource,
    handle: crate::page::SubresourceNetworkRequestHandle,
) -> (bool, std::sync::Arc<ScriptNetworkOutputItem>) {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let event = events.recv().await.unwrap();
            if let BrowserEvent::NetworkSourceClosed { source: closed, .. } = &event.event {
                assert_ne!(
                    *closed,
                    source.identity(),
                    "source must retain its admitted preflight"
                );
            }
            let completed = matches!(event.event, BrowserEvent::NetworkRequestCompleted(_));
            let occurrence = match event.event {
                BrowserEvent::NetworkActivity(occurrence)
                | BrowserEvent::NetworkRequestCompleted(occurrence)
                    if occurrence.owner == NetworkOwner::Document(document)
                        && occurrence.renderer.source == *source =>
                {
                    occurrence
                }
                _ => continue,
            };
            if crate::browser::network::request_key(&occurrence.renderer)
                == Some((
                    source.identity(),
                    crate::browser::network::NetworkRequestIdentity::Resource(handle.get()),
                ))
                && let RendererNetworkOutputItem::Resource(item) = &occurrence.renderer.item
            {
                return (completed, item.clone());
            }
        }
    })
    .await
    .expect("native preflight stage must precede the next transport gate")
}
