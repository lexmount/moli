use moli_core::{
    PageId, RendererDocumentLifecycleIdentity, RendererOutputCursor, RendererOutputFence,
    RendererOutputItem, RendererOutputPublication, RendererOutputPublicationOrdering,
    RendererOutputRecord, RendererOutputStreamControl, RendererOutputStreamIdentity,
    RendererOutputTransportMessage, RendererProtocolObservation,
    RendererRuntimeInspectorAsyncCompletion,
    page::{
        RendererDocumentLifecycleEvent, RendererDocumentLifecycleEventKind,
        RendererDocumentLifecycleMilestone, RendererDocumentToken, RendererFrameToken,
        RendererLifecycleEpoch,
    },
};
use moli_protocol::{
    BackgroundProtocolEvent, CdpConnection, CdpSchedulerEvent,
    DeferredMainDocumentLoadCompletionOutputAction,
    DeferredMainDocumentLoadCompletionOutputInterest, DeferredMainDocumentLoadPredecessorCandidate,
    ProtocolSchedulerWork,
    automation::{
        AutomationCommand, AutomationContext, AutomationEvent, AutomationResult,
        BrowserDownloadWillBeginEvent, DevToolsCreateTargetCommand, DevToolsFrameId,
        DevToolsLoaderId, DevToolsNavigationWait, DevToolsNetworkResourceType, DevToolsRequestId,
        DevToolsSessionId, DevToolsTargetId, FrontendProtocol, NavigationFrameEvent,
        NavigationFrameEventKind, NetworkRequestEvent,
    },
    conn::RuntimeInspectorResponseReady,
    test_support::{
        arm_background_navigation_request, deferred_main_document_load_observation_id,
        deferred_main_document_load_output_interest,
        root_frame_stopped_loading_work as make_root_frame_stopped_loading_work,
        root_frame_stopped_loading_work_for_target, settle_background_navigation_request,
    },
};
use serde_json::json;

use super::{
    CdpScheduler, ForegroundNavigationNetworkBarrier, ProtocolOutputSequence,
    ProtocolSchedulerResidence, ProtocolSchedulerStep, SchedulerQueues,
    devtools_navigation_lifecycle_milestone, drain_pending_background_events,
    next_page_screencast_deadline, page_screencast_interval,
};

#[test]
fn screencast_deadlines_respect_configured_milliseconds_without_catch_up() {
    let started = tokio::time::Instant::now();
    assert_eq!(
        page_screencast_interval(1000, 1),
        std::time::Duration::from_secs(1)
    );
    assert_eq!(
        page_screencast_interval(1000, 3),
        std::time::Duration::from_secs(3)
    );

    let completion = started + std::time::Duration::from_secs(10);
    assert_eq!(
        next_page_screencast_deadline(completion, page_screencast_interval(1000, 1)),
        started + std::time::Duration::from_secs(11),
        "an overdue capture schedules from completion instead of catching up"
    );
    assert_eq!(
        page_screencast_interval(17, 1),
        std::time::Duration::from_millis(17)
    );
    assert_eq!(
        page_screencast_interval(17, 3),
        std::time::Duration::from_millis(51)
    );
    assert_eq!(
        next_page_screencast_deadline(completion, page_screencast_interval(17, 3)),
        completion + std::time::Duration::from_millis(51)
    );
    assert_eq!(
        page_screencast_interval(u32::MAX, 2),
        std::time::Duration::from_millis(8_589_934_590),
        "multiplying the millisecond interval must not overflow u32"
    );
}

#[test]
fn navigation_wait_maps_only_observable_document_milestones() {
    use moli_core::page::RendererDocumentLifecycleMilestone;

    assert_eq!(
        devtools_navigation_lifecycle_milestone(Some(DevToolsNavigationWait::DomContentLoaded)),
        Some(RendererDocumentLifecycleMilestone::DomContentLoaded)
    );
    assert_eq!(
        devtools_navigation_lifecycle_milestone(Some(DevToolsNavigationWait::Load)),
        Some(RendererDocumentLifecycleMilestone::Load)
    );
    assert_eq!(
        devtools_navigation_lifecycle_milestone(Some(DevToolsNavigationWait::None)),
        None
    );
    assert_eq!(devtools_navigation_lifecycle_milestone(None), None);
}

#[tokio::test]
async fn dialog_handling_reaches_its_owner_during_pending_document_admission() {
    let mut conn = CdpConnection::new();
    let navigation = arm_background_navigation_request(&mut conn, "LOADER-dialog-control");
    let target = navigation.target_id().to_owned();
    let (mut scheduler, protocol_work_completion_rx) = CdpScheduler::new(conn);
    let mut receivers = super::CdpSchedulerEventReceivers {
        state_events_ready: Default::default(),
        protocol_work_completion_rx,
        document_activity_completion_rx: tokio::sync::mpsc::unbounded_channel().1,
        background_event_rx: tokio::sync::mpsc::unbounded_channel().1,
        background_navigation_completion_rx: tokio::sync::mpsc::unbounded_channel().1,
        renderer_publication_rx: moli_core::renderer_output_transport_channel().1,
    };
    let (ready, _ready_rx) = tokio::sync::mpsc::unbounded_channel();
    let command = AutomationCommand::HandleJavaScriptDialog(
        moli_protocol::automation::DevToolsHandleJavaScriptDialogCommand {
            context: AutomationContext {
                protocol: FrontendProtocol::WebDriverBidi,
                session_id: None,
                target_id: Some(target.clone().into()),
                browser_context_id: None,
            },
            accept: true,
            prompt_text: Some(String::new()),
        },
    );
    let progress = scheduler
        .start_owned_automation_command_progress(&mut receivers, &ready, command, None)
        .await
        .expect("handling an existing dialog must not wait for a replacement Document");
    let super::AutomationCommandProgress::Complete(execution) = progress else {
        panic!("dialog handling is a finite owner operation");
    };
    assert_eq!(
        execution.result.unwrap_err().kind,
        moli_protocol::automation::DevToolsErrorKind::NoSuchAlert,
        "the real handler must check the target's dialog even while navigation is pending"
    );
    assert!(
        scheduler
            .state
            .with_state(|state| state.has_inflight_background_navigation_for_target(&target))
            .unwrap()
    );
    assert!(!navigation.is_cancelled());
}

fn classic_alert_command(operation: &str, context: AutomationContext) -> AutomationCommand {
    use moli_protocol::automation::{
        DevToolsGetJavaScriptDialogCommand, DevToolsHandleJavaScriptDialogCommand,
        DevToolsSetJavaScriptDialogPromptTextCommand,
    };
    match operation {
        "get" => {
            AutomationCommand::GetJavaScriptDialog(DevToolsGetJavaScriptDialogCommand { context })
        }
        "set" => AutomationCommand::SetJavaScriptDialogPromptText(
            DevToolsSetJavaScriptDialogPromptTextCommand {
                context,
                prompt_text: "Classic text".to_owned(),
            },
        ),
        "accept" | "dismiss" => {
            AutomationCommand::HandleJavaScriptDialog(DevToolsHandleJavaScriptDialogCommand {
                context,
                accept: operation == "accept",
                prompt_text: None,
            })
        }
        _ => panic!("unknown alert operation"),
    }
}

#[tokio::test]
async fn classic_alert_wait_is_interrupted_by_a_dialog_not_by_a_blanket_exemption() {
    use moli_core::page::{
        RendererJavaScriptDialogCompletion, RendererJavaScriptDialogId,
        RendererJavaScriptDialogSource, RendererPendingJavaScriptDialog,
    };
    for operation in ["get", "set", "accept", "dismiss"] {
        for dialog_already_open in [false, true] {
            let mut conn = CdpConnection::new();
            let navigation = arm_background_navigation_request(&mut conn, "LOADER-classic-alert");
            let context = AutomationContext {
                protocol: FrontendProtocol::WebDriverClassic,
                session_id: None,
                target_id: Some(navigation.target_id().into()),
                browser_context_id: None,
            };
            let completion = RendererJavaScriptDialogCompletion::pending();
            let page_id = PageId::new_for_testing(77);
            let dialog = RendererPendingJavaScriptDialog::new(
                RendererJavaScriptDialogId::new(1),
                RendererDocumentLifecycleIdentity {
                    frame: RendererFrameToken { page_id },
                    document: RendererDocumentToken::new_for_testing(page_id, 1),
                    epoch: RendererLifecycleEpoch(1),
                },
                RendererJavaScriptDialogSource::RootFrame,
                "about:blank".to_owned(),
                "prompt".to_owned(),
                "Classic prompt".to_owned(),
                String::new(),
                Some(completion.clone()),
            );
            let (mut scheduler, protocol_work_completion_rx) = CdpScheduler::new(conn);
            let state = scheduler.state.handle();
            let (document_tx, document_activity_completion_rx) =
                tokio::sync::mpsc::unbounded_channel();
            let (background_tx, background_event_rx) = tokio::sync::mpsc::unbounded_channel();
            let (navigation_tx, background_navigation_completion_rx) =
                tokio::sync::mpsc::unbounded_channel();
            let (renderer_tx, renderer_publication_rx) =
                moli_core::renderer_output_transport_channel();
            let ready = scheduler.state.scheduler_events_ready();
            let mut receivers = super::CdpSchedulerEventReceivers {
                state_events_ready: ready.clone(),
                protocol_work_completion_rx,
                document_activity_completion_rx,
                background_event_rx,
                background_navigation_completion_rx,
                renderer_publication_rx,
            };
            let install = || {
                state
                    .with_state(|state| {
                        state.push_page_javascript_dialog_for_test(&context, dialog);
                    })
                    .unwrap();
                ready.notify_one();
            };
            let command = classic_alert_command(operation, context.clone());
            let execution = if dialog_already_open {
                install();
                let wait = scheduler
                    .execute_automation_command_with_external_load_wait(&mut receivers, command);
                tokio::pin!(wait);
                match futures_util::poll!(&mut wait) {
                    std::task::Poll::Ready(execution) => execution,
                    std::task::Poll::Pending => {
                        panic!("{operation} must reach the existing prompt")
                    }
                }
            } else {
                let wait = scheduler
                    .execute_automation_command_with_external_load_wait(&mut receivers, command);
                tokio::pin!(wait);
                assert!(
                    futures_util::poll!(&mut wait).is_pending(),
                    "{operation} must still wait when no prompt exists"
                );
                install();
                match futures_util::poll!(&mut wait) {
                    std::task::Poll::Ready(execution) => execution,
                    std::task::Poll::Pending => {
                        panic!("{operation} must recheck the prompt during navigation wait")
                    }
                }
            };
            let result = execution.expect("finite alert operation");
            if operation == "get" {
                let AutomationResult::JavaScriptDialog(dialog) = result else {
                    panic!("dialog result")
                };
                assert_eq!(dialog.message, "Classic prompt");
            } else {
                assert!(matches!(result, AutomationResult::Empty));
            }
            assert!(
                state
                    .with_state(|state| state
                        .has_inflight_background_navigation_for_target(navigation.target_id()))
                    .unwrap()
            );
            assert!(!navigation.is_cancelled());
            if matches!(operation, "get" | "set") {
                let execution = scheduler
                    .execute_automation_command_with_external_load_wait(
                        &mut receivers,
                        classic_alert_command("accept", context),
                    )
                    .await;
                execution.unwrap();
            }
            let response = completion.wait();
            assert_eq!(response.accepted, operation != "dismiss");
            assert_eq!(
                response.user_input,
                if operation == "set" {
                    "Classic text"
                } else {
                    ""
                }
            );
            assert!(!completion.finish(false, String::new()));
            drop((document_tx, background_tx, navigation_tx, renderer_tx));
        }
    }
}

fn navigation_started_automation_event() -> AutomationEvent {
    AutomationEvent::NavigationFrame(NavigationFrameEvent {
        target_id: DevToolsTargetId::from("TID-output"),
        frame_id: DevToolsFrameId::from("TID-output"),
        parent_frame_id: None,
        loader_id: Some(DevToolsLoaderId::from("LOADER-output")),
        url: "https://example.test/output".to_owned(),
        kind: NavigationFrameEventKind::StartedNavigating,
        frame_name: None,
        security_origin: None,
        secure_context_type: None,
    })
}

#[test]
fn protocol_output_sequence_preserves_background_event_sidecar() {
    let message = json!({
        "method": "Page.frameStartedNavigating",
        "params": {
            "frameId": "TID-output",
            "loaderId": "LOADER-output",
            "url": "https://example.test/output",
            "navigationType": "differentDocument"
        }
    });
    let automation_event = navigation_started_automation_event();

    let output = ProtocolOutputSequence::from_background_event(
        BackgroundProtocolEvent::immediate_automation_event(
            message.clone(),
            automation_event.clone(),
        ),
    );
    let mut events = output.into_background_events();
    let (actual_message, actual_automation_event) = events
        .pop()
        .expect("protocol output should keep event")
        .into_parts();

    assert_eq!(actual_message, message);
    assert_eq!(actual_automation_event, Some(automation_event));
}

#[test]
fn load_ordering_splits_network_observations_from_page_side_effects() {
    let network = network_response_event(DevToolsNetworkResourceType::EventSource, "REQ-sse");
    let page_effect = download_will_begin_event("FRAME-nav", "download-after-load");
    let output =
        ProtocolOutputSequence::from_background_events(vec![network.clone(), page_effect.clone()]);

    let (load_independent, load_ordered) = output.split_network_observations();

    assert_eq!(load_independent.into_background_events(), vec![network]);
    assert_eq!(load_ordered.into_background_events(), vec![page_effect]);
}

fn download_will_begin_event(frame_id: &str, guid: &str) -> BackgroundProtocolEvent {
    BackgroundProtocolEvent::immediate_automation_event(
        json!({
            "method": "Browser.downloadWillBegin",
            "params": {
                "frameId": frame_id,
                "guid": guid,
                "url": "https://example.test/download",
                "suggestedFilename": "download.bin"
            }
        }),
        AutomationEvent::BrowserDownloadWillBegin(BrowserDownloadWillBeginEvent {
            frame_id: DevToolsFrameId::from(frame_id),
            guid: guid.to_owned(),
            url: "https://example.test/download".to_owned(),
            suggested_filename: "download.bin".to_owned(),
        }),
    )
}

