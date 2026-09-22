use super::*;
use crate::frame_owner_model::FrameDocumentLoadDeliveryPhase;

fn handle(index: usize) -> DomHandle {
    DomHandle::new(index)
}

fn url(value: &str) -> Url {
    Url::parse(value).expect("test url should parse")
}

fn policy_context() -> crate::types::SubresourcePolicyContext {
    crate::types::SubresourcePolicyContext::default()
}

fn policy_container() -> crate::document_runtime::DocumentPolicyContainer {
    crate::document_runtime::DocumentPolicyContainer::default()
}

fn service_worker_client(value: u64) -> crate::service_worker_runtime::ServiceWorkerClientId {
    crate::service_worker_runtime::ServiceWorkerClientId::from_u64_for_test(value)
}

fn commit_test_child_document(
    store: &mut FrameOwnerStore,
    child_handle: DomHandle,
    document_handle: DomHandle,
    frame_id: &str,
    parent_frame_id: Option<&str>,
) -> FrameDocumentTaskOwner {
    store.ensure_child_frame(
        child_handle,
        frame_id.to_owned(),
        parent_frame_id.map(str::to_owned),
    );
    store
        .commit_child_document(
            child_handle,
            document_handle,
            url(&format!("https://{frame_id}.test/")),
            url(&format!("https://{frame_id}.test/")),
            format!("https://{frame_id}.test"),
            None,
            RequestCredentialsMode::SameOrigin,
            policy_container(),
            policy_context(),
        )
        .expect("test child document should commit");
    store
        .current_child_document_task_owner(child_handle)
        .expect("test child document should expose an owner")
}

fn materialize_test_child_realm(
    store: &mut FrameOwnerStore,
    child_handle: DomHandle,
    inspector_execution_context_id: i64,
) -> FrameRealmId {
    let owner = store
        .current_child_document_task_owner(child_handle)
        .expect("test child document should expose an owner");
    store
        .ensure_child_realm(child_handle)
        .expect("test child realm identity should reserve");
    let request = store
        .request_child_realm_materialization(child_handle, owner)
        .expect("test child realm should accept materialization");
    let realm_id = request.realm_id();
    assert_eq!(
        store.bind_child_realm_inspector_context(
            child_handle,
            owner,
            inspector_execution_context_id,
        ),
        Some(realm_id)
    );
    assert!(store.complete_child_realm_materialization(child_handle, owner, realm_id));
    realm_id
}

fn advance_test_child_to_domcontentloaded(
    store: &mut FrameOwnerStore,
    child_handle: DomHandle,
    owner: FrameDocumentTaskOwner,
) {
    let interactive = store
        .finish_current_child_document_parsing(child_handle, owner.document_owner())
        .expect("test parser should finish");
    assert!(store.apply_current_child_document_interactive_transition(interactive));
    let domcontentloaded = store
        .prepare_current_child_document_domcontentloaded_transition(child_handle, owner)
        .expect("test document should prepare DCL");
    assert!(store.apply_current_child_document_domcontentloaded_transition(domcontentloaded));
}

fn prepare_test_child_load_delivery(
    store: &mut FrameOwnerStore,
    child_handle: DomHandle,
    owner: FrameDocumentTaskOwner,
) -> FrameDocumentLoadDeliveryTask {
    advance_test_child_to_domcontentloaded(store, child_handle, owner);
    let complete = store
        .prepare_current_child_document_complete_transition(child_handle, owner)
        .expect("test document should prepare complete");
    assert!(store.apply_current_child_document_complete_transition(complete));
    FrameDocumentLoadDeliveryTask {
        child_handle,
        owner,
    }
}

fn finish_test_child_load_delivery(
    store: &mut FrameOwnerStore,
    task: FrameDocumentLoadDeliveryTask,
) -> FrameDocumentLoadDispatchFinish {
    let phases = [
        FrameDocumentLoadDeliveryPhase::WindowLoad,
        FrameDocumentLoadDeliveryPhase::OwnerElementLoad,
        FrameDocumentLoadDeliveryPhase::PageShow,
        FrameDocumentLoadDeliveryPhase::FrameFinish,
    ];
    for (index, phase) in phases.into_iter().enumerate() {
        let action = store
            .begin_current_child_document_load_delivery(task)
            .expect("current child load should expose its next delivery phase");
        assert_eq!(action.phase(), phase);
        let progress = store
            .finish_current_child_document_load_delivery(action)
            .expect("current child load phase should finish");
        if index + 1 == phases.len() {
            let FrameDocumentLoadDeliveryProgress::Finished(finish) = progress else {
                panic!("frame-finish phase must produce the typed finish output");
            };
            return finish;
        }
        assert_eq!(progress, FrameDocumentLoadDeliveryProgress::Continue(task));
    }
    unreachable!("frame-finish phase returns from the loop")
}

#[test]
fn main_frame_identity_uses_reserved_owner_records() {
    let mut store = FrameOwnerStore::default();
    let realm_id = store.ensure_main_frame(
        handle(1),
        url("https://example.test/page"),
        url("https://example.test/base/"),
        "https://example.test".to_owned(),
        policy_container(),
        policy_context(),
        Some(service_worker_client(17)),
    );

    assert_eq!(realm_id, FrameRealmId(0));
    assert_eq!(
        store.current_main_document_task_owner(),
        Some(FrameDocumentTaskOwner::new(
            FrameSchedulerLaneId(0),
            LocalWindowId(0),
            DocumentId(0),
        ))
    );
    let frame_id = FrameId("main".to_owned());
    let frame = store.frames.get(&frame_id).expect("main frame record");
    assert_eq!(frame.kind, FrameKind::Main);
    assert_eq!(frame.owner_element_handle, None);
    assert_eq!(frame.window_proxy_id, WindowProxyId(0));
    assert_eq!(frame.scheduler_lane_id, FrameSchedulerLaneId(0));
    assert_eq!(frame.current_local_window_id, Some(LocalWindowId(0)));
    assert_eq!(frame.current_document_id, Some(DocumentId(0)));
    assert_eq!(frame.lifecycle, FrameLifecycleState::Attached);
    let lane = store
        .scheduler_lanes
        .get(&FrameSchedulerLaneId(0))
        .expect("main frame scheduler lane record");
    assert_eq!(lane.id, FrameSchedulerLaneId(0));
    assert_eq!(lane.frame_id, frame_id);
    assert_eq!(lane.lifecycle, FrameSchedulerLaneLifecycleState::Active);
    assert!(store.document_is_current(DocumentId(0)));
    assert_eq!(
        store
            .local_windows
            .get(&LocalWindowId(0))
            .and_then(|window| window.realm_id),
        Some(FrameRealmId(0))
    );
    assert_eq!(
        store
            .documents
            .get(&DocumentId(0))
            .map(|document| document.document_handle),
        Some(handle(1))
    );
    let snapshot = store
        .current_main_owner_snapshot()
        .expect("main owner snapshot should exist");
    assert_eq!(snapshot.frame_id, frame_id);
    assert_eq!(snapshot.kind, FrameKind::Main);
    assert_eq!(snapshot.parent_frame_id, None);
    assert_eq!(snapshot.owner_element_handle, None);
    assert_eq!(snapshot.window_proxy_id, WindowProxyId(0));
    assert_eq!(snapshot.scheduler_lane_id, FrameSchedulerLaneId(0));
    assert_eq!(snapshot.local_window_id, LocalWindowId(0));
    assert_eq!(snapshot.document_id, DocumentId(0));
    assert_eq!(snapshot.document_handle, handle(1));
    assert_eq!(snapshot.document_url, url("https://example.test/page"));
    assert_eq!(
        snapshot.document_base_url,
        url("https://example.test/base/")
    );
    assert_eq!(snapshot.realm_id, Some(FrameRealmId(0)));
    assert_eq!(snapshot.settings.origin, "https://example.test");
    assert_eq!(
        snapshot.settings.credentials_mode,
        RequestCredentialsMode::SameOrigin
    );
    assert_eq!(
        snapshot.settings.document_policy_container,
        policy_container()
    );
    assert_eq!(
        snapshot.settings.subresource_policy_context,
        policy_context()
    );
    assert_eq!(
        snapshot.settings.service_worker_client_id,
        Some(service_worker_client(17))
    );
    assert_eq!(
        snapshot.settings.module_map_owner,
        ModuleMapOwner::Document(DocumentId(0))
    );
    let main_job = store
        .frame_source_script_job(
            &frame_id,
            FrameScriptJobKind::ProtocolEvaluate,
            "3 + 4".to_owned(),
        )
        .expect("main owner snapshot should build a frame script job");
    assert_eq!(main_job.frame_id, frame_id);
    assert_eq!(main_job.local_window_id, LocalWindowId(0));
    assert_eq!(main_job.document_id, DocumentId(0));
    assert_eq!(main_job.kind, FrameScriptJobKind::ProtocolEvaluate);
    assert_eq!(
        store.current_realm_id_for_frame_script_job(&main_job),
        Some(FrameRealmId(0))
    );
    assert_eq!(
        store.current_frame_document_task_owner(&frame_id),
        Some(FrameDocumentTaskOwner::new(
            FrameSchedulerLaneId(0),
            LocalWindowId(0),
            DocumentId(0),
        ))
    );
    assert_eq!(
        store.current_materialized_realm_id_for_document_task_owner(FrameDocumentTaskOwner::new(
            FrameSchedulerLaneId(0),
            LocalWindowId(0),
            DocumentId(0),
        )),
        Some(FrameRealmId(0))
    );
    let main_task_owner =
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(0), LocalWindowId(0), DocumentId(0));
    assert_eq!(
        store.document_task_owner_realm_currentness(main_task_owner, FrameRealmId(0)),
        FrameDocumentTaskRealmCurrentness::Current {
            owner: main_task_owner,
            realm_id: FrameRealmId(0),
        }
    );
    assert_eq!(
        store.frame_document_owner_realm_currentness(
            main_task_owner.document_owner(),
            FrameRealmId(0),
        ),
        FrameDocumentTaskRealmCurrentness::Current {
            owner: main_task_owner,
            realm_id: FrameRealmId(0),
        }
    );
    assert!(store.frame_owner_elements.is_empty());
}

#[test]
fn main_document_replacement_rotates_document_owner_without_replacing_window_or_realm() {
    let mut store = FrameOwnerStore::default();
    store.ensure_main_frame(
        handle(1),
        url("https://example.test/page"),
        url("https://example.test/base/"),
        "https://example.test".to_owned(),
        policy_container(),
        policy_context(),
        Some(service_worker_client(17)),
    );
    let original = store
        .current_main_owner_snapshot()
        .expect("original main owner snapshot");

    let transition = store
        .replace_main_document(
            handle(1),
            url("https://example.test/page"),
            url("https://example.test/base/"),
        )
        .expect("main document replacement should commit");
    let replacement = store
        .current_main_owner_snapshot()
        .expect("replacement main owner snapshot");

    assert_eq!(transition.retired_owner().document_id, original.document_id);
    assert_eq!(
        transition.current_owner().document_id,
        replacement.document_id
    );
    assert_ne!(original.document_id, replacement.document_id);
    assert_eq!(original.scheduler_lane_id, replacement.scheduler_lane_id);
    assert_eq!(original.local_window_id, replacement.local_window_id);
    assert_eq!(original.realm_id, replacement.realm_id);
    assert_eq!(replacement.document_handle, original.document_handle);
    assert_eq!(
        store
            .documents
            .get(&original.document_id)
            .map(|document| document.lifecycle),
        Some(DocumentLifecycleState::Replaced)
    );
    assert_eq!(
        replacement.settings.module_map_owner,
        ModuleMapOwner::Document(replacement.document_id)
    );
    assert!(store.document_task_owner_is_current(transition.current_owner()));
    assert!(!store.document_task_owner_is_current(transition.retired_owner()));
    let chained_transition = store
        .replace_main_document(
            handle(1),
            url("https://example.test/page"),
            url("https://example.test/base/"),
        )
        .expect("second same-turn main replacement should commit");
    assert_eq!(
        chained_transition.retired_owner(),
        transition.current_owner(),
        "same-turn replacements must form one contiguous owner transition chain"
    );
    assert_eq!(
        store.take_pending_main_document_owner_transitions(),
        vec![transition, chained_transition],
        "the owner store must journal the committed replacement exactly once"
    );
    assert!(
        store
            .take_pending_main_document_owner_transitions()
            .is_empty(),
        "claiming the main replacement transaction must consume its journal entry"
    );
}

#[test]
fn main_document_lifecycle_actions_are_owned_and_replacement_stale_drops_them() {
    let mut store = FrameOwnerStore::default();
    store.ensure_main_frame(
        handle(1),
        url("https://example.test/page"),
        url("https://example.test/base/"),
        "https://example.test".to_owned(),
        policy_container(),
        policy_context(),
        None,
    );
    let original = store
        .current_main_owner_snapshot()
        .expect("main owner snapshot");
    let original_owner = FrameDocumentTaskOwner::new(
        original.scheduler_lane_id,
        original.local_window_id,
        original.document_id,
    );
    assert_eq!(
        store
            .documents
            .get(&original.document_id)
            .expect("main document record")
            .lifecycle_progress
            .load_delay_token_count(),
        2,
        "main documents must own parsing and DOMContentLoaded transition tokens from creation"
    );
    assert_eq!(
        store.current_main_document_domcontentloaded_transition_is_ready(original_owner),
        Some(false),
        "DOMContentLoaded cannot become ready before the owner reaches interactive"
    );

    let stale_interactive = store
        .finish_current_main_document_parsing(original_owner)
        .expect("original parser should finish");
    let first_replacement = store
        .replace_main_document(
            handle(1),
            url("https://example.test/first"),
            url("https://example.test/first"),
        )
        .expect("first replacement");
    assert!(!store.apply_current_main_document_interactive_transition(stale_interactive));

    let first_owner = first_replacement.current_owner();
    let first_interactive = store
        .finish_current_main_document_parsing(first_owner)
        .expect("first replacement parser should finish");
    assert!(store.apply_current_main_document_interactive_transition(first_interactive));
    assert_eq!(
        store.current_main_document_domcontentloaded_transition_is_ready(first_owner),
        Some(true)
    );
    let stale_domcontentloaded = store
        .prepare_current_main_document_domcontentloaded_transition(first_owner)
        .expect("first replacement should prepare DCL");
    let second_replacement = store
        .replace_main_document(
            handle(1),
            url("https://example.test/second"),
            url("https://example.test/second"),
        )
        .expect("second replacement");
    assert!(!store.apply_current_main_document_domcontentloaded_transition(stale_domcontentloaded));

    let second_owner = second_replacement.current_owner();
    let second_interactive = store
        .finish_current_main_document_parsing(second_owner)
        .expect("second replacement parser should finish");
    assert!(store.apply_current_main_document_interactive_transition(second_interactive));
    let second_domcontentloaded = store
        .prepare_current_main_document_domcontentloaded_transition(second_owner)
        .expect("second replacement should prepare DCL");
    assert!(store.apply_current_main_document_domcontentloaded_transition(second_domcontentloaded));
    assert_eq!(
        store.current_main_document_complete_transition_is_ready(second_owner),
        Some(true)
    );
    let stale_complete = store
        .prepare_current_main_document_complete_transition(second_owner)
        .expect("second replacement should prepare complete");
    let third_replacement = store
        .replace_main_document(
            handle(1),
            url("https://example.test/third"),
            url("https://example.test/third"),
        )
        .expect("third replacement");
    assert!(!store.apply_current_main_document_complete_transition(stale_complete));

    let third_owner = third_replacement.current_owner();
    let third_interactive = store
        .finish_current_main_document_parsing(third_owner)
        .expect("third replacement parser should finish");
    assert!(store.apply_current_main_document_interactive_transition(third_interactive));
    let third_domcontentloaded = store
        .prepare_current_main_document_domcontentloaded_transition(third_owner)
        .expect("third replacement should prepare DCL");
    assert!(store.apply_current_main_document_domcontentloaded_transition(third_domcontentloaded));
    let third_complete = store
        .prepare_current_main_document_complete_transition(third_owner)
        .expect("third replacement should prepare complete");
    assert!(store.apply_current_main_document_complete_transition(third_complete));
    assert!(store.begin_current_main_document_load_dispatch(third_owner));
    assert_eq!(
        store.finish_current_main_document_load_dispatch(third_owner),
        Some(MainDocumentLoadCompletionState::Completed)
    );
}

#[test]
fn main_parser_deferred_delay_is_the_dcl_fact_source_and_replacement_owned() {
    let mut store = FrameOwnerStore::default();
    store.ensure_main_frame(
        handle(1),
        url("https://example.test/page"),
        url("https://example.test/base/"),
        "https://example.test".to_owned(),
        policy_container(),
        policy_context(),
        None,
    );
    let snapshot = store
        .current_main_owner_snapshot()
        .expect("main document owner snapshot");
    let owner = FrameDocumentTaskOwner::new(
        snapshot.scheduler_lane_id,
        snapshot.local_window_id,
        snapshot.document_id,
    );
    let parser_deferred = store
        .acquire_current_main_parser_deferred_script_load_delay(owner)
        .expect("parser-deferred acceptance should acquire lifecycle ownership");
    assert_eq!(
        store.current_main_document_has_parser_deferred_script_load_delay(owner),
        Some(true)
    );

    let interactive = store
        .finish_current_main_document_parsing(owner)
        .expect("parser EOF should prepare interactive");
    assert!(store.apply_current_main_document_interactive_transition(interactive));
    assert_eq!(
        store.current_main_document_domcontentloaded_transition_is_ready(owner),
        Some(false),
        "the lifecycle token must block DCL without consulting scheduler queues"
    );
    assert!(store.release_parser_deferred_script_load_delay(owner, parser_deferred));
    assert_eq!(
        store.current_main_document_has_parser_deferred_script_load_delay(owner),
        Some(false)
    );
    assert_eq!(
        store.current_main_document_domcontentloaded_transition_is_ready(owner),
        Some(true),
        "settling the exact PendingScript token must unblock DCL"
    );

    let stale_token = store
        .acquire_current_main_parser_deferred_script_load_delay(owner)
        .expect("current owner should acquire another delay");
    let replacement = store
        .replace_main_document(
            handle(1),
            url("https://example.test/replacement"),
            url("https://example.test/replacement"),
        )
        .expect("main document replacement");
    assert_eq!(replacement.retired_owner(), owner);
    assert_eq!(
        store.current_main_document_has_parser_deferred_script_load_delay(owner),
        None,
        "retired owner lifecycle state must no longer be queryable as current"
    );
    assert!(
        !store.release_parser_deferred_script_load_delay(owner, stale_token),
        "a stale completion must not release state on the replacement document"
    );
    assert_eq!(
        store.current_main_document_has_parser_deferred_script_load_delay(
            replacement.current_owner()
        ),
        Some(false)
    );
}

