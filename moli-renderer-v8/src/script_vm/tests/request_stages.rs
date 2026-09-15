use super::*;
use crate::runtime::{RendererNetworkInput, RendererNetworkOutputItem};
use crate::types::{ScriptNetworkOutputItem, SubresourceBodyFinishedResult};
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

#[derive(Clone, Copy)]
enum MaterializedConsumer {
    SynchronousXhr,
    Manifest,
    Popup,
}

#[derive(Clone, Copy)]
enum MaterializedFinish {
    Complete,
    Partial,
    Cancelled,
}

macro_rules! materialized_consumer_stage_tests {
    ($($name:ident: $consumer:ident, $finish:ident;)*) => {
        $(#[tokio::test(flavor = "multi_thread")]
        async fn $name() -> anyhow::Result<()> {
            materialized_consumer_stages(MaterializedConsumer::$consumer, MaterializedFinish::$finish).await
        })*
    };
}

materialized_consumer_stage_tests! {
    popup_document_publishes_head_and_data_before_eof: Popup, Complete;
    popup_document_retains_partial_native_body: Popup, Partial;
    synchronous_window_xhr_publishes_head_and_data_before_eof: SynchronousXhr, Complete;
    synchronous_window_xhr_retains_partial_native_body: SynchronousXhr, Partial;
    synchronous_window_xhr_cancellation_closes_original_body: SynchronousXhr, Cancelled;
    manifest_retirement_closes_original_body_without_vm_result_publication: Manifest, Cancelled;
    manifest_publishes_stages_without_vm_result_publication: Manifest, Complete;
    manifest_retains_partial_native_body_without_vm_result_publication: Manifest, Partial;
}

