use super::*;
use crate::browser::{BrowserEventReceiver, NetworkOwner};
use crate::page::{
    RendererNetworkOutputItem, RendererNetworkSource, ScriptNetworkOutputItem,
    SubresourceBodyFinishedResult, SubresourceNetworkRequestHandle, SubresourceResourceType,
};
use std::{sync::Arc, time::Duration};

#[derive(Clone, Copy, Debug)]
enum Finish {
    Complete,
    Partial,
    FailedRedirect,
    RemoveBeforeHead,
    RemoveAfterPrefix,
    CloseAfterPrefix,
    CloseBeforeHead,
}

impl Finish {
    fn cancels(self) -> bool {
        matches!(
            self,
            Self::RemoveBeforeHead
                | Self::RemoveAfterPrefix
                | Self::CloseAfterPrefix
                | Self::CloseBeforeHead
        )
    }
}

macro_rules! child_stage_tests {
    ($($name:ident: $controlled:literal, $finish:ident;)*) => {
        $(#[tokio::test]
        async fn $name() { resource_response_stages($controlled, Finish::$finish, RequestKind::ChildDocument).await; })*
    };
}

child_stage_tests! {
    native_child_document_stages_http: false, Complete;
    native_child_document_stages_http_partial: false, Partial;
    native_child_document_stages_http_remove_before_head: false, RemoveBeforeHead;
    native_child_document_stages_http_remove_after_prefix: false, RemoveAfterPrefix;
    native_child_document_stages_http_close_after_prefix: false, CloseAfterPrefix;
    native_child_document_stages_controlled: true, Complete;
    native_child_document_stages_controlled_partial: true, Partial;
    native_child_document_stages_controlled_remove_before_head: true, RemoveBeforeHead;
    native_child_document_stages_controlled_remove_after_prefix: true, RemoveAfterPrefix;
    native_child_document_stages_controlled_close_after_prefix: true, CloseAfterPrefix;
}

#[derive(Clone, Copy, Debug)]
enum RequestKind {
    ChildDocument,
    Stylesheet,
    StyleImport,
    Preload,
    RuntimeClassic,
    RuntimeModule,
    DynamicImport,
    ScriptPreload,
    ChildClassic,
    ChildModule,
    ParserClassic,
    ParserModule,
    ParserStylesheet,
    ParserPreload,
    DocumentWrite,
}

impl RequestKind {
    fn parser_html(self) -> Option<&'static str> {
        match self {
            Self::ParserClassic => Some("<!doctype html><script src='/child'></script>"),
            Self::ParserModule => {
                Some("<!doctype html><script type='module' src='/child'></script>")
            }
            Self::ParserStylesheet => Some(
                "<!doctype html><link rel='stylesheet' href='/child'><script>globalThis.afterStylesheet=true</script>",
            ),
            Self::ParserPreload => Some(
                "<!doctype html><link rel='preload' as='script' href='/child'><script src='/child'></script>",
            ),
            Self::DocumentWrite => Some(
                "<!doctype html><script>document.write('<script src=\"/child\"><'+'/script>')</script>",
            ),
            _ => None,
        }
    }

    fn response_body(self) -> (&'static str, &'static [u8], &'static [u8]) {
        match self {
            Self::ChildDocument => ("text/html", b"<!doctype html><p>\0\xff", b"child-end</p>"),
            Self::Stylesheet | Self::StyleImport | Self::Preload | Self::ParserStylesheet => {
                ("text/css", b"/*\0\xff", b"*/body{color:red}")
            }
            _ => (
                "text/javascript",
                b"/*\0\xff",
                b"*/globalThis.nativeScriptLoaded=true;",
            ),
        }
    }
}

macro_rules! document_loader_stage_tests {
    ($($name:ident: $kind:ident, $controlled:literal, $finish:ident;)*) => {
        $(#[tokio::test]
        async fn $name() { resource_response_stages($controlled, Finish::$finish, RequestKind::$kind).await; })*
    };
}

