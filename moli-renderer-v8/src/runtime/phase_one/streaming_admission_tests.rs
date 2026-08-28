use super::*;
use crate::page_task_queue::{RendererOwnerWake, RendererOwnerWakeSource};
use crate::runtime::page_vm::{
    DocumentLifecycleTurnAction, DocumentLifecycleTurnOutcome, DocumentLifecycleTurnReadiness,
};
use moli_dom::native::Node;
use tokio::sync::{mpsc, oneshot};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AppliedStreamingNetworkingTask {
    /// The exact parser-resume carrier ran through the production selected
    /// dispatcher. Phase-one resume consumes its admission fact.
    MainParserContinuation,
    /// The head was an exact stylesheet terminal executed through the
    /// production selected-task dispatcher. Its action stays opaque so the
    /// fixture cannot reproduce or bypass task completion.
    StylesheetCompletion,
}

fn has_ready_main_parser_continuation(residence: &PendingPhaseOneResidence) -> bool {
    residence
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
}

fn has_ready_stylesheet_completion(residence: &PendingPhaseOneResidence) -> bool {
    residence
        .page_vm()
        .page_task_executor_sources_for_test()
        .has_scheduler_task_for_executor_test(|descriptor| {
            matches!(
                descriptor,
                crate::page_task_queue::RendererPageReadyDescriptor::Networking {
                    owner:
                        crate::page_task_queue::RendererPageNetworkingOwner::StylesheetCompletion(
                            _
                        ),
                    ..
                }
            )
        })
}

fn has_ready_connected_style_event(residence: &PendingPhaseOneResidence) -> bool {
    residence
        .page_vm()
        .page_task_executor_sources_for_test()
        .has_scheduler_task_for_executor_test(|descriptor| {
            matches!(
                descriptor,
                crate::page_task_queue::RendererPageReadyDescriptor::DomManipulation {
                    owner:
                        crate::page_task_queue::RendererPageDomManipulationOwner::ConnectedStyleEvent(
                            _
                        ),
                    ..
                }
            )
        })
}

async fn wait_for_owner_wake_source(
    wake_rx: &mut mpsc::UnboundedReceiver<RendererOwnerWake>,
    expected: RendererOwnerWakeSource,
) {
    let mut observed = Vec::new();
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let wake = wake_rx
                .recv()
                .await
                .expect("open-stream owner wake route should remain open");
            let source = wake.source_for_test();
            observed.push(source);
            if source == expected {
                break;
            }
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!("open-stream producer should publish {expected:?}; observed wakes: {observed:?}")
    });
}

async fn resume_open_stream(
    residence: PendingPhaseOneResidence,
    operation: &'static str,
) -> PendingPhaseOneResidence {
    let executor = residence.page_vm().local_executor.clone();
    let outcome =
        super::access::run_named_owner_local_task(executor, operation, residence.resume())
            .await
            .expect("open-stream continuation should resume");
    let PendingPhaseOneResumeOutcome::Progress(outcome) = outcome else {
        panic!("successful open-stream fixture must not fail its main resource");
    };
    let ParseTimePageVmCreationOutcome::PendingPhaseOne(residence) = outcome else {
        panic!("open body and its ready Page task should keep phase one resident");
    };
    residence
}

