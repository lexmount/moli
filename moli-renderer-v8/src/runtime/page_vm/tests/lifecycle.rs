use super::super::main_document_lifecycle_completion::execute_main_document_lifecycle_on_owner_local_task;
use super::super::parser_continuation::MainParserContinuationCompletion;
use super::super::parser_task_completion::MainParserContinuationTaskEffect;
use super::super::{
    DocumentLifecycleTurnAction, DocumentLifecycleTurnOutcome, DocumentLifecycleTurnReadiness,
    ParseTimeMainParserBoundaryOutcome, PendingDocumentLifecycleTurn,
};
use super::*;
use crate::{
    RendererDocumentLifecycleEventKind, RendererDocumentLifecycleIdentity,
    RendererDocumentLifecycleMilestone, RendererDocumentLifecycleWaitOutcome,
    RendererDocumentTerminationReason, RendererLifecycleStartReason, RendererPageState,
    page_task_queue::PostParseLifecycleWork,
    runtime::document_lifecycle_turn::DocumentLifecycleNavigationTiming,
    script_vm::{
        MainDocumentLifecycleBody, MainDocumentLifecycleBodyKind,
        MainDocumentLifecycleCallbackEffect, MainDocumentLifecycleFollowup,
        MainDocumentLifecycleStep, MainDocumentLifecycleTargetEffect,
        MainDocumentLifecycleTargetRejection,
    },
};
use std::sync::Arc;

#[test]
fn stale_domcontentloaded_barrier_preserves_current_document_frontend_bindings() {
    let mut page_vm = test_page_vm();
    let document_handle = page_vm
        .vm()
        .document_runtime
        .dom_host()
        .dom()
        .document_node_id();
    let backend_node_id = page_vm
        .renderer_backend_node_id_for_live_handle(document_handle)
        .expect("current document backend node id");
    let frontend_node_id = page_vm
        .document_frontend_node_id_for_backend_node_id(Some("session-current"), backend_node_id);
    let current_document = page_vm.document_lifecycle.identity();
    let stale_document = RendererDocumentLifecycleIdentity {
        epoch: crate::RendererLifecycleEpoch(0),
        ..current_document
    };

    assert!(
        !page_vm.prepare_dom_agent_for_main_document_dom_content_loaded(stale_document),
        "a stale DCL completion must not run the current Document's binding barrier"
    );
    assert_eq!(
        page_vm.document_frontend_node_binding(Some("session-current"), frontend_node_id),
        crate::RendererDomFrontendNodeBindingResolution::BackendNodeId(backend_node_id),
        "the stale barrier must preserve the current Document's frontend identity"
    );
}

#[tokio::test]
async fn history_keeps_action_window_but_document_open_cancels_it() {
    run_page_vm_async_test(async move {
        let page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                let mut page_vm = page_vm;
                let initial_document = page_vm.document_lifecycle.identity();
                let outcome = page_vm.queue_wheel_event(
                    10.0,
                    10.0,
                    -1,
                    Some(0),
                    0,
                    0.0,
                    25.0,
                    crate::RendererPointerEventProperties::default(),
                    0,
                )?;
                assert!(outcome.handled);
                let deadline = page_vm
                    .next_action_window_deadline()
                    .expect("the first wheel should open an action window");
                assert_eq!(page_vm.pending_action_counts_for_test(), (1, 1));

                let history_snapshot = page_vm.document_replacement_lifecycle_action_snapshot();
                assert_eq!(
                    page_vm
                        .vm_mut()
                        .eval("history.pushState({}, '', '#same-document'); location.hash")?,
                    "#same-document"
                );
                let mut pending_document_lifecycle_turn = None;
                assert!(
                    page_vm
                        .reconcile_document_replacement_lifecycle_after_owner_action(
                            history_snapshot,
                            &mut pending_document_lifecycle_turn,
                        )
                        .await?
                        .is_none()
                );
                assert_eq!(page_vm.document_lifecycle.identity(), initial_document);
                assert_eq!(
                    page_vm.next_action_window_deadline(),
                    Some(deadline),
                    "same-Document history must retain the existing action window"
                );
                assert_eq!(page_vm.pending_action_counts_for_test(), (1, 1));

                let open_snapshot = page_vm.document_replacement_lifecycle_action_snapshot();
                assert_eq!(
                    page_vm.vm_mut().eval("document.open(); 'opened'")?,
                    "opened"
                );
                let opened_document = page_vm.document_lifecycle.identity();
                assert_eq!(
                    opened_document.document, initial_document.document,
                    "document.open keeps the cross-document token generation"
                );
                assert_ne!(
                    opened_document.epoch, initial_document.epoch,
                    "document.open must still create a new exact Document lifecycle"
                );
                assert_eq!(
                    page_vm.pending_action_counts_for_test(),
                    (1, 1),
                    "cancellation belongs to the owner lifecycle reconciliation boundary"
                );

                assert!(
                    page_vm
                        .reconcile_document_replacement_lifecycle_after_owner_action(
                            open_snapshot,
                            &mut pending_document_lifecycle_turn,
                        )
                        .await?
                        .is_none(),
                    "an open replacement stream should not yet install lifecycle work"
                );
                assert_eq!(page_vm.next_action_window_deadline(), None);
                assert_eq!(
                    page_vm.pending_action_counts_for_test(),
                    (0, 0),
                    "retiring the exact Document must remove semantic actions and host payloads"
                );
                Ok::<_, anyhow::Error>(())
            })
            .await
    })
    .await
    .expect("action-window exact Document lifecycle test should run");
}

fn replacement_collision_dynamic_import_owner_action(
    target: ChildDocumentModuleFetchTarget,
) -> crate::frame_owner_model::FrameDocumentDynamicImportTerminalPreparedAction {
    let key = ModuleMapKey::java_script(
        Url::parse("https://retired-document.test/dynamic-owner-action.mjs")
            .expect("replacement collision dynamic-import URL"),
    );
    let client = crate::module_runtime::NativeDynamicImportSingleModuleClient::new(
        module_tree::SingleModuleClientToken {
            tree_id: module_tree::ModuleTreeId(991),
            sequence: 1,
        },
        module_tree::ModuleImportPhase::Evaluation,
    );
    crate::frame_owner_model::FrameDocumentDynamicImportTerminalPreparedAction::from_terminal_work(
        crate::frame_owner_model::FrameDocumentDynamicImportTerminalWork::from_terminal_parts(
            target.task_owner(),
            target.realm_id(),
            key,
            client,
        ),
    )
}

fn post_parse_detached_runs_work(runs: Vec<ScriptRun>) -> PostParsePageOwnedWork {
    PostParsePageOwnedWork::lifecycle_work(PostParseLifecycleWork::RecordDetachedPostParseRuns(
        runs,
    ))
}

async fn advance_unblocked_exact_lifecycle_to_stage(
    page_vm: &mut PageVm,
    pending: &mut Option<PendingDocumentLifecycleTurn>,
    document: RendererDocumentLifecycleIdentity,
    stage: PageVmInitStage,
) -> anyhow::Result<DocumentLifecycleTurnOutcome> {
    for _ in 0..64 {
        let outcome = page_vm
            .advance_post_parse_lifecycle_one_owner_turn(pending, document)
            .await?;
        if matches!(
            outcome.action,
            DocumentLifecycleTurnAction::ReachedStage(reached) if reached == stage
        ) {
            return Ok(outcome);
        }
        match &outcome {
            DocumentLifecycleTurnOutcome {
                readiness:
                    DocumentLifecycleTurnReadiness::Runnable {
                        document: runnable_document,
                    },
                ..
            } => assert_eq!(*runnable_document, document),
            _ => panic!(
                "unblocked exact lifecycle should remain runnable before {stage:?}: {outcome:?}"
            ),
        }
    }
    panic!("bounded exact lifecycle fixture should reach {stage:?}")
}

async fn prepare_parse_time_exact_domcontentloaded(
    page_vm: &mut PageVm,
) -> anyhow::Result<crate::frame_owner_model::FrameDocumentTaskOwner> {
    let owner = page_vm
        .vm()
        .current_main_document_task_owner()
        .expect("parse-time DCL fixture requires a current Document owner");
    let _driver = {
        let PageVm {
            vm,
            page_task_queue,
            report,
            ..
        } = page_vm;
        vm.as_mut()
            .expect("parse-time DCL fixture should retain ScriptVm")
            .start_post_parse_lifecycle_round(
                PageVmInitStage::DomContentLoaded,
                page_task_queue,
                report,
                Vec::new(),
            )
            .await
    };
    let interactive = page_vm
        .page_task_queue
        .post_parse_pop_front()
        .expect("post-parse admission should queue interactive before exact DCL");
    let PostParsePageOwnedWork::Lifecycle(interactive) = interactive else {
        panic!("first admitted lifecycle task must be interactive");
    };
    let PostParseLifecycleWork::ApplyMainDocumentInteractive(interactive) = *interactive else {
        panic!("first admitted lifecycle task must be interactive");
    };
    let run = super::super::main_document_lifecycle_completion::execute_main_document_lifecycle_on_owner_local_task(
        page_vm,
        crate::script_vm::MainDocumentLifecycleBody::Interactive(interactive),
    )
    .await?;
    assert_eq!(
        run.completion.kind(),
        MainDocumentLifecycleBodyKind::Interactive
    );
    assert!(
        page_vm
            .page_task_queue
            .post_parse_front()
            .is_some_and(|work| work.is_domcontentloaded_task()),
        "interactive completion must publish the exact DCL successor before parser claim"
    );
    Ok(owner)
}

async fn exact_lifecycle_turn_reaches_handler_navigation(
    event_target: &str,
    event_name: &str,
    stage: PageVmInitStage,
    handler_expression: &str,
) -> anyhow::Result<()> {
    let expression = format!(
        "{event_target}.addEventListener({event_name:?}, () => {{ {handler_expression} }})"
    );
    let page_vm = test_page_vm();
    let local_executor = page_vm.local_executor.clone();
    local_executor
        .run(async move {
            let mut page_vm = page_vm;
            page_vm.vm_mut().eval(&expression)?;
            let mut pending = None;
            let document = match page_vm
                .begin_post_parse_lifecycle_on_named_owner_lane(
                    &mut pending,
                    Vec::new(),
                    stage,
                    Instant::now(),
                )
                .await?
            {
                DocumentLifecycleTurnOutcome {
                    readiness: DocumentLifecycleTurnReadiness::Runnable { document },
                    ..
                } => document,
                outcome => {
                    panic!("handler lifecycle should install a runnable resident: {outcome:?}")
                }
            };

            for _ in 0..128 {
                match page_vm
                    .advance_post_parse_lifecycle_one_owner_turn(&mut pending, document)
                    .await?
                {
                    DocumentLifecycleTurnOutcome {
                        action: DocumentLifecycleTurnAction::RequestedTopLevelNavigation {
                            source_document,
                            stage: reached,
                            timing,
                        },
                        readiness: DocumentLifecycleTurnReadiness::Idle,
                    } => {
                        assert_eq!(reached, stage);
                        assert_eq!(source_document, document);
                        assert_eq!(
                            timing,
                            DocumentLifecycleNavigationTiming::AfterMilestone
                        );
                        assert!(page_vm.vm().has_pending_location_navigation());
                        assert!(pending.is_none());
                        return Ok(());
                    }
                    DocumentLifecycleTurnOutcome {
                        readiness: DocumentLifecycleTurnReadiness::Runnable { .. },
                        ..
                    } => {}
                    outcome => panic!(
                        "handler navigation should be surfaced by its exact lifecycle action: {outcome:?}"
                    ),
                }
            }
            panic!("handler navigation did not become runnable within the bounded lifecycle fixture")
        })
        .await
}

async fn exact_lifecycle_turn_publishes_document_open_replacement(
    listener_source: &str,
) -> anyhow::Result<()> {
    let mut page_vm = test_page_vm();
    let local_executor = page_vm.local_executor.clone();
    let listener_source = listener_source.to_owned();
    local_executor
        .run(async move {
            let mut pending_document_lifecycle_turn = None;
            page_vm.vm_mut().eval(&listener_source)?;
            let mut document = match page_vm
                .begin_post_parse_lifecycle_on_named_owner_lane(
                    &mut pending_document_lifecycle_turn,
                    Vec::new(),
                    PageVmInitStage::Load,
                    Instant::now(),
                )
                .await?
            {
                DocumentLifecycleTurnOutcome {
                    readiness: DocumentLifecycleTurnReadiness::Runnable { document },
                    ..
                } => document,
                outcome => {
                    panic!("replacement lifecycle should install a runnable resident: {outcome:?}")
                }
            };
            let initial_document = document;

            for _ in 0..96 {
                match page_vm
                    .advance_post_parse_lifecycle_one_owner_turn(
                        &mut pending_document_lifecycle_turn,
                        document,
                    )
                    .await?
                {
                    DocumentLifecycleTurnOutcome {
                        action: DocumentLifecycleTurnAction::DocumentReplaced { current, .. },
                        readiness: DocumentLifecycleTurnReadiness::Runnable { document },
                    } => {
                        assert_eq!(document, current);
                        assert_ne!(document, initial_document);
                        assert_eq!(
                            document,
                            page_vm.document_lifecycle.current_snapshot().into()
                        );
                        assert_eq!(
                            pending_document_lifecycle_turn
                                .as_ref()
                                .map(|pending| pending.document),
                            Some(document),
                            "the replacement boundary must install a newly-bound continuation"
                        );
                        return Ok::<_, anyhow::Error>(());
                    }
                    DocumentLifecycleTurnOutcome {
                        readiness:
                            DocumentLifecycleTurnReadiness::Runnable {
                                document: next_document,
                            },
                        ..
                    } => {
                        assert_eq!(next_document, document);
                        document = next_document;
                    }
                    DocumentLifecycleTurnOutcome {
                        readiness: DocumentLifecycleTurnReadiness::Blocked { .. },
                        ..
                    } => panic!("replacement fixture has no external lifecycle blocker"),
                    DocumentLifecycleTurnOutcome {
                        action,
                        readiness: DocumentLifecycleTurnReadiness::Idle,
                    } => panic!("old lifecycle ended without publishing replacement: {action:?}"),
                }
            }
            panic!("lifecycle callback did not publish replacement within the bounded fixture")
        })
        .await
}

#[test]
fn domcontentloaded_handler_navigation_is_an_exact_turn_action() {
    run_page_vm_local_runtime_async_test("page-vm-dcl-handler-navigation", || async move {
        exact_lifecycle_turn_reaches_handler_navigation(
            "document",
            "DOMContentLoaded",
            PageVmInitStage::DomContentLoaded,
            "location.href = 'https://replacement.test/next'",
        )
        .await
        .expect("DCL handler navigation should be surfaced")
    });
}

#[test]
fn load_handler_navigation_is_an_exact_turn_action() {
    run_page_vm_local_runtime_async_test("page-vm-load-handler-navigation", || async move {
        exact_lifecycle_turn_reaches_handler_navigation(
            "window",
            "load",
            PageVmInitStage::Load,
            "location.href = 'https://replacement.test/next'",
        )
        .await
        .expect("load handler navigation should be surfaced")
    });
}

#[test]
fn load_handler_navigation_reload_is_an_exact_turn_action() {
    run_page_vm_local_runtime_async_test("page-vm-load-handler-navigation-reload", || async move {
        exact_lifecycle_turn_reaches_handler_navigation(
            "window",
            "load",
            PageVmInitStage::Load,
            "navigation.reload()",
        )
        .await
        .expect("load handler Navigation API reload should be surfaced")
    });
}

#[test]
fn load_stage_script_navigation_is_classified_before_load_milestone() {
    run_page_vm_local_runtime_async_test(
        "page-vm-load-stage-navigation-before-load",
        || async move {
            let mut page_vm = test_page_vm();
            let navigation_script = prepared_loaded_classic_for_page_vm_test(
                &page_vm,
                41,
                "location.href = 'https://replacement.test/before-load'",
            );
            let local_executor = page_vm.local_executor.clone();

            local_executor
                .run(async move {
                    let mut pending = None;
                    let document = match page_vm
                        .begin_post_parse_lifecycle_on_named_owner_lane(
                            &mut pending,
                            vec![classic_defer_work(navigation_script)],
                            PageVmInitStage::Load,
                            Instant::now(),
                        )
                        .await?
                    {
                        DocumentLifecycleTurnOutcome {
                            readiness: DocumentLifecycleTurnReadiness::Runnable { document },
                            ..
                        } => document,
                        outcome => panic!(
                            "load-stage script should install a runnable resident: {outcome:?}"
                        ),
                    };

                    for _ in 0..128 {
                        match page_vm
                            .advance_post_parse_lifecycle_one_owner_turn(&mut pending, document)
                            .await?
                        {
                            DocumentLifecycleTurnOutcome {
                                action:
                                    DocumentLifecycleTurnAction::RequestedTopLevelNavigation {
                                        source_document,
                                        stage: PageVmInitStage::Load,
                                        timing:
                                            DocumentLifecycleNavigationTiming::BeforeMilestone,
                                    },
                                readiness: DocumentLifecycleTurnReadiness::Idle,
                            } => {
                                assert_eq!(source_document, document);
                                assert!(page_vm.vm().has_pending_location_navigation());
                                assert!(pending.is_none());
                                assert!(matches!(
                                    page_vm.document_lifecycle_wait_outcome(
                                        RendererDocumentLifecycleMilestone::Load
                                    ),
                                    RendererDocumentLifecycleWaitOutcome::Pending
                                ));
                                return Ok::<_, anyhow::Error>(());
                            }
                            DocumentLifecycleTurnOutcome {
                                readiness: DocumentLifecycleTurnReadiness::Runnable { .. },
                                ..
                            } => {}
                            outcome => panic!(
                                "load-stage navigation should precede the load fact: {outcome:?}"
                            ),
                        }
                    }
                    panic!(
                        "load-stage navigation did not become runnable within the bounded lifecycle fixture"
                    )
                })
                .await
                .expect("load-stage navigation should remain ordered before load")
        },
    );
}