#[test]
fn main_async_script_delays_are_exact_and_replacement_owned() {
    let mut store = FrameOwnerStore::default();
    store.ensure_main_frame(
        handle(1),
        url("https://example.test/page"),
        url("https://example.test/base/"),
        "https://example.test".to_owned(),
        policy_container(),
        policy_context(),
        None,
    );
    let snapshot = store
        .current_main_owner_snapshot()
        .expect("main document owner snapshot");
    let owner = FrameDocumentTaskOwner::new(
        snapshot.scheduler_lane_id,
        snapshot.local_window_id,
        snapshot.document_id,
    );
    let classic = store
        .acquire_current_main_document_script_load_delay(
            owner,
            MainDocumentScriptLoadDelayKind::Classic,
        )
        .expect("classic dynamic script should bind to the current document");
    let module = store
        .acquire_current_main_document_script_load_delay(
            owner,
            MainDocumentScriptLoadDelayKind::Module,
        )
        .expect("module dynamic script should bind to the current document");
    assert!(classic.load_delay_token().is_some());
    assert!(module.load_delay_token().is_some());
    assert_eq!(
        store.current_main_document_has_async_script_load_delay(owner),
        Some(true)
    );

    let interactive = store
        .finish_current_main_document_parsing(owner)
        .expect("parser EOF should prepare interactive");
    assert!(store.apply_current_main_document_interactive_transition(interactive));
    let domcontentloaded = store
        .prepare_current_main_document_domcontentloaded_transition(owner)
        .expect("async scripts must not block DOMContentLoaded");
    assert!(store.apply_current_main_document_domcontentloaded_transition(domcontentloaded));
    assert_eq!(
        store.current_main_document_complete_transition_is_ready(owner),
        Some(false),
        "dynamic script lifecycle tokens must block complete without queue scans"
    );

    let repeated_classic = MainDocumentScriptLoadDelayLease::new(
        classic.owner(),
        classic.kind(),
        classic.load_delay_token(),
    );
    assert_eq!(
        store.release_main_document_script_load_delay(classic),
        MainDocumentScriptLoadDelayRelease::StillBlocked
    );
    assert_eq!(
        store.release_main_document_script_load_delay(repeated_classic),
        MainDocumentScriptLoadDelayRelease::NotOwned,
        "one dynamic script lease must settle exactly once"
    );
    assert_eq!(
        store.current_main_document_complete_transition_is_ready(owner),
        Some(false),
        "the remaining module binding must continue to delay complete"
    );
    assert_eq!(
        store.release_main_document_script_load_delay(module),
        MainDocumentScriptLoadDelayRelease::BecameUnblocked
    );
    assert_eq!(
        store.current_main_document_has_async_script_load_delay(owner),
        Some(false)
    );
    assert_eq!(
        store.current_main_document_complete_transition_is_ready(owner),
        Some(true)
    );

    let stale = store
        .acquire_current_main_document_script_load_delay(
            owner,
            MainDocumentScriptLoadDelayKind::Classic,
        )
        .expect("current document should accept another dynamic script");
    let replacement = store
        .replace_main_document(
            handle(1),
            url("https://example.test/replacement"),
            url("https://example.test/replacement"),
        )
        .expect("main document replacement");
    assert_eq!(
        store.release_main_document_script_load_delay(stale),
        MainDocumentScriptLoadDelayRelease::NotOwned,
        "stale dynamic script completion must not mutate the replacement lifecycle"
    );
    assert_eq!(
        store.current_main_document_has_async_script_load_delay(replacement.current_owner()),
        Some(false)
    );
}

#[test]
fn main_style_load_event_binding_delays_complete_until_event_settlement() {
    let mut store = FrameOwnerStore::default();
    store.ensure_main_frame(
        handle(1),
        url("https://example.test/"),
        url("https://example.test/"),
        "https://example.test".to_owned(),
        policy_container(),
        policy_context(),
        None,
    );
    let snapshot = store
        .current_main_owner_snapshot()
        .expect("main owner snapshot");
    let owner = FrameDocumentTaskOwner::new(
        snapshot.scheduler_lane_id,
        snapshot.local_window_id,
        snapshot.document_id,
    );
    let binding = store
        .accept_current_main_style_load_event(owner, handle(8))
        .expect("current style event should acquire document ownership");
    assert_eq!(binding.owner(), owner);
    assert_eq!(binding.element(), handle(8));
    assert!(binding.load_delay_token().is_some());

    let interactive = store
        .finish_current_main_document_parsing(owner)
        .expect("parser EOF should prepare interactive");
    assert!(store.apply_current_main_document_interactive_transition(interactive));
    let domcontentloaded = store
        .prepare_current_main_document_domcontentloaded_transition(owner)
        .expect("style loads do not block DOMContentLoaded");
    assert!(store.apply_current_main_document_domcontentloaded_transition(domcontentloaded));
    assert_eq!(
        store.current_main_document_complete_transition_is_ready(owner),
        Some(false),
        "the exact style event token must block complete"
    );

    assert!(store.settle_main_style_load_event(binding));
    assert!(
        !store.settle_main_style_load_event(binding),
        "one style event binding must settle exactly once"
    );
    assert_eq!(
        store.current_main_document_complete_transition_is_ready(owner),
        Some(true)
    );
}

#[test]
fn main_modulepreload_owner_never_allocates_a_load_event_delay() {
    let mut store = FrameOwnerStore::default();
    store.ensure_main_frame(
        handle(1),
        url("https://example.test/"),
        url("https://example.test/"),
        "https://example.test".to_owned(),
        policy_container(),
        policy_context(),
        None,
    );
    let snapshot = store
        .current_main_owner_snapshot()
        .expect("main owner snapshot");
    let owner = FrameDocumentTaskOwner::new(
        snapshot.scheduler_lane_id,
        snapshot.local_window_id,
        snapshot.document_id,
    );

    let event_owner = store
        .accept_current_main_modulepreload_event_owner(owner, handle(10))
        .expect("current modulepreload should retain exact event ownership");
    assert_eq!(event_owner.owner(), owner);
    assert_eq!(event_owner.element(), handle(10));
    assert_eq!(
        store.current_main_document_has_style_load_event_delay(owner),
        Some(false),
        "network-phase owner identity must not allocate a load-delay token"
    );
}

#[test]
fn main_style_load_event_binding_cannot_settle_replacement_document() {
    let mut store = FrameOwnerStore::default();
    store.ensure_main_frame(
        handle(1),
        url("https://example.test/"),
        url("https://example.test/"),
        "https://example.test".to_owned(),
        policy_container(),
        policy_context(),
        None,
    );
    let snapshot = store
        .current_main_owner_snapshot()
        .expect("main owner snapshot");
    let owner = FrameDocumentTaskOwner::new(
        snapshot.scheduler_lane_id,
        snapshot.local_window_id,
        snapshot.document_id,
    );
    let stale = store
        .accept_current_main_style_load_event(owner, handle(9))
        .expect("current style event should bind");
    let replacement = store
        .replace_main_document(
            handle(1),
            url("https://example.test/replacement"),
            url("https://example.test/replacement"),
        )
        .expect("main replacement");

    assert!(!store.main_style_load_event_is_current(stale));
    assert!(
        !store.settle_main_style_load_event(stale),
        "stale style completion must not mutate replacement lifecycle"
    );
    assert_eq!(
        store.current_main_document_complete_transition_is_ready(replacement.current_owner()),
        Some(false),
        "replacement starts in loading independently of the stale token"
    );
}

#[test]
fn main_image_request_delay_is_element_owned_and_replacement_safe() {
    let mut store = FrameOwnerStore::default();
    store.ensure_main_frame(
        handle(1),
        url("https://example.test/"),
        url("https://example.test/"),
        "https://example.test".to_owned(),
        policy_container(),
        policy_context(),
        None,
    );
    let snapshot = store
        .current_main_owner_snapshot()
        .expect("main owner snapshot");
    let owner = FrameDocumentTaskOwner::new(
        snapshot.scheduler_lane_id,
        snapshot.local_window_id,
        snapshot.document_id,
    );
    let event = store
        .accept_current_main_image_load_delay(owner, handle(8))
        .expect("main image request should bind");
    assert_eq!(event.owner(), owner);
    assert_eq!(event.element(), handle(8));
    assert!(event.load_delay_token().is_some());

    let interactive = store
        .finish_current_main_document_parsing(owner)
        .expect("parser EOF should prepare interactive");
    assert!(store.apply_current_main_document_interactive_transition(interactive));
    let domcontentloaded = store
        .prepare_current_main_document_domcontentloaded_transition(owner)
        .expect("images do not block DOMContentLoaded");
    assert!(store.apply_current_main_document_domcontentloaded_transition(domcontentloaded));
    assert_eq!(
        store.current_main_document_complete_transition_is_ready(owner),
        Some(false)
    );

    assert!(store.settle_main_image_load_delay(event));
    assert_eq!(
        store.current_main_document_complete_transition_is_ready(owner),
        Some(true),
        "one image request sequence must release its one exact delay"
    );

    let stale = store
        .accept_current_main_image_load_delay(owner, handle(9))
        .expect("current document should accept another image event");
    let replacement = store
        .replace_main_document(
            handle(1),
            url("https://example.test/replacement"),
            url("https://example.test/replacement"),
        )
        .expect("main replacement");
    assert!(!store.main_image_load_delay_is_current(stale));
    assert!(
        !store.settle_main_image_load_delay(stale),
        "stale image event must not release replacement state"
    );
    assert_eq!(
        store.current_main_document_complete_transition_is_ready(replacement.current_owner()),
        Some(false)
    );
}

#[test]
fn main_stylesheet_subresource_delays_are_exact_and_replacement_safe() {
    let mut store = FrameOwnerStore::default();
    store.ensure_main_frame(
        handle(1),
        url("https://example.test/"),
        url("https://example.test/"),
        "https://example.test".to_owned(),
        policy_container(),
        policy_context(),
        None,
    );
    let snapshot = store
        .current_main_owner_snapshot()
        .expect("main owner snapshot");
    let owner = FrameDocumentTaskOwner::new(
        snapshot.scheduler_lane_id,
        snapshot.local_window_id,
        snapshot.document_id,
    );
    let image = store
        .accept_current_main_stylesheet_subresource_load_delay(owner)
        .expect("CSS image should bind");
    let font = store
        .accept_current_main_stylesheet_subresource_load_delay(owner)
        .expect("CSS font should bind independently");
    assert!(image.load_delay_token().is_some());
    assert_ne!(image.load_delay_token(), font.load_delay_token());

    let interactive = store
        .finish_current_main_document_parsing(owner)
        .expect("parser EOF should prepare interactive");
    assert!(store.apply_current_main_document_interactive_transition(interactive));
    let domcontentloaded = store
        .prepare_current_main_document_domcontentloaded_transition(owner)
        .expect("stylesheet subresources do not block DOMContentLoaded");
    assert!(store.apply_current_main_document_domcontentloaded_transition(domcontentloaded));
    assert_eq!(
        store.current_main_document_complete_transition_is_ready(owner),
        Some(false)
    );
    assert!(store.settle_stylesheet_subresource_load_delay(image));
    assert_eq!(
        store.current_main_document_complete_transition_is_ready(owner),
        Some(false),
        "the remaining CSS font token must keep complete blocked"
    );
    assert!(store.settle_stylesheet_subresource_load_delay(font));
    assert_eq!(
        store.current_main_document_complete_transition_is_ready(owner),
        Some(true)
    );

    let stale = store
        .accept_current_main_stylesheet_subresource_load_delay(owner)
        .expect("current document should accept another CSS resource");
    let replacement = store
        .replace_main_document(
            handle(1),
            url("https://example.test/replacement"),
            url("https://example.test/replacement"),
        )
        .expect("main replacement");
    assert!(!store.stylesheet_subresource_load_delay_is_current(stale));
    assert!(!store.settle_stylesheet_subresource_load_delay(stale));
    assert_eq!(
        store.current_main_document_complete_transition_is_ready(replacement.current_owner()),
        Some(false)
    );
}

#[test]
fn main_media_delay_is_element_owned_and_cannot_settle_replacement() {
    let mut store = FrameOwnerStore::default();
    store.ensure_main_frame(
        handle(1),
        url("https://example.test/"),
        url("https://example.test/"),
        "https://example.test".to_owned(),
        policy_container(),
        policy_context(),
        None,
    );
    let snapshot = store
        .current_main_owner_snapshot()
        .expect("main owner snapshot");
    let owner = FrameDocumentTaskOwner::new(
        snapshot.scheduler_lane_id,
        snapshot.local_window_id,
        snapshot.document_id,
    );
    let binding = store
        .accept_current_main_media_load_delay(owner, handle(8))
        .expect("main media selection should bind");
    assert_eq!(binding.owner(), owner);
    assert_eq!(binding.element(), handle(8));
    assert!(binding.load_delay_token().is_some());

    let interactive = store
        .finish_current_main_document_parsing(owner)
        .expect("parser EOF should prepare interactive");
    assert!(store.apply_current_main_document_interactive_transition(interactive));
    let domcontentloaded = store
        .prepare_current_main_document_domcontentloaded_transition(owner)
        .expect("media does not block DOMContentLoaded");
    assert!(store.apply_current_main_document_domcontentloaded_transition(domcontentloaded));
    assert_eq!(
        store.current_main_document_complete_transition_is_ready(owner),
        Some(false),
        "the media element must retain its load delay through first data"
    );
    assert!(store.settle_main_media_load_delay(binding));
    assert_eq!(
        store.current_main_document_complete_transition_is_ready(owner),
        Some(true)
    );

    let stale = store
        .accept_current_main_media_load_delay(owner, handle(9))
        .expect("current document should accept another media selection");
    let replacement = store
        .replace_main_document(
            handle(1),
            url("https://example.test/replacement"),
            url("https://example.test/replacement"),
        )
        .expect("main replacement");
    assert!(!store.main_media_load_delay_is_current(stale));
    assert!(
        !store.settle_main_media_load_delay(stale),
        "stale media completion must not release replacement state"
    );
    assert_eq!(
        store.current_main_document_complete_transition_is_ready(replacement.current_owner()),
        Some(false)
    );
}

