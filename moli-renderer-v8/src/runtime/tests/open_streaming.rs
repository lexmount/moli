use super::*;

const ASYNC_SCRIPT_HTML: &str = r#"<!doctype html><body><script async src="/work.js"></script>"#;
const ASYNC_SUBRESOURCE_HTML: &str = r#"<!doctype html><body><script>
fetch('/work.js')
  .then(response => response.text())
  .then(source => (0, eval)(source));
</script>"#;
const STATIC_MODULE_HTML: &str =
    r#"<!doctype html><body><script type="module" src="/root.js"></script>"#;
const CHILD_DOCUMENT_HTML: &str = r#"<!doctype html><body><iframe src="/child.html"></iframe>"#;
const PARSER_MODULEPRELOAD_PATH: &str = "/parser-open-stream-modulepreload.mjs";
const DOCUMENT_WRITE_MODULEPRELOAD_PATH: &str = "/document-write-open-stream-modulepreload.mjs";

struct OpenStreamingPage {
    page: RendererPageHandle,
    _runtime_owner: crate::JsRuntimeOwner,
    _loader_owner: crate::network::ResourceRequestClientOwner,
    body_tx: mpsc::Sender<Vec<u8>>,
    completion_tx: oneshot::Sender<anyhow::Result<()>>,
    activity_wake_rx: RendererExternalActivityTestReceiver,
}

const SELF_REPLACE_WHILE_PARSING_HTML: &str = r#"<!doctype html><script>
globalThis.__sourceDocument = true;
setTimeout(() => location.replace(location.href), 0);
</script>"#;

#[tokio::test(flavor = "multi_thread")]
async fn location_replace_commits_while_source_phase_one_stream_remains_open() {
    let (base_url, server) = spawn_owner_wake_server_with_content_type(
        "/page",
        "<!doctype html><main id='replacement'>replacement document</main>",
        "text/html",
        Duration::ZERO,
    )
    .await;
    // The timer runs as Page work after the parser has parked waiting for
    // more body input. Keep the source stream open throughout the commit.
    let mut source = OpenStreamingPage::create(&base_url, SELF_REPLACE_WHILE_PARSING_HTML).await;

    tokio::time::timeout(
        Duration::from_secs(5),
        recv_page_lifecycle_until(
            &mut source.activity_wake_rx,
            &source.page,
            RendererDocumentLifecycleMilestone::Load,
        ),
    )
    .await
    .expect("self-replacement must reach Load before the source stream reaches EOF");
    tokio::time::timeout(Duration::from_secs(5), source.body_tx.closed())
        .await
        .expect("committing the replacement must release the source body receiver");
    assert!(
        source.completion_tx.send(Ok(())).is_err(),
        "the discarded phase-one bridge must release its completion receiver"
    );
    let (reply, _) = source
        .page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "JSON.stringify([!!document.getElementById('replacement'), document.readyState, history.length, typeof __sourceDocument])".to_owned(),
            await_promise: false,
        })
        .await
        .expect("replacement Document should remain usable after phase-one settlement");
    assert_eq!(
        renderer_json_value(reply),
        Some(serde_json::json!("[true,\"complete\",1,\"undefined\"]"))
    );
    let state = RendererPageTestingHandle::new_for_testing(&source.page)
        .current_page_state_async()
        .await
        .expect("replacement state should be published");
    assert_eq!(state.final_url().as_str(), format!("{base_url}/page"));
    assert_eq!(
        has_pending_location_navigation_for_test(&source.page).await,
        Some(false)
    );

    source
        .page
        .close_async()
        .await
        .expect("replacement Page should close");
    server.await.expect("self-replacement server should finish");
}