#[test]
fn load_stage_non_replacing_javascript_navigation_resumes_exact_lifecycle() {
    run_page_vm_local_runtime_async_test(
        "page-vm-load-stage-non-replacing-javascript-navigation",
        || async move {
            let mut page_vm = test_page_vm();
            let navigation_script = prepared_loaded_classic_for_page_vm_test(
                &page_vm,
                42,
                "globalThis.__javascriptNavigationRan = true; location.href = 'javascript:void 0'",
            );
            let local_executor = page_vm.local_executor.clone();

            local_executor
                .run(async move {
                    let mut pending = None;
                    let document = match page_vm
                        .begin_post_parse_lifecycle_on_named_owner_lane(
                            &mut pending,
                            vec![classic_defer_work(navigation_script)],
                            PageVmInitStage::Load,
                            Instant::now(),
                        )
                        .await?
                    {
                        DocumentLifecycleTurnOutcome {
                            readiness: DocumentLifecycleTurnReadiness::Runnable { document },
                            ..
                        } => document,
                        outcome => panic!(
                            "load-stage script should install a runnable resident: {outcome:?}"
                        ),
                    };

                    for _ in 0..128 {
                        let outcome = page_vm
                            .advance_post_parse_lifecycle_one_owner_turn(&mut pending, document)
                            .await?;
                        if !matches!(
                            outcome.action,
                            DocumentLifecycleTurnAction::RequestedTopLevelNavigation { .. }
                        ) {
                            assert!(matches!(
                                outcome.readiness,
                                DocumentLifecycleTurnReadiness::Runnable { .. }
                            ));
                            continue;
                        }

                        assert_eq!(
                            pending.as_ref().map(|resident| resident.document),
                            Some(document),
                            "a conditional javascript: navigation must suspend, not retire, the exact lifecycle resident"
                        );
                        let follow = page_vm
                            .follow_pending_location_navigation_one_turn_async(
                                &mut pending,
                                PageVmInitStage::Load,
                            )
                            .await?;
                        assert!(matches!(
                            follow,
                            crate::runtime::PageVmFollowNavigationTurnOutcome::PostParseLifecycle {
                                target_stage: PageVmInitStage::Load,
                                outcome: DocumentLifecycleTurnOutcome {
                                    action: DocumentLifecycleTurnAction::None,
                                    readiness:
                                        DocumentLifecycleTurnReadiness::Runnable {
                                            document: resumed_document,
                                        },
                                },
                            } if resumed_document == document
                        ));
                        assert_eq!(page_vm.document_lifecycle.identity(), document);
                        assert_eq!(
                            pending.as_ref().map(|resident| resident.document),
                            Some(document),
                            "a non-replacing javascript: result must restore the same exact lifecycle continuation"
                        );

                        for _ in 0..128 {
                            let resumed = page_vm
                                .advance_post_parse_lifecycle_one_owner_turn(
                                    &mut pending,
                                    document,
                                )
                                .await?;
                            if matches!(
                                resumed.action,
                                DocumentLifecycleTurnAction::ReachedStage(PageVmInitStage::Load)
                            ) {
                                assert!(pending.is_none());
                                assert_eq!(
                                    page_vm.vm_mut().eval(
                                        "String(globalThis.__javascriptNavigationRan === true)"
                                    )?,
                                    "true"
                                );
                                return Ok::<_, anyhow::Error>(());
                            }
                            assert!(matches!(
                                resumed.readiness,
                                DocumentLifecycleTurnReadiness::Runnable { .. }
                            ));
                        }
                        panic!("retained lifecycle did not reach Load after javascript: follow");
                    }
                    panic!("load-stage javascript: navigation was not surfaced")
                })
                .await
                .expect("non-replacing javascript: navigation should resume exact lifecycle")
        },
    );
}

#[test]
fn window_event_target_methods_are_inherited_and_bare_calls_still_work() {
    let mut page_vm = test_page_vm();
    let observed = page_vm
        .vm_mut()
        .eval(
            r#"
            (() => {
                const events = [];
                function bareListener(event) {
                    events.push(`bare:${event.type}:${this === window}`);
                }
                function windowListener(event) {
                    events.push(`window:${event.type}:${this === window}`);
                }
                addEventListener("lm-window-shape", bareListener);
                window.addEventListener("lm-window-shape", windowListener);
                dispatchEvent(new Event("lm-window-shape"));
                removeEventListener("lm-window-shape", bareListener);
                window.removeEventListener("lm-window-shape", windowListener);
                window.dispatchEvent(new Event("lm-window-shape"));
                let plainDispatchError = null;
                try {
                    window.dispatchEvent({ type: "plain" });
                } catch (error) {
                    plainDispatchError = `${error && error.name}:${error instanceof TypeError}`;
                }
                return JSON.stringify({
                    windowOwnAdd: Object.hasOwn(window, "addEventListener"),
                    windowOwnRemove: Object.hasOwn(window, "removeEventListener"),
                    windowOwnDispatch: Object.hasOwn(window, "dispatchEvent"),
                    windowPrototypeOwnAdd: Object.hasOwn(Window.prototype, "addEventListener"),
                    windowPrototypeOwnRemove: Object.hasOwn(Window.prototype, "removeEventListener"),
                    windowPrototypeOwnDispatch: Object.hasOwn(Window.prototype, "dispatchEvent"),
                    eventTargetPrototypeOwnAdd: Object.hasOwn(EventTarget.prototype, "addEventListener"),
                    inheritedName: window.addEventListener.name,
                    inheritedLength: window.addEventListener.length,
                    windowInstanceOfEventTarget: window instanceof EventTarget,
                    events,
                    plainDispatchError,
                });
            })()
            "#,
        )
        .expect("window EventTarget descriptor test should evaluate");

    assert_eq!(
        observed,
        r#"{"windowOwnAdd":false,"windowOwnRemove":false,"windowOwnDispatch":false,"windowPrototypeOwnAdd":false,"windowPrototypeOwnRemove":false,"windowPrototypeOwnDispatch":false,"eventTargetPrototypeOwnAdd":true,"inheritedName":"addEventListener","inheritedLength":2,"windowInstanceOfEventTarget":true,"events":["bare:lm-window-shape:true","window:lm-window-shape:true"],"plainDispatchError":"TypeError:true"}"#
    );
}

#[test]
fn host_owned_file_chooser_queue_retains_document_identity_and_drains() {
    let mut page_vm = test_page_vm();
    page_vm
        .vm_mut()
        .eval(
            r#"
document.body.innerHTML = '<input id="picker" type="file" multiple>';
"#,
        )
        .expect("install file input");
    page_vm
        .vm_mut()
        .eval("document.getElementById('picker').click(); 'done'")
        .expect("file input click should dispatch");

    let snapshot = page_vm
        .vm_mut()
        .page_diagnostics_snapshot()
        .expect("page diagnostics snapshot");
    assert_eq!(snapshot.diagnostics.pending_file_chooser_activations, 1);

    let activations = page_vm.vm_mut().take_pending_file_chooser_activations();
    assert_eq!(activations.len(), 1);
    assert_eq!(
        activations[0].source_document(),
        page_vm.document_lifecycle.identity(),
        "file chooser output should retain the exact producing Document"
    );
    assert_eq!(activations[0].source_frame_id(), None);
    let snapshot = page_vm
        .vm_mut()
        .page_diagnostics_snapshot()
        .expect("page diagnostics snapshot after drain");
    assert_eq!(snapshot.diagnostics.pending_file_chooser_activations, 0);
}

#[test]
fn page_diagnostics_snapshot_includes_page_lifecycle_error_count() {
    let mut page_vm = test_page_vm();
    page_vm
        .report
        .extend_observable_output(ScriptObservableOutput::from_items([
            ScriptObservableOutputItem::LifecycleError("script failure".to_owned()),
        ]));

    let snapshot = page_vm
        .page_diagnostics_snapshot()
        .expect("page diagnostics snapshot");

    assert_eq!(snapshot.diagnostics.runtime_lifecycle_errors, 1);
}

#[test]
fn page_diagnostics_snapshot_includes_runtime_observable_lifecycle_payload() {
    let mut page_vm = test_page_vm();
    page_vm
        .report
        .extend_observable_output(ScriptObservableOutput::from_items([
            ScriptObservableOutputItem::ConsoleMessage("console fallback".to_owned()),
            ScriptObservableOutputItem::LifecycleError("script failure".to_owned()),
        ]));

    let snapshot = page_vm
        .page_diagnostics_snapshot()
        .expect("page diagnostics snapshot");
    let source = snapshot
        .runtime_observable_source()
        .expect("runtime observable source should be present");

    assert_eq!(source.console_messages_with_context(), 0);
    assert_eq!(source.lifecycle_errors(), 1);
    assert_eq!(source.source_items().len(), 1);
    assert!(matches!(
        &source.source_items()[0],
        RendererRuntimeObservableSourceItem::LifecycleError { text, .. }
            if text == "script failure"
    ));
}

#[test]
fn page_diagnostics_snapshot_uses_cached_runtime_observable_console_source_items() {
    let mut page_vm = test_page_vm();
    page_vm
        .vm_mut()
        .dispatch_inspector_protocol_message(r#"{"id":1,"method":"Runtime.enable"}"#)
        .expect("enable runtime");
    page_vm
        .vm_mut()
        .eval("console.log('first'); 'done'")
        .expect("first console log");

    let snapshot = page_vm
        .page_diagnostics_snapshot()
        .expect("first page diagnostics snapshot");
    let source = snapshot
        .runtime_observable_source()
        .expect("runtime observable source should be present");
    assert_eq!(source.console_messages_with_context(), 1);
    assert_eq!(source.source_items().len(), 1);

    let snapshot = page_vm
        .page_diagnostics_snapshot()
        .expect("second page diagnostics snapshot");
    let source = snapshot
        .runtime_observable_source()
        .expect("runtime observable source should still be present");
    assert_eq!(source.console_messages_with_context(), 1);
    assert_eq!(
        source.source_items().len(),
        1,
        "repeated snapshots without new console messages must not duplicate source items"
    );

    page_vm
        .vm_mut()
        .eval("console.log('second'); 'done'")
        .expect("second console log");
    let snapshot = page_vm
        .page_diagnostics_snapshot()
        .expect("third page diagnostics snapshot");
    let source = snapshot
        .runtime_observable_source()
        .expect("runtime observable source should still be present");
    assert_eq!(source.console_messages_with_context(), 2);
    assert_eq!(source.source_items().len(), 2);
    assert!(matches!(
        &source.source_items()[1],
        RendererRuntimeObservableSourceItem::ConsoleMessage { message, .. }
            if message.message == "log: second"
    ));
}

#[test]
fn page_diagnostics_snapshot_observes_but_does_not_drain_lifecycle_state() {
    let mut page_vm = test_page_vm();
    let _ = page_vm.take_page_creation_artifacts();
    let document = page_vm.document_lifecycle.identity();
    assert_eq!(
        page_vm.document_lifecycle.begin_milestone_dispatch(
            document,
            RendererDocumentLifecycleMilestone::DomContentLoaded,
        ),
        crate::runtime::RendererDocumentLifecycleTransition::DispatchStarted,
    );
    assert!(matches!(
        page_vm.document_lifecycle.complete_milestone_dispatch(
            document,
            RendererDocumentLifecycleMilestone::DomContentLoaded,
        ),
        crate::runtime::RendererDocumentLifecycleTransition::Recorded(_),
    ));

    let snapshot = page_vm
        .page_diagnostics_snapshot()
        .expect("read-only page diagnostics snapshot");
    assert_eq!(snapshot.document_lifecycle_identity(), Some(document));
    assert_eq!(
        page_vm.drain_document_lifecycle_events().len(),
        1,
        "the authorized projection should still own the one-shot drain"
    );
}

#[tokio::test]
async fn javascript_location_navigation_executes_when_pending_navigation_is_followed() {
    run_page_vm_async_test(async move {
        let page_vm = test_page_vm_with_document_url(
            Url::parse("https://javascript-location.test/start.html").unwrap(),
        );
        let local_executor = page_vm.local_executor.clone();

        let (outcome_is_completed, log, href, network_records) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                let mut pending_document_lifecycle_turn = None;
                page_vm.vm_mut().eval(
                    r#"
globalThis.__events = [];
location.href = "javascript:globalThis.__events.push('javascript URL')";
globalThis.__events.push('after setter');
"done"
"#,
                )?;
                assert!(
                    page_vm.vm().has_pending_location_navigation(),
                    "javascript: location should remain pending until the owner follows it"
                );

                let outcome = page_vm
                    .follow_pending_location_navigation_one_turn_async(
                        &mut pending_document_lifecycle_turn,
                        PageVmInitStage::Load,
                    )
                    .await?;
                let log = page_vm
                    .vm_mut()
                    .eval("JSON.stringify(globalThis.__events)")?;
                let href = page_vm.vm_mut().eval("location.href")?;
                let (network_records, _, _) =
                    split_network_output_items(page_vm.vm_mut().take_network_output());
                Ok::<_, anyhow::Error>((
                    matches!(
                        outcome,
                        crate::runtime::PageVmFollowNavigationTurnOutcome::Completed
                    ),
                    log,
                    href,
                    network_records.len(),
                ))
            })
            .await
            .expect("javascript location navigation should execute");

        assert!(outcome_is_completed);
        assert_eq!(log, r#"["after setter","javascript URL"]"#);
        assert_eq!(href, "https://javascript-location.test/start.html");
        assert_eq!(
            network_records, 0,
            "javascript: location navigation must not be fetched as a network URL"
        );
    })
    .await;
}

#[tokio::test]
async fn javascript_string_completion_restarts_renderer_lifecycle_on_same_document_token() {
    run_page_vm_async_test(async move {
        let page_vm = test_page_vm_with_document_url(
            Url::parse("https://javascript-replacement.test/start.html").unwrap(),
        );
        let local_executor = page_vm.local_executor.clone();

        let (initial, events, body_text, document_input_stream_opened) = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                let mut pending_document_lifecycle_turn = None;
                let initial = page_vm.take_page_creation_artifacts();
                let retired_document = match page_vm
                    .begin_post_parse_lifecycle_on_named_owner_lane(
                        &mut pending_document_lifecycle_turn,
                        Vec::new(),
                        PageVmInitStage::Load,
                        Instant::now(),
                    )
                    .await?
                {
                    DocumentLifecycleTurnOutcome {
                        readiness: DocumentLifecycleTurnReadiness::Runnable { document },
                        ..
                    } => document,
                    _ => panic!("initial document should install one lifecycle continuation"),
                };
                page_vm
                    .vm_mut()
                    .eval(r#"location.href = "javascript:%22replacement-body%22"; "queued""#)?;
                let outcome = page_vm
                    .follow_pending_location_navigation_one_turn_async(
                        &mut pending_document_lifecycle_turn,
                        PageVmInitStage::Load,
                    )
                    .await?;
                let replacement_document = page_vm.document_lifecycle.identity();
                assert!(matches!(
                    outcome,
                    crate::runtime::PageVmFollowNavigationTurnOutcome::PostParseLifecycle {
                        target_stage: PageVmInitStage::Load,
                        outcome: DocumentLifecycleTurnOutcome {
                            action: DocumentLifecycleTurnAction::DocumentReplaced {
                                previous,
                                current,
                            },
                            readiness: DocumentLifecycleTurnReadiness::Runnable {
                                document,
                            },
                        },
                    } if previous == retired_document
                        && current == replacement_document
                        && document == replacement_document
                ));
                assert_ne!(
                    replacement_document,
                    retired_document,
                    "javascript string completion must install a new lifecycle epoch"
                );
                assert_eq!(
                    pending_document_lifecycle_turn
                        .as_ref()
                        .map(|pending| pending.document),
                    Some(replacement_document),
                    "the explicit replacement boundary must retire D1 and install an exact D2 continuation"
                );
                let document_input_stream_opened = page_vm
                    .page_diagnostics_snapshot()?
                    .document_input_stream_opened();
                let events = page_vm.drain_document_lifecycle_events();
                let body_text = page_vm.vm_mut().eval("document.body.textContent")?;
                Ok::<_, anyhow::Error>((initial, events, body_text, document_input_stream_opened))
            })
            .await
            .expect("javascript string completion should replace the document");

        assert_eq!(body_text, "replacement-body");
        assert!(document_input_stream_opened);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].document, initial.active_document);
        assert_eq!(events[1].document, initial.active_document);
        assert_eq!(events[1].epoch.0, initial.active_epoch.0 + 1);
        assert!(matches!(
            events[0].kind,
            RendererDocumentLifecycleEventKind::Terminated {
                reason: RendererDocumentTerminationReason::ReplacedByJavascriptResult,
                ..
            }
        ));
        assert!(matches!(
            events[1].kind,
            RendererDocumentLifecycleEventKind::Started {
                reason: RendererLifecycleStartReason::JavascriptDocumentReplacement,
            }
        ));
    })
    .await;
}