#[test]
fn child_document_commit_replaces_current_owner_records() {
    let mut store = FrameOwnerStore::default();
    let child_handle = handle(10);
    let frame_id =
        store.ensure_child_frame(child_handle, "frame-1".to_owned(), Some("main".to_owned()));
    let scheduler_lane_id = store
        .frames
        .get(&frame_id)
        .expect("child frame record should exist")
        .scheduler_lane_id;
    assert_ne!(scheduler_lane_id, FrameSchedulerLaneId(0));
    let owner_element = store
        .frame_owner_element_for_child_handle(child_handle)
        .expect("child frame owner element record should exist");
    assert_eq!(owner_element.owner_handle, child_handle);
    assert_eq!(owner_element.content_frame_id.as_ref(), Some(&frame_id));
    assert_eq!(
        owner_element.parent_frame_id,
        Some(FrameId("main".to_owned()))
    );
    assert_eq!(
        owner_element.lifecycle,
        FrameOwnerElementLifecycleState::Attached
    );

    let (first_window, first_document) = store
        .commit_child_document(
            child_handle,
            handle(11),
            url("https://child.test/one"),
            url("https://child.test/one"),
            "https://child.test".to_owned(),
            None,
            RequestCredentialsMode::SameOrigin,
            policy_container(),
            policy_context(),
        )
        .expect("first child document should commit");
    let realm_id = materialize_test_child_realm(&mut store, child_handle, 77);
    let first_task_owner = store
        .current_child_document_task_owner(child_handle)
        .expect("first child document should have a current task owner");
    assert_eq!(realm_id, FrameRealmId(1));
    assert_ne!(realm_id, FrameRealmId(77));
    assert_eq!(
        store
            .current_child_owner_snapshot(child_handle)
            .and_then(|snapshot| snapshot.realm_id),
        Some(realm_id)
    );
    assert_eq!(
        store
            .realms
            .get(&realm_id)
            .and_then(|realm| realm.inspector_execution_context_id),
        Some(77)
    );
    let updated_realm_id = materialize_test_child_realm(&mut store, child_handle, 177);
    assert_eq!(updated_realm_id, realm_id);
    assert_eq!(
        store
            .realms
            .get(&realm_id)
            .and_then(|realm| realm.inspector_execution_context_id),
        Some(177)
    );
    assert!(store.document_is_current(first_document));

    let job = store
        .child_source_script_job(
            child_handle,
            FrameScriptJobKind::ProtocolEvaluate,
            "1 + 1".to_owned(),
        )
        .expect("current child owner should build script job");
    assert_eq!(job.local_window_id, first_window);
    assert_eq!(job.document_id, first_document);
    assert_eq!(job.kind, FrameScriptJobKind::ProtocolEvaluate);
    assert_eq!(
        store.current_realm_id_for_frame_script_job(&job),
        Some(realm_id)
    );
    assert_eq!(
        store.current_reserved_realm_id_for_document_task_owner(first_task_owner),
        Some(realm_id)
    );
    assert_eq!(
        store.current_child_document_task_owner_reserved_realm(child_handle),
        Some((first_task_owner, realm_id))
    );
    assert_eq!(
        store.child_document_task_owner_realm_currentness(child_handle, first_task_owner, realm_id),
        FrameDocumentTaskRealmCurrentness::Current {
            owner: first_task_owner,
            realm_id,
        }
    );
    assert_eq!(
        store.document_task_owner_realm_currentness(first_task_owner, realm_id),
        FrameDocumentTaskRealmCurrentness::Current {
            owner: first_task_owner,
            realm_id,
        }
    );
    assert_eq!(
        store.document_task_owner_realm_currentness(first_task_owner, FrameRealmId(99)),
        FrameDocumentTaskRealmCurrentness::StaleRealm {
            owner: first_task_owner,
            current_realm_id: realm_id,
        }
    );
    assert_eq!(
        store.frame_document_owner_realm_currentness(first_task_owner.document_owner(), realm_id),
        FrameDocumentTaskRealmCurrentness::Current {
            owner: first_task_owner,
            realm_id,
        }
    );

    let function_job = store
        .child_function_constructor_script_job(
            child_handle,
            vec!["value".to_owned(), "extra".to_owned()],
            "return value + extra;".to_owned(),
        )
        .expect("current child owner should build function constructor job");
    assert_eq!(function_job.local_window_id, first_window);
    assert_eq!(function_job.document_id, first_document);
    assert_eq!(function_job.kind, FrameScriptJobKind::FunctionConstructor);
    assert_eq!(
        store.current_realm_id_for_frame_script_job(&function_job),
        Some(realm_id)
    );
    let FrameScriptSource::FunctionConstructor(function_source) = &function_job.source else {
        panic!("function constructor job should carry function constructor source");
    };
    assert_eq!(function_source.parameters.as_slice(), ["value", "extra"]);
    assert_eq!(function_source.body, "return value + extra;");

    let parser_classic_job = store
        .child_parser_classic_script_job(
            child_handle,
            Some(handle(13)),
            "globalThis.__parserClassic = true;".to_owned(),
        )
        .expect("current child owner should build parser classic script job");
    assert_eq!(parser_classic_job.local_window_id, first_window);
    assert_eq!(parser_classic_job.document_id, first_document);
    assert_eq!(parser_classic_job.current_script, Some(handle(13)));
    assert_eq!(parser_classic_job.kind, FrameScriptJobKind::ParserClassic);
    assert_eq!(parser_classic_job.script_url, url("https://child.test/one"));
    assert_eq!(parser_classic_job.base_url, url("https://child.test/one"));
    assert_eq!(
        store.current_realm_id_for_frame_script_job(&parser_classic_job),
        Some(realm_id)
    );
    let FrameScriptSource::SourceText(parser_source) = &parser_classic_job.source else {
        panic!("parser classic job should carry source text");
    };
    assert_eq!(parser_source, "globalThis.__parserClassic = true;");

    let external_classic_job = store
        .child_external_classic_script_job(
            child_handle,
            Some(handle(14)),
            url("https://cdn.child.test/app.js"),
            url("https://cdn.child.test/app.js"),
            "globalThis.__externalClassic = true;".to_owned(),
        )
        .expect("current child owner should build external classic script job");
    assert_eq!(external_classic_job.local_window_id, first_window);
    assert_eq!(external_classic_job.document_id, first_document);
    assert_eq!(external_classic_job.current_script, Some(handle(14)));
    assert_eq!(
        external_classic_job.kind,
        FrameScriptJobKind::ExternalClassic
    );
    assert_eq!(
        external_classic_job.script_url,
        url("https://cdn.child.test/app.js")
    );
    assert_eq!(
        external_classic_job.base_url,
        url("https://cdn.child.test/app.js")
    );
    assert_eq!(
        external_classic_job.credentials_mode,
        RequestCredentialsMode::SameOrigin
    );
    assert_eq!(
        store.current_realm_id_for_frame_script_job(&external_classic_job),
        Some(realm_id)
    );
    assert!(
        store
            .child_parser_classic_script_job_for_owner(
                child_handle,
                first_window,
                first_document,
                Some(handle(15)),
                "globalThis.__ownedParserClassic = true;".to_owned(),
            )
            .is_some(),
        "current owner token should build parser classic jobs"
    );
    let dynamic_classic_job = store
        .child_dynamic_classic_script_job_for_owner(
            child_handle,
            first_window,
            first_document,
            Some(handle(16)),
            "globalThis.__dynamicClassic = true;".to_owned(),
        )
        .expect("current child owner should build dynamic classic script job");
    assert_eq!(dynamic_classic_job.local_window_id, first_window);
    assert_eq!(dynamic_classic_job.document_id, first_document);
    assert_eq!(dynamic_classic_job.current_script, Some(handle(16)));
    assert_eq!(dynamic_classic_job.kind, FrameScriptJobKind::DynamicClassic);
    assert_eq!(
        dynamic_classic_job.script_url,
        url("https://child.test/one")
    );
    assert_eq!(dynamic_classic_job.base_url, url("https://child.test/one"));
    assert_eq!(
        store.current_realm_id_for_frame_script_job(&dynamic_classic_job),
        Some(realm_id)
    );

    let (second_window, second_document) = store
        .commit_child_document(
            child_handle,
            handle(12),
            url("https://child.test/two"),
            url("https://child.test/two"),
            "https://child.test".to_owned(),
            Some("strict-origin".to_owned()),
            RequestCredentialsMode::Include,
            policy_container(),
            policy_context(),
        )
        .expect("second child document should commit");

    assert_ne!(first_window, second_window);
    assert_ne!(first_document, second_document);
    assert!(!store.document_is_current(first_document));
    assert!(store.document_is_current(second_document));
    assert_eq!(
        store
            .local_windows
            .get(&first_window)
            .map(|window| window.lifecycle),
        Some(LocalWindowLifecycleState::NavigatedAway)
    );
    assert_eq!(
        store
            .documents
            .get(&first_document)
            .map(|document| document.lifecycle),
        Some(DocumentLifecycleState::Replaced)
    );
    assert_eq!(
        store.realms.get(&realm_id).map(|realm| realm.lifecycle),
        Some(FrameRealmLifecycleState::DetachedReachable)
    );
    let second_snapshot = store
        .current_child_owner_snapshot(child_handle)
        .expect("second child document should have a current owner snapshot");
    assert_eq!(second_snapshot.scheduler_lane_id, scheduler_lane_id);
    assert_eq!(second_snapshot.local_window_id, second_window);
    assert_eq!(second_snapshot.document_id, second_document);
    assert_eq!(second_snapshot.realm_id, None);
    assert_eq!(store.current_realm_id_for_frame_script_job(&job), None);
    assert_eq!(
        store.current_realm_id_for_frame_script_job(&function_job),
        None
    );
    assert_eq!(
        store.current_realm_id_for_frame_script_job(&parser_classic_job),
        None
    );
    assert_eq!(
        store.current_realm_id_for_frame_script_job(&external_classic_job),
        None
    );
    assert_eq!(
        store.current_realm_id_for_frame_script_job(&dynamic_classic_job),
        None
    );
    assert_eq!(
        store.current_reserved_realm_id_for_document_task_owner(first_task_owner),
        None
    );
    assert_eq!(
        store.current_child_document_task_owner_reserved_realm(child_handle),
        None
    );
    assert_eq!(
        store.child_document_task_owner_realm_currentness(child_handle, first_task_owner, realm_id),
        FrameDocumentTaskRealmCurrentness::StaleOwner
    );
    assert_eq!(
        store.document_task_owner_realm_currentness(first_task_owner, realm_id),
        FrameDocumentTaskRealmCurrentness::StaleOwner
    );
    let second_task_owner = store
        .current_child_document_task_owner(child_handle)
        .expect("second child document should have a current task owner");
    assert_eq!(
        store.document_task_owner_realm_currentness(second_task_owner, realm_id),
        FrameDocumentTaskRealmCurrentness::MissingRealm {
            owner: second_task_owner,
        }
    );
    assert_eq!(
        store.frame_document_owner_realm_currentness(second_task_owner.document_owner(), realm_id),
        FrameDocumentTaskRealmCurrentness::MissingRealm {
            owner: second_task_owner,
        }
    );
    assert!(
        store
            .child_parser_classic_script_job_for_owner(
                child_handle,
                first_window,
                first_document,
                Some(handle(15)),
                "globalThis.__staleParserClassic = true;".to_owned(),
            )
            .is_none(),
        "stale owner token must not build parser classic jobs"
    );
    assert!(
        store
            .child_external_classic_script_job_for_owner(
                child_handle,
                first_window,
                first_document,
                Some(handle(16)),
                url("https://cdn.child.test/stale.js"),
                url("https://cdn.child.test/stale.js"),
                "globalThis.__staleExternalClassic = true;".to_owned(),
            )
            .is_none(),
        "stale owner token must not build external classic jobs"
    );
    let second_realm_id = materialize_test_child_realm(&mut store, child_handle, 78);
    assert_eq!(second_realm_id, FrameRealmId(2));
    assert_ne!(second_realm_id, FrameRealmId(78));
    assert_eq!(
        store
            .realms
            .get(&second_realm_id)
            .and_then(|realm| realm.inspector_execution_context_id),
        Some(78)
    );
    let second_job = store
        .child_source_script_job(
            child_handle,
            FrameScriptJobKind::ProtocolEvaluate,
            "2 + 2".to_owned(),
        )
        .expect("second child owner should build script job");
    assert_eq!(second_job.local_window_id, second_window);
    assert_eq!(second_job.document_id, second_document);
    assert_eq!(
        store.current_realm_id_for_frame_script_job(&second_job),
        Some(second_realm_id)
    );
    assert_eq!(
        store.current_reserved_realm_id_for_document_task_owner(second_task_owner),
        Some(second_realm_id)
    );
    assert_eq!(
        store.current_child_document_task_owner_reserved_realm(child_handle),
        Some((second_task_owner, second_realm_id))
    );
    assert_eq!(
        store.child_document_task_owner_realm_currentness(
            child_handle,
            second_task_owner,
            second_realm_id,
        ),
        FrameDocumentTaskRealmCurrentness::Current {
            owner: second_task_owner,
            realm_id: second_realm_id,
        }
    );
    assert_eq!(
        store.document_task_owner_realm_currentness(second_task_owner, second_realm_id),
        FrameDocumentTaskRealmCurrentness::Current {
            owner: second_task_owner,
            realm_id: second_realm_id,
        }
    );
}

#[test]
fn child_realm_identity_precedes_inspector_projection() {
    let mut store = FrameOwnerStore::default();
    let child_handle = handle(340);
    let owner = commit_test_child_document(
        &mut store,
        child_handle,
        handle(341),
        "preinspector-realm",
        Some("main"),
    );

    let realm_id = store
        .ensure_child_realm(child_handle)
        .expect("prebootstrap should reserve semantic realm identity");
    assert_eq!(
        store.realms.get(&realm_id).map(|realm| realm.lifecycle),
        Some(FrameRealmLifecycleState::Reserved)
    );
    assert_eq!(
        store.current_reserved_realm_id_for_document_task_owner(owner),
        Some(realm_id)
    );
    assert_eq!(
        store.current_materialized_realm_id_for_document_task_owner(owner),
        None,
        "reserved identity must not authorize script execution"
    );
    assert_eq!(
        store.document_task_owner_realm_currentness(owner, realm_id),
        FrameDocumentTaskRealmCurrentness::PendingRealm { owner, realm_id }
    );
    assert_eq!(
        store
            .realms
            .get(&realm_id)
            .and_then(|realm| realm.inspector_execution_context_id),
        None,
        "Inspector attachment is a later projection, not the source of realm identity"
    );

    assert_eq!(
        store.request_child_realm_materialization(child_handle, owner),
        Some(FrameRealmMaterializationRequest::NewlyQueued { realm_id })
    );
    assert_eq!(
        store.request_child_realm_materialization(child_handle, owner),
        Some(FrameRealmMaterializationRequest::AlreadyQueued { realm_id }),
        "reentrant exposure must share the exact Document reservation"
    );
    assert!(store.has_queued_child_realm_materialization());
    assert!(store.rollback_child_realm_materialization_request(child_handle, owner, realm_id));
    assert!(!store.has_queued_child_realm_materialization());
    assert_eq!(
        store.realms.get(&realm_id).map(|realm| realm.lifecycle),
        Some(FrameRealmLifecycleState::Reserved),
        "closed typed route must return the exact realm to its reserved state"
    );

    let projected_realm_id = materialize_test_child_realm(&mut store, child_handle, 91);
    assert_eq!(projected_realm_id, realm_id);
    assert_eq!(
        store
            .realms
            .get(&realm_id)
            .and_then(|realm| realm.inspector_execution_context_id),
        Some(91)
    );
    assert_eq!(
        store.current_materialized_realm_id_for_document_task_owner(owner),
        Some(realm_id)
    );
}

#[test]
fn child_detach_marks_current_records_detached() {
    let mut store = FrameOwnerStore::default();
    let child_handle = handle(20);
    let frame_id = store.ensure_child_frame(child_handle, "frame-2".to_owned(), None);
    let scheduler_lane_id = store
        .frames
        .get(&frame_id)
        .expect("child frame record should exist")
        .scheduler_lane_id;
    assert_eq!(
        store
            .frames
            .get(&frame_id)
            .and_then(|frame| frame.owner_element_handle),
        Some(child_handle)
    );
    let (window_id, document_id) = store
        .commit_child_document(
            child_handle,
            handle(21),
            url("https://detach.test/"),
            url("https://detach.test/"),
            "https://detach.test".to_owned(),
            None,
            RequestCredentialsMode::SameOrigin,
            policy_container(),
            policy_context(),
        )
        .expect("child document should commit");
    let realm_id = materialize_test_child_realm(&mut store, child_handle, 88);

    store.detach_child_frame(child_handle);

    assert_eq!(
        store.frames.get(&frame_id).map(|frame| frame.lifecycle),
        Some(FrameLifecycleState::Detached)
    );
    assert_eq!(
        store
            .scheduler_lanes
            .get(&scheduler_lane_id)
            .map(|lane| lane.lifecycle),
        Some(FrameSchedulerLaneLifecycleState::Detached)
    );
    assert_eq!(
        store
            .local_windows
            .get(&window_id)
            .map(|window| window.lifecycle),
        Some(LocalWindowLifecycleState::DetachedReachable)
    );
    assert_eq!(
        store
            .documents
            .get(&document_id)
            .map(|document| document.lifecycle),
        Some(DocumentLifecycleState::Detached)
    );
    assert_eq!(
        store.realms.get(&realm_id).map(|realm| realm.lifecycle),
        Some(FrameRealmLifecycleState::DetachedReachable)
    );
    assert_eq!(store.frame_id_for_child_handle(child_handle), None);
    let owner_element = store
        .frame_owner_element_for_child_handle(child_handle)
        .expect("detached child owner element record should remain diagnosable");
    assert_eq!(owner_element.owner_handle, child_handle);
    assert_eq!(owner_element.content_frame_id, None);
    assert_eq!(
        owner_element.lifecycle,
        FrameOwnerElementLifecycleState::Detached
    );
    assert!(!store.document_is_current(document_id));
}

#[test]
fn child_owner_element_rebind_detaches_previous_content_frame() {
    let mut store = FrameOwnerStore::default();
    let child_handle = handle(25);
    let first_frame = store.ensure_child_frame(child_handle, "frame-old".to_owned(), None);
    let (_, first_document) = store
        .commit_child_document(
            child_handle,
            handle(26),
            url("https://rebind.test/old"),
            url("https://rebind.test/old"),
            "https://rebind.test".to_owned(),
            None,
            RequestCredentialsMode::SameOrigin,
            policy_container(),
            policy_context(),
        )
        .expect("first child document should commit");

    let second_frame = store.ensure_child_frame(
        child_handle,
        "frame-new".to_owned(),
        Some("main".to_owned()),
    );

    assert_ne!(first_frame, second_frame);
    assert_eq!(
        store.frame_id_for_child_handle(child_handle),
        Some(&second_frame)
    );
    assert_eq!(
        store.frames.get(&first_frame).map(|frame| frame.lifecycle),
        Some(FrameLifecycleState::Detached)
    );
    assert!(!store.document_is_current(first_document));
    let owner_element = store
        .frame_owner_element_for_child_handle(child_handle)
        .expect("rebound owner element record should exist");
    assert_eq!(owner_element.owner_handle, child_handle);
    assert_eq!(owner_element.content_frame_id.as_ref(), Some(&second_frame));
    assert_eq!(
        owner_element.parent_frame_id,
        Some(FrameId("main".to_owned()))
    );
    assert_eq!(
        owner_element.lifecycle,
        FrameOwnerElementLifecycleState::Attached
    );
}

#[test]
fn child_current_owner_snapshot_requires_live_owner_records() {
    let mut store = FrameOwnerStore::default();
    let child_handle = handle(28);
    let frame_id = store.ensure_child_frame(
        child_handle,
        "frame-snapshot".to_owned(),
        Some("main".to_owned()),
    );
    assert!(store.current_child_owner_snapshot(child_handle).is_none());

    let (local_window_id, document_id) = store
        .commit_child_document(
            child_handle,
            handle(29),
            url("https://snapshot.test/doc"),
            url("https://snapshot.test/base/"),
            "https://snapshot.test".to_owned(),
            Some("origin".to_owned()),
            RequestCredentialsMode::Include,
            policy_container(),
            policy_context(),
        )
        .expect("snapshot child document should commit");
    let snapshot = store
        .current_child_owner_snapshot(child_handle)
        .expect("current child owner snapshot should exist after document commit");
    let generic_snapshot = store
        .current_frame_owner_snapshot(&frame_id)
        .expect("generic frame owner snapshot should exist after document commit");
    assert_eq!(generic_snapshot.frame_id, frame_id);
    assert_eq!(generic_snapshot.kind, FrameKind::ChildIframe);
    assert_eq!(
        generic_snapshot.parent_frame_id,
        Some(FrameId("main".to_owned()))
    );
    assert_eq!(generic_snapshot.owner_element_handle, Some(child_handle));
    assert_eq!(generic_snapshot.local_window_id, local_window_id);
    assert_eq!(generic_snapshot.document_id, document_id);
    assert_eq!(generic_snapshot.realm_id, None);
    assert_eq!(snapshot.owner_handle, child_handle);
    assert_eq!(snapshot.frame_id, generic_snapshot.frame_id);
    assert_eq!(snapshot.local_window_id, local_window_id);
    assert_eq!(snapshot.document_id, document_id);
    assert_eq!(snapshot.document_handle, handle(29));
    assert_eq!(snapshot.document_url, url("https://snapshot.test/doc"));
    assert_eq!(
        snapshot.document_base_url,
        url("https://snapshot.test/base/")
    );
    assert_eq!(snapshot.realm_id, None);
    assert_eq!(snapshot.settings.origin, "https://snapshot.test");
    assert_eq!(snapshot.settings.referrer_policy.as_deref(), Some("origin"));
    assert_eq!(
        snapshot.settings.credentials_mode,
        RequestCredentialsMode::Include
    );
    assert_eq!(
        snapshot.settings.document_policy_container,
        policy_container()
    );
    assert_eq!(
        snapshot.settings.subresource_policy_context,
        policy_context()
    );
    assert_eq!(snapshot.settings.service_worker_client_id, None);
    assert!(
        store.set_current_child_service_worker_client_id(
            child_handle,
            Some(service_worker_client(42))
        )
    );
    let snapshot = store
        .current_child_owner_snapshot(child_handle)
        .expect("current child owner snapshot should include service worker projection");
    assert_eq!(
        snapshot.settings.service_worker_client_id,
        Some(service_worker_client(42))
    );
    assert!(store.set_current_child_service_worker_client_id(child_handle, None));
    let snapshot = store
        .current_child_owner_snapshot(child_handle)
        .expect("current child owner snapshot should clear service worker projection");
    assert_eq!(snapshot.settings.service_worker_client_id, None);
    assert_eq!(
        snapshot.settings.module_map_owner,
        ModuleMapOwner::Document(document_id)
    );
    assert!(store.update_current_child_document_urls(
        child_handle,
        url("https://snapshot.test/updated"),
        url("https://snapshot.test/live-base/"),
    ));
    let snapshot = store
        .current_child_owner_snapshot(child_handle)
        .expect("current child owner snapshot should include updated document URLs");
    assert_eq!(snapshot.document_url, url("https://snapshot.test/updated"));
    assert_eq!(
        snapshot.document_base_url,
        url("https://snapshot.test/live-base/")
    );
    assert_eq!(
        snapshot.settings.base_url,
        url("https://snapshot.test/live-base/")
    );

    let realm_id = materialize_test_child_realm(&mut store, child_handle, 99);
    let snapshot = store
        .current_child_owner_snapshot(child_handle)
        .expect("current child owner snapshot should include materialized realm");
    assert_eq!(snapshot.realm_id, Some(realm_id));
    assert_eq!(
        snapshot.settings.module_map_owner,
        ModuleMapOwner::Document(document_id)
    );
    let realm_snapshot = store
        .current_child_owner_snapshot_for_realm(realm_id)
        .expect("materialized child realm should resolve to current owner snapshot");
    assert_eq!(realm_snapshot.owner_handle, child_handle);
    assert_eq!(realm_snapshot.frame_id, frame_id);
    assert_eq!(realm_snapshot.local_window_id, local_window_id);
    assert_eq!(realm_snapshot.document_id, document_id);
    assert_eq!(realm_snapshot.realm_id, Some(realm_id));

    store.clear_child_realm(child_handle);
    assert!(
        store
            .current_child_owner_snapshot_for_realm(realm_id)
            .is_none()
    );
    let realm_id = materialize_test_child_realm(&mut store, child_handle, 100);
    assert!(
        store
            .current_child_owner_snapshot_for_realm(realm_id)
            .is_some()
    );

    store.detach_current_child_document(child_handle);
    assert!(store.current_child_owner_snapshot(child_handle).is_none());
    assert!(
        store
            .current_child_owner_snapshot_for_realm(realm_id)
            .is_none()
    );
}

#[test]
fn stale_realm_clear_does_not_clear_replacement_child_realm() {
    let mut store = FrameOwnerStore::default();
    let child_handle = handle(350);
    commit_test_child_document(
        &mut store,
        child_handle,
        handle(351),
        "stale-realm-clear-initial",
        Some("main"),
    );
    let retired_realm_id = materialize_test_child_realm(&mut store, child_handle, 401);

    let (replacement_window_id, _) = store
        .commit_child_document(
            child_handle,
            handle(352),
            url("https://stale-realm-clear-replacement.test/"),
            url("https://stale-realm-clear-replacement.test/"),
            "https://stale-realm-clear-replacement.test".to_owned(),
            None,
            RequestCredentialsMode::SameOrigin,
            policy_container(),
            policy_context(),
        )
        .expect("replacement child document should commit");
    let replacement_realm_id = materialize_test_child_realm(&mut store, child_handle, 402);

    assert_ne!(retired_realm_id, replacement_realm_id);
    assert!(!store.clear_child_realm_if_matches(child_handle, retired_realm_id));
    assert_eq!(
        store
            .local_windows
            .get(&replacement_window_id)
            .and_then(|window| window.realm_id),
        Some(replacement_realm_id)
    );
    assert_eq!(
        store
            .realms
            .get(&replacement_realm_id)
            .map(|realm| realm.lifecycle),
        Some(FrameRealmLifecycleState::Materialized)
    );
}