async fn materialized_consumer_stages(
    consumer: MaterializedConsumer,
    finish: MaterializedFinish,
) -> anyhow::Result<()> {
    use anyhow::Context;

    let complete = matches!(finish, MaterializedFinish::Complete);
    let cancelled = matches!(finish, MaterializedFinish::Cancelled);
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let origin = format!("http://{}", listener.local_addr()?);
    let request_url = format!("{origin}/held");
    let payload: &[u8] = match consumer {
        MaterializedConsumer::SynchronousXhr | MaterializedConsumer::Popup => b"body",
        MaterializedConsumer::Manifest => br#"{"name":"native manifest"}"#,
    };
    let (arrived, request_arrived) = tokio::sync::oneshot::channel();
    let (prefix, release_prefix) = tokio::sync::oneshot::channel();
    let (tail, release_tail) = tokio::sync::oneshot::channel();
    let mut server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut head = Vec::new();
        while !head.ends_with(b"\r\n\r\n") {
            head.push(stream.read_u8().await.unwrap());
        }
        let head = String::from_utf8(head).unwrap();
        let length = head
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().unwrap())
            })
            .unwrap_or(0);
        let mut body = vec![0; length];
        stream.read_exact(&mut body).await.unwrap();
        match consumer {
            MaterializedConsumer::SynchronousXhr => {
                assert!(head.starts_with("POST /held HTTP/1.1\r\n"));
                assert_eq!(body, [0, 128, 255, 65]);
            }
            MaterializedConsumer::Manifest | MaterializedConsumer::Popup => {
                assert!(head.starts_with("GET /held HTTP/1.1\r\n"));
                assert!(body.is_empty());
            }
        }
        let content_type = if matches!(consumer, MaterializedConsumer::Popup) {
            "text/html"
        } else {
            "application/json"
        };
        stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", payload.len()).as_bytes()).await.unwrap();
        let _ = arrived.send(());
        if release_prefix.await.is_err() {
            return;
        }
        stream.write_all(&payload[..2]).await.unwrap();
        if cancelled {
            let read = tokio::time::timeout(Duration::from_secs(5), stream.read_u8())
                .await
                .unwrap();
            assert_eq!(
                read.unwrap_err().kind(),
                std::io::ErrorKind::UnexpectedEof,
                "retirement must close the original physical response while its tail is withheld"
            );
            return;
        }
        if release_tail.await.is_err() {
            return;
        }
        if complete {
            stream.write_all(&payload[2..]).await.unwrap();
        }
    });
    let (send, mut events) = tokio::sync::mpsc::unbounded_channel();
    let (cancel_send, cancel_receive) = tokio::sync::oneshot::channel();
    let vm_task = tokio::task::spawn_blocking(move || {
        let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
        let mut vm = crate::runtime::PageVmTaskExecutorTestHarness::new(
            Url::parse(&format!("{origin}/page")).unwrap(),
            &loader,
        );
        vm._context_host
            .borrow()
            .browser_context_runtime()
            .install_network_handler(move |input| {
                if let RendererNetworkInput::Observation(input) = input {
                    let _ = send.send(input.occurrence.clone());
                }
            });
        let document = vm
            ._context_host
            .borrow()
            .current_main_document_resource_loader()
            .unwrap();
        cancel_send
            .send((vm.page_context_cancel_sender(), document.clone()))
            .unwrap();
        match consumer {
            MaterializedConsumer::SynchronousXhr => {
                let expected = match finish {
                    MaterializedFinish::Complete => "body",
                    MaterializedFinish::Partial => "NetworkError",
                    MaterializedFinish::Cancelled => "aborted",
                };
                let actual = vm.eval("(()=>{const xhr=new XMLHttpRequest();xhr.open('POST','/held',false);try{xhr.send(new Uint8Array([0,128,255,65]));return xhr.readyState===0&&xhr.status===0?'aborted':xhr.responseText}catch(e){return e.name}})()").unwrap();
                assert_eq!(
                    actual, expected,
                    "physical result must preserve synchronous XHR behavior"
                );
            }
            MaterializedConsumer::Popup => {
                assert_eq!(vm.eval("globalThis.nativePopup=open('/held','native-popup');String(nativePopup!==null)").unwrap(), "true");
                tokio::runtime::Handle::current().block_on(async {
                    tokio::time::timeout(Duration::from_secs(5), async {
                        while vm.has_pending_lightweight_popup_document_loads() {
                            if !vm
                                .run_one_oldest_ready_page_task_executor_turn()
                                .await
                                .unwrap()
                            {
                                assert!(vm.wait_for_task_executor_work_arrival().await);
                            }
                        }
                    })
                    .await
                    .unwrap();
                });
                if complete {
                    assert_eq!(
                        vm.eval("nativePopup.document.body.textContent").unwrap(),
                        "body"
                    );
                }
            }
            MaterializedConsumer::Manifest => {
                vm.exec("const link=document.createElement('link');link.rel='manifest';link.href='/held';document.head.appendChild(link)", None).unwrap();
                let crate::RendererAppManifestLoadPreparation::Ready(pending) =
                    vm.prepare_app_manifest_load()
                else {
                    panic!("manifest must admit its original resource load")
                };
                let (result, _publication) = tokio::runtime::Handle::current()
                    .block_on(pending.execute())
                    .into_parts();
                assert_eq!(
                    result.manifest.name.as_deref(),
                    complete.then_some("native manifest")
                );
                // The query result never re-enters the VM: network ownership
                // must remain with the admitted resource, independently of cache publication.
            }
        }
    });
    let mut prefix = Some(prefix);
    let mut tail = Some(tail);
    let mut request_seen = false;
    let mut cancellation = Some(cancel_receive);
    let evidence = async {
        tokio::time::timeout(Duration::from_secs(5), request_arrived).await??;
        request_seen = true;
        let start = tokio::time::timeout(Duration::from_secs(2), async {
            while let Some(occurrence) = events.recv().await {
                if let RendererNetworkOutputItem::Resource(item) = &occurrence.item
                    && let ScriptNetworkOutputItem::SubresourceRequestStarted(start) = item.as_ref()
                    && start.url().as_str() == request_url
                {
                    return Some(start.clone());
                }
            }
            None
        })
        .await
        .context("native request must precede its held response body")?
        .context("native source closed before request admission")?;
        for stage in 0..3 {
            let item = tokio::time::timeout(Duration::from_secs(2), async {
                while let Some(occurrence) = events.recv().await {
                    let RendererNetworkOutputItem::Resource(item) = &occurrence.item else {
                        continue;
                    };
                    let matches = match item.as_ref() {
                        ScriptNetworkOutputItem::SubresourceResponseStarted(head) => {
                            stage == 0 && head.handle() == start.handle()
                        }
                        ScriptNetworkOutputItem::SubresourceDataReceived(data) => {
                            stage == 1 && data.handle() == start.handle()
                        }
                        ScriptNetworkOutputItem::SubresourceBodyFinished(body) => {
                            body.handle() == start.handle()
                        }
                        _ => false,
                    };
                    if matches {
                        return Some(item.clone());
                    }
                }
                None
            })
            .await
            .with_context(|| format!("native stage {stage} must precede the next body gate"))?
            .context("native source closed before terminal")?;
            match (stage, item.as_ref()) {
                (0, ScriptNetworkOutputItem::SubresourceResponseStarted(head)) => {
                    anyhow::ensure!(head.status() == 200);
                    prefix.take().unwrap().send(()).unwrap();
                }
                (1, ScriptNetworkOutputItem::SubresourceDataReceived(data)) => {
                    anyhow::ensure!(data.data_length() == 2);
                    if cancelled {
                        let (page_cancel, document) = cancellation.take().unwrap().await?;
                        match consumer {
                            MaterializedConsumer::SynchronousXhr => page_cancel.cancel(
                                crate::runtime::RendererPageContextCancelReason::PageClosed,
                            ),
                            MaterializedConsumer::Popup => {
                                unreachable!("popup retirement uses its navigation owner")
                            }
                            MaterializedConsumer::Manifest => {
                                assert!(document.begin_detach());
                                document.finish_detach();
                            }
                        }
                    } else {
                        tail.take().unwrap().send(()).unwrap();
                    }
                }
                (2, ScriptNetworkOutputItem::SubresourceBodyFinished(body)) => {
                    match body.result() {
                        SubresourceBodyFinishedResult::Ready(body) if complete => {
                            anyhow::ensure!(body.clone_body_bytes() == payload)
                        }
                        SubresourceBodyFinishedResult::FailedWithPartialBody {
                            partial_body,
                            error_text,
                        } if !complete => {
                            anyhow::ensure!(partial_body.clone_body_bytes() == payload[..2]);
                            anyhow::ensure!(!error_text.is_empty());
                        }
                        other => anyhow::bail!(
                            "original physical result must survive materialization: {other:?}"
                        ),
                    }
                }
                other => anyhow::bail!("native terminal overtook a held stage: {other:?}"),
            }
        }
        Ok(())
    }
    .await;
    if let Some(prefix) = prefix {
        let _ = prefix.send(());
    }
    if let Some(tail) = tail {
        let _ = tail.send(());
    }
    if !request_seen {
        server.abort();
    }
    let server_result = tokio::time::timeout(Duration::from_secs(5), &mut server).await;
    if server_result.is_err() {
        server.abort();
    }
    let vm_result = tokio::time::timeout(Duration::from_secs(5), vm_task).await;
    evidence?;
    server_result??;
    vm_result??;
    Ok(())
}