#[test]
fn navigation_download_observation_ignores_output_before_command_boundary() {
    let mut output = ProtocolOutputSequence::from_background_event(download_will_begin_event(
        "FRAME-nav",
        "old-download",
    ));
    let command_output_start = output.len();

    assert!(!output.contains_download_start_for_frame_since("FRAME-nav", command_output_start,));

    output.append(ProtocolOutputSequence::from_background_event(
        download_will_begin_event("FRAME-nav", "current-download"),
    ));
    assert!(output.contains_download_start_for_frame_since("FRAME-nav", command_output_start,));
}

#[test]
fn drain_pending_background_events_preserves_typed_sidecar() {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let message = json!({
        "method": "Page.frameStartedNavigating",
        "params": {
            "frameId": "TID-output",
            "loaderId": "LOADER-output",
            "url": "https://example.test/output",
            "navigationType": "differentDocument"
        }
    });
    let automation_event = navigation_started_automation_event();
    tx.send(BackgroundProtocolEvent::immediate_automation_event(
        message.clone(),
        automation_event.clone(),
    ))
    .expect("background event receiver should be open");

    let output = drain_pending_background_events(&mut rx);
    let mut events = output.into_background_events();
    let (actual_message, actual_automation_event) = events
        .pop()
        .expect("renderer publication ingress should preserve event")
        .into_parts();

    assert_eq!(actual_message, message);
    assert_eq!(actual_automation_event, Some(automation_event));
}

fn runtime_response_ready_event(command_id: u64) -> BackgroundProtocolEvent {
    BackgroundProtocolEvent::runtime_inspector_response_ready(RuntimeInspectorResponseReady::new(
        command_id,
        None,
        Ok(
            RendererRuntimeInspectorAsyncCompletion::from_protocol_message(
                i32::try_from(command_id).expect("test command id should fit i32"),
                json!({
                    "id": command_id,
                    "result": {}
                }),
            ),
        ),
    ))
}

#[test]
fn protocol_output_loose_response_take_ignores_typed_runtime_response() {
    let mut output =
        ProtocolOutputSequence::from_background_event(runtime_response_ready_event(42));

    let events = output.take_protocol_events_with_id(42);

    assert!(
        events.is_empty(),
        "typed runtime responses must not be flattened by loose protocol response scans"
    );
    let Some((prefix, response)) = output.split_next_runtime_response_ready() else {
        panic!("typed runtime response should remain available for typed routing");
    };
    assert!(prefix.is_empty());
    assert_eq!(response.command_id(), 42);
}

#[test]
fn protocol_output_loose_response_split_ignores_typed_runtime_response() {
    let mut output = ProtocolOutputSequence::from_background_events(vec![
        runtime_response_ready_event(42),
        BackgroundProtocolEvent::immediate(json!({
            "id": 7,
            "result": {}
        })),
    ]);

    assert!(
        output
            .split_next_protocol_message_with_any_id(&[42])
            .is_none(),
        "typed runtime responses must not satisfy loose protocol response lookup"
    );
    let Some((mut prefix, command_id, event)) =
        output.split_next_protocol_message_with_any_id(&[7])
    else {
        panic!("ordinary protocol responses should still be found");
    };
    assert_eq!(command_id, 7);
    let message = event.into_protocol_message();
    assert_eq!(message["id"], json!(7));
    let Some((typed_prefix, response)) = prefix.split_next_runtime_response_ready() else {
        panic!("typed runtime response should stay in the prefix for typed routing");
    };
    assert!(typed_prefix.is_empty());
    assert_eq!(response.command_id(), 42);
}

fn network_response_event(
    resource_type: DevToolsNetworkResourceType,
    request_id: &str,
) -> BackgroundProtocolEvent {
    network_response_event_for_target(resource_type, request_id, "TID-nav")
}

fn network_response_event_for_target(
    resource_type: DevToolsNetworkResourceType,
    request_id: &str,
    target_id: &str,
) -> BackgroundProtocolEvent {
    let message = json!({
        "method": "Network.responseReceived",
        "params": {
            "requestId": request_id,
            "type": resource_type.as_cdp_type(),
            "response": {
                "url": "https://example.test/resource",
                "status": 200,
                "fromDiskCache": false
            }
        }
    });
    let automation_event = AutomationEvent::NetworkResponseStarted(NetworkRequestEvent {
        target_id: DevToolsTargetId::from(target_id),
        frame_id: Some(DevToolsFrameId::from("FRAME-nav")),
        request_id: DevToolsRequestId::from(request_id),
        loader_id: Some(DevToolsLoaderId::from("LOADER-nav")),
        url: "https://example.test/resource".to_owned(),
        document_url: None,
        method: None,
        request_headers: Vec::new(),
        request_body: None,
        request_initiator_type: None,
        bidi_request_initiator_type: None,
        redirect_response: None,
        redirect_has_extra_info: false,
        request_cookie_report: None,
        resource_type: Some(resource_type),
        timestamp: Some(1.0),
        wall_time: None,
        status: Some(200),
        status_text: None,
        response_headers: Vec::new(),
        response_mime_type: None,
        response_protocol: None,
        has_extra_info: false,
        encoded_data_length: Some(0),
        from_cache: false,
        fetch_request_id: None,
        error_text: None,
        loading_failed_canceled: false,
        blocked_intercepts: Vec::new(),
        network_id: None,

        auth_challenge: None,
    });
    BackgroundProtocolEvent::immediate_automation_event(message, automation_event)
}

fn network_request_event_for_target(
    resource_type: DevToolsNetworkResourceType,
    request_id: &str,
    target_id: &str,
) -> BackgroundProtocolEvent {
    let (_, Some(AutomationEvent::NetworkResponseStarted(mut network_event))) =
        network_response_event_for_target(resource_type, request_id, target_id).into_parts()
    else {
        unreachable!("network response fixture should retain its typed sidecar")
    };
    network_event.status = None;
    network_event.encoded_data_length = None;
    BackgroundProtocolEvent::immediate_automation_event(
        json!({
            "method": "Network.requestWillBeSent",
            "params": {
                "requestId": request_id,
                "loaderId": "LOADER-nav",
                "type": resource_type.as_cdp_type(),
                "request": {
                    "url": "https://example.test/resource",
                    "method": "GET"
                }
            }
        }),
        AutomationEvent::NetworkBeforeRequestSent(network_event),
    )
}

fn network_finished_event_for_target(
    resource_type: DevToolsNetworkResourceType,
    request_id: &str,
    target_id: &str,
) -> BackgroundProtocolEvent {
    let (_, Some(AutomationEvent::NetworkResponseStarted(network_event))) =
        network_response_event_for_target(resource_type, request_id, target_id).into_parts()
    else {
        unreachable!("network response fixture should retain its typed sidecar")
    };
    BackgroundProtocolEvent::immediate_automation_event(
        json!({
            "method": "Network.loadingFinished",
            "params": {
                "requestId": request_id,
                "timestamp": 2.0,
                "encodedDataLength": 0
            }
        }),
        AutomationEvent::NetworkResponseCompleted(network_event),
    )
}

#[test]
fn mixed_owner_protocol_output_retains_only_its_known_targets() {
    let mut conn = CdpConnection::new();
    let navigation = arm_background_navigation_request(&mut conn, "LOADER-known");
    let target_id = navigation.target_id().to_owned();
    let known = network_response_event_for_target(
        DevToolsNetworkResourceType::Xhr,
        "REQ-known",
        &target_id,
    );
    let owner = moli_protocol::ProtocolStateOwner::new(conn);
    assert_eq!(
        owner
            .with_state(|state| {
                ProtocolOutputSequence::from_background_event(known.clone())
                    .navigation_gate_target_ids(&state)
            })
            .unwrap(),
        std::slice::from_ref(&target_id)
    );

    let unresolved = BackgroundProtocolEvent::immediate(json!({
        "method": "Runtime.consoleAPICalled",
        "sessionId": "SID-missing",
        "params": {}
    }));
    let mixed = ProtocolOutputSequence::from_background_events(vec![known, unresolved]);
    assert_eq!(
        owner
            .with_state(|state| mixed.navigation_gate_target_ids(&state))
            .unwrap(),
        [target_id],
        "an unresolved sibling does not expand a batch's exact Target ownership"
    );
}

fn output_request_ids(output: ProtocolOutputSequence) -> Vec<String> {
    output
        .into_background_events()
        .into_iter()
        .map(|event| {
            let (message, automation_event) = event.into_parts();
            assert!(
                automation_event.is_some(),
                "deferred background events must preserve typed sidecars"
            );
            message["params"]["requestId"]
                .as_str()
                .expect("network event request id")
                .to_owned()
        })
        .collect()
}

fn renderer_output_record(page_id: PageId, sequence: u64) -> RendererOutputRecord {
    RendererOutputRecord::new_for_test(RendererOutputItem::Observation(
        RendererProtocolObservation::DocumentLifecycle(RendererDocumentLifecycleEvent {
            frame: RendererFrameToken { page_id },
            document: RendererDocumentToken::new_for_testing(page_id, 1),
            epoch: RendererLifecycleEpoch(1),
            sequence,
            timestamp_micros: sequence,
            kind: RendererDocumentLifecycleEventKind::Milestone(
                RendererDocumentLifecycleMilestone::DomContentLoaded,
            ),
        }),
    ))
}

fn renderer_publication(
    page_id: PageId,
    sequence: u64,
    ordering: RendererOutputPublicationOrdering,
) -> RendererOutputTransportMessage {
    let stream = RendererOutputStreamIdentity::new_page_for_protocol_test(page_id);
    RendererOutputPublication::new_for_test_with_ordering(
        RendererOutputCursor::new_for_test(stream, sequence),
        ordering,
        vec![renderer_output_record(page_id, sequence)],
    )
    .into()
}

fn unconstrained_renderer_publication(page_id: PageId) -> RendererOutputTransportMessage {
    renderer_publication(page_id, 1, RendererOutputPublicationOrdering::Unconstrained)
}

fn post_load_renderer_publication(
    page_id: PageId,
    lifecycle_document_id: u64,
) -> RendererOutputTransportMessage {
    let source_document = root_document_lifecycle_identity(page_id, lifecycle_document_id);
    renderer_publication(
        page_id,
        1,
        RendererOutputPublicationOrdering::AfterPendingPageLoad { source_document },
    )
}

fn publication_cursor(publication: &RendererOutputTransportMessage) -> RendererOutputCursor {
    let RendererOutputTransportMessage::Publication(publication) = publication else {
        panic!("test fixture must contain one concrete publication");
    };
    publication.cursor()
}

fn root_document_lifecycle_identity(
    page_id: PageId,
    identity_value: u64,
) -> RendererDocumentLifecycleIdentity {
    RendererDocumentLifecycleIdentity {
        frame: RendererFrameToken { page_id },
        document: RendererDocumentToken::new_for_testing(page_id, identity_value),
        epoch: RendererLifecycleEpoch(identity_value),
    }
}

fn root_frame_stopped_loading_work(publish_sequence: u64, frame_id: &str) -> ProtocolSchedulerWork {
    make_root_frame_stopped_loading_work(
        publish_sequence,
        vec![Some("SID-nav".to_owned())],
        frame_id.to_owned(),
        format!("LOADER-{frame_id}"),
    )
}

#[test]
fn delayed_command_wake_cannot_hold_or_reorder_published_work() {
    use moli_protocol::CommandResponseFlushState;
    use moli_protocol::test_support::{
        command_output_with_scheduler_events, publish_protocol_work,
    };

    let mut conn = CdpConnection::new();
    let (permit, context) = conn.begin_command_response_flush_permit();
    publish_protocol_work(&mut conn, |sequence| {
        root_frame_stopped_loading_work(sequence, "FRAME-first")
    });
    let held_command = command_output_with_scheduler_events(conn.take_scheduler_events());
    publish_protocol_work(&mut conn, |sequence| {
        root_frame_stopped_loading_work(sequence, "FRAME-second")
    });
    let later_wake = conn.take_scheduler_events();
    assert!(matches!(
        later_wake.as_slice(),
        [CdpSchedulerEvent::ProtocolWorkReady]
    ));
    let (mut scheduler, _receivers) = CdpScheduler::new(conn);

    // The first command has not even materialized its output yet. A later
    // wake still admits both concrete publications, in their original order.
    scheduler.apply_scheduler_events(later_wake);
    let sequences = scheduler
        .queues
        .protocol_residences
        .iter()
        .map(|residence| match residence {
            ProtocolSchedulerResidence::ProtocolWork { work, .. } => work.publish_sequence().get(),
            _ => panic!("only concrete work was published"),
        })
        .collect::<Vec<_>>();
    assert_eq!(sequences, [1, 2]);
    assert_eq!(context.state(), CommandResponseFlushState::Pending);
    assert_eq!(
        scheduler.next_protocol_scheduler_step(),
        ProtocolSchedulerStep::CompleteReadyResidence,
        "ordinary work has no blanket command-output dependency"
    );

    for _ in 0..2 {
        scheduler.close_protocol_residence_load_binding_window();
        assert_eq!(
            scheduler.next_protocol_scheduler_step(),
            ProtocolSchedulerStep::CompleteReadyResidence
        );
        let _ = scheduler.complete_next_protocol_residence();
    }
    assert_eq!(scheduler.queues.protocol_residence_len(), 0);
    let super::CommandTaskStep::Complete(output) =
        scheduler.materialize_command_dispatch_step(held_command)
    else {
        panic!("the held command has a completed output plan");
    };
    assert_eq!(
        scheduler.queues.protocol_residence_len(),
        0,
        "a stale wake must not duplicate already consumed publications"
    );
    let (_, _, _, _, _, release) = output
        .with_output_release_permit(super::CommandOutputReleasePermit::new(permit, None))
        .into_parts();
    assert_eq!(context.state(), CommandResponseFlushState::Pending);
    assert!(
        scheduler
            .finish_command_dispatch_output_flush(release)
            .is_empty()
    );
    assert_eq!(context.state(), CommandResponseFlushState::Flushed);
}