#[test]
fn frame_scheduler_lane_is_required_for_current_owner_snapshot() {
    let mut store = FrameOwnerStore::default();
    let child_handle = handle(29);
    let frame_id = store.ensure_child_frame(
        child_handle,
        "frame-scheduler-lane".to_owned(),
        Some("main".to_owned()),
    );
    store
        .commit_child_document(
            child_handle,
            handle(30),
            url("https://scheduler-lane.test/doc"),
            url("https://scheduler-lane.test/doc"),
            "https://scheduler-lane.test".to_owned(),
            None,
            RequestCredentialsMode::SameOrigin,
            policy_container(),
            policy_context(),
        )
        .expect("child document should commit");
    let snapshot = store
        .current_child_owner_snapshot(child_handle)
        .expect("current child owner snapshot should exist");
    let scheduler_lane_id = snapshot.scheduler_lane_id;
    assert_eq!(
        store
            .frames
            .get(&frame_id)
            .map(|frame| frame.scheduler_lane_id),
        Some(scheduler_lane_id)
    );

    store
        .scheduler_lanes
        .get_mut(&scheduler_lane_id)
        .expect("scheduler lane should exist")
        .lifecycle = FrameSchedulerLaneLifecycleState::Detached;
    assert!(
        store.current_child_owner_snapshot(child_handle).is_none(),
        "a detached scheduler lane must invalidate the frame owner snapshot"
    );

    store
        .scheduler_lanes
        .get_mut(&scheduler_lane_id)
        .expect("scheduler lane should exist")
        .lifecycle = FrameSchedulerLaneLifecycleState::Active;
    assert!(
        store.current_child_owner_snapshot(child_handle).is_some(),
        "reactivating the same frame lane should restore currentness"
    );
}

#[test]
fn frame_lane_task_owner_tracks_frame_before_and_after_document_commit() {
    let mut store = FrameOwnerStore::default();
    let child_handle = handle(30);
    let frame_id = store.ensure_child_frame(
        child_handle,
        "frame-lane-task-owner".to_owned(),
        Some("main".to_owned()),
    );
    let lane_owner = store
        .current_child_frame_lane_task_owner(child_handle)
        .expect("attached child frame should have a frame lane task owner before document commit");
    assert_eq!(
        store.current_frame_lane_task_owner(&frame_id),
        Some(lane_owner)
    );
    assert!(store.child_frame_lane_task_owner_is_current(child_handle, lane_owner));
    assert_eq!(store.current_child_document_task_owner(child_handle), None);

    let _ = store
        .commit_child_document(
            child_handle,
            handle(31),
            url("https://frame-lane-task.test/one"),
            url("https://frame-lane-task.test/one"),
            "https://frame-lane-task.test".to_owned(),
            None,
            RequestCredentialsMode::SameOrigin,
            policy_container(),
            policy_context(),
        )
        .expect("child document should commit");
    assert_eq!(
        store.current_child_frame_lane_task_owner(child_handle),
        Some(lane_owner),
        "cross-document work keeps the frame lane owner"
    );

    let _ = store
        .commit_child_document(
            child_handle,
            handle(32),
            url("https://frame-lane-task.test/two"),
            url("https://frame-lane-task.test/two"),
            "https://frame-lane-task.test".to_owned(),
            None,
            RequestCredentialsMode::SameOrigin,
            policy_container(),
            policy_context(),
        )
        .expect("replacement child document should commit");
    assert_eq!(
        store.current_child_frame_lane_task_owner(child_handle),
        Some(lane_owner),
        "replacement document should not replace the frame lane"
    );

    store.detach_child_frame(child_handle);
    assert_eq!(
        store.current_child_frame_lane_task_owner(child_handle),
        None
    );
    assert!(!store.child_frame_lane_task_owner_is_current(child_handle, lane_owner));
}

#[test]
fn child_document_owner_token_tracks_current_child_document() {
    let mut store = FrameOwnerStore::default();
    let child_handle = handle(33);
    store.ensure_child_frame(
        child_handle,
        "frame-document-owner".to_owned(),
        Some("main".to_owned()),
    );
    assert_eq!(store.current_child_document_owner(child_handle), None);
    assert_eq!(store.current_child_document_task_owner(child_handle), None);

    let (first_window, first_document) = store
        .commit_child_document(
            child_handle,
            handle(31),
            url("https://owner-token.test/one"),
            url("https://owner-token.test/one"),
            "https://owner-token.test".to_owned(),
            None,
            RequestCredentialsMode::SameOrigin,
            policy_container(),
            policy_context(),
        )
        .expect("first document should commit");
    let first_owner = FrameDocumentOwner::new(first_window, first_document);
    let first_task_owner = store
        .current_child_document_task_owner(child_handle)
        .expect("first document should have a current task owner");
    assert_eq!(first_task_owner.document_owner(), first_owner);
    assert_eq!(
        store.current_child_document_owner(child_handle),
        Some(first_owner)
    );
    assert!(store.child_document_owner_is_current(child_handle, first_owner));
    assert!(store.child_document_task_owner_is_current(child_handle, first_task_owner));

    let (second_window, second_document) = store
        .commit_child_document(
            child_handle,
            handle(32),
            url("https://owner-token.test/two"),
            url("https://owner-token.test/two"),
            "https://owner-token.test".to_owned(),
            None,
            RequestCredentialsMode::SameOrigin,
            policy_container(),
            policy_context(),
        )
        .expect("replacement document should commit");
    let second_owner = FrameDocumentOwner::new(second_window, second_document);
    let second_task_owner = store
        .current_child_document_task_owner(child_handle)
        .expect("replacement document should have a current task owner");
    assert_ne!(first_owner, second_owner);
    assert_eq!(second_task_owner.document_owner(), second_owner);
    assert_eq!(
        first_task_owner.scheduler_lane_id, second_task_owner.scheduler_lane_id,
        "cross-document navigation keeps the frame scheduler lane"
    );
    assert!(!store.child_document_owner_is_current(child_handle, first_owner));
    assert!(store.child_document_owner_is_current(child_handle, second_owner));
    assert!(!store.child_document_task_owner_is_current(child_handle, first_task_owner));
    assert!(store.child_document_task_owner_is_current(child_handle, second_task_owner));

    store.detach_current_child_document(child_handle);
    assert_eq!(store.current_child_document_owner(child_handle), None);
    assert_eq!(store.current_child_document_task_owner(child_handle), None);
    assert!(!store.child_document_owner_is_current(child_handle, second_owner));
    assert!(!store.child_document_task_owner_is_current(child_handle, second_task_owner));
}

#[test]
fn child_import_map_registry_is_document_scoped_across_static_and_late_resolution() {
    let mut store = FrameOwnerStore::default();
    let child_handle = handle(910);
    let first_document_handle = handle(911);
    let first_owner = commit_test_child_document(
        &mut store,
        child_handle,
        first_document_handle,
        "import-map-frame",
        None,
    );
    let first_realm = materialize_test_child_realm(&mut store, child_handle, 9101);
    let first_base = url("https://import-map-frame.test/frame/");

    store
        .register_current_child_document_import_map(
            child_handle,
            first_document_handle,
            r#"{"imports":{"fixture":"/first.mjs"},"integrity":{"/first.mjs":"sha384-first"}}"#,
            &first_base,
        )
        .expect("current child Document should accept its parser import map");
    let first_url = store
        .resolve_frame_document_module_specifier(
            first_owner.document_owner(),
            first_realm,
            "fixture",
            &first_base,
        )
        .expect("static child module resolution should use the Document import map");
    assert_eq!(first_url, url("https://import-map-frame.test/first.mjs"));
    assert_eq!(
        store.resolve_frame_document_module_integrity(
            first_owner.document_owner(),
            first_realm,
            &first_url,
        ),
        Some("sha384-first".to_owned())
    );
    assert_eq!(
        store
            .resolve_frame_document_module_specifier(
                first_owner.document_owner(),
                first_realm,
                "fixture",
                &url("https://import-map-frame.test/later.mjs"),
            )
            .expect("later dynamic import should share the same Document registry"),
        first_url
    );

    let second_document_handle = handle(912);
    let second_owner = commit_test_child_document(
        &mut store,
        child_handle,
        second_document_handle,
        "import-map-frame",
        None,
    );
    let second_realm = materialize_test_child_realm(&mut store, child_handle, 9102);
    assert_ne!(first_owner.document_owner(), second_owner.document_owner());
    assert!(
        store
            .resolve_frame_document_module_specifier(
                first_owner.document_owner(),
                first_realm,
                "fixture",
                &first_base,
            )
            .is_err(),
        "retired child Document settings must not resolve modules"
    );
    assert!(
        store
            .resolve_frame_document_module_specifier(
                second_owner.document_owner(),
                second_realm,
                "fixture",
                &first_base,
            )
            .is_err(),
        "replacement child Document must start with a fresh import map registry"
    );
    store
        .register_current_child_document_import_map(
            child_handle,
            second_document_handle,
            r#"{"imports":{"fixture":"/second.mjs"}}"#,
            &first_base,
        )
        .expect("replacement child Document should accept its own import map");
    assert_eq!(
        store
            .resolve_frame_document_module_specifier(
                second_owner.document_owner(),
                second_realm,
                "fixture",
                &first_base,
            )
            .expect("replacement child Document should use only its own registry"),
        url("https://import-map-frame.test/second.mjs")
    );
}

#[test]
fn replace_child_document_returns_one_old_to_new_owner_transition() {
    let mut store = FrameOwnerStore::default();
    let child_handle = handle(330);
    store.ensure_child_frame(
        child_handle,
        "frame-document-transition".to_owned(),
        Some("main".to_owned()),
    );
    let _ = store
        .commit_child_document(
            child_handle,
            handle(331),
            url("https://owner-transition.test/one"),
            url("https://owner-transition.test/one"),
            "https://owner-transition.test".to_owned(),
            None,
            RequestCredentialsMode::SameOrigin,
            policy_container(),
            policy_context(),
        )
        .expect("initial child document should commit");
    let first_owner = store
        .current_child_document_task_owner(child_handle)
        .expect("initial document should expose a task owner");

    let transition = store
        .replace_child_document(
            child_handle,
            handle(332),
            url("https://owner-transition.test/two"),
            url("https://owner-transition.test/two"),
            "https://owner-transition.test".to_owned(),
            None,
            RequestCredentialsMode::SameOrigin,
            policy_container(),
            policy_context(),
        )
        .expect("replacement child document should commit");
    let second_owner = transition
        .current_owner()
        .expect("replacement transition should install a current owner");

    assert_eq!(transition.child_handle(), child_handle);
    assert_eq!(transition.retired_owner(), Some(first_owner));
    assert_ne!(first_owner, second_owner);
    assert_eq!(
        first_owner.scheduler_lane_id,
        second_owner.scheduler_lane_id
    );
    assert_eq!(
        store
            .documents
            .get(&first_owner.document_id)
            .map(|document| document.lifecycle),
        Some(DocumentLifecycleState::Replaced),
        "cross-document commit must replace rather than detach the old document"
    );
    assert_eq!(
        store.current_child_document_task_owner(child_handle),
        Some(second_owner)
    );
}

#[test]
fn initial_empty_same_origin_commit_reuses_local_window_exactly_once() {
    let mut store = FrameOwnerStore::default();
    let child_handle = handle(333);
    store.ensure_child_frame(
        child_handle,
        "initial-empty-reuse".to_owned(),
        Some("main".to_owned()),
    );
    let initial_transition = store
        .initialize_child_frame_document(
            child_handle,
            handle(334),
            url("about:blank"),
            url("https://initial-empty-reuse.test/"),
            "https://initial-empty-reuse.test".to_owned(),
            None,
            RequestCredentialsMode::SameOrigin,
            policy_container(),
            policy_context(),
        )
        .expect("initial empty test document should commit");
    let first_owner = store
        .current_child_document_task_owner(child_handle)
        .expect("initial empty test document should expose an owner");
    assert_eq!(initial_transition.retired_owner(), None);
    assert_eq!(initial_transition.current_owner(), Some(first_owner));
    assert_eq!(
        initial_transition.local_window_owner_transition(),
        FrameLocalWindowOwnerTransition::Installed {
            current: first_owner.local_window_id,
        }
    );
    let document_count = store.documents.len();
    assert_eq!(
        store.initialize_child_frame_document(
            child_handle,
            handle(340),
            url("about:blank"),
            url("https://initial-empty-reuse.test/"),
            "https://initial-empty-reuse.test".to_owned(),
            None,
            RequestCredentialsMode::SameOrigin,
            policy_container(),
            policy_context(),
        ),
        None,
        "frame initialization must not replace an already installed document owner"
    );
    assert_eq!(store.documents.len(), document_count);
    assert_eq!(
        store.current_child_document_task_owner(child_handle),
        Some(first_owner)
    );
    let initial_navigation = store
        .ensure_current_child_navigation_load(child_handle)
        .expect("initial empty document should own the accepted navigation");
    assert_eq!(initial_navigation.owner(), first_owner);
    assert_eq!(
        store.child_document_local_window_transition_for_commit(
            child_handle,
            Some(first_owner),
            false,
            &policy_container(),
        ),
        FrameDocumentLocalWindowTransition::ReplaceLocalWindow,
        "an initial empty LocalWindow cannot securely transition across origins"
    );
    assert_eq!(
        store.child_document_local_window_transition_for_commit(
            child_handle,
            Some(first_owner),
            false,
            &policy_container(),
        ),
        FrameDocumentLocalWindowTransition::ReplaceLocalWindow,
        "document.domain relaxation must prevent initial empty LocalWindow reuse"
    );
    let mut credentialless_policy = policy_container();
    credentialless_policy.credentialless = true;
    assert_eq!(
        store.child_document_local_window_transition_for_commit(
            child_handle,
            Some(first_owner),
            true,
            &credentialless_policy,
        ),
        FrameDocumentLocalWindowTransition::ReplaceLocalWindow,
        "credentialless state is LocalWindow-owned and must match across reuse"
    );
    let mut opaque_policy = policy_container();
    opaque_policy.sandbox.forces_opaque_origin = true;
    assert_eq!(
        store.child_document_local_window_transition_for_commit(
            child_handle,
            Some(first_owner),
            true,
            &opaque_policy,
        ),
        FrameDocumentLocalWindowTransition::ReplaceLocalWindow,
        "sandbox origin state must match across reuse"
    );
    let same_origin_transition = store.child_document_local_window_transition_for_commit(
        child_handle,
        Some(first_owner),
        true,
        &policy_container(),
    );
    assert_eq!(
        same_origin_transition,
        FrameDocumentLocalWindowTransition::ReuseInitialEmptyLocalWindow
    );

    let first_real_transition = store
        .replace_child_document_with_local_window_transition(
            child_handle,
            handle(335),
            url("https://initial-empty-reuse.test/first-real"),
            url("https://initial-empty-reuse.test/first-real"),
            "https://initial-empty-reuse.test".to_owned(),
            None,
            RequestCredentialsMode::SameOrigin,
            policy_container(),
            policy_context(),
            DocumentCreationKind::Srcdoc,
            same_origin_transition,
            Some(first_owner),
        )
        .expect("first same-origin document should securely transition");
    let first_real_owner = first_real_transition
        .current_owner()
        .expect("first real document should have an owner");
    assert_eq!(
        first_real_owner.local_window_id,
        first_owner.local_window_id
    );
    assert_eq!(
        first_real_transition.local_window_owner_transition(),
        FrameLocalWindowOwnerTransition::Preserved {
            current: first_owner.local_window_id,
        },
        "secure initial-empty commit must report the preserved LocalWindow identity"
    );
    assert_ne!(first_real_owner.document_id, first_owner.document_id);
    assert_eq!(
        store.current_child_navigation_load(child_handle),
        None,
        "secure initial-empty commit must consume the retired document's navigation binding"
    );
    assert_eq!(
        store.child_document_local_window_transition_for_commit(
            child_handle,
            Some(first_real_owner),
            true,
            &policy_container(),
        ),
        FrameDocumentLocalWindowTransition::ReplaceLocalWindow,
        "only the initial empty document is eligible for LocalWindow reuse"
    );

    let second_transition = store
        .replace_child_document_with_local_window_transition(
            child_handle,
            handle(336),
            url("https://initial-empty-reuse.test/second"),
            url("https://initial-empty-reuse.test/second"),
            "https://initial-empty-reuse.test".to_owned(),
            None,
            RequestCredentialsMode::SameOrigin,
            policy_container(),
            policy_context(),
            DocumentCreationKind::Navigation,
            FrameDocumentLocalWindowTransition::ReplaceLocalWindow,
            Some(first_real_owner),
        )
        .expect("later navigation should replace the LocalWindow");
    let second_owner = second_transition
        .current_owner()
        .expect("later document should have an owner");
    assert_ne!(
        second_owner.local_window_id,
        first_real_owner.local_window_id
    );
    assert_eq!(
        second_transition.local_window_owner_transition(),
        FrameLocalWindowOwnerTransition::Replaced {
            retired: first_real_owner.local_window_id,
            current: second_owner.local_window_id,
        },
        "a later navigation must report the exact retired and replacement LocalWindows"
    );
}

#[test]
fn suppressed_initial_empty_load_is_terminal_for_child_lifecycle() {
    let mut store = FrameOwnerStore::default();
    let child_handle = handle(354);
    store.ensure_child_frame(
        child_handle,
        "initial-empty-suppressed-load".to_owned(),
        Some("main".to_owned()),
    );
    store
        .initialize_child_frame_document(
            child_handle,
            handle(355),
            url("about:blank"),
            url("https://initial-empty-suppressed-load.test/"),
            "https://initial-empty-suppressed-load.test".to_owned(),
            None,
            RequestCredentialsMode::SameOrigin,
            policy_container(),
            policy_context(),
        )
        .expect("initial empty document should commit");

    let task = store
        .complete_current_child_initial_empty_document(child_handle)
        .expect("initial empty document should expose its ready load delivery");
    assert!(store.current_child_document_load_delivery_is_ready(child_handle, task.owner));
    assert!(store.has_pending_current_child_document_lifecycle());

    assert!(store.suppress_current_child_initial_empty_load_delivery(task));
    assert!(!store.current_child_document_load_delivery_is_ready(child_handle, task.owner));
    assert!(!store.has_pending_current_child_document_lifecycle());
    assert_eq!(store.begin_current_child_document_load_delivery(task), None);
    assert!(
        !store.suppress_current_child_initial_empty_load_delivery(task),
        "suppression must be an exact one-way lifecycle transition"
    );
}