async fn apply_next_networking_task(
    mut residence: PendingPhaseOneResidence,
) -> (PendingPhaseOneResidence, AppliedStreamingNetworkingTask) {
    let executor = residence.page_vm().local_executor.clone();
    let request_client = residence
        .page_vm()
        .main_document_resource_loader()
        .request_client()
        .clone();
    let (residence, outcome) = super::access::run_named_owner_local_task(
        executor,
        "open-stream stylesheet-networking executor channel closed",
        async move {
            if residence
                .page_vm_mut()
                .run_exact_selected_page_task_for_test(
                    crate::runtime::page_vm::PageSelectedTaskTestSelector::StylesheetCompletion,
                    &request_client,
                )
                .await?
            {
                return Ok::<_, anyhow::Error>((
                    residence,
                    Some(AppliedStreamingNetworkingTask::StylesheetCompletion),
                ));
            }
            if residence
                .page_vm_mut()
                .run_exact_selected_page_task_for_test(
                    crate::runtime::page_vm::PageSelectedTaskTestSelector::MainParserContinuation,
                    &request_client,
                )
                .await?
            {
                return Ok::<_, anyhow::Error>((
                    residence,
                    Some(AppliedStreamingNetworkingTask::MainParserContinuation),
                ));
            }
            Ok::<_, anyhow::Error>((residence, None))
        },
    )
    .await
    .expect("stylesheet Networking terminal should execute");
    assert!(
        outcome.is_some(),
        "the exact stylesheet Networking terminal should remain resident after its wake is consumed"
    );
    (
        residence,
        outcome.expect("checked Networking outcome should remain present"),
    )
}

async fn apply_next_main_parser_continuation(
    residence: PendingPhaseOneResidence,
) -> PendingPhaseOneResidence {
    let (residence, action) = apply_next_networking_task(residence).await;
    let AppliedStreamingNetworkingTask::MainParserContinuation = action else {
        panic!("stylesheet unblock must be followed by its parser continuation task");
    };
    residence
}

async fn apply_oldest_ready_page_task(
    mut residence: PendingPhaseOneResidence,
) -> (PendingPhaseOneResidence, bool) {
    let executor = residence.page_vm().local_executor.clone();
    let request_client = residence
        .page_vm()
        .main_document_resource_loader()
        .request_client()
        .clone();
    super::access::run_named_owner_local_task(
        executor,
        "open-stream production selected-task executor channel closed",
        async move {
            let applied = residence
                .page_vm_mut()
                .run_one_oldest_ready_page_task_on_owner_lane_for_test(&request_client)
                .await?;
            Ok::<_, anyhow::Error>((residence, applied))
        },
    )
    .await
    .expect("production selected Page task should execute")
}

async fn resume_phase_one_once(
    residence: PendingPhaseOneResidence,
    operation: &'static str,
) -> ParseTimePageVmCreationOutcome {
    let executor = residence.page_vm().local_executor.clone();
    let outcome =
        super::access::run_named_owner_local_task(executor, operation, residence.resume())
            .await
            .expect("phase-one continuation should resume");
    let PendingPhaseOneResumeOutcome::Progress(outcome) = outcome else {
        panic!("successful phase-one fixture must not fail its main resource");
    };
    outcome
}