#[tokio::test(flavor = "multi_thread")]
async fn location_replace_without_document_preserves_pending_phase_one_stream() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind no-Document navigation server");
    let address = listener.local_addr().expect("navigation server address");
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("accept self-replacement request");
        let request = read_owner_wake_http_request_head(&mut stream).await;
        assert_eq!(request.lines().next().unwrap(), "GET /page HTTP/1.1");
        stream
            .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .await
            .expect("write no-Document navigation response");
    });
    let mut source = OpenStreamingPage::create(
        &format!("http://{address}"),
        SELF_REPLACE_WHILE_PARSING_HTML,
    )
    .await;
    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .expect("self-replacement request should reach the server")
        .expect("no-Document navigation server should finish");
    // This command is ordered after the checked-out navigation task, so the
    // 204 has been settled before the remaining source bytes are delivered.
    let (reply, _) = source
        .page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "__sourceDocument".to_owned(),
            await_promise: false,
        })
        .await
        .expect("the source Document must remain active after a 204 navigation");
    assert_eq!(renderer_json_value(reply), Some(serde_json::json!(true)));
    source
        .body_tx
        .send(b"<script>globalThis.__sourceTailParsed = true;</script>".to_vec())
        .await
        .expect("the pending parser must retain its source body receiver");
    drop(source.body_tx);
    source
        .completion_tx
        .send(Ok(()))
        .expect("source body should finish");
    tokio::time::timeout(
        Duration::from_secs(5),
        recv_page_lifecycle_until(
            &mut source.activity_wake_rx,
            &source.page,
            RendererDocumentLifecycleMilestone::Load,
        ),
    )
    .await
    .expect("the source parser should resume and reach Load after the 204");
    let (reply, _) = source
        .page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression:
                "JSON.stringify([__sourceDocument, __sourceTailParsed, document.readyState])"
                    .to_owned(),
            await_promise: false,
        })
        .await
        .expect("source parser tail should execute in the original Document");
    assert_eq!(
        renderer_json_value(reply),
        Some(serde_json::json!("[true,true,\"complete\"]"))
    );
    source
        .page
        .close_async()
        .await
        .expect("source Page should close");
}

