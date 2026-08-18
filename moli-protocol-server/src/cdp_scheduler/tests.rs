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
    conn::RuntimeInspectorResponseReady,
    devtools_runtime::{
        AutomationEvent, BrowserDownloadWillBeginEvent, DevToolsCommand, DevToolsCommandContext,
        DevToolsCommandResult, DevToolsCreateTargetCommand, DevToolsFrameId, DevToolsLoaderId,
        DevToolsNavigationWait, DevToolsNetworkResourceType, DevToolsProtocol, DevToolsRequestId,
        DevToolsSessionId, DevToolsTargetId, NavigationFrameEvent, NavigationFrameEventKind,
        NetworkRequestEvent,
    },
    test_support::{
        arm_background_navigation_request, arm_background_navigation_request_for_target,
        deferred_main_document_load_observation_id, deferred_main_document_load_output_interest,
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
fn screencast_deadlines_are_one_hz_downsampled_without_catch_up() {
    let started = tokio::time::Instant::now();
    assert_eq!(
        page_screencast_interval(1),
        std::time::Duration::from_secs(1)
    );
    assert_eq!(
        page_screencast_interval(3),
        std::time::Duration::from_secs(3)
    );

    let completion = started + std::time::Duration::from_secs(10);
    assert_eq!(
        next_page_screencast_deadline(completion, page_screencast_interval(1)),
        started + std::time::Duration::from_secs(11),
        "an overdue capture schedules from completion instead of catching up"
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
fn mixed_owner_protocol_output_keeps_the_conservative_global_gate() {
    let mut conn = CdpConnection::new();
    let navigation = arm_background_navigation_request(&mut conn, "LOADER-known");
    let target_id = navigation.target_id().to_owned();
    let known = network_response_event_for_target(
        DevToolsNetworkResourceType::Xhr,
        "REQ-known",
        &target_id,
    );
    assert_eq!(
        ProtocolOutputSequence::from_background_event(known.clone())
            .navigation_gate_target_ids(&conn),
        [target_id]
    );

    let unresolved = BackgroundProtocolEvent::immediate(json!({
        "method": "Runtime.consoleAPICalled",
        "sessionId": "SID-missing",
        "params": {}
    }));
    let mixed = ProtocolOutputSequence::from_background_events(vec![known, unresolved]);
    assert!(
        mixed.navigation_gate_target_ids(&conn).is_empty(),
        "an atomic batch with any unresolved owner must remain connection-gated"
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

    queues.satisfy_front_client_turn_predecessor();
    assert!(!queues.should_complete_next_residence());
    queues.satisfy_load_predecessor(second);
    assert!(
        !queues.should_complete_next_residence(),
        "an unrelated completed load must not release the frozen batch early"
    );
    queues.satisfy_load_predecessor(first);
    assert!(queues.should_complete_next_residence());
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
    queues.enqueue_protocol_work(
        root_frame_stopped_loading_work(1, "FRAME-1"),
        Vec::new(),
        None,
    );

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
    let mut scheduler = CdpScheduler::new(CdpConnection::new());
    scheduler.apply_scheduler_events_with_load_predecessors(
        vec![CdpSchedulerEvent::ProtocolWorkPublished {
            work: root_frame_stopped_loading_work(1, "FRAME-1"),
        }],
        &[load],
        None,
    );

    assert_eq!(
        scheduler.next_protocol_scheduler_step(),
        ProtocolSchedulerStep::SatisfyClientTurnPredecessor
    );
    scheduler.satisfy_front_protocol_residence_client_turn_predecessor();
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
        root_frame_stopped_loading_work(1, "FRAME-1"),
        Vec::new(),
        Some(candidate),
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
fn client_turn_yield_closes_the_provisional_load_binding_window() {
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
        root_frame_stopped_loading_work(1, "FRAME-1"),
        Vec::new(),
        Some(candidate),
    );

    queues.satisfy_front_client_turn_predecessor();

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
fn client_turn_yield_closes_candidate_behind_an_unrelated_front_residence() {
    let publication = post_load_renderer_publication(PageId::new_for_testing(7), 1);
    let cursor = publication_cursor(&publication);
    let candidate = future_load_candidate(&publication);
    let interest = load_interest_for_publication(&publication);
    let unrelated_later_load = deferred_main_document_load_observation_id(1);
    let mut queues = SchedulerQueues::default();
    queues.enqueue_protocol_work(
        root_frame_stopped_loading_work(1, "FRAME-1"),
        Vec::new(),
        None,
    );
    queues.enqueue_renderer_output_publication(
        cursor,
        ProtocolOutputSequence::from_messages(vec![json!({"method": "Runtime.consoleAPICalled"})]),
        Vec::new(),
        Some(candidate),
    );

    queues.satisfy_front_client_turn_predecessor();

    assert_eq!(
        queues.bind_main_document_load_predecessor(unrelated_later_load, &interest),
        queues.protocol_residence_len(),
        "yielding for an older front residence must also close every candidate admitted before that client turn"
    );
    assert!(matches!(
        queues.protocol_residences.get(1),
        Some(ProtocolSchedulerResidence::RendererOutputPublication(work))
            if work.load_predecessors.is_empty()
    ));
}

#[tokio::test]
async fn runtime_response_releases_its_frozen_renderer_publication_predecessor() {
    let publication = post_load_renderer_publication(PageId::new_for_testing(1), 1);
    let candidate = future_load_candidate(&publication);
    let cursor = publication_cursor(&publication);
    let mut scheduler = CdpScheduler::new(CdpConnection::new());
    scheduler.queues.enqueue_renderer_output_publication(
        cursor,
        ProtocolOutputSequence::from_messages(vec![json!({
            "method": "Page.navigatedWithinDocument"
        })]),
        Vec::new(),
        Some(candidate),
    );

    let output = scheduler
        .complete_renderer_output_predecessor_before_runtime_response(
            &RendererOutputFence::new_for_test(cursor),
        )
        .await;

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
    let mut scheduler = CdpScheduler::new(CdpConnection::new());
    scheduler.queues.enqueue_renderer_output_publication(
        cursor,
        ProtocolOutputSequence::from_messages(vec![json!({
            "method": "Runtime.consoleAPICalled"
        })]),
        vec![load],
        None,
    );

    let output = scheduler
        .complete_renderer_output_predecessor_before_runtime_response(
            &RendererOutputFence::new_for_test(cursor),
        )
        .await;

    assert!(output.is_empty());
    assert_eq!(
        scheduler.queues.protocol_residence_len(),
        1,
        "the exact load observation remains the only release authority"
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
        root_frame_stopped_loading_work(1, "FRAME-1"),
        Vec::new(),
        Some(candidate),
    );

    assert!(
        queues.take_external_load_wait_snapshot().is_empty(),
        "a specialized wait drain must not steal same-ingress work before command completion gets its load-binding opportunity"
    );
    assert_eq!(queues.protocol_residence_len(), 2);
}

#[tokio::test]
async fn specialized_wait_yield_closes_candidate_retained_in_the_main_fifo() {
    let publication = post_load_renderer_publication(PageId::new_for_testing(7), 1);
    let cursor = publication_cursor(&publication);
    let candidate = future_load_candidate(&publication);
    let interest = load_interest_for_publication(&publication);
    let unrelated_later_load = deferred_main_document_load_observation_id(1);
    let mut scheduler = CdpScheduler::new(CdpConnection::new());
    scheduler.queues.enqueue_renderer_output_publication(
        cursor,
        ProtocolOutputSequence::from_messages(vec![json!({"method": "Runtime.consoleAPICalled"})]),
        Vec::new(),
        Some(candidate),
    );
    scheduler.apply_scheduler_events(vec![CdpSchedulerEvent::ProtocolWorkPublished {
        work: root_frame_stopped_loading_work(1, "FRAME-1"),
    }]);

    let _ = scheduler
        .complete_ready_protocol_residences_for_external_load_wait()
        .await;

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
    let mut scheduler = CdpScheduler::new(CdpConnection::new());
    scheduler.queues.enqueue_renderer_output_publication(
        cursor,
        ProtocolOutputSequence::from_messages(vec![json!({"method": "Runtime.consoleAPICalled"})]),
        vec![load],
        None,
    );

    scheduler.satisfy_front_protocol_residence_client_turn_predecessor();
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
    let mut scheduler = CdpScheduler::new(CdpConnection::new());
    let output = scheduler.ingest_renderer_publication_now(control).await;

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
    let mut receivers = super::CdpSchedulerEventReceivers {
        background_event_rx,
        background_navigation_completion_rx,
        renderer_publication_rx,
    };
    let stream =
        RendererOutputStreamIdentity::new_page_for_protocol_test(PageId::new_for_testing(90));
    let predecessor =
        RendererOutputFence::new_for_test(RendererOutputCursor::new_for_test(stream, 1));
    let mut scheduler = CdpScheduler::new(CdpConnection::new());

    let failure = scheduler
        .project_renderer_output_predecessor_before_devtools_result(&mut receivers, &predecessor)
        .await
        .expect_err("transport terminal must fail rather than wait forever");
    let (output, error) = failure.into_parts();
    assert!(output.is_empty());
    assert_eq!(
        error.kind,
        moli_protocol::devtools_runtime::DevToolsErrorKind::Internal
    );
    assert!(error.message.contains("closed"));
}

#[test]
fn concrete_protocol_output_waits_for_its_client_turn_predecessor() {
    let mut scheduler = CdpScheduler::new(CdpConnection::new());
    scheduler.apply_scheduler_events(vec![CdpSchedulerEvent::ProtocolWorkPublished {
        work: root_frame_stopped_loading_work(1, "FRAME-1"),
    }]);

    assert_eq!(
        scheduler.next_protocol_scheduler_step(),
        ProtocolSchedulerStep::SatisfyClientTurnPredecessor
    );
    scheduler.satisfy_front_protocol_residence_client_turn_predecessor();
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
    let mut scheduler = CdpScheduler::new(CdpConnection::new());
    scheduler.apply_scheduler_events(vec![
        CdpSchedulerEvent::ProtocolWorkPublished {
            work: root_frame_stopped_loading_work(1, "FRAME-1"),
        },
        CdpSchedulerEvent::ProtocolWorkPublished {
            work: root_frame_stopped_loading_work(2, "FRAME-2"),
        },
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
#[should_panic(
    expected = "protocol work must enter scheduler residence in exact publication order"
)]
fn concrete_protocol_output_rejects_duplicate_publication_sequence() {
    let mut queues = SchedulerQueues::default();
    queues.enqueue_protocol_work(
        root_frame_stopped_loading_work(1, "FRAME-1"),
        Vec::new(),
        None,
    );
    queues.enqueue_protocol_work(
        root_frame_stopped_loading_work(1, "FRAME-1-DUPLICATE"),
        Vec::new(),
        None,
    );
}

#[test]
#[should_panic(
    expected = "protocol work must enter scheduler residence in exact publication order"
)]
fn concrete_protocol_output_rejects_missing_earlier_publication() {
    let mut queues = SchedulerQueues::default();
    queues.enqueue_protocol_work(
        root_frame_stopped_loading_work(2, "FRAME-2"),
        Vec::new(),
        None,
    );
}

#[tokio::test]
async fn background_navigation_blocks_only_its_target_protocol_residences() {
    let mut conn = CdpConnection::new();
    let navigation = arm_background_navigation_request(&mut conn, "LOADER-nav");
    let navigation_target_id = navigation.target_id().to_owned();
    let browser_context_id = conn.default_browser_context_id().to_owned();
    let mut scheduler = CdpScheduler::new(conn);
    scheduler.apply_scheduler_events(vec![
        CdpSchedulerEvent::ProtocolWorkPublished {
            work: root_frame_stopped_loading_work_for_target(
                1,
                vec![Some("SID-A".to_owned())],
                browser_context_id.clone(),
                navigation_target_id,
                "FRAME-A".to_owned(),
                "LOADER-A".to_owned(),
            ),
        },
        CdpSchedulerEvent::ProtocolWorkPublished {
            work: root_frame_stopped_loading_work_for_target(
                2,
                vec![Some("SID-B".to_owned())],
                browser_context_id,
                "TID-independent".to_owned(),
                "FRAME-B".to_owned(),
                "LOADER-B".to_owned(),
            ),
        },
    ]);

    assert_eq!(
        scheduler.next_protocol_scheduler_step(),
        ProtocolSchedulerStep::SatisfyClientTurnPredecessor,
        "the independent target must advance around target A's navigation"
    );
    scheduler.satisfy_front_protocol_residence_client_turn_predecessor();
    assert_eq!(
        scheduler.next_protocol_scheduler_step(),
        ProtocolSchedulerStep::CompleteReadyResidence
    );
    assert!(
        scheduler
            .complete_next_protocol_residence()
            .await
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
    let mut scheduler = CdpScheduler::new(conn);
    scheduler.apply_scheduler_events(vec![
        CdpSchedulerEvent::ProtocolWorkPublished {
            work: root_frame_stopped_loading_work_for_target(
                1,
                vec![Some("SID-A".to_owned())],
                browser_context_id.clone(),
                target_a,
                "FRAME-A".to_owned(),
                "LOADER-A".to_owned(),
            ),
        },
        CdpSchedulerEvent::ProtocolWorkPublished {
            work: root_frame_stopped_loading_work_for_target(
                2,
                vec![Some("SID-B".to_owned())],
                browser_context_id,
                "TID-B".to_owned(),
                "FRAME-B".to_owned(),
                "LOADER-B".to_owned(),
            ),
        },
    ]);
    let snapshot = std::mem::take(&mut scheduler.queues.protocol_residences);

    assert!(
        scheduler
            .complete_protocol_residence_snapshot(snapshot)
            .await
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
fn scheduler_defers_subresource_network_events_until_background_navigation_gate_clears() {
    let mut conn = CdpConnection::new();
    let navigation = arm_background_navigation_request(&mut conn, "LOADER-nav");
    let target_id = navigation.target_id().to_owned();
    let mut scheduler = CdpScheduler::new(conn);

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

    assert!(settle_background_navigation_request(
        &mut scheduler.conn,
        &navigation
    ));
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
    let mut conn = CdpConnection::new();
    conn.install_default_browser_target();
    let context = DevToolsCommandContext {
        protocol: DevToolsProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from("bidi-test")),
        target_id: None,
        browser_context_id: None,
    };
    let create_target = |context: DevToolsCommandContext| {
        DevToolsCommand::CreateTarget(DevToolsCreateTargetCommand {
            context,
            url: "about:blank".to_owned(),
            browser_context_id: None,
            activate: false,
        })
    };
    let first_create = conn
        .execute_devtools_command(create_target(context.clone()))
        .await;
    let (first_result, _) = first_create.into_parts();
    let DevToolsCommandResult::CreateTarget(first_result) =
        first_result.expect("first target should be created")
    else {
        panic!("expected create-target result")
    };
    let first_target_id = first_result.target_id.into_string();
    let second_create = conn.execute_devtools_command(create_target(context)).await;
    let (second_result, _) = second_create.into_parts();
    let DevToolsCommandResult::CreateTarget(second_result) =
        second_result.expect("second target should be created")
    else {
        panic!("expected create-target result")
    };
    let second_target_id = second_result.target_id.into_string();
    assert_ne!(first_target_id, second_target_id);
    let navigation = arm_background_navigation_request(&mut conn, "LOADER-A");
    let target_a = navigation.target_id().to_owned();
    let target_b = if target_a == second_target_id {
        first_target_id
    } else {
        second_target_id
    };
    assert!(conn.has_inflight_background_navigation_for_target(&target_a));
    assert!(!conn.has_inflight_background_navigation_for_target(&target_b));

    let mut scheduler = CdpScheduler::new(conn);
    let target_b_output = scheduler.route_background_event_around_inflight_navigation(
        network_response_event_for_target(DevToolsNetworkResourceType::Xhr, "REQ-B", &target_b),
    );
    assert_eq!(output_request_ids(target_b_output), ["REQ-B"]);

    let navigation_b =
        arm_background_navigation_request_for_target(&mut scheduler.conn, &target_b, "LOADER-B");
    let target_b_held = scheduler.route_background_event_around_inflight_navigation(
        network_response_event_for_target(
            DevToolsNetworkResourceType::Xhr,
            "REQ-B-held",
            &target_b,
        ),
    );
    assert!(target_b_held.is_empty());
    let target_a_output = scheduler.route_background_event_around_inflight_navigation(
        network_response_event_for_target(DevToolsNetworkResourceType::Xhr, "REQ-A", &target_a),
    );
    assert!(target_a_output.is_empty());
    assert_eq!(scheduler.pending_navigation_background_events.len(), 2);

    assert!(settle_background_navigation_request(
        &mut scheduler.conn,
        &navigation
    ));
    assert_eq!(
        output_request_ids(scheduler.drain_pending_navigation_background_events()),
        ["REQ-A"],
        "settling target A must not release target B's held event"
    );
    assert_eq!(scheduler.pending_navigation_background_events.len(), 1);
    assert!(settle_background_navigation_request(
        &mut scheduler.conn,
        &navigation_b
    ));
    assert_eq!(
        output_request_ids(scheduler.drain_pending_navigation_background_events()),
        ["REQ-B-held"]
    );
}

#[test]
fn navigation_gate_release_precedes_later_renderer_boundary_network_output() {
    let mut conn = CdpConnection::new();
    let navigation = arm_background_navigation_request(&mut conn, "LOADER-nav");
    let target_id = navigation.target_id().to_owned();
    let mut scheduler = CdpScheduler::new(conn);

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

    assert!(settle_background_navigation_request(
        &mut scheduler.conn,
        &navigation
    ));
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
