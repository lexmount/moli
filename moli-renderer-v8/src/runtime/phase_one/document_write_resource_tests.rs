use super::*;

use std::time::Instant;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use url::Url;

use crate::page_resource_completion::{
    PageResourceCompletionDocumentEffect, PageResourceCompletionOutputEffect,
    RendererPageResourceCompletion, RendererPageResourceCompletionOwner,
    RendererPageResourceCompletionTestSource,
};
use crate::page_task_queue::RendererPageNetworkingSource;
use crate::types::{
    DocumentWriteExternalScriptFetchTarget, DocumentWriteExternalScriptLoadCompletion,
    DocumentWriteExternalScriptNetworkAttribution, ScriptNetworkOutputItem,
    SubresourceNetworkOutcome,
};

async fn spawn_document_write_script_server(script: &'static str) -> (Url, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("document.write script server should bind");
    let address = listener
        .local_addr()
        .expect("document.write script server should have an address");
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("document.write script request should arrive");
        let mut request = vec![0; 4096];
        let read = stream
            .read(&mut request)
            .await
            .expect("document.write script request should be readable");
        let request = String::from_utf8_lossy(&request[..read]);
        assert!(
            request.starts_with("GET /written.js "),
            "unexpected document.write script request: {request}"
        );
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            script.len(),
            script,
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("document.write script response should be writable");
        stream
            .shutdown()
            .await
            .expect("document.write script response should close");
    });
    (
        Url::parse(&format!("http://{address}/written.js"))
            .expect("document.write script URL should parse"),
        server,
    )
}

async fn spawn_aborted_document_write_script_server() -> (Url, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("aborted document.write script server should bind");
    let address = listener
        .local_addr()
        .expect("aborted document.write script server should have an address");
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("aborted document.write script request should arrive");
        let mut request = vec![0; 4096];
        let read = stream
            .read(&mut request)
            .await
            .expect("aborted document.write script request should be readable");
        let request = String::from_utf8_lossy(&request[..read]);
        assert!(
            request.starts_with("GET /written.js "),
            "unexpected aborted document.write script request: {request}"
        );
        stream
            .shutdown()
            .await
            .expect("aborted document.write script response should close");
    });
    (
        Url::parse(&format!("http://{address}/written.js"))
            .expect("aborted document.write script URL should parse"),
        server,
    )
}

async fn spawn_two_document_write_script_server(
    first_script: &'static str,
    second_script: &'static str,
) -> (
    Url,
    Url,
    tokio::sync::oneshot::Sender<()>,
    JoinHandle<(usize, usize)>,
) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("two-script document.write server should bind");
    let address = listener
        .local_addr()
        .expect("two-script document.write server should have an address");
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let mut first_requests = 0;
        let mut second_requests = 0;
        loop {
            let (mut stream, _) = tokio::select! {
                biased;
                _ = &mut shutdown_rx => break,
                accepted = listener.accept() => {
                    accepted.expect("document.write script request should arrive")
                }
            };
            let mut request = vec![0; 4096];
            let read = stream
                .read(&mut request)
                .await
                .expect("document.write script request should be readable");
            let request = String::from_utf8_lossy(&request[..read]);
            let script = if request.starts_with("GET /first.js ") {
                first_requests += 1;
                first_script
            } else if request.starts_with("GET /second.js ") {
                second_requests += 1;
                second_script
            } else {
                panic!("unexpected sequential document.write script request: {request}");
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                script.len(),
                script,
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("document.write script response should be writable");
            stream
                .shutdown()
                .await
                .expect("document.write script response should close");
        }
        (first_requests, second_requests)
    });
    (
        Url::parse(&format!("http://{address}/first.js"))
            .expect("first document.write script URL should parse"),
        Url::parse(&format!("http://{address}/second.js"))
            .expect("second document.write script URL should parse"),
        shutdown_tx,
        server,
    )
}

struct PendingStandaloneDocumentWritePage {
    runtime: Box<ConcurrentParseTimeRuntime>,
    started: Instant,
    owner_wake_rx: tokio::sync::mpsc::UnboundedReceiver<crate::page_task_queue::RendererOwnerWake>,
}

async fn start_standalone_document_write_page(
    html: String,
    document_url: Url,
) -> PendingStandaloneDocumentWritePage {
    start_standalone_document_write_page_for_page_id(
        PageId::new_for_testing(901),
        html,
        document_url,
    )
    .await
}