#[tokio::test(flavor = "multi_thread")]
async fn location_replace_commit_precondition_failure_preserves_open_source_stream() {
    assert_failed_prepared_navigation_preserves_open_source_stream(
        "x-moli-test-missing-navigation-script-environment",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn location_replace_window_proxy_validation_failure_preserves_open_source_stream() {
    assert_failed_prepared_navigation_preserves_open_source_stream(
        "x-moli-test-mismatched-navigation-page-id",
    )
    .await;
}

async fn assert_failed_prepared_navigation_preserves_open_source_stream(
    injection_header: &'static str,
) {
    let (base_url, server) = spawn_owner_wake_server_with_content_type_and_headers(
        "/page",
        "<!doctype html><main id='replacement'>replacement document</main>",
        "text/html",
        vec![(injection_header, "1")],
        Duration::ZERO,
    )
    .await;
    let mut source = OpenStreamingPage::create(
        &base_url,
        r#"<!doctype html><script>
globalThis.__sourceWindow = window;
globalThis.__sourceDocument = document;
setTimeout(() => location.replace(location.href), 0);
</script>"#,
    )
    .await;
    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .expect("the replacement response must exist before testing commit failure")
        .expect("prepared navigation failure server should finish");

    // Owner commands are ordered after the checked-out navigation task. The
    // response reached Prepared, but its fallible commit must return a Live
    // entry with the original context, inspector and parser still resident.
    let messages = dispatch_runtime_protocol_for_test(
        &source.page,
        serde_json::json!({
            "id": 1,
            "method": "Runtime.evaluate",
            "params": {
                "expression": "window === __sourceWindow && document === __sourceDocument && !document.getElementById('replacement')",
                "returnByValue": true
            }
        }),
    )
    .await
    .expect("commit validation failure must preserve the source Inspector context");
    let response = runtime_protocol_response_by_id(&messages, 1)
        .expect("source Inspector evaluation should reply");
    assert_eq!(response["result"]["result"]["value"], true);
    assert_eq!(
        has_pending_location_navigation_for_test(&source.page).await,
        Some(false)
    );
    assert!(
        !source.body_tx.is_closed(),
        "a failed Prepared commit must retain the source body receiver"
    );
    source
        .body_tx
        .send(b"<script>globalThis.__sourceTailParsed = true;</script>".to_vec())
        .await
        .expect("the retained source parser must accept later body bytes");
    drop(source.body_tx);
    source
        .completion_tx
        .send(Ok(()))
        .expect("the original source stream must retain its completion receiver");
    let events = tokio::time::timeout(
        Duration::from_secs(5),
        recv_page_lifecycle_until(
            &mut source.activity_wake_rx,
            &source.page,
            RendererDocumentLifecycleMilestone::Load,
        ),
    )
    .await
    .expect("the original source parser must resume and reach Load after commit failure");
    assert!(
        !events.iter().any(|event| matches!(
            event.kind,
            RendererDocumentLifecycleEventKind::Terminated {
                reason: super::super::RendererDocumentTerminationReason::SupersededByCrossDocumentNavigation,
                ..
            }
        )),
        "a failed Prepared commit must not terminate the source lifecycle"
    );
    let (reply, _) = source
        .page
        .run_async_command(RendererPageCommand::EvaluateExpression {
            expression: "JSON.stringify([window === __sourceWindow, document === __sourceDocument, __sourceTailParsed, document.readyState])".to_owned(),
            await_promise: false,
        })
        .await
        .expect("the source Document must remain usable after its parser reaches EOF");
    assert_eq!(
        renderer_json_value(reply),
        Some(serde_json::json!("[true,true,true,\"complete\"]"))
    );
    source
        .page
        .close_async()
        .await
        .expect("source Page should close after a failed Prepared commit");
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_loop_executes_async_script_while_main_document_stream_remains_open() {
    assert_open_stream_work_executes_before_eof(ASYNC_SCRIPT_HTML, "async script").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_loop_executes_async_subresource_while_main_document_stream_remains_open() {
    assert_open_stream_work_executes_before_eof(ASYNC_SUBRESOURCE_HTML, "async subresource").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_loop_advances_static_module_graph_while_main_document_stream_remains_open() {
    let (base_url, root_request_seen, release_root_response, dependency_request_seen, server) =
        spawn_open_stream_module_graph_server().await;
    let page = OpenStreamingPage::create(&base_url, STATIC_MODULE_HTML).await;

    tokio::time::timeout(Duration::from_secs(2), root_request_seen)
        .await
        .expect("module root request should start before main-Document EOF")
        .expect("module root request signal should remain open");
    release_root_response
        .send(())
        .expect("module root response should release");
    tokio::time::timeout(Duration::from_secs(2), dependency_request_seen)
        .await
        .expect("static dependency request should start before main-Document EOF")
        .expect("static dependency request signal should remain open");

    page.finish().await;
    server
        .await
        .expect("open-stream module graph server should finish");
}

#[tokio::test(flavor = "multi_thread")]
async fn owner_loop_advances_child_document_while_main_document_stream_remains_open() {
    let (base_url, child_request_seen, release_child_response, child_effect_seen, server) =
        spawn_open_stream_child_document_server().await;
    let page = OpenStreamingPage::create(&base_url, CHILD_DOCUMENT_HTML).await;

    tokio::time::timeout(Duration::from_secs(2), child_request_seen)
        .await
        .expect("child Document request should start before main-Document EOF")
        .expect("child Document request signal should remain open");
    release_child_response
        .send(())
        .expect("child Document response should release");
    tokio::time::timeout(Duration::from_secs(2), child_effect_seen)
        .await
        .expect("child Document should execute before main-Document EOF")
        .expect("child Document effect signal should remain open");

    page.finish().await;
    server
        .await
        .expect("open-stream child Document server should finish");
}

#[tokio::test(flavor = "multi_thread")]
async fn parser_modulepreload_starts_before_main_document_stream_eof() {
    let (base_url, request_seen, release_response, server) =
        spawn_owner_wake_gated_server_with_content_type(
            PARSER_MODULEPRELOAD_PATH,
            "export const parserOpenStreamModulepreload = true;",
            "application/javascript",
        )
        .await;
    let html = format!(
        r#"<!doctype html><head><link rel="modulepreload" href="{base_url}{PARSER_MODULEPRELOAD_PATH}">"#
    );
    let page = OpenStreamingPage::create(&base_url, &html).await;

    tokio::time::timeout(Duration::from_secs(2), request_seen)
        .await
        .expect("parser modulepreload must start before the next body chunk or EOF")
        .expect("parser modulepreload request signal should remain open");
    release_response
        .send(())
        .expect("parser modulepreload response should release");

    page.finish().await;
    server
        .await
        .expect("parser modulepreload server should finish");
}

#[tokio::test(flavor = "multi_thread")]
async fn document_write_modulepreload_starts_before_main_document_stream_eof() {
    let (base_url, request_seen, release_response, server) =
        spawn_owner_wake_gated_server_with_content_type(
            DOCUMENT_WRITE_MODULEPRELOAD_PATH,
            "export const documentWriteOpenStreamModulepreload = true;",
            "application/javascript",
        )
        .await;
    let html = format!(
        r#"<!doctype html><script>
document.write(`<link rel="modulepreload" href="{base_url}{DOCUMENT_WRITE_MODULEPRELOAD_PATH}">`);
</script>"#
    );
    let page = OpenStreamingPage::create(&base_url, &html).await;

    tokio::time::timeout(Duration::from_secs(2), request_seen)
        .await
        .expect("document.write modulepreload must start before the next body chunk or EOF")
        .expect("document.write modulepreload request signal should remain open");
    release_response
        .send(())
        .expect("document.write modulepreload response should release");

    page.finish().await;
    server
        .await
        .expect("document.write modulepreload server should finish");
}

async fn assert_open_stream_work_executes_before_eof(html: &str, work_label: &str) {
    let (base_url, work_request_seen, release_work_response, effect_seen, server) =
        spawn_open_stream_work_effect_server().await;
    let page = OpenStreamingPage::create(&base_url, html).await;

    tokio::time::timeout(Duration::from_secs(2), work_request_seen)
        .await
        .unwrap_or_else(|_| panic!("{work_label} request should start before main-Document EOF"))
        .unwrap_or_else(|_| panic!("{work_label} request signal should remain open"));
    release_work_response
        .send(())
        .unwrap_or_else(|_| panic!("{work_label} response should release"));
    tokio::time::timeout(Duration::from_secs(2), effect_seen)
        .await
        .unwrap_or_else(|_| panic!("{work_label} should execute while the stream remains open"))
        .unwrap_or_else(|_| panic!("{work_label} effect signal should remain open"));

    page.finish().await;
    server.await.expect("open-stream server should finish");
}

impl OpenStreamingPage {
    async fn create(base_url: &str, html: &str) -> Self {
        let runtime_owner = JsRuntime::initialize();
        let runtime = runtime_owner.handle();
        let (activity_wake_tx, activity_wake_rx) = renderer_external_activity_test_channel();
        runtime.set_renderer_output_transport_sender(activity_wake_tx);
        let loader_owner = ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
            .expect("default loader");
        let loader = loader_owner.handle();
        let page_url = url::Url::parse(&format!("{base_url}/page")).expect("page url");
        let (completion_tx, completion_rx) = oneshot::channel();
        let (body_tx, raw_body) = ExternalRawDocumentBodyStream::channel(completion_rx);
        body_tx
            .send(html.as_bytes().to_vec())
            .await
            .expect("first open-stream body chunk should send");

        let (mut page, _, _, creation_artifacts, pending_download) = tokio::time::timeout(
            Duration::from_secs(2),
            runtime.create_streaming_raw_page_from_external_body(
                page_url.clone(),
                page_url,
                None,
                false,
                0,
                Vec::new(),
                200,
                vec![("content-type".to_owned(), b"text/html".to_vec())],
                &loader,
                crate::RendererWebStorageHandles::ephemeral(),
                raw_body,
                false,
                PageVmInitStage::Load,
                RendererReplyBoundary::DocumentCommit,
                RendererTopLevelNavigationDispatch::FollowInStandaloneAdapter,
                RendererNavigationReplyPolicy::FollowBeforeReply,
                None,
                None,
                crate::RendererDocumentOptions {
                    ..Default::default()
                },
            ),
        )
        .await
        .expect("open main-Document stream should attach at DocumentCommit")
        .expect("open main-Document stream should create a Page");
        assert!(pending_download.is_none());
        assert!(creation_artifacts.lifecycle_snapshot.load.is_none());
        page.take_committed_document_post_response_continuation()
            .expect("DocumentCommit should defer parser continuation")
            .release();

        Self {
            page,
            _runtime_owner: runtime_owner,
            _loader_owner: loader_owner,
            body_tx,
            completion_tx,
            activity_wake_rx,
        }
    }

    async fn finish(self) {
        let Self {
            mut page,
            _runtime_owner,
            _loader_owner,
            body_tx,
            completion_tx,
            mut activity_wake_rx,
        } = self;
        drop(body_tx);
        completion_tx
            .send(Ok(()))
            .expect("main-Document stream completion should send");

        tokio::time::timeout(Duration::from_secs(2), async {
            while let Some(publication) = activity_wake_rx.recv().await {
                if publication_document_lifecycle_events(&publication).any(|event| {
                    matches!(
                        event.kind,
                        RendererDocumentLifecycleEventKind::Milestone(
                            RendererDocumentLifecycleMilestone::Load
                        )
                    )
                }) {
                    return;
                }
            }
            panic!("external activity wake channel closed before open-stream Load")
        })
        .await
        .expect("open-stream Page should publish Load after its body terminal");

        page.close_async()
            .await
            .expect("open-stream Page should close");
    }
}

async fn spawn_open_stream_work_effect_server() -> (
    String,
    oneshot::Receiver<()>,
    oneshot::Sender<()>,
    oneshot::Receiver<()>,
    tokio::task::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind open-stream work server");
    let addr = listener.local_addr().expect("server local addr");
    let (work_request_seen_tx, work_request_seen_rx) = oneshot::channel();
    let (release_work_tx, release_work_rx) = oneshot::channel();
    let (effect_seen_tx, effect_seen_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut work_stream, _) = listener
            .accept()
            .await
            .expect("accept open-stream work request");
        let work_request = read_owner_wake_http_request_head(&mut work_stream).await;
        let work_path = work_request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .expect("open-stream work request path");
        assert_eq!(work_path, "/work.js");
        work_request_seen_tx
            .send(())
            .expect("signal open-stream work request");
        release_work_rx
            .await
            .expect("wait for open-stream work response release");
        let work_body = "fetch('/work-effect', { method: 'POST' });";
        let work_response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            work_body.len(),
            work_body
        );
        work_stream
            .write_all(work_response.as_bytes())
            .await
            .expect("write open-stream work response");

        let (mut effect_stream, _) = listener
            .accept()
            .await
            .expect("accept open-stream effect request");
        let effect_request = read_owner_wake_http_request_head(&mut effect_stream).await;
        let effect_path = effect_request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .expect("open-stream effect request path");
        assert_eq!(effect_path, "/work-effect");
        effect_seen_tx
            .send(())
            .expect("signal open-stream work effect");
        effect_stream
            .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .await
            .expect("write open-stream work effect response");
    });
    (
        format!("http://{addr}"),
        work_request_seen_rx,
        release_work_tx,
        effect_seen_rx,
        server,
    )
}

async fn spawn_open_stream_module_graph_server() -> (
    String,
    oneshot::Receiver<()>,
    oneshot::Sender<()>,
    oneshot::Receiver<()>,
    tokio::task::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind open-stream module graph server");
    let addr = listener.local_addr().expect("server local addr");
    let (root_seen_tx, root_seen_rx) = oneshot::channel();
    let (release_root_tx, release_root_rx) = oneshot::channel();
    let (dependency_seen_tx, dependency_seen_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut root_stream, _) = listener
            .accept()
            .await
            .expect("accept open-stream module root request");
        let root_request = read_owner_wake_http_request_head(&mut root_stream).await;
        let root_path = root_request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .expect("module root request path");
        assert_eq!(root_path, "/root.js");
        root_seen_tx
            .send(())
            .expect("signal open-stream module root request");
        release_root_rx
            .await
            .expect("wait for module root response release");
        let root_body = "import '/dependency.js';";
        let root_response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            root_body.len(),
            root_body
        );
        root_stream
            .write_all(root_response.as_bytes())
            .await
            .expect("write open-stream module root response");

        let (mut dependency_stream, _) = listener
            .accept()
            .await
            .expect("accept open-stream static dependency request");
        let dependency_request = read_owner_wake_http_request_head(&mut dependency_stream).await;
        let dependency_path = dependency_request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .expect("static dependency request path");
        assert_eq!(dependency_path, "/dependency.js");
        dependency_seen_tx
            .send(())
            .expect("signal open-stream static dependency request");
        dependency_stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .await
            .expect("write open-stream static dependency response");
    });
    (
        format!("http://{addr}"),
        root_seen_rx,
        release_root_tx,
        dependency_seen_rx,
        server,
    )
}

