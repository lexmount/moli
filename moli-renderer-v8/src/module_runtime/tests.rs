use std::sync::Arc;

use url::Url;

use crate::dom::native::NativeNodeId;
use moli_module_script_tree as module_tree;

use super::modulator::NativeDocumentModulator as NativeDocumentModulatorUnderTest;
use super::{
    DynamicModuleFetchOwnerAdvance, ImportMapRegistryState, ModuleGraphHandle, ModuleLoadError,
    ModuleLoadStage, ModuleMapEntryState, ModuleMapFetchDisposition, ModuleMapKey,
    ModuleOwnerState, ModuleSource, NativeModuleGraphFetchRequest, NativeModuleGraphJob,
    NativeModuleGraphJobAdvance, NativeModuleMapSingleModuleClient, NativeModuleOwnerEvent,
    NativeModulepreloadLinkClient, PendingDynamicModuleImport,
};

#[test]
fn import_map_registry_state_unifies_registration_and_resolution() {
    let base_url = Url::parse("https://example.test/app/index.html").unwrap();
    let mut registry = ImportMapRegistryState::default();
    registry
        .register_import_map(r#"{"imports":{"fixture":"/mapped.mjs"}}"#, &base_url)
        .expect("initial import map should register");

    let resolved = registry
        .resolve_module_specifier("fixture", &base_url)
        .expect("mapped specifier should resolve");
    assert_eq!(resolved.as_str(), "https://example.test/mapped.mjs");
}

#[test]
fn import_map_registry_state_merges_late_maps_without_remapping_resolved_specifiers() {
    let base_url = Url::parse("https://example.test/app/index.html").unwrap();
    let mut registry = ImportMapRegistryState::default();
    let first = registry
        .resolve_module_specifier("./stable.mjs", &base_url)
        .expect("initial module should resolve");
    registry
        .register_import_map(
            r#"{"imports":{"./stable.mjs":"/changed.mjs","late":"/late.mjs"}}"#,
            &base_url,
        )
        .expect("later import map should merge");
    assert_eq!(
        registry
            .resolve_module_specifier("./stable.mjs", &base_url)
            .expect("resolved specifier should stay stable"),
        first
    );
    assert_eq!(
        registry
            .resolve_module_specifier("late", &base_url)
            .expect("new specifier should use the later map")
            .as_str(),
        "https://example.test/late.mjs"
    );
}

#[test]
fn module_owner_document_replacement_preserves_script_state_module_environment() {
    let mut owner = ModuleOwnerState::default();
    let base_url = Url::parse("https://example.test/app/page.html").unwrap();
    owner
        .register_import_map(r#"{"imports":{"retained":"/retained.mjs"}}"#, &base_url)
        .expect("import map should register");
    let module_key =
        ModuleMapKey::java_script(Url::parse("https://example.test/already-fetched.mjs").unwrap());
    let ModuleMapFetchDisposition::StartedFetch(entry_id) =
        owner.start_or_join_native_module_fetch(module_key.clone())
    else {
        panic!("first module-map lookup should start a fetch");
    };
    owner.insert_native_module_source(
        module_key.clone(),
        ModuleSource::text("export const retained = true;".to_owned()),
    );
    owner.queue_native_dynamic_module_import(pending_dynamic_module_import());

    owner.clear_for_document_replacement();

    assert_eq!(
        owner
            .resolve_module_specifier("retained", &base_url)
            .expect("the same ScriptState should retain its import map")
            .as_str(),
        "https://example.test/retained.mjs"
    );
    assert_eq!(
        owner.start_or_join_native_module_fetch(module_key),
        ModuleMapFetchDisposition::AlreadyFetched(entry_id),
        "document.open must retain the current realm's module map"
    );
    assert!(
        owner.has_ready_native_dynamic_module_import(),
        "document.open must retain a dynamic import queued by the current ScriptState"
    );
}

#[test]
fn module_owner_document_replacement_drops_document_clients_but_keeps_dynamic_joins() {
    let mut owner = ModuleOwnerState::default();
    let url = Url::parse("https://example.test/mixed-document-open.mjs").unwrap();
    let key = ModuleMapKey::java_script(url.clone());
    assert!(matches!(
        owner.start_or_join_native_module_fetch(key.clone()),
        ModuleMapFetchDisposition::StartedFetch(_)
    ));
    let document_client = joined_module_fetch_request(url.clone(), 71);
    let dynamic_client = joined_module_fetch_request_for_requester(
        url,
        72,
        module_tree::ModuleFetchRequester::DynamicImport,
        module_tree::ModuleFetchOrdering::Runtime,
    );
    owner.suspend_native_module_fetch_waiter(
        key.clone(),
        module_map_client_from_request(&document_client),
    );
    owner.suspend_native_module_fetch_waiter(
        key.clone(),
        module_map_client_from_request(&dynamic_client),
    );

    owner.clear_for_document_replacement();
    assert_eq!(
        owner.module_script_client_count_for_testing(),
        0,
        "a replaced Document must not retain parser or script-element clients"
    );

    owner.insert_native_module_source(key, ModuleSource::text("export {};".to_owned()));
    owner.drain_posted_native_module_owner_event_tasks_for_testing();
    let (_, clients, successful) =
        take_module_terminal_notification_for_test(&mut owner).into_parts();
    assert!(successful);
    let (single_module_clients, parser_clients, modulepreload_clients) = clients.into_parts();
    assert!(parser_clients.is_empty());
    assert!(modulepreload_clients.is_empty());
    assert_eq!(single_module_clients.len(), 1);
    assert_eq!(single_module_clients[0].token(), dynamic_client.client);
    assert_eq!(single_module_clients[0].client_name(), "DynamicImport");
}

#[test]
fn native_document_modulator_start_or_join_module_fetch_does_not_reset_existing_entries() {
    let mut store = NativeDocumentModulatorUnderTest::default();
    let url = Url::parse("https://example.test/entry.mjs").unwrap();
    let key = ModuleMapKey::java_script(url);
    let ModuleMapFetchDisposition::StartedFetch(entry_id) =
        store.start_or_join_module_fetch(key.clone())
    else {
        panic!("first module map fetch should start a new entry");
    };
    assert_eq!(
        store.module_entry_state(entry_id),
        ModuleMapEntryState::Fetching
    );
    assert_eq!(
        store.start_or_join_module_fetch(key.clone()),
        ModuleMapFetchDisposition::JoinedFetching(entry_id),
        "second fetch for the same key should join the existing fetching entry"
    );

    store.insert_module_source(
        key.clone(),
        ModuleSource::text("export default 1;".to_owned()),
    );
    assert_eq!(
        store.start_or_join_module_fetch(key.clone()),
        ModuleMapFetchDisposition::AlreadyFetched(entry_id),
        "finished source should be reported as reusable instead of reset to fetching"
    );
    assert_eq!(
        store.module_entry_state(entry_id),
        ModuleMapEntryState::Fetched,
        "start-or-join must not clear the fetched source entry"
    );

    let failed_key =
        ModuleMapKey::java_script(Url::parse("https://example.test/failed.mjs").unwrap());
    let ModuleMapFetchDisposition::StartedFetch(failed_entry) =
        store.start_or_join_module_fetch(failed_key.clone())
    else {
        panic!("first failed-key fetch should start a new entry");
    };
    store.mark_failed(
        failed_key.clone(),
        ModuleLoadError::new(ModuleLoadStage::Fetch, "network error"),
    );
    assert_eq!(
        store.start_or_join_module_fetch(failed_key),
        ModuleMapFetchDisposition::AlreadyFailed(failed_entry),
        "sticky failures should be reported to later clients without refetching"
    );
}

fn joined_module_fetch_request(url: Url, sequence: u64) -> module_tree::ModuleFetchRequest {
    joined_module_fetch_request_for_requester(
        url,
        sequence,
        module_tree::ModuleFetchRequester::ParserPendingScript,
        module_tree::ModuleFetchOrdering::DclCritical,
    )
}

fn joined_module_fetch_request_for_requester(
    url: Url,
    sequence: u64,
    requester: module_tree::ModuleFetchRequester,
    ordering: module_tree::ModuleFetchOrdering,
) -> module_tree::ModuleFetchRequest {
    module_tree::ModuleFetchRequest {
        key: module_tree::ModuleMapKey::javascript(url.clone()),
        tree_id: module_tree::ModuleTreeId(1),
        client: module_tree::SingleModuleClientToken {
            tree_id: module_tree::ModuleTreeId(1),
            sequence,
        },
        specifier: None,
        source_url: url.clone(),
        base_url: url.clone(),
        initiator_url: Url::parse("https://example.test/").unwrap(),
        referrer: module_tree::ModuleReferrer::client(),
        position: module_tree::TextPosition::default(),
        parent: None,
        kind: module_tree::ModuleKind::JavaScript,
        attributes: module_tree::ModuleAttributesKey::empty(),
        phase: module_tree::ModuleImportPhase::Evaluation,
        graph_level: module_tree::ModuleGraphLevel::Dependent,
        fetch_metadata: module_tree::ModuleFetchMetadata::default(),
        render_blocking: module_tree::RenderBlockingBehavior::NonBlocking,
        requester,
        ordering,
        custom_fetch_type: module_tree::ModuleScriptCustomFetchType::None,
    }
}

fn module_map_client_from_request(
    request: &module_tree::ModuleFetchRequest,
) -> NativeModuleMapSingleModuleClient {
    match request.requester {
        module_tree::ModuleFetchRequester::ParserPendingScript
        | module_tree::ModuleFetchRequester::RuntimeModuleScript => {
            NativeModuleMapSingleModuleClient::module_script(request.client, request.phase)
        }
        module_tree::ModuleFetchRequester::DynamicImport => {
            NativeModuleMapSingleModuleClient::dynamic_import(request.client, request.phase)
        }
        module_tree::ModuleFetchRequester::ModulePreload => {
            panic!("modulepreload is not a module map tree client")
        }
    }
}

fn pending_dynamic_module_import() -> PendingDynamicModuleImport {
    let _js_runtime = crate::JsRuntime::initialize();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let resolver = v8::PromiseResolver::new(scope).expect("promise resolver");
    PendingDynamicModuleImport::new(
        v8::Global::new(scope, scope.get_current_context()),
        v8::Global::new(scope, resolver),
        super::DynamicModuleImportOwner::main_for_test(),
        "./dynamic.mjs",
        Url::parse("https://example.test/app/page.html").unwrap(),
        super::ModuleAttributesKey::empty(),
        super::ModuleImportPhase::Evaluation,
    )
}

fn dynamic_fetch_request(url: Url) -> NativeModuleGraphFetchRequest {
    NativeModuleGraphFetchRequest::new_for_test(
        url,
        Url::parse("https://example.test/app/page.html").unwrap(),
        super::ModuleFetchMetadata::default(),
        super::ModuleKind::JavaScript,
    )
}

#[test]
fn native_document_modulator_keeps_module_script_clients_on_module_map_entry() {
    let mut store = NativeDocumentModulatorUnderTest::default();
    let url = Url::parse("https://example.test/parser-module.mjs").unwrap();
    let key = ModuleMapKey::java_script(url.clone());
    assert!(matches!(
        store.start_or_join_module_fetch(key.clone()),
        ModuleMapFetchDisposition::StartedFetch(_)
    ));

    let request = joined_module_fetch_request(url, 11);
    let client = request.client;
    store.suspend_module_fetch_waiter(key.clone(), module_map_client_from_request(&request));
    assert_eq!(
        store.module_script_client_count_for_testing(),
        1,
        "parser module script join should be stored as a module-map entry client"
    );

    store.insert_module_source(
        key.clone(),
        ModuleSource::text("export default 1;".to_owned()),
    );
    assert_eq!(
        store.module_script_client_count_for_testing(),
        0,
        "entry terminal transition should snapshot and clear parser module clients before async notification dispatch"
    );

    let (notification_key, clients, successful) = store
        .take_next_terminal_notification()
        .expect("terminal transition should enqueue a notification")
        .into_parts();
    assert_eq!(notification_key, key);
    assert!(successful);
    let (waiters, _, _) = clients.into_parts();
    assert_eq!(waiters.len(), 1);
    let waiter = waiters.into_iter().next().unwrap();
    assert_eq!(waiter.token(), client);
    assert_eq!(waiter.client_name(), "ModuleScript");
    assert_eq!(waiter.import_phase(), request.phase);
    assert_eq!(
        store.module_script_client_count_for_testing(),
        0,
        "notification drain should not need to clear parser module clients a second time"
    );
}

#[test]
fn native_document_modulator_keeps_runtime_module_script_clients_on_module_map_entry() {
    let mut store = NativeDocumentModulatorUnderTest::default();
    let url = Url::parse("https://example.test/runtime-module.mjs").unwrap();
    let key = ModuleMapKey::java_script(url.clone());
    assert!(matches!(
        store.start_or_join_module_fetch(key.clone()),
        ModuleMapFetchDisposition::StartedFetch(_)
    ));

    let request = joined_module_fetch_request(url, 12);
    let client = request.client;
    store.suspend_module_fetch_waiter(key.clone(), module_map_client_from_request(&request));
    assert_eq!(
        store.module_script_client_count_for_testing(),
        1,
        "runtime module script join should be stored as a module-map entry client"
    );

    store.insert_module_source(
        key.clone(),
        ModuleSource::text("export default 1;".to_owned()),
    );
    assert_eq!(
        store.module_script_client_count_for_testing(),
        0,
        "entry terminal transition should snapshot and clear runtime module clients before async notification dispatch"
    );

    let (notification_key, clients, successful) = store
        .take_next_terminal_notification()
        .expect("terminal transition should enqueue a notification")
        .into_parts();
    assert_eq!(notification_key, key);
    assert!(successful);
    let (waiters, _, _) = clients.into_parts();
    assert_eq!(waiters.len(), 1);
    let waiter = waiters.into_iter().next().unwrap();
    assert_eq!(waiter.token(), client);
    assert_eq!(waiter.client_name(), "ModuleScript");
    assert_eq!(waiter.import_phase(), request.phase);
    assert_eq!(
        store.module_script_client_count_for_testing(),
        0,
        "notification drain should not need to clear runtime module-map clients a second time"
    );
}

#[test]
fn native_document_modulator_detaches_module_script_client_before_terminal_notification() {
    let mut store = NativeDocumentModulatorUnderTest::default();
    let url = Url::parse("https://example.test/detached-module.mjs").unwrap();
    let key = ModuleMapKey::java_script(url.clone());
    assert!(matches!(
        store.start_or_join_module_fetch(key.clone()),
        ModuleMapFetchDisposition::StartedFetch(_)
    ));

    let detached_request = joined_module_fetch_request(url.clone(), 31);
    let detached_client = detached_request.client;
    let retained_request = joined_module_fetch_request(url, 32);
    let retained_client = retained_request.client;
    store.suspend_module_fetch_waiter(
        key.clone(),
        module_map_client_from_request(&detached_request),
    );
    store.suspend_module_fetch_waiter(
        key.clone(),
        module_map_client_from_request(&retained_request),
    );
    assert_eq!(store.module_script_client_count_for_testing(), 2);

    assert!(
        store.detach_single_module_fetch_client(detached_client),
        "detaching a live owner client should remove it from the module map entry"
    );
    assert_eq!(store.module_script_client_count_for_testing(), 1);

    store.insert_module_source(key, ModuleSource::text("export default 1;".to_owned()));
    let (_, clients, successful) = store
        .take_next_terminal_notification()
        .expect("retained client should still receive terminal notification")
        .into_parts();
    assert!(successful);
    let (clients, _, _) = clients.into_parts();
    assert_eq!(clients.len(), 1);
    assert_eq!(clients.into_iter().next().unwrap().token(), retained_client);
}

#[test]
fn native_document_modulator_detaches_module_script_client_after_terminal_snapshot() {
    let mut store = NativeDocumentModulatorUnderTest::default();
    let url = Url::parse("https://example.test/detached-after-terminal.mjs").unwrap();
    let key = ModuleMapKey::java_script(url.clone());
    assert!(matches!(
        store.start_or_join_module_fetch(key.clone()),
        ModuleMapFetchDisposition::StartedFetch(_)
    ));

    let request = joined_module_fetch_request(url, 41);
    let client = request.client;
    store.suspend_module_fetch_waiter(key.clone(), module_map_client_from_request(&request));
    store.insert_module_source(key, ModuleSource::text("export default 1;".to_owned()));

    assert!(
        store.detach_single_module_fetch_client(client),
        "detaching after terminal snapshot should remove the pending notification client"
    );
    assert!(
        store.take_next_terminal_notification().is_none(),
        "empty terminal notification should be dropped after its last client is detached"
    );
}

#[test]
fn native_document_modulator_detaches_runtime_module_script_client_after_terminal_snapshot() {
    let mut store = NativeDocumentModulatorUnderTest::default();
    let url = Url::parse("https://example.test/runtime-detached-after-terminal.mjs").unwrap();
    let key = ModuleMapKey::java_script(url.clone());
    assert!(matches!(
        store.start_or_join_module_fetch(key.clone()),
        ModuleMapFetchDisposition::StartedFetch(_)
    ));

    let request = joined_module_fetch_request_for_requester(
        url,
        43,
        module_tree::ModuleFetchRequester::RuntimeModuleScript,
        module_tree::ModuleFetchOrdering::Runtime,
    );
    let client = request.client;
    store.suspend_module_fetch_waiter(key.clone(), module_map_client_from_request(&request));
    store.insert_module_source(key, ModuleSource::text("export default 1;".to_owned()));

    assert!(
        store.detach_single_module_fetch_client(client),
        "detaching runtime-owned module graph failure should remove its pending notification client"
    );
    assert!(
        store.take_next_terminal_notification().is_none(),
        "runtime-owned module graph failure should not leave an empty terminal notification"
    );
}

#[test]
fn native_document_modulator_keeps_dynamic_import_clients_as_single_module_clients() {
    let mut store = NativeDocumentModulatorUnderTest::default();
    let url = Url::parse("https://example.test/dynamic-module.mjs").unwrap();
    let key = ModuleMapKey::java_script(url.clone());
    assert!(matches!(
        store.start_or_join_module_fetch(key.clone()),
        ModuleMapFetchDisposition::StartedFetch(_)
    ));

    let request = joined_module_fetch_request_for_requester(
        url,
        21,
        module_tree::ModuleFetchRequester::DynamicImport,
        module_tree::ModuleFetchOrdering::Runtime,
    );
    let client = request.client;
    store.suspend_module_fetch_waiter(key.clone(), module_map_client_from_request(&request));
    assert_eq!(
        store.single_module_fetch_client_count_for_testing(),
        1,
        "dynamic import join should use the same single-module client queue as module scripts"
    );
    assert_eq!(
        store.module_script_client_count_for_testing(),
        0,
        "dynamic import clients must not be stored in a module-script-specific waiter list"
    );

    store.insert_module_source(
        key.clone(),
        ModuleSource::text("export default 1;".to_owned()),
    );

    assert_eq!(
        store.single_module_fetch_client_count_for_testing(),
        0,
        "entry terminal transition should snapshot and clear dynamic import single-module clients"
    );
    let (notification_key, clients, successful) = store
        .take_next_terminal_notification()
        .expect("terminal transition should enqueue a notification")
        .into_parts();
    assert_eq!(notification_key, key);
    assert!(successful);
    let (clients, _, _) = clients.into_parts();
    assert_eq!(clients.len(), 1);
    let terminal_client = clients.into_iter().next().unwrap();
    assert_eq!(terminal_client.token(), client);
    assert_eq!(terminal_client.client_name(), "DynamicImport");
    assert_eq!(terminal_client.import_phase(), request.phase);
    assert_eq!(
        store.single_module_fetch_client_count_for_testing(),
        0,
        "taking terminal clients should clear dynamic import single-module clients"
    );
}

#[test]
fn module_owner_clear_dynamic_import_pending_tree_detaches_pending_notification_client() {
    let mut owner = ModuleOwnerState::default();
    let url = Url::parse("https://example.test/dynamic-detach.mjs").unwrap();
    let key = ModuleMapKey::java_script(url.clone());
    assert!(matches!(
        owner.start_or_join_native_module_fetch(key.clone()),
        ModuleMapFetchDisposition::StartedFetch(_)
    ));

    let joined_request = joined_module_fetch_request_for_requester(
        url.clone(),
        51,
        module_tree::ModuleFetchRequester::DynamicImport,
        module_tree::ModuleFetchOrdering::Runtime,
    );
    owner.suspend_native_module_fetch_waiter(
        key.clone(),
        module_map_client_from_request(&joined_request),
    );
    let job = NativeModuleGraphJob::dynamic_import(pending_dynamic_module_import());
    let fetches = owner.suspend_native_dynamic_module_import_fetches(
        vec![dynamic_fetch_request(url)],
        vec![joined_request.client],
        job,
        Vec::new(),
    );
    let inflight = owner
        .take_inflight_native_dynamic_module_import_fetch(fetches[0].load_id())
        .expect("dynamic fetch should expose its resume handle");
    let continuation =
        inflight.finish_with_advance_for_test(NativeModuleGraphJobAdvance::WaitingForFetches);
    let _ = owner.continue_native_dynamic_module_import_fetch(continuation, Vec::new());

    owner.insert_native_module_source(key, ModuleSource::text("export default 1;".to_owned()));
    assert!(
        !owner.has_local_native_module_owner_event_for_testing(),
        "entry terminal transition should post through the owner task source, not synchronously expose a local event"
    );
    owner.drain_posted_native_module_owner_event_tasks_for_testing();
    assert!(
        owner.has_local_native_module_owner_event_for_testing(),
        "owner task source drain should make the posted terminal event locally visible"
    );

    let joined = owner
        .take_joined_native_dynamic_module_import_fetch(joined_request.client)
        .expect("dynamic join should expose its resume handle");
    let _ = owner.clear_failed_native_dynamic_module_import_fetch(joined.into_failure_for_test(
        ModuleLoadError::new(ModuleLoadStage::Fetch, "dynamic import owner cleared"),
    ));

    assert!(
        owner.take_next_native_module_owner_event().is_none(),
        "clearing the dynamic import owner should detach its posted terminal notification client"
    );
}

#[test]
fn module_owner_dynamic_import_resume_decides_waiting_and_ready_internally() {
    let mut owner = ModuleOwnerState::default();
    let job = NativeModuleGraphJob::dynamic_import(pending_dynamic_module_import());
    let first_graph_root = owner.insert_native_module_source(
        ModuleMapKey::java_script(Url::parse("https://example.test/graph-a.mjs").unwrap()),
        ModuleSource::text("export {};".to_owned()),
    );
    let second_graph_root = owner.insert_native_module_source(
        ModuleMapKey::java_script(Url::parse("https://example.test/graph-b.mjs").unwrap()),
        ModuleSource::text("export {};".to_owned()),
    );
    let fetches = owner.suspend_native_dynamic_module_import_fetches(
        vec![
            dynamic_fetch_request(Url::parse("https://example.test/a.mjs").unwrap()),
            dynamic_fetch_request(Url::parse("https://example.test/b.mjs").unwrap()),
        ],
        Vec::new(),
        job,
        Vec::new(),
    );

    let first = owner
        .take_inflight_native_dynamic_module_import_fetch(fetches[0].load_id())
        .expect("first dynamic fetch should expose its resume handle");
    let first_continuation = first.finish_with_advance_for_test(
        NativeModuleGraphJobAdvance::Complete(ModuleGraphHandle {
            root_entry: first_graph_root,
            entries: Vec::new(),
        }),
    );
    let first_advance =
        owner.continue_native_dynamic_module_import_fetch(first_continuation, Vec::new());
    assert!(
        matches!(
            first_advance,
            DynamicModuleFetchOwnerAdvance::RestoredAfterUnexpectedComplete
        ),
        "owner must not expose dynamic import ready while sibling waits remain"
    );

    let second = owner
        .take_inflight_native_dynamic_module_import_fetch(fetches[1].load_id())
        .expect("second dynamic fetch should recover the restored job");
    let second_continuation = second.finish_with_advance_for_test(
        NativeModuleGraphJobAdvance::Complete(ModuleGraphHandle {
            root_entry: second_graph_root,
            entries: Vec::new(),
        }),
    );
    let second_advance =
        owner.continue_native_dynamic_module_import_fetch(second_continuation, Vec::new());
    assert!(
        matches!(second_advance, DynamicModuleFetchOwnerAdvance::Ready(_)),
        "owner should expose the dynamic import advance only after the wait set is empty"
    );
}

#[test]
fn module_owner_posts_terminal_notification_as_owner_event_task() {
    let mut owner = ModuleOwnerState::default();
    let url = Url::parse("https://example.test/owner-event.mjs").unwrap();
    let key = ModuleMapKey::java_script(url.clone());
    assert!(matches!(
        owner.start_or_join_native_module_fetch(key.clone()),
        ModuleMapFetchDisposition::StartedFetch(_)
    ));

    let request = joined_module_fetch_request(url, 61);
    let client = request.client;
    owner.suspend_native_module_fetch_waiter(key.clone(), module_map_client_from_request(&request));

    owner.insert_native_module_source(key.clone(), ModuleSource::text("export {};".to_owned()));
    assert!(
        !owner.has_local_native_module_owner_event_for_testing(),
        "terminal transition should post the owner event through its task source"
    );
    owner.drain_posted_native_module_owner_event_tasks_for_testing();
    assert!(
        owner.has_local_native_module_owner_event_for_testing(),
        "posted terminal owner event should become locally ready only after task-source drain"
    );

    let (notification_key, clients, successful) =
        take_module_terminal_notification_for_test(&mut owner).into_parts();
    assert_eq!(notification_key, key);
    assert!(successful);
    let (script_clients, _, _) = clients.into_parts();
    assert_eq!(script_clients.len(), 1);
    assert_eq!(script_clients[0].token(), client);
    assert!(
        owner.take_next_native_module_owner_event().is_none(),
        "posted owner event should be consumed once"
    );
}

fn take_module_terminal_notification_for_test(
    owner: &mut ModuleOwnerState,
) -> super::ModuleMapTerminalNotification {
    match owner
        .take_next_native_module_owner_event()
        .expect("terminal transition should post a module owner event")
    {
        NativeModuleOwnerEvent::ModuleMapTerminalNotification(notification) => notification,
        NativeModuleOwnerEvent::ModulepreloadLinkError(_) => {
            panic!("expected module map terminal notification owner event")
        }
    }
}

#[test]
fn native_document_modulator_keeps_modulepreload_link_clients_on_module_map_entry() {
    let mut store = NativeDocumentModulatorUnderTest::default();
    let url = Url::parse("https://example.test/preloaded.mjs").unwrap();
    let key = ModuleMapKey::java_script(url);
    let link = NativeNodeId::new(7);
    assert!(matches!(
        store.start_or_join_module_fetch(key.clone()),
        ModuleMapFetchDisposition::StartedFetch(_)
    ));

    let link_client = NativeModulepreloadLinkClient::new(link, key.clone());
    store.suspend_modulepreload_link_client(key.clone(), Arc::clone(&link_client));
    assert_eq!(
        store.modulepreload_link_client_count_for_testing(),
        1,
        "modulepreload link join should be stored as a module-map entry client"
    );

    store.insert_module_source(
        key.clone(),
        ModuleSource::text("export default 1;".to_owned()),
    );
    assert_eq!(
        store.modulepreload_link_client_count_for_testing(),
        0,
        "entry terminal transition should snapshot and clear modulepreload link clients before async notification dispatch"
    );

    let (notification_key, clients, successful) = store
        .take_next_terminal_notification()
        .expect("terminal transition should enqueue a notification")
        .into_parts();
    assert_eq!(notification_key, key);
    assert!(successful);
    let (_, _, clients) = clients.into_parts();
    assert_eq!(clients.len(), 1);
    assert!(NativeModulepreloadLinkClient::ptr_eq(
        &clients[0],
        &link_client
    ));
    assert_eq!(
        store.modulepreload_link_client_count_for_testing(),
        0,
        "taking modulepreload link clients should clear them for that entry"
    );
}

#[test]
fn native_document_modulator_notifies_modulepreload_link_clients_joining_terminal_entries() {
    let mut store = NativeDocumentModulatorUnderTest::default();
    let success_key =
        ModuleMapKey::java_script(Url::parse("https://example.test/already-fetched.mjs").unwrap());
    assert!(matches!(
        store.start_or_join_module_fetch(success_key.clone()),
        ModuleMapFetchDisposition::StartedFetch(_)
    ));
    store.insert_module_source(
        success_key.clone(),
        ModuleSource::text("export default 1;".to_owned()),
    );
    assert!(
        store.take_next_terminal_notification().is_none(),
        "terminal entry without clients should not enqueue a notification"
    );

    let success_link = NativeNodeId::new(17);
    let success_client = NativeModulepreloadLinkClient::new(success_link, success_key.clone());
    store.add_terminal_modulepreload_link_client(success_key.clone(), success_client.clone());
    let (notification_key, clients, successful) = store
        .take_next_terminal_notification()
        .expect("modulepreload client joining fetched entry should be notified")
        .into_parts();
    assert_eq!(notification_key, success_key);
    assert!(successful);
    let (_, _, link_clients) = clients.into_parts();
    assert_eq!(link_clients.len(), 1);
    assert!(NativeModulepreloadLinkClient::ptr_eq(
        &link_clients[0],
        &success_client
    ));

    let failed_key =
        ModuleMapKey::java_script(Url::parse("https://example.test/already-failed.mjs").unwrap());
    assert!(matches!(
        store.start_or_join_module_fetch(failed_key.clone()),
        ModuleMapFetchDisposition::StartedFetch(_)
    ));
    store.mark_failed(
        failed_key.clone(),
        ModuleLoadError::new(ModuleLoadStage::Fetch, "network error"),
    );
    assert!(
        store.take_next_terminal_notification().is_none(),
        "failed terminal entry without clients should not enqueue a notification"
    );

    let failed_link = NativeNodeId::new(18);
    let failed_client = NativeModulepreloadLinkClient::new(failed_link, failed_key.clone());
    store.add_terminal_modulepreload_link_client(failed_key.clone(), failed_client.clone());
    let (notification_key, clients, successful) = store
        .take_next_terminal_notification()
        .expect("modulepreload client joining failed entry should be notified")
        .into_parts();
    assert_eq!(notification_key, failed_key);
    assert!(!successful);
    let (_, _, link_clients) = clients.into_parts();
    assert_eq!(link_clients.len(), 1);
    assert!(NativeModulepreloadLinkClient::ptr_eq(
        &link_clients[0],
        &failed_client
    ));
}

#[test]
fn native_document_modulator_keeps_mixed_fetch_clients_on_one_module_map_entry() {
    let mut store = NativeDocumentModulatorUnderTest::default();
    let url = Url::parse("https://example.test/mixed-client.mjs").unwrap();
    let key = ModuleMapKey::java_script(url.clone());
    assert!(matches!(
        store.start_or_join_module_fetch(key.clone()),
        ModuleMapFetchDisposition::StartedFetch(_)
    ));

    let link = NativeNodeId::new(9);
    let link_client = NativeModulepreloadLinkClient::new(link, key.clone());
    store.suspend_modulepreload_link_client(key.clone(), Arc::clone(&link_client));
    store.suspend_module_fetch_waiter(
        key.clone(),
        module_map_client_from_request(&joined_module_fetch_request(url.clone(), 13)),
    );

    assert_eq!(
        store.fetch_client_count_for_testing(),
        2,
        "module map entry should use one fetch-client queue for all waiter kinds"
    );
    assert_eq!(store.modulepreload_link_client_count_for_testing(), 1);
    assert_eq!(store.single_module_fetch_client_count_for_testing(), 1);

    store.insert_module_source(
        key.clone(),
        ModuleSource::text("export default 1;".to_owned()),
    );
    assert_eq!(
        store.fetch_client_count_for_testing(),
        0,
        "entry terminal transition should snapshot and clear every client kind"
    );

    let (notification_key, clients, successful) = store
        .take_next_terminal_notification()
        .expect("terminal transition should enqueue a notification")
        .into_parts();
    assert_eq!(notification_key, key);
    assert!(successful);
    let (script_clients, _, link_clients) = clients.into_parts();
    assert_eq!(link_clients.len(), 1);
    assert!(NativeModulepreloadLinkClient::ptr_eq(
        &link_clients[0],
        &link_client
    ));
    assert_eq!(script_clients.len(), 1);
    assert_eq!(
        store.fetch_client_count_for_testing(),
        0,
        "terminal drain should clear every client kind for that entry"
    );
    assert_eq!(store.modulepreload_link_client_count_for_testing(), 0);
    assert_eq!(store.single_module_fetch_client_count_for_testing(), 0);
    assert_eq!(store.fetch_client_count_for_testing(), 0);
}

#[test]
fn native_document_modulator_terminal_notification_snapshots_clients_before_later_joins() {
    let mut store = NativeDocumentModulatorUnderTest::default();
    let url = Url::parse("https://example.test/snapshot-client.mjs").unwrap();
    let key = ModuleMapKey::java_script(url.clone());
    assert!(matches!(
        store.start_or_join_module_fetch(key.clone()),
        ModuleMapFetchDisposition::StartedFetch(_)
    ));

    let first = joined_module_fetch_request(url.clone(), 31);
    let first_client = first.client;
    store.suspend_module_fetch_waiter(key.clone(), module_map_client_from_request(&first));
    store.insert_module_source(
        key.clone(),
        ModuleSource::text("export default 1;".to_owned()),
    );

    let later = joined_module_fetch_request_for_requester(
        url,
        32,
        module_tree::ModuleFetchRequester::DynamicImport,
        module_tree::ModuleFetchOrdering::Runtime,
    );
    let later_client = later.client;
    store.suspend_module_fetch_waiter(key.clone(), module_map_client_from_request(&later));

    let (_, clients, successful) = store
        .take_next_terminal_notification()
        .expect("first terminal transition should keep a pending notification")
        .into_parts();
    assert!(successful);
    let (script_clients, _, _) = clients.into_parts();
    assert_eq!(script_clients.len(), 1);
    let first_notification_client = script_clients.into_iter().next().unwrap();
    assert_eq!(first_notification_client.token(), first_client);
    assert_eq!(
        first_notification_client.client_name(),
        "ModuleScript",
        "terminal notification must contain the clients present at the terminal transition"
    );
    assert_eq!(
        store.single_module_fetch_client_count_for_testing(),
        1,
        "a later client must not be drained by the older terminal notification"
    );

    store.mark_failed(
        key,
        ModuleLoadError::new(ModuleLoadStage::Fetch, "later failure"),
    );
    let (_, clients, successful) = store
        .take_next_terminal_notification()
        .expect("later client should be notified by a later terminal transition")
        .into_parts();
    assert!(!successful);
    let (clients, _, _) = clients.into_parts();
    assert_eq!(clients.len(), 1);
    let later_notification_client = clients.into_iter().next().unwrap();
    assert_eq!(later_notification_client.token(), later_client);
    assert_eq!(later_notification_client.client_name(), "DynamicImport");
}