async fn start_standalone_document_write_page_for_page_id(
    page_id: PageId,
    html: String,
    document_url: Url,
) -> PendingStandaloneDocumentWritePage {
    let loader_owner =
        ResourceRequestClient::new(&FetchConfig::default()).expect("document.write test loader");
    let local_executor = JsLocalExecutor::new();
    let (owner_wake_tx, owner_wake_rx) = tokio::sync::mpsc::unbounded_channel();
    let owner_wake = crate::page_task_queue::RendererOwnerWakeSender::new(
        owner_wake_tx,
        crate::runtime::RendererPageToken::new_for_testing(page_id),
    );
    let runtime_hooks =
        PageVmRuntimeHooks::standalone_with_owner_wake_without_owner_reservation_for_test(
            owner_wake,
        );
    let env = super::tests::default_test_page_vm_env_config();
    let creation_executor = local_executor.clone();
    let outcome = super::access::run_named_owner_local_task(
        local_executor,
        "owner-attached document.write phase-one bootstrap channel closed",
        async move {
            let outcome = ConcurrentParseTimeRuntime::finish_creation_from_html_bootstrap(
                page_id,
                creation_executor,
                &loader_owner,
                &env,
                runtime_hooks,
                document_url,
                PageVmInitStage::Load,
                html,
                Instant::now(),
            )
            .await?;
            anyhow::Ok((outcome, loader_owner))
        },
    )
    .await
    .expect("document.write phase one should reach its typed-resource boundary");

    let (outcome, loader_owner) = outcome;

    let ParseTimePageVmCreationOutcome::PendingPhaseOne(
        PendingPhaseOneResidence::ClosedInputPageWork {
            mut runtime,
            started,
        },
    ) = outcome
    else {
        panic!("owner-attached document.write must park on its typed Page resource source");
    };
    runtime
        .page_vm
        .retain_standalone_request_client_owner_for_test(loader_owner);
    let target = runtime
        .page_vm
        .vm()
        .current_document_write_external_script_fetch_target()
        .expect("pending document.write load should retain its exact target");
    assert_eq!(
        runtime.page_vm.vm().current_main_document_task_owner(),
        Some(target.task_owner()),
        "producer target must capture the current main Document owner"
    );
    PendingStandaloneDocumentWritePage {
        runtime,
        started,
        owner_wake_rx,
    }
}

async fn wait_for_standalone_page_resource(
    pending: &mut PendingStandaloneDocumentWritePage,
) -> crate::page_task_queue::RendererPageResourceCompletionTestSource {
    let resource_source = pending.runtime.page_vm.page_resource_completion_queue();
    while !resource_source.has_ready_completion() {
        pending
            .owner_wake_rx
            .recv()
            .await
            .expect("document.write producer should retain its owner wake route");
    }
    resource_source
}

async fn resume_standalone_document_write_page(
    pending: PendingStandaloneDocumentWritePage,
) -> (
    ParseTimePageVmCreationOutcome,
    tokio::sync::mpsc::UnboundedReceiver<crate::page_task_queue::RendererOwnerWake>,
) {
    let mut pending = pending;
    let mut resource_source = wait_for_standalone_page_resource(&mut pending).await;
    let PendingStandaloneDocumentWritePage {
        mut runtime,
        started,
        owner_wake_rx,
    } = pending;

    // This fixture deliberately has no RendererOwnerLocalPageSlot. Give its
    // standalone source heads to the real Page scheduler policy, then execute
    // exactly the selected task. The following phase-one continuation may
    // observe the result, but it never receives dequeue authority itself.
    let loader = runtime.loader.clone();
    let turn_executor = runtime.page_vm.local_executor.clone();
    runtime = super::access::run_named_owner_local_task(
        turn_executor,
        "standalone document.write Page turn channel closed",
        async move {
            let ready = resource_source
                .next_ready_metadata()
                .expect("standalone resource source must retain its ready head");
            let owner = resource_source
                .next_ready_owner()
                .map(crate::page_task_queue::RendererPageNetworkingOwner::ResourceCompletion)
                .expect("standalone resource source must retain its exact owner");
            let mut descriptors = vec![
                crate::page_task_queue::RendererPageReadyDescriptor::Networking { ready, owner },
            ];
            if let Some(timer) = runtime.page_vm.due_page_timer_ready_descriptor() {
                descriptors.push(timer);
            }
            let selected = crate::runtime::page_turn_scheduler::PageTurnScheduler::new(())
                .select_ready_descriptor(descriptors)
                .expect("standalone Page fixture must expose one runnable source");
            match selected {
                crate::page_task_queue::RendererPageReadyDescriptor::Networking { .. } => {
                    runtime
                        .page_vm
                        .apply_one_page_resource_terminal_owner_admission_for_test(
                            &mut resource_source,
                        )?
                        .expect("selected standalone resource head must be executable");
                }
                crate::page_task_queue::RendererPageReadyDescriptor::Timer { deadline } => {
                    runtime
                        .page_vm
                        .apply_selected_page_scheduler_task(
                            crate::page_task_queue::RendererPageSchedulerTask::Timer { deadline },
                            &loader,
                        )
                        .await?;
                }
                other => unreachable!(
                    "standalone document.write fixture exposed unexpected source: {other:?}"
                ),
            }
            Ok(runtime)
        },
    )
    .await
    .expect("standalone document.write fixture should execute one bounded Page turn");

    let continuation_executor = runtime.page_vm.local_executor.clone();
    let outcome = super::access::run_named_owner_local_task(
        continuation_executor,
        "owner-attached document.write phase-one continuation channel closed",
        async move {
            (*runtime)
                .continue_creation_from_phase_one_runtime(started)
                .await
        },
    )
    .await
    .expect("typed document.write completion should resume phase one");
    (outcome, owner_wake_rx)
}