#[test]
fn lifecycle_work_observes_only_its_existing_command_output_permit() {
    use moli_protocol::CommandResponseFlushState;
    use moli_protocol::test_support::publish_retired_target_termination_after_command_output;

    let mut conn = CdpConnection::new();
    let (first_permit, first) = conn.begin_command_response_flush_permit();
    let (second_permit, second) = conn.begin_command_response_flush_permit();
    publish_retired_target_termination_after_command_output(&mut conn, "TID-first", first.clone());
    publish_retired_target_termination_after_command_output(
        &mut conn,
        "TID-second",
        second.clone(),
    );
    let wake = conn.take_scheduler_events();
    let (mut scheduler, _receivers) = CdpScheduler::new(conn);
    scheduler.apply_scheduler_events(wake);
    assert_eq!(
        scheduler.next_protocol_scheduler_step(),
        ProtocolSchedulerStep::Wait
    );
    scheduler.close_protocol_residence_load_binding_window();
    assert_eq!(
        scheduler.next_protocol_scheduler_step(),
        ProtocolSchedulerStep::Wait
    );

    let _ = scheduler.finish_command_dispatch_output_flush(Some(
        super::CommandOutputReleasePermit::new(second_permit, None),
    ));
    assert_eq!(first.state(), CommandResponseFlushState::Pending);
    assert_eq!(second.state(), CommandResponseFlushState::Flushed);
    assert_eq!(
        scheduler.next_ungated_protocol_residence_index(),
        Some(1),
        "an independent target need not wait for an earlier command"
    );
    scheduler.close_protocol_residence_load_binding_window();
    let _ = scheduler.complete_next_protocol_residence();
    assert_eq!(scheduler.queues.protocol_residence_len(), 1);
    assert_eq!(
        scheduler.next_protocol_scheduler_step(),
        ProtocolSchedulerStep::Wait
    );

    let _ = scheduler.finish_command_dispatch_output_flush(Some(
        super::CommandOutputReleasePermit::new(first_permit, None),
    ));
    assert_eq!(first.state(), CommandResponseFlushState::Flushed);
    assert_eq!(
        scheduler.next_protocol_scheduler_step(),
        ProtocolSchedulerStep::CompleteReadyResidence
    );
    scheduler.close_protocol_residence_load_binding_window();
    let _ = scheduler.complete_next_protocol_residence();
    assert_eq!(scheduler.queues.protocol_residence_len(), 0);
}

#[tokio::test]
async fn abandoned_command_output_wakes_lifecycle_work_without_reporting_a_flush() {
    use futures_util::FutureExt as _;
    use moli_protocol::CommandResponseFlushState;
    use moli_protocol::test_support::publish_retired_target_termination_after_command_output;

    let mut conn = CdpConnection::new();
    let (permit, context) = conn.begin_command_response_flush_permit();
    publish_retired_target_termination_after_command_output(
        &mut conn,
        "TID-abandoned",
        context.clone(),
    );
    let wake = conn.take_scheduler_events();
    let (mut scheduler, _receivers) = CdpScheduler::new(conn);
    scheduler.apply_scheduler_events(wake);
    assert_eq!(
        scheduler.next_protocol_scheduler_step(),
        ProtocolSchedulerStep::Wait
    );
    let ready = scheduler.state.scheduler_events_ready();
    while ready.notified().now_or_never().is_some() {}

    std::thread::spawn(move || drop(permit)).join().unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), ready.notified())
        .await
        .expect("abandoning the original permit wakes its scheduler dependency");
    assert_eq!(context.state(), CommandResponseFlushState::Cancelled);
    assert_eq!(
        scheduler.next_protocol_scheduler_step(),
        ProtocolSchedulerStep::CompleteReadyResidence
    );
    scheduler.close_protocol_residence_load_binding_window();
    let _ = scheduler.complete_next_protocol_residence();
    assert_eq!(
        scheduler.queues.protocol_residence_len(),
        0,
        "abandonment must not leave cleanup parked forever"
    );
}

#[test]
fn command_dependent_retirement_does_not_block_its_targets_output() {
    use moli_protocol::test_support::{
        publish_protocol_work, publish_retired_target_termination_after_command_output,
    };
    for snapshot in [false, true] {
        let mut conn = CdpConnection::new();
        let (permit, context) = conn.begin_command_response_flush_permit();
        publish_retired_target_termination_after_command_output(
            &mut conn,
            "TID-held",
            context.clone(),
        );
        publish_protocol_work(&mut conn, |sequence| {
            root_frame_stopped_loading_work_for_target(
                sequence,
                vec![None],
                "BID-held".into(),
                "TID-held".into(),
                "FRAME-held".into(),
                "LOADER-held".into(),
            )
        });
        let wake = conn.take_scheduler_events();
        let (mut scheduler, _receivers) = CdpScheduler::new(conn);
        scheduler.apply_scheduler_events(wake);
        assert_eq!(
            scheduler.next_ungated_protocol_residence_index(),
            Some(1),
            "a pending owner action cannot turn its command dependency into an output barrier"
        );
        if snapshot {
            let pending = std::mem::take(&mut scheduler.queues.protocol_residences);
            let _ = scheduler.complete_protocol_residence_snapshot(pending);
        } else {
            scheduler.close_protocol_residence_load_binding_window();
            let _ = scheduler.complete_next_protocol_residence();
        }
        assert_eq!(
            context.state(),
            moli_protocol::CommandResponseFlushState::Pending
        );
        assert_eq!(scheduler.queues.protocol_residence_len(), 1);
        assert!(
            matches!(scheduler.queues.protocol_residences.front(), Some(ProtocolSchedulerResidence::ProtocolWork { work, .. }) if work.publish_sequence().get() == 1)
        );
        let _ = scheduler.finish_command_dispatch_output_flush(Some(
            super::CommandOutputReleasePermit::new(permit, None),
        ));
        scheduler.close_protocol_residence_load_binding_window();
        let _ = scheduler.complete_next_protocol_residence();
        assert_eq!(scheduler.queues.protocol_residence_len(), 0);
    }
}

fn future_load_candidate(
    publication: &moli_core::RendererOutputTransportMessage,
) -> DeferredMainDocumentLoadPredecessorCandidate {
    DeferredMainDocumentLoadPredecessorCandidate::from_renderer_publication(publication)
        .expect("timer publication should be eligible for an exact future load predecessor")
}

fn load_interest_for_publication(
    publication: &moli_core::RendererOutputTransportMessage,
) -> DeferredMainDocumentLoadCompletionOutputInterest {
    let RendererOutputTransportMessage::Publication(publication) = publication else {
        panic!("test fixture must contain one concrete publication");
    };
    let RendererOutputPublicationOrdering::AfterPendingPageLoad { source_document } =
        publication.ordering()
    else {
        panic!("test fixture must carry exact post-load ordering");
    };
    deferred_main_document_load_output_interest(
        publication.cursor().stream().residence(),
        Some(source_document),
    )
}

#[test]
fn only_post_load_renderer_output_can_become_a_future_load_predecessor_candidate() {
    let page_id = PageId::new_for_testing(7);
    let post_load = post_load_renderer_publication(page_id, 1);
    assert!(
        DeferredMainDocumentLoadPredecessorCandidate::from_renderer_publication(&post_load)
            .is_some(),
        "only a publication carrying typed post-load ordering may become a candidate"
    );

    let unconstrained = unconstrained_renderer_publication(page_id);
    assert!(
        DeferredMainDocumentLoadPredecessorCandidate::from_renderer_publication(&unconstrained)
            .is_none(),
        "ordinary concrete output is not inferred to be post-load from its payload"
    );

    let control: RendererOutputTransportMessage = RendererOutputStreamControl::Opened {
        stream: RendererOutputStreamIdentity::new_page_for_protocol_test(page_id),
    }
    .into();
    assert!(
        DeferredMainDocumentLoadPredecessorCandidate::from_renderer_publication(&control).is_none(),
        "stream lifetime control is never Page output"
    );
}

#[test]
fn renderer_output_publication_waits_for_its_exact_load_predecessors() {
    let publication = post_load_renderer_publication(PageId::new_for_testing(7), 1);
    let cursor = publication_cursor(&publication);
    let first = deferred_main_document_load_observation_id(1);
    let second = deferred_main_document_load_observation_id(2);
    let mut queues = SchedulerQueues::default();
    queues.enqueue_renderer_output_publication(
        cursor,
        ProtocolOutputSequence::from_messages(vec![json!({"method": "Runtime.consoleAPICalled"})]),
        vec![first, second],
        None,
    );

    queues.close_front_load_binding_window();
    let owner = moli_protocol::ProtocolStateOwner::new(CdpConnection::new());
    assert!(
        !owner
            .with_state(|state| queues.should_complete_next_residence(&state))
            .unwrap()
    );
    queues.satisfy_load_predecessor(second);
    assert!(
        !owner
            .with_state(|state| queues.should_complete_next_residence(&state))
            .unwrap(),
        "an unrelated completed load must not release the frozen batch early"
    );
    queues.satisfy_load_predecessor(first);
    assert!(
        owner
            .with_state(|state| queues.should_complete_next_residence(&state))
            .unwrap()
    );
}

#[test]
fn pending_renderer_projection_keeps_its_load_window_while_peer_work_advances() {
    let publication = post_load_renderer_publication(PageId::new_for_testing(7), 1);
    let cursor = publication_cursor(&publication);
    let candidate = future_load_candidate(&publication);
    let (mut scheduler, _protocol_completions) = CdpScheduler::new(CdpConnection::new());
    scheduler.queues.enqueue_pending_renderer_output_projection(
        cursor,
        Vec::new(),
        Some(candidate),
    );
    scheduler
        .apply_protocol_work_publications(vec![root_frame_stopped_loading_work(1, "FRAME-peer")]);
    assert_eq!(scheduler.next_ungated_protocol_residence_index(), Some(1));
    scheduler.close_protocol_residence_load_binding_window();
    let snapshot = scheduler.queues.take_external_load_wait_snapshot();
    assert_eq!(snapshot.len(), 1, "peer work may leave the queue");
    assert_eq!(
        scheduler.queues.renderer_output_order(cursor),
        Some((Vec::new(), Some(candidate)))
    );
    assert!(scheduler.queues.complete_renderer_output_projection(
        cursor,
        ProtocolOutputSequence::from_messages(vec![json!({"method": "Page.loadEventFired"})]),
    ));
    assert!(
        scheduler
            .queues
            .take_unconstrained_renderer_output(cursor)
            .is_none()
    );
    assert_eq!(
        scheduler.next_protocol_scheduler_step(),
        ProtocolSchedulerStep::CloseLoadBindingWindow
    );
    scheduler.close_protocol_residence_load_binding_window();
    assert_eq!(
        scheduler.next_protocol_scheduler_step(),
        ProtocolSchedulerStep::CompleteReadyResidence
    );
}

#[test]
fn renderer_owner_work_waits_for_its_original_projection_and_exact_load() {
    let publication = post_load_renderer_publication(PageId::new_for_testing(7), 1);
    let cursor = publication_cursor(&publication);
    let load = deferred_main_document_load_observation_id(1);
    let (mut scheduler, _protocol_completions) = CdpScheduler::new(CdpConnection::new());
    scheduler
        .queues
        .enqueue_pending_renderer_output_projection(cursor, vec![load], None);
    scheduler.apply_protocol_work_publications(vec![
        moli_protocol::test_support::protocol_work_from_renderer_output(
            root_frame_stopped_loading_work(1, "FRAME-source"),
            cursor,
        ),
    ]);
    let Some(ProtocolSchedulerResidence::ProtocolWork {
        load_predecessors, ..
    }) = scheduler.queues.protocol_residences.get(1)
    else {
        panic!("the original owner work stays queued");
    };
    assert_eq!(load_predecessors, &[load]);
    scheduler.queues.close_future_load_predecessor_window();
    assert!(scheduler.queues.take_command_followup_snapshot().is_empty());
    assert_eq!(scheduler.next_ungated_protocol_residence_index(), None);
    scheduler.queues.satisfy_load_predecessor(load);
    assert_eq!(
        scheduler.next_ungated_protocol_residence_index(),
        None,
        "load completion alone cannot release unfinished source work"
    );
    assert!(
        scheduler
            .queues
            .complete_renderer_output_projection(cursor, ProtocolOutputSequence::empty())
    );
    assert!(
        scheduler
            .queues
            .take_unconstrained_renderer_output(cursor)
            .is_some()
    );
    assert_eq!(
        scheduler.next_protocol_scheduler_step(),
        ProtocolSchedulerStep::CompleteReadyResidence
    );
}