#[test]
fn main_document_lifecycle_coordinator_preserves_applied_callback_facts() {
    run_page_vm_local_runtime_async_test("page-vm-typed-main-lifecycle-effect", || async move {
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                let _initial = page_vm.take_page_creation_artifacts();
                let owner = page_vm
                    .vm()
                    .current_main_document_task_owner()
                    .expect("typed lifecycle test document owner");
                let interactive = page_vm
                    .vm_mut()
                    .finish_current_main_document_parsing(owner)
                    .expect("parser completion should prepare interactive work");
                let run = execute_main_document_lifecycle_on_owner_local_task(
                    &mut page_vm,
                    MainDocumentLifecycleBody::Interactive(interactive),
                )
                .await?;
                let execution = run.completion;
                assert_eq!(execution.kind(), MainDocumentLifecycleBodyKind::Interactive);
                assert_eq!(execution.owner(), owner);
                assert!(matches!(
                    execution.target(),
                    MainDocumentLifecycleTargetEffect::Applied {
                        current_owner_after_execution: Some(current_owner),
                    } if current_owner == owner
                ));
                assert_eq!(
                    execution.callback(),
                    MainDocumentLifecycleCallbackEffect::InteractiveReadystatechangeAttempted
                );
                assert!(matches!(
                    execution.into_followup(),
                    MainDocumentLifecycleFollowup::None
                ));

                let run = execute_main_document_lifecycle_on_owner_local_task(
                    &mut page_vm,
                    MainDocumentLifecycleBody::DomContentLoaded { owner },
                )
                .await?;
                let execution = run.completion;
                assert_eq!(
                    execution.kind(),
                    MainDocumentLifecycleBodyKind::DomContentLoaded
                );
                assert!(matches!(
                    execution.target(),
                    MainDocumentLifecycleTargetEffect::Applied {
                        current_owner_after_execution: Some(current_owner),
                    } if current_owner == owner
                ));
                assert_eq!(
                    execution.callback(),
                    MainDocumentLifecycleCallbackEffect::DomContentLoadedAttempted
                );
                assert!(matches!(
                    execution.into_followup(),
                    MainDocumentLifecycleFollowup::None
                ));

                let run = execute_main_document_lifecycle_on_owner_local_task(
                    &mut page_vm,
                    MainDocumentLifecycleBody::WindowLoad { owner },
                )
                .await?;
                let execution = run.completion;
                assert_eq!(execution.kind(), MainDocumentLifecycleBodyKind::WindowLoad);
                assert!(matches!(
                    execution.target(),
                    MainDocumentLifecycleTargetEffect::Applied {
                        current_owner_after_execution: Some(current_owner),
                    } if current_owner == owner
                ));
                assert_eq!(
                    execution.callback(),
                    MainDocumentLifecycleCallbackEffect::WindowLoadCompoundAttempted
                );
                assert!(matches!(
                    execution.into_followup(),
                    MainDocumentLifecycleFollowup::None
                ));
                Ok::<_, anyhow::Error>(())
            })
            .await
            .expect("typed main lifecycle effect should reconcile");
    });
}

#[test]
fn ordinary_main_lifecycle_body_leaves_listener_reaction_for_its_typed_checkpoint() {
    run_page_vm_local_runtime_async_test(
        "page-vm-main-lifecycle-body-only-checkpoint",
        || async move {
            let mut page_vm = test_page_vm();
            let local_executor = page_vm.local_executor.clone();

            local_executor
                .run(async move {
                    let _initial = page_vm.take_page_creation_artifacts();
                    page_vm.vm_mut().eval(
                        r#"
globalThis.__mainLifecycleBodyBoundary = [];
document.addEventListener("readystatechange", () => {
  if (document.readyState !== "interactive") return;
  __mainLifecycleBodyBoundary.push("callback");
  Promise.resolve().then(() => __mainLifecycleBodyBoundary.push("microtask"));
});
"installed"
"#,
                    )?;
                    let owner = page_vm
                        .vm()
                        .current_main_document_task_owner()
                        .expect("body-only lifecycle owner");
                    let action = page_vm
                        .vm_mut()
                        .finish_current_main_document_parsing(owner)
                        .expect("parser completion should prepare interactive work");

                    let step = page_vm.vm_mut().begin_main_document_lifecycle_body(
                        crate::script_vm::MainDocumentLifecycleBody::Interactive(action),
                    );
                    assert!(matches!(step, MainDocumentLifecycleStep::Checkpoint(_)));
                    assert_eq!(
                        page_vm
                            .vm_mut()
                            .eval_without_microtask_checkpoint_for_test(
                                "__mainLifecycleBodyBoundary.join('|')"
                            )?,
                        "callback",
                        "the lifecycle body must not perform its typed task-end checkpoint"
                    );
                    Ok::<_, anyhow::Error>(())
                })
                .await
                .expect("body-only main lifecycle witness should run");
        },
    );
}

#[test]
fn ordinary_main_lifecycle_coordinator_does_not_run_a_compatibility_pre_task_checkpoint() {
    run_page_vm_local_runtime_async_test(
        "page-vm-main-lifecycle-no-pre-task-checkpoint",
        || async move {
            let mut page_vm = test_page_vm();
            let local_executor = page_vm.local_executor.clone();

            local_executor
                .run(async move {
                    let _initial = page_vm.take_page_creation_artifacts();
                    page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
                        r#"
globalThis.__mainLifecycleCheckpointOrder = [];
Promise.resolve().then(() => __mainLifecycleCheckpointOrder.push("preexisting"));
document.addEventListener("readystatechange", () => {
  if (document.readyState !== "interactive") return;
  __mainLifecycleCheckpointOrder.push("callback");
  Promise.resolve().then(() => __mainLifecycleCheckpointOrder.push("callback:microtask"));
});
"installed"
"#,
                    )?;
                    let owner = page_vm
                        .vm()
                        .current_main_document_task_owner()
                        .expect("ordinary lifecycle owner");
                    let action = page_vm
                        .vm_mut()
                        .finish_current_main_document_parsing(owner)
                        .expect("parser completion should prepare interactive work");
                    execute_main_document_lifecycle_on_owner_local_task(
                        &mut page_vm,
                        MainDocumentLifecycleBody::Interactive(action),
                    )
                    .await?;

                    assert_eq!(
                        page_vm
                            .vm_mut()
                            .eval_without_microtask_checkpoint_for_test(
                                "__mainLifecycleCheckpointOrder.join('|')"
                            )?,
                        "callback|preexisting|callback:microtask",
                        "ordinary lifecycle must dispatch its body before the one typed task-end checkpoint"
                    );
                    Ok::<_, anyhow::Error>(())
                })
                .await
                .expect("ordinary lifecycle checkpoint ownership witness should run");
        },
    );
}

#[test]
fn parse_time_exact_dcl_separates_parser_terminal_and_listener_checkpoints() {
    run_page_vm_local_runtime_async_test(
        "page-vm-parse-time-exact-dcl-separate-checkpoints",
        || async move {
            let mut page_vm = test_page_vm();
            let local_executor = page_vm.local_executor.clone();

            local_executor
                .run(async move {
                    let _initial = page_vm.take_page_creation_artifacts();
                    page_vm.vm_mut().eval(
                        r#"
globalThis.__parseTimeExactDclOrder = [];
document.addEventListener("DOMContentLoaded", () => {
  __parseTimeExactDclOrder.push("dcl");
  Promise.resolve().then(() => __parseTimeExactDclOrder.push("dcl:microtask"));
});
"installed"
"#,
                    )?;
                    let owner = prepare_parse_time_exact_domcontentloaded(&mut page_vm).await?;
                    page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
                        r#"
Promise.resolve().then(() => __parseTimeExactDclOrder.push("module-terminal:microtask"));
"scheduled"
"#,
                    )?;

                    let outcome = page_vm
                        .finish_parse_time_main_parser_boundary(
                            MainParserContinuationCompletion::drained_for_test(
                                owner,
                                MainParserContinuationTaskEffect::callback_for_test(owner),
                            ),
                        )
                        .await?;
                    assert_eq!(
                        outcome,
                        ParseTimeMainParserBoundaryOutcome::CurrentDocumentRetained
                    );
                    assert_eq!(
                        page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
                            "__parseTimeExactDclOrder.join('|')"
                        )?,
                        "module-terminal:microtask|dcl|dcl:microtask",
                        "the parser task-end must settle its terminal reactions before the exact DCL successor runs its own task and checkpoint"
                    );
                    Ok::<_, anyhow::Error>(())
                })
                .await
                .expect("parse-time exact DCL checkpoint boundary should run");
        },
    );
}

#[test]
fn parse_time_parser_terminal_reaction_replaces_document_before_exact_dcl() {
    run_page_vm_local_runtime_async_test(
        "page-vm-parse-time-parser-terminal-replaces-before-exact-dcl",
        || async move {
            let mut page_vm = test_page_vm();
            let local_executor = page_vm.local_executor.clone();

            local_executor
                .run(async move {
                    let _initial = page_vm.take_page_creation_artifacts();
                    page_vm.vm_mut().eval(
                        r#"
globalThis.__oldDocumentDclDispatched = false;
document.addEventListener("DOMContentLoaded", () => {
  globalThis.__oldDocumentDclDispatched = true;
});
"installed"
"#,
                    )?;
                    let owner = prepare_parse_time_exact_domcontentloaded(&mut page_vm).await?;
                    page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
                        r#"
Promise.resolve().then(() => {
  document.open();
  document.write("<main id='parser-terminal-replacement'>replacement</main>");
  document.close();
});
"scheduled"
"#,
                    )?;

                    let outcome = page_vm
                        .finish_parse_time_main_parser_boundary(
                            MainParserContinuationCompletion::drained_for_test(
                                owner,
                                MainParserContinuationTaskEffect::callback_for_test(owner),
                            ),
                        )
                        .await?;
                    assert_eq!(
                        outcome,
                        ParseTimeMainParserBoundaryOutcome::DocumentReplaced,
                        "the parser task-end checkpoint must report replacement before attempting the old exact DCL"
                    );
                    assert_ne!(
                        page_vm.vm().current_main_document_task_owner(),
                        Some(owner)
                    );
                    assert_eq!(
                        page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
                            "String(globalThis.__oldDocumentDclDispatched)"
                        )?,
                        "false",
                        "the claimed DCL for the retired Document must not dispatch after replacement"
                    );
                    assert_eq!(
                        page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
                            "String(document.getElementById('parser-terminal-replacement') !== null)"
                        )?,
                        "true"
                    );
                    Ok::<_, anyhow::Error>(())
                })
                .await
                .expect("parser terminal replacement should cancel old exact DCL");
        },
    );
}

#[test]
fn parse_time_parser_finish_without_dcl_settles_only_existing_task_end_debt() {
    run_page_vm_local_runtime_async_test(
        "page-vm-parse-time-parser-finish-without-dcl",
        || async move {
            for checkpoint_expected in [false, true] {
                let mut page_vm = test_page_vm();
                let local_executor = page_vm.local_executor.clone();
                local_executor
                    .run(async move {
                        let _initial = page_vm.take_page_creation_artifacts();
                        let owner = page_vm
                            .vm()
                            .current_main_document_task_owner()
                            .expect("parser task-end fixture requires a current owner");
                        let task_effect = if checkpoint_expected {
                            MainParserContinuationTaskEffect::checkpoint_only_for_test(owner)
                        } else {
                            MainParserContinuationTaskEffect::NotApplied
                        };
                        page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
                            r#"
globalThis.__parserFinishWithoutDcl = "pending";
Promise.resolve().then(() => {
  globalThis.__parserFinishWithoutDcl = "checkpointed";
});
"scheduled"
"#,
                        )?;

                        let outcome = page_vm
                            .finish_parse_time_main_parser_boundary(
                                if checkpoint_expected {
                                    MainParserContinuationCompletion::drained_for_test(
                                        owner,
                                        task_effect,
                                    )
                                } else {
                                    MainParserContinuationCompletion::pending_for_test(task_effect)
                                },
                            )
                            .await?;
                        assert_eq!(
                            outcome,
                            ParseTimeMainParserBoundaryOutcome::CurrentDocumentRetained
                        );
                        let observed = page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
                            "globalThis.__parserFinishWithoutDcl"
                        )?;
                        if checkpoint_expected {
                            assert_eq!(
                                observed, "checkpointed",
                                "an applied parser continuation must discharge its task-end checkpoint even when no exact DCL was claimed"
                            );
                        } else {
                            assert_eq!(
                                observed, "pending",
                                "a stale parser claim must not manufacture a checkpoint when no exact DCL was claimed"
                            );
                        }
                        Ok::<_, anyhow::Error>(())
                    })
                    .await
                    .expect("parse-time no-DCL task-end fixture should run");
            }
        },
    );
}

#[test]
fn parse_time_exact_dcl_reaction_reports_document_replacement_to_phase_one() {
    run_page_vm_local_runtime_async_test(
        "page-vm-parse-time-exact-dcl-replacement",
        || async move {
            let mut page_vm = test_page_vm();
            let local_executor = page_vm.local_executor.clone();

            local_executor
                .run(async move {
                    let _initial = page_vm.take_page_creation_artifacts();
                    page_vm.vm_mut().eval(
                        r#"
document.addEventListener("DOMContentLoaded", () => {
  Promise.resolve().then(() => {
    document.open();
    document.write("<main id='parse-time-replacement'>replacement</main>");
    document.close();
  });
}, { once: true });
"installed"
"#,
                    )?;
                    let owner = prepare_parse_time_exact_domcontentloaded(&mut page_vm).await?;

                    let outcome = page_vm
                        .finish_parse_time_main_parser_boundary(
                            MainParserContinuationCompletion::drained_for_test(
                                owner,
                                MainParserContinuationTaskEffect::checkpoint_only_for_test(owner),
                            ),
                        )
                        .await?;
                    assert_eq!(
                        outcome,
                        ParseTimeMainParserBoundaryOutcome::DocumentReplaced,
                        "phase one must stop the retired Document after an exact-DCL reaction replaces it"
                    );
                    assert_ne!(
                        page_vm.vm().current_main_document_task_owner(),
                        Some(owner)
                    );
                    assert_eq!(
                        page_vm.vm_mut().eval(
                            "String(document.getElementById('parse-time-replacement') !== null)"
                        )?,
                        "true"
                    );
                    Ok::<_, anyhow::Error>(())
                })
                .await
                .expect("parse-time DCL replacement should reconcile");
        },
    );
}

#[test]
fn pending_cross_document_navigation_cancels_queued_dcl_before_dispatch() {
    run_page_vm_local_runtime_async_test("page-vm-pending-navigation-before-dcl", || async move {
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        let (ready_state, lifecycle_events) = local_executor
            .run(async move {
                let _initial = page_vm.take_page_creation_artifacts();
                page_vm
                    .vm_mut()
                    .eval("location.href = 'https://replacement.test/'; 'queued'")?;
                assert!(page_vm.vm().has_pending_location_navigation());

                let owner = page_vm
                    .vm()
                    .current_main_document_task_owner()
                    .expect("main lifecycle test document owner");
                let run = execute_main_document_lifecycle_on_owner_local_task(
                    &mut page_vm,
                    MainDocumentLifecycleBody::DomContentLoaded { owner },
                )
                .await?;
                let execution = run.completion;
                assert_eq!(
                    execution.kind(),
                    MainDocumentLifecycleBodyKind::DomContentLoaded
                );
                assert_eq!(execution.owner(), owner);
                assert!(matches!(
                    execution.target(),
                    MainDocumentLifecycleTargetEffect::NotApplied {
                        reason,
                        current_owner: Some(current_owner),
                    } if reason.is_pending_cross_document_navigation()
                        && current_owner == owner
                ));
                assert_eq!(
                    execution.callback(),
                    MainDocumentLifecycleCallbackEffect::NotEntered
                );
                assert!(matches!(
                    execution.into_followup(),
                    MainDocumentLifecycleFollowup::None
                ));

                Ok::<_, anyhow::Error>((
                    page_vm
                        .vm()
                        .document_runtime
                        .host_document()
                        .ready_state()
                        .to_owned(),
                    page_vm.drain_document_lifecycle_events(),
                ))
            })
            .await
            .expect("queued DCL cancellation should complete");

        assert_eq!(ready_state, crate::dom::native::DocumentReadyState::Loading);
        assert_eq!(lifecycle_events.len(), 1);
        assert!(matches!(
            lifecycle_events[0].kind,
            RendererDocumentLifecycleEventKind::Terminated {
                last_reached: None,
                reason: RendererDocumentTerminationReason::SupersededByCrossDocumentNavigation,
            }
        ));
    });
}

#[test]
fn stale_main_lifecycle_task_neither_checkpoints_nor_starts_replacement_milestone() {
    run_page_vm_local_runtime_async_test(
        "page-vm-stale-main-lifecycle-no-replacement-checkpoint",
        || async move {
            let mut page_vm = test_page_vm();
            let local_executor = page_vm.local_executor.clone();

            local_executor
                .run(async move {
                    let _initial = page_vm.take_page_creation_artifacts();
                    let stale_owner = page_vm
                        .vm()
                        .current_main_document_task_owner()
                        .expect("initial lifecycle owner");
                    let replacement_snapshot =
                        page_vm.document_replacement_lifecycle_action_snapshot();
                    page_vm.vm_mut().eval(
                        "document.open(); document.write('<main>replacement</main>'); document.close()",
                    )?;
                    page_vm
                        .take_document_replacement_lifecycle_admission_after_action(
                            replacement_snapshot,
                        )?
                        .expect("document.close should publish replacement lifecycle admission");
                    let current_owner = page_vm
                        .vm()
                        .current_main_document_task_owner()
                        .expect("replacement lifecycle owner");
                    assert_ne!(current_owner, stale_owner);

                    page_vm.vm_mut().eval_without_microtask_checkpoint_for_test(
                        r#"
globalThis.__staleLifecycleCheckpoint = "pending";
Promise.resolve().then(() => {
  globalThis.__staleLifecycleCheckpoint = "ran";
});
"scheduled"
"#,
                    )?;
                    let run = execute_main_document_lifecycle_on_owner_local_task(
                        &mut page_vm,
                        MainDocumentLifecycleBody::DomContentLoaded {
                            owner: stale_owner,
                        },
                    )
                    .await?;
                    let execution = run.completion;
                    assert!(matches!(
                        execution.target(),
                        MainDocumentLifecycleTargetEffect::NotApplied {
                            reason: MainDocumentLifecycleTargetRejection::TransitionRejected,
                            current_owner: Some(observed),
                        } if observed == current_owner
                    ));
                    assert_eq!(
                        execution.callback(),
                        MainDocumentLifecycleCallbackEffect::NotEntered
                    );
                    assert_eq!(
                        page_vm
                            .vm_mut()
                            .eval_without_microtask_checkpoint_for_test(
                                "globalThis.__staleLifecycleCheckpoint"
                            )?,
                        "pending",
                        "a stale lifecycle task must not checkpoint the replacement realm"
                    );
                    assert!(matches!(
                        page_vm.document_lifecycle_wait_outcome(
                            RendererDocumentLifecycleMilestone::DomContentLoaded,
                        ),
                        RendererDocumentLifecycleWaitOutcome::Pending
                    ));
                    Ok::<_, anyhow::Error>(())
                })
                .await
                .expect("stale lifecycle task should preserve replacement authority");
        },
    );
}