async fn resume_standalone_main_parser_continuation_if_ready(
    outcome: ParseTimePageVmCreationOutcome,
    owner_wake_rx: tokio::sync::mpsc::UnboundedReceiver<crate::page_task_queue::RendererOwnerWake>,
) -> (
    ParseTimePageVmCreationOutcome,
    tokio::sync::mpsc::UnboundedReceiver<crate::page_task_queue::RendererOwnerWake>,
) {
    let residence = match outcome {
        ParseTimePageVmCreationOutcome::PendingPhaseOne(residence) => residence,
        outcome => return (outcome, owner_wake_rx),
    };
    if !residence
        .page_vm()
        .page_task_executor_sources_for_test()
        .has_scheduler_task_for_executor_test(|descriptor| {
            matches!(
                descriptor,
                crate::page_task_queue::RendererPageReadyDescriptor::Networking {
                    owner:
                        crate::page_task_queue::RendererPageNetworkingOwner::MainParserContinuation(
                            _
                        ),
                    ..
                }
            )
        })
    {
        return (
            ParseTimePageVmCreationOutcome::PendingPhaseOne(residence),
            owner_wake_rx,
        );
    };
    let PendingPhaseOneResidence::ClosedInputPageWork {
        mut runtime,
        started,
    } = residence
    else {
        panic!("closed document.write fixture should retain a blocked parser runtime");
    };

    let executor = runtime.page_vm.local_executor.clone();
    runtime = super::access::run_named_owner_local_task(
        executor,
        "standalone main-parser continuation Page turn channel closed",
        async move {
            let request_client = runtime
                .page_vm
                .main_document_resource_loader()
                .request_client()
                .clone();
            assert!(
                runtime
                    .page_vm
                    .run_exact_selected_page_task_for_test(
                        crate::runtime::page_vm::PageSelectedTaskTestSelector::MainParserContinuation,
                        &request_client,
                    )
                    .await?,
                "fixture should execute its exact parser continuation through the production dispatcher"
            );
            Ok(runtime)
        },
    )
    .await
    .expect("standalone parser continuation should execute one bounded Page turn");

    let executor = runtime.page_vm.local_executor.clone();
    let outcome = super::access::run_named_owner_local_task(
        executor,
        "standalone admitted parser continuation channel closed",
        async move {
            (*runtime)
                .continue_creation_from_phase_one_runtime(started)
                .await
        },
    )
    .await
    .expect("admitted parser continuation should resume phase one");
    (outcome, owner_wake_rx)
}

async fn finish_standalone_document_write_page(html: String, document_url: Url) -> PageVm {
    let (outcome, owner_wake_rx) = resume_standalone_document_write_page(
        start_standalone_document_write_page(html, document_url).await,
    )
    .await;
    let (outcome, _) =
        resume_standalone_main_parser_continuation_if_ready(outcome, owner_wake_rx).await;
    let ParseTimePageVmCreationOutcome::ContinuePhaseTwo { page_vm, .. } = outcome else {
        panic!(
            "one document.write terminal and its continuation should finish this full-body fixture"
        );
    };
    page_vm
}

async fn evaluate_on_owner_local_task(
    mut page_vm: PageVm,
    expression: &'static str,
) -> serde_json::Value {
    let local_executor = page_vm.local_executor.clone();
    super::access::run_named_owner_local_task(
        local_executor,
        "document.write phase-one result evaluation channel closed",
        async move { page_vm.evaluate_expression(expression) },
    )
    .await
    .expect("document.write result should evaluate")
}

async fn evaluate_pending_on_owner_local_task(
    mut pending: PendingStandaloneDocumentWritePage,
    expression: &'static str,
) -> (PendingStandaloneDocumentWritePage, serde_json::Value) {
    let local_executor = pending.runtime.page_vm.local_executor.clone();
    super::access::run_named_owner_local_task(
        local_executor,
        "pending document.write result evaluation channel closed",
        async move {
            let result = pending.runtime.page_vm.evaluate_expression(expression)?;
            Ok((pending, result))
        },
    )
    .await
    .expect("pending document.write result should evaluate")
}

#[test]
fn standalone_document_write_external_script_uses_typed_phase_one_route() {
    super::tests::run_phase_one_large_stack_test("typed-document-write-phase-one", || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");
        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let (script_url, server) = spawn_document_write_script_server(
                "globalThis.__documentWriteEvents.push('external');",
            )
            .await;
            let document_url = script_url.join("page.html").expect("document URL");
            let html = format!(
                r#"<!doctype html><html><body><script>
globalThis.__documentWriteEvents = ['inline-before'];
document.write(`<script src="{script_url}" onload="globalThis.__documentWriteEvents.push('load')"><\/script><span id="written-tail">written</span>`);
globalThis.__documentWriteEvents.push('inline-after');
</script><p id="parser-tail">parser</p></body></html>"#,
            );
            let page_vm = finish_standalone_document_write_page(html, document_url).await;
            let result = evaluate_on_owner_local_task(
                page_vm,
                r#"JSON.stringify({
  events: globalThis.__documentWriteEvents,
  writtenTail: !!document.getElementById('written-tail'),
  parserTail: !!document.getElementById('parser-tail')
})"#,
            )
            .await;
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(
                    r#"{"events":["inline-before","inline-after","external","load"],"writtenTail":true,"parserTail":true}"#,
                ),
                "one typed terminal must execute the external script and synchronously resume its parser insertion"
            );
            server
                .await
                .expect("document.write script server should finish");
        }));
    });
}