#[test]
fn complete_creation_delivery_blocks_child_across_specialized_drains() {
    let parent = publication_cursor(&post_load_renderer_publication(
        PageId::new_for_testing(7),
        1,
    ));
    let child = publication_cursor(&post_load_renderer_publication(
        PageId::new_for_testing(8),
        1,
    ));
    let load = deferred_main_document_load_observation_id(1);
    let (mut scheduler, _tasks) = CdpScheduler::new(CdpConnection::new());
    scheduler
        .queues
        .enqueue_pending_renderer_output_projection(parent, vec![load], None);
    scheduler
        .queues
        .enqueue_pending_renderer_output_projection(child, Vec::new(), None);
    scheduler
        .queues
        .bind_renderer_delivery_predecessor(child, Some(parent));
    assert!(scheduler.queues.complete_renderer_output_projection(
        parent,
        ProtocolOutputSequence::from_messages(vec![json!({"method":"Target.targetCreated"})])
    ));
    assert!(scheduler.queues.complete_renderer_output_projection(
        child,
        ProtocolOutputSequence::from_messages(vec![
            json!({"method":"Page.javascriptDialogOpening"})
        ])
    ));
    assert!(
        scheduler
            .queues
            .take_unconstrained_renderer_output(child)
            .is_none(),
        "projection is not delivery"
    );
    let snapshot = std::mem::take(&mut scheduler.queues.protocol_residences);
    let child_work = snapshot.iter().find(|item| matches!(item,
        ProtocolSchedulerResidence::RendererOutputPublication(work) if work.renderer_output_cursor == child)).unwrap();
    assert!(
        scheduler
            .queues
            .renderer_output_source_is_pending(child_work),
        "taking the parent out of residence does not deliver it"
    );
    scheduler.queues.restore_snapshot_to_front(snapshot);
    scheduler.queues.satisfy_load_predecessor(load);
    let parent_work = scheduler.queues.protocol_residences.pop_front().unwrap();
    let mut output = scheduler.complete_protocol_residence(parent_work);
    output.append(
        scheduler
            .queues
            .take_unconstrained_renderer_output(child)
            .unwrap(),
    );
    assert_eq!(
        output.into_messages(),
        [
            json!({"method":"Target.targetCreated"}),
            json!({"method":"Page.javascriptDialogOpening"})
        ]
    );
    assert_eq!(scheduler.queues.protocol_residence_len(), 0);
}