#[test]
fn followed_navigation_returns_at_document_commit_before_parsing_buffered_body() {
    super::tests::run_phase_one_large_stack_test("followed-navigation-document-commit", || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");
        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let loader =
                ResourceRequestClient::new(&FetchConfig::default()).expect("default test loader");
            let page_id = PageId::new_for_testing(79);
            let local_executor = JsLocalExecutor::new();
            let (wake_tx, _wake_rx) = mpsc::unbounded_channel();
            let owner_wake = crate::page_task_queue::RendererOwnerWakeSender::new(
                wake_tx,
                crate::runtime::RendererPageToken::new_for_testing(page_id),
            );
            let hooks =
                PageVmRuntimeHooks::standalone_with_owner_wake_without_owner_reservation_for_test(
                    owner_wake,
                );
            let (completion_tx, completion_rx) = oneshot::channel();
            let (body_tx, raw_body) = ExternalRawDocumentBodyStream::channel(completion_rx);
            body_tx
                .try_send(
                    b"<!doctype html><body><main id=\"must-not-be-parsed-before-commit\"></main></body>"
                        .to_vec(),
                )
                .expect("complete response body should fit the fixture channel");
            drop(body_tx);
            completion_tx
                .send(Ok(()))
                .expect("complete response should publish its terminal");

            let creation_executor = local_executor.clone();
            let mut env = super::tests::default_test_page_vm_env_config();
            env.document_start_scripts.push(crate::DocumentStartScript {
                registry_key: Some("commit-boundary-probe".to_owned()),
                source: "globalThis.__documentStartAfterCommit = true".to_owned(),
                world_name: None,
                has_bidi_channel_argument: false,
                browser_internal: false,
                bidi_channel_handoffs: Vec::new(),
            });
            let creation = super::access::run_named_owner_local_task(
                local_executor,
                "followed-navigation commit bootstrap channel closed",
                async move {
                    ConcurrentParseTimeRuntime::prepare_document_from_committed_external_raw_document_response(
                        page_id,
                        creation_executor,
                        &loader,
                        &env,
                        hooks,
                        PageVmInitStage::Load,
                        Instant::now(),
                        Url::parse("https://example.test/").expect("test URL"),
                        200,
                        vec![("content-type".to_owned(), "text/html".to_owned())],
                        raw_body,
                    )
                    .await
                },
            )
            .await
            .expect("followed navigation should prepare its committed Document");
            let StreamingNavigationPageCreationResult::Html(creation) = creation else {
                panic!("HTML response should not become a download");
            };
            let ParseTimePageVmCreationOutcome::PendingPhaseOne(residence) = creation.outcome else {
                panic!("Document commit must return a parkable phase-one continuation");
            };
            assert!(
                has_ready_main_parser_continuation(&residence),
                "buffered body input must advertise its exact parser continuation after commit"
            );
            let parsed_target = residence
                .page_vm()
                .vm()
                .document_runtime
                .dom_host()
                .dom()
                .nodes()
                .iter()
                .filter_map(Node::as_element)
                .any(|element| {
                    element.attribute("id") == Some("must-not-be-parsed-before-commit")
                });
            assert!(
                !parsed_target,
                "buffered response bytes must remain parser input until after Document publication"
            );
            let observation_executor = residence.page_vm().local_executor.clone();
            let (residence, document_start_before_commit) =
                super::access::run_named_owner_local_task(
                    observation_executor,
                    "followed-navigation pre-commit document-start observation closed",
                    async move {
                        let mut residence = residence;
                        let observed = residence.page_vm_mut().vm_mut().eval(
                            "String(globalThis.__documentStartAfterCommit === true)",
                        )?;
                        Ok::<_, anyhow::Error>((residence, observed))
                    },
                )
                .await
                .expect("pre-commit document-start observation should run");
            assert_eq!(
                document_start_before_commit, "false",
                "document-start scripts must remain in the phase-one residence until after Document publication"
            );

            let resumed = resume_phase_one_once(
                residence,
                "followed-navigation post-commit document-start continuation closed",
            )
            .await;
            let observation_executor = match &resumed {
                ParseTimePageVmCreationOutcome::PendingPhaseOne(residence) => residence.page_vm(),
                ParseTimePageVmCreationOutcome::TriggeredNavigation { page_vm, .. }
                | ParseTimePageVmCreationOutcome::ContinuePhaseTwo { page_vm, .. } => page_vm,
            }
            .local_executor
            .clone();
            let (_, document_start_after_commit) = super::access::run_named_owner_local_task(
                observation_executor,
                "followed-navigation post-commit document-start observation closed",
                async move {
                    let mut resumed = resumed;
                    let page_vm = match &mut resumed {
                        ParseTimePageVmCreationOutcome::PendingPhaseOne(residence) => {
                            residence.page_vm_mut()
                        }
                        ParseTimePageVmCreationOutcome::TriggeredNavigation { page_vm, .. }
                        | ParseTimePageVmCreationOutcome::ContinuePhaseTwo { page_vm, .. } => {
                            page_vm
                        }
                    };
                    let observed = page_vm
                        .vm_mut()
                        .eval("String(globalThis.__documentStartAfterCommit === true)")?;
                    Ok::<_, anyhow::Error>((resumed, observed))
                },
            )
            .await
            .expect("post-commit document-start observation should run");
            assert_eq!(
                document_start_after_commit, "true",
                "the exact phase-one continuation must execute deferred document-start scripts after commit"
            );
        }));
    });
}