#[test]
fn document_write_external_script_live_collection_walk_uses_incremental_parser_frontier() {
    super::tests::run_phase_one_large_stack_test("document-write-live-collection", || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");
        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let (script_url, server) = spawn_document_write_script_server(
                r#"
const nodes = document.getElementsByTagName("*");
const ids = [];
for (let index = 0; index < nodes.length; index++) {
  if (nodes[index].id) ids.push(nodes[index].id);
}
globalThis.__documentWriteVisibleIds = ids.join("|");
"#,
            )
            .await;
            let document_url = script_url.join("page.html").expect("document URL");
            let before = (0..256)
                .map(|index| format!(r#"<div id="before-{index}"><span></span></div>"#))
                .collect::<String>();
            let future = (0..256)
                .map(|index| format!(r#"<article id="future-{index}"><i></i></article>"#))
                .collect::<String>();
            let html = format!(
                r#"<!doctype html><html><body>{before}<script>document.write(`<script src="{script_url}"><\/script>`);</script>{future}</body></html>"#,
            );

            let page_vm = finish_standalone_document_write_page(html, document_url).await;
            let result = evaluate_on_owner_local_task(
                page_vm,
                r#"JSON.stringify({
  sawFirst: __documentWriteVisibleIds.includes("before-0"),
  sawLast: __documentWriteVisibleIds.includes("before-255"),
  hidFuture: !__documentWriteVisibleIds.includes("future-0"),
  parserResumed: !!document.getElementById("future-255")
})"#,
            )
            .await;
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(
                    r#"{"sawFirst":true,"sawLast":true,"hidFuture":true,"parserResumed":true}"#
                ),
                "a repeated live-collection indexed walk must observe only parser-committed DOM and let parsing resume"
            );
            server
                .await
                .expect("document.write script server should finish");
        }));
    });
}

#[test]
fn document_write_external_runaway_script_is_terminated_and_parser_recovers() {
    super::tests::run_phase_one_large_stack_test("document-write-script-watchdog", || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");
        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let (script_url, server) =
                spawn_document_write_script_server("for (;;) {}").await;
            let document_url = script_url.join("page.html").expect("document URL");
            let html = format!(
                r#"<!doctype html><script>document.write(`<script src="{script_url}"><\/script>`);</script><main id="parser-recovered">ready</main>"#,
            );

            let started = Instant::now();
            let page_vm = finish_standalone_document_write_page(html, document_url).await;
            assert!(
                started.elapsed() < std::time::Duration::from_secs(4),
                "the document.write script watchdog should interrupt the runaway turn promptly"
            );
            let result = evaluate_on_owner_local_task(
                page_vm,
                "String(!!document.getElementById('parser-recovered'))",
            )
            .await;
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some("true"),
                "the parser and isolate must remain usable after terminating the written script"
            );
            server
                .await
                .expect("document.write script server should finish");
        }));
    });
}

#[test]
fn resident_queue_phase_one_continuation_uses_the_exact_owner_arbiter() {
    super::tests::run_phase_one_large_stack_test("document-write-resident-queue", || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");
        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let (script_url, server) = spawn_document_write_script_server(
                "globalThis.__residentQueueScriptRan = true;",
            )
            .await;
            let document_url = script_url.join("page.html").expect("document URL");
            let html = format!(
                r#"<!doctype html><script>document.write(`<script src="{script_url}"><\/script><main id="resident-tail">tail</main>`);</script>"#,
            );
            let mut pending = start_standalone_document_write_page(html, document_url).await;
            let resident_queue = wait_for_standalone_page_resource(&mut pending).await;
            let (outcome, owner_wake_rx) =
                resume_standalone_document_write_page(pending).await;
            assert!(
                !resident_queue.has_ready_completion(),
                "one resident typed terminal should be consumed exactly once"
            );
            let (outcome, _) =
                resume_standalone_main_parser_continuation_if_ready(outcome, owner_wake_rx).await;
            let ParseTimePageVmCreationOutcome::ContinuePhaseTwo { page_vm, .. } = outcome else {
                panic!("the resident terminal's parser continuation should finish this fixture");
            };
            let result = evaluate_on_owner_local_task(
                page_vm,
                r#"JSON.stringify({
  scriptRan: !!globalThis.__residentQueueScriptRan,
  tail: !!document.getElementById('resident-tail')
})"#,
            )
            .await;
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(r#"{"scriptRan":true,"tail":true}"#)
            );
            server
                .await
                .expect("resident queue script server should finish");
        }));
    });
}