#[test]
fn initial_empty_reuse_validation_failure_does_not_partially_commit() {
    let mut store = FrameOwnerStore::default();
    let child_handle = handle(337);
    store.ensure_child_frame(
        child_handle,
        "initial-empty-atomicity".to_owned(),
        Some("main".to_owned()),
    );
    store
        .initialize_child_frame_document(
            child_handle,
            handle(338),
            url("about:blank"),
            url("https://initial-empty-atomicity.test/"),
            "https://initial-empty-atomicity.test".to_owned(),
            None,
            RequestCredentialsMode::SameOrigin,
            policy_container(),
            policy_context(),
        )
        .expect("initial empty document should commit");
    let owner = store
        .current_child_document_task_owner(child_handle)
        .expect("initial empty document should expose an owner");
    let realm_id = materialize_test_child_realm(&mut store, child_handle, 45);
    let realm = store
        .realms
        .remove(&realm_id)
        .expect("test should remove the materialized realm record");
    let document_count = store.documents.len();

    let transition = store.replace_child_document_with_local_window_transition(
        child_handle,
        handle(339),
        url("https://initial-empty-atomicity.test/next"),
        url("https://initial-empty-atomicity.test/next"),
        "https://initial-empty-atomicity.test".to_owned(),
        None,
        RequestCredentialsMode::SameOrigin,
        policy_container(),
        policy_context(),
        DocumentCreationKind::Navigation,
        FrameDocumentLocalWindowTransition::ReuseInitialEmptyLocalWindow,
        Some(owner),
    );

    assert_eq!(transition, None);
    assert_eq!(store.documents.len(), document_count);
    assert_eq!(
        store
            .documents
            .get(&owner.document_id)
            .map(|document| document.lifecycle),
        Some(DocumentLifecycleState::Current)
    );
    assert_eq!(
        store
            .local_windows
            .get(&owner.local_window_id)
            .map(|local_window| {
                (
                    local_window.lifecycle,
                    local_window.document_id,
                    local_window.realm_id,
                )
            }),
        Some((
            LocalWindowLifecycleState::Current,
            owner.document_id,
            Some(realm_id),
        ))
    );
    let frame_id = store
        .frame_ids_by_child_handle
        .get(&child_handle)
        .expect("child frame id should remain installed");
    assert_eq!(
        store
            .frames
            .get(frame_id)
            .map(|frame| (frame.current_local_window_id, frame.current_document_id)),
        Some((Some(owner.local_window_id), Some(owner.document_id)))
    );

    store.realms.insert(realm_id, realm);
    assert_eq!(
        store.current_child_document_task_owner(child_handle),
        Some(owner),
        "restoring the validation dependency should expose the unchanged owner"
    );
}

#[test]
fn child_document_open_preflight_rejection_keeps_the_current_owner_unchanged() {
    let mut store = FrameOwnerStore::default();
    let child_handle = handle(330);
    let document_handle = handle(331);
    let owner = commit_test_child_document(
        &mut store,
        child_handle,
        document_handle,
        "document-open-preflight",
        Some("main"),
    );
    let document_count = store.documents.len();

    assert!(
        store
            .plan_child_document_open_replacement(
                child_handle,
                handle(332),
                url("https://document-open-preflight.test/replacement"),
                url("https://document-open-preflight.test/replacement"),
            )
            .is_none(),
        "a stale Document handle must fail before the owner transaction starts"
    );

    assert_eq!(
        store.current_child_document_task_owner(child_handle),
        Some(owner)
    );
    assert_eq!(store.documents.len(), document_count);
    assert_eq!(
        store
            .current_child_owner_snapshot(child_handle)
            .map(|snapshot| snapshot.document_handle),
        Some(document_handle),
        "failed preflight must leave the old Document installed"
    );
}

#[test]
fn document_open_reprojects_execution_context_work_to_replacement_document_owner() {
    let mut store = FrameOwnerStore::default();
    let child_handle = handle(333);
    store.ensure_child_frame(
        child_handle,
        "same-window-owner".to_owned(),
        Some("main".to_owned()),
    );
    let mut document_policy = policy_container();
    document_policy.credentialless = true;
    document_policy.credentialless_storage_nonce =
        Some(moli_storage_key::OpaqueOriginNonce::new(17));
    store
        .commit_child_document(
            child_handle,
            handle(334),
            url("https://same-window-owner.test/"),
            url("https://same-window-owner.test/"),
            "https://same-window-owner.test".to_owned(),
            None,
            RequestCredentialsMode::SameOrigin,
            document_policy.clone(),
            policy_context(),
        )
        .expect("credentialless test document should commit");
    let first_owner = store
        .current_child_document_task_owner(child_handle)
        .expect("credentialless test document should expose an owner");
    let realm_id = materialize_test_child_realm(&mut store, child_handle, 44);

    let plan = store
        .plan_child_document_open_replacement(
            child_handle,
            handle(334),
            url("https://same-window-owner.test/replacement"),
            url("https://same-window-owner.test/replacement"),
        )
        .expect("document.open replacement should pass preflight");
    let transition = store.commit_child_document_open_replacement(plan);
    let replacement_owner = transition
        .current_owner()
        .expect("document.open should install a replacement document owner");

    assert_eq!(
        replacement_owner.local_window_id,
        first_owner.local_window_id
    );
    assert_ne!(replacement_owner.document_id, first_owner.document_id);
    assert_eq!(
        store
            .current_child_owner_snapshot(child_handle)
            .expect("replacement document should expose its LocalWindow settings")
            .settings
            .document_policy_container,
        document_policy,
        "document.open must not replace the current LocalWindow policy container"
    );
    assert_eq!(
        store.current_document_task_owner_for_execution_context(
            first_owner.local_window_id,
            realm_id,
        ),
        Some(replacement_owner),
        "execution-context work must reproject to the current document in the same LocalWindow"
    );
    assert_eq!(
        store.current_document_task_owner_for_document_owner(first_owner.document_owner()),
        None,
        "exact-document work must remain stale after document.open"
    );
}

#[test]
fn detach_child_document_reports_one_retirement_transition() {
    let mut store = FrameOwnerStore::default();
    let child_handle = handle(340);
    store.ensure_child_frame(
        child_handle,
        "frame-document-retirement".to_owned(),
        Some("main".to_owned()),
    );
    let _ = store
        .commit_child_document(
            child_handle,
            handle(341),
            url("https://owner-retirement.test/"),
            url("https://owner-retirement.test/"),
            "https://owner-retirement.test".to_owned(),
            None,
            RequestCredentialsMode::SameOrigin,
            policy_container(),
            policy_context(),
        )
        .expect("child document should commit");
    let retired_owner = store
        .current_child_document_task_owner(child_handle)
        .expect("child document should expose a current owner");

    let transition = store
        .detach_current_child_document(child_handle)
        .expect("detach should produce a retirement transition");
    assert_eq!(transition.retired_owner(), Some(retired_owner));
    assert_eq!(transition.current_owner(), None);
    assert_eq!(
        store.take_pending_document_owner_retirements(),
        vec![transition]
    );
    assert!(store.take_pending_document_owner_retirements().is_empty());
    assert!(
        store.detach_current_child_document(child_handle).is_none(),
        "detached owner must not report a second retirement"
    );
}

#[test]
fn child_document_lifecycle_owns_interactive_and_domcontentloaded_transitions() {
    let mut store = FrameOwnerStore::default();
    let child_handle = handle(350);
    store.ensure_child_frame(
        child_handle,
        "frame-document-lifecycle".to_owned(),
        Some("main".to_owned()),
    );
    let _ = store
        .commit_child_document(
            child_handle,
            handle(351),
            url("https://document-lifecycle.test/one"),
            url("https://document-lifecycle.test/one"),
            "https://document-lifecycle.test".to_owned(),
            None,
            RequestCredentialsMode::SameOrigin,
            policy_container(),
            policy_context(),
        )
        .expect("initial child document should commit");
    let first_owner = store
        .current_child_document_task_owner(child_handle)
        .expect("initial child document should expose an owner");
    let first_lifecycle = &store
        .documents
        .get(&first_owner.document_id)
        .expect("initial document record should exist")
        .lifecycle_progress;
    assert_eq!(first_lifecycle.load_delay_token_count(), 2);
    assert_eq!(
        store.current_child_document_has_load_delay_tokens(child_handle, first_owner),
        Some(true)
    );

    let first_action = store
        .finish_current_child_document_parsing(child_handle, first_owner.document_owner())
        .expect("parser EOF should produce an interactive action");
    assert_eq!(first_action.owner(), first_owner);
    assert_eq!(first_action.child_handle(), child_handle);
    let first_lifecycle = &store
        .documents
        .get(&first_owner.document_id)
        .expect("initial document record should remain")
        .lifecycle_progress;
    assert_eq!(first_lifecycle.load_delay_token_count(), 2);
    assert!(first_lifecycle.is_interactive_pending());
    assert!(
        store
            .finish_current_child_document_parsing(child_handle, first_owner.document_owner())
            .is_none(),
        "parser EOF must produce exactly one interactive action"
    );
    assert!(store.apply_current_child_document_interactive_transition(first_action));
    let first_lifecycle = &store
        .documents
        .get(&first_owner.document_id)
        .expect("initial document record should remain")
        .lifecycle_progress;
    assert_eq!(first_lifecycle.load_delay_token_count(), 1);
    assert!(first_lifecycle.is_interactive());
    assert!(
        !store.apply_current_child_document_interactive_transition(first_action),
        "interactive action must be consumed exactly once"
    );
    let first_domcontentloaded_action = store
        .prepare_current_child_document_domcontentloaded_transition(child_handle, first_owner)
        .expect("interactive document should produce one DCL action");
    assert!(
        store
            .documents
            .get(&first_owner.document_id)
            .expect("initial document record should remain")
            .lifecycle_progress
            .is_domcontentloaded_pending()
    );
    assert!(
        store.apply_current_child_document_domcontentloaded_transition(
            first_domcontentloaded_action
        )
    );
    let first_lifecycle = &store
        .documents
        .get(&first_owner.document_id)
        .expect("initial document record should remain")
        .lifecycle_progress;
    assert_eq!(first_lifecycle.load_delay_token_count(), 0);
    assert!(first_lifecycle.is_domcontentloaded());
    assert!(
        !store.apply_current_child_document_domcontentloaded_transition(
            first_domcontentloaded_action
        ),
        "DCL action must be consumed exactly once"
    );

    let first_complete_action = store
        .prepare_current_child_document_complete_transition(child_handle, first_owner)
        .expect("DCL-complete document without delays should produce one complete action");
    assert!(
        store
            .documents
            .get(&first_owner.document_id)
            .expect("initial document record should remain")
            .lifecycle_progress
            .is_complete_pending()
    );
    assert!(store.apply_current_child_document_complete_transition(first_complete_action));
    let first_lifecycle = &store
        .documents
        .get(&first_owner.document_id)
        .expect("initial document record should remain")
        .lifecycle_progress;
    assert!(first_lifecycle.is_complete());
    assert!(first_lifecycle.load_is_ready());
    let first_host_load = crate::frame_owner_model::FrameDocumentLoadDeliveryTask {
        child_handle,
        owner: first_owner,
    };
    let _ = finish_test_child_load_delivery(&mut store, first_host_load);
    assert!(
        store
            .documents
            .get(&first_owner.document_id)
            .expect("initial document record should remain")
            .lifecycle_progress
            .load_was_dispatched()
    );
    assert!(
        store
            .prepare_current_child_document_complete_transition(child_handle, first_owner)
            .is_none(),
        "complete/load transitions must be exactly once per document owner"
    );

    let transition = store
        .replace_child_document(
            child_handle,
            handle(352),
            url("https://document-lifecycle.test/two"),
            url("https://document-lifecycle.test/two"),
            "https://document-lifecycle.test".to_owned(),
            None,
            RequestCredentialsMode::SameOrigin,
            policy_container(),
            policy_context(),
        )
        .expect("replacement child document should commit");
    let second_owner = transition
        .current_owner()
        .expect("replacement should expose the new owner");
    assert_eq!(
        store.current_child_document_has_load_delay_tokens(child_handle, second_owner),
        Some(true),
        "each committed document must receive its own parsing delay"
    );
    let second_action = store
        .finish_current_child_document_parsing(child_handle, second_owner.document_owner())
        .expect("replacement parser EOF should own its interactive action");
    assert!(
        store
            .finish_current_child_document_parsing(child_handle, first_owner.document_owner())
            .is_none(),
        "stale parser completion must not release the replacement token"
    );

    let third_transition = store
        .replace_child_document(
            child_handle,
            handle(353),
            url("https://document-lifecycle.test/three"),
            url("https://document-lifecycle.test/three"),
            "https://document-lifecycle.test".to_owned(),
            None,
            RequestCredentialsMode::SameOrigin,
            policy_container(),
            policy_context(),
        )
        .expect("second replacement child document should commit");
    let third_owner = third_transition
        .current_owner()
        .expect("second replacement should expose the new owner");
    assert!(
        !store.apply_current_child_document_interactive_transition(second_action),
        "retired document action must not mutate the replacement"
    );
    assert_eq!(
        store
            .documents
            .get(&second_owner.document_id)
            .expect("replaced document record should remain diagnosable")
            .lifecycle_progress
            .load_delay_token_count(),
        0,
        "replacement must retire the old interactive transition token"
    );

    store.detach_current_child_document(child_handle);
    assert_eq!(
        store
            .documents
            .get(&third_owner.document_id)
            .expect("detached document record should remain diagnosable")
            .lifecycle_progress
            .load_delay_token_count(),
        0,
        "detach must retire all tokens owned by the old document"
    );
}

#[test]
fn child_host_load_admission_is_unique_and_exact_across_replacement() {
    let mut store = FrameOwnerStore::default();
    let child_handle = handle(422);
    let first_owner = commit_test_child_document(
        &mut store,
        child_handle,
        handle(423),
        "frame-host-load-admission",
        Some("main"),
    );
    let first_task = prepare_test_child_load_delivery(&mut store, child_handle, first_owner);

    let first = store
        .reserve_current_child_document_load_delivery_task(first_task)
        .expect("ready child document should reserve one HostLoad admission");
    assert!(
        store
            .reserve_current_child_document_load_delivery_task(first_task)
            .is_none(),
        "repeated lifecycle reconciliation must not duplicate one HostLoad admission"
    );
    assert!(store.current_child_document_load_delivery_task_is_reserved(first));

    assert!(
        store.retire_current_child_document_load_delivery_task_reservation(child_handle),
        "navigation admission should retire the current HostLoad reservation"
    );
    let second = store
        .reserve_current_child_document_load_delivery_task(first_task)
        .expect("the same ready document may reserve a fresh task after retirement");
    assert_ne!(first.admission_id(), second.admission_id());
    assert!(
        !store.release_current_child_document_load_delivery_task_reservation(first),
        "an old stable task must not clear the newer exact admission token"
    );
    assert!(store.current_child_document_load_delivery_task_is_reserved(second));

    let transition = store
        .replace_child_document(
            child_handle,
            handle(424),
            url("https://frame-host-load-admission.test/replacement"),
            url("https://frame-host-load-admission.test/replacement"),
            "https://frame-host-load-admission.test".to_owned(),
            None,
            RequestCredentialsMode::SameOrigin,
            policy_container(),
            policy_context(),
        )
        .expect("replacement child document should commit");
    let replacement_owner = transition.current_owner().expect("replacement owner");
    let replacement_task =
        prepare_test_child_load_delivery(&mut store, child_handle, replacement_owner);
    let replacement = store
        .reserve_current_child_document_load_delivery_task(replacement_task)
        .expect("replacement document should own an independent HostLoad admission");

    assert!(
        !store.release_current_child_document_load_delivery_task_reservation(second),
        "retired Document work must not clear the replacement Document reservation"
    );
    assert!(store.current_child_document_load_delivery_task_is_reserved(replacement));
    assert!(
        store.release_current_child_document_load_delivery_task_reservation(replacement),
        "the exact replacement admission should remain claimable"
    );
}

#[test]
fn child_load_delivery_abort_retries_only_the_current_phase() {
    let mut store = FrameOwnerStore::default();
    let child_handle = handle(346);
    let owner = commit_test_child_document(
        &mut store,
        child_handle,
        handle(347),
        "frame-load-phases",
        Some("main"),
    );
    let task = prepare_test_child_load_delivery(&mut store, child_handle, owner);

    let window = store
        .begin_current_child_document_load_delivery(task)
        .expect("ready child load should begin with Window load");
    assert_eq!(window.phase(), FrameDocumentLoadDeliveryPhase::WindowLoad);
    assert_eq!(
        store.finish_current_child_document_load_delivery(window),
        Some(FrameDocumentLoadDeliveryProgress::Continue(task))
    );

    let owner_element = store
        .begin_current_child_document_load_delivery(task)
        .expect("Window completion should expose owner-element load");
    assert_eq!(
        owner_element.phase(),
        FrameDocumentLoadDeliveryPhase::OwnerElementLoad
    );
    assert!(store.abort_child_document_load_delivery(owner_element));
    assert!(store.current_child_document_load_delivery_is_ready(child_handle, owner));

    let owner_element_retry = store
        .begin_current_child_document_load_delivery(task)
        .expect("aborted owner-element load should remain retryable");
    assert_eq!(
        owner_element_retry.phase(),
        FrameDocumentLoadDeliveryPhase::OwnerElementLoad,
        "retry must not dispatch Window load twice"
    );
    assert_eq!(
        store.finish_current_child_document_load_delivery(owner_element_retry),
        Some(FrameDocumentLoadDeliveryProgress::Continue(task))
    );

    let pageshow = store
        .begin_current_child_document_load_delivery(task)
        .expect("owner-element completion should expose pageshow");
    assert_eq!(pageshow.phase(), FrameDocumentLoadDeliveryPhase::PageShow);
    assert_eq!(
        store.finish_current_child_document_load_delivery(pageshow),
        Some(FrameDocumentLoadDeliveryProgress::Continue(task))
    );

    let frame_finish = store
        .begin_current_child_document_load_delivery(task)
        .expect("pageshow completion should expose frame finish");
    assert_eq!(
        frame_finish.phase(),
        FrameDocumentLoadDeliveryPhase::FrameFinish
    );
    let Some(FrameDocumentLoadDeliveryProgress::Finished(finish)) =
        store.finish_current_child_document_load_delivery(frame_finish)
    else {
        panic!("frame-finish must produce exact FrameLoader/FrameClient output");
    };
    assert_eq!(finish.child_handle, child_handle);
    assert_eq!(finish.owner, owner);
    assert_eq!(finish.frame_id, FrameId("frame-load-phases".to_owned()));
    assert_eq!(finish.parent_frame_id, Some(FrameId("main".to_owned())));
    assert_eq!(finish.document_url, url("https://frame-load-phases.test/"));
    assert!(
        store
            .documents
            .get(&owner.document_id)
            .expect("current child document record")
            .lifecycle_progress
            .load_was_dispatched()
    );
}