#[test]
fn fenced_creation_stays_in_the_complete_source_publication() {
    use moli_core::{RendererOutputFence, RendererOutputStreamIdentity};
    use moli_protocol::test_support::{
        protocol_work_from_renderer_output, renderer_fenced_output_work,
    };
    let creation = moli_core::RendererOutputCursor::new_for_test(
        RendererOutputStreamIdentity::new_shared_worker_for_protocol_test(81),
        1,
    );
    let native = moli_core::RendererOutputCursor::new_for_test(
        RendererOutputStreamIdentity::new_shared_worker_for_protocol_test(82),
        1,
    );
    let child = moli_core::RendererOutputCursor::new_for_test(
        RendererOutputStreamIdentity::new_shared_worker_for_protocol_test(83),
        1,
    );
    let mut queues = SchedulerQueues::default();
    queues.enqueue_pending_renderer_output_projection(creation, Vec::new(), None);
    queues.complete_renderer_output_projection(creation, ProtocolOutputSequence::empty());
    queues.enqueue_protocol_work(protocol_work_from_renderer_output(
        renderer_fenced_output_work(
            1,
            vec![BackgroundProtocolEvent::immediate(
                json!({"method":"Target.targetCreated"}),
            )],
            vec![RendererOutputFence::new_for_test(native)],
        ),
        creation,
    ));
    assert_eq!(
        queues.protocol_residence_len(),
        1,
        "the original cursor owns all creation output"
    );
    assert!(
        queues
            .take_unconstrained_renderer_output(creation)
            .is_none(),
        "an empty original cannot escape its fenced output"
    );
    queues.enqueue_pending_renderer_output_projection(child, Vec::new(), None);
    queues.bind_renderer_delivery_predecessor(child, Some(creation));
    queues.complete_renderer_output_projection(
        child,
        ProtocolOutputSequence::from_messages(vec![
            json!({"method":"Page.javascriptDialogOpening"}),
        ]),
    );
    assert!(queues.take_unconstrained_renderer_output(child).is_none());
    let state = moli_protocol::ProtocolStateOwner::new(CdpConnection::new());
    assert!(
        !state
            .with_state(|state| queues.residence_is_ready_to_complete(
                queues.protocol_residences.front().unwrap(),
                &state
            ))
            .unwrap(),
        "a real independent Native fence still holds creation"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn navigation_tail_waits_for_native_delivery_independently_of_source_facts() {
    use moli_protocol::test_support::{
        bind_renderer_page_output_stream_to_retired_owner, protocol_work_after_renderer_delivery,
        protocol_work_from_renderer_output, renderer_fenced_output_work,
    };
    tokio::task::LocalSet::new()
        .run_until(async {
            for source_attribution in [false, true] {
                for snapshot in [false, true] {
                    let source = publication_cursor(&unconstrained_renderer_publication(
                        PageId::new_for_testing(91),
                    ));
                    let native_publication =
                        unconstrained_renderer_publication(PageId::new_for_testing(92));
                    let native = publication_cursor(&native_publication);
                    let load = deferred_main_document_load_observation_id(1);
                    let mut conn = CdpConnection::new();
                    bind_renderer_page_output_stream_to_retired_owner(&mut conn, native.stream());
                    conn.admit_renderer_output(
                        RendererOutputStreamControl::Opened {
                            stream: native.stream(),
                        }
                        .into(),
                    );
                    let (mut scheduler, mut completions) = CdpScheduler::new(conn);
                    scheduler.queues.enqueue_renderer_output_publication(
                        source,
                        ProtocolOutputSequence::from_messages(vec![
                            json!({"method":"Moli.sourceBefore"}),
                            json!({"method":"Moli.sourceAfter"}),
                        ]),
                        Vec::new(),
                        None,
                    );
                    scheduler.queues.enqueue_pending_renderer_output_projection(
                        native,
                        vec![load],
                        None,
                    );
                    scheduler
                        .queues
                        .append_renderer_output_projection(
                            native,
                            ProtocolOutputSequence::from_messages(vec![
                                json!({"method":"Moli.nativeCommit"}),
                            ]),
                        )
                        .unwrap();
                    let mut tail = protocol_work_after_renderer_delivery(
                        renderer_fenced_output_work(
                            1,
                            vec![BackgroundProtocolEvent::immediate(
                                json!({"method":"Moli.navigationTail"}),
                            )],
                            vec![RendererOutputFence::new_for_test(native)],
                        ),
                        native,
                    );
                    assert!(
                        tail.renderer_output_source().is_none(),
                        "navigation completion starts independently"
                    );
                    if source_attribution {
                        tail = protocol_work_from_renderer_output(tail, source);
                    }
                    scheduler.apply_protocol_work_publications(vec![tail]);
                    assert_eq!(
                        scheduler.queues.protocol_residence_len(),
                        3,
                        "a delivery-dependent tail is not merged into source facts"
                    );
                    assert_eq!(
                        scheduler.complete_next_protocol_residence().into_messages(),
                        [
                            json!({"method":"Moli.sourceBefore"}),
                            json!({"method":"Moli.sourceAfter"}),
                        ]
                    );
                    assert!(
                        scheduler
                            .ingest_renderer_publication_now(native_publication)
                            .is_empty()
                    );
                    assert!(
                        scheduler
                            .complete_protocol_work(completions.recv().await.unwrap())
                            .unwrap()
                            .is_empty()
                    );
                    assert!(
                        scheduler
                            .state
                            .with_state(|state| state.renderer_output_cursor_is_projected(native))
                            .unwrap()
                    );
                    assert_eq!(
                        scheduler.next_protocol_scheduler_step(),
                        ProtocolSchedulerStep::Wait
                    );
                    assert!(
                        scheduler.complete_next_protocol_residence().is_empty(),
                        "projection alone cannot release the navigation tail"
                    );
                    scheduler.queues.satisfy_load_predecessor(load);
                    let commit = if snapshot {
                        let native_output = scheduler
                            .queues
                            .take_renderer_output_predecessor_snapshot(native);
                        assert_eq!(native_output.len(), 1);
                        assert_eq!(
                            scheduler.next_protocol_scheduler_step(),
                            ProtocolSchedulerStep::Wait
                        );
                        assert!(
                            scheduler.complete_next_protocol_residence().is_empty(),
                            "snapshot checkout is not Native delivery"
                        );
                        scheduler.complete_protocol_residence_snapshot(native_output)
                    } else {
                        scheduler.complete_next_protocol_residence()
                    };
                    assert_eq!(
                        commit.into_messages(),
                        [json!({"method":"Moli.nativeCommit"})]
                    );
                    assert_eq!(
                        scheduler.complete_next_protocol_residence().into_messages(),
                        [json!({"method":"Moli.navigationTail"})]
                    );
                    assert_eq!(scheduler.queues.protocol_residence_len(), 0);
                    assert!(scheduler.complete_next_protocol_residence().is_empty());
                }
            }
        })
        .await;
}

fn observable_protocol_work(sequence: u64) -> ProtocolSchedulerWork {
    moli_protocol::test_support::renderer_fenced_output_work(
        sequence,
        vec![BackgroundProtocolEvent::immediate(
            json!({"method":"Moli.workCompleted", "params":{"sequence":sequence}}),
        )],
        Vec::new(),
    )
}

#[test]
fn command_output_keeps_its_response_while_admitting_shared_publications() {
    use moli_protocol::test_support::publish_protocol_work;
    let mut conn = CdpConnection::new();
    publish_protocol_work(&mut conn, observable_protocol_work);
    let response = json!({"id":41,"result":{}});
    let complete = moli_protocol::test_support::command_completion_with_scheduler_events(
        vec![BackgroundProtocolEvent::immediate(response.clone())],
        conn.take_scheduler_events(),
    );
    publish_protocol_work(&mut conn, observable_protocol_work);
    let later_wake = conn.take_scheduler_events();
    let (mut scheduler, _tasks) = CdpScheduler::new(conn);
    let super::CommandTaskStep::Complete(output) =
        scheduler.materialize_command_dispatch_step(complete)
    else {
        panic!("finite completion");
    };
    let (output, _, _, _, _, release) = output.into_parts();
    assert_eq!(output.into_messages(), [response]);
    assert_eq!(scheduler.queues.protocol_residence_len(), 2);
    scheduler.apply_scheduler_events(later_wake);
    assert!(
        scheduler
            .finish_command_dispatch_output_flush(release)
            .is_empty()
    );
    assert_eq!(
        scheduler.queues.protocol_residence_len(),
        2,
        "the command's later wake must not duplicate work"
    );
    for sequence in [1, 2] {
        let messages = scheduler.complete_next_protocol_residence().into_messages();
        assert_eq!(
            messages,
            [json!({"method":"Moli.workCompleted", "params":{"sequence":sequence}})],
            "coalesced wakes must retain publication order and execute each payload once"
        );
    }
    assert_eq!(scheduler.queues.protocol_residence_len(), 0);
}

#[test]
fn renderer_output_handoff_keeps_before_and_after_response_order() {
    let publication = post_load_renderer_publication(PageId::new_for_testing(7), 1);
    let cursor = publication_cursor(&publication);
    let mut queues = SchedulerQueues::default();
    queues.enqueue_pending_renderer_output_projection(cursor, Vec::new(), None);
    queues
        .append_renderer_output_projection(
            cursor,
            ProtocolOutputSequence::from_messages(vec![json!({"method": "Moli.afterResponse"})]),
        )
        .unwrap();
    assert!(queues.take_unconstrained_renderer_output(cursor).is_none());
    assert!(queues.complete_renderer_output_projection(
        cursor,
        ProtocolOutputSequence::from_messages(vec![json!({"method": "Debugger.paused"})])
    ));
    assert_eq!(
        queues
            .take_unconstrained_renderer_output(cursor)
            .unwrap()
            .into_messages(),
        vec![
            json!({"method": "Debugger.paused"}),
            json!({"method": "Moli.afterResponse"})
        ]
    );
    assert_eq!(queues.protocol_residence_len(), 0);
}

#[test]
fn concrete_renderer_output_precedes_work_published_by_the_same_ingress() {
    let publication = post_load_renderer_publication(PageId::new_for_testing(7), 1);
    let cursor = publication_cursor(&publication);
    let load = deferred_main_document_load_observation_id(1);
    let mut queues = SchedulerQueues::default();
    queues.enqueue_renderer_output_publication(
        cursor,
        ProtocolOutputSequence::from_messages(vec![json!({"method": "Page.loadEventFired"})]),
        vec![load],
        None,
    );
    queues.enqueue_protocol_work(root_frame_stopped_loading_work(1, "FRAME-1"));

    assert!(matches!(
        queues.protocol_residences.front(),
        Some(ProtocolSchedulerResidence::RendererOutputPublication(work))
            if work.renderer_output_cursor == cursor
    ));
    assert!(matches!(
        queues.protocol_residences.get(1),
        Some(ProtocolSchedulerResidence::ProtocolWork { work, .. })
            if work.publish_sequence().get() == 1
    ));
}

#[test]
fn protocol_work_published_by_held_ingress_inherits_exact_load_predecessor() {
    let load = deferred_main_document_load_observation_id(1);
    let publication = post_load_renderer_publication(PageId::new_for_testing(7), 1);
    let cursor = publication_cursor(&publication);
    let (mut scheduler, _protocol_completions) = CdpScheduler::new(CdpConnection::new());
    scheduler.queues.enqueue_renderer_output_publication(
        cursor,
        ProtocolOutputSequence::from_messages(vec![json!({"method": "Page.loadEventFired"})]),
        vec![load],
        None,
    );
    scheduler.apply_protocol_work_publications(vec![
        moli_protocol::test_support::protocol_work_from_renderer_output(
            root_frame_stopped_loading_work(1, "FRAME-1"),
            cursor,
        ),
    ]);
    assert!(
        matches!(scheduler.queues.protocol_residences.back(), Some(ProtocolSchedulerResidence::ProtocolWork { load_predecessors, .. }) if load_predecessors == &[load])
    );

    // Even an explicit client-turn yield cannot release work inherited from
    // this still-held source; the exact load observation remains necessary.
    scheduler.queues.close_load_binding_window_at(1);
    assert_eq!(
        scheduler.next_protocol_scheduler_step(),
        ProtocolSchedulerStep::Wait,
        "a yielded client turn cannot satisfy the exact load predecessor"
    );
    scheduler.queues.satisfy_load_predecessor(load);
    assert_eq!(
        scheduler.next_protocol_scheduler_step(),
        ProtocolSchedulerStep::CompleteReadyResidence
    );
}

#[test]
fn newly_published_load_binds_every_residence_from_deferred_renderer_ingress() {
    let publication = post_load_renderer_publication(PageId::new_for_testing(7), 1);
    let cursor = publication_cursor(&publication);
    let candidate = future_load_candidate(&publication);
    let interest = load_interest_for_publication(&publication);
    let load = deferred_main_document_load_observation_id(1);
    let mut queues = SchedulerQueues::default();
    queues.enqueue_renderer_output_publication(
        cursor,
        ProtocolOutputSequence::from_messages(vec![json!({"method": "Page.loadEventFired"})]),
        Vec::new(),
        Some(candidate),
    );
    queues.enqueue_protocol_work(
        moli_protocol::test_support::protocol_work_from_renderer_output(
            root_frame_stopped_loading_work(1, "FRAME-1"),
            cursor,
        ),
    );

    assert_eq!(
        queues.bind_main_document_load_predecessor(load, &interest),
        0,
        "the load owner action must be inserted before every residence produced by the ingress"
    );
    assert!(matches!(
        queues.protocol_residences.front(),
        Some(ProtocolSchedulerResidence::RendererOutputPublication(work))
            if work.load_predecessors == [load]
    ));
    assert!(matches!(
        queues.protocol_residences.get(1),
        Some(ProtocolSchedulerResidence::ProtocolWork {
            load_predecessors,
            ..
        }) if load_predecessors == &[load]
    ));
}

#[test]
fn future_load_predecessor_candidate_cannot_bind_another_page_load() {
    let publication = post_load_renderer_publication(PageId::new_for_testing(7), 1);
    let cursor = publication_cursor(&publication);
    let candidate = future_load_candidate(&publication);
    let unrelated_page_publication = post_load_renderer_publication(PageId::new_for_testing(8), 1);
    let unrelated_interest = load_interest_for_publication(&unrelated_page_publication);
    let load = deferred_main_document_load_observation_id(1);
    let mut queues = SchedulerQueues::default();
    queues.enqueue_renderer_output_publication(
        cursor,
        ProtocolOutputSequence::from_messages(vec![json!({"method": "Runtime.consoleAPICalled"})]),
        Vec::new(),
        Some(candidate),
    );

    assert_eq!(
        queues.bind_main_document_load_predecessor(load, &unrelated_interest),
        queues.protocol_residence_len(),
        "a load observation may bind only output from its exact Page"
    );
    assert!(matches!(
        queues.protocol_residences.front(),
        Some(ProtocolSchedulerResidence::RendererOutputPublication(work))
            if work.load_predecessors.is_empty()
    ));
}

#[test]
fn after_load_candidate_cannot_bind_another_document_on_the_same_page() {
    let page_id = PageId::new_for_testing(7);
    let replacement_document = root_document_lifecycle_identity(page_id, 2);
    let publication = post_load_renderer_publication(page_id, 1);
    let cursor = publication_cursor(&publication);
    let candidate =
        DeferredMainDocumentLoadPredecessorCandidate::from_renderer_publication(&publication)
            .expect("after-load lifecycle output should expose an exact Document candidate");
    let replacement_interest = deferred_main_document_load_output_interest(
        publication.residence(),
        Some(replacement_document),
    );
    let load = deferred_main_document_load_observation_id(1);
    let mut queues = SchedulerQueues::default();
    queues.enqueue_renderer_output_publication(
        cursor,
        ProtocolOutputSequence::from_messages(vec![json!({
            "method": "Page.frameStartedNavigating"
        })]),
        Vec::new(),
        Some(candidate),
    );

    assert_eq!(
        queues.bind_main_document_load_predecessor(load, &replacement_interest),
        queues.protocol_residence_len(),
        "an after-load action may bind only the exact source Document's load observation"
    );
    assert!(matches!(
        queues.protocol_residences.front(),
        Some(ProtocolSchedulerResidence::RendererOutputPublication(work))
            if work.load_predecessors.is_empty()
    ));
}

#[test]
fn old_document_timer_output_does_not_wait_behind_replacement_document_load() {
    let page_id = PageId::new_for_testing(7);
    let publication = post_load_renderer_publication(page_id, 1);
    let replacement_interest = deferred_main_document_load_output_interest(
        publication.residence(),
        Some(root_document_lifecycle_identity(page_id, 2)),
    );

    assert_eq!(
        replacement_interest.route_output_while_waiting(&publication),
        DeferredMainDocumentLoadCompletionOutputAction::ProcessNow,
        "a timer turn must retain the Document that authorized it across replacement"
    );
    assert!(
        !replacement_interest.observes_predecessor_candidate(future_load_candidate(&publication)),
        "a replacement Document must not adopt the old timer publication as its load successor"
    );
}

#[test]
fn scheduler_turn_closes_the_provisional_load_binding_window() {
    let publication = post_load_renderer_publication(PageId::new_for_testing(7), 1);
    let cursor = publication_cursor(&publication);
    let candidate = future_load_candidate(&publication);
    let interest = load_interest_for_publication(&publication);
    let unrelated_later_load = deferred_main_document_load_observation_id(1);
    let mut queues = SchedulerQueues::default();
    queues.enqueue_renderer_output_publication(
        cursor,
        ProtocolOutputSequence::from_messages(vec![json!({"method": "Runtime.consoleAPICalled"})]),
        Vec::new(),
        Some(candidate),
    );
    queues.enqueue_protocol_work(
        moli_protocol::test_support::protocol_work_from_renderer_output(
            root_frame_stopped_loading_work(1, "FRAME-1"),
            cursor,
        ),
    );

    queues.close_front_load_binding_window();

    assert_eq!(
        queues.bind_main_document_load_predecessor(unrelated_later_load, &interest),
        queues.protocol_residence_len(),
        "a later command must not retroactively bind work after its client-turn boundary"
    );
    assert!(matches!(
        queues.protocol_residences.front(),
        Some(ProtocolSchedulerResidence::RendererOutputPublication(work))
            if work.load_predecessors.is_empty()
    ));
    assert!(matches!(
        queues.protocol_residences.get(1),
        Some(ProtocolSchedulerResidence::ProtocolWork {
            load_predecessors,
            ..
        }) if load_predecessors.is_empty()
    ));
}

#[test]
fn executing_ordinary_work_closes_a_later_provisional_load_binding_window() {
    let publication = post_load_renderer_publication(PageId::new_for_testing(7), 1);
    let cursor = publication_cursor(&publication);
    let candidate = future_load_candidate(&publication);
    let interest = load_interest_for_publication(&publication);
    let unrelated_later_load = deferred_main_document_load_observation_id(1);
    let mut queues = SchedulerQueues::default();
    queues.enqueue_protocol_work(observable_protocol_work(1));
    queues.enqueue_renderer_output_publication(
        cursor,
        ProtocolOutputSequence::from_messages(vec![json!({"method": "Runtime.consoleAPICalled"})]),
        Vec::new(),
        Some(candidate),
    );

    let (mut scheduler, _completions) = CdpScheduler::new(CdpConnection::new());
    scheduler.queues = queues;
    let messages = scheduler.complete_next_protocol_residence().into_messages();
    assert_eq!(
        messages,
        [json!({"method":"Moli.workCompleted", "params":{"sequence":1}})]
    );
    let queues = &mut scheduler.queues;

    assert_eq!(
        queues.bind_main_document_load_predecessor(unrelated_later_load, &interest),
        queues.protocol_residence_len(),
        "executing independent queued work closes earlier candidates without a permission-only turn"
    );
    assert!(matches!(
        queues.protocol_residences.front(),
        Some(ProtocolSchedulerResidence::RendererOutputPublication(work))
            if work.load_predecessors.is_empty()
    ));
}

#[tokio::test]
async fn runtime_response_releases_its_frozen_renderer_publication_predecessor() {
    let publication = post_load_renderer_publication(PageId::new_for_testing(1), 1);
    let candidate = future_load_candidate(&publication);
    let cursor = publication_cursor(&publication);
    let (mut scheduler, _protocol_completions) = CdpScheduler::new(CdpConnection::new());
    scheduler.queues.enqueue_renderer_output_publication(
        cursor,
        ProtocolOutputSequence::from_messages(vec![json!({
            "method": "Page.navigatedWithinDocument"
        })]),
        Vec::new(),
        Some(candidate),
    );

    let output = scheduler.complete_renderer_output_predecessor_before_runtime_response(
        &RendererOutputFence::new_for_test(cursor),
    );

    assert_eq!(
        output.into_messages(),
        vec![json!({"method": "Page.navigatedWithinDocument"})],
        "a provisional load-binding window must close before its later Runtime response"
    );
    assert_eq!(scheduler.queues.protocol_residence_len(), 0);
}

#[tokio::test]
async fn runtime_response_cannot_release_renderer_output_with_an_exact_load_predecessor() {
    let publication = post_load_renderer_publication(PageId::new_for_testing(1), 1);
    let load = deferred_main_document_load_observation_id(1);
    let cursor = publication_cursor(&publication);
    let (mut scheduler, _protocol_completions) = CdpScheduler::new(CdpConnection::new());
    scheduler.queues.enqueue_renderer_output_publication(
        cursor,
        ProtocolOutputSequence::from_messages(vec![json!({
            "method": "Runtime.consoleAPICalled"
        })]),
        vec![load],
        None,
    );

    let output = scheduler.complete_renderer_output_predecessor_before_runtime_response(
        &RendererOutputFence::new_for_test(cursor),
    );

    assert!(output.is_empty());
    assert_eq!(
        scheduler.queues.protocol_residence_len(),
        1,
        "the exact load observation remains the only release authority"
    );
}

#[test]
fn load_publication_cannot_become_a_successor_of_its_own_post_load_action() {
    let page = PageId::new_for_testing(7);
    let document = root_document_lifecycle_identity(page, 1);
    let parent = renderer_publication(
        page,
        1,
        RendererOutputPublicationOrdering::AfterPendingPageLoad {
            source_document: document,
        },
    );
    let timer = renderer_publication(
        page,
        2,
        RendererOutputPublicationOrdering::AfterPendingPageLoad {
            source_document: document,
        },
    );
    let mut queues = SchedulerQueues::default();
    for publication in [&parent, &timer] {
        queues.enqueue_pending_renderer_output_projection(
            publication_cursor(publication),
            Vec::new(),
            Some(future_load_candidate(publication)),
        );
    }
    queues.close_load_publication_window(publication_cursor(&parent));
    let observation = deferred_main_document_load_observation_id(1);
    assert_eq!(
        queues.bind_main_document_load_predecessor(
            observation,
            &load_interest_for_publication(&parent)
        ),
        1
    );
    let (parent_predecessors, parent_candidate) = queues
        .renderer_output_order(publication_cursor(&parent))
        .unwrap();
    assert!(
        parent_predecessors.is_empty() && parent_candidate.is_none(),
        "the publication carrying Load cannot depend on its post-load successor"
    );
    assert_eq!(
        queues
            .renderer_output_order(publication_cursor(&timer))
            .unwrap()
            .0,
        [observation],
        "a later timer must keep its exact load predecessor"
    );
}

#[test]
fn provisional_load_binding_prevents_specialized_wait_drain_from_overtaking_ingress() {
    let publication = post_load_renderer_publication(PageId::new_for_testing(7), 1);
    let cursor = publication_cursor(&publication);
    let candidate = future_load_candidate(&publication);
    let mut queues = SchedulerQueues::default();
    queues.enqueue_renderer_output_publication(
        cursor,
        ProtocolOutputSequence::from_messages(vec![json!({"method": "Page.loadEventFired"})]),
        Vec::new(),
        Some(candidate),
    );
    queues.enqueue_protocol_work(
        moli_protocol::test_support::protocol_work_from_renderer_output(
            root_frame_stopped_loading_work(1, "FRAME-1"),
            cursor,
        ),
    );

    assert!(
        queues.take_external_load_wait_snapshot().is_empty(),
        "a specialized wait drain must not steal same-ingress work before command completion gets its load-binding opportunity"
    );
    assert_eq!(queues.protocol_residence_len(), 2);
}

#[tokio::test]
async fn external_load_wait_cannot_publish_post_load_before_held_load_output() {
    async fn command(
        scheduler: &mut CdpScheduler,
        receivers: &mut super::CdpSchedulerEventReceivers,
        message: serde_json::Value,
    ) -> ProtocolOutputSequence {
        let handle = scheduler.state.handle();
        let raw = message.to_string();
        let pending = handle.process_message_for_test(&raw);
        tokio::pin!(pending);
        let mut output = ProtocolOutputSequence::empty();
        loop {
            tokio::select! {
                biased;
                outcome = &mut pending => {
                    output.append(scheduler.apply_renderer_owner_turn_outcome(receivers, outcome.unwrap()).await
                        .unwrap_or_else(|failure| panic!("fixture command failed: {:?}", failure.into_parts().1)));
                    let sent = messages(&output);
                    assert!(sent.iter().any(|event| event["id"] == message["id"] && event.get("result").is_some()),
                        "fixture command failed: {message}; output={sent:?}");
                    return output;
                }
                input = receivers.recv_interleaved_input() => {
                    output.append(scheduler.complete_interleaved_scheduler_input(receivers, input.unwrap()).await
                        .unwrap_or_else(|failure| panic!("fixture input failed: {:?}", failure.into_parts().1)));
                }
            }
        }
    }

    fn messages(output: &ProtocolOutputSequence) -> Vec<serde_json::Value> {
        output
            .events
            .iter()
            .cloned()
            .filter_map(BackgroundProtocolEvent::into_wire_message)
            .collect()
    }

    async fn external_drain(
        scheduler: &mut CdpScheduler,
        receivers: &mut super::CdpSchedulerEventReceivers,
    ) -> ProtocolOutputSequence {
        let mut output = scheduler.complete_ready_protocol_residences_for_external_load_wait();
        while !scheduler.queues.pending_loads.is_empty() {
            let completed = receivers.protocol_work_completion_rx.recv().await.unwrap();
            output.append(scheduler.complete_protocol_work(completed).unwrap());
        }
        output.append(scheduler.complete_ready_protocol_residences_for_external_load_wait());
        output
    }

    tokio::task::LocalSet::new().run_until(async {
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            let (release, stylesheet) = tokio::sync::watch::channel(false);
            let fixture = axum::Router::new()
                .route("/page", axum::routing::get(|| async {
                    axum::response::Html("<link rel=stylesheet href=/held.css><body>load order</body>")
                }))
                .route("/held.css", axum::routing::get(move || {
                    let mut stylesheet = stylesheet.clone();
                    async move {
                        stylesheet.wait_for(|ready| *ready).await.unwrap();
                        ([("Content-Type", "text/css")], "body { color: black; }")
                    }
                }));
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move { axum::serve(listener, fixture).await.unwrap() });
            let (mut scheduler, mut receivers) = CdpScheduler::new_with_initial_state_runtime_config(
                moli_protocol::CdpInitialStoragePartition::memory(),
                moli_core::runtime::NavigationRuntimeConfig::new(
                    moli_fetch::FetchConfig::default(), moli_core::OptionalResourceFetchMask::NONE,
                    true, moli_core::LayoutPolicy::OnDemand,
                ),
            );
            command(&mut scheduler, &mut receivers, json!({"id": 1, "method": "Page.enable"})).await;
            let owner = AutomationContext {
                protocol: FrontendProtocol::Cdp, session_id: None,
                target_id: Some(DevToolsTargetId::from(moli_protocol::DEFAULT_CDP_PAGE_TARGET_ID)),
                browser_context_id: None,
            };
            command(&mut scheduler, &mut receivers, json!({"id": 2, "method": "Network.enable"})).await;
            command(&mut scheduler, &mut receivers, json!({"id": 3, "method": "Page.setLifecycleEventsEnabled", "params": {"enabled": true}})).await;
            let navigation = scheduler.execute_automation_command_with_protocol_messages(
                &mut receivers, AutomationCommand::Navigate(moli_protocol::automation::DevToolsNavigateCommand {
                    context: owner.clone(), url: format!("http://{address}/page"), referrer: None,
                    wait: DevToolsNavigationWait::None,
                }),
            ).await;
            let AutomationResult::Navigate(navigated) = navigation.result.unwrap() else {
                panic!("fixture navigation should return its committed loader");
            };
            let loader = navigated.loader_id.unwrap().into_string();
            let mut client = navigation.protocol_output.into_messages();
            while !client.iter().any(|event| event["method"] == "Page.domContentEventFired") {
                let input = receivers.recv_interleaved_input().await.unwrap();
                client.extend(scheduler.complete_interleaved_scheduler_input(&mut receivers, input).await
                    .unwrap_or_else(|failure| panic!("DCL projection failed: {:?}", failure.into_parts().1)).into_messages());
            }
            assert!(client.iter().all(|event| event["method"] != "Page.loadEventFired"),
                "the held stylesheet must keep Load pending");
            release.send_replace(true);

            // Consume real Native publications until the Document has produced
            // Load. Retain that frozen batch at the scheduler delivery boundary
            // instead of handing it to the client.
            let mut frozen = loop {
                let input = receivers.recv_interleaved_input().await.unwrap();
                let output = scheduler.complete_interleaved_scheduler_input(&mut receivers, input).await
                    .unwrap_or_else(|failure| panic!("lifecycle projection failed: {:?}", failure.into_parts().1));
                let projected = messages(&output);
                if projected.iter().any(|event| event["method"] == "Page.loadEventFired") {
                    break output;
                }
                client.extend(projected);
            };
            let source = scheduler.queues.protocol_residences.iter().find_map(|residence| match residence {
                ProtocolSchedulerResidence::ProtocolWork { work, .. }
                    if work.kind() == moli_protocol::ProtocolSchedulerWorkKind::MainDocumentLoadOwnerAction => {
                        work.renderer_output_source()
                    }
                _ => None,
            }).expect("ready post-load work must retain its producing Native cursor");
            while !scheduler.state.with_state(|state| state.renderer_output_cursor_is_projected(source)).unwrap() {
                let input = receivers.recv_interleaved_input().await.unwrap();
                frozen.append(scheduler.complete_interleaved_scheduler_input(&mut receivers, input).await
                    .unwrap_or_else(|failure| panic!("source projection failed: {:?}", failure.into_parts().1)));
            }
            assert!(scheduler.queues.renderer_output_order(source).is_none(),
                "the completed source is retained below at its delivery boundary");
            assert!(scheduler.queues.pending_loads.is_empty());
            scheduler.queues.enqueue_renderer_output_publication(source, frozen, Vec::new(), None);
            let held = scheduler.queues.protocol_residences.pop_back().unwrap();
            scheduler.queues.protocol_residences.push_front(held);

            // Settle any work the specialized drain started, so this checks
            // actual notifications, including frameStoppedLoading and idle.
            let premature = external_drain(&mut scheduler, &mut receivers).await;
            assert!(premature.is_empty(),
                "post-load notifications overtook the held Load: {:?}", messages(&premature));
            assert!(scheduler.queues.pending_loads.is_empty());

            client.extend(scheduler.complete_renderer_output_predecessor_before_runtime_response(
                &RendererOutputFence::new_for_test(source),
            ).into_messages());
            client.extend(external_drain(&mut scheduler, &mut receivers).await.into_messages());
            let load = client.iter().position(|event| event["method"] == "Page.loadEventFired").unwrap();
            let stopped = client.iter().position(|event| event["method"] == "Page.frameStoppedLoading").unwrap();
            let document_request = client.iter().find(|event| event["method"] == "Network.requestWillBeSent"
                && event["params"]["loaderId"] == loader && event["params"]["type"] == "Document")
                .unwrap()["params"]["requestId"].clone();
            let finished = client.iter().position(|event| event["method"] == "Network.loadingFinished"
                && event["params"]["requestId"] == document_request).unwrap();
            assert!(finished < load && load < stopped, "invalid delivery order: {client:#?}");
            server.abort();
        }).await.expect("held-load ordering fixture should finish");
    }).await;
}