#[test]
fn document_open_snapshot_marks_input_stream_open_before_close() {
    let mut page_vm = test_page_vm();
    page_vm
        .vm_mut()
        .eval("document.open(); 'opened'")
        .expect("document.open should evaluate without closing the input stream");

    let snapshot = page_vm
        .page_diagnostics_snapshot()
        .expect("document.open state should be visible in the activity snapshot");
    let open_document = page_vm.document_lifecycle.identity();

    assert!(snapshot.document_input_stream_opened());
    assert!(
        page_vm.has_blocked_document_replacement_lifecycle_admission(open_document),
        "an open input stream must remain blocked until document.close"
    );
}

#[test]
fn document_open_close_restarts_renderer_lifecycle_on_same_document_token() {
    let mut page_vm = test_page_vm();
    let initial = page_vm.take_page_creation_artifacts();
    let replacement_lifecycle_snapshot = page_vm.document_replacement_lifecycle_action_snapshot();
    page_vm
        .vm_mut()
        .eval(
            r#"document.open(); document.write('<main>replacement</main>'); document.close(); 'done'"#,
        )
        .expect("document.open replacement should evaluate");
    assert!(
        page_vm
            .ready_document_replacement_lifecycle_admission()
            .is_some(),
        "document.close should hand a typed replacement admission to the lifecycle owner"
    );
    page_vm
        .take_document_replacement_lifecycle_admission_after_action(replacement_lifecycle_snapshot)
        .expect("the exact owner action should settle its replacement admission")
        .expect("document.close should produce a replacement admission");
    let snapshot = page_vm
        .page_diagnostics_snapshot()
        .expect("the settled replacement lifecycle should be visible in the activity snapshot");
    assert!(snapshot.document_input_stream_opened());
    assert!(
        page_vm
            .ready_document_replacement_lifecycle_admission()
            .is_none(),
        "the lifecycle journal should expose no pending admission after exact activation"
    );
    assert!(matches!(
        page_vm.document_lifecycle_wait_outcome(RendererDocumentLifecycleMilestone::Load),
        RendererDocumentLifecycleWaitOutcome::Pending
    ));
    let events = page_vm.drain_document_lifecycle_events();

    assert_eq!(events.len(), 2);
    assert_eq!(events[0].document, initial.active_document);
    assert_eq!(events[1].document, initial.active_document);
    assert_eq!(events[1].epoch.0, initial.active_epoch.0 + 1);
    assert!(matches!(
        events[0].kind,
        RendererDocumentLifecycleEventKind::Terminated {
            reason: RendererDocumentTerminationReason::RestartedByDocumentOpen,
            ..
        }
    ));
    assert!(matches!(
        events[1].kind,
        RendererDocumentLifecycleEventKind::Started {
            reason: RendererLifecycleStartReason::ExplicitDocumentOpen,
        }
    ));
}

#[test]
fn document_open_and_later_close_publish_admission_from_the_close_action() {
    let mut page_vm = test_page_vm();
    let _ = page_vm.take_page_creation_artifacts();

    let open_snapshot = page_vm.document_replacement_lifecycle_action_snapshot();
    page_vm
        .vm_mut()
        .eval("document.open(); 'opened'")
        .expect("document.open should start a replacement input stream");
    assert!(
        page_vm
            .take_document_replacement_lifecycle_admission_after_action(open_snapshot)
            .expect("document.open settlement should preserve lifecycle invariants")
            .is_none(),
        "document.open alone must not make the replacement lifecycle runnable"
    );
    let open_document = page_vm.document_lifecycle.identity();
    assert!(
        page_vm.has_blocked_document_replacement_lifecycle_admission(open_document),
        "the exact replacement admission must remain durable while its input stream awaits close"
    );

    let close_snapshot = page_vm.document_replacement_lifecycle_action_snapshot();
    page_vm
        .vm_mut()
        .eval("document.write('<main>replacement</main>'); document.close(); 'closed'")
        .expect("a later document.close should finish the replacement input stream");
    let admission = page_vm
        .take_document_replacement_lifecycle_admission_after_action(close_snapshot)
        .expect("document.close settlement should preserve lifecycle invariants")
        .expect("document.close should publish the replacement lifecycle admission");
    assert_eq!(admission.to, page_vm.document_lifecycle.identity());
    assert!(
        !page_vm.has_blocked_document_replacement_lifecycle_admission(open_document),
        "settlement must transition the admission from blocked to active"
    );
    assert!(page_vm.repeated_document_lifecycle_load_is_pending());
}

#[test]
fn later_admission_only_snapshot_cannot_consume_an_older_replacement() {
    let mut page_vm = test_page_vm();
    let _ = page_vm.take_page_creation_artifacts();

    let producing_snapshot = page_vm.document_replacement_lifecycle_action_snapshot();
    page_vm
        .vm_mut()
        .eval("document.open(); document.write('<main>replacement</main>'); document.close()")
        .expect("document replacement should complete");
    let admission_only_snapshot = page_vm.document_replacement_lifecycle_action_snapshot();
    assert!(
        page_vm
            .take_document_replacement_lifecycle_admission_after_action(admission_only_snapshot)
            .expect("an unrelated action snapshot should remain a no-op")
            .is_none(),
        "a later action must not repair an admission it did not produce"
    );
    assert!(
        page_vm
            .ready_document_replacement_lifecycle_admission()
            .is_some(),
        "the exact producing action must retain sole settlement authority"
    );
    page_vm
        .take_document_replacement_lifecycle_admission_after_action(producing_snapshot)
        .expect("the producing action should settle its exact admission")
        .expect("the producing action should still own the admission");
}

#[test]
fn stale_post_parse_owner_turn_neither_advances_nor_removes_replacement_continuation() {
    run_page_vm_local_runtime_async_test(
        "page-vm-stale-post-parse-document-owner",
        || async move {
            let mut page_vm = test_page_vm();
            let local_executor = page_vm.local_executor.clone();

            local_executor
                .run(async move {
                    let mut pending_document_lifecycle_turn = None;
                    let initial_document = match page_vm
                        .begin_post_parse_lifecycle_on_named_owner_lane(
                            &mut pending_document_lifecycle_turn,
                            Vec::new(),
                            PageVmInitStage::Load,
                            Instant::now(),
                        )
                        .await?
                    {
                        DocumentLifecycleTurnOutcome {
                            readiness: DocumentLifecycleTurnReadiness::Runnable { document },
                            ..
                        } => document,
                        _ => panic!("initial lifecycle should publish an exact continuation"),
                    };

                    page_vm.vm_mut().eval(
                        "document.open(); document.write('<main>replacement</main>'); document.close();",
                    )?;
                    let replacement_snapshot =
                        page_vm.document_lifecycle.current_snapshot();
                    assert_ne!(
                        replacement_snapshot.epoch, initial_document.epoch,
                        "document.open must rotate the exact lifecycle owner"
                    );

                    assert!(matches!(
                        page_vm
                            .advance_post_parse_lifecycle_one_owner_turn(
                                &mut pending_document_lifecycle_turn,
                                initial_document,
                            )
                            .await?,
                        DocumentLifecycleTurnOutcome {
                            action: DocumentLifecycleTurnAction::None,
                            readiness: DocumentLifecycleTurnReadiness::Idle,
                        }
                    ));
                    assert_eq!(
                        page_vm.document_lifecycle.current_snapshot(),
                        replacement_snapshot,
                        "a stale resume must be an empty arbitration turn"
                    );
                    assert!(
                        pending_document_lifecycle_turn.is_none(),
                        "the old exact-Document continuation should be discarded"
                    );

                    let replacement_document = match page_vm
                        .begin_post_parse_lifecycle_on_named_owner_lane(
                            &mut pending_document_lifecycle_turn,
                            Vec::new(),
                            PageVmInitStage::Load,
                            Instant::now(),
                        )
                        .await?
                    {
                        DocumentLifecycleTurnOutcome {
                            readiness: DocumentLifecycleTurnReadiness::Runnable { document },
                            ..
                        } => document,
                        _ => panic!("replacement lifecycle should publish its own continuation"),
                    };
                    assert_eq!(replacement_document, replacement_snapshot.into());

                    assert!(matches!(
                        page_vm
                            .advance_post_parse_lifecycle_one_owner_turn(
                                &mut pending_document_lifecycle_turn,
                                initial_document,
                            )
                            .await?,
                        DocumentLifecycleTurnOutcome {
                            action: DocumentLifecycleTurnAction::None,
                            readiness: DocumentLifecycleTurnReadiness::Idle,
                        }
                    ));
                    assert_eq!(
                        pending_document_lifecycle_turn
                            .as_ref()
                            .map(|pending| pending.document),
                        Some(replacement_document),
                        "an old owner turn must not consume a newer exact-Document continuation"
                    );
                    assert_eq!(
                        page_vm.document_lifecycle.current_snapshot(),
                        replacement_snapshot,
                        "rejecting an old turn must not advance replacement lifecycle facts"
                    );
                    Ok::<_, anyhow::Error>(())
                })
                .await
                .expect("stale lifecycle ownership scenario should run");
        },
    );
}

#[test]
fn domcontentloaded_turn_transitions_same_exact_document_to_load_residence() {
    run_page_vm_local_runtime_async_test(
        "page-vm-dcl-to-load-resident-transition",
        || async move {
            let mut page_vm = test_page_vm();
            let local_executor = page_vm.local_executor.clone();

            local_executor
                .run(async move {
                    let mut pending_document_lifecycle_turn = None;
                    let document = match page_vm
                        .begin_post_parse_lifecycle_on_named_owner_lane(
                            &mut pending_document_lifecycle_turn,
                            Vec::new(),
                            PageVmInitStage::DomContentLoaded,
                            Instant::now(),
                        )
                        .await?
                    {
                        DocumentLifecycleTurnOutcome {
                            action: DocumentLifecycleTurnAction::None,
                            readiness: DocumentLifecycleTurnReadiness::Runnable { document },
                        } => document,
                        outcome => panic!(
                            "DCL lifecycle should install one exact runnable residence: {outcome:?}"
                        ),
                    };

                    let dcl_outcome = loop {
                        let outcome = page_vm
                            .advance_post_parse_lifecycle_one_owner_turn(
                                &mut pending_document_lifecycle_turn,
                                document,
                            )
                            .await?;
                        match outcome {
                            DocumentLifecycleTurnOutcome {
                                action: DocumentLifecycleTurnAction::ReachedStage(
                                    PageVmInitStage::DomContentLoaded,
                                ),
                                readiness:
                                    DocumentLifecycleTurnReadiness::Runnable {
                                        document: runnable_document,
                                    },
                            } => {
                                assert_eq!(runnable_document, document);
                                break outcome;
                            }
                            DocumentLifecycleTurnOutcome {
                                readiness:
                                    DocumentLifecycleTurnReadiness::Runnable {
                                        document: runnable_document,
                                    },
                                ..
                            } => assert_eq!(runnable_document, document),
                            outcome => panic!(
                                "unblocked DCL fixture should stay runnable until DCL: {outcome:?}"
                            ),
                        }
                    };

                    assert!(matches!(
                        dcl_outcome,
                        DocumentLifecycleTurnOutcome {
                            action: DocumentLifecycleTurnAction::ReachedStage(
                                PageVmInitStage::DomContentLoaded,
                            ),
                            readiness: DocumentLifecycleTurnReadiness::Runnable { .. },
                        }
                    ));
                    assert!(matches!(
                        page_vm.document_lifecycle_wait_outcome(
                            RendererDocumentLifecycleMilestone::DomContentLoaded,
                        ),
                        RendererDocumentLifecycleWaitOutcome::Reached(_)
                    ));
                    assert!(matches!(
                        page_vm.document_lifecycle_wait_outcome(
                            RendererDocumentLifecycleMilestone::Load,
                        ),
                        RendererDocumentLifecycleWaitOutcome::Pending
                    ));
                    let pending = pending_document_lifecycle_turn
                        .as_ref()
                        .expect("reaching DCL must retain the same exact Document for load");
                    assert_eq!(pending.document, document);
                    assert_eq!(pending.stage, PageVmInitStage::Load);

                    loop {
                        match page_vm
                            .advance_post_parse_lifecycle_one_owner_turn(
                                &mut pending_document_lifecycle_turn,
                                document,
                            )
                            .await?
                        {
                            DocumentLifecycleTurnOutcome {
                                action:
                                    DocumentLifecycleTurnAction::ReachedStage(
                                        PageVmInitStage::Load,
                                    ),
                                readiness: DocumentLifecycleTurnReadiness::Idle,
                            } => break,
                            DocumentLifecycleTurnOutcome {
                                readiness:
                                    DocumentLifecycleTurnReadiness::Runnable {
                                        document: runnable_document,
                                    },
                                ..
                            } => assert_eq!(runnable_document, document),
                            outcome => panic!(
                                "unblocked load fixture should stay runnable until load: {outcome:?}"
                            ),
                        }
                    }

                    assert!(pending_document_lifecycle_turn.is_none());
                    assert!(matches!(
                        page_vm.document_lifecycle_wait_outcome(
                            RendererDocumentLifecycleMilestone::Load,
                        ),
                        RendererDocumentLifecycleWaitOutcome::Reached(_)
                    ));
                    Ok::<_, anyhow::Error>(())
                })
                .await
                .expect("DCL-to-load resident transition should run");
        },
    );
}

#[test]
fn lifecycle_resident_publishes_milestones_after_listener_microtasks() {
    run_page_vm_local_runtime_async_test(
        "page-vm-lifecycle-milestone-microtask-order",
        || async move {
            let mut page_vm = test_page_vm();
            let local_executor = page_vm.local_executor.clone();

            local_executor
                .run(async move {
                    page_vm.vm_mut().eval(
                        r#"
globalThis.__mainLifecycleOrder = [];
const recordLifecycleStep = step => {
  __mainLifecycleOrder.push(step);
  Promise.resolve().then(() => __mainLifecycleOrder.push(`${step}:microtask`));
};
document.addEventListener("readystatechange", () => {
  recordLifecycleStep(`readystatechange:${document.readyState}`);
});
document.addEventListener("DOMContentLoaded", () => {
  recordLifecycleStep("domcontentloaded");
});
window.addEventListener("load", () => {
  recordLifecycleStep("load");
});
window.addEventListener("pageshow", () => {
  recordLifecycleStep("pageshow");
});
"installed"
"#,
                    )?;

                    let mut pending_document_lifecycle_turn = None;
                    let document = match page_vm
                        .begin_post_parse_lifecycle_on_named_owner_lane(
                            &mut pending_document_lifecycle_turn,
                            Vec::new(),
                            PageVmInitStage::DomContentLoaded,
                            Instant::now(),
                        )
                        .await?
                    {
                        DocumentLifecycleTurnOutcome {
                            readiness: DocumentLifecycleTurnReadiness::Runnable { document },
                            ..
                        } => document,
                        outcome => panic!(
                            "main lifecycle should install one exact runnable residence: {outcome:?}"
                        ),
                    };

                    let dcl_outcome = advance_unblocked_exact_lifecycle_to_stage(
                        &mut page_vm,
                        &mut pending_document_lifecycle_turn,
                        document,
                        PageVmInitStage::DomContentLoaded,
                    )
                    .await?;
                    assert!(matches!(
                        dcl_outcome,
                        DocumentLifecycleTurnOutcome {
                            readiness:
                                DocumentLifecycleTurnReadiness::Runnable {
                                    document: runnable_document,
                                },
                            ..
                        } if runnable_document == document
                    ));
                    assert_eq!(
                        page_vm.vm_mut().eval("__mainLifecycleOrder.join('|')")?,
                        "readystatechange:interactive|readystatechange:interactive:microtask|domcontentloaded|domcontentloaded:microtask",
                        "the DCL milestone must not become observable before interactive and DCL reactions"
                    );
                    assert!(matches!(
                        page_vm.document_lifecycle_wait_outcome(
                            RendererDocumentLifecycleMilestone::DomContentLoaded,
                        ),
                        RendererDocumentLifecycleWaitOutcome::Reached(_)
                    ));
                    assert!(matches!(
                        page_vm.document_lifecycle_wait_outcome(
                            RendererDocumentLifecycleMilestone::Load,
                        ),
                        RendererDocumentLifecycleWaitOutcome::Pending
                    ));

                    let load_outcome = advance_unblocked_exact_lifecycle_to_stage(
                        &mut page_vm,
                        &mut pending_document_lifecycle_turn,
                        document,
                        PageVmInitStage::Load,
                    )
                    .await?;
                    assert!(matches!(
                        load_outcome,
                        DocumentLifecycleTurnOutcome {
                            readiness: DocumentLifecycleTurnReadiness::Idle,
                            ..
                        }
                    ));
                    assert_eq!(
                        page_vm.vm_mut().eval("__mainLifecycleOrder.join('|')")?,
                        "readystatechange:interactive|readystatechange:interactive:microtask|domcontentloaded|domcontentloaded:microtask|readystatechange:complete|readystatechange:complete:microtask|load|load:microtask|pageshow|pageshow:microtask",
                        "the load milestone must become observable only after the complete, load, and pageshow reactions"
                    );
                    assert!(pending_document_lifecycle_turn.is_none());
                    assert!(matches!(
                        page_vm.document_lifecycle_wait_outcome(
                            RendererDocumentLifecycleMilestone::Load,
                        ),
                        RendererDocumentLifecycleWaitOutcome::Reached(_)
                    ));
                    Ok::<_, anyhow::Error>(())
                })
                .await
                .expect("main lifecycle milestone ordering should run");
        },
    );
}

#[test]
fn each_main_lifecycle_callback_publishes_exact_document_open_replacement() {
    run_page_vm_local_runtime_async_test(
        "page-vm-main-lifecycle-document-open-replacement",
        || async move {
            for (stage, listener_source) in [
                (
                    "interactive",
                    r#"
document.addEventListener("readystatechange", () => {
  if (document.readyState !== "interactive") return;
  document.open();
  document.write("<main>interactive replacement</main>");
  document.close();
});
"#,
                ),
                (
                    "DOMContentLoaded",
                    r#"
document.addEventListener("DOMContentLoaded", () => {
  document.open();
  document.write("<main>DCL replacement</main>");
  document.close();
}, { once: true });
"#,
                ),
                (
                    "complete",
                    r#"
document.addEventListener("readystatechange", () => {
  if (document.readyState !== "complete") return;
  document.open();
  document.write("<main>complete replacement</main>");
  document.close();
});
"#,
                ),
                (
                    "load",
                    r#"
window.addEventListener("load", () => {
  document.open();
  document.write("<main>load replacement</main>");
  document.close();
}, { once: true });
"#,
                ),
                (
                    "pageshow",
                    r#"
window.addEventListener("pageshow", () => {
  document.open();
  document.write("<main>pageshow replacement</main>");
  document.close();
}, { once: true });
"#,
                ),
            ] {
                exact_lifecycle_turn_publishes_document_open_replacement(listener_source)
                    .await
                    .unwrap_or_else(|error| {
                        panic!("{stage} replacement scenario should run: {error:#}")
                    });
            }
        },
    );
}