#[test]
fn complete_current_chunk_does_not_manufacture_a_budget_continuation() {
    super::tests::run_phase_one_large_stack_test("complete-parser-chunk-no-continuation", || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");
        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
            let _js_runtime = crate::JsRuntime::initialize();
            let loader =
                ResourceRequestClient::new(&FetchConfig::default()).expect("default test loader");
            let page_id = PageId::new_for_testing(80);
            let local_executor = JsLocalExecutor::new();
            let (wake_tx, _wake_rx) = mpsc::unbounded_channel();
            let owner_wake = crate::page_task_queue::RendererOwnerWakeSender::new(
                wake_tx,
                crate::runtime::RendererPageToken::new_for_testing(page_id),
            );
            let hooks =
                PageVmRuntimeHooks::standalone_with_owner_wake_without_owner_reservation_for_test(
                    owner_wake,
                );
            let env = super::tests::default_test_page_vm_env_config();
            let creation_executor = local_executor.clone();
            let outcome = super::access::run_named_owner_local_task(
                local_executor,
                "complete parser chunk bootstrap channel closed",
                async move {
                    ConcurrentParseTimeRuntime::finish_creation_from_html_bootstrap(
                        page_id,
                        creation_executor,
                        &loader,
                        &env,
                        hooks,
                        Url::parse("https://example.test/").expect("test URL"),
                        PageVmInitStage::Load,
                        "<!doctype html><body>complete</body>".to_owned(),
                        Instant::now(),
                    )
                    .await
                },
            )
            .await
            .expect("small complete body should finish phase one");

            assert!(
                matches!(
                    outcome,
                    ParseTimePageVmCreationOutcome::ContinuePhaseTwo { .. }
                ),
                "consuming the complete current chunk is not parser-budget exhaustion"
            );
        }));
    });
}