#[tokio::test]
async fn specialized_wait_yield_closes_candidate_retained_in_the_main_fifo() {
    let publication = post_load_renderer_publication(PageId::new_for_testing(7), 1);
    let cursor = publication_cursor(&publication);
    let candidate = future_load_candidate(&publication);
    let interest = load_interest_for_publication(&publication);
    let unrelated_later_load = deferred_main_document_load_observation_id(1);
    let (mut scheduler, _protocol_completions) = CdpScheduler::new(CdpConnection::new());
    scheduler.queues.enqueue_renderer_output_publication(
        cursor,
        ProtocolOutputSequence::from_messages(vec![json!({"method": "Runtime.consoleAPICalled"})]),
        Vec::new(),
        Some(candidate),
    );
    scheduler.apply_protocol_work_publications(vec![root_frame_stopped_loading_work(1, "FRAME-1")]);

    let _ = scheduler.complete_ready_protocol_residences_for_external_load_wait();

    assert_eq!(
        scheduler
            .queues
            .bind_main_document_load_predecessor(unrelated_later_load, &interest),
        scheduler.queues.protocol_residence_len(),
        "a specialized wait turn must close candidates retained outside its checked-out snapshot"
    );
}

#[test]
fn scheduler_holds_concrete_renderer_output_until_its_exact_load_predecessor_finishes() {
    let publication = post_load_renderer_publication(PageId::new_for_testing(7), 1);
    let cursor = publication_cursor(&publication);
    let load = deferred_main_document_load_observation_id(1);
    let (mut scheduler, _protocol_completions) = CdpScheduler::new(CdpConnection::new());
    scheduler.queues.enqueue_renderer_output_publication(
        cursor,
        ProtocolOutputSequence::from_messages(vec![json!({"method": "Runtime.consoleAPICalled"})]),
        vec![load],
        None,
    );

    scheduler.close_protocol_residence_load_binding_window();
    assert_eq!(
        scheduler.next_protocol_scheduler_step(),
        ProtocolSchedulerStep::Wait,
        "the exact load predecessor owns scheduler readiness"
    );
    scheduler.queues.satisfy_load_predecessor(load);
    assert_eq!(
        scheduler.next_protocol_scheduler_step(),
        ProtocolSchedulerStep::CompleteReadyResidence
    );
}

#[tokio::test]
async fn renderer_stream_control_is_consumed_without_protocol_residence() {
    let control: RendererOutputTransportMessage = RendererOutputStreamControl::Opened {
        stream: RendererOutputStreamIdentity::new_page_for_protocol_test(PageId::new_for_testing(
            7,
        )),
    }
    .into();
    let (mut scheduler, _protocol_completions) = CdpScheduler::new(CdpConnection::new());
    let output = scheduler.ingest_renderer_publication_now(control);

    assert!(output.is_empty());
    assert_eq!(scheduler.queues.protocol_residence_len(), 0);
    assert_eq!(
        scheduler.next_protocol_scheduler_step(),
        ProtocolSchedulerStep::Wait
    );
}