#[derive(Clone, Copy, Debug)]
enum Resource {
    Fetch,
    Xhr,
    Csp,
}

#[derive(Clone, Copy, Debug)]
enum Finish {
    Complete,
    Partial,
    DocumentOpened,
}

macro_rules! request_stage_tests {
    ($($name:ident: $resource:ident, $auth:literal, $finish:ident;)*) => {
        $(#[tokio::test]
        async fn $name() {
            request_stages(Resource::$resource, $auth, Finish::$finish).await;
        })*
    };
}

request_stage_tests! {
    continued_fetch_publishes_physical_stages: Fetch, false, Complete;
    continued_xhr_publishes_physical_stages: Xhr, false, Complete;
    continued_csp_publishes_physical_stages: Csp, false, Complete;
    authenticated_fetch_publishes_physical_stages: Fetch, true, Complete;
    authenticated_xhr_publishes_physical_stages: Xhr, true, Complete;
    authenticated_csp_publishes_physical_stages: Csp, true, Complete;
    continued_fetch_retains_partial_response: Fetch, false, Partial;
    authenticated_xhr_retains_partial_response: Xhr, true, Partial;
    continued_fetch_retains_source_after_document_open: Fetch, false, DocumentOpened;
    authenticated_fetch_retains_source_after_document_open: Fetch, true, DocumentOpened;
}