#[test]
fn older_due_timer_runs_before_ready_document_write_terminal() {
    super::tests::run_phase_one_large_stack_test("document-write-timer-arbitration", || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");
        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let (script_url, server) = spawn_document_write_script_server(
                "globalThis.__documentWriteEvents.push('external');",
            )
            .await;
            let document_url = script_url.join("page.html").expect("document URL");
            let html = format!(
                r#"<!doctype html><script>
globalThis.__documentWriteEvents = ['inline-before'];
setTimeout(() => globalThis.__documentWriteEvents.push('timer'), 0);
document.write(`<script src="{script_url}" onload="globalThis.__documentWriteEvents.push('load')"><\/script>`);
globalThis.__documentWriteEvents.push('inline-after');
</script>"#,
            );
            let first_pending =
                start_standalone_document_write_page(html, document_url).await;
            let (first_outcome, owner_wake_rx) =
                resume_standalone_document_write_page(first_pending).await;
            let ParseTimePageVmCreationOutcome::PendingPhaseOne(
                PendingPhaseOneResidence::ClosedInputPageWork { runtime, started },
            ) = first_outcome
            else {
                panic!("the older timer must consume its own turn before the typed terminal");
            };
            let pending = PendingStandaloneDocumentWritePage {
                runtime,
                started,
                owner_wake_rx,
            };
            let (pending, result) = evaluate_pending_on_owner_local_task(
                pending,
                "JSON.stringify(globalThis.__documentWriteEvents)",
            )
            .await;
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(r#"["inline-before","inline-after","timer"]"#),
                "the first continuation must run only the older due timer"
            );

            let (second_outcome, owner_wake_rx) =
                resume_standalone_document_write_page(pending).await;
            let (third_outcome, _) =
                resume_standalone_main_parser_continuation_if_ready(
                    second_outcome,
                    owner_wake_rx,
                )
                .await;
            let ParseTimePageVmCreationOutcome::ContinuePhaseTwo { page_vm, .. } = third_outcome
            else {
                panic!(
                    "the parser continuation after the retained terminal should finish phase one"
                );
            };
            let result = evaluate_on_owner_local_task(
                page_vm,
                "JSON.stringify(globalThis.__documentWriteEvents)",
            )
            .await;
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(r#"["inline-before","inline-after","timer","external","load"]"#),
                "transitional arbitration must preserve timer/resource order without draining both"
            );
            server
                .await
                .expect("timer arbitration script server should finish");
        }));
    });
}

#[test]
fn typed_document_write_fetch_failure_dispatches_error_and_resumes_parser() {
    super::tests::run_phase_one_large_stack_test("typed-document-write-fetch-failure", || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");
        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let (script_url, server) = spawn_aborted_document_write_script_server().await;
            let document_url = script_url.join("page.html").expect("document URL");
            let html = format!(
                r#"<!doctype html><html><body><script>
globalThis.__documentWriteEvents = ['inline-before'];
document.write(`<script src="{script_url}" onerror="globalThis.__documentWriteEvents.push('error')"><\/script><span id="written-tail">written</span>`);
globalThis.__documentWriteEvents.push('inline-after');
</script><p id="parser-tail">parser</p></body></html>"#,
            );
            let page_vm = finish_standalone_document_write_page(html, document_url).await;
            let result = evaluate_on_owner_local_task(
                page_vm,
                r#"JSON.stringify({
  events: globalThis.__documentWriteEvents,
  writtenTail: !!document.getElementById('written-tail'),
  parserTail: !!document.getElementById('parser-tail')
})"#,
            )
            .await;
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(
                    r#"{"events":["inline-before","inline-after","error"],"writtenTail":true,"parserTail":true}"#,
                ),
                "a failed typed fetch must dispatch error and resume the suspended parser insertion"
            );
            server
                .await
                .expect("aborted document.write script server should finish");
        }));
    });
}

#[test]
fn document_open_before_terminal_discards_document_write_effect_but_preserves_network() {
    super::tests::run_phase_one_large_stack_test("document-write-stale-before-terminal", || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");
        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let (script_url, server) = spawn_document_write_script_server(
                "globalThis.__retiredDocumentWriteScriptRan = true;",
            )
            .await;
            let document_url = script_url.join("page.html").expect("document URL");
            let html = format!(
                r#"<!doctype html><html><body><script>
document.write(`<script src="{script_url}"><\/script><span id="retired-written-tail">retired</span>`);
</script><p id="retired-parser-tail">retired parser</p></body></html>"#,
            );
            let mut pending =
                start_standalone_document_write_page(html, document_url).await;
            let root_document = pending.runtime.page_vm.document_lifecycle.identity().document;
            let retired_target = pending
                .runtime
                .page_vm
                .vm()
                .current_document_write_external_script_fetch_target()
                .expect("retired document.write target should exist before document.open");

            let executor = pending.runtime.page_vm.local_executor.clone();
            pending = super::access::run_named_owner_local_task(
                executor,
                "document.write stale-target replacement channel closed",
                async move {
                    pending.runtime.page_vm.vm_mut().eval(
                        r#"
document.open();
document.write('<main id="replacement">replacement</main>');
document.close();
"#,
                    )?;
                    Ok(pending)
                },
            )
            .await
            .expect("document.open should replace the pending document.write owner");
            assert_ne!(
                pending
                    .runtime
                    .page_vm
                    .vm()
                    .current_main_document_task_owner(),
                Some(retired_target.task_owner()),
                "document.open must install a new main Document owner"
            );
            assert_eq!(
                pending
                    .runtime
                    .page_vm
                    .vm()
                    .current_document_write_external_script_fetch_target(),
                None,
                "replacement must retire the old pending script target"
            );
            let page_resource_queue =
                pending.runtime.page_vm.page_resource_completion_queue();
            while !page_resource_queue.has_ready_completion() {
                pending
                    .owner_wake_rx
                    .recv()
                    .await
                    .expect("the retired request should retain its owner wake route");
            }

            let activity_epoch_before = pending
                .runtime
                .page_vm
                .vm()
                .subresource_activity_epoch();
            let executor = pending.runtime.page_vm.local_executor.clone();
            let (mut pending, outcome) = super::access::run_named_owner_local_task(
                executor,
                "document.write stale terminal owner turn channel closed",
                async move {
                    let (_, completion) = page_resource_queue
                        .pop_front()
                        .expect("the retired terminal should remain queued for its lane executor");
                    let outcome = pending
                        .runtime
                        .page_vm
                        .apply_selected_page_resource_completion_turn(completion)?;
                    Ok((pending, outcome))
                },
            )
            .await
            .expect("stale document.write terminal arbitration should succeed");
            assert_eq!(
                outcome.action.owner,
                RendererPageResourceCompletionOwner::document_write_external_script(
                    root_document,
                    retired_target,
                )
            );
            assert_eq!(
                outcome.action.document_effect,
                PageResourceCompletionDocumentEffect::DiscardedStaleOwner {
                    current_owner: None,
                }
            );
            assert_eq!(
                outcome.action.output_effect,
                PageResourceCompletionOutputEffect::CaptureRequired,
                "the completed retired request remains historical Network output"
            );

            assert_eq!(
                pending
                    .runtime
                    .page_vm
                    .vm()
                    .subresource_activity_epoch(),
                activity_epoch_before,
                "historical Network output must not become replacement Document activity"
            );
            let network_output = pending.runtime.page_vm.vm_mut().take_network_output();
            assert!(network_output.into_items().into_iter().any(|item| {
                matches!(
                    item,
                    ScriptNetworkOutputItem::SubresourceNetworkRecord(record)
                        if record.url() == &script_url
                )
            }));

            let ConcurrentParseTimeRuntime { page_vm, .. } = *pending.runtime;
            let result = evaluate_on_owner_local_task(
                page_vm,
                r#"JSON.stringify({
  replacement: !!document.getElementById('replacement'),
  retiredScriptRan: !!globalThis.__retiredDocumentWriteScriptRan,
  writtenTail: !!document.getElementById('retired-written-tail'),
  parserTail: !!document.getElementById('retired-parser-tail')
})"#,
            )
            .await;
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(
                    r#"{"replacement":true,"retiredScriptRan":false,"writtenTail":false,"parserTail":false}"#,
                )
            );
            server
                .await
                .expect("retired document.write script server should finish");
        }));
    });
}