#[test]
fn child_document_open_during_owner_load_resumes_with_pageshow() {
    let mut store = FrameOwnerStore::default();
    let child_handle = handle(348);
    let original_owner = commit_test_child_document(
        &mut store,
        child_handle,
        handle(349),
        "document-open-during-load",
        Some("main"),
    );
    let original_task = prepare_test_child_load_delivery(&mut store, child_handle, original_owner);

    let window_load = store
        .begin_current_child_document_load_delivery(original_task)
        .expect("the original child should begin Window load delivery");
    assert_eq!(
        window_load.phase(),
        FrameDocumentLoadDeliveryPhase::WindowLoad
    );
    assert_eq!(
        store.finish_current_child_document_load_delivery(window_load),
        Some(FrameDocumentLoadDeliveryProgress::Continue(original_task))
    );
    let owner_element_load = store
        .begin_current_child_document_load_delivery(original_task)
        .expect("the original child should begin owner-element load delivery");
    assert_eq!(
        owner_element_load.phase(),
        FrameDocumentLoadDeliveryPhase::OwnerElementLoad
    );

    let plan = store
        .plan_child_document_open_replacement(
            child_handle,
            handle(349),
            url("https://document-open-during-load.test/replacement"),
            url("https://document-open-during-load.test/replacement"),
        )
        .expect("document.open replacement should pass preflight");
    let replacement = store.commit_child_document_open_replacement(plan);
    let replacement_owner = replacement
        .current_owner()
        .expect("document.open should install a replacement owner");
    assert!(
        store
            .finish_current_child_document_load_delivery(owner_element_load)
            .is_none(),
        "the replaced document's owner-element load tail must be stale"
    );

    advance_test_child_to_domcontentloaded(&mut store, child_handle, replacement_owner);
    let complete = store
        .prepare_current_child_document_complete_transition(child_handle, replacement_owner)
        .expect("the replacement document should prepare complete");
    assert!(store.apply_current_child_document_complete_transition(complete));

    let replacement_task = FrameDocumentLoadDeliveryTask {
        child_handle,
        owner: replacement_owner,
    };
    let pageshow = store
        .begin_current_child_document_load_delivery(replacement_task)
        .expect("the replacement should resume after the active owner-element load");
    assert_eq!(
        pageshow.phase(),
        FrameDocumentLoadDeliveryPhase::PageShow,
        "document.open must not redispatch the Window or owner-element load"
    );
    assert_eq!(
        store.finish_current_child_document_load_delivery(pageshow),
        Some(FrameDocumentLoadDeliveryProgress::Continue(
            replacement_task
        ))
    );
    let frame_finish = store
        .begin_current_child_document_load_delivery(replacement_task)
        .expect("the replacement should finish the inherited load transaction");
    assert_eq!(
        frame_finish.phase(),
        FrameDocumentLoadDeliveryPhase::FrameFinish
    );
    assert!(matches!(
        store.finish_current_child_document_load_delivery(frame_finish),
        Some(FrameDocumentLoadDeliveryProgress::Finished(_))
    ));
}

#[test]
fn child_document_open_during_pageshow_resumes_after_pageshow() {
    let mut store = FrameOwnerStore::default();
    let child_handle = handle(350);
    let document_handle = handle(351);
    let original_owner = commit_test_child_document(
        &mut store,
        child_handle,
        document_handle,
        "document-open-during-pageshow",
        Some("main"),
    );
    let original_task = prepare_test_child_load_delivery(&mut store, child_handle, original_owner);

    for expected_phase in [
        FrameDocumentLoadDeliveryPhase::WindowLoad,
        FrameDocumentLoadDeliveryPhase::OwnerElementLoad,
    ] {
        let action = store
            .begin_current_child_document_load_delivery(original_task)
            .expect("the original child load should advance to pageshow");
        assert_eq!(action.phase(), expected_phase);
        assert_eq!(
            store.finish_current_child_document_load_delivery(action),
            Some(FrameDocumentLoadDeliveryProgress::Continue(original_task))
        );
    }
    let original_pageshow = store
        .begin_current_child_document_load_delivery(original_task)
        .expect("the original child should begin pageshow delivery");
    assert_eq!(
        original_pageshow.phase(),
        FrameDocumentLoadDeliveryPhase::PageShow
    );

    let plan = store
        .plan_child_document_open_replacement(
            child_handle,
            document_handle,
            url("https://document-open-during-pageshow.test/replacement"),
            url("https://document-open-during-pageshow.test/replacement"),
        )
        .expect("document.open replacement should pass preflight");
    let replacement_owner = store
        .commit_child_document_open_replacement(plan)
        .current_owner()
        .expect("document.open should install a replacement owner");
    assert!(
        store
            .finish_current_child_document_load_delivery(original_pageshow)
            .is_none(),
        "the replaced document's pageshow tail must be stale"
    );

    advance_test_child_to_domcontentloaded(&mut store, child_handle, replacement_owner);
    let complete = store
        .prepare_current_child_document_complete_transition(child_handle, replacement_owner)
        .expect("the replacement document should prepare complete");
    assert!(store.apply_current_child_document_complete_transition(complete));

    let replacement_task = FrameDocumentLoadDeliveryTask {
        child_handle,
        owner: replacement_owner,
    };
    let frame_finish = store
        .begin_current_child_document_load_delivery(replacement_task)
        .expect("the replacement should resume after the active pageshow");
    assert_eq!(
        frame_finish.phase(),
        FrameDocumentLoadDeliveryPhase::FrameFinish,
        "document.open must not redispatch a pageshow that is already on the stack"
    );
    assert!(matches!(
        store.finish_current_child_document_load_delivery(frame_finish),
        Some(FrameDocumentLoadDeliveryProgress::Finished(_))
    ));
}

#[test]
fn child_load_delivery_waits_for_descendant_started_during_window_load() {
    let mut store = FrameOwnerStore::default();
    let parent_handle = handle(373);
    let parent_owner = commit_test_child_document(
        &mut store,
        parent_handle,
        handle(374),
        "frame-load-reentrant-parent",
        Some("main"),
    );
    let parent_task = prepare_test_child_load_delivery(&mut store, parent_handle, parent_owner);
    let parent_window = store
        .begin_current_child_document_load_delivery(parent_task)
        .expect("ready parent child should begin Window load");
    assert_eq!(
        parent_window.phase(),
        FrameDocumentLoadDeliveryPhase::WindowLoad
    );

    let descendant_handle = handle(375);
    let descendant_owner = commit_test_child_document(
        &mut store,
        descendant_handle,
        handle(376),
        "frame-load-reentrant-descendant",
        Some("frame-load-reentrant-parent"),
    );
    assert!(
        store
            .begin_child_frame_parent_document_load(descendant_handle)
            .is_none(),
        "new descendant load should acquire the current parent without releasing an old binding"
    );
    assert_eq!(
        store.finish_current_child_document_load_delivery(parent_window),
        Some(FrameDocumentLoadDeliveryProgress::AwaitingDescendantCompletion(parent_task)),
        "Window load completion must preserve its phase while frame finish waits for the descendant"
    );
    assert!(
        !store.current_child_document_load_delivery_is_ready(parent_handle, parent_owner),
        "complete readiness alone must not make the next phase runnable"
    );
    assert!(
        store
            .begin_current_child_document_load_delivery(parent_task)
            .is_none(),
        "a blocked delivery must not claim owner-element load"
    );

    let descendant_task =
        prepare_test_child_load_delivery(&mut store, descendant_handle, descendant_owner);
    let descendant_finish = finish_test_child_load_delivery(&mut store, descendant_task);
    let parent_completion = descendant_finish
        .parent_descendant_completion
        .expect("descendant FrameFinish should release the exact parent blocker");
    assert_eq!(
        parent_completion.parent,
        FrameDocumentDescendantLoadParent::ChildDocument(parent_handle)
    );
    assert_eq!(parent_completion.parent_owner, parent_owner);
    assert!(store.current_child_document_load_delivery_is_ready(parent_handle, parent_owner));

    let owner_element = store
        .begin_current_child_document_load_delivery(parent_task)
        .expect("descendant completion should resume the next parent phase");
    assert_eq!(
        owner_element.phase(),
        FrameDocumentLoadDeliveryPhase::OwnerElementLoad,
        "resumption must not dispatch Window load twice"
    );
    assert_eq!(
        store.finish_current_child_document_load_delivery(owner_element),
        Some(FrameDocumentLoadDeliveryProgress::Continue(parent_task))
    );
    let pageshow = store
        .begin_current_child_document_load_delivery(parent_task)
        .expect("owner-element load should resume pageshow");
    assert_eq!(pageshow.phase(), FrameDocumentLoadDeliveryPhase::PageShow);
    assert_eq!(
        store.finish_current_child_document_load_delivery(pageshow),
        Some(FrameDocumentLoadDeliveryProgress::Continue(parent_task))
    );
    let frame_finish = store
        .begin_current_child_document_load_delivery(parent_task)
        .expect("pageshow should resume FrameFinish");
    assert_eq!(
        frame_finish.phase(),
        FrameDocumentLoadDeliveryPhase::FrameFinish
    );
    assert!(matches!(
        store.finish_current_child_document_load_delivery(frame_finish),
        Some(FrameDocumentLoadDeliveryProgress::Finished(_))
    ));
}

#[test]
fn child_unload_is_exact_document_owned_and_requires_started_load() {
    let mut store = FrameOwnerStore::default();
    let child_handle = handle(366);
    let first_owner = commit_test_child_document(
        &mut store,
        child_handle,
        handle(367),
        "frame-unload-owner",
        Some("main"),
    );
    let first_task = prepare_test_child_load_delivery(&mut store, child_handle, first_owner);
    assert!(
        store
            .begin_current_child_document_unload(child_handle)
            .is_none(),
        "complete readiness alone must not make unload runnable before Window load starts"
    );

    let first_window = store
        .begin_current_child_document_load_delivery(first_task)
        .expect("first document Window load should start");
    let first_unload = store
        .begin_current_child_document_unload(child_handle)
        .expect("an in-progress Window load should make the exact document unloadable");
    assert_eq!(first_unload.owner(), first_owner);
    assert!(
        store
            .begin_current_child_document_unload(child_handle)
            .is_none(),
        "one document must not claim unload twice"
    );
    assert!(store.finish_current_child_document_unload(first_unload));
    assert!(!store.finish_current_child_document_unload(first_unload));
    assert!(matches!(
        store.finish_current_child_document_load_delivery(first_window),
        Some(FrameDocumentLoadDeliveryProgress::Continue(_))
    ));

    let transition = store
        .replace_child_document(
            child_handle,
            handle(368),
            url("https://frame-unload-owner.test/replacement"),
            url("https://frame-unload-owner.test/replacement"),
            "https://frame-unload-owner.test".to_owned(),
            None,
            RequestCredentialsMode::SameOrigin,
            policy_container(),
            policy_context(),
        )
        .expect("replacement child document should commit");
    let second_owner = transition
        .current_owner()
        .expect("replacement must expose its exact owner");
    let second_task = prepare_test_child_load_delivery(&mut store, child_handle, second_owner);
    let _second_window = store
        .begin_current_child_document_load_delivery(second_task)
        .expect("replacement Window load should start independently");
    let second_unload = store
        .begin_current_child_document_unload(child_handle)
        .expect("replacement document must own a fresh unload lifecycle");
    assert_eq!(second_unload.owner(), second_owner);
    assert!(store.finish_current_child_document_unload(second_unload));
}

#[test]
fn child_navigation_load_is_exact_and_replacement_owned() {
    let mut store = FrameOwnerStore::default();
    let child_handle = handle(354);
    let owner = commit_test_child_document(
        &mut store,
        child_handle,
        handle(355),
        "frame-navigation-delay",
        Some("main"),
    );
    advance_test_child_to_domcontentloaded(&mut store, child_handle, owner);
    let stale_complete = store
        .prepare_current_child_document_complete_transition(child_handle, owner)
        .expect("ready child should initially prepare complete");

    let first = store
        .replace_current_child_navigation_load(child_handle)
        .expect("navigation acceptance should acquire an exact delay");
    assert_eq!(first.owner(), owner);
    assert_eq!(
        store.current_child_navigation_load(child_handle),
        Some(first)
    );
    assert!(
        !store.apply_current_child_document_complete_transition(stale_complete),
        "navigation acceptance must invalidate an unconsumed complete transition"
    );
    assert!(
        store
            .prepare_current_child_document_complete_transition(child_handle, owner)
            .is_none(),
        "navigation delay must block complete without consulting navigation adapter state"
    );
    assert_eq!(
        store.ensure_current_child_navigation_load(child_handle),
        Some(first),
        "repeated delivery wake must retain the same navigation identity"
    );

    let second = store
        .replace_current_child_navigation_load(child_handle)
        .expect("a newer navigation should replace the old delay identity");
    assert_ne!(first, second);
    assert_eq!(
        store.settle_current_child_navigation_load(child_handle, first),
        None,
        "stale terminal must not release the newer navigation"
    );
    assert_eq!(
        store.current_child_navigation_load(child_handle),
        Some(second)
    );
    assert_eq!(
        store.settle_current_child_navigation_load(child_handle, second),
        Some(owner)
    );
    assert_eq!(store.current_child_navigation_load(child_handle), None);
    let complete = store
        .prepare_current_child_document_complete_transition(child_handle, owner)
        .expect("exact terminal should make complete claimable again");
    assert!(store.apply_current_child_document_complete_transition(complete));

    let post_complete = store
        .replace_current_child_navigation_load(child_handle)
        .expect("a complete document must still own an exact navigation identity");
    assert_eq!(
        post_complete.document_load_delay_token(),
        None,
        "navigation after complete must not regress the old document lifecycle"
    );
    assert_eq!(
        store.settle_current_child_navigation_load(child_handle, post_complete),
        Some(owner),
        "a post-complete navigation identity must settle without a document delay token"
    );

    let third = store
        .replace_current_child_navigation_load(child_handle)
        .expect("replacement navigation should acquire a final old-document delay");
    assert_eq!(third.document_load_delay_token(), None);
    let transition = store
        .replace_child_document(
            child_handle,
            handle(356),
            url("https://frame-navigation-delay.test/replacement"),
            url("https://frame-navigation-delay.test/replacement"),
            "https://frame-navigation-delay.test".to_owned(),
            None,
            RequestCredentialsMode::SameOrigin,
            policy_container(),
            policy_context(),
        )
        .expect("navigation commit should replace the document owner");
    let replacement_owner = transition
        .current_owner()
        .expect("replacement owner should exist");
    assert_eq!(
        store.current_child_navigation_load(child_handle),
        None,
        "commit transaction must retire the old navigation delay"
    );
    assert_eq!(
        store.settle_current_child_navigation_load(child_handle, third),
        None,
        "old terminal must not mutate the replacement lifecycle"
    );
    assert_eq!(
        store.current_child_document_has_load_delay_tokens(child_handle, replacement_owner),
        Some(true),
        "replacement must retain its own parsing lifecycle bundle"
    );
}

#[test]
fn child_navigation_commit_reservation_is_single_and_exact_generation_owned() {
    let mut store = FrameOwnerStore::default();
    let child_handle = handle(357);
    commit_test_child_document(
        &mut store,
        child_handle,
        handle(358),
        "frame-navigation-commit-reservation",
        Some("main"),
    );
    let lane_owner = store
        .current_child_frame_lane_task_owner(child_handle)
        .expect("current child frame should expose its scheduler lane");
    let first_load = store
        .replace_current_child_navigation_load(child_handle)
        .expect("first navigation should own an exact load generation");
    let first_task = FrameLaneNavigationCommitTask {
        child_handle,
        owner: lane_owner,
        navigation_load: first_load,
    };

    assert_eq!(
        store.reserve_current_child_navigation_commit_task(first_task),
        FrameNavigationCommitReservationResult::Reserved
    );
    assert_eq!(
        store.reserve_current_child_navigation_commit_task(first_task),
        FrameNavigationCommitReservationResult::AlreadyReserved,
        "duplicate producer observations must reuse the admitted stable task"
    );
    assert_eq!(
        store.current_child_navigation_commit_task(child_handle),
        Some(first_task)
    );

    let replacement_load = store
        .replace_current_child_navigation_load(child_handle)
        .expect("replacement navigation should rotate its load generation");
    let replacement_task = FrameLaneNavigationCommitTask {
        child_handle,
        owner: lane_owner,
        navigation_load: replacement_load,
    };
    assert_eq!(
        store.current_child_navigation_commit_task(child_handle),
        None,
        "replacing the navigation must invalidate the old reservation"
    );
    assert_eq!(
        store.reserve_current_child_navigation_commit_task(replacement_task),
        FrameNavigationCommitReservationResult::Reserved
    );
    assert!(
        !store.retire_child_navigation_commit_task(first_task),
        "an old stable task must not retire the replacement reservation"
    );
    assert_eq!(
        store.current_child_navigation_commit_task(child_handle),
        Some(replacement_task)
    );
    assert!(store.claim_current_child_navigation_commit_task(replacement_task));
    assert_eq!(
        store.current_child_navigation_commit_task(child_handle),
        None
    );
}

#[test]
fn parent_document_lifecycle_owns_exact_incomplete_descendant_frames() {
    let mut store = FrameOwnerStore::default();
    let parent_handle = handle(360);
    let parent_owner = commit_test_child_document(
        &mut store,
        parent_handle,
        handle(361),
        "frame-descendant-parent",
        Some("main"),
    );
    advance_test_child_to_domcontentloaded(&mut store, parent_handle, parent_owner);
    let stale_parent_complete = store
        .prepare_current_child_document_complete_transition(parent_handle, parent_owner)
        .expect("parent without descendants should initially prepare complete");

    let first_child_handle = handle(362);
    let first_child_owner = commit_test_child_document(
        &mut store,
        first_child_handle,
        handle(363),
        "frame-descendant-first",
        Some("frame-descendant-parent"),
    );
    assert_eq!(
        store.current_frame_owner_document_target(first_child_handle),
        Some(FrameOwnerDocumentTarget {
            parent: FrameDocumentDescendantLoadParent::ChildDocument(parent_handle),
            owner: parent_owner,
        }),
        "a nested frame owner must bind the exact parent child-Document owner"
    );
    assert!(
        store
            .begin_child_frame_parent_document_load(first_child_handle)
            .is_none(),
        "first acceptance should not release an earlier parent binding"
    );
    assert!(
        store
            .begin_child_frame_parent_document_load(first_child_handle)
            .is_none(),
        "joined navigation acceptance must reuse the exact descendant binding"
    );
    assert!(
        !store.apply_current_child_document_complete_transition(stale_parent_complete),
        "descendant acceptance must invalidate an unconsumed parent complete action"
    );

    let second_child_handle = handle(364);
    commit_test_child_document(
        &mut store,
        second_child_handle,
        handle(365),
        "frame-descendant-second",
        Some("frame-descendant-parent"),
    );
    assert!(
        store
            .begin_child_frame_parent_document_load(second_child_handle)
            .is_none()
    );
    let parent_lifecycle = &store
        .documents
        .get(&parent_owner.document_id)
        .expect("parent document should remain current")
        .lifecycle_progress;
    assert_eq!(parent_lifecycle.incomplete_child_frame_count(), 2);
    assert!(
        store
            .prepare_current_child_document_complete_transition(parent_handle, parent_owner)
            .is_none(),
        "either incomplete sibling must keep the parent out of complete"
    );

    let first_host_load =
        prepare_test_child_load_delivery(&mut store, first_child_handle, first_child_owner);
    let first_finish = finish_test_child_load_delivery(&mut store, first_host_load);
    let first_parent_completion = first_finish
        .parent_descendant_completion
        .expect("current child frame finish should release its exact parent binding");
    assert_eq!(
        first_parent_completion.parent,
        FrameDocumentDescendantLoadParent::ChildDocument(parent_handle)
    );
    assert_eq!(first_parent_completion.parent_owner, parent_owner);
    assert_eq!(
        first_parent_completion.child_frame_id,
        FrameId("frame-descendant-first".to_owned())
    );
    assert_eq!(
        store
            .documents
            .get(&parent_owner.document_id)
            .expect("parent document should remain current")
            .lifecycle_progress
            .incomplete_child_frame_count(),
        1
    );
    assert!(
        store
            .prepare_current_child_document_complete_transition(parent_handle, parent_owner)
            .is_none(),
        "finishing one sibling must not release the other sibling"
    );

    let second_parent_completion = store
        .detach_child_frame(second_child_handle)
        .expect("detaching an incomplete child must release its exact parent binding");
    assert_eq!(
        second_parent_completion.parent,
        FrameDocumentDescendantLoadParent::ChildDocument(parent_handle)
    );
    assert_eq!(second_parent_completion.parent_owner, parent_owner);
    assert_eq!(
        store
            .documents
            .get(&parent_owner.document_id)
            .expect("parent document should remain current")
            .lifecycle_progress
            .incomplete_child_frame_count(),
        0
    );
    let pending_parent_complete = store
        .prepare_current_child_document_complete_transition(parent_handle, parent_owner)
        .expect("the last exact descendant release should make parent complete claimable");
    let canceled_child_handle = handle(371);
    commit_test_child_document(
        &mut store,
        canceled_child_handle,
        handle(372),
        "frame-descendant-canceled",
        Some("frame-descendant-parent"),
    );
    store.begin_child_frame_parent_document_load(canceled_child_handle);
    assert!(
        !store.apply_current_child_document_complete_transition(pending_parent_complete),
        "a later descendant navigation must invalidate the earlier complete action"
    );
    let canceled = store
        .cancel_child_frame_parent_document_load(canceled_child_handle)
        .expect("navigation cancellation should release its exact descendant binding");
    assert_eq!(canceled.parent_owner, parent_owner);
    assert!(
        store
            .prepare_current_child_document_complete_transition(parent_handle, parent_owner)
            .is_some(),
        "canceling the last descendant navigation should wake parent complete"
    );
}