#[test]
fn open_streaming_residence_does_not_treat_link_event_as_parser_obstruction() {
    super::tests::run_phase_one_large_stack_test("open-stream-link-event-independence", || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");
        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
                let _js_runtime = crate::JsRuntime::initialize();
                let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("default test loader");
                let page_id = PageId::new_for_testing(81);
                let local_executor = JsLocalExecutor::new();
                let (wake_tx, mut wake_rx) = mpsc::unbounded_channel();
                let owner_wake = crate::page_task_queue::RendererOwnerWakeSender::new(
                    wake_tx,
                    crate::runtime::RendererPageToken::new_for_testing(page_id),
                );
                let hooks =
                    PageVmRuntimeHooks::standalone_with_owner_wake_without_owner_reservation_for_test(
                        owner_wake,
                    );
                let (completion_tx, completion_rx) = oneshot::channel();
                let (body_tx, raw_body) =
                    ExternalRawDocumentBodyStream::channel(completion_rx);
                body_tx
                    .try_send(b"<!doctype html><head>".to_vec())
                    .expect("initial body chunk should fit the bounded input");

                let creation_executor = local_executor.clone();
                let creation = super::access::run_named_owner_local_task(
                    local_executor,
                    "open-stream suspension bootstrap channel closed",
                    async move {
                        ConcurrentParseTimeRuntime::finish_creation_from_committed_external_raw_document_response(
                            page_id,
                            creation_executor,
                            &loader,
                            &super::tests::default_test_page_vm_env_config(),
                            hooks,
                            PageVmInitStage::Load,
                            Instant::now(),
                            Url::parse("https://example.test/").expect("test URL"),
                            200,
                            vec![("content-type".to_owned(), "text/html".to_owned())],
                            raw_body,
                        )
                        .await
                    },
                )
                .await
                .expect("open streaming Page should bootstrap");
                let StreamingNavigationPageCreationResult::Html(creation) = creation else {
                    panic!("HTML body should not become a download");
                };
                let ParseTimePageVmCreationOutcome::PendingPhaseOne(mut residence) =
                    creation.outcome
                else {
                    panic!("open body should retain a phase-one residence");
                };
                if has_ready_main_parser_continuation(&residence) {
                    residence = apply_next_main_parser_continuation(residence).await;
                    residence = resume_open_stream(
                        residence,
                        "open-stream initial parser continuation channel closed",
                    )
                    .await;
                }
                assert!(
                    !has_ready_main_parser_continuation(&residence),
                    "initial parser continuation should be consumed before later input"
                );

                body_tx
                    .send(
                        br#"<link rel="stylesheet" href="data:text/css,body%7Bcolor%3Agreen%7D">"#
                            .to_vec(),
                    )
                    .await
                    .expect("later stylesheet chunk should send");
                wait_for_owner_wake_source(
                    &mut wake_rx,
                    RendererOwnerWakeSource::NetworkingTask,
                )
                .await;
                let mut residence = resume_open_stream(
                    residence,
                    "open-stream connected-style resume channel closed",
                )
                .await;

                // Parser budget and stylesheet completion share one FIFO.
                // Dequeue by the actual action rather than guessing that a
                // particular wake must name the stylesheet terminal.
                if !has_ready_main_parser_continuation(&residence)
                    && !has_ready_stylesheet_completion(&residence)
                {
                    wait_for_owner_wake_source(
                        &mut wake_rx,
                        RendererOwnerWakeSource::NetworkingTask,
                    )
                    .await;
                    residence = resume_open_stream(
                        residence,
                        "open-stream stylesheet-networking observation channel closed",
                    )
                    .await;
                }
                assert!(
                    has_ready_main_parser_continuation(&residence)
                        || has_ready_stylesheet_completion(&residence),
                    "a concrete Networking descriptor, not phase-one state, must retain the work"
                );
                let mut saw_stylesheet_completion = false;
                for _ in 0..8 {
                    let (next, action) = apply_next_networking_task(residence).await;
                    residence = next;
                    match action {
                        AppliedStreamingNetworkingTask::MainParserContinuation => {
                            residence = resume_open_stream(
                                residence,
                                "open-stream parser-budget continuation channel closed",
                            )
                            .await;
                            if !has_ready_main_parser_continuation(&residence)
                                && !has_ready_stylesheet_completion(&residence)
                            {
                                wait_for_owner_wake_source(
                                    &mut wake_rx,
                                    RendererOwnerWakeSource::NetworkingTask,
                                )
                                .await;
                                residence = resume_open_stream(
                                    residence,
                                    "open-stream stylesheet-terminal observation channel closed",
                                )
                                .await;
                            }
                        }
                        AppliedStreamingNetworkingTask::StylesheetCompletion => {
                            saw_stylesheet_completion = true;
                            break;
                        }
                    }
                    assert!(
                        has_ready_main_parser_continuation(&residence)
                            || has_ready_stylesheet_completion(&residence),
                        "the shared Networking FIFO should retain a concrete descriptor"
                    );
                }
                assert!(
                    saw_stylesheet_completion,
                    "the bounded Networking FIFO must reach stylesheet completion"
                );

                // The `<link>` event is independent DOM-manipulation work,
                // while stylesheet unblock publishes a separate Networking
                // parser continuation. Observing the DOM wake does not grant
                // permission to skip that continuation task.
                wait_for_owner_wake_source(
                    &mut wake_rx,
                    RendererOwnerWakeSource::DomManipulationTask,
                )
                .await;
                let residence = resume_open_stream(
                    residence,
                    "open-stream connected-style source-observation channel closed",
                )
                .await;
                assert!(
                    has_ready_main_parser_continuation(&residence),
                    "stylesheet unblock must remain an explicit parser continuation"
                );
                let residence = apply_next_main_parser_continuation(residence).await;
                let residence = resume_open_stream(
                    residence,
                    "open-stream admitted parser continuation channel closed",
                )
                .await;
                assert!(
                    has_ready_connected_style_event(&residence),
                    "the posted <link> event must not become a parser prerequisite"
                );

                drop(residence);
                drop(body_tx);
                let _ = completion_tx.send(Ok(()));
            }));
    });
}