#[test]
fn load_id_mismatch_is_rejected_by_the_page_arbiter() {
    super::tests::run_phase_one_large_stack_test("document-write-stale-load-id", || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");
        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let (script_url, server) = spawn_document_write_script_server(
                "globalThis.__currentDocumentWriteScriptRan = true;",
            )
            .await;
            let document_url = script_url.join("page.html").expect("document URL");
            let html = format!(
                r#"<!doctype html><script>document.write(`<script src="{script_url}"><\/script>`);</script>"#,
            );
            let mut pending =
                start_standalone_document_write_page(html, document_url.clone()).await;
            let root_document = pending.runtime.page_vm.document_lifecycle.identity().document;
            let current_target = pending
                .runtime
                .page_vm
                .vm()
                .current_document_write_external_script_fetch_target()
                .expect("current pending load should expose its exact target");
            let stale_target = DocumentWriteExternalScriptFetchTarget::new(
                current_target.task_owner(),
                current_target
                    .load_id()
                    .checked_add(1)
                    .expect("test load id should have a successor"),
            );
            let stale_completion = DocumentWriteExternalScriptLoadCompletion::new(
                stale_target,
                Ok("globalThis.__staleLoadIdScriptRan = true;".to_owned()),
                None,
                DocumentWriteExternalScriptNetworkAttribution::new(
                    document_url,
                    script_url.clone(),
                ),
            );
            let mut stale_queue = RendererPageNetworkingSource::new_for_test();
            stale_queue
                .sender()
                .send(RendererPageResourceCompletion::document_write_external_script(
                    root_document,
                    stale_completion,
                ))
                .expect("stale load-id terminal should enter the test queue");

            let executor = pending.runtime.page_vm.local_executor.clone();
            let (pending, outcome) = super::access::run_named_owner_local_task(
                executor,
                "document.write stale-load-id arbitration channel closed",
                async move {
                    let outcome = pending
                        .runtime
                        .page_vm
                        .apply_one_page_resource_terminal_owner_admission_for_test(&mut stale_queue)?
                        .expect("stale load-id terminal should consume one bounded turn");
                    Ok((pending, outcome))
                },
            )
            .await
            .expect("stale load-id arbitration should succeed");
            assert_eq!(
                outcome.action.document_effect,
                PageResourceCompletionDocumentEffect::DiscardedStaleOwner {
                    current_owner: None,
                },
                "an exact-target mismatch must be rejected before projecting a current owner"
            );
            assert_eq!(
                outcome.action.output_effect,
                PageResourceCompletionOutputEffect::None
            );
            assert_eq!(
                pending
                    .runtime
                    .page_vm
                    .vm()
                    .current_document_write_external_script_fetch_target(),
                Some(current_target),
                "a mismatched load id must not consume the current pending load"
            );
            let (pending, result) = evaluate_pending_on_owner_local_task(
                pending,
                "String(globalThis.__staleLoadIdScriptRan)",
            )
            .await;
            assert_eq!(result.get("value").and_then(serde_json::Value::as_str), Some("undefined"));
            // Keep the pending page, and therefore its request client, alive until the server has
            // accepted and served the current script load. Dropping it before this await can
            // cancel the connect while the server is still blocked in `accept()`.
            server
                .await
                .expect("current document.write script server should finish");
            drop(pending);
        }));
    });
}