#[test]
fn each_main_lifecycle_reaction_publishes_exact_document_open_replacement() {
    run_page_vm_local_runtime_async_test(
        "page-vm-main-lifecycle-reaction-document-open-replacement",
        || async move {
            for (stage, listener_source) in [
                (
                    "interactive reaction",
                    r#"
document.addEventListener("readystatechange", () => {
  if (document.readyState !== "interactive") return;
  Promise.resolve().then(() => {
    document.open();
    document.write("<main>interactive reaction replacement</main>");
    document.close();
  });
});
"#,
                ),
                (
                    "DOMContentLoaded reaction",
                    r#"
document.addEventListener("DOMContentLoaded", () => {
  Promise.resolve().then(() => {
    document.open();
    document.write("<main>DCL reaction replacement</main>");
    document.close();
  });
}, { once: true });
"#,
                ),
                (
                    "complete reaction",
                    r#"
document.addEventListener("readystatechange", () => {
  if (document.readyState !== "complete") return;
  Promise.resolve().then(() => {
    document.open();
    document.write("<main>complete reaction replacement</main>");
    document.close();
  });
});
"#,
                ),
                (
                    "load reaction",
                    r#"
window.addEventListener("load", () => {
  Promise.resolve().then(() => {
    document.open();
    document.write("<main>load reaction replacement</main>");
    document.close();
  });
}, { once: true });
"#,
                ),
                (
                    "pageshow reaction",
                    r#"
window.addEventListener("pageshow", () => {
  Promise.resolve().then(() => {
    document.open();
    document.write("<main>pageshow reaction replacement</main>");
    document.close();
  });
}, { once: true });
"#,
                ),
            ] {
                exact_lifecycle_turn_publishes_document_open_replacement(listener_source)
                    .await
                    .unwrap_or_else(|error| {
                        panic!("{stage} replacement scenario should run: {error:#}")
                    });
            }
        },
    );
}

#[test]
fn exact_lifecycle_action_publishes_replacement_document_continuation() {
    run_page_vm_local_runtime_async_test(
        "page-vm-post-parse-action-document-replacement",
        || async move {
            exact_lifecycle_turn_publishes_document_open_replacement(
                r#"
document.addEventListener("DOMContentLoaded", () => {
  document.open();
  document.write("<main id='replacement-owner'>replacement</main>");
  document.close();
}, { once: true });
"#,
            )
            .await
            .expect("exact lifecycle replacement scenario should run");
        },
    );
}

#[tokio::test]
async fn runtime_document_close_response_does_not_wait_for_replacement_lifecycle() {
    run_page_vm_async_test(async move {
        let page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        local_executor
            .run(async move {
                let mut page_vm = page_vm;
                let _ = page_vm.take_page_creation_artifacts();
                let replacement_lifecycle_snapshot =
                    page_vm.document_replacement_lifecycle_action_snapshot();
                let call_id = 710_220;
                let (response_tx, mut response_rx) = tokio::sync::oneshot::channel();
                let messages = page_vm
                    .dispatch_runtime_protocol_message_for_inspector_session_with_deferred_response(
                        None,
                        &serde_json::json!({
                            "id": call_id,
                            "method": "Runtime.evaluate",
                            "params": {
                                "expression": "document.open(); document.write('<main>replacement</main>'); document.close(); 'done'",
                                "returnByValue": true,
                            },
                        })
                        .to_string(),
                        crate::runtime::RendererRuntimeInspectorResponseSender::new(
                            call_id,
                            response_tx,
                        ),
                    )?;
                assert!(messages.is_empty());
                assert!(page_vm.has_pending_runtime_command_lifecycle());
                assert!(matches!(
                    response_rx.try_recv(),
                    Err(tokio::sync::oneshot::error::TryRecvError::Empty)
                ));
                let mut pending_document_lifecycle_turn = None;
                let installed = page_vm
                    .reconcile_document_replacement_lifecycle_after_owner_action(
                        replacement_lifecycle_snapshot,
                        &mut pending_document_lifecycle_turn,
                    )
                    .await?
                    .expect("the explicit replacement boundary should install a resident");
                let replacement_snapshot = page_vm.document_lifecycle.current_snapshot();
                let replacement_document: RendererDocumentLifecycleIdentity =
                    replacement_snapshot.into();
                assert!(matches!(
                    installed,
                    DocumentLifecycleTurnOutcome {
                        action: DocumentLifecycleTurnAction::None,
                        readiness: DocumentLifecycleTurnReadiness::Runnable { document },
                    } if document == replacement_document
                ));
                assert_eq!(
                    pending_document_lifecycle_turn
                        .as_ref()
                        .map(|pending| pending.document),
                    Some(replacement_document),
                    "the command observer target must be bound at the same explicit replacement boundary"
                );

                page_vm.complete_pending_runtime_command_lifecycle()?;
                assert!(
                    !page_vm.has_pending_runtime_command_lifecycle(),
                    "the Runtime response boundary must not remain coupled to page lifecycle work"
                );
                assert!(matches!(
                    response_rx.try_recv(),
                    Err(tokio::sync::oneshot::error::TryRecvError::Closed)
                ));
                let output = page_vm.take_runtime_command_output();
                assert_eq!(
                    output
                        .protocol_response(call_id)
                        .expect("completed Runtime command output should retain its response")
                        ["result"]["result"]["value"],
                    serde_json::json!("done")
                );
                assert!(matches!(
                    page_vm.document_lifecycle_wait_outcome(
                        RendererDocumentLifecycleMilestone::Load
                    ),
                    RendererDocumentLifecycleWaitOutcome::Pending
                ));
                assert_eq!(
                    pending_document_lifecycle_turn
                        .as_ref()
                        .map(|pending| pending.document),
                    Some(replacement_document),
                    "Runtime response completion must not consume the exact replacement lifecycle resident"
                );
                Ok::<_, anyhow::Error>(())
            })
            .await
            .expect("runtime document.close response should precede replacement lifecycle");
    })
    .await;
}

#[tokio::test]
async fn javascript_location_navigation_drains_style_invalidations() {
    run_page_vm_async_test(async move {
        let page_vm = test_page_vm_with_document_url(
            Url::parse("https://javascript-location-style-drain.test/start.html").unwrap(),
        );
        let local_executor = page_vm.local_executor.clone();

        let remaining_cache_entries = local_executor
            .run(async move {
                let mut page_vm = page_vm;
                let mut pending_document_lifecycle_turn = None;
                let document = page_vm.vm().document_handle_for_test();
                let initial = page_vm.vm_mut().eval(
                    r#"
(() => {
  const style = document.createElement('style');
  style.textContent = '.active { color: rgb(80, 90, 100); }';
  document.head.appendChild(style);
  globalThis.__jsLocationDrainTarget = document.createElement('div');
  document.body.appendChild(globalThis.__jsLocationDrainTarget);
  return getComputedStyle(globalThis.__jsLocationDrainTarget).color;
})()
"#,
                )?;
                assert_eq!(initial, "rgb(0, 0, 0)");
                assert_eq!(
                    page_vm
                        .vm()
                        .computed_style_cache_entry_count_for_document_for_test(document),
                    1
                );

                page_vm.vm_mut().eval(
                    r#"
location.href = "javascript:void (globalThis.__jsLocationDrainTarget.className = 'active')";
"queued"
"#,
                )?;
                assert!(
                    page_vm.vm().has_pending_location_navigation(),
                    "javascript: location should remain pending until the owner follows it"
                );

                let outcome = page_vm
                    .follow_pending_location_navigation_one_turn_async(
                        &mut pending_document_lifecycle_turn,
                        PageVmInitStage::Load,
                    )
                    .await?;
                assert!(matches!(
                    outcome,
                    crate::runtime::PageVmFollowNavigationTurnOutcome::Completed
                ));
                Ok::<_, anyhow::Error>(
                    page_vm
                        .vm()
                        .computed_style_cache_entry_count_for_document_for_test(document),
                )
            })
            .await
            .expect("javascript location navigation should execute");

        assert_eq!(remaining_cache_entries, 0);
    })
    .await;
}