#[test]
fn streaming_stylesheet_and_json_ld_reach_tail_and_post_parse_boundary() {
    super::tests::run_phase_one_large_stack_test("streaming-stylesheet-json-ld-liveness", || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime should build");
        runtime.block_on(tokio::task::LocalSet::new().run_until(async move {
                let _js_runtime = crate::JsRuntime::initialize();
                let loader = ResourceRequestClient::new(&FetchConfig::default()).expect("default test loader");
                let page_id = PageId::new_for_testing(82);
                let local_executor = JsLocalExecutor::new();
                let (wake_tx, _wake_rx) = mpsc::unbounded_channel();
                let owner_wake = crate::page_task_queue::RendererOwnerWakeSender::new(
                    wake_tx,
                    crate::runtime::RendererPageToken::new_for_testing(page_id),
                );
                let hooks =
                    PageVmRuntimeHooks::standalone_with_owner_wake_without_owner_reservation_for_test(
                        owner_wake,
                    );
                let (completion_tx, completion_rx) = oneshot::channel();
                let (body_tx, raw_body) =
                    ExternalRawDocumentBodyStream::channel(completion_rx);
                body_tx
                    .try_send(b"<!doctype html><html><head>".to_vec())
                    .expect("initial body chunk should fit");

                let creation_executor = local_executor.clone();
                let creation = super::access::run_named_owner_local_task(
                    local_executor,
                    "streaming stylesheet JSON-LD bootstrap channel closed",
                    async move {
                        ConcurrentParseTimeRuntime::finish_creation_from_committed_external_raw_document_response(
                            page_id,
                            creation_executor,
                            &loader,
                            &super::tests::default_test_page_vm_env_config(),
                            hooks,
                            PageVmInitStage::Load,
                            Instant::now(),
                            Url::parse("https://example.test/").expect("test URL"),
                            200,
                            vec![("content-type".to_owned(), "text/html".to_owned())],
                            raw_body,
                        )
                        .await
                    },
                )
                .await
                .expect("streaming Page should bootstrap");
                let StreamingNavigationPageCreationResult::Html(creation) = creation else {
                    panic!("HTML body should not become a download");
                };
                let ParseTimePageVmCreationOutcome::PendingPhaseOne(residence) = creation.outcome
                else {
                    panic!("open body should retain a phase-one residence");
                };

                body_tx
                    .send(
                        br##"<link rel="stylesheet" href="data:text/css,%23css-marker%7Bdisplay%3Anone%7D"><script type="application/ld+json">{"name":"moli"}</script></head><body><div id="css-marker"></div><script>document.body.setAttribute("data-css-before-script",getComputedStyle(document.querySelector("#css-marker")).display);document.addEventListener("DOMContentLoaded",()=>document.body.setAttribute("data-dcl","fired"))</script><main id="stream-tail">tail</main></body></html>"##
                            .to_vec(),
                    )
                    .await
                    .expect("tail body chunk should send");
                drop(body_tx);
                completion_tx
                    .send(Ok(()))
                    .expect("body completion should send");

                let (page_vm, page_tasks) = tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    async move {
                        let mut residence = residence;
                        let mut turns = 0;
                        loop {
                            turns += 1;
                            assert!(
                                turns <= 32,
                                "phase one exceeded its bounded continuation budget"
                            );
                            residence = apply_oldest_ready_page_task(residence).await.0;
                            match resume_phase_one_once(
                                residence,
                                "streaming stylesheet JSON-LD continuation channel closed",
                            )
                            .await
                            {
                                ParseTimePageVmCreationOutcome::PendingPhaseOne(next) => {
                                    residence = next;
                                }
                                ParseTimePageVmCreationOutcome::ContinuePhaseTwo {
                                    page_vm,
                                    page_tasks,
                                    ..
                                } => break (page_vm, page_tasks),
                                ParseTimePageVmCreationOutcome::TriggeredNavigation { .. } => {
                                    panic!("fixture must not trigger navigation")
                                }
                            }
                        }
                    },
                )
                .await
                .expect("streaming CSS + JSON-LD must not deadlock");

                let snapshot = page_vm.vm().snapshot_live_document();
                assert!(
                    snapshot
                        .nodes()
                        .iter()
                        .filter_map(Node::as_element)
                        .any(|element| element.attribute("id") == Some("stream-tail")),
                    "parser must consume the marker after the data block"
                );
                let body = snapshot.document_body_handle().expect("body");
                assert_eq!(
                    snapshot
                        .node(body)
                        .and_then(Node::as_element)
                        .and_then(|element| element.attribute("data-css-before-script")),
                    Some("none"),
                    "the later parser script must observe CSS before execution"
                );
                assert!(
                    page_vm.report.runs.iter().any(|run| {
                        run.kind() == crate::types::ScriptKind::DataBlock
                            && matches!(
                                run.outcome(),
                                crate::types::ScriptRunOutcome::Skipped(
                                    crate::types::ScriptSkipReason::UnsupportedType(_)
                                )
                            )
                    }),
                    "JSON-LD must remain visible in the internal report without becoming a Page task"
                );

                // Phase one only hands the parser-produced work to the
                // post-parse driver. Drive that real owner-local lifecycle to
                // its authoritative milestone instead of assuming DCL is
                // already represented by one raw handoff task.
                let executor = page_vm.local_executor.clone();
                super::access::run_named_owner_local_task(
                    executor,
                    "streaming stylesheet JSON-LD lifecycle channel closed",
                    async move {
                        let mut page_vm = page_vm;
                        let mut pending = None;
                        let document = match page_vm
                            .begin_post_parse_lifecycle_on_named_owner_lane(
                                &mut pending,
                                page_tasks,
                                PageVmInitStage::DomContentLoaded,
                                Instant::now(),
                            )
                            .await?
                        {
                            DocumentLifecycleTurnOutcome {
                                readiness:
                                    DocumentLifecycleTurnReadiness::Runnable { document },
                                ..
                            } => document,
                            outcome => {
                                panic!(
                                    "post-parse DCL lifecycle should start runnable: {outcome:?}"
                                )
                            }
                        };

                        for _ in 0..128 {
                            match page_vm
                                .advance_post_parse_lifecycle_one_owner_turn(
                                    &mut pending,
                                    document,
                                )
                                .await?
                            {
                                DocumentLifecycleTurnOutcome {
                                    action:
                                        DocumentLifecycleTurnAction::ReachedStage(
                                            PageVmInitStage::DomContentLoaded,
                                        ),
                                    ..
                                } => {
                                    let snapshot = page_vm.vm().snapshot_live_document();
                                    let body =
                                        snapshot.document_body_handle().expect("body after DCL");
                                    assert_eq!(
                                        snapshot
                                            .node(body)
                                            .and_then(Node::as_element)
                                            .and_then(|element| element.attribute("data-dcl")),
                                        Some("fired"),
                                        "the real DOMContentLoaded listener must run"
                                    );
                                    return Ok::<_, anyhow::Error>(());
                                }
                                DocumentLifecycleTurnOutcome {
                                    readiness:
                                        DocumentLifecycleTurnReadiness::Runnable { .. },
                                    ..
                                } => {}
                                outcome => panic!(
                                    "streaming fixture should reach DCL without parking: {outcome:?}"
                                ),
                            }
                        }
                        panic!("streaming fixture did not reach DCL within the bounded lifecycle")
                    },
                )
                .await
                .expect("streaming fixture should finish its DCL lifecycle");
            }));
    });
}