document_loader_stage_tests! {
    native_document_loader_child_failed_redirect: ChildDocument, false, FailedRedirect;
    native_document_loader_parser_script_failed_redirect: ParserClassic, false, FailedRedirect;
    native_document_loader_parser_style_failed_redirect: ParserStylesheet, false, FailedRedirect;
    native_document_loader_stylesheet_http_complete: Stylesheet, false, Complete;
    native_document_loader_stylesheet_http_partial: Stylesheet, false, Partial;
    native_document_loader_stylesheet_http_close_before_head: Stylesheet, false, CloseBeforeHead;
    native_document_loader_stylesheet_http_close_after_prefix: Stylesheet, false, CloseAfterPrefix;
    native_document_loader_stylesheet_controlled_complete: Stylesheet, true, Complete;
    native_document_loader_stylesheet_controlled_partial: Stylesheet, true, Partial;
    native_document_loader_stylesheet_controlled_close_before_head: Stylesheet, true, CloseBeforeHead;
    native_document_loader_stylesheet_controlled_close_after_prefix: Stylesheet, true, CloseAfterPrefix;
    native_document_loader_style_import_http_complete: StyleImport, false, Complete;
    native_document_loader_style_import_http_partial: StyleImport, false, Partial;
    native_document_loader_style_import_http_close_before_head: StyleImport, false, CloseBeforeHead;
    native_document_loader_style_import_http_close_after_prefix: StyleImport, false, CloseAfterPrefix;
    native_document_loader_style_import_controlled_complete: StyleImport, true, Complete;
    native_document_loader_style_import_controlled_partial: StyleImport, true, Partial;
    native_document_loader_style_import_controlled_close_before_head: StyleImport, true, CloseBeforeHead;
    native_document_loader_style_import_controlled_close_after_prefix: StyleImport, true, CloseAfterPrefix;
    native_document_loader_preload_http_complete: Preload, false, Complete;
    native_document_loader_preload_http_partial: Preload, false, Partial;
    native_document_loader_preload_http_close_before_head: Preload, false, CloseBeforeHead;
    native_document_loader_preload_http_close_after_prefix: Preload, false, CloseAfterPrefix;
    native_document_loader_preload_controlled_complete: Preload, true, Complete;
    native_document_loader_preload_controlled_partial: Preload, true, Partial;
    native_document_loader_preload_controlled_close_before_head: Preload, true, CloseBeforeHead;
    native_document_loader_preload_controlled_close_after_prefix: Preload, true, CloseAfterPrefix;
}