#[test]
fn root_document_namespace_rejects_cross_page_target_collision() {
    super::tests::run_phase_one_large_stack_test("document-write-root-document-namespace", || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");
        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
                let _js_runtime = crate::JsRuntime::initialize();
                let (retired_script_url, retired_server) = spawn_document_write_script_server(
                    "globalThis.__retiredNamespaceScriptRan = true;",
                )
                .await;
                let (current_script_url, current_server) = spawn_document_write_script_server(
                    "globalThis.__currentNamespaceScriptRan = true;",
                )
                .await;
                let retired_html = format!(
                    r#"<!doctype html><script>document.write(`<script src="{retired_script_url}"><\/script>`);</script>"#,
                );
                let current_html = format!(
                    r#"<!doctype html><script>document.write(`<script src="{current_script_url}"><\/script>`);</script>"#,
                );
                let mut retired = start_standalone_document_write_page(
                    retired_html,
                    retired_script_url.join("page.html").expect("retired page URL"),
                )
                .await;
                let mut current = start_standalone_document_write_page_for_page_id(
                    PageId::new_for_testing(902),
                    current_html,
                    current_script_url.join("page.html").expect("current page URL"),
                )
                .await;
                let retired_root = retired
                    .runtime
                    .page_vm
                    .document_lifecycle
                    .identity()
                    .document;
                let current_root = current
                    .runtime
                    .page_vm
                    .document_lifecycle
                    .identity()
                    .document;
                let retired_target = retired
                    .runtime
                    .page_vm
                    .vm()
                    .current_document_write_external_script_fetch_target()
                    .expect("retired PageVm should expose its pending exact target");
                let current_target = current
                    .runtime
                    .page_vm
                    .vm()
                    .current_document_write_external_script_fetch_target()
                    .expect("current PageVm should expose its pending exact target");
                assert_eq!(
                    retired_target, current_target,
                    "fresh PageVms naturally reuse their PageVm-local owner/load namespace"
                );
                assert_ne!(
                    retired_root, current_root,
                    "root Document tokens must distinguish the colliding local targets"
                );
                let mut retired_queue = wait_for_standalone_page_resource(&mut retired).await;
                let _current_queue = wait_for_standalone_page_resource(&mut current).await;
                let activity_epoch_before = current
                    .runtime
                    .page_vm
                    .vm()
                    .subresource_activity_epoch();

                let executor = current.runtime.page_vm.local_executor.clone();
                let (mut current, outcome) = super::access::run_named_owner_local_task(
                    executor,
                    "document.write root namespace arbitration channel closed",
                    async move {
                        let outcome = current
                            .runtime
                            .page_vm
                            .apply_one_page_resource_terminal_owner_admission_for_test(&mut retired_queue)?
                            .expect("the retired terminal should consume one bounded turn");
                        Ok((current, outcome))
                    },
                )
                .await
                .expect("root namespace arbitration should succeed");
                assert_eq!(
                    outcome.action.owner,
                    RendererPageResourceCompletionOwner::document_write_external_script(
                        retired_root,
                        retired_target,
                    )
                );
                assert_eq!(
                    outcome.action.document_effect,
                    PageResourceCompletionDocumentEffect::DiscardedStaleOwner {
                        current_owner: Some(
                            RendererPageResourceCompletionOwner::document_write_external_script(
                                current_root,
                                current_target,
                            ),
                        ),
                    }
                );
                assert_eq!(
                    outcome.action.output_effect,
                    PageResourceCompletionOutputEffect::CaptureRequired
                );

                assert_eq!(
                    current
                        .runtime
                        .page_vm
                        .vm()
                        .subresource_activity_epoch(),
                    activity_epoch_before,
                    "retired Network output must not become current PageVm activity"
                );
                assert_eq!(
                    current
                        .runtime
                        .page_vm
                        .vm()
                        .current_document_write_external_script_fetch_target(),
                    Some(current_target),
                    "a colliding retired terminal must leave the current pending load untouched"
                );
                let network_output = current.runtime.page_vm.vm_mut().take_network_output();
                assert!(network_output.into_items().into_iter().any(|item| {
                    matches!(
                        item,
                        ScriptNetworkOutputItem::SubresourceNetworkRecord(record)
                            if record.url() == &retired_script_url
                    )
                }));

                let ConcurrentParseTimeRuntime { page_vm, .. } = *current.runtime;
                let result = evaluate_on_owner_local_task(
                    page_vm,
                    "String(globalThis.__retiredNamespaceScriptRan)",
                )
                .await;
                assert_eq!(
                    result.get("value").and_then(serde_json::Value::as_str),
                    Some("undefined"),
                    "retired source must not execute in the colliding current PageVm"
                );
                retired_server
                    .await
                    .expect("retired namespace script server should finish");
                current_server
                    .await
                    .expect("current namespace script server should finish");
            }));
    });
}