#[test]
fn page_resource_namespace_rejects_real_page_vm_replacement_owner_collisions() {
    run_page_vm_large_stack_async_test(
        "page-resource-real-replacement-owner-collision",
        || async move {
            let replacement_markup =
                "<!doctype html><html><head></head><body></body></html>".to_owned();
            let (base_url, server) = spawn_path_response_http_server(vec![
                (
                    "/replacement.html",
                    "HTTP/1.1 200 OK",
                    replacement_markup,
                    Duration::ZERO,
                ),
                (
                    "/replacement-child-pending.html",
                    "HTTP/1.1 200 OK",
                    "<!doctype html><p>replacement child response</p>".to_owned(),
                    Duration::from_millis(100),
                ),
            ])
            .await;
            let loader = crate::network::ResourceRequestClient::new(&FetchConfig::default())
                .expect("loader");
            let initial_url = Url::parse(&format!("{base_url}/initial.html")).unwrap();
            let runtime_hooks = PageVmRuntimeHooks::standalone_without_owner_reservation_for_test();
            let page_vm = test_page_vm_with_loader_document_url_and_hooks(
                &loader,
                Vec::new(),
                initial_url,
                runtime_hooks,
            );
            let local_executor = page_vm.local_executor.clone();
            let replacement_url = format!("{base_url}/replacement.html");
            let replacement_child_url = format!("{base_url}/replacement-child-pending.html");

            local_executor
                .run(async move {
                    let mut page_vm = page_vm;
                    let mut pending_document_lifecycle_turn = None;
                    page_vm.vm_mut().eval(
                        r#"
const oldFrame = document.createElement("iframe");
oldFrame.id = "namespace-collision-child";
document.body.appendChild(oldFrame);
"installed"
"#,
                    )?;
                    let old_child_handle = page_vm
                        .vm()
                        .element_handle_by_id_for_test("namespace-collision-child")
                        .expect("initial PageVm should install the first child handle");
                    let old_child_owner = page_vm
                        .vm()
                        .current_child_document_task_owner(old_child_handle)
                        .expect("initial PageVm should install the first child owner");
                    materialize_child_realm_through_page_turn_for_test(
                        &mut page_vm,
                        "namespace-collision-child",
                    )?;
                    let old_module_target = page_vm
                        .vm()
                        .current_child_document_module_fetch_target(old_child_handle)
                        .expect("initial PageVm should expose its exact child module target");
                    let old_main_owner = page_vm
                        .vm()
                        .current_main_document_task_owner()
                        .expect("initial PageVm should install its main owner");
                    let old_root_document = page_vm.document_lifecycle.identity().document;
                    page_vm.page_task_executor_sources_for_test().dynamic_import_owner_action()
                        .enqueue_local_for_test(
                            old_root_document,
                            replacement_collision_dynamic_import_owner_action(old_module_target),
                        );

                    page_vm.vm_mut().eval(&format!(
                        "location.href = {replacement_url:?}; 'queued'"
                    ))?;
                    let navigation = page_vm
                        .follow_pending_location_navigation_one_turn_async(
                            &mut pending_document_lifecycle_turn,
                            PageVmInitStage::Load,
                        )
                        .await?;
                    assert!(matches!(
                        navigation,
                        crate::runtime::PageVmFollowNavigationTurnOutcome::Completed
                            | crate::runtime::PageVmFollowNavigationTurnOutcome::PostParseLifecycle {
                                ..
                            }
                    ));

                    let replacement_root_document = page_vm.document_lifecycle.identity().document;
                    let replacement_main_owner = page_vm
                        .vm()
                        .current_main_document_task_owner()
                        .expect("replacement PageVm should install its main owner");
                    assert_ne!(old_root_document, replacement_root_document);
                    assert_eq!(
                        old_main_owner, replacement_main_owner,
                        "fresh PageVm frame-owner counters naturally reuse the main (0,0,0) identity"
                    );

                    page_vm.vm_mut().eval(
                        r#"
const replacementFrame = document.createElement("iframe");
replacementFrame.id = "namespace-collision-child";
document.body.appendChild(replacementFrame);
"installed"
"#,
                    )?;
                    let replacement_child_handle = page_vm
                        .vm()
                        .element_handle_by_id_for_test("namespace-collision-child")
                        .expect("replacement PageVm should install the first child handle");
                    let replacement_child_owner = page_vm
                        .vm()
                        .current_child_document_task_owner(replacement_child_handle)
                        .expect("replacement PageVm should install the first child owner");
                    materialize_child_realm_through_page_turn_for_test(
                        &mut page_vm,
                        "namespace-collision-child",
                    )?;
                    let replacement_module_target = page_vm
                        .vm()
                        .current_child_document_module_fetch_target(replacement_child_handle)
                        .expect("replacement PageVm should expose its exact child module target");
                    assert_eq!(
                        old_child_handle, replacement_child_handle,
                        "equivalent replacement DOMs should reuse the first child native handle"
                    );
                    assert_eq!(
                        old_child_owner, replacement_child_owner,
                        "fresh PageVm frame-owner counters naturally reuse the first child owner"
                    );
                    assert_eq!(
                        old_module_target, replacement_module_target,
                        "fresh PageVm frame-owner and realm counters naturally reuse the full local module target"
                    );

                    let old_dynamic_action = page_vm
                        .run_page_dynamic_import_owner_action_body_for_test()
                        .expect("retired dynamic-import action should survive replacement for one stale discard turn");
                    assert_eq!(
                        old_dynamic_action.action.owner,
                        crate::page_task_queue::RendererPageDynamicImportOwnerActionOwner::new(
                            old_root_document,
                            old_module_target.task_owner(),
                            old_module_target.realm_id(),
                        )
                    );
                    assert_eq!(
                        old_dynamic_action.action.document_effect,
                        crate::page_task_queue::PageDynamicImportOwnerActionDocumentEffect::DiscardedStaleOwner {
                            current_owner: Some(
                                crate::page_task_queue::RendererPageDynamicImportOwnerActionOwner::new(
                                    replacement_root_document,
                                    replacement_module_target.task_owner(),
                                    replacement_module_target.realm_id(),
                                ),
                            ),
                        },
                        "stable Page storage must not let naturally reused local Document/realm IDs authorize an old action"
                    );

                    assert!(
                        !page_vm.page_task_executor_sources_for_test().dynamic_import_owner_action()
                            .has_ready_task(),
                        "the stale old-Document action should be consumed from the stable typed source"
                    );

                    let pending_script_id = ParserPendingScriptId::from_key(
                        MainParserDocumentOwner::new(old_main_owner),
                        ParserPendingScriptKey::from_parts_for_test(1, NodeId::new(9)),
                    );
                    let mut queue = RendererPageNetworkingSource::new_for_test();
                    queue.enqueue_local_for_test(
                        RendererPageResourceCompletion::main_parser_deferred_classic_source(
                            old_root_document,
                            MainParserDeferredClassicSourceLoadCompletion::new(
                                pending_script_id,
                                PreparedScriptSourceLoadOutcome {
                                    source_result: Ok(
                                        "globalThis.__oldNamespaceDeferRan = true".to_owned(),
                                    ),
                                    source_bytes: None,
                                    network_result: None,
                                },
                            ),
                            MainParserDeferredClassicSourceNetworkAttribution::new(
                                Url::parse("https://old-document.test/").unwrap(),
                                Url::parse("https://old-document.test/defer.js").unwrap(),
                            ),
                        ),
                    );
                    let main_outcome = page_vm
                        .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
                        .expect("late old main terminal should consume one bounded turn");
                    assert_eq!(
                        main_outcome.action,
                        PageResourceCompletionTurnAction {
                            source:
                                RendererOwnerResourceActivitySource::MainParserDeferredClassicSource,
                            owner: RendererPageResourceCompletionOwner::main_document(
                                old_root_document,
                                old_main_owner,
                            ),
                            document_effect:
                                PageResourceCompletionDocumentEffect::DiscardedStaleOwner {
                                    current_owner: Some(
                                        RendererPageResourceCompletionOwner::main_document(
                                            replacement_root_document,
                                            replacement_main_owner,
                                        ),
                                    ),
                                },
                            body_activity:
                                PageResourceCompletionBodyActivity::NoPageCodeOrEventDispatch,
                            post_checkpoint_effect:
                                PageResourceCompletionPostCheckpointEffect::None,
                            output_effect: PageResourceCompletionOutputEffect::None,
                        }
                    );
                    assert_eq!(
                        page_vm
                            .vm_mut()
                            .eval("String(globalThis.__oldNamespaceDeferRan)")?,
                        "undefined"
                    );

                    let activity_epoch_before = page_vm.vm().subresource_activity_epoch();
                    queue.enqueue_local_for_test(
                        RendererPageResourceCompletion::child_classic_script(
                            old_root_document,
                            ChildClassicScriptLoadCompletion {
                                owner: old_child_owner,
                                load_id: 31,
                                handle: old_child_handle,
                                script_handle: NodeId::new(32),
                                result: Ok(
                                    "parent.__oldNamespaceChildClassicRan = true".to_owned(),
                                ),
                                network_result: Some(Arc::new(Err(
                                    "retired child classic failed".to_owned(),
                                ))),
                                network_attribution: ChildClassicScriptNetworkAttribution {
                                    frame_id: Some("retired-child-classic-frame".to_owned()),
                                    document_url: Url::parse(
                                        "https://old-document.test/classic-child",
                                    )
                                    .unwrap(),
                                    request_url: Url::parse(
                                        "https://old-document.test/classic-child.js",
                                    )
                                    .unwrap(),
                                },
                            },
                        ),
                    );
                    let classic_outcome = page_vm
                        .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
                        .expect("late old child classic terminal should consume one bounded turn");
                    assert_eq!(
                        classic_outcome.action,
                        PageResourceCompletionTurnAction {
                            source: RendererOwnerResourceActivitySource::ChildClassicScript,
                            owner: RendererPageResourceCompletionOwner::child_document(
                                old_root_document,
                                old_child_handle,
                                old_child_owner,
                            ),
                            document_effect:
                                PageResourceCompletionDocumentEffect::DiscardedStaleOwner {
                                    current_owner: Some(
                                        RendererPageResourceCompletionOwner::child_document(
                                            replacement_root_document,
                                            replacement_child_handle,
                                            replacement_child_owner,
                                        ),
                                    ),
                                },
                            body_activity:
                                PageResourceCompletionBodyActivity::NoPageCodeOrEventDispatch,
                            post_checkpoint_effect:
                                PageResourceCompletionPostCheckpointEffect::None,
                            output_effect: PageResourceCompletionOutputEffect::CaptureRequired,
                        }
                    );
                    assert_eq!(
                        page_vm.vm().subresource_activity_epoch(),
                        activity_epoch_before,
                        "retired classic Network output must not advance replacement Document activity"
                    );
                    let (network_records, _, _) =
                        split_network_output_items(page_vm.vm_mut().take_network_output());
                    assert_eq!(network_records.len(), 1);
                    assert_eq!(
                        network_records[0].frame_id(),
                        Some("retired-child-classic-frame")
                    );

                    queue.enqueue_local_for_test(
                        RendererPageResourceCompletion::child_parser_module_root_fetch(
                            old_root_document,
                            test_child_parser_module_root_completion_for_target(
                                old_module_target,
                                37,
                                "retired-child-module-root",
                                Some("retired child module root failed"),
                            ),
                        ),
                    );
                    queue.enqueue_local_for_test(
                        RendererPageResourceCompletion::child_module_dependency_fetch(
                            old_root_document,
                            test_child_module_dependency_completion_for_target(
                                old_module_target,
                                41,
                                "retired-child-module-dependency",
                                Some("retired child module dependency failed"),
                            ),
                        ),
                    );
                    queue.enqueue_local_for_test(
                        RendererPageResourceCompletion::child_modulepreload_fetch(
                            old_root_document,
                            test_child_modulepreload_completion_for_target(
                                old_module_target,
                                43,
                                "retired-child-modulepreload",
                                Some("retired child modulepreload failed"),
                            ),
                        ),
                    );
                    queue.enqueue_local_for_test(
                        RendererPageResourceCompletion::child_dynamic_import_fetch(
                            old_root_document,
                            test_child_dynamic_import_completion_for_target(
                                old_module_target,
                                47,
                                "retired-child-dynamic-import",
                                Some("retired child dynamic import failed"),
                            ),
                        ),
                    );
                    let old_module_owner = RendererPageResourceCompletionOwner::child_module_fetch(
                        old_root_document,
                        old_module_target,
                    );
                    let replacement_module_owner =
                        RendererPageResourceCompletionOwner::child_module_fetch(
                            replacement_root_document,
                            replacement_module_target,
                        );
                    let root_module_outcome = page_vm
                        .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
                        .expect("late old child module root terminal should consume one turn");
                    assert_eq!(root_module_outcome.action.owner, old_module_owner);
                    assert_eq!(
                        root_module_outcome.action.document_effect,
                        PageResourceCompletionDocumentEffect::DiscardedStaleOwner {
                            current_owner: Some(replacement_module_owner),
                        }
                    );

                    let dependency_module_outcome = page_vm
                        .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
                        .expect("late old child module dependency terminal should consume one turn");
                    assert_eq!(dependency_module_outcome.action.owner, old_module_owner);
                    assert_eq!(
                        dependency_module_outcome.action.document_effect,
                        PageResourceCompletionDocumentEffect::DiscardedStaleOwner {
                            current_owner: Some(replacement_module_owner),
                        }
                    );

                    let modulepreload_outcome = page_vm
                        .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
                        .expect("late old child modulepreload terminal should consume one turn");
                    assert_eq!(modulepreload_outcome.action.owner, old_module_owner);
                    assert_eq!(
                        modulepreload_outcome.action.document_effect,
                        PageResourceCompletionDocumentEffect::DiscardedStaleOwner {
                            current_owner: Some(replacement_module_owner),
                        }
                    );

                    let dynamic_import_outcome = page_vm
                        .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
                        .expect("late old child dynamic-import terminal should consume one turn");
                    assert_eq!(dynamic_import_outcome.action.owner, old_module_owner);
                    assert_eq!(
                        dynamic_import_outcome.action.document_effect,
                        PageResourceCompletionDocumentEffect::DiscardedStaleOwner {
                            current_owner: Some(replacement_module_owner),
                        }
                    );
                    assert_eq!(
                        dynamic_import_outcome.action.output_effect,
                        PageResourceCompletionOutputEffect::CaptureRequired
                    );

                    assert_eq!(
                        page_vm.vm().subresource_activity_epoch(),
                        activity_epoch_before,
                        "retired module Network output must not advance replacement Document activity"
                    );
                    let (network_records, _, _) =
                        split_network_output_items(page_vm.vm_mut().take_network_output());
                    assert_eq!(network_records.len(), 4);
                    assert_eq!(
                        network_records
                            .iter()
                            .map(|record| record.frame_id())
                            .collect::<Vec<_>>(),
                        vec![
                            Some("retired-child-module-root-frame"),
                            Some("retired-child-module-dependency-frame"),
                            Some("retired-child-modulepreload-frame"),
                            Some("retired-child-dynamic-import-frame"),
                        ]
                    );
                    assert_eq!(
                        network_records[3].request_initiator_type(),
                        SubresourceRequestInitiatorType::Script,
                        "dynamic import must retain script initiator attribution after replacement"
                    );

                    queue.enqueue_local_for_test(
                        RendererPageResourceCompletion::child_blocking_stylesheet(
                            old_root_document,
                            ChildBlockingStylesheetLoadCompletion {
                                child_handle: old_child_handle,
                                owner: old_child_owner,
                                signature: crate::DocumentBlockingStylesheetSignature::
                                    ParserCreatedStyleImport { urls: Vec::new() },
                                network_results: vec![ChildBlockingStylesheetNetworkResult {
                                    frame_id: Some("retired-child-frame".to_owned()),
                                    document_url: Url::parse(
                                        "https://old-document.test/child",
                                    )
                                    .unwrap(),
                                    request_url: Url::parse(
                                        "https://old-document.test/child.css",
                                    )
                                    .unwrap(),
                                    initiator_type: SubresourceRequestInitiatorType::Parser,
                                    terminal: crate::stylesheet_blocking::
                                        StylesheetFetchTerminal::network_error(
                                            "retired child stylesheet failed",
                                        ),
                                }],
                            },
                        ),
                    );
                    let child_outcome = page_vm
                        .apply_one_page_resource_terminal_owner_admission_for_test(&mut queue)?
                        .expect("late old child terminal should consume one bounded turn");
                    assert_eq!(
                        child_outcome.action,
                        PageResourceCompletionTurnAction {
                            source: RendererOwnerResourceActivitySource::ChildBlockingStylesheet,
                            owner: RendererPageResourceCompletionOwner::child_document(
                                old_root_document,
                                old_child_handle,
                                old_child_owner,
                            ),
                            document_effect:
                                PageResourceCompletionDocumentEffect::DiscardedStaleOwner {
                                    current_owner: Some(
                                        RendererPageResourceCompletionOwner::child_document(
                                            replacement_root_document,
                                            replacement_child_handle,
                                            replacement_child_owner,
                                        ),
                                    ),
                                },
                            body_activity:
                                PageResourceCompletionBodyActivity::NoPageCodeOrEventDispatch,
                            post_checkpoint_effect:
                                PageResourceCompletionPostCheckpointEffect::None,
                            output_effect: PageResourceCompletionOutputEffect::CaptureRequired,
                        }
                    );
                    assert_eq!(
                        page_vm.vm().subresource_activity_epoch(),
                        activity_epoch_before,
                        "old PageVm Network output must not advance replacement Document activity"
                    );
                    let (network_records, _, _) =
                        split_network_output_items(page_vm.vm_mut().take_network_output());
                    assert_eq!(network_records.len(), 1);
                    assert_eq!(network_records[0].frame_id(), Some("retired-child-frame"));

                    page_vm.vm_mut().eval(&format!(
                        "document.getElementById('namespace-collision-child').src = {replacement_child_url:?};"
                    ))?;
                    let stale_navigation = page_vm
                        .run_child_navigation_commit_body_for_test()?
                        .expect("superseded replacement navigation should retain a stale turn");
                    assert!(matches!(
                        stale_navigation.action.target_effect,
                        crate::page_task_queue::PageChildNavigationCommitTargetEffect::DiscardedStaleOwner {
                            current_owner: Some(_)
                        }
                    ));
                    let current_navigation = page_vm
                        .run_child_navigation_commit_body_for_test()?
                        .expect("replacement PageVm navigation should retain its current turn");
                    assert!(matches!(
                        current_navigation.action.target_effect,
                        crate::page_task_queue::PageChildNavigationCommitTargetEffect::AppliedToCurrentOwner
                    ));
                    let replacement_navigation_target = page_vm
                        .vm()
                        .current_child_document_navigation_fetch_target(replacement_child_handle)
                        .expect("replacement child should expose an in-flight exact navigation target");
                    assert_eq!(
                        replacement_navigation_target.task_owner(),
                        old_child_owner,
                        "fresh PageVm child navigation must naturally collide with the old PageVm's local Document owner"
                    );
                    let old_navigation_owner =
                        RendererPageResourceCompletionOwner::child_document_navigation(
                            old_root_document,
                            replacement_navigation_target,
                        );
                    let replacement_navigation_owner =
                        RendererPageResourceCompletionOwner::child_document_navigation(
                            replacement_root_document,
                            replacement_navigation_target,
                        );
                    let mut child_navigation_queue = RendererPageNetworkingSource::new_for_test();
                    child_navigation_queue.enqueue_local_for_test(
                        RendererPageResourceCompletion::child_document_load(
                            old_root_document,
                            super::child_document_completion::stale_loaded_completion(
                                replacement_navigation_target,
                                "retired-root-child-frame",
                                "https://retired-document.test/child-response.html",
                            ),
                        ),
                    );
                    let activity_epoch_before_navigation_terminal =
                        page_vm.vm().subresource_activity_epoch();
                    let navigation_outcome = page_vm
                        .apply_one_page_resource_terminal_owner_admission_for_test(&mut child_navigation_queue)?
                        .expect("old-root child terminal should consume one stale-discard turn");
                    assert_eq!(navigation_outcome.action.owner, old_navigation_owner);
                    assert_eq!(
                        navigation_outcome.action.document_effect,
                        PageResourceCompletionDocumentEffect::DiscardedStaleOwner {
                            current_owner: Some(replacement_navigation_owner),
                        },
                        "equal PageVm-local navigation IDs must not bypass the root Document namespace"
                    );
                    assert_eq!(
                        page_vm
                            .vm()
                            .current_child_document_navigation_fetch_target(
                                replacement_child_handle,
                            ),
                        Some(replacement_navigation_target),
                        "cross-root stale cleanup must not settle the replacement PageVm's colliding request"
                    );
                    assert_eq!(
                        page_vm.vm().subresource_activity_epoch(),
                        activity_epoch_before_navigation_terminal,
                        "old-root child Network output must not become replacement Document activity"
                    );
                    let historical_child_networks =
                        page_vm.take_completed_child_document_networks();
                    assert_eq!(historical_child_networks.len(), 1);
                    assert_eq!(
                        historical_child_networks[0].snapshot.request_url,
                        "https://retired-document.test/child-response.html"
                    );
                    Ok::<_, anyhow::Error>(())
                })
                .await
                .expect("real replacement namespace collision fixture should run");

            server
                .await
                .expect("replacement response server should finish");
        },
    );
}

#[test]
fn top_level_http_location_navigation_reserves_service_worker_client_until_commit() {
    run_page_vm_large_stack_async_test("page-vm-top-level-sw-reserved-client", || async move {
        let (base_url, server) = spawn_path_response_http_server(vec![(
            "/next.html",
            "HTTP/1.1 200 OK",
            "<!doctype html><title>next</title><body>next</body>".to_owned(),
            Duration::ZERO,
        )])
        .await;
        let loader =
            crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
        let page_vm = test_page_vm_with_loader_and_document_url(
            &loader,
            Vec::new(),
            Url::parse(&format!("{base_url}/start.html")).unwrap(),
        );
        let browser_context_runtime = page_vm.runtime_hooks.browser_context_runtime.clone();
        let browser_context_runtime_after_drop = browser_context_runtime.clone();
        let local_executor = page_vm.local_executor.clone();
        let next_url = format!("{base_url}/next.html");

        local_executor
            .run(async move {
                let mut page_vm = page_vm;
                let mut pending_document_lifecycle_turn = None;
                let initial_diagnostics = browser_context_runtime
                    .service_worker_runtime()
                    .diagnostics_snapshot();
                assert_eq!(initial_diagnostics.live_client_count, 1);

                page_vm
                    .vm_mut()
                    .eval(&format!("location.href = {next_url:?}; 'queued'"))?;
                assert!(
                    page_vm.vm().has_pending_location_navigation(),
                    "http location should remain pending until the owner follows it"
                );
                let pending_diagnostics = browser_context_runtime
                    .service_worker_runtime()
                    .diagnostics_snapshot();
                assert_eq!(pending_diagnostics.live_client_count, 2);

                let outcome = page_vm
                    .follow_pending_location_navigation_one_turn_async(
                        &mut pending_document_lifecycle_turn,
                        PageVmInitStage::Load,
                    )
                    .await?;
                match outcome {
                    crate::runtime::PageVmFollowNavigationTurnOutcome::PostParseLifecycle {
                        target_stage: PageVmInitStage::Load,
                        outcome:
                            DocumentLifecycleTurnOutcome {
                                readiness:
                                    DocumentLifecycleTurnReadiness::Runnable {
                                        document: mut lifecycle_document,
                                    },
                                ..
                            },
                    } => {
                        loop {
                            match page_vm
                                .advance_post_parse_lifecycle_one_owner_turn(
                                    &mut pending_document_lifecycle_turn,
                                    lifecycle_document,
                                )
                                .await?
                            {
                                DocumentLifecycleTurnOutcome {
                                    readiness:
                                        DocumentLifecycleTurnReadiness::Runnable { document },
                                    ..
                                } => lifecycle_document = document,
                                DocumentLifecycleTurnOutcome {
                                    action:
                                        DocumentLifecycleTurnAction::ReachedStage(
                                            PageVmInitStage::Load,
                                        ),
                                    readiness: DocumentLifecycleTurnReadiness::Idle,
                                } => break,
                                _ => panic!(
                                    "unblocked navigation lifecycle should complete without parking or replacement"
                                ),
                            }
                        }
                    }
                    crate::runtime::PageVmFollowNavigationTurnOutcome::Completed => {}
                    _ => panic!(
                        "top-level HTTP navigation should commit or enter its post-parse continuation"
                    ),
                }
                assert_eq!(page_vm.vm_mut().eval("location.href")?, next_url);
                assert_eq!(page_vm.vm_mut().eval("document.title")?, "next");
                let committed_diagnostics = browser_context_runtime
                    .service_worker_runtime()
                    .diagnostics_snapshot();
                assert_eq!(
                    committed_diagnostics.live_client_count, 1,
                    "committing a reserved navigation should release the old page window client during context teardown"
                );
                Ok::<_, anyhow::Error>(())
            })
            .await
            .expect("http location navigation should commit reserved service worker client");
        let after_drop_diagnostics = browser_context_runtime_after_drop
            .service_worker_runtime()
            .diagnostics_snapshot();
        assert_eq!(after_drop_diagnostics.live_client_count, 0);

        server
            .await
            .expect("top-level reserved navigation response server should finish");
    });
}

#[test]
fn runtime_evaluate_without_enable_uses_inspector_default_context() {
    let mut page_vm = test_page_vm();

    let messages = page_vm
        .vm_mut()
        .dispatch_inspector_protocol_message(
            r#"{"id":7,"method":"Runtime.evaluate","params":{"expression":"21 + 21"}}"#,
        )
        .expect("Runtime.evaluate without Runtime.enable should dispatch");

    let response = messages
        .iter()
        .find(|message| message["id"] == json!(7))
        .expect("Runtime.evaluate response");
    assert_eq!(response["result"]["result"]["type"], json!("number"));
    assert_eq!(response["result"]["result"]["value"], json!(42));
    assert!(
        !messages
            .iter()
            .any(|message| message["method"] == json!("Runtime.executionContextCreated")),
        "default-context materialization must not open the Runtime event surface"
    );
}