document_loader_stage_tests! {
    native_document_loader_runtime_classic_http_complete: RuntimeClassic, false, Complete;
    native_document_loader_runtime_classic_http_partial: RuntimeClassic, false, Partial;
    native_document_loader_runtime_classic_http_close_before_head: RuntimeClassic, false, CloseBeforeHead;
    native_document_loader_runtime_classic_http_close_after_prefix: RuntimeClassic, false, CloseAfterPrefix;
    native_document_loader_runtime_classic_controlled_complete: RuntimeClassic, true, Complete;
    native_document_loader_runtime_classic_controlled_partial: RuntimeClassic, true, Partial;
    native_document_loader_runtime_classic_controlled_close_before_head: RuntimeClassic, true, CloseBeforeHead;
    native_document_loader_runtime_classic_controlled_close_after_prefix: RuntimeClassic, true, CloseAfterPrefix;
    native_document_loader_runtime_module_http_complete: RuntimeModule, false, Complete;
    native_document_loader_runtime_module_http_partial: RuntimeModule, false, Partial;
    native_document_loader_runtime_module_http_close_before_head: RuntimeModule, false, CloseBeforeHead;
    native_document_loader_runtime_module_http_close_after_prefix: RuntimeModule, false, CloseAfterPrefix;
    native_document_loader_runtime_module_controlled_complete: RuntimeModule, true, Complete;
    native_document_loader_runtime_module_controlled_partial: RuntimeModule, true, Partial;
    native_document_loader_runtime_module_controlled_close_before_head: RuntimeModule, true, CloseBeforeHead;
    native_document_loader_runtime_module_controlled_close_after_prefix: RuntimeModule, true, CloseAfterPrefix;
    native_document_loader_dynamic_import_http_complete: DynamicImport, false, Complete;
    native_document_loader_dynamic_import_http_partial: DynamicImport, false, Partial;
    native_document_loader_dynamic_import_http_close_before_head: DynamicImport, false, CloseBeforeHead;
    native_document_loader_dynamic_import_http_close_after_prefix: DynamicImport, false, CloseAfterPrefix;
    native_document_loader_dynamic_import_controlled_complete: DynamicImport, true, Complete;
    native_document_loader_dynamic_import_controlled_partial: DynamicImport, true, Partial;
    native_document_loader_dynamic_import_controlled_close_before_head: DynamicImport, true, CloseBeforeHead;
    native_document_loader_dynamic_import_controlled_close_after_prefix: DynamicImport, true, CloseAfterPrefix;
    native_document_loader_script_preload_http_complete: ScriptPreload, false, Complete;
    native_document_loader_script_preload_http_partial: ScriptPreload, false, Partial;
    native_document_loader_script_preload_http_close_before_head: ScriptPreload, false, CloseBeforeHead;
    native_document_loader_script_preload_http_close_after_prefix: ScriptPreload, false, CloseAfterPrefix;
    native_document_loader_script_preload_controlled_complete: ScriptPreload, true, Complete;
    native_document_loader_script_preload_controlled_partial: ScriptPreload, true, Partial;
    native_document_loader_script_preload_controlled_close_before_head: ScriptPreload, true, CloseBeforeHead;
    native_document_loader_script_preload_controlled_close_after_prefix: ScriptPreload, true, CloseAfterPrefix;
    native_document_loader_child_classic_http_complete: ChildClassic, false, Complete;
    native_document_loader_child_classic_http_partial: ChildClassic, false, Partial;
    native_document_loader_child_classic_http_close_before_head: ChildClassic, false, CloseBeforeHead;
    native_document_loader_child_classic_http_close_after_prefix: ChildClassic, false, CloseAfterPrefix;
    native_document_loader_child_classic_controlled_complete: ChildClassic, true, Complete;
    native_document_loader_child_classic_controlled_partial: ChildClassic, true, Partial;
    native_document_loader_child_classic_controlled_close_before_head: ChildClassic, true, CloseBeforeHead;
    native_document_loader_child_classic_controlled_close_after_prefix: ChildClassic, true, CloseAfterPrefix;
    native_document_loader_child_module_http_complete: ChildModule, false, Complete;
    native_document_loader_child_module_http_partial: ChildModule, false, Partial;
    native_document_loader_child_module_http_close_before_head: ChildModule, false, CloseBeforeHead;
    native_document_loader_child_module_http_close_after_prefix: ChildModule, false, CloseAfterPrefix;
    native_document_loader_child_module_controlled_complete: ChildModule, true, Complete;
    native_document_loader_child_module_controlled_partial: ChildModule, true, Partial;
    native_document_loader_child_module_controlled_close_before_head: ChildModule, true, CloseBeforeHead;
    native_document_loader_child_module_controlled_close_after_prefix: ChildModule, true, CloseAfterPrefix;
}