#[test]
fn sequential_document_write_scripts_require_distinct_fifo_owner_turns() {
    super::tests::run_phase_one_large_stack_test("document-write-sequential-fifo", || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");
        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let (first_url, second_url, shutdown_server, server) =
                spawn_two_document_write_script_server(
                    "globalThis.__documentWriteEvents.push('first');",
                    "globalThis.__documentWriteEvents.push('second');",
                )
                .await;
            let document_url = first_url.join("page.html").expect("document URL");
            let html = format!(
                r#"<!doctype html><html><body><script>
globalThis.__documentWriteEvents = ['inline-before'];
document.write(`<script src="{first_url}" onload="globalThis.__documentWriteEvents.push('first-load')" onerror="globalThis.__documentWriteEvents.push('first-error')"><\/script><script src="{second_url}" onload="globalThis.__documentWriteEvents.push('second-load')" onerror="globalThis.__documentWriteEvents.push('second-error')"><\/script><span id="written-tail">written</span>`);
globalThis.__documentWriteEvents.push('inline-after');
</script><p id="parser-tail">parser</p></body></html>"#,
            );
            let first_pending =
                start_standalone_document_write_page(html, document_url).await;
            let first_target = first_pending
                .runtime
                .page_vm
                .vm()
                .current_document_write_external_script_fetch_target()
                .expect("first sequential script should expose its exact target");
            let (first_outcome, owner_wake_rx) =
                resume_standalone_document_write_page(first_pending).await;
            let (first_outcome, owner_wake_rx) =
                resume_standalone_main_parser_continuation_if_ready(
                    first_outcome,
                    owner_wake_rx,
                )
                .await;
            let ParseTimePageVmCreationOutcome::PendingPhaseOne(
                PendingPhaseOneResidence::ClosedInputPageWork { runtime, started },
            ) = first_outcome
            else {
                panic!("one owner turn must not consume both sequential script terminals");
            };
            let second_pending = PendingStandaloneDocumentWritePage {
                runtime,
                started,
                owner_wake_rx,
            };
            let second_target = second_pending
                .runtime
                .page_vm
                .vm()
                .current_document_write_external_script_fetch_target()
                .expect("second sequential script should install its own exact target");
            assert_eq!(
                second_target.task_owner(),
                first_target.task_owner(),
                "both sequential loads belong to the same exact Document"
            );
            assert!(
                second_target.load_id() > first_target.load_id(),
                "sequential loads need distinct monotonic request identities"
            );

            let (second_outcome, owner_wake_rx) =
                resume_standalone_document_write_page(second_pending).await;
            let (second_outcome, _) =
                resume_standalone_main_parser_continuation_if_ready(
                    second_outcome,
                    owner_wake_rx,
                )
                .await;
            let ParseTimePageVmCreationOutcome::ContinuePhaseTwo { mut page_vm, .. } =
                second_outcome
            else {
                panic!("the second script continuation should finish this two-script fixture");
            };
            shutdown_server
                .send(())
                .expect("completed two-script fixture should stop its server");
            let (first_requests, second_requests) = server
                .await
                .expect("two-script document.write server should finish");
            assert_eq!(first_requests, 1, "the first blocking script loads once");
            assert!(
                second_requests >= 1,
                "the second blocking script must perform or join a real fetch"
            );
            let network_output = page_vm.vm_mut().take_network_output();
            let network_records = network_output
                .into_items()
                .filter_map(|item| match item {
                    ScriptNetworkOutputItem::SubresourceNetworkRecord(record) => Some(record),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(network_records.len(), 2);
            assert_eq!(network_records[0].url(), &first_url);
            assert_eq!(network_records[1].url(), &second_url);
            assert!(matches!(
                network_records[0].outcome(),
                SubresourceNetworkOutcome::Success { .. }
            ));
            assert!(matches!(
                network_records[1].outcome(),
                SubresourceNetworkOutcome::Success { .. }
            ));
            let result = evaluate_on_owner_local_task(
                page_vm,
                r#"JSON.stringify({
  events: globalThis.__documentWriteEvents,
  writtenTail: !!document.getElementById('written-tail'),
  parserTail: !!document.getElementById('parser-tail')
})"#,
            )
            .await;
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(
                    r#"{"events":["inline-before","inline-after","first","first-load","second","second-load"],"writtenTail":true,"parserTail":true}"#,
                ),
                "separate owner turns must preserve parser insertion and script-event FIFO order"
            );
        }));
    });
}

#[test]
fn document_write_load_handler_replacement_cannot_resume_retired_parser_insertion() {
    super::tests::run_phase_one_large_stack_test("document-write-replacement-reentry", || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");
        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let (script_url, server) =
                spawn_document_write_script_server("globalThis.__documentWriteExternalRan = true;")
                    .await;
            let document_url = script_url.join("page.html").expect("document URL");
            let html = format!(
                r#"<!doctype html><html><body><script>
document.write(`<script src="{script_url}" onload="document.open(); document.write('<main id=&quot;replacement&quot;>replacement</main>'); document.close();"><\/script><span id="retired-written-tail">retired</span>`);
</script><p id="retired-parser-tail">retired parser</p></body></html>"#,
            );
            let page_vm = finish_standalone_document_write_page(html, document_url).await;
            let result = evaluate_on_owner_local_task(
                page_vm,
                r#"JSON.stringify({
  replacement: !!document.getElementById('replacement'),
  writtenTail: !!document.getElementById('retired-written-tail'),
  parserTail: !!document.getElementById('retired-parser-tail')
})"#,
            )
            .await;
            assert_eq!(
                result.get("value").and_then(serde_json::Value::as_str),
                Some(r#"{"replacement":true,"writtenTail":false,"parserTail":false}"#),
                "load-handler document.open must retire the old insertion before it can resume"
            );
            server
                .await
                .expect("document.write replacement script server should finish");
        }));
    });
}