#[tokio::test]
async fn closed_renderer_transport_fails_an_unprojected_command_fence() {
    let (_background_event_tx, background_event_rx) = tokio::sync::mpsc::unbounded_channel();
    let (_navigation_tx, background_navigation_completion_rx) =
        tokio::sync::mpsc::unbounded_channel();
    let (renderer_tx, renderer_publication_rx) = moli_core::renderer_output_transport_channel();
    drop(renderer_tx);
    let (_activity_tx, document_activity_completion_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut receivers = super::CdpSchedulerEventReceivers {
        state_events_ready: Default::default(),
        protocol_work_completion_rx: tokio::sync::mpsc::unbounded_channel().1,
        document_activity_completion_rx,
        background_event_rx,
        background_navigation_completion_rx,
        renderer_publication_rx,
    };
    let stream =
        RendererOutputStreamIdentity::new_page_for_protocol_test(PageId::new_for_testing(90));
    let predecessor =
        RendererOutputFence::new_for_test(RendererOutputCursor::new_for_test(stream, 1));
    let (mut scheduler, _protocol_completions) = CdpScheduler::new(CdpConnection::new());

    let failure = scheduler
        .project_renderer_output_predecessor_before_devtools_result(&mut receivers, &predecessor)
        .await
        .expect_err("transport terminal must fail rather than wait forever");
    let (output, error) = failure.into_parts();
    assert!(output.is_empty());
    assert_eq!(
        error.kind,
        moli_protocol::automation::DevToolsErrorKind::Internal
    );
    assert!(error.message.contains("closed"));
}

#[test]
fn ordinary_protocol_work_is_ready_without_a_permission_turn() {
    let (mut scheduler, _protocol_completions) = CdpScheduler::new(CdpConnection::new());
    scheduler.apply_protocol_work_publications(vec![root_frame_stopped_loading_work(1, "FRAME-1")]);

    assert_eq!(
        scheduler.next_protocol_scheduler_step(),
        ProtocolSchedulerStep::CompleteReadyResidence
    );
    scheduler.close_protocol_residence_load_binding_window();
    assert_eq!(
        scheduler.next_protocol_scheduler_step(),
        ProtocolSchedulerStep::CompleteReadyResidence
    );

    let Some(ProtocolSchedulerResidence::ProtocolWork { work, .. }) =
        scheduler.queues.pop_next_protocol_residence()
    else {
        panic!("ready output must remain the same concrete work");
    };
    assert_eq!(work.publish_sequence().get(), 1);
    assert_eq!(
        scheduler.next_protocol_scheduler_step(),
        ProtocolSchedulerStep::Wait
    );
}

#[test]
fn concrete_protocol_output_preserves_protocol_publish_order() {
    let (mut scheduler, _protocol_completions) = CdpScheduler::new(CdpConnection::new());
    scheduler.apply_protocol_work_publications(vec![
        root_frame_stopped_loading_work(1, "FRAME-1"),
        root_frame_stopped_loading_work(2, "FRAME-2"),
    ]);

    let sequences = scheduler
        .queues
        .protocol_residences
        .iter()
        .map(|residence| match residence {
            ProtocolSchedulerResidence::ProtocolWork { work, .. } => work.publish_sequence().get(),
            _ => panic!("test enqueued only concrete protocol output"),
        })
        .collect::<Vec<_>>();
    assert_eq!(sequences, [1, 2]);
}

#[test]
#[should_panic(expected = "protocol work must be admitted in exact publication order")]
fn concrete_protocol_output_rejects_duplicate_publication_sequence() {
    let mut queues = SchedulerQueues::default();
    queues.enqueue_protocol_work(root_frame_stopped_loading_work(1, "FRAME-1"));
    queues.enqueue_protocol_work(root_frame_stopped_loading_work(1, "FRAME-1-DUPLICATE"));
}

#[test]
#[should_panic(expected = "protocol work must be admitted in exact publication order")]
fn concrete_protocol_output_rejects_missing_earlier_publication() {
    let mut queues = SchedulerQueues::default();
    queues.enqueue_protocol_work(root_frame_stopped_loading_work(2, "FRAME-2"));
}

#[tokio::test]
async fn pending_navigation_does_not_suspend_projected_renderer_notifications() {
    let publication = post_load_renderer_publication(PageId::new_for_testing(7), 1);
    let mut conn = CdpConnection::new();
    let navigation = arm_background_navigation_request(&mut conn, "LOADER-next");
    let (mut scheduler, _protocol_completions) = CdpScheduler::new(conn);
    let message = json!({"method": "Runtime.consoleAPICalled", "params": {"marker": "current"}});
    scheduler.queues.enqueue_renderer_output_publication(
        publication_cursor(&publication),
        ProtocolOutputSequence::from_messages(vec![message.clone()]),
        Vec::new(),
        Some(future_load_candidate(&publication)),
    );

    assert_eq!(
        scheduler.next_protocol_scheduler_step(),
        ProtocolSchedulerStep::CloseLoadBindingWindow
    );
    scheduler.close_protocol_residence_load_binding_window();
    assert_eq!(
        scheduler.next_protocol_scheduler_step(),
        ProtocolSchedulerStep::CompleteReadyResidence
    );
    assert_eq!(
        scheduler.complete_next_protocol_residence().into_messages(),
        vec![message]
    );
    assert!(
        scheduler
            .state
            .with_state(
                |state| state.has_inflight_background_navigation_for_target(navigation.target_id())
            )
            .expect("live protocol actor state")
    );
    assert_eq!(scheduler.queues.protocol_residence_len(), 0);
}

#[tokio::test]
async fn navigation_notification_snapshot_preserves_exact_load_predecessors() {
    let publication = post_load_renderer_publication(PageId::new_for_testing(7), 1);
    let cursor = publication_cursor(&publication);
    let load = deferred_main_document_load_observation_id(1);
    let mut conn = CdpConnection::new();
    let navigation = arm_background_navigation_request(&mut conn, "LOADER-next");
    let (mut scheduler, _protocol_completions) = CdpScheduler::new(conn);
    let message = json!({"method": "Runtime.consoleAPICalled", "params": {"marker": "after-load"}});
    scheduler.queues.enqueue_renderer_output_publication(
        cursor,
        ProtocolOutputSequence::from_messages(vec![message.clone()]),
        vec![load],
        None,
    );
    assert_eq!(
        scheduler.next_protocol_scheduler_step(),
        ProtocolSchedulerStep::Wait
    );
    let fence = RendererOutputFence::new_for_test(cursor);
    assert!(
        scheduler
            .complete_renderer_output_predecessor_before_runtime_response(&fence)
            .is_empty()
    );
    assert_eq!(scheduler.queues.protocol_residence_len(), 1);

    scheduler.queues.satisfy_load_predecessor(load);
    assert_eq!(
        scheduler
            .complete_renderer_output_predecessor_before_runtime_response(&fence)
            .into_messages(),
        vec![message]
    );
    assert!(
        scheduler
            .state
            .with_state(
                |state| state.has_inflight_background_navigation_for_target(navigation.target_id())
            )
            .expect("live protocol actor state")
    );
    assert_eq!(scheduler.queues.protocol_residence_len(), 0);
}

#[tokio::test]
async fn background_navigation_blocks_only_its_target_protocol_residences() {
    let mut conn = CdpConnection::new();
    let navigation = arm_background_navigation_request(&mut conn, "LOADER-nav");
    let navigation_target_id = navigation.target_id().to_owned();
    let browser_context_id = conn.default_browser_context_id().to_owned();
    let (mut scheduler, _protocol_completions) = CdpScheduler::new(conn);
    scheduler.apply_protocol_work_publications(vec![
        root_frame_stopped_loading_work_for_target(
            1,
            vec![Some("SID-A".to_owned())],
            browser_context_id.clone(),
            navigation_target_id,
            "FRAME-A".to_owned(),
            "LOADER-A".to_owned(),
        ),
        root_frame_stopped_loading_work_for_target(
            2,
            vec![Some("SID-B".to_owned())],
            browser_context_id,
            "TID-independent".to_owned(),
            "FRAME-B".to_owned(),
            "LOADER-B".to_owned(),
        ),
    ]);

    assert_eq!(
        scheduler.next_protocol_scheduler_step(),
        ProtocolSchedulerStep::CompleteReadyResidence,
        "the independent target must advance around target A's navigation"
    );
    scheduler.close_protocol_residence_load_binding_window();
    assert_eq!(
        scheduler.next_protocol_scheduler_step(),
        ProtocolSchedulerStep::CompleteReadyResidence
    );
    assert!(scheduler.complete_next_protocol_residence().is_empty());
    let remaining_sequences = scheduler
        .queues
        .protocol_residences
        .iter()
        .map(|residence| match residence {
            ProtocolSchedulerResidence::ProtocolWork { work, .. } => work.publish_sequence().get(),
            _ => panic!("test enqueued protocol work only"),
        })
        .collect::<Vec<_>>();
    assert_eq!(remaining_sequences, [1]);
    assert_eq!(
        scheduler.next_protocol_scheduler_step(),
        ProtocolSchedulerStep::Wait,
        "target A's own residence must remain gated"
    );
}

#[tokio::test]
async fn protocol_residence_snapshot_skips_another_targets_navigation_gate() {
    let mut conn = CdpConnection::new();
    let navigation = arm_background_navigation_request(&mut conn, "LOADER-A");
    let target_a = navigation.target_id().to_owned();
    let browser_context_id = conn.default_browser_context_id().to_owned();
    let (mut scheduler, _protocol_completions) = CdpScheduler::new(conn);
    scheduler.apply_protocol_work_publications(vec![
        root_frame_stopped_loading_work_for_target(
            1,
            vec![Some("SID-A".to_owned())],
            browser_context_id.clone(),
            target_a,
            "FRAME-A".to_owned(),
            "LOADER-A".to_owned(),
        ),
        root_frame_stopped_loading_work_for_target(
            2,
            vec![Some("SID-B".to_owned())],
            browser_context_id,
            "TID-B".to_owned(),
            "FRAME-B".to_owned(),
            "LOADER-B".to_owned(),
        ),
    ]);
    let snapshot = std::mem::take(&mut scheduler.queues.protocol_residences);

    assert!(
        scheduler
            .complete_protocol_residence_snapshot(snapshot)
            .is_empty()
    );
    let remaining_sequences = scheduler
        .queues
        .protocol_residences
        .iter()
        .map(|residence| match residence {
            ProtocolSchedulerResidence::ProtocolWork { work, .. } => work.publish_sequence().get(),
            _ => panic!("test enqueued protocol work only"),
        })
        .collect::<Vec<_>>();
    assert_eq!(remaining_sequences, [1]);
}

#[test]
fn replacement_navigation_cancels_and_exactly_settles_the_target_owned_request() {
    let mut conn = CdpConnection::new();
    let source = arm_background_navigation_request(&mut conn, "LOADER-source");
    let replacement = arm_background_navigation_request(&mut conn, "LOADER-replacement");
    assert!(source.is_cancelled());
    assert!(!replacement.is_cancelled());

    assert!(!settle_background_navigation_request(&mut conn, &source));
    assert!(
        conn.has_inflight_background_navigation(),
        "a stale completion must not clear the replacement request"
    );
    assert!(settle_background_navigation_request(
        &mut conn,
        &replacement
    ));
    assert!(!conn.has_inflight_background_navigation());
}

#[test]
fn unattributed_fence_wait_does_not_block_an_independent_target() {
    for snapshot in [false, true] {
        let unresolved = publication_cursor(&unconstrained_renderer_publication(
            PageId::new_for_testing(101),
        ));
        let conn = CdpConnection::new();
        let context = conn.default_browser_context_id().to_owned();
        let (mut scheduler, _tasks) = CdpScheduler::new(conn);
        scheduler.apply_protocol_work_publications(vec![
            moli_protocol::test_support::renderer_fenced_output_work(
                1,
                Vec::new(),
                vec![RendererOutputFence::new_for_test(unresolved)],
            ),
            root_frame_stopped_loading_work_for_target(
                2,
                vec![None],
                context,
                "TID-independent".into(),
                "FRAME-independent".into(),
                "LOADER-independent".into(),
            ),
        ]);
        if snapshot {
            let work = std::mem::take(&mut scheduler.queues.protocol_residences);
            assert!(
                scheduler
                    .complete_protocol_residence_snapshot(work)
                    .is_empty()
            );
        } else {
            assert_eq!(scheduler.next_ungated_protocol_residence_index(), Some(1));
            assert!(scheduler.complete_next_protocol_residence().is_empty());
        }
        assert_eq!(scheduler.queues.protocol_residence_len(), 1);
        assert!(
            matches!(scheduler.queues.protocol_residences.front(), Some(ProtocolSchedulerResidence::ProtocolWork { work, .. }) if work.publish_sequence().get()==1)
        );
    }
}

#[test]
fn channel_cleanup_does_not_wait_for_its_targets_navigation() {
    for snapshot in [false, true] {
        let mut conn = CdpConnection::new();
        let pending = arm_background_navigation_request(&mut conn, "LOADER-pending");
        moli_protocol::test_support::publish_bidi_channel_cleanup(&mut conn, "retired-channel");
        let wake = conn.take_scheduler_events();
        let (mut scheduler, _tasks) = CdpScheduler::new(conn);
        scheduler.apply_scheduler_events(wake);
        if snapshot {
            let work = std::mem::take(&mut scheduler.queues.protocol_residences);
            assert!(
                scheduler
                    .complete_protocol_residence_snapshot(work)
                    .is_empty()
            );
        } else {
            assert_eq!(scheduler.next_ungated_protocol_residence_index(), Some(0));
            assert!(scheduler.complete_next_protocol_residence().is_empty());
        }
        assert_eq!(scheduler.queues.protocol_residence_len(), 0);
        assert!(
            !pending.is_cancelled(),
            "cleanup does not replace the navigation"
        );
    }
}

#[tokio::test]
async fn target_retirement_waits_for_its_command_but_not_the_load_it_cancels() {
    tokio::task::LocalSet::new().run_until(async {
    for snapshot in [false, true] {
        let mut conn = CdpConnection::new();
        let pending = arm_background_navigation_request(&mut conn, "LOADER-pending");
        let context = conn.default_browser_context_id().to_owned();
        let target = pending.target_id().to_owned();
        moli_protocol::test_support::publish_protocol_work(&mut conn, |sequence| {
            root_frame_stopped_loading_work_for_target(
                sequence,
                vec![None],
                context,
                target.clone(),
                "FRAME-pending".into(),
                "LOADER-pending".into(),
            )
        });
        let (permit, output) = conn.begin_command_response_flush_permit();
        moli_protocol::test_support::publish_retired_target_termination_after_command_output(
            &mut conn, &target, output,
        );
        let wake = conn.take_scheduler_events();
        let (mut scheduler, _tasks) = CdpScheduler::new(conn);
        scheduler.apply_scheduler_events(wake);
        assert_eq!(scheduler.next_ungated_protocol_residence_index(), None);
        let _ = scheduler.finish_command_dispatch_output_flush(Some(
            super::CommandOutputReleasePermit::new(permit, None),
        ));
        if snapshot {
            let work = std::mem::take(&mut scheduler.queues.protocol_residences);
            let _ = scheduler.complete_protocol_residence_snapshot(work);
        } else {
            assert_eq!(scheduler.next_ungated_protocol_residence_index(), Some(1));
            let _ = scheduler.complete_next_protocol_residence();
        }
        assert_eq!(scheduler.queues.protocol_residence_len(), 1);
        assert!(matches!(scheduler.queues.protocol_residences.front(),
            Some(ProtocolSchedulerResidence::ProtocolWork { work, .. })
                if work.is_root_frame_stopped_loading()));
    }
    }).await;
}

#[test]
fn replacement_navigation_cannot_bypass_its_explicit_delivery_predecessor() {
    for snapshot in [false, true] {
        let mut conn = CdpConnection::new();
        let pending = arm_background_navigation_request(&mut conn, "LOADER-pending");
        let target = pending.target_id().to_owned();
        let context = conn.default_browser_context_id().to_owned();
        let source = publication_cursor(&unconstrained_renderer_publication(
            PageId::new_for_testing(102),
        ));
        let unresolved = publication_cursor(&unconstrained_renderer_publication(
            PageId::new_for_testing(103),
        ));
        let (mut scheduler, _tasks) = CdpScheduler::new(conn);
        scheduler.queues.enqueue_renderer_output_publication(
            source,
            ProtocolOutputSequence::from_background_event(network_response_event_for_target(
                DevToolsNetworkResourceType::Xhr,
                "REQ-before-replacement",
                &target,
            )),
            Vec::new(),
            None,
        );
        scheduler.apply_protocol_work_publications(vec![
            moli_protocol::test_support::protocol_work_from_renderer_output(
                moli_protocol::test_support::renderer_fenced_output_work(
                    1,
                    Vec::new(),
                    vec![RendererOutputFence::new_for_test(unresolved)],
                ),
                source,
            ),
            // Sharing a Target alone does not establish execution order. The
            // navigation depends on delivery of this particular publication.
            moli_protocol::test_support::protocol_work_after_renderer_delivery(
                moli_protocol::test_support::retired_location_navigation_work_for_target(
                    2,
                    context,
                    target,
                    root_document_lifecycle_identity(PageId::new_for_testing(99), 1),
                ),
                source,
            ),
        ]);
        if snapshot {
            let work = std::mem::take(&mut scheduler.queues.protocol_residences);
            assert!(
                scheduler
                    .complete_protocol_residence_snapshot(work)
                    .is_empty()
            );
        } else {
            assert_eq!(
                scheduler.next_ungated_protocol_residence_index(),
                None,
                "replacing a load does not satisfy an unrelated Renderer fence"
            );
            assert!(scheduler.complete_next_protocol_residence().is_empty());
        }
        assert_eq!(scheduler.queues.protocol_residence_len(), 2);
        assert!(!pending.is_cancelled());
    }
}

#[tokio::test]
async fn replacement_navigation_passes_load_blocked_observations_but_checks_its_owner() {
    for (snapshot_drain, inherits_load_predecessor) in
        [(false, false), (true, false), (false, true), (true, true)]
    {
        let mut conn = CdpConnection::new();
        let navigation = arm_background_navigation_request(&mut conn, "LOADER-pending");
        let target = navigation.target_id().to_owned();
        let context = conn.default_browser_context_id().to_owned();
        let (mut scheduler, _protocol_completions) = CdpScheduler::new(conn);
        let publications = vec![
            root_frame_stopped_loading_work_for_target(
                1,
                vec![None],
                context.clone(),
                target.clone(),
                "FRAME-pending".to_owned(),
                "LOADER-pending".to_owned(),
            ),
            moli_protocol::test_support::retired_location_navigation_work_for_target(
                2,
                context,
                target.clone(),
                root_document_lifecycle_identity(PageId::new_for_testing(99), 1),
            ),
        ];
        if inherits_load_predecessor {
            // A navigation published by a still-loading document must reach
            // its owner before that load completes, so it can cancel the load.
            let publication = post_load_renderer_publication(PageId::new_for_testing(7), 1);
            let cursor = publication_cursor(&publication);
            scheduler.queues.enqueue_renderer_output_publication(
                cursor,
                ProtocolOutputSequence::from_background_event(network_response_event_for_target(
                    DevToolsNetworkResourceType::Xhr,
                    "REQ-held-source",
                    &target,
                )),
                vec![deferred_main_document_load_observation_id(1)],
                None,
            );
            scheduler.apply_protocol_work_publications(
                publications
                    .into_iter()
                    .map(|work| {
                        moli_protocol::test_support::protocol_work_from_renderer_output(
                            work, cursor,
                        )
                    })
                    .collect(),
            );
        } else {
            scheduler.apply_protocol_work_publications(publications);
        }
        assert_eq!(
            scheduler.next_ungated_protocol_residence_index(),
            Some(if inherits_load_predecessor { 2 } else { 1 }),
            "replacement must reach its owner even behind a load-blocked observation"
        );
        if snapshot_drain {
            let snapshot = std::mem::take(&mut scheduler.queues.protocol_residences);
            assert!(
                scheduler
                    .complete_protocol_residence_snapshot(snapshot)
                    .is_empty()
            );
        } else {
            scheduler.close_protocol_residence_load_binding_window();
            assert!(scheduler.complete_next_protocol_residence().is_empty());
        }
        assert_eq!(
            scheduler.queues.protocol_residences.len(),
            if inherits_load_predecessor { 2 } else { 1 },
            "the held observation remains, while the replacement action is consumed"
        );
        assert!(
            !navigation.is_cancelled(),
            "an action from a retired owner has no execution authority"
        );
        assert!(
            scheduler
                .state
                .with_state(|state| state.has_inflight_background_navigation())
                .expect("live protocol actor state")
        );
        assert!(
            scheduler
                .state
                .with_state(|state| state.settle_background_navigation_for_test(&navigation))
                .unwrap()
        );
        if inherits_load_predecessor {
            assert_eq!(
                scheduler.next_ungated_protocol_residence_index(),
                None,
                "settling a navigation request does not settle the separate exact load observation"
            );
            scheduler
                .queues
                .satisfy_load_predecessor(deferred_main_document_load_observation_id(1));
        }
        assert_eq!(scheduler.next_ungated_protocol_residence_index(), Some(0));
    }
}

#[test]
fn scheduler_defers_subresource_network_events_until_background_navigation_gate_clears() {
    let mut conn = CdpConnection::new();
    let navigation = arm_background_navigation_request(&mut conn, "LOADER-nav");
    let target_id = navigation.target_id().to_owned();
    let (mut scheduler, _protocol_completions) = CdpScheduler::new(conn);

    let document_output = scheduler.route_background_event_around_inflight_navigation(
        network_response_event_for_target(
            DevToolsNetworkResourceType::Document,
            "REQ-document",
            &target_id,
        ),
    );
    assert_eq!(output_request_ids(document_output), ["REQ-document"]);

    let script_output = scheduler.route_background_event_around_inflight_navigation(
        network_response_event_for_target(
            DevToolsNetworkResourceType::Script,
            "REQ-script",
            &target_id,
        ),
    );
    assert!(script_output.is_empty());
    let script_terminal = scheduler.route_background_event_around_inflight_navigation(
        network_finished_event_for_target(
            DevToolsNetworkResourceType::Script,
            "REQ-script",
            &target_id,
        ),
    );
    assert!(script_terminal.is_empty());
    assert_eq!(scheduler.pending_navigation_background_events.len(), 2);

    assert!(
        scheduler
            .state
            .with_state(|state| state.settle_background_navigation_for_test(&navigation))
            .unwrap()
    );
    let released = scheduler.drain_pending_navigation_background_events();
    let released = released
        .into_background_events()
        .into_iter()
        .map(BackgroundProtocolEvent::into_protocol_message)
        .map(|message| {
            (
                message["method"].as_str().unwrap().to_owned(),
                message["params"]["requestId"].as_str().unwrap().to_owned(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        released,
        [
            (
                "Network.responseReceived".to_owned(),
                "REQ-script".to_owned()
            ),
            (
                "Network.loadingFinished".to_owned(),
                "REQ-script".to_owned()
            ),
        ]
    );
}

#[tokio::test]
async fn target_a_navigation_does_not_defer_target_b_network_events() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let (mut scheduler, mut receivers) =
                CdpScheduler::new_with_initial_state_runtime_config(
                    moli_protocol::CdpInitialStoragePartition::memory(),
                    moli_core::runtime::NavigationRuntimeConfig::new(
                        moli_fetch::FetchConfig::default(),
                        moli_core::OptionalResourceFetchMask::NONE,
                        true,
                        moli_core::LayoutPolicy::OnDemand,
                    ),
                );
            let context = AutomationContext {
                protocol: FrontendProtocol::WebDriverBidi,
                session_id: Some(DevToolsSessionId::from("bidi-test")),
                target_id: None,
                browser_context_id: None,
            };
            let create_target = |context: AutomationContext| {
                AutomationCommand::CreateTarget(DevToolsCreateTargetCommand {
                    context,
                    url: "about:blank".to_owned(),
                    browser_context_id: None,
                    activate: false,
                })
            };
            let first_create = scheduler
                .execute_automation_command_with_protocol_messages(
                    &mut receivers,
                    create_target(context.clone()),
                )
                .await;
            let first_result = first_create.result;
            let AutomationResult::CreateTarget(first_result) =
                first_result.expect("first target should be created")
            else {
                panic!("expected create-target result")
            };
            let first_target_id = first_result.target_id.into_string();
            let second_create = scheduler
                .execute_automation_command_with_protocol_messages(
                    &mut receivers,
                    create_target(context),
                )
                .await;
            let second_result = second_create.result;
            let AutomationResult::CreateTarget(second_result) =
                second_result.expect("second target should be created")
            else {
                panic!("expected create-target result")
            };
            let second_target_id = second_result.target_id.into_string();
            assert_ne!(first_target_id, second_target_id);
            let navigation = scheduler
                .state
                .with_state(|state| {
                    state.arm_background_navigation_for_test(&first_target_id, "LOADER-A")
                })
                .unwrap();
            let target_a = navigation.target_id().to_owned();
            let target_b = if target_a == second_target_id {
                first_target_id
            } else {
                second_target_id
            };
            assert!(
                scheduler
                    .state
                    .with_state(
                        |state| state.has_inflight_background_navigation_for_target(&target_a)
                    )
                    .unwrap()
            );
            assert!(
                !scheduler
                    .state
                    .with_state(
                        |state| state.has_inflight_background_navigation_for_target(&target_b)
                    )
                    .unwrap()
            );
            let target_b_output = scheduler.route_background_event_around_inflight_navigation(
                network_response_event_for_target(
                    DevToolsNetworkResourceType::Xhr,
                    "REQ-B",
                    &target_b,
                ),
            );
            assert_eq!(output_request_ids(target_b_output), ["REQ-B"]);

            let navigation_b = scheduler
                .state
                .with_state(|state| state.arm_background_navigation_for_test(&target_b, "LOADER-B"))
                .unwrap();
            let target_b_held = scheduler.route_background_event_around_inflight_navigation(
                network_response_event_for_target(
                    DevToolsNetworkResourceType::Xhr,
                    "REQ-B-held",
                    &target_b,
                ),
            );
            assert!(target_b_held.is_empty());
            let target_a_output = scheduler.route_background_event_around_inflight_navigation(
                network_response_event_for_target(
                    DevToolsNetworkResourceType::Xhr,
                    "REQ-A",
                    &target_a,
                ),
            );
            assert!(target_a_output.is_empty());
            assert_eq!(scheduler.pending_navigation_background_events.len(), 2);

            assert!(
                scheduler
                    .state
                    .with_state(|state| state.settle_background_navigation_for_test(&navigation))
                    .unwrap()
            );
            assert_eq!(
                output_request_ids(scheduler.drain_pending_navigation_background_events()),
                ["REQ-A"],
                "settling target A must not release target B's held event"
            );
            assert_eq!(scheduler.pending_navigation_background_events.len(), 1);
            assert!(
                scheduler
                    .state
                    .with_state(|state| state.settle_background_navigation_for_test(&navigation_b))
                    .unwrap()
            );
            assert_eq!(
                output_request_ids(scheduler.drain_pending_navigation_background_events()),
                ["REQ-B-held"]
            );
        })
        .await;
}

#[test]
fn navigation_gate_release_precedes_later_renderer_boundary_network_output() {
    let mut conn = CdpConnection::new();
    let navigation = arm_background_navigation_request(&mut conn, "LOADER-nav");
    let target_id = navigation.target_id().to_owned();
    let (mut scheduler, _protocol_completions) = CdpScheduler::new(conn);

    let request_id = "REQ-boundary-race";
    assert!(
        scheduler
            .route_background_event_around_inflight_navigation(network_request_event_for_target(
                DevToolsNetworkResourceType::Xhr,
                request_id,
                &target_id,
            ))
            .is_empty()
    );

    assert!(
        scheduler
            .state
            .with_state(|state| state.settle_background_navigation_for_test(&navigation))
            .unwrap()
    );
    let mut output = ProtocolOutputSequence::empty();
    scheduler.append_navigation_gate_release_before_renderer_boundary(&mut output);
    output.append(ProtocolOutputSequence::from_background_event(
        network_response_event_for_target(DevToolsNetworkResourceType::Xhr, request_id, &target_id),
    ));
    output.append(ProtocolOutputSequence::from_background_event(
        network_finished_event_for_target(DevToolsNetworkResourceType::Xhr, request_id, &target_id),
    ));

    let methods = output
        .into_background_events()
        .into_iter()
        .map(BackgroundProtocolEvent::into_protocol_message)
        .map(|message| message["method"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(
        methods,
        [
            "Network.requestWillBeSent",
            "Network.responseReceived",
            "Network.loadingFinished",
        ]
    );
}

#[test]
fn foreground_load_wait_network_barrier_keeps_unrelated_raw_events_opaque() {
    let mut barrier =
        ForegroundNavigationNetworkBarrier::for_navigation_wait(Some(DevToolsNavigationWait::Load));
    let raw = format!(
        r#"{{"method":"Debugger.scriptParsed","params":{{"sourceMapURL":"{}","description":"\ud800"}}}}"#,
        "x".repeat(256 * 1024)
    );
    let message = moli_inspector_json::InspectorJson::from_slice(raw.as_bytes()).unwrap();
    let events = barrier
        .route_event(BackgroundProtocolEvent::immediate(message))
        .into_background_events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].protocol_method(), Some("Debugger.scriptParsed"));
    assert!(!events[0].protocol_json().unwrap().is_materialized());
    assert_eq!(
        events
            .into_iter()
            .next()
            .unwrap()
            .into_cdp_message()
            .to_string(),
        raw
    );
    assert!(barrier.finish().is_empty());
}

#[test]
fn foreground_load_wait_network_barrier_releases_document_response_before_subresources() {
    let mut barrier =
        ForegroundNavigationNetworkBarrier::for_navigation_wait(Some(DevToolsNavigationWait::Load));

    let script_output = barrier.route_event(network_response_event(
        DevToolsNetworkResourceType::Script,
        "REQ-script",
    ));
    assert!(script_output.is_empty());

    let document_output = barrier.route_event(network_response_event(
        DevToolsNetworkResourceType::Document,
        "REQ-document",
    ));
    assert_eq!(
        output_request_ids(document_output),
        ["REQ-document", "REQ-script"]
    );

    let image_output = barrier.route_event(network_response_event(
        DevToolsNetworkResourceType::Image,
        "REQ-image",
    ));
    assert_eq!(output_request_ids(image_output), ["REQ-image"]);
    assert!(barrier.finish().is_empty());
}

#[test]
fn foreground_load_wait_network_barrier_drains_subresources_if_document_response_never_arrives() {
    let mut barrier =
        ForegroundNavigationNetworkBarrier::for_navigation_wait(Some(DevToolsNavigationWait::Load));

    let script_output = barrier.route_event(network_response_event(
        DevToolsNetworkResourceType::Script,
        "REQ-script",
    ));
    assert!(script_output.is_empty());

    assert_eq!(output_request_ids(barrier.finish()), ["REQ-script"]);
}

#[tokio::test(flavor = "current_thread")]
async fn adapter_closes_provisional_load_window_and_delivers_in_the_same_turn() {
    tokio::task::LocalSet::new()
        .run_until(async {
            let publication = post_load_renderer_publication(PageId::new_for_testing(7), 1);
            let cursor = publication_cursor(&publication);
            let (mut scheduler, _completions) = CdpScheduler::new(CdpConnection::new());
            let message =
                json!({"method":"Runtime.consoleAPICalled", "params":{"marker":"single-turn"}});
            scheduler.queues.enqueue_renderer_output_publication(
                cursor,
                ProtocolOutputSequence::from_messages(vec![message.clone()]),
                Vec::new(),
                Some(future_load_candidate(&publication)),
            );
            assert_eq!(
                scheduler.next_protocol_scheduler_step(),
                ProtocolSchedulerStep::CloseLoadBindingWindow
            );
            let mut adapter = super::ProtocolAdapterScheduler::default();
            adapter.schedule_turn_if_needed(&scheduler, false);
            let input = adapter.recv_input().await;
            let super::ProtocolAdapterSchedulerAdvance::ProtocolResidenceCompleted(output) =
                adapter.advance_input(&mut scheduler, input)
            else {
                panic!("one adapter turn closes the binding window and delivers ready output")
            };
            assert_eq!(output.into_messages(), [message]);
            assert_eq!(scheduler.queues.protocol_residence_len(), 0);
            assert_eq!(
                scheduler.queues.bind_main_document_load_predecessor(
                    deferred_main_document_load_observation_id(1),
                    &load_interest_for_publication(&publication)
                ),
                0,
                "a later unrelated load cannot recover the closed candidate"
            );
        })
        .await;
}