#[test]
fn page_diagnostics_snapshot_uses_rust_runtime_observable_console_source_queue() {
    let mut page_vm = test_page_vm();
    page_vm
        .vm_mut()
        .dispatch_inspector_protocol_message(r#"{"id":1,"method":"Runtime.enable"}"#)
        .expect("enable runtime");
    page_vm
        .vm_mut()
        .eval(
            r#"
console.log('queued-before-js-buffer-clear');
globalThis.__moliConsole = [];
globalThis.__moliConsoleDetails = [];
'done';
"#,
        )
        .expect("console log and clear JS buffers");

    let snapshot = page_vm
        .page_diagnostics_snapshot()
        .expect("page diagnostics snapshot");
    let source = snapshot
        .runtime_observable_source()
        .expect("runtime observable source should come from the Rust queue");

    assert_eq!(source.console_messages_with_context(), 1);
    assert_eq!(source.source_items().len(), 1);
    assert!(matches!(
        &source.source_items()[0],
        RendererRuntimeObservableSourceItem::ConsoleMessage { message, .. }
            if message.message == "log: queued-before-js-buffer-clear"
    ));
}

#[test]
fn runtime_console_message_snapshot_ignores_user_tampered_console_buffers() {
    let mut page_vm = test_page_vm();
    page_vm
        .vm_mut()
        .dispatch_inspector_protocol_message(r#"{"id":1,"method":"Runtime.enable"}"#)
        .expect("enable runtime");
    page_vm
        .vm_mut()
        .eval(
            r#"
globalThis.__moliConsole = null;
globalThis.__moliConsoleDetails = "not an array";
console.info('slot snapshot console');
'done';
"#,
        )
        .expect("console log after user buffer tamper");

    let messages = page_vm
        .vm_mut()
        .snapshot_console_messages_with_context()
        .expect("runtime console messages with context");
    assert!(
        messages.iter().any(|message| {
            message.message == "info: slot snapshot console"
                && message.args.first().and_then(|arg| arg.get("value"))
                    == Some(&json!("slot snapshot console"))
                && message.execution_context_id > 0
        }),
        "runtime console message snapshot should come from context slots: {messages:?}"
    );
}

#[test]
fn page_report_observable_console_uses_runtime_source_queue() {
    let mut page_vm = test_page_vm();
    page_vm
        .vm_mut()
        .eval(
            r#"
console.log('report-from-source-queue');
globalThis.__moliConsole = [];
globalThis.__moliConsoleDetails = [];
'done';
"#,
        )
        .expect("console log and clear JS buffers");

    let state_capture = page_vm.capture_page_state().expect("page state capture");

    assert_eq!(
        state_capture.report.console_messages(),
        &["log: report-from-source-queue".to_owned()],
        "page report console output should come from the renderer producer queue, not a JS buffer capture"
    );
}

#[test]
fn page_report_capture_reuses_unchanged_network_report() {
    let mut page_vm = test_page_vm();
    let document_url = Url::parse("https://report-cache.test/").expect("document URL");
    let request_url = Url::parse("https://report-cache.test/api").expect("request URL");
    page_vm
        .report
        .extend_network_output(ScriptNetworkOutput::from_items([
            ScriptNetworkOutputItem::SubresourceNetworkRecord(Box::new(
                SubresourceNetworkRecord::success(
                    None,
                    document_url,
                    request_url.clone(),
                    "GET".to_owned(),
                    Vec::new(),
                    None,
                    SubresourceResourceType::Fetch,
                    None,
                    Vec::new(),
                    request_url,
                    200,
                    Vec::new(),
                    "ok".to_owned(),
                    Vec::new(),
                ),
            )),
        ]));

    let first = page_vm
        .capture_page_state()
        .expect("first page state capture");
    assert_eq!(first.report.network_output_items().len(), 1);
    let second = page_vm
        .capture_page_state()
        .expect("second page state capture");

    assert!(
        Arc::ptr_eq(&first.report, &second.report),
        "unchanged report captures should share the same Arc instead of cloning network history"
    );

    page_vm
        .vm_mut()
        .eval("console.log('new-output');")
        .expect("queue observable output");
    let third = page_vm
        .capture_page_state()
        .expect("third page state capture");

    assert!(
        !Arc::ptr_eq(&second.report, &third.report),
        "new report output should publish a new immutable report capture"
    );
}

#[test]
fn page_state_capture_publishes_lightweight_document_metadata() {
    let loader =
        crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
    let document_url = Url::parse("https://metadata-cache.test/page").expect("document URL");
    let dom_host = DomHost::from_dom(
        HtmlParser.parse(
            document_url.clone(),
            "<!doctype html><html><head><title>metadata-title</title></head><body></body></html>"
                .to_owned(),
        ),
    );
    let mut page_vm = test_page_vm_with_loader_and_dom_host(&loader, dom_host);

    let state_capture = page_vm.capture_page_state().expect("page state capture");
    assert_eq!(state_capture.final_url, document_url);
    assert_eq!(state_capture.document_title, "metadata-title");

    let requested_url = Url::parse("https://metadata-cache.test/request").expect("requested URL");
    let state = RendererPageState::from_vm_state_capture(
        requested_url,
        None,
        false,
        0,
        200,
        Vec::new(),
        state_capture,
    );
    assert_eq!(
        state.final_url().as_str(),
        "https://metadata-cache.test/page"
    );
    assert_eq!(state.document_title(), "metadata-title");
}

#[test]
fn serialize_html_reads_current_document_after_document_open() {
    let loader =
        crate::network::ResourceRequestClient::new(&FetchConfig::default()).expect("loader");
    let document_url = Url::parse("https://document-cache.test/").expect("test URL");
    let dom_host = DomHost::from_dom(HtmlParser.parse(
        document_url,
        "<!doctype html><html><head></head><body>before</body></html>".to_owned(),
    ));
    let mut page_vm = test_page_vm_with_loader_and_dom_host(&loader, dom_host);

    let before_owner = page_vm
        .vm()
        .current_main_document_task_owner()
        .expect("initial document owner");
    let before_html = page_vm.serialize_html();
    assert!(before_html.contains(">before<"));

    page_vm
        .vm_mut()
        .eval(
            r#"document.open();
document.write("<input id='chooser' type='file' multiple>");
document.close();"#,
        )
        .expect("replace document");

    let after_owner = page_vm
        .vm()
        .current_main_document_task_owner()
        .expect("replacement document owner");
    let after_html = page_vm.serialize_html();

    assert_ne!(before_owner.document_id, after_owner.document_id);
    assert!(
        after_html.contains("<input id=\"chooser\" type=\"file\" multiple=\"\">"),
        "live HTML serialization must describe the current Document"
    );
    assert!(
        !after_html.contains(">before<"),
        "live HTML serialization must not reuse retired Document output"
    );
}

#[test]
fn page_report_lifecycle_output_does_not_duplicate_pending_activity_source() {
    let mut page_vm = test_page_vm();
    page_vm
        .vm_mut()
        .record_runtime_warning(format_args!("runtime warning"));

    let state_capture = page_vm.capture_page_state().expect("page state capture");
    assert_eq!(
        state_capture.report.lifecycle_errors(),
        &["runtime warning".to_owned()]
    );

    let activity = page_vm
        .page_diagnostics_snapshot()
        .expect("page diagnostics snapshot");

    assert_eq!(activity.diagnostics.runtime_lifecycle_errors, 1);
    let source = activity
        .runtime_observable_source()
        .expect("report lifecycle output should remain visible as RuntimeObservable source");
    assert_eq!(source.lifecycle_errors(), 1);
    assert_eq!(source.source_items().len(), 1);
    assert!(matches!(
        &source.source_items()[0],
        RendererRuntimeObservableSourceItem::LifecycleError { text, .. }
            if text == "runtime warning"
    ));
}

#[tokio::test]
async fn handle_post_parse_lifecycle_advance_returns_completion_without_running_task() {
    let mut page_vm = test_page_vm();
    let local_executor = page_vm.local_executor.clone();

    let disposition = local_executor
        .run(async move {
            page_vm
                .handle_post_parse_lifecycle_advance_on_named_owner_lane(
                    PageVmInitStage::DomContentLoaded,
                    PostParseLifecycleAdvance::Complete(
                        PostParseLifecycleCompletionAction::ReturnAtStage("DOMContentLoaded"),
                    ),
                )
                .await
        })
        .await
        .expect("helper should succeed");

    assert!(matches!(
        disposition,
        PostParseLifecycleLoopAdvance::Complete(PostParseLifecycleCompletionAction::ReturnAtStage(
            "DOMContentLoaded"
        ))
    ));
}

#[test]
fn handle_post_parse_lifecycle_advance_runs_page_owned_task_and_returns_completed_task() {
    run_page_vm_local_runtime_async_test("page-vm-lifecycle-advance-owned-task", || async move {
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        let disposition = local_executor
            .run(async move {
                let loader = page_vm.main_document_resource_loader();
                let lifecycle_driver = {
                    let PageVm {
                        vm,
                        page_task_queue,
                        report,
                        ..
                    } = &mut page_vm;
                    vm.as_mut()
                        .expect("page vm must retain a live ScriptVm until drop")
                        .start_post_parse_lifecycle_round(
                            PageVmInitStage::Load,
                            page_task_queue,
                            report,
                            vec![post_parse_detached_runs_work(vec![])],
                        )
                        .await
                };
                let advance = {
                    let PageVm {
                        vm,
                        page_task_queue,
                        report,
                        ..
                    } = &mut page_vm;
                    vm.as_mut()
                        .expect("page vm must retain a live ScriptVm until drop")
                        .advance_post_parse_lifecycle(
                            loader.request_client(),
                            page_task_queue,
                            report,
                            lifecycle_driver,
                            None,
                        )
                        .await
                        .expect("post-parse driver should produce a page-owned task")
                };
                let disposition = page_vm
                    .handle_post_parse_lifecycle_advance_on_named_owner_lane(
                        PageVmInitStage::Load,
                        advance,
                    )
                    .await?;
                Ok::<_, anyhow::Error>(disposition)
            })
            .await
            .expect("helper should succeed");

        let PostParseLifecycleLoopAdvance::Continue(task) = disposition else {
            panic!("page-owned advance should continue with completed task");
        };
        let Some(_task) = *task else {
            panic!("page-owned advance should continue with completed task");
        };
    });
}

#[test]
fn drive_post_parse_lifecycle_loop_returns_at_domcontentloaded_before_trailing_task() {
    run_page_vm_local_runtime_async_test("page-vm-dcl-before-trailing-task", || async move {
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        let (completion, trailing_task_remained) = local_executor
            .run(async move {
                let lifecycle_driver = {
                    let PageVm {
                        vm,
                        page_task_queue,
                        report,
                        ..
                    } = &mut page_vm;
                    vm.as_mut()
                        .expect("page vm must retain a live ScriptVm until drop")
                        .start_post_parse_lifecycle_round(
                            PageVmInitStage::DomContentLoaded,
                            page_task_queue,
                            report,
                            vec![post_parse_detached_runs_work(vec![])],
                        )
                        .await
                };
                let completion = page_vm
                    .drive_post_parse_lifecycle_loop_on_named_owner_lane(
                        PageVmInitStage::DomContentLoaded,
                        lifecycle_driver,
                    )
                    .await?;
                let trailing_task_remained = matches!(
                    page_vm.page_task_queue.post_parse_front(),
                    Some(work)
                        if matches!(
                            work.as_page_task(),
                            Some(PageTask::RecordDetachedPostParseRuns(_))
                        )
                );
                Ok::<_, anyhow::Error>((completion, trailing_task_remained))
            })
            .await
            .expect("driver loop should succeed");

        assert!(matches!(
            completion,
            PostParseLifecycleCompletionAction::ReturnAtStage("DOMContentLoaded")
        ));
        assert!(
            trailing_task_remained,
            "DCL-stage post-parse loop must not overrun trailing post-DCL work"
        );
    });
}

#[test]
fn drive_post_parse_lifecycle_loop_returns_at_domcontentloaded_before_listener_work() {
    run_page_vm_local_runtime_async_test("page-vm-dcl-before-listener-work", || async move {
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        let completion = local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r#"
                    (() => {
                        document.addEventListener("DOMContentLoaded", () => {
                            const script = document.createElement("script");
                            script.text = "globalThis.__postDclDynamicScriptRan = true";
                            document.body.appendChild(script);
                        });
                    })()
                    "#,
                )?;
                let lifecycle_driver = {
                    let PageVm {
                        vm,
                        page_task_queue,
                        report,
                        ..
                    } = &mut page_vm;
                    vm.as_mut()
                        .expect("page vm must retain a live ScriptVm until drop")
                        .start_post_parse_lifecycle_round(
                            PageVmInitStage::DomContentLoaded,
                            page_task_queue,
                            report,
                            Vec::new(),
                        )
                        .await
                };
                page_vm
                    .drive_post_parse_lifecycle_loop_on_named_owner_lane(
                        PageVmInitStage::DomContentLoaded,
                        lifecycle_driver,
                    )
                    .await
            })
            .await
            .expect("driver loop should succeed");

        assert!(matches!(
            completion,
            PostParseLifecycleCompletionAction::ReturnAtStage("DOMContentLoaded")
        ));
    });
}

#[test]
fn lifecycle_callback_enters_v8_from_fresh_owner_local_task_on_default_stack() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build_local(tokio::runtime::LocalOptions::default())
        .expect("default-stack page_vm lifecycle test runtime should build")
        .block_on(run_page_vm_async_test(async move {
            let mut page_vm = test_page_vm();
            let local_executor = page_vm.local_executor.clone();

            let (completion, log) = local_executor
                .run(async move {
                    page_vm.vm_mut().eval(
                        r##"
                    (() => {
                        globalThis.__lifecycleFreshTaskLog = [];
                        navigation.oncurrententrychange = event => {
                            globalThis.__lifecycleFreshTaskLog.push(
                                `change:${event.navigationType}:${location.hash}`
                            );
                        };
                        document.addEventListener("DOMContentLoaded", () => {
                            globalThis.__lifecycleFreshTaskLog.push("dcl");
                            history.pushState({ from: "dcl" }, "", "#from-dcl");
                            globalThis.__lifecycleFreshTaskLog.push(`after:${location.hash}`);
                        });
                    })()
                    "##,
                    )?;
                    let lifecycle_driver = {
                        let PageVm {
                            vm,
                            page_task_queue,
                            report,
                            ..
                        } = &mut page_vm;
                        vm.as_mut()
                            .expect("page vm must retain a live ScriptVm until drop")
                            .start_post_parse_lifecycle_round(
                                PageVmInitStage::DomContentLoaded,
                                page_task_queue,
                                report,
                                Vec::new(),
                            )
                            .await
                    };
                    let completion = page_vm
                        .drive_post_parse_lifecycle_loop_on_named_owner_lane(
                            PageVmInitStage::DomContentLoaded,
                            lifecycle_driver,
                        )
                        .await?;
                    let log = page_vm.vm_mut().eval("__lifecycleFreshTaskLog.join('|')")?;
                    Ok::<_, anyhow::Error>((completion, log))
                })
                .await
                .expect("driver loop should succeed on default-stack local runtime");

            assert!(matches!(
                completion,
                PostParseLifecycleCompletionAction::ReturnAtStage("DOMContentLoaded")
            ));
            assert_eq!(log, "dcl|change:push:#from-dcl|after:#from-dcl");
        }));
}

#[test]
fn drive_post_parse_lifecycle_loop_does_not_wait_for_child_frame_load_at_domcontentloaded() {
    run_page_vm_local_runtime_async_test("page-vm-dcl-before-child-frame-load", || async move {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
        let frame_url = format!("http://{}/slow-frame", listener.local_addr().expect("addr"));
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let mut request = [0_u8; 1];
            let _ = stream.read(&mut request).await;
            sleep(Duration::from_secs(5)).await;
        });

        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        let completion = tokio::time::timeout(
            Duration::from_secs(1),
            local_executor.run(async move {
                page_vm.vm_mut().eval(&format!(
                    r#"
                    (() => {{
                        const frame = document.createElement("iframe");
                        frame.src = "{}";
                        document.body.appendChild(frame);
                    }})()
                    "#,
                    frame_url
                ))?;
                let lifecycle_driver = {
                    let PageVm {
                        vm,
                        page_task_queue,
                        report,
                        ..
                    } = &mut page_vm;
                    vm.as_mut()
                        .expect("page vm must retain a live ScriptVm until drop")
                        .start_post_parse_lifecycle_round(
                            PageVmInitStage::DomContentLoaded,
                            page_task_queue,
                            report,
                            Vec::new(),
                        )
                        .await
                };
                page_vm
                    .drive_post_parse_lifecycle_loop_on_named_owner_lane(
                        PageVmInitStage::DomContentLoaded,
                        lifecycle_driver,
                    )
                    .await
            }),
        )
        .await
        .expect("top-level DOMContentLoaded must not wait for child frame load")
        .expect("driver loop should succeed");

        assert!(matches!(
            completion,
            PostParseLifecycleCompletionAction::ReturnAtStage("DOMContentLoaded")
        ));

        server.abort();
    });
}

#[test]
fn domcontentloaded_stage_does_not_wait_for_media_response() {
    run_page_vm_local_runtime_async_test("page-vm-dcl-before-media-response", || async move {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
        let addr = listener.local_addr().expect("media server addr");
        let media_url = format!("http://{addr}/slow-media.mp3");
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept media request");
            read_http_request_head(&mut stream)
                .await
                .expect("read media request");
            sleep(Duration::from_secs(5)).await;
        });

        let document_url =
            Url::parse(&format!("http://{addr}/page.html")).expect("main document URL");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();
        page_vm
            .vm_mut()
            .eval(&format!(
                r#"
                (() => {{
                    const media = document.createElement("audio");
                    media.preload = "auto";
                    media.src = {media_url:?};
                    document.body.appendChild(media);
                }})()
                "#,
            ))
            .expect("media setup should evaluate");
        let lifecycle_driver = {
            let PageVm {
                vm,
                page_task_queue,
                report,
                ..
            } = &mut page_vm;
            vm.as_mut()
                .expect("page vm must retain a live ScriptVm until drop")
                .start_post_parse_lifecycle_round(
                    PageVmInitStage::DomContentLoaded,
                    page_task_queue,
                    report,
                    Vec::new(),
                )
                .await
        };
        let lifecycle = tokio::time::timeout(
            Duration::from_secs(1),
            local_executor.run(page_vm.drive_post_parse_lifecycle_loop_on_named_owner_lane(
                PageVmInitStage::DomContentLoaded,
                lifecycle_driver,
            )),
        )
        .await;
        let completion = match lifecycle {
            Ok(result) => result.expect("DOMContentLoaded lifecycle should run"),
            Err(_) => panic!(
                "DOMContentLoaded waited for media: ready_state={}, dcl_dispatched={}, pending_load_delays={}",
                page_vm.vm().document_runtime.host_document().ready_state(),
                page_vm
                    .vm()
                    .document_runtime
                    .dom_content_loaded_dispatched(),
                page_vm
                    .vm()
                    .has_pending_load_event_delaying_subresource_requests(),
            ),
        };

        assert!(matches!(
            completion,
            PostParseLifecycleCompletionAction::ReturnAtStage("DOMContentLoaded")
        ));
        server.abort();
    });
}