async fn spawn_open_stream_child_document_server() -> (
    String,
    oneshot::Receiver<()>,
    oneshot::Sender<()>,
    oneshot::Receiver<()>,
    tokio::task::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind open-stream child Document server");
    let addr = listener.local_addr().expect("server local addr");
    let (child_seen_tx, child_seen_rx) = oneshot::channel();
    let (release_child_tx, release_child_rx) = oneshot::channel();
    let (effect_seen_tx, effect_seen_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut child_stream, _) = listener
            .accept()
            .await
            .expect("accept open-stream child Document request");
        let child_request = read_owner_wake_http_request_head(&mut child_stream).await;
        let child_path = child_request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .expect("child Document request path");
        assert_eq!(child_path, "/child.html");
        child_seen_tx
            .send(())
            .expect("signal open-stream child Document request");
        release_child_rx
            .await
            .expect("wait for child Document response release");
        let child_body =
            "<!doctype html><script>fetch('/child-effect', { method: 'POST' });</script>";
        let child_response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            child_body.len(),
            child_body
        );
        child_stream
            .write_all(child_response.as_bytes())
            .await
            .expect("write open-stream child Document response");

        let (mut effect_stream, _) = listener
            .accept()
            .await
            .expect("accept open-stream child effect request");
        let effect_request = read_owner_wake_http_request_head(&mut effect_stream).await;
        let effect_path = effect_request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .expect("child effect request path");
        assert_eq!(effect_path, "/child-effect");
        effect_seen_tx
            .send(())
            .expect("signal open-stream child effect");
        effect_stream
            .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .await
            .expect("write open-stream child effect response");
    });
    (
        format!("http://{addr}"),
        child_seen_rx,
        release_child_tx,
        effect_seen_rx,
        server,
    )
}