document_loader_stage_tests! {
    native_document_loader_parser_classic_http_complete: ParserClassic, false, Complete;
    native_document_loader_parser_classic_http_partial: ParserClassic, false, Partial;
    native_document_loader_parser_classic_http_close_before_head: ParserClassic, false, CloseBeforeHead;
    native_document_loader_parser_classic_http_close_after_prefix: ParserClassic, false, CloseAfterPrefix;
    native_document_loader_parser_classic_controlled_complete: ParserClassic, true, Complete;
    native_document_loader_parser_classic_controlled_partial: ParserClassic, true, Partial;
    native_document_loader_parser_classic_controlled_close_before_head: ParserClassic, true, CloseBeforeHead;
    native_document_loader_parser_classic_controlled_close_after_prefix: ParserClassic, true, CloseAfterPrefix;
    native_document_loader_parser_module_http_complete: ParserModule, false, Complete;
    native_document_loader_parser_module_http_partial: ParserModule, false, Partial;
    native_document_loader_parser_module_http_close_before_head: ParserModule, false, CloseBeforeHead;
    native_document_loader_parser_module_http_close_after_prefix: ParserModule, false, CloseAfterPrefix;
    native_document_loader_parser_module_controlled_complete: ParserModule, true, Complete;
    native_document_loader_parser_module_controlled_partial: ParserModule, true, Partial;
    native_document_loader_parser_module_controlled_close_before_head: ParserModule, true, CloseBeforeHead;
    native_document_loader_parser_module_controlled_close_after_prefix: ParserModule, true, CloseAfterPrefix;
    native_document_loader_parser_stylesheet_http_complete: ParserStylesheet, false, Complete;
    native_document_loader_parser_stylesheet_http_partial: ParserStylesheet, false, Partial;
    native_document_loader_parser_stylesheet_http_close_before_head: ParserStylesheet, false, CloseBeforeHead;
    native_document_loader_parser_stylesheet_http_close_after_prefix: ParserStylesheet, false, CloseAfterPrefix;
    native_document_loader_parser_stylesheet_controlled_complete: ParserStylesheet, true, Complete;
    native_document_loader_parser_stylesheet_controlled_partial: ParserStylesheet, true, Partial;
    native_document_loader_parser_stylesheet_controlled_close_before_head: ParserStylesheet, true, CloseBeforeHead;
    native_document_loader_parser_stylesheet_controlled_close_after_prefix: ParserStylesheet, true, CloseAfterPrefix;
    native_document_loader_parser_preload_http_complete: ParserPreload, false, Complete;
    native_document_loader_parser_preload_http_partial: ParserPreload, false, Partial;
    native_document_loader_parser_preload_http_close_before_head: ParserPreload, false, CloseBeforeHead;
    native_document_loader_parser_preload_http_close_after_prefix: ParserPreload, false, CloseAfterPrefix;
    native_document_loader_parser_preload_controlled_complete: ParserPreload, true, Complete;
    native_document_loader_parser_preload_controlled_partial: ParserPreload, true, Partial;
    native_document_loader_parser_preload_controlled_close_before_head: ParserPreload, true, CloseBeforeHead;
    native_document_loader_parser_preload_controlled_close_after_prefix: ParserPreload, true, CloseAfterPrefix;
    native_document_loader_document_write_http_complete: DocumentWrite, false, Complete;
    native_document_loader_document_write_http_partial: DocumentWrite, false, Partial;
    native_document_loader_document_write_http_close_before_head: DocumentWrite, false, CloseBeforeHead;
    native_document_loader_document_write_http_close_after_prefix: DocumentWrite, false, CloseAfterPrefix;
    native_document_loader_document_write_controlled_complete: DocumentWrite, true, Complete;
    native_document_loader_document_write_controlled_partial: DocumentWrite, true, Partial;
    native_document_loader_document_write_controlled_close_before_head: DocumentWrite, true, CloseBeforeHead;
    native_document_loader_document_write_controlled_close_after_prefix: DocumentWrite, true, CloseAfterPrefix;
}