#[tokio::test]
async fn child_frame_load_waits_for_nested_url_frames() {
    run_page_vm_async_test(async move {
        let (base_url, server) = spawn_path_response_http_server(vec![
            (
                "/frames/level1.html",
                "HTTP/1.1 200 OK",
                r#"<!doctype html><iframe src="level2.html"></iframe>"#.to_owned(),
                Duration::from_millis(0),
            ),
            (
                "/frames/level2.html",
                "HTTP/1.1 200 OK",
                r#"<!doctype html><iframe src="level3.html"></iframe>"#.to_owned(),
                Duration::from_millis(0),
            ),
            (
                "/frames/level3.html",
                "HTTP/1.1 200 OK",
                r#"<!doctype html><iframe src="level4.html"></iframe>"#.to_owned(),
                Duration::from_millis(0),
            ),
            (
                "/frames/level4.html",
                "HTTP/1.1 200 OK",
                r#"<!doctype html><p id="marker">level4</p>"#.to_owned(),
                Duration::from_millis(0),
            ),
        ])
        .await;
        let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
        let mut page_vm = test_page_vm_with_document_url(document_url);
        let local_executor = page_vm.local_executor.clone();

        let result = local_executor
            .run(async move {
                page_vm.vm_mut().eval(
                    r##"
                    (() => {
                        globalThis.__nestedFrameLoadDone = false;
                        globalThis.__nestedFrameLoadResult = "not-run";
                        const frame = document.createElement("iframe");
                        frame.onload = () => {
                            try {
                                const level1 = frame.contentWindow;
                                const level2 = level1.frames[0];
                                const level3 = level2.frames[0];
                                const level4 = level3.frames[0];
                                globalThis.__nestedFrameLoadResult = [
                                    level1.length,
                                    level2.length,
                                    level3.length,
                                    level4.document.querySelector("#marker").textContent,
                                    level2.parent === level1 &&
                                        level3.parent === level2 &&
                                        level4.parent === level3 &&
                                        level4.top === window
                                ].join("|");
                            } catch (error) {
                                globalThis.__nestedFrameLoadResult =
                                    `${error.name}:${error.message}`;
                            } finally {
                                globalThis.__nestedFrameLoadDone = true;
                            }
                        };
                        frame.src = "/frames/level1.html";
                        document.body.appendChild(frame);
                    })()
                    "##,
                )?;
                drive_websocket_until_done(
                    &mut page_vm,
                    "String(globalThis.__nestedFrameLoadDone === true)",
                    "nested URL child frame load should wait for descendants",
                )
                .await?;
                page_vm.vm_mut().eval("globalThis.__nestedFrameLoadResult")
            })
            .await
            .expect("nested frame load test should run on owner lane");

        server.await.expect("nested frame server should finish");
        assert_eq!(result, "1|1|1|level4|true");
    })
    .await;
}

#[test]
fn drive_post_parse_lifecycle_loop_returns_at_load_after_boundary_task() {
    run_page_vm_local_runtime_async_test("page-vm-load-boundary-task", || async move {
        let mut page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        let (completion, detached_runs, task_queue_empty) = local_executor
            .run(async move {
                let detached_run = detached_test_run();
                let lifecycle_driver = {
                    let PageVm {
                        vm,
                        page_task_queue,
                        report,
                        ..
                    } = &mut page_vm;
                    vm.as_mut()
                        .expect("page vm must retain a live ScriptVm until drop")
                        .start_post_parse_lifecycle_round(
                            PageVmInitStage::Load,
                            page_task_queue,
                            report,
                            vec![post_parse_detached_runs_work(vec![detached_run])],
                        )
                        .await
                };
                let completion = page_vm
                    .drive_post_parse_lifecycle_loop_on_named_owner_lane(
                        PageVmInitStage::Load,
                        lifecycle_driver,
                    )
                    .await?;
                Ok::<_, anyhow::Error>((
                    completion,
                    page_vm.report.runs.len(),
                    page_vm.page_task_queue.is_empty(),
                ))
            })
            .await
            .expect("driver loop should succeed");

        assert!(matches!(
            completion,
            PostParseLifecycleCompletionAction::ReturnAtStage("Load")
        ));
        assert_eq!(detached_runs, 1);
        assert!(
            task_queue_empty,
            "load boundary task should run before returning at Load"
        );
    });
}

#[test]
fn finish_post_parse_execution_returns_at_domcontentloaded_without_detached_tail() {
    run_page_vm_local_runtime_async_test("page-vm-dcl-with-detached-tail", || async move {
        let page_vm = test_page_vm();
        let local_executor = page_vm.local_executor.clone();

        let (detached_runs, trailing_task_remained) = local_executor
            // Keep the lifecycle state machine on the heap while preserving
            // the production named-owner turn and its synchronous DCL boundary.
            .run(Box::pin(async move {
                let mut page_vm = match page_vm
                    .finish_post_parse_execution_on_named_owner_lane(
                        vec![post_parse_detached_runs_work(vec![detached_test_run()])],
                        PageVmInitStage::DomContentLoaded,
                        Instant::now(),
                    )
                    .await?
                {
                    PageVmNavigationTurnOutcome::Completed(page_vm) => page_vm,
                    PageVmNavigationTurnOutcome::TriggeredNavigation => {
                        panic!("test fixture should not trigger location navigation")
                    }
                };
                let detached_runs = page_vm.report.runs.len();
                let trailing_task_remained = matches!(
                    page_vm.page_task_queue.post_parse_front(),
                    Some(work)
                        if matches!(
                            work.as_page_task(),
                            Some(PageTask::RecordDetachedPostParseRuns(_))
                        )
                );
                Ok::<_, anyhow::Error>((detached_runs, trailing_task_remained))
            }))
            .await
            .expect("post-parse finish should succeed");

        assert_eq!(detached_runs, 0);
        assert!(
            trailing_task_remained,
            "DCL stage must return before draining detached post-DCL tail"
        );
    });
}

#[test]
fn finish_post_parse_execution_triggers_immediate_meta_refresh_after_load() {
    run_page_vm_large_stack_async_test("page-vm-load-meta-refresh", || async move {
        let mut page_vm = test_page_vm();
        page_vm
            .vm_mut()
            .eval(
                r#"
                document.head.innerHTML = '<meta http-equiv="refresh" content="0;next.html">';
                'ready'
                "#,
            )
            .expect("install meta refresh");
        let local_executor = page_vm.local_executor.clone();

        let (mut page_vm, internal_loading_outcome) = local_executor
            .run(async move {
                let outcome = page_vm
                    .finish_post_parse_execution_on_named_owner_lane(
                        Vec::new(),
                        PageVmInitStage::Load,
                        Instant::now(),
                    )
                    .await?;
                let PageVmNavigationTurnOutcome::Completed(mut page_vm) = outcome else {
                    panic!("load must complete before immediate meta refresh navigation");
                };
                assert!(
                    !page_vm.vm().has_pending_location_navigation(),
                    "load dispatch must not activate meta refresh inline"
                );
                let internal_loading_outcome = page_vm
                    .run_internal_loading_body_for_test()
                    .expect("load completion should publish one typed internal-loading task");
                Ok::<_, anyhow::Error>((page_vm, internal_loading_outcome))
            })
            .await
            .expect("post-parse finish should succeed");

        assert_eq!(
            internal_loading_outcome.action.target_effect,
            crate::page_task_queue::PageInternalLoadingTargetEffect::AppliedToCurrentOwner {
                effect: crate::page_task_queue::PageOwnedInternalLoadingTaskEffect::MetaRefreshNavigationActivated,
            }
        );
        let pending = page_vm
            .vm_mut()
            .take_pending_location_navigation_with_seed()
            .expect("meta refresh pending navigation");
        assert_eq!(pending.url.as_str(), "https://example.com/next.html");
    });
}

#[test]
fn immediate_meta_refresh_task_drops_after_document_replacement() {
    run_page_vm_large_stack_async_test("page-vm-stale-meta-refresh", || async move {
        let mut page_vm = test_page_vm();
        page_vm
            .vm_mut()
            .eval(
                r#"
                document.head.innerHTML = '<meta http-equiv="refresh" content="0;stale.html">';
                'ready'
                "#,
            )
            .expect("install meta refresh");
        let original_owner = page_vm
            .vm()
            .current_main_document_task_owner()
            .expect("test page should have a document owner");
        let local_executor = page_vm.local_executor.clone();

        let mut page_vm = local_executor
            .run(async move {
                let outcome = page_vm
                    .finish_post_parse_execution_on_named_owner_lane(
                        Vec::new(),
                        PageVmInitStage::Load,
                        Instant::now(),
                    )
                    .await?;
                let PageVmNavigationTurnOutcome::Completed(page_vm) = outcome else {
                    panic!("load must complete before immediate meta refresh navigation");
                };
                Ok::<_, anyhow::Error>(page_vm)
            })
            .await
            .expect("post-parse finish should succeed");

        page_vm
            .vm_mut()
            .eval("document.open(); document.write('<p>replacement</p>'); document.close();")
            .expect("replace document before refresh task runs");
        assert_ne!(
            page_vm.vm().current_main_document_task_owner(),
            Some(original_owner),
            "document.open should rotate the exact Document owner"
        );
        let outcome = page_vm
            .run_internal_loading_body_for_test()
            .expect("old document refresh task should remain available for stale validation");
        assert!(matches!(
            outcome.action.target_effect,
            crate::page_task_queue::PageInternalLoadingTargetEffect::DiscardedStaleOwner {
                current_owner: Some(_),
            }
        ));
        assert!(
            !page_vm.vm().has_pending_location_navigation(),
            "retired document refresh must not navigate its replacement"
        );
    });
}

#[test]
fn finish_post_parse_execution_load_drains_boundary_task() {
    run_page_vm_local_runtime_test("page-vm-load-boundary-task-finish", || async {
        run_page_vm_async_test(async move {
            let page_vm = test_page_vm();
            let local_executor = page_vm.local_executor.clone();

            let (detached_runs, task_queue_empty) =
                PageVm::run_bootstrap_future_on_fresh_local_task(
                    local_executor,
                    "post-parse load boundary test local task channel closed",
                    Box::pin(async move {
                        let mut page_vm = match page_vm
                            .finish_post_parse_execution_on_named_owner_lane(
                                vec![post_parse_detached_runs_work(vec![detached_test_run()])],
                                PageVmInitStage::Load,
                                Instant::now(),
                            )
                            .await?
                        {
                            PageVmNavigationTurnOutcome::Completed(page_vm) => page_vm,
                            PageVmNavigationTurnOutcome::TriggeredNavigation => {
                                panic!("test fixture should not trigger location navigation")
                            }
                        };
                        Ok::<_, anyhow::Error>((
                            page_vm.report.runs.len(),
                            page_vm.page_task_queue.is_empty(),
                        ))
                    }),
                )
                .await
                .expect("post-parse finish should succeed");

            assert_eq!(detached_runs, 1);
            assert!(
                task_queue_empty,
                "Load stage must drain the boundary task before returning"
            );
        })
        .await;
    });
}

#[test]
fn finish_post_parse_window_load_allows_nested_child_click_listener() {
    run_page_vm_local_runtime_test("page-vm-window-load-nested-click", || async {
        run_page_vm_async_test(async move {
            let mut page_vm = test_page_vm();
            page_vm
                .vm_mut()
                .eval(
                    r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  body.innerHTML = `
    <form id=form1>
      <button id=submitbutton type=submit>
        <span id=outerchild>
          <span id=innerchild>submit</span>
        </span>
      </button>
    </form>`;
  globalThis.__postParseLoadClickEvents = [];
  window.addEventListener('load', () => {
    const button = document.getElementById('submitbutton');
    const inner = document.getElementById('innerchild');
    button.addEventListener('click', event => {
      globalThis.__postParseLoadClickEvents.push([
        event.target === inner,
        event.currentTarget === button,
        event.bubbles
      ].join(':'));
      event.preventDefault();
    });
    inner.click();
  });
})()
"#,
                )
                .expect("post-parse load click setup should evaluate");

            let local_executor = page_vm.local_executor.clone();
            let mut page_vm = match PageVm::run_bootstrap_future_on_fresh_local_task(
                local_executor,
                "post-parse window-load nested click test local task channel closed",
                Box::pin(async move {
                    page_vm
                        .finish_post_parse_execution_on_named_owner_lane(
                            Vec::new(),
                            PageVmInitStage::Load,
                            Instant::now(),
                        )
                        .await
                }),
            )
            .await
            .expect("post-parse load should complete")
            {
                PageVmNavigationTurnOutcome::Completed(page_vm) => page_vm,
                PageVmNavigationTurnOutcome::TriggeredNavigation => {
                    panic!("test fixture should not trigger location navigation")
                }
            };

            let result = page_vm
                .vm_mut()
                .eval("globalThis.__postParseLoadClickEvents.join('|')")
                .expect("post-parse load click events should be readable");

            assert_eq!(result, "true:true:true");
        })
        .await;
    });
}

#[test]
fn post_parse_load_waits_for_main_media_loadeddata_owner_turn() {
    run_page_vm_local_runtime_test("page-vm-main-media-load-owner-turn", || async {
        run_page_vm_async_test(async move {
            let mut page_vm = test_page_vm();
            page_vm
                .vm_mut()
                .eval(
                    r#"
(() => {
  globalThis.__mainMediaLoadOwnerTurnEvents = [];
  const body = document.body;
  const media = document.createElement('video');
  for (const type of ['loadstart', 'loadedmetadata', 'loadeddata', 'canplay']) {
    media.addEventListener(type, () => {
      __mainMediaLoadOwnerTurnEvents.push(`${type}:${document.readyState}`);
    });
  }
  window.addEventListener('load', () => {
    __mainMediaLoadOwnerTurnEvents.push(`window:${document.readyState}`);
  });
  media.src = 'data:video/webm;base64,AA==';
  body.appendChild(media);
})()
"#,
                )
                .expect("main media load owner-turn setup should evaluate");

            let local_executor = page_vm.local_executor.clone();
            let mut page_vm = match PageVm::run_bootstrap_future_on_fresh_local_task(
                local_executor,
                "post-parse main media load owner-turn local task channel closed",
                Box::pin(async move {
                    page_vm
                        .finish_post_parse_execution_on_named_owner_lane(
                            Vec::new(),
                            PageVmInitStage::Load,
                            Instant::now(),
                        )
                        .await
                }),
            )
            .await
            .expect("post-parse main media load should complete")
            {
                PageVmNavigationTurnOutcome::Completed(page_vm) => page_vm,
                PageVmNavigationTurnOutcome::TriggeredNavigation => {
                    panic!("test fixture should not trigger location navigation")
                }
            };

            assert_eq!(
                page_vm
                    .vm_mut()
                    .eval(
                        r#"
(() => {
  const events = globalThis.__mainMediaLoadOwnerTurnEvents;
  const loadeddata = events.findIndex(event => event.startsWith('loadeddata:'));
  const windowLoad = events.findIndex(event => event.startsWith('window:'));
  return JSON.stringify({
    events,
    readyState: document.readyState,
    loadeddataBeforeWindow: loadeddata >= 0 && windowLoad > loadeddata,
    duplicateMediaEvents: ['loadstart', 'loadedmetadata', 'loadeddata', 'canplay'].some(
      type => events.filter(event => event.startsWith(type + ':')).length > 1
    )
  });
})()
"#,
                    )
                    .expect("main media/load event order should evaluate"),
                r#"{"events":["loadstart:interactive","loadedmetadata:interactive","loadeddata:interactive","window:complete"],"readyState":"complete","loadeddataBeforeWindow":true,"duplicateMediaEvents":false}"#,
                "Window load must be a later lifecycle turn than loadeddata settlement"
            );
        })
        .await;
    });
}

#[test]
fn child_history_back_ignores_unload_location_navigation() {
    run_page_vm_large_stack_async_test(
        "page-vm-child-history-back-unload-navigation",
        || async move {
            let (base_url, shutdown_server, server) =
                spawn_shutdown_path_response_http_server(vec![
                    (
                        "/003-1.html",
                        "HTTP/1.1 200 OK",
                        r#"<!doctype html>
<script>
onload = function() {
  parent.__childHistoryBackTimerId = String(setTimeout(function() { location = "003-2.html"; }, 0));
  parent.postMessage("003-1", "*");
}
</script>"#
                            .to_owned(),
                        Duration::from_millis(0),
                    ),
                    (
                        "/003-1.html",
                        "HTTP/1.1 200 OK",
                        r#"<!doctype html>
<script>
onload = function() {
  parent.postMessage("003-1", "*");
}
</script>"#
                            .to_owned(),
                        Duration::from_millis(0),
                    ),
                    (
                        "/003-2.html",
                        "HTTP/1.1 200 OK",
                        r#"<!doctype html>
003-2
<script>
onload = function() {
  parent.postMessage("003-2", "*");
  setTimeout(function() { history.go(-1); }, 0);
}
onunload = function() { location = "003-3.html"; }
</script>"#
                            .to_owned(),
                        Duration::from_millis(0),
                    ),
                    (
                        "/003-3.html",
                        "HTTP/1.1 200 OK",
                        r#"<!doctype html><script>parent.postMessage("003-3", "*");</script>"#
                            .to_owned(),
                        Duration::from_millis(0),
                    ),
                ])
                .await;
            let document_url = Url::parse(&format!("{base_url}/page.html")).expect("document url");
            let mut page_vm = test_page_vm_with_document_url(document_url);
            let local_executor = page_vm.local_executor.clone();

            let result = local_executor
                .run(async move {
                    page_vm.vm_mut().eval(
                        r#"
(() => {
  globalThis.__childHistoryBackMessages = [];
  onmessage = event => {
    __childHistoryBackMessages.push(event.data);
  };
  const frame = document.createElement("iframe");
  frame.src = "003-1.html";
  document.body.appendChild(frame);
})()
"#,
                    )?;
                    drive_websocket_until_done(
                        &mut page_vm,
                        "globalThis.__childHistoryBackMessages.length >= 3",
                        "child history back unload navigation should complete",
                    )
                    .await?;
                    page_vm
                        .vm_mut()
                        .eval("globalThis.__childHistoryBackMessages.join('|')")
                })
                .await
                .expect("child history back unload test should run on owner lane");

            let _ = shutdown_server.send(());
            let requested_paths = server
                .await
                .expect("child history back server should finish");
            assert_eq!(result, "003-1|003-2|003-1");
            assert!(
                !requested_paths.iter().any(|path| path == "/003-3.html"),
                "unload location navigation should not win history traversal: {requested_paths:?}"
            );
        },
    );
}