#[test]
fn direct_child_frame_finish_targets_the_exact_main_document_lifecycle() {
    let mut store = FrameOwnerStore::default();
    store.ensure_main_frame(
        handle(377),
        url("https://main-descendant.test/"),
        url("https://main-descendant.test/"),
        "https://main-descendant.test".to_owned(),
        policy_container(),
        policy_context(),
        None,
    );
    let main_snapshot = store
        .current_main_owner_snapshot()
        .expect("main owner should be installed");
    let main_owner = FrameDocumentTaskOwner::new(
        main_snapshot.scheduler_lane_id,
        main_snapshot.local_window_id,
        main_snapshot.document_id,
    );
    let interactive = store
        .finish_current_main_document_parsing(main_owner)
        .expect("main parser should finish");
    assert!(store.apply_current_main_document_interactive_transition(interactive));
    let domcontentloaded = store
        .prepare_current_main_document_domcontentloaded_transition(main_owner)
        .expect("main document should prepare DOMContentLoaded");
    assert!(store.apply_current_main_document_domcontentloaded_transition(domcontentloaded));

    let child_handle = handle(378);
    let child_owner = commit_test_child_document(
        &mut store,
        child_handle,
        handle(379),
        "frame-main-descendant",
        Some("main"),
    );
    assert_eq!(
        store.current_frame_owner_document_target(child_handle),
        Some(FrameOwnerDocumentTarget {
            parent: FrameDocumentDescendantLoadParent::MainDocument,
            owner: main_owner,
        }),
        "a direct frame owner must bind the exact main Document owner"
    );
    assert!(
        store
            .begin_child_frame_parent_document_load(child_handle)
            .is_none(),
        "first direct-child load should acquire the main lifecycle without releasing an old binding"
    );
    assert_eq!(
        store.current_main_document_complete_transition_is_ready(main_owner),
        Some(false),
        "the exact direct-child binding must block main complete"
    );

    let child_task = prepare_test_child_load_delivery(&mut store, child_handle, child_owner);
    let child_finish = finish_test_child_load_delivery(&mut store, child_task);
    let completion = child_finish
        .parent_descendant_completion
        .expect("direct child FrameFinish should release the exact main binding");
    assert_eq!(
        completion.parent,
        FrameDocumentDescendantLoadParent::MainDocument
    );
    assert_eq!(completion.parent_owner, main_owner);
    assert_eq!(
        store.current_main_document_complete_transition_is_ready(main_owner),
        Some(true),
        "the last direct-child FrameFinish must make main complete claimable"
    );
}

#[test]
fn child_explicit_open_can_reacquire_parent_while_main_load_is_dispatching() {
    let mut store = FrameOwnerStore::default();
    store.ensure_main_frame(
        handle(380),
        url("https://main-explicit-open.test/"),
        url("https://main-explicit-open.test/"),
        "https://main-explicit-open.test".to_owned(),
        policy_container(),
        policy_context(),
        None,
    );
    let main_snapshot = store
        .current_main_owner_snapshot()
        .expect("main owner should be installed");
    let main_owner = FrameDocumentTaskOwner::new(
        main_snapshot.scheduler_lane_id,
        main_snapshot.local_window_id,
        main_snapshot.document_id,
    );
    let interactive = store
        .finish_current_main_document_parsing(main_owner)
        .expect("main parser should finish");
    assert!(store.apply_current_main_document_interactive_transition(interactive));
    let domcontentloaded = store
        .prepare_current_main_document_domcontentloaded_transition(main_owner)
        .expect("main document should prepare DOMContentLoaded");
    assert!(store.apply_current_main_document_domcontentloaded_transition(domcontentloaded));
    let complete = store
        .prepare_current_main_document_complete_transition(main_owner)
        .expect("main document should prepare complete");
    assert!(store.apply_current_main_document_complete_transition(complete));
    assert!(store.begin_current_main_document_load_dispatch(main_owner));

    let child_handle = handle(381);
    commit_test_child_document(
        &mut store,
        child_handle,
        handle(382),
        "frame-main-explicit-open",
        Some("main"),
    );
    assert!(
        store
            .begin_child_frame_parent_document_load(child_handle)
            .is_none()
    );
    assert_eq!(
        store
            .documents
            .get(&main_owner.document_id)
            .expect("main document should remain current")
            .lifecycle_progress
            .incomplete_child_frame_count(),
        1,
        "document.open during the parent load callback must restart child progress"
    );

    assert_eq!(
        store.finish_current_main_document_load_dispatch(main_owner),
        Some(MainDocumentLoadCompletionState::WaitingForDescendants)
    );
    assert!(
        store
            .begin_child_frame_parent_document_load(child_handle)
            .is_none()
    );
    assert_eq!(
        store
            .documents
            .get(&main_owner.document_id)
            .expect("main document should remain current")
            .lifecycle_progress
            .incomplete_child_frame_count(),
        1,
        "document.open after parent load completion must not restart child progress"
    );
    assert!(
        store
            .cancel_child_frame_parent_document_load(child_handle)
            .is_some()
    );
    assert_eq!(
        store.finish_current_main_document_load_after_descendant_completion(main_owner),
        Some(MainDocumentLoadCompletionState::Completed)
    );
    assert_eq!(
        store
            .documents
            .get(&main_owner.document_id)
            .expect("main document should remain current")
            .lifecycle_progress
            .incomplete_child_frame_count(),
        0
    );
    assert!(
        store
            .begin_child_frame_parent_document_load(child_handle)
            .is_none(),
        "a later document.open must not restart progress after main load completion"
    );
}

#[test]
fn stale_descendant_finish_cannot_release_replacement_parent_document() {
    let mut store = FrameOwnerStore::default();
    let parent_handle = handle(366);
    let first_parent_owner = commit_test_child_document(
        &mut store,
        parent_handle,
        handle(367),
        "frame-stale-descendant-parent",
        Some("main"),
    );
    let child_handle = handle(368);
    commit_test_child_document(
        &mut store,
        child_handle,
        handle(369),
        "frame-stale-descendant-child",
        Some("frame-stale-descendant-parent"),
    );
    store.begin_child_frame_parent_document_load(child_handle);
    assert_eq!(
        store
            .documents
            .get(&first_parent_owner.document_id)
            .expect("first parent document should exist")
            .lifecycle_progress
            .incomplete_child_frame_count(),
        1
    );

    let replacement = store
        .replace_child_document(
            parent_handle,
            handle(370),
            url("https://frame-stale-descendant-parent.test/replacement"),
            url("https://frame-stale-descendant-parent.test/replacement"),
            "https://frame-stale-descendant-parent.test".to_owned(),
            None,
            RequestCredentialsMode::SameOrigin,
            policy_container(),
            policy_context(),
        )
        .expect("parent replacement should commit");
    let replacement_owner = replacement
        .current_owner()
        .expect("replacement parent should expose a current owner");
    assert!(
        store.detach_child_frame(child_handle).is_none(),
        "stale child finish must not produce a wake for the replacement parent"
    );
    assert_eq!(
        store
            .documents
            .get(&replacement_owner.document_id)
            .expect("replacement parent document should exist")
            .lifecycle_progress
            .incomplete_child_frame_count(),
        0,
        "an old child binding must never mutate the replacement lifecycle"
    );
}

#[test]
fn child_async_classic_load_delays_are_exact_document_owned_tokens() {
    let mut store = FrameOwnerStore::default();
    let child_handle = handle(354);
    store.ensure_child_frame(
        child_handle,
        "frame-async-classic-load-delay".to_owned(),
        Some("main".to_owned()),
    );
    let _ = store
        .commit_child_document(
            child_handle,
            handle(355),
            url("https://async-classic-delay.test/one"),
            url("https://async-classic-delay.test/one"),
            "https://async-classic-delay.test".to_owned(),
            None,
            RequestCredentialsMode::SameOrigin,
            policy_container(),
            policy_context(),
        )
        .expect("child document should commit");
    let owner = store
        .current_child_document_task_owner(child_handle)
        .expect("child document should expose an owner");
    let first_delay = store
        .acquire_current_child_async_classic_script_load_delay(child_handle, owner)
        .expect("first async classic script should acquire a load delay");
    let second_delay = store
        .acquire_current_child_async_classic_script_load_delay(child_handle, owner)
        .expect("second async classic script should acquire a distinct load delay");
    assert_ne!(first_delay, second_delay);

    let interactive = store
        .finish_current_child_document_parsing(child_handle, owner.document_owner())
        .expect("parser EOF should prepare the interactive transition");
    assert!(store.apply_current_child_document_interactive_transition(interactive));
    let domcontentloaded = store
        .prepare_current_child_document_domcontentloaded_transition(child_handle, owner)
        .expect("interactive document should prepare DCL");
    assert!(store.apply_current_child_document_domcontentloaded_transition(domcontentloaded));
    assert_eq!(
        store.current_child_document_has_load_delay_tokens(child_handle, owner),
        Some(true),
        "async classic scripts must continue delaying load after DCL"
    );

    let first_token = first_delay.token().expect("loading document owns a token");
    let second_token = second_delay.token().expect("loading document owns a token");
    assert!(store.release_async_classic_script_load_delay(owner, first_token));
    assert_eq!(
        store.current_child_document_has_load_delay_tokens(child_handle, owner),
        Some(true),
        "releasing one async classic script must not release its sibling"
    );
    assert!(store.release_async_classic_script_load_delay(owner, second_token));
    assert_eq!(
        store.current_child_document_has_load_delay_tokens(child_handle, owner),
        Some(false),
        "load becomes ready only after the final async classic script settles"
    );
    assert!(
        !store.release_async_classic_script_load_delay(owner, second_token),
        "a consumed load-delay token must not be reusable"
    );

    let retired_delay = store
        .acquire_current_child_async_classic_script_load_delay(child_handle, owner)
        .expect("current document should acquire another async delay");
    let transition = store
        .replace_child_document(
            child_handle,
            handle(356),
            url("https://async-classic-delay.test/two"),
            url("https://async-classic-delay.test/two"),
            "https://async-classic-delay.test".to_owned(),
            None,
            RequestCredentialsMode::SameOrigin,
            policy_container(),
            policy_context(),
        )
        .expect("replacement child document should commit");
    assert_eq!(transition.retired_owner(), Some(owner));
    assert!(
        !store.release_async_classic_script_load_delay(
            owner,
            retired_delay
                .token()
                .expect("pre-replacement document owns a token"),
        ),
        "replacement must retire every delay token owned by the old document"
    );
    let replacement_owner = transition
        .current_owner()
        .expect("replacement should expose the new document owner");
    let _ = store
        .acquire_current_child_async_classic_script_load_delay(child_handle, replacement_owner)
        .expect("replacement should acquire its first async delay");
    let _ = store
        .acquire_current_child_async_classic_script_load_delay(child_handle, replacement_owner)
        .expect("replacement should acquire its second async delay");
    assert_eq!(
        store.release_all_document_script_load_delays(replacement_owner),
        2,
        "document-script cancellation should release every async classic delay"
    );
    assert_eq!(
        store.current_child_document_has_load_delay_tokens(child_handle, replacement_owner),
        Some(true),
        "canceling async scripts must preserve the replacement document's parsing and DCL delays"
    );
}

#[test]
fn child_document_script_delays_own_dcl_and_complete_readiness() {
    let mut store = FrameOwnerStore::default();
    let child_handle = handle(365);
    store.ensure_child_frame(
        child_handle,
        "frame-document-script-load-delay".to_owned(),
        Some("main".to_owned()),
    );
    let _ = store
        .replace_child_document(
            child_handle,
            handle(366),
            url("https://document-script-delay.test/one"),
            url("https://document-script-delay.test/one"),
            "https://document-script-delay.test".to_owned(),
            None,
            RequestCredentialsMode::SameOrigin,
            policy_container(),
            policy_context(),
        )
        .expect("child document should commit");
    let owner = store
        .current_child_document_task_owner(child_handle)
        .expect("child document should expose an owner");
    let parser_deferred = store
        .acquire_current_child_parser_deferred_script_load_delay(child_handle, owner)
        .expect("parser-deferred script should acquire a DCL and load delay");
    let async_module = store
        .acquire_current_child_async_module_script_load_delay(child_handle, owner)
        .expect("async module should acquire a load-only delay");

    let interactive = store
        .finish_current_child_document_parsing(child_handle, owner.document_owner())
        .expect("parser EOF should prepare interactive");
    assert!(store.apply_current_child_document_interactive_transition(interactive));
    assert!(
        store
            .prepare_current_child_document_domcontentloaded_transition(child_handle, owner)
            .is_none(),
        "the parser-deferred token must block DCL without consulting scheduler state"
    );
    assert!(
        !store.release_async_module_script_load_delay(owner, parser_deferred),
        "a delay token may only be released by its owning script class"
    );
    assert!(store.release_parser_deferred_script_load_delay(owner, parser_deferred));

    let domcontentloaded = store
        .prepare_current_child_document_domcontentloaded_transition(child_handle, owner)
        .expect("releasing the final parser-deferred token should unblock DCL");
    assert!(store.apply_current_child_document_domcontentloaded_transition(domcontentloaded));
    assert!(
        store
            .prepare_current_child_document_complete_transition(child_handle, owner)
            .is_none(),
        "the async-module token must delay complete without blocking DCL"
    );
    assert!(store.release_async_module_script_load_delay(owner, async_module));
    let complete = store
        .prepare_current_child_document_complete_transition(child_handle, owner)
        .expect("the final async-module terminal should unblock complete");
    assert!(store.apply_current_child_document_complete_transition(complete));

    let replacement_transition = store
        .replace_child_document(
            child_handle,
            handle(367),
            url("https://document-script-delay.test/two"),
            url("https://document-script-delay.test/two"),
            "https://document-script-delay.test".to_owned(),
            None,
            RequestCredentialsMode::SameOrigin,
            policy_container(),
            policy_context(),
        )
        .expect("replacement child document should commit");
    assert_eq!(replacement_transition.retired_owner(), Some(owner));
    let replacement_owner = replacement_transition
        .current_owner()
        .expect("replacement should expose a new owner");
    let retired_parser_deferred = store
        .acquire_current_child_parser_deferred_script_load_delay(child_handle, replacement_owner)
        .expect("replacement should acquire another parser-deferred delay");
    let retired_async_module = store
        .acquire_current_child_async_module_script_load_delay(child_handle, replacement_owner)
        .expect("replacement should acquire another async-module delay");

    let final_transition = store
        .replace_child_document(
            child_handle,
            handle(368),
            url("https://document-script-delay.test/three"),
            url("https://document-script-delay.test/three"),
            "https://document-script-delay.test".to_owned(),
            None,
            RequestCredentialsMode::SameOrigin,
            policy_container(),
            policy_context(),
        )
        .expect("second replacement child document should commit");
    assert_eq!(final_transition.retired_owner(), Some(replacement_owner));
    assert!(
        !store
            .release_parser_deferred_script_load_delay(replacement_owner, retired_parser_deferred,),
        "replacement must retire parser-deferred delays owned by the old document"
    );
    assert!(
        !store.release_async_module_script_load_delay(replacement_owner, retired_async_module),
        "replacement must retire async-module delays owned by the old document"
    );
    let final_owner = final_transition
        .current_owner()
        .expect("second replacement should expose a new owner");
    let replacement_parser_deferred = store
        .acquire_current_child_parser_deferred_script_load_delay(child_handle, final_owner)
        .expect("replacement parser-deferred script should acquire its own delay");
    let replacement_async_module = store
        .acquire_current_child_async_module_script_load_delay(child_handle, final_owner)
        .expect("replacement async module should acquire its own delay");
    assert_eq!(
        store.release_all_document_script_load_delays(final_owner),
        2,
        "document-script cancellation must release both parser and async module delays"
    );
    assert!(
        !store.release_parser_deferred_script_load_delay(final_owner, replacement_parser_deferred,),
        "bulk cancellation must consume the parser-deferred token exactly once"
    );
    assert!(
        !store.release_async_module_script_load_delay(final_owner, replacement_async_module),
        "bulk cancellation must consume the async-module token exactly once"
    );
    assert_eq!(
        store.current_child_document_has_load_delay_tokens(child_handle, final_owner),
        Some(true),
        "script cancellation must preserve the replacement document's parsing and DCL tokens"
    );
}

#[test]
fn child_blocking_stylesheet_load_delay_is_exact_and_replacement_owned() {
    let mut store = FrameOwnerStore::default();
    let child_handle = handle(362);
    store.ensure_child_frame(
        child_handle,
        "frame-blocking-stylesheet-load-delay".to_owned(),
        Some("main".to_owned()),
    );
    let _ = store
        .commit_child_document(
            child_handle,
            handle(363),
            url("https://stylesheet-delay.test/one"),
            url("https://stylesheet-delay.test/one"),
            "https://stylesheet-delay.test".to_owned(),
            None,
            RequestCredentialsMode::SameOrigin,
            policy_container(),
            policy_context(),
        )
        .expect("child document should commit");
    let owner = store
        .current_child_document_task_owner(child_handle)
        .expect("child document should expose an owner");
    let token = store
        .acquire_current_child_blocking_stylesheet_load_delay(child_handle, owner)
        .expect("accepted stylesheet should acquire a load delay");

    let interactive = store
        .finish_current_child_document_parsing(child_handle, owner.document_owner())
        .expect("parser EOF should prepare the interactive transition");
    assert!(store.apply_current_child_document_interactive_transition(interactive));
    let domcontentloaded = store
        .prepare_current_child_document_domcontentloaded_transition(child_handle, owner)
        .expect("interactive document should prepare DCL");
    assert!(store.apply_current_child_document_domcontentloaded_transition(domcontentloaded));
    assert!(
        store
            .prepare_current_child_document_complete_transition(child_handle, owner)
            .is_none(),
        "stylesheet token, not a stylesheet-store scan, must block complete"
    );

    assert!(store.release_blocking_stylesheet_load_delay(owner, token));
    assert!(
        !store.release_blocking_stylesheet_load_delay(owner, token),
        "stylesheet terminal may consume its exact token only once"
    );
    let complete = store
        .prepare_current_child_document_complete_transition(child_handle, owner)
        .expect("the final stylesheet terminal should unblock complete");
    let replacement_pending_token = store
        .acquire_current_child_blocking_stylesheet_load_delay(child_handle, owner)
        .expect("new stylesheet acceptance should invalidate an unconsumed complete action");
    assert!(
        !store.apply_current_child_document_complete_transition(complete),
        "a later stylesheet acceptance must stale the earlier complete action"
    );

    let transition = store
        .replace_child_document(
            child_handle,
            handle(364),
            url("https://stylesheet-delay.test/two"),
            url("https://stylesheet-delay.test/two"),
            "https://stylesheet-delay.test".to_owned(),
            None,
            RequestCredentialsMode::SameOrigin,
            policy_container(),
            policy_context(),
        )
        .expect("replacement child document should commit");
    assert_eq!(transition.retired_owner(), Some(owner));
    assert!(
        !store.release_blocking_stylesheet_load_delay(owner, replacement_pending_token),
        "replacement must retire the old document's stylesheet token"
    );
    let replacement_owner = transition
        .current_owner()
        .expect("replacement should expose a new document owner");
    assert_eq!(
        store.current_child_document_has_load_delay_tokens(child_handle, replacement_owner),
        Some(true),
        "retiring an old stylesheet token must not release the replacement's lifecycle delays"
    );
}