async fn next_item(
    events: &mut BrowserEventReceiver,
    document: DocumentHandle,
    source: &RendererNetworkSource,
    handle: SubresourceNetworkRequestHandle,
) -> (
    crate::browser::BrowserSequence,
    Arc<ScriptNetworkOutputItem>,
) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let record = events.recv().await.unwrap();
            if let BrowserEvent::NetworkSourceClosed { source: closed, .. } = &record.event {
                assert_ne!(
                    *closed,
                    source.identity(),
                    "source closure cannot overtake its admitted child response"
                );
            }
            let occurrence = match record.event {
                BrowserEvent::NetworkRequestStarted(occurrence)
                | BrowserEvent::NetworkActivity(occurrence)
                | BrowserEvent::NetworkRequestCompleted(occurrence)
                    if occurrence.owner == NetworkOwner::Document(document)
                        && occurrence.renderer.source == *source =>
                {
                    occurrence
                }
                _ => continue,
            };
            let RendererNetworkOutputItem::Resource(item) = &occurrence.renderer.item else {
                continue;
            };
            let belongs = match item.as_ref() {
                ScriptNetworkOutputItem::SubresourceRequestStarted(request) => {
                    request.handle() == handle
                }
                ScriptNetworkOutputItem::SubresourceResponseStarted(response) => {
                    response.handle() == handle
                }
                ScriptNetworkOutputItem::SubresourceDataReceived(data) => data.handle() == handle,
                ScriptNetworkOutputItem::SubresourceBodyFinished(body) => body.handle() == handle,
                _ => false,
            };
            if belongs {
                return (record.sequence, item.clone());
            }
        }
    })
    .await
    .expect("original child phase must precede the next physical gate")
}