async fn request_stages(resource: Resource, authenticate: bool, finish: Finish) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let request_url = Url::parse(&format!("{origin}/probe")).unwrap();
    let (headers, release_headers) = tokio::sync::oneshot::channel();
    let (chunk, release_chunk) = tokio::sync::oneshot::channel();
    let (tail, release_tail) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let mut attempts = 0;
        let mut first_body = None;
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut head = Vec::new();
            while !head.ends_with(b"\r\n\r\n") {
                head.push(stream.read_u8().await.unwrap());
            }
            let head = String::from_utf8(head).unwrap();
            assert!(head.starts_with("POST /probe HTTP/1.1\r\n"));
            let length = head
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse().unwrap())
                })
                .unwrap();
            let mut body = vec![0; length];
            stream.read_exact(&mut body).await.unwrap();
            if let Some(first) = &first_body {
                assert_eq!(&body, first, "authentication must reuse the original body");
            }
            match resource {
                Resource::Csp => assert_eq!(
                    serde_json::from_slice::<serde_json::Value>(&body).unwrap()["csp-report"]["effective-directive"],
                    "connect-src"
                ),
                Resource::Fetch | Resource::Xhr => assert_eq!(body, [0, 128, 255, 65]),
            }
            attempts += 1;
            if authenticate && attempts == 1 {
                first_body = Some(body);
                stream.write_all(b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"stage\"\r\nContent-Length: 4\r\nConnection: close\r\n\r\n").await.unwrap();
                // Retry must become possible at the challenge head and close
                // this transport while its nonempty body remains withheld.
                let mut byte = [0];
                match stream.read(&mut byte).await {
                    Ok(0) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => {}
                    other => panic!("authentication retry must close its challenge: {other:?}"),
                }
                continue;
            }
            if authenticate {
                assert!(head.lines().any(|line| {
                    line.split_once(':').is_some_and(|(name, value)| {
                        name.eq_ignore_ascii_case("authorization")
                            && value.trim() == "Basic dXNlcjpwYXNz"
                    })
                }));
            }
            assert_eq!(attempts, if authenticate { 2 } else { 1 });
            if release_headers.await.is_err() {
                return;
            }
            stream.write_all(b"HTTP/1.1 200 OK\r\nX-Physical: retained\r\nContent-Length: 4\r\nConnection: close\r\n\r\n").await.unwrap();
            if release_chunk.await.is_err() {
                return;
            }
            stream.write_all(b"bo").await.unwrap();
            if release_tail.await.is_err() {
                return;
            }
            if !matches!(finish, Finish::Partial) {
                stream.write_all(b"dy").await.unwrap();
            }
            return;
        }
    });
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    let document_url = Url::parse(&format!("{origin}/page")).unwrap();
    let (mut vm, mut completions) = new_storage_test_vm_with_loader_and_resource_completion_queue(
        document_url.as_str(),
        &loader,
    );
    let (send, mut events) = tokio::sync::mpsc::unbounded_channel();
    vm._context_host
        .borrow()
        .browser_context_runtime()
        .install_network_handler(move |input| {
            if let RendererNetworkInput::Observation(input) = input {
                let _ = send.send(input.occurrence.clone());
            }
        });
    vm.set_fetch_subresource_interception(true, None);
    let keepalive = matches!(finish, Finish::DocumentOpened);
    match resource {
        Resource::Fetch => {
            vm.exec(&format!("fetch('/probe',{{method:'POST',body:new Uint8Array([0,128,255,65]),keepalive:{keepalive}}}).then(r=>r.text()).catch(()=>{{}})"), None).unwrap();
        }
        Resource::Xhr => {
            vm.exec("const xhr=new XMLHttpRequest();xhr.open('POST','/probe');xhr.send(new Uint8Array([0,128,255,65]));", None).unwrap();
        }
        Resource::Csp => {
            let violation = test_window_csp_report_violation(&document_url, &request_url);
            vm.with_default_context_scope_and_checkpoint_for_test(|scope, host_ptr| {
                let host = unsafe { &mut *host_ptr };
                let context = crate::network_host::capture_window_csp_report_request_context(
                    scope, host, crate::native_bridge::OwnerDispatchScope::Top,
                ).unwrap();
                crate::network_host::send_content_security_policy_violation_report_from_window_context(host, &context, &violation);
                Ok(())
            }).unwrap();
        }
    }
    let pending = vm.take_pending_subresource_fetch_infos();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].url, request_url);
    let initial = tokio::time::timeout(std::time::Duration::from_secs(5), events.recv())
        .await
        .unwrap()
        .unwrap();
    let RendererNetworkOutputItem::Resource(item) = &initial.item else {
        panic!("resource start")
    };
    let ScriptNetworkOutputItem::SubresourceRequestStarted(start) = item.as_ref() else {
        panic!("start before transfer")
    };
    let handle = start.handle();
    assert_eq!(start.url(), &request_url);
    vm.continue_pending_subresource_fetch(
        pending[0].internal_id,
        None,
        None,
        None,
        None,
        false,
        authenticate,
    )
    .unwrap();
    if authenticate {
        apply_next_async_subresource_callback_for_test(&mut vm, &mut completions).await;
        let pauses = vm.take_pending_subresource_continue_events();
        let [crate::types::PendingSubresourceContinueEvent::AuthRequired(auth)] = pauses.as_slice()
        else {
            panic!("one auth challenge: {pauses:?}")
        };
        assert_eq!(auth.internal_id, pending[0].internal_id);
        assert!(
            events.try_recv().is_err(),
            "an unresolved auth challenge must not become the final response"
        );
        let _ = vm
            .continue_pending_subresource_auth_body(
                auth.internal_id,
                crate::SubresourceAuthCredentials {
                    target: crate::types::SubresourceAuthTarget::Server,
                    scheme: crate::types::SubresourceAuthScheme::Basic,
                    username: "user".into(),
                    password: "pass".into(),
                },
            )
            .unwrap();
    }
    headers.send(()).unwrap();
    let mut received_bytes = 0;
    for (stage, release) in [(0, Some(chunk)), (1, Some(tail)), (2, None)] {
        let observed = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let observed = if stage == 2 {
                    tokio::select! {
                        item = events.recv() => item.unwrap(),
                        arrived = completions.wait_for_arrival_without_timeout() => {
                            assert!(arrived);
                            while let Some(event) = completions.pop_next_async_subresource_event() {
                                let _ = vm.complete_async_subresource_fetch_event_body(event).unwrap();
                            }
                            continue;
                        }
                    }
                } else {
                    events.recv().await.unwrap()
                };
                assert_eq!(observed.source, initial.source, "request source must survive document replacement");
                if stage == 2
                    && let RendererNetworkOutputItem::Resource(item) = &observed.item
                    && let ScriptNetworkOutputItem::SubresourceDataReceived(data) = item.as_ref()
                {
                    assert_eq!(data.handle(), handle);
                    received_bytes += data.data_length();
                    continue;
                }
                break observed;
            }
        }).await.unwrap_or_else(|_| panic!("{resource:?} auth={authenticate} {finish:?}: native stage {stage} must precede the next transport gate"));
        assert_eq!(
            observed.source, initial.source,
            "request source must survive document replacement"
        );
        let RendererNetworkOutputItem::Resource(item) = &observed.item else {
            panic!("resource phase")
        };
        match (stage, item.as_ref()) {
            (0, ScriptNetworkOutputItem::SubresourceResponseStarted(head)) => {
                assert_eq!(head.handle(), handle);
                assert_eq!(head.status(), 200);
                let request_headers = head
                    .network_request_headers()
                    .expect("original wire headers");
                assert!(
                    request_headers
                        .iter()
                        .any(|(name, _)| name.eq_ignore_ascii_case("host"))
                );
                assert!(
                    !request_headers
                        .iter()
                        .any(|(name, _)| name.eq_ignore_ascii_case("authorization")),
                    "auth rounds retain the original request header block"
                );
                assert!(
                    head.response_headers()
                        .iter()
                        .any(|(name, value)| name.eq_ignore_ascii_case("x-physical")
                            && value == "retained")
                );
            }
            (1, ScriptNetworkOutputItem::SubresourceDataReceived(data)) => {
                assert_eq!(data.handle(), handle);
                assert_eq!(data.data_length(), 2);
                received_bytes += data.data_length();
            }
            (2, ScriptNetworkOutputItem::SubresourceBodyFinished(terminal)) => {
                assert_eq!(terminal.handle(), handle);
                assert_eq!(
                    received_bytes,
                    if matches!(finish, Finish::Partial) {
                        2
                    } else {
                        4
                    }
                );
                match (finish, terminal.result()) {
                    (
                        Finish::Partial,
                        SubresourceBodyFinishedResult::FailedWithPartialBody {
                            partial_body,
                            error_text,
                        },
                    ) => {
                        assert_eq!(partial_body.clone_body_bytes(), b"bo");
                        assert!(!error_text.is_empty());
                    }
                    (
                        Finish::Complete | Finish::DocumentOpened,
                        SubresourceBodyFinishedResult::Ready(body),
                    ) => {
                        assert!(terminal.data_was_streamed());
                        assert_eq!(body.clone_body_bytes(), b"body");
                    }
                    outcome => panic!("retain physical response: {outcome:?}"),
                }
            }
            _ => panic!("unexpected native stage {stage}: {item:?}"),
        }
        if stage == 1 && matches!(finish, Finish::DocumentOpened) {
            vm.eval("document.open();document.write('<p>replacement</p>');document.close();")
                .unwrap();
        }
        if let Some(release) = release {
            release.send(()).unwrap();
        }
    }
    server.await.unwrap();
    while let Some(event) = completions.pop_next_async_subresource_event() {
        let _ = vm
            .complete_async_subresource_fetch_event_body(event)
            .unwrap();
    }
    assert_eq!(
        vm._context_host
            .borrow()
            .pending_subresource_request_count(),
        0,
        "completed responses must release request activity, including after document.open"
    );
    assert!(
        events.try_recv().is_err(),
        "each request must terminate once"
    );
}