#[test]
fn child_image_load_event_binding_delays_complete_and_retires_with_document() {
    let mut store = FrameOwnerStore::default();
    let child_handle = handle(365);
    store.ensure_child_frame(
        child_handle,
        "frame-image-load-delay".to_owned(),
        Some("main".to_owned()),
    );
    let _ = store
        .commit_child_document(
            child_handle,
            handle(366),
            url("https://image-delay.test/one"),
            url("https://image-delay.test/one"),
            "https://image-delay.test".to_owned(),
            None,
            RequestCredentialsMode::SameOrigin,
            policy_container(),
            policy_context(),
        )
        .expect("child document should commit");
    let owner = store
        .current_child_document_task_owner(child_handle)
        .expect("child document should expose an owner");
    let image = store
        .accept_current_child_image_load_event(child_handle, owner, handle(368))
        .expect("image acceptance should bind the current document");
    assert_eq!(image.element(), handle(368));
    assert!(
        image.load_delay_token().is_some(),
        "an image accepted before complete must own a load-delay token"
    );

    let interactive = store
        .finish_current_child_document_parsing(child_handle, owner.document_owner())
        .expect("parser EOF should prepare interactive");
    assert!(store.apply_current_child_document_interactive_transition(interactive));
    let domcontentloaded = store
        .prepare_current_child_document_domcontentloaded_transition(child_handle, owner)
        .expect("image load must not block DOMContentLoaded");
    assert!(store.apply_current_child_document_domcontentloaded_transition(domcontentloaded));
    assert!(
        store
            .prepare_current_child_document_complete_transition(child_handle, owner)
            .is_none(),
        "the exact image token must block complete"
    );

    assert!(store.settle_child_image_load_event_binding(image));
    assert!(
        !store.settle_child_image_load_event_binding(image),
        "image terminal must consume its load-delay token exactly once"
    );
    let complete = store
        .prepare_current_child_document_complete_transition(child_handle, owner)
        .expect("image terminal should unblock complete");
    assert!(store.apply_current_child_document_complete_transition(complete));
    let post_complete_image = store
        .accept_current_child_image_load_event(child_handle, owner, handle(369))
        .expect("a post-complete image still needs an exact event owner");
    assert_eq!(
        post_complete_image.load_delay_token(),
        None,
        "post-complete image acceptance must not regress readyState"
    );

    let transition = store
        .replace_child_document(
            child_handle,
            handle(367),
            url("https://image-delay.test/two"),
            url("https://image-delay.test/two"),
            "https://image-delay.test".to_owned(),
            None,
            RequestCredentialsMode::SameOrigin,
            policy_container(),
            policy_context(),
        )
        .expect("replacement child document should commit");
    assert_eq!(transition.retired_owner(), Some(owner));
    assert!(
        !store.child_image_load_event_binding_is_current(post_complete_image),
        "replacement must stale the old image event owner"
    );
    assert!(
        !store.settle_child_image_load_event_binding(post_complete_image),
        "a stale image event must not settle against the replacement document"
    );
}

#[test]
fn child_stylesheet_subresource_delays_release_only_their_document() {
    let mut store = FrameOwnerStore::default();
    let child_handle = handle(394);
    let owner = commit_test_child_document(
        &mut store,
        child_handle,
        handle(395),
        "frame-css-resource-delay",
        Some("main"),
    );
    let image = store
        .accept_current_child_stylesheet_subresource_load_delay(child_handle, owner)
        .expect("child CSS image should bind");
    let font = store
        .accept_current_child_stylesheet_subresource_load_delay(child_handle, owner)
        .expect("child CSS font should bind independently");
    assert_eq!(image.child_handle(), Some(child_handle));
    assert_ne!(image.load_delay_token(), font.load_delay_token());

    advance_test_child_to_domcontentloaded(&mut store, child_handle, owner);
    assert!(
        store
            .prepare_current_child_document_complete_transition(child_handle, owner)
            .is_none()
    );
    assert!(store.settle_stylesheet_subresource_load_delay(image));
    assert!(
        store
            .prepare_current_child_document_complete_transition(child_handle, owner)
            .is_none(),
        "the CSS font token must remain independently load-blocking"
    );

    let transition = store
        .replace_child_document(
            child_handle,
            handle(396),
            url("https://frame-css-resource-delay.test/replacement"),
            url("https://frame-css-resource-delay.test/replacement"),
            "https://frame-css-resource-delay.test".to_owned(),
            None,
            RequestCredentialsMode::SameOrigin,
            policy_container(),
            policy_context(),
        )
        .expect("child replacement");
    assert_eq!(transition.retired_owner(), Some(owner));
    assert!(!store.stylesheet_subresource_load_delay_is_current(font));
    assert!(
        !store.settle_stylesheet_subresource_load_delay(font),
        "stale CSS terminal must not release replacement lifecycle state"
    );
    let replacement = transition.current_owner().expect("replacement owner");
    assert!(
        store
            .prepare_current_child_document_complete_transition(child_handle, replacement)
            .is_none(),
        "replacement starts parsing independently of stale CSS work"
    );
}

#[test]
fn child_media_load_delay_is_element_owned_and_cannot_settle_replacement() {
    let mut store = FrameOwnerStore::default();
    let child_handle = handle(390);
    let media_handle = handle(391);
    store.ensure_child_frame(
        child_handle,
        "frame-media-load-delay".to_owned(),
        Some("main".to_owned()),
    );
    let _ = store
        .commit_child_document(
            child_handle,
            handle(392),
            url("https://media-delay.test/one"),
            url("https://media-delay.test/one"),
            "https://media-delay.test".to_owned(),
            None,
            RequestCredentialsMode::SameOrigin,
            policy_container(),
            policy_context(),
        )
        .expect("child document should commit");
    let owner = store
        .current_child_document_task_owner(child_handle)
        .expect("child document should expose an owner");
    let media = store
        .accept_current_child_media_load_delay(child_handle, owner, media_handle)
        .expect("media acceptance should bind the current document");
    assert_eq!(media.child_handle(), child_handle);
    assert_eq!(media.owner(), owner);
    assert_eq!(media.element(), media_handle);
    assert!(media.load_delay_token().is_some());

    let interactive = store
        .finish_current_child_document_parsing(child_handle, owner.document_owner())
        .expect("parser EOF should prepare interactive");
    assert!(store.apply_current_child_document_interactive_transition(interactive));
    let domcontentloaded = store
        .prepare_current_child_document_domcontentloaded_transition(child_handle, owner)
        .expect("media load must not block DOMContentLoaded");
    assert!(store.apply_current_child_document_domcontentloaded_transition(domcontentloaded));
    assert!(
        store
            .prepare_current_child_document_complete_transition(child_handle, owner)
            .is_none(),
        "the exact media token must block complete"
    );

    assert!(store.settle_child_media_load_delay(media));
    assert!(
        !store.settle_child_media_load_delay(media),
        "loadeddata must consume the media token exactly once"
    );
    let complete = store
        .prepare_current_child_document_complete_transition(child_handle, owner)
        .expect("loadeddata should unblock complete");
    assert!(store.apply_current_child_document_complete_transition(complete));
    let post_complete_media = store
        .accept_current_child_media_load_delay(child_handle, owner, media_handle)
        .expect("post-complete media still needs an exact event owner");
    assert_eq!(post_complete_media.load_delay_token(), None);

    let transition = store
        .replace_child_document(
            child_handle,
            handle(393),
            url("https://media-delay.test/two"),
            url("https://media-delay.test/two"),
            "https://media-delay.test".to_owned(),
            None,
            RequestCredentialsMode::SameOrigin,
            policy_container(),
            policy_context(),
        )
        .expect("replacement child document should commit");
    assert_eq!(transition.retired_owner(), Some(owner));
    assert!(
        !store.child_media_load_delay_is_current(post_complete_media),
        "replacement must stale the old media owner"
    );
    assert!(
        !store.settle_child_media_load_delay(post_complete_media),
        "a stale media event must not settle against the replacement document"
    );
}

#[test]
fn child_modulepreload_event_owner_is_exact_and_never_delays_complete() {
    let mut store = FrameOwnerStore::default();
    let child_handle = handle(368);
    store.ensure_child_frame(
        child_handle,
        "frame-modulepreload-load-delay".to_owned(),
        Some("main".to_owned()),
    );
    let _ = store
        .commit_child_document(
            child_handle,
            handle(369),
            url("https://modulepreload-delay.test/one"),
            url("https://modulepreload-delay.test/one"),
            "https://modulepreload-delay.test".to_owned(),
            None,
            RequestCredentialsMode::SameOrigin,
            policy_container(),
            policy_context(),
        )
        .expect("child document should commit");
    let owner = store
        .current_child_document_task_owner(child_handle)
        .expect("child document should expose an owner");
    let load_delay_before_modulepreload =
        store.current_child_document_has_load_delay_tokens(child_handle, owner);
    let client = store
        .accept_current_child_modulepreload_link(child_handle, owner, handle(370))
        .expect("modulepreload acceptance should bind the current document");
    assert_eq!(client.owner(), owner);
    assert_eq!(client.link_handle(), handle(370));
    assert_eq!(
        store.current_child_document_has_load_delay_tokens(child_handle, owner),
        load_delay_before_modulepreload,
        "modulepreload admission must retain identity without changing the load gate"
    );

    let interactive = store
        .finish_current_child_document_parsing(child_handle, owner.document_owner())
        .expect("parser EOF should prepare interactive");
    assert!(store.apply_current_child_document_interactive_transition(interactive));
    let domcontentloaded = store
        .prepare_current_child_document_domcontentloaded_transition(child_handle, owner)
        .expect("modulepreload must not block DOMContentLoaded");
    assert!(store.apply_current_child_document_domcontentloaded_transition(domcontentloaded));
    let complete = store
        .prepare_current_child_document_complete_transition(child_handle, owner)
        .expect("fetching modulepreload must not block complete");
    assert!(
        store.apply_current_child_document_complete_transition(complete),
        "a queued modulepreload terminal event must not invalidate child complete"
    );
    let post_complete_client = store
        .accept_current_child_modulepreload_link(child_handle, owner, handle(371))
        .expect("a post-complete modulepreload still needs an exact event owner");
    assert_eq!(
        post_complete_client.owner(),
        owner,
        "post-complete modulepreload processing must retain the same exact Document"
    );

    let transition = store
        .replace_child_document(
            child_handle,
            handle(372),
            url("https://modulepreload-delay.test/two"),
            url("https://modulepreload-delay.test/two"),
            "https://modulepreload-delay.test".to_owned(),
            None,
            RequestCredentialsMode::SameOrigin,
            policy_container(),
            policy_context(),
        )
        .expect("replacement child document should commit");
    assert_eq!(transition.retired_owner(), Some(owner));
    assert!(
        !store.child_document_task_owner_is_current(client.child_handle(), client.owner()),
        "replacement must stale the old modulepreload identity without settlement"
    );
    assert!(
        !store.child_document_task_owner_is_current(
            post_complete_client.child_handle(),
            post_complete_client.owner(),
        ),
        "post-complete identity must not target the replacement Document"
    );
}

#[test]
fn child_complete_transition_is_invalidated_by_a_new_load_delay() {
    let mut store = FrameOwnerStore::default();
    let child_handle = handle(357);
    store.ensure_child_frame(
        child_handle,
        "frame-complete-transition-invalidation".to_owned(),
        Some("main".to_owned()),
    );
    let _ = store
        .commit_child_document(
            child_handle,
            handle(358),
            url("https://complete-transition.test/"),
            url("https://complete-transition.test/"),
            "https://complete-transition.test".to_owned(),
            None,
            RequestCredentialsMode::SameOrigin,
            policy_container(),
            policy_context(),
        )
        .expect("child document should commit");
    let owner = store
        .current_child_document_task_owner(child_handle)
        .expect("child document should expose an owner");
    let interactive = store
        .finish_current_child_document_parsing(child_handle, owner.document_owner())
        .expect("parser EOF should prepare interactive");
    assert!(store.apply_current_child_document_interactive_transition(interactive));
    let domcontentloaded = store
        .prepare_current_child_document_domcontentloaded_transition(child_handle, owner)
        .expect("interactive document should prepare DCL");
    assert!(store.apply_current_child_document_domcontentloaded_transition(domcontentloaded));

    let stale_complete = store
        .prepare_current_child_document_complete_transition(child_handle, owner)
        .expect("delay-free DCL document should prepare complete");
    let delay = store
        .acquire_current_child_async_classic_script_load_delay(child_handle, owner)
        .expect("new async work should invalidate pending complete");
    assert!(
        !store.apply_current_child_document_complete_transition(stale_complete),
        "complete action prepared before a new delay must become stale"
    );
    assert!(store.release_async_classic_script_load_delay(
        owner,
        delay.token().expect("DCL document owns a delay token"),
    ));
    let current_complete = store
        .prepare_current_child_document_complete_transition(child_handle, owner)
        .expect("final delay release should permit a new complete action");
    assert_ne!(stale_complete, current_complete);
    assert!(store.apply_current_child_document_complete_transition(current_complete));
    assert_eq!(
        store.acquire_current_child_async_classic_script_load_delay(child_handle, owner),
        Some(ChildDocumentAsyncClassicScriptLoadDelay::AlreadyUnblocked),
        "async classic work remains admissible after load without reopening the load gate"
    );
}

#[test]
fn child_document_requests_track_current_document_owner() {
    let mut store = FrameOwnerStore::default();
    let child_handle = handle(33);
    store.ensure_child_frame(child_handle, "frame-3".to_owned(), Some("main".to_owned()));
    assert_eq!(store.begin_child_document_request(child_handle), None);

    let (_, first_document) = store
        .commit_child_document(
            child_handle,
            handle(31),
            url("https://request.test/one"),
            url("https://request.test/one"),
            "https://request.test".to_owned(),
            None,
            RequestCredentialsMode::SameOrigin,
            policy_container(),
            policy_context(),
        )
        .expect("first child document should commit");
    let (request_document, request_id) = store
        .begin_child_document_request(child_handle)
        .expect("current child document should own navigation request");
    assert_eq!(request_document, first_document);
    assert!(store.document_request_is_current(
        first_document,
        request_id,
        FrameRequestKind::DocumentNavigation
    ));
    assert!(!store.document_request_is_current(
        first_document,
        request_id,
        FrameRequestKind::ClassicScript
    ));

    assert!(store.finish_document_request(first_document, request_id));
    assert!(!store.document_request_is_current(
        first_document,
        request_id,
        FrameRequestKind::DocumentNavigation
    ));
    assert!(!store.finish_document_request(first_document, request_id));

    let (_, replacing_request_id) = store
        .begin_child_document_request(child_handle)
        .expect("replacement navigation request should start on current document");
    let (_, second_document) = store
        .commit_child_document(
            child_handle,
            handle(32),
            url("https://request.test/two"),
            url("https://request.test/two"),
            "https://request.test".to_owned(),
            None,
            RequestCredentialsMode::SameOrigin,
            policy_container(),
            policy_context(),
        )
        .expect("second child document should commit");
    assert_ne!(first_document, second_document);
    assert!(!store.document_request_is_current(
        first_document,
        replacing_request_id,
        FrameRequestKind::DocumentNavigation
    ));
    assert!(
        store
            .documents
            .get(&first_document)
            .is_some_and(|document| document.active_requests.is_empty())
    );
    assert_eq!(
        store.begin_document_request(first_document, FrameRequestKind::DocumentNavigation),
        None
    );

    let (_, second_request_id) = store
        .begin_child_document_request(child_handle)
        .expect("current replacement document should own new request");
    store.detach_child_frame(child_handle);
    assert!(!store.document_request_is_current(
        second_document,
        second_request_id,
        FrameRequestKind::DocumentNavigation
    ));
    assert!(
        store
            .documents
            .get(&second_document)
            .is_some_and(|document| document.active_requests.is_empty())
    );
}

#[test]
fn child_frame_module_requests_track_current_document_owner() {
    let mut store = FrameOwnerStore::default();
    let child_handle = handle(35);
    store.ensure_child_frame(
        child_handle,
        "module-request-frame".to_owned(),
        Some("main".to_owned()),
    );
    assert_eq!(
        store.begin_child_frame_request(child_handle, FrameRequestKind::ModuleRoot),
        None
    );

    let (_, first_document) = store
        .commit_child_document(
            child_handle,
            handle(36),
            url("https://module-request.test/one"),
            url("https://module-request.test/one"),
            "https://module-request.test".to_owned(),
            None,
            RequestCredentialsMode::SameOrigin,
            policy_container(),
            policy_context(),
        )
        .expect("first child document should commit");
    let (root_document, root_request_id) = store
        .begin_child_frame_request(child_handle, FrameRequestKind::ModuleRoot)
        .expect("current child document should own module root request");
    let (dynamic_document, dynamic_request_id) = store
        .begin_child_frame_request(child_handle, FrameRequestKind::DynamicImport)
        .expect("current child document should own dynamic import request");
    assert_eq!(root_document, first_document);
    assert_eq!(dynamic_document, first_document);
    assert_ne!(root_request_id, dynamic_request_id);
    assert!(store.document_request_is_current(
        first_document,
        root_request_id,
        FrameRequestKind::ModuleRoot
    ));
    assert!(!store.document_request_is_current(
        first_document,
        root_request_id,
        FrameRequestKind::DynamicImport
    ));
    assert!(store.document_request_is_current(
        first_document,
        dynamic_request_id,
        FrameRequestKind::DynamicImport
    ));

    let (_, second_document) = store
        .commit_child_document(
            child_handle,
            handle(37),
            url("https://module-request.test/two"),
            url("https://module-request.test/two"),
            "https://module-request.test".to_owned(),
            None,
            RequestCredentialsMode::SameOrigin,
            policy_container(),
            policy_context(),
        )
        .expect("second child document should commit");
    assert_ne!(first_document, second_document);
    assert!(!store.document_request_is_current(
        first_document,
        root_request_id,
        FrameRequestKind::ModuleRoot
    ));
    assert!(!store.document_request_is_current(
        first_document,
        dynamic_request_id,
        FrameRequestKind::DynamicImport
    ));
    assert!(
        store
            .documents
            .get(&first_document)
            .is_some_and(|document| document.active_requests.is_empty())
    );

    let (_, dependency_request_id) = store
        .begin_child_frame_request(child_handle, FrameRequestKind::ModuleDependency)
        .expect("replacement child document should own module dependency request");
    store.detach_current_child_document(child_handle);
    assert!(!store.document_request_is_current(
        second_document,
        dependency_request_id,
        FrameRequestKind::ModuleDependency
    ));
}