async fn resource_response_stages(controlled: bool, finish: Finish, kind: RequestKind) {
    let (_, prefix_bytes, tail_bytes) = kind.response_body();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let child_url = format!("{origin}/child");
    let (requested, request_arrived) = oneshot::channel();
    let (headers, release_headers) = oneshot::channel();
    let (prefix, release_prefix) = oneshot::channel();
    let (tail, release_tail) = oneshot::channel();
    let (closed, peer_closed) = oneshot::channel();
    let server = tokio::spawn(async move {
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
            if path == "/child" && matches!(finish, Finish::FailedRedirect) {
                requested.send(()).unwrap();
                release_headers.await.unwrap();
                stream.write_all(b"HTTP/1.1 303 See Other\r\nLocation: /failed\r\nX-Redirect: observed\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
                drop(stream);
                let (mut failed, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    failed.read_exact(&mut byte).await.unwrap();
                    request.push(byte[0]);
                }
                assert!(request.starts_with(b"GET /failed HTTP/"));
                // Fail the original redirected transport before any response head.
                drop(failed);
                return;
            }
            let (mime, body) = match path {
                "/setup" => ("text/html", "<!doctype html><title>controller setup</title>".to_owned()),
                "/" if kind.parser_html().is_some() => ("text/html", kind.parser_html().unwrap().to_owned()),
                "/" => {
                    let bootstrap = match kind {
                        RequestKind::ChildDocument => "const frame=document.createElement('iframe');frame.id='stream-child';frame.src='/child';document.body.appendChild(frame)",
                        RequestKind::Stylesheet => "const link=document.createElement('link');link.rel='stylesheet';link.href='/child';document.head.appendChild(link)",
                        RequestKind::StyleImport => "const style=document.createElement('style');style.textContent=\"@import url('/child');\";document.head.appendChild(style)",
                        RequestKind::Preload => "const link=document.createElement('link');link.rel='preload';link.as='fetch';link.href='/child';document.head.appendChild(link)",
                        RequestKind::RuntimeClassic => "const script=document.createElement('script');script.src='/child';document.head.appendChild(script)",
                        RequestKind::RuntimeModule => "const script=document.createElement('script');script.type='module';script.src='/child';document.head.appendChild(script)",
                        RequestKind::DynamicImport => "import('/child').catch(()=>{})",
                        RequestKind::ScriptPreload => "const link=document.createElement('link');link.rel='preload';link.as='script';link.href='/child';document.head.appendChild(link)",
                        RequestKind::ChildClassic => "const frame=document.createElement('iframe');frame.id='stream-child';frame.src='/frame';document.body.appendChild(frame)",
                        RequestKind::ChildModule => "const frame=document.createElement('iframe');frame.id='stream-child';frame.src='/frame';document.body.appendChild(frame)",
                        _ => unreachable!("parser fixture uses the document response directly"),
                    };
                    let script = if controlled {
                        format!("(async()=>{{await navigator.serviceWorker.register('/sw.js');await navigator.serviceWorker.ready;if(!navigator.serviceWorker.controller)await new Promise(r=>navigator.serviceWorker.addEventListener('controllerchange',r,{{once:true}}));{bootstrap}}})()")
                    } else { bootstrap.to_owned() };
                    ("text/html", format!("<!doctype html><body><script>{script}</script>"))
                }
                "/frame" => ("text/html", if matches!(kind, RequestKind::ChildModule) {
                    "<!doctype html><script type='module' src='/child'></script>".to_owned()
                } else {
                    "<!doctype html><script src='/child'></script>".to_owned()
                }),
                "/sw.js" if controlled => ("text/javascript", "self.addEventListener('install',e=>e.waitUntil(self.skipWaiting()));self.addEventListener('activate',e=>e.waitUntil(self.clients.claim()));self.addEventListener('fetch',e=>{if(new URL(e.request.url).pathname==='/child')e.respondWith(fetch('/upstream',{signal:e.request.signal}).then(r=>new Response(r.body,{status:r.status,headers:r.headers})))})".to_owned()),
                "/child" if !controlled => break serve_resource(stream, kind, finish, requested, release_headers, release_prefix, release_tail, closed).await,
                "/upstream" if controlled => break serve_resource(stream, kind, finish, requested, release_headers, release_prefix, release_tail, closed).await,
                _ => panic!("unexpected request: {request}"),
            };
            stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        }
    });
    let service = BrowserService::start().unwrap();
    let (context, contents) = context_with_contents(&service);
    let (_, mut events) = service.handle().subscribe().unwrap();
    if kind.parser_html().is_some() && controlled {
        let setup = navigate(&context, contents, &format!("{origin}/setup")).await;
        let ready = tokio::time::timeout(Duration::from_secs(5), context.evaluate_document_expression_for_test(
            setup,
            "(async()=>{await navigator.serviceWorker.register('/sw.js');await navigator.serviceWorker.ready;if(!navigator.serviceWorker.controller)await new Promise(r=>navigator.serviceWorker.addEventListener('controllerchange',r,{once:true}));return true})()",
            true,
        )).await.expect("controller must be active before parser navigation").unwrap();
        assert_eq!(ready["value"], true);
    }
    let document = if kind.parser_html().is_some() {
        commit_navigation(&context, contents, &format!("{origin}/")).await
    } else {
        navigate(&context, contents, &format!("{origin}/")).await
    };
    tokio::time::timeout(Duration::from_secs(5), request_arrived)
        .await
        .unwrap()
        .unwrap();
    let (mut sequence, source, handle) = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let record = events.recv().await.unwrap();
            if let BrowserEvent::NetworkRequestStarted(occurrence) = record.event
                && occurrence.owner == NetworkOwner::Document(document)
                && let RendererNetworkOutputItem::Resource(item) = &occurrence.renderer.item
                && let ScriptNetworkOutputItem::SubresourceRequestStarted(request) = item.as_ref()
                && request.url().as_str() == child_url
            {
                let expected_type = match kind {
                    RequestKind::ChildDocument => SubresourceResourceType::Document,
                    RequestKind::Stylesheet
                    | RequestKind::StyleImport
                    | RequestKind::ParserStylesheet => SubresourceResourceType::Stylesheet,
                    RequestKind::Preload => SubresourceResourceType::Fetch,
                    _ => SubresourceResourceType::Script,
                };
                assert_eq!(request.resource_type(), expected_type);
                if matches!(kind, RequestKind::ChildDocument) {
                    assert!(
                        request.frame_id().is_some() && request.navigation_loader_id().is_some()
                    );
                } else {
                    assert!(request.navigation_loader_id().is_none());
                    if matches!(kind, RequestKind::ChildClassic | RequestKind::ChildModule) {
                        assert!(
                            request.frame_id().is_some(),
                            "script must retain its actual child frame"
                        );
                    }
                    let initiator = if matches!(
                        kind,
                        RequestKind::RuntimeClassic
                            | RequestKind::RuntimeModule
                            | RequestKind::DynamicImport
                    ) {
                        crate::page::SubresourceRequestInitiatorType::Script
                    } else if matches!(kind, RequestKind::StyleImport) {
                        crate::page::SubresourceRequestInitiatorType::Css
                    } else {
                        crate::page::SubresourceRequestInitiatorType::Parser
                    };
                    assert_eq!(request.request_initiator_type(), initiator);
                }
                return (
                    record.sequence,
                    occurrence.renderer.source.clone(),
                    request.handle(),
                );
            }
        }
    })
    .await
    .unwrap();
    let mut received = 0;
    if matches!(finish, Finish::FailedRedirect) {
        headers.send(()).unwrap();
        let (next, terminal) = next_item(&mut events, document, &source, handle).await;
        assert!(next > sequence);
        let ScriptNetworkOutputItem::SubresourceBodyFinished(terminal) = terminal.as_ref() else {
            panic!("failed redirect has no final response head: {terminal:?}")
        };
        assert!(matches!(
            terminal.result(),
            SubresourceBodyFinishedResult::Failed(_)
        ));
        let failure = terminal
            .failure_context()
            .expect("native failure retains the actual transport context");
        let request = failure
            .request_context()
            .expect("the fetch owns its final request");
        assert_eq!(request.current_url().as_str(), format!("{origin}/failed"));
        assert_eq!(request.request_method(), "GET");
        assert!(request.request_body().is_none());
        let [redirect] = request.redirect_chain() else {
            panic!("one real redirect")
        };
        assert_eq!(redirect.from_url.as_str(), child_url);
        assert_eq!(redirect.to_url, *request.current_url());
        assert_eq!(redirect.status, 303);
        assert!(
            redirect
                .headers
                .iter()
                .any(|(key, value)| key.eq_ignore_ascii_case("x-redirect") && value == b"observed")
        );
        let exchanges = failure.observation_journal().exchanges();
        assert_eq!(exchanges.len(), 2);
        assert_eq!(exchanges[0].response().unwrap().status(), 303);
        assert!(exchanges[1].response().is_none());
        server.await.unwrap();
        context
            .close_web_contents(contents)
            .unwrap()
            .close_async()
            .await;
        service.shutdown();
        return;
    }
    if !matches!(finish, Finish::RemoveBeforeHead | Finish::CloseBeforeHead) {
        headers.send(()).unwrap();
        let (next, head) = next_item(&mut events, document, &source, handle).await;
        assert!(next > sequence);
        sequence = next;
        let ScriptNetworkOutputItem::SubresourceResponseStarted(head) = head.as_ref() else {
            panic!("head before body: {head:?}")
        };
        assert_eq!(head.status(), 200);
        assert_eq!(head.final_url().as_str(), child_url);
        prefix.send(()).unwrap();
        while received < prefix_bytes.len() {
            let (next, item) = next_item(&mut events, document, &source, handle).await;
            assert!(next > sequence);
            sequence = next;
            let ScriptNetworkOutputItem::SubresourceDataReceived(data) = item.as_ref() else {
                panic!("prefix before terminal: {item:?}")
            };
            received += data.data_length();
        }
        assert_eq!(received, prefix_bytes.len());
    }
    if finish.cancels() {
        if matches!(finish, Finish::CloseAfterPrefix | Finish::CloseBeforeHead) {
            context
                .close_web_contents(contents)
                .unwrap()
                .close_async()
                .await;
        } else {
            assert_eq!(context.evaluate_document_expression_for_test(document, "document.getElementById('stream-child').remove();document.getElementById('stream-child')===null", false).await.unwrap()["value"], true);
        }
        let physically_closed = tokio::time::timeout(Duration::from_secs(5), peer_closed).await;
        // Release the fixture after recording the gate result, including on a red run.
        let _ = tail.send(());
        physically_closed
            .expect("retirement must close the original upstream before fixture release")
            .unwrap();
    } else {
        tail.send(()).unwrap();
    }
    let terminal = loop {
        let (next, item) = next_item(&mut events, document, &source, handle).await;
        assert!(next > sequence);
        sequence = next;
        match item.as_ref() {
            ScriptNetworkOutputItem::SubresourceDataReceived(data) => {
                received += data.data_length()
            }
            ScriptNetworkOutputItem::SubresourceBodyFinished(body) => break body.clone(),
            other => panic!("a request has one response head and one terminal: {other:?}"),
        }
    };
    let expected = if matches!(finish, Finish::Complete) {
        [prefix_bytes, tail_bytes].concat()
    } else if matches!(finish, Finish::RemoveBeforeHead | Finish::CloseBeforeHead) {
        Vec::new()
    } else {
        prefix_bytes.to_vec()
    };
    assert_eq!(received, expected.len());
    match (finish, terminal.result()) {
        (Finish::Complete, SubresourceBodyFinishedResult::Ready(body)) => {
            assert_eq!(body.clone_body_bytes(), expected)
        }
        (
            Finish::RemoveBeforeHead | Finish::CloseBeforeHead,
            SubresourceBodyFinishedResult::Failed(error),
        ) => {
            assert!(!error.is_empty())
        }
        (
            Finish::Partial | Finish::RemoveAfterPrefix | Finish::CloseAfterPrefix,
            SubresourceBodyFinishedResult::FailedWithPartialBody {
                partial_body,
                error_text,
            },
        ) => {
            assert!(!error_text.is_empty());
            assert_eq!(partial_body.clone_body_bytes(), expected);
        }
        other => panic!("original physical outcome must survive: {other:?}"),
    }
    server.await.unwrap();
    if !matches!(finish, Finish::CloseAfterPrefix | Finish::CloseBeforeHead) {
        context
            .close_web_contents(contents)
            .unwrap()
            .close_async()
            .await;
    }
    service.shutdown();

    async fn serve_resource(
        mut stream: tokio::net::TcpStream,
        kind: RequestKind,
        finish: Finish,
        requested: oneshot::Sender<()>,
        headers: oneshot::Receiver<()>,
        prefix: oneshot::Receiver<()>,
        tail: oneshot::Receiver<()>,
        closed: oneshot::Sender<()>,
    ) {
        let (mime, prefix_bytes, tail_bytes) = kind.response_body();
        requested.send(()).unwrap();
        if !matches!(finish, Finish::RemoveBeforeHead | Finish::CloseBeforeHead) {
            headers.await.unwrap();
            stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", prefix_bytes.len()+tail_bytes.len()).as_bytes()).await.unwrap();
            prefix.await.unwrap();
            stream.write_all(prefix_bytes).await.unwrap();
        }
        if finish.cancels() {
            let mut byte = [0];
            tokio::select! {
                result = stream.read(&mut byte) => { assert_eq!(result.unwrap(), 0); let _ = closed.send(()); }
                _ = tail => {}
            }
        } else {
            tail.await.unwrap();
            if matches!(finish, Finish::Complete) {
                stream.write_all(tail_bytes).await.unwrap();
            }
        }
    }
}
