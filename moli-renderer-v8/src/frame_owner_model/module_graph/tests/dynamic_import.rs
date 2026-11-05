use super::*;

#[test]
fn child_dynamic_import_inflight_fetches_are_realm_local() {
    let mut store = ChildDocumentModulatorStore::default();
    let owner =
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(2), DocumentId(3))
            .document_owner();
    let realm_id = FrameRealmId(4);
    let other_realm_id = FrameRealmId(5);

    let scheduled = store.suspend_dynamic_module_import_fetches(
        owner,
        realm_id,
        vec![dynamic_fetch_request("dynamic-root.mjs")],
        Vec::new(),
        NativeModuleGraphJob::dynamic_import(pending_dynamic_module_import()),
        Vec::new(),
    );
    assert_eq!(scheduled.len(), 1);
    let load_id = scheduled[0].load_id();

    assert!(
        store
            .take_inflight_dynamic_module_import_fetch(owner, other_realm_id, load_id)
            .is_none(),
        "wrong FrameRealm must not take an owner-local dynamic import fetch"
    );
    assert!(
        store
            .take_inflight_dynamic_module_import_fetch(owner, realm_id, load_id)
            .is_some(),
        "matching owner and FrameRealm should take the retained dynamic import fetch"
    );
}

#[test]
fn child_dynamic_import_graph_advance_need_fetches_returns_waiting_action() {
    let mut store = ChildDocumentModulatorStore::default();
    let owner =
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(2), DocumentId(3))
            .document_owner();
    let realm_id = FrameRealmId(4);

    let followup = store.dynamic_import_graph_advance_followup(
        owner,
        realm_id,
        NativeModuleGraphJob::dynamic_import(pending_dynamic_module_import()),
        NativeModuleGraphJobAdvance::NeedFetches(vec![dynamic_fetch_request("dynamic-dep.mjs")]),
    );

    let FrameDocumentDynamicImportGraphAdvanceFollowup::QueueOwnerAction(request) = followup else {
        panic!("NeedFetches should produce a waiting dynamic-import owner action");
    };
    let FrameDocumentDynamicImportOwnerActionQueueRequest::Waiting {
        owner: request_owner,
        realm_id: request_realm_id,
        fetch_actions,
    } = *request
    else {
        panic!("NeedFetches should queue Waiting dynamic-import action");
    };
    assert_eq!(request_owner, owner);
    assert_eq!(request_realm_id, realm_id);
    assert_eq!(fetch_actions.len(), 1);
}

#[test]
fn child_dynamic_import_graph_advance_complete_returns_ready_action() {
    let mut store = ChildDocumentModulatorStore::default();
    let owner =
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(2), DocumentId(3))
            .document_owner();
    let realm_id = FrameRealmId(4);
    let graph = ModuleGraphHandle {
        root_entry: ModuleEntryId::for_test(9),
        entries: vec![ModuleEntryId::for_test(9)],
    };

    let followup = store.dynamic_import_graph_advance_followup(
        owner,
        realm_id,
        NativeModuleGraphJob::dynamic_import(pending_dynamic_module_import()),
        NativeModuleGraphJobAdvance::Complete(graph),
    );

    let FrameDocumentDynamicImportGraphAdvanceFollowup::QueueOwnerAction(request) = followup else {
        panic!("Complete should produce a ready dynamic-import owner action");
    };
    let FrameDocumentDynamicImportOwnerActionQueueRequest::Continuation {
        owner: queued_owner,
        realm_id: queued_realm_id,
        actions,
    } = *request
    else {
        panic!("Complete should queue continuation dynamic-import action");
    };
    assert_eq!(queued_owner, owner);
    assert_eq!(queued_realm_id, realm_id);
    let FrameDocumentDynamicImportOwnerAction::Ready(ready_action) = actions.into_single_for_test()
    else {
        panic!("Complete should queue Ready dynamic-import action");
    };
    assert!(ready_action.is_evaluation_phase());
    assert_eq!(ready_action.root_entry(), ModuleEntryId::for_test(9));
}

#[test]
fn child_dynamic_import_graph_advance_waiting_without_joined_clients_resumes_job() {
    let mut store = ChildDocumentModulatorStore::default();
    let owner =
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(2), DocumentId(3))
            .document_owner();
    let realm_id = FrameRealmId(4);

    let followup = store.dynamic_import_graph_advance_followup(
        owner,
        realm_id,
        NativeModuleGraphJob::dynamic_import(pending_dynamic_module_import()),
        NativeModuleGraphJobAdvance::WaitingForFetches,
    );

    let FrameDocumentDynamicImportGraphAdvanceFollowup::ResumePendingJob(resume) = followup else {
        panic!("WaitingForFetches without joined clients should resume the dynamic import job");
    };
    assert!(resume.job().dynamic_import_request().is_some());
}

#[test]
fn child_dynamic_import_ready_followup_returns_ready_action() {
    let mut store = ChildDocumentModulatorStore::default();
    let owner =
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(2), DocumentId(3))
            .document_owner();
    let realm_id = FrameRealmId(4);
    let graph = ModuleGraphHandle {
        root_entry: ModuleEntryId::for_test(10),
        entries: vec![ModuleEntryId::for_test(10)],
    };

    let followup = store.dynamic_import_ready_followup(
        owner,
        realm_id,
        NativeDynamicModuleImportReady {
            job: NativeModuleGraphJob::dynamic_import(pending_dynamic_module_import()),
            graph,
        },
    );

    let FrameDocumentDynamicImportGraphAdvanceFollowup::QueueOwnerAction(request) = followup else {
        panic!("ready followup should produce a ready dynamic-import owner action");
    };
    let FrameDocumentDynamicImportOwnerActionQueueRequest::Continuation {
        owner: queued_owner,
        realm_id: queued_realm_id,
        actions,
    } = *request
    else {
        panic!("ready followup should queue continuation dynamic-import action");
    };
    assert_eq!(queued_owner, owner);
    assert_eq!(queued_realm_id, realm_id);
    let FrameDocumentDynamicImportOwnerAction::Ready(ready_action) = actions.into_single_for_test()
    else {
        panic!("ready followup should queue Ready dynamic-import action");
    };
    assert!(ready_action.is_evaluation_phase());
    assert_eq!(ready_action.root_entry(), ModuleEntryId::for_test(10));
}

#[test]
fn child_dynamic_import_ready_followup_types_source_phase_action() {
    let mut store = ChildDocumentModulatorStore::default();
    let owner =
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(2), DocumentId(3))
            .document_owner();
    let realm_id = FrameRealmId(4);
    let graph = ModuleGraphHandle {
        root_entry: ModuleEntryId::for_test(11),
        entries: vec![ModuleEntryId::for_test(11)],
    };

    let followup = store.dynamic_import_ready_followup(
        owner,
        realm_id,
        NativeDynamicModuleImportReady {
            job: NativeModuleGraphJob::dynamic_import(pending_dynamic_module_import_with_phase(
                ModuleImportPhase::Source,
            )),
            graph,
        },
    );

    let FrameDocumentDynamicImportGraphAdvanceFollowup::QueueOwnerAction(request) = followup else {
        panic!("source ready followup should produce a ready dynamic-import owner action");
    };
    let FrameDocumentDynamicImportOwnerActionQueueRequest::Continuation { actions, .. } = *request
    else {
        panic!("source ready followup should queue continuation dynamic-import action");
    };
    let FrameDocumentDynamicImportOwnerAction::Ready(ready_action) = actions.into_single_for_test()
    else {
        panic!("source ready followup should queue Ready dynamic-import action");
    };
    assert!(ready_action.is_source_phase());
    assert_eq!(ready_action.root_entry(), ModuleEntryId::for_test(11));
}

#[test]
fn child_dynamic_import_owner_module_fetch_completion_followup_returns_fetch_completion_action() {
    let mut store = ChildDocumentModulatorStore::default();
    let task_owner =
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(2), DocumentId(3));
    let owner = task_owner.document_owner();
    let realm_id = FrameRealmId(4);
    let tree_client = module_tree::SingleModuleClientToken {
        tree_id: module_tree::ModuleTreeId(5),
        sequence: 6,
    };
    let source_url =
        Url::parse("https://child-module-graph.test/app/owner-module-fetch.mjs").unwrap();
    let key = ModuleMapKey::java_script(source_url.clone());
    let entry_id = FrameDocumentModuleClientEntryId::from_raw(7);
    let owner_start = FrameDocumentModuleFetchClientStart::new(
        owner,
        FrameRequestId(8),
        FrameRequestKind::ModuleDependency,
        key,
        FrameDocumentModuleClientRegistration::new(
            entry_id,
            FrameDocumentModuleClientId::from_raw(9),
            FrameDocumentModuleFetchDisposition::StartedFetch(entry_id),
        ),
    );
    let scheduled = store.suspend_dynamic_module_import_fetches(
        owner,
        realm_id,
        vec![dynamic_tree_fetch_request(
            "owner-module-fetch.mjs",
            tree_client,
        )],
        Vec::new(),
        NativeModuleGraphJob::dynamic_import(pending_dynamic_module_import()),
        vec![Some(owner_start.clone())],
    );
    let load_id = scheduled
        .first()
        .expect("owner module fetch should be scheduled")
        .load_id();
    let inflight = store
        .take_inflight_dynamic_module_import_fetch(owner, realm_id, load_id)
        .expect("owner module fetch should be in-flight")
        .inflight;
    let source = Ok(ModuleGraphFetchedSource::new(
        source_url.clone(),
        ModuleSource::text("export const value = 1;".to_owned()),
    ));

    let followup = store.dynamic_import_owner_module_fetch_completion_followup(
        owner,
        realm_id,
        load_id,
        owner_start.clone(),
        source,
        inflight,
    );

    let FrameDocumentDynamicImportGraphAdvanceFollowup::QueueOwnerAction(request) = followup else {
        panic!("owner-module fetch completion should produce a dynamic-import owner action");
    };
    let FrameDocumentDynamicImportOwnerActionQueueRequest::FetchCompletion {
        owner: queued_owner,
        realm_id: queued_realm_id,
        load_id: queued_load_id,
        actions,
    } = *request
    else {
        panic!("owner-module fetch completion should queue fetch-completion action");
    };
    assert_eq!(queued_owner, owner);
    assert_eq!(queued_realm_id, realm_id);
    assert_eq!(queued_load_id, load_id);
    let FrameDocumentDynamicImportOwnerAction::OwnerModuleFetchCompleted {
        load_id: action_load_id,
        settle,
        restore,
    } = actions.into_single_for_test()
    else {
        panic!("owner-module fetch completion should carry owner fetch payload");
    };
    assert_eq!(restore.owner(), owner);
    assert_eq!(restore.realm_id(), realm_id);
    assert_eq!(action_load_id, load_id);
    assert_eq!(settle.start(), &owner_start);
    let fetched_source = match settle.source().as_ref() {
        Ok(fetched_source) => fetched_source,
        Err(error) => panic!("owner fetch source should be successful: {error:?}"),
    };
    assert_eq!(fetched_source.final_url(), &source_url);
    let (_, _, inflight) = restore.into_parts();
    assert!(inflight.owner_module_fetch_start().is_some());
}

#[test]
fn child_dynamic_import_fetch_finish_followup_returns_fetch_completion_action() {
    let mut store = ChildDocumentModulatorStore::default();
    let owner =
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(2), DocumentId(3))
            .document_owner();
    let realm_id = FrameRealmId(4);
    let tree_client = module_tree::SingleModuleClientToken {
        tree_id: module_tree::ModuleTreeId(5),
        sequence: 6,
    };
    assert!(
        store
            .suspend_dynamic_module_import_fetches(
                owner,
                realm_id,
                Vec::new(),
                vec![tree_client],
                NativeModuleGraphJob::dynamic_import(pending_dynamic_module_import()),
                Vec::new(),
            )
            .is_empty()
    );
    let joined = store
        .take_joined_dynamic_module_import_fetch(owner, realm_id, tree_client)
        .expect("matching realm should expose joined dynamic import before failure");
    let failure = joined.joined.into_failure_for_test(ModuleLoadError::new(
        ModuleLoadStage::Fetch,
        "ordinary fetch finish failure",
    ));
    let load_id = 77;

    let followup = store.dynamic_import_fetch_finish_followup(
        owner,
        realm_id,
        load_id,
        DynamicModuleFetchFinish::Failed(failure),
    );

    let FrameDocumentDynamicImportGraphAdvanceFollowup::QueueOwnerAction(request) = followup else {
        panic!("dynamic import fetch finish should produce a dynamic-import owner action");
    };
    let FrameDocumentDynamicImportOwnerActionQueueRequest::FetchCompletion {
        owner: queued_owner,
        realm_id: queued_realm_id,
        load_id: queued_load_id,
        actions,
    } = *request
    else {
        panic!("dynamic import fetch finish should queue fetch-completion action");
    };
    assert_eq!(queued_owner, owner);
    assert_eq!(queued_realm_id, realm_id);
    assert_eq!(queued_load_id, load_id);
    let FrameDocumentDynamicImportOwnerAction::Reject(action) = actions.into_single_for_test()
    else {
        panic!("failed dynamic import fetch finish should queue Failed owner action");
    };
    assert_eq!(
        action.reason(),
        FrameDocumentDynamicImportRejectReason::FetchFailure
    );
    assert_eq!(action.request().specifier(), "./dynamic.mjs");
    assert_eq!(action.error().message(), "ordinary fetch finish failure");
}

#[test]
fn child_dynamic_import_fetch_finish_followup_reports_retained_wait() {
    let mut store = ChildDocumentModulatorStore::default();
    let owner =
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(2), DocumentId(3))
            .document_owner();
    let realm_id = FrameRealmId(4);
    let tree_client = module_tree::SingleModuleClientToken {
        tree_id: module_tree::ModuleTreeId(5),
        sequence: 6,
    };
    let scheduled = store.suspend_dynamic_module_import_fetches(
        owner,
        realm_id,
        vec![dynamic_tree_fetch_request("dynamic-dep.mjs", tree_client)],
        Vec::new(),
        NativeModuleGraphJob::dynamic_import(pending_dynamic_module_import()),
        Vec::new(),
    );
    let load_id = scheduled
        .first()
        .expect("dynamic fetch should be scheduled")
        .load_id();
    let inflight = store
        .take_inflight_dynamic_module_import_fetch(owner, realm_id, load_id)
        .expect("dynamic fetch should be in-flight")
        .inflight;
    let continuation =
        inflight.finish_with_advance_for_test(NativeModuleGraphJobAdvance::WaitingForFetches);

    let followup = store.dynamic_import_fetch_finish_followup(
        owner,
        realm_id,
        load_id,
        DynamicModuleFetchFinish::Advanced(continuation),
    );

    assert!(
        matches!(
            followup,
            FrameDocumentDynamicImportGraphAdvanceFollowup::WaitRetained
        ),
        "WaitingForFetches without new fetches should report retained wait, not queue an empty action"
    );
}

#[test]
fn child_dynamic_import_fetch_finish_followup_reports_unexpected_complete_warning_action() {
    let mut store = ChildDocumentModulatorStore::default();
    let owner =
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(2), DocumentId(3))
            .document_owner();
    let realm_id = FrameRealmId(4);
    let first_client = module_tree::SingleModuleClientToken {
        tree_id: module_tree::ModuleTreeId(5),
        sequence: 6,
    };
    let second_client = module_tree::SingleModuleClientToken {
        tree_id: module_tree::ModuleTreeId(7),
        sequence: 8,
    };
    let scheduled = store.suspend_dynamic_module_import_fetches(
        owner,
        realm_id,
        vec![
            dynamic_tree_fetch_request("dynamic-first.mjs", first_client),
            dynamic_tree_fetch_request("dynamic-second.mjs", second_client),
        ],
        Vec::new(),
        NativeModuleGraphJob::dynamic_import(pending_dynamic_module_import()),
        Vec::new(),
    );
    let first_load_id = scheduled
        .first()
        .expect("first dynamic fetch should be scheduled")
        .load_id();
    let first_inflight = store
        .take_inflight_dynamic_module_import_fetch(owner, realm_id, first_load_id)
        .expect("first dynamic fetch should be in-flight")
        .inflight;
    let first_continuation = first_inflight.finish_with_advance_for_test(
        NativeModuleGraphJobAdvance::Complete(ModuleGraphHandle {
            root_entry: ModuleEntryId::for_test(9),
            entries: Vec::new(),
        }),
    );

    let followup = store.dynamic_import_fetch_finish_followup(
        owner,
        realm_id,
        first_load_id,
        DynamicModuleFetchFinish::Advanced(first_continuation),
    );

    let FrameDocumentDynamicImportGraphAdvanceFollowup::RecordUnexpectedCompleteWarning(warning) =
        followup
    else {
        panic!(
            "first completed fetch must report a typed unexpected-complete warning while sibling waits remain"
        );
    };
    assert_eq!(warning.owner(), owner);
    assert_eq!(warning.realm_id(), realm_id);
}

#[test]
fn child_dynamic_import_fetch_finish_followup_reports_missing_joined_terminal_fetch() {
    let mut store = ChildDocumentModulatorStore::default();
    let owner =
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(2), DocumentId(3))
            .document_owner();
    let realm_id = FrameRealmId(4);
    let stale_realm_id = FrameRealmId(5);
    let tree_client = module_tree::SingleModuleClientToken {
        tree_id: module_tree::ModuleTreeId(6),
        sequence: 7,
    };
    assert!(
        store
            .suspend_dynamic_module_import_fetches(
                owner,
                realm_id,
                Vec::new(),
                vec![tree_client],
                NativeModuleGraphJob::dynamic_import(pending_dynamic_module_import()),
                Vec::new(),
            )
            .is_empty()
    );
    let joined = store
        .take_joined_dynamic_module_import_fetch(owner, realm_id, tree_client)
        .expect("matching realm should expose joined dynamic import before failure");
    let failure = joined.joined.into_failure_for_test(ModuleLoadError::new(
        ModuleLoadStage::Fetch,
        "stale realm dynamic import failure",
    ));
    let load_id = 88;

    let followup = store.dynamic_import_fetch_finish_followup(
        owner,
        stale_realm_id,
        load_id,
        DynamicModuleFetchFinish::Failed(failure),
    );

    let FrameDocumentDynamicImportGraphAdvanceFollowup::RecordMissingJoinedTerminalFetch(missing) =
        followup
    else {
        panic!("missing joined terminal fetch should be reported as a typed diagnostic follow-up");
    };
    assert_eq!(missing.owner(), owner);
    assert_eq!(missing.realm_id(), stale_realm_id);
    assert_eq!(missing.load_id(), load_id);
}

#[test]
fn child_dynamic_import_graph_advance_failure_returns_failed_action() {
    let mut store = ChildDocumentModulatorStore::default();
    let owner =
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(2), DocumentId(3))
            .document_owner();
    let realm_id = FrameRealmId(4);

    let followup = store.dynamic_import_graph_advance_failure_followup(
        owner,
        realm_id,
        NativeModuleGraphJob::dynamic_import(pending_dynamic_module_import()),
        ModuleLoadError::new(ModuleLoadStage::Resolve, "forced graph failure"),
    );

    let FrameDocumentDynamicImportGraphAdvanceFollowup::QueueOwnerAction(request) = followup else {
        panic!("graph failure should produce a failed dynamic-import owner action");
    };
    let FrameDocumentDynamicImportOwnerActionQueueRequest::Continuation {
        owner: queued_owner,
        realm_id: queued_realm_id,
        actions,
    } = *request
    else {
        panic!("graph failure should queue continuation dynamic-import action");
    };
    assert_eq!(queued_owner, owner);
    assert_eq!(queued_realm_id, realm_id);
    let FrameDocumentDynamicImportOwnerAction::Reject(action) = actions.into_single_for_test()
    else {
        panic!("graph failure should queue Reject dynamic-import action");
    };
    assert_eq!(
        action.reason(),
        FrameDocumentDynamicImportRejectReason::GraphAdvanceFailure
    );
    assert_eq!(action.request().phase(), ModuleImportPhase::Evaluation);
    assert_eq!(action.error().message(), "forced graph failure");
}

#[test]
fn child_dynamic_import_joined_terminal_clients_are_realm_local() {
    let mut store = ChildDocumentModulatorStore::default();
    let task_owner =
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(2), DocumentId(3));
    let owner = task_owner.document_owner();
    let realm_id = FrameRealmId(4);
    let other_realm_id = FrameRealmId(5);
    let url = Url::parse("https://child-module-graph.test/joined-dynamic.js").expect("module url");
    let key = ModuleMapKey::java_script(url.clone());
    let tree_client = module_tree::SingleModuleClientToken {
        tree_id: module_tree::ModuleTreeId(8),
        sequence: 21,
    };

    let scheduled = store.suspend_dynamic_module_import_fetches(
        owner,
        realm_id,
        Vec::new(),
        vec![tree_client],
        NativeModuleGraphJob::dynamic_import(pending_dynamic_module_import()),
        Vec::new(),
    );
    assert!(
        scheduled.is_empty(),
        "a joined fetch waits on the existing ModuleMapEntry and should not schedule network"
    );

    {
        let mut document_modulator = store.take_or_create_document_modulator(owner, realm_id);
        assert!(matches!(
            document_modulator.start_or_join_module_fetch(key.clone()),
            ModuleMapFetchDisposition::StartedFetch(_)
        ));
        document_modulator.add_single_module_fetch_client(
            key.clone(),
            NativeModuleMapSingleModuleClient::dynamic_import(
                tree_client,
                module_tree::ModuleImportPhase::Evaluation,
            ),
        );
        let tasks = store.restore_document_modulator(task_owner, realm_id, document_modulator);
        assert!(
            tasks.is_empty(),
            "fetching entry should not emit terminal owner events yet"
        );
    }

    let batch = {
        let mut document_modulator = store.take_or_create_document_modulator(owner, realm_id);
        document_modulator.insert_fetched_source_for_request(
            key.clone(),
            key.clone(),
            ModuleSource::text("export const value = 1;".to_owned()),
            ModuleFetchMetadata::default(),
        );
        store.restore_document_modulator(task_owner, realm_id, document_modulator)
    };
    assert_eq!(
        batch.len(),
        0,
        "dynamic import terminal must not be emitted as a module-script terminal task"
    );
    let actions = batch.into_dynamic_import_owner_actions();
    assert_eq!(actions.len(), 1);
    let action = actions
        .into_iter()
        .next()
        .expect("dynamic import owner action");
    let (_trace, action) = action.into_parts();
    let FrameDocumentDynamicImportOwnerAction::TerminalClient(action) = action else {
        panic!("dynamic import terminal should produce TerminalClient owner action");
    };
    assert_eq!(action.task_owner(), task_owner);
    assert_eq!(action.realm_id(), realm_id);
    assert_eq!(action.key(), &key);
    assert_eq!(action.client_token(), tree_client);
    assert!(
        store
            .take_joined_dynamic_module_import_fetch(owner, other_realm_id, tree_client)
            .is_none(),
        "wrong FrameRealm must not consume the joined dynamic import continuation"
    );
    assert!(
        store
            .take_joined_dynamic_module_import_fetch(owner, realm_id, tree_client)
            .is_some(),
        "matching owner and FrameRealm should recover the joined dynamic import continuation"
    );
    assert!(
        store
            .take_joined_dynamic_module_import_fetch(owner, realm_id, tree_client)
            .is_none(),
        "joined dynamic import continuation should be consumed once"
    );
}

#[test]
fn child_dynamic_import_restore_joined_client_is_realm_local() {
    let mut store = ChildDocumentModulatorStore::default();
    let owner =
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(2), DocumentId(3))
            .document_owner();
    let realm_id = FrameRealmId(4);
    let replacement_realm_id = FrameRealmId(5);
    let tree_client = module_tree::SingleModuleClientToken {
        tree_id: module_tree::ModuleTreeId(8),
        sequence: 21,
    };

    let scheduled = store.suspend_dynamic_module_import_fetches(
        owner,
        realm_id,
        vec![dynamic_tree_fetch_request("dynamic-dep.mjs", tree_client)],
        Vec::new(),
        NativeModuleGraphJob::dynamic_import(pending_dynamic_module_import()),
        Vec::new(),
    );
    let load_id = scheduled
        .first()
        .expect("dynamic import dependency fetch should be scheduled")
        .load_id();
    let inflight = store
        .take_inflight_dynamic_module_import_fetch(owner, realm_id, load_id)
        .expect("matching owner and FrameRealm should take the in-flight fetch");

    let _replacement_document_modulator =
        store.take_or_create_document_modulator(owner, replacement_realm_id);

    assert!(
        store
            .restore_dynamic_module_import_fetch_as_joined_owner_client(
                owner,
                realm_id,
                inflight.inflight,
            )
            .is_none(),
        "stale child FrameRealm must not restore a dynamic import joined client into the replacement document graph"
    );
    assert!(
        store
            .take_joined_dynamic_module_import_fetch(owner, replacement_realm_id, tree_client)
            .is_none(),
        "replacement FrameRealm must not receive joined clients from a stale in-flight fetch"
    );
}

#[test]
fn child_dynamic_import_restore_inflight_by_load_id_is_realm_local() {
    let mut store = ChildDocumentModulatorStore::default();
    let owner =
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(2), DocumentId(3))
            .document_owner();
    let realm_id = FrameRealmId(4);
    let other_realm_id = FrameRealmId(5);
    let tree_client = module_tree::SingleModuleClientToken {
        tree_id: module_tree::ModuleTreeId(8),
        sequence: 21,
    };

    let scheduled = store.suspend_dynamic_module_import_fetches(
        owner,
        realm_id,
        vec![dynamic_tree_fetch_request("dynamic-dep.mjs", tree_client)],
        Vec::new(),
        NativeModuleGraphJob::dynamic_import(pending_dynamic_module_import()),
        Vec::new(),
    );
    let load_id = scheduled
        .first()
        .expect("dynamic import dependency fetch should be scheduled")
        .load_id();

    assert!(
        store
            .restore_inflight_dynamic_module_import_fetch_as_joined_owner_client(
                owner,
                other_realm_id,
                load_id,
            )
            .is_none(),
        "wrong FrameRealm must not restore a dynamic import joined client"
    );
    assert_eq!(
        store.restore_inflight_dynamic_module_import_fetch_as_joined_owner_client(
            owner, realm_id, load_id,
        ),
        Some(tree_client),
        "matching owner and FrameRealm should restore the in-flight fetch as a joined client"
    );
    assert!(
        store
            .take_joined_dynamic_module_import_fetch(owner, realm_id, tree_client)
            .is_some(),
        "restored joined client should be available from the owner-local document graph"
    );
    assert!(
        store
            .restore_inflight_dynamic_module_import_fetch_as_joined_owner_client(
                owner, realm_id, load_id,
            )
            .is_none(),
        "restored in-flight fetch should be consumed once"
    );
}

#[test]
fn child_dynamic_import_failure_clear_is_realm_local() {
    let mut store = ChildDocumentModulatorStore::default();
    let owner =
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(2), DocumentId(3))
            .document_owner();
    let realm_id = FrameRealmId(4);
    let other_realm_id = FrameRealmId(5);

    let wrong_realm_client = module_tree::SingleModuleClientToken {
        tree_id: module_tree::ModuleTreeId(31),
        sequence: 1,
    };
    assert!(
        store
            .suspend_dynamic_module_import_fetches(
                owner,
                realm_id,
                Vec::new(),
                vec![wrong_realm_client],
                NativeModuleGraphJob::dynamic_import(pending_dynamic_module_import()),
                Vec::new(),
            )
            .is_empty()
    );
    let wrong_realm_joined = store
        .take_joined_dynamic_module_import_fetch(owner, realm_id, wrong_realm_client)
        .expect("matching realm should expose joined dynamic import before failure");
    let wrong_realm_failure =
        wrong_realm_joined
            .joined
            .into_failure_for_test(ModuleLoadError::new(
                ModuleLoadStage::Fetch,
                "wrong realm failure",
            ));
    assert!(
        store
            .clear_failed_dynamic_module_import_fetch(owner, other_realm_id, wrong_realm_failure)
            .is_none(),
        "wrong FrameRealm must not clear an owner-local dynamic import failure"
    );

    let matching_client = module_tree::SingleModuleClientToken {
        tree_id: module_tree::ModuleTreeId(32),
        sequence: 1,
    };
    assert!(
        store
            .suspend_dynamic_module_import_fetches(
                owner,
                realm_id,
                Vec::new(),
                vec![matching_client],
                NativeModuleGraphJob::dynamic_import(pending_dynamic_module_import()),
                Vec::new(),
            )
            .is_empty()
    );
    let matching_joined = store
        .take_joined_dynamic_module_import_fetch(owner, realm_id, matching_client)
        .expect("matching realm should expose joined dynamic import before failure");
    let matching_failure = matching_joined
        .joined
        .into_failure_for_test(ModuleLoadError::new(
            ModuleLoadStage::Fetch,
            "matching realm failure",
        ));
    let FrameDocumentDynamicImportTerminalClientFinishResult::FollowupActions(actions) = store
        .dynamic_import_fetch_finish_to_terminal_client_finish_result(
            owner,
            realm_id,
            DynamicModuleFetchFinish::Failed(matching_failure),
        )
    else {
        panic!("failed dynamic import fetch finish should produce a follow-up owner action");
    };
    let FrameDocumentDynamicImportOwnerAction::Reject(action) = actions.into_single_for_test()
    else {
        panic!("failed dynamic import fetch finish should produce a failed owner action");
    };
    assert_eq!(
        action.reason(),
        FrameDocumentDynamicImportRejectReason::FetchFailure
    );
    assert_eq!(action.request().specifier(), "./dynamic.mjs");
    assert_eq!(action.error().message(), "matching realm failure");
}

#[test]
fn dynamic_import_graph_evaluated_mark_stays_in_owner_store() {
    let mut store = ChildDocumentModulatorStore::default();
    let owner =
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(2), DocumentId(3))
            .document_owner();
    let realm_id = FrameRealmId(4);
    let stale_realm_id = FrameRealmId(5);
    let root_url =
        Url::parse("https://child-module-graph.test/dynamic-root.js").expect("dynamic root url");
    let root_key = ModuleMapKey::java_script(root_url);
    let entry_id = {
        let mut document_modulator = store.take_or_create_document_modulator(owner, realm_id);
        let disposition = document_modulator.start_or_join_module_fetch(root_key.clone());
        store.restore_document_modulator_without_owner_events(owner, realm_id, document_modulator);
        disposition.entry_id()
    };

    assert!(store.mark_dynamic_module_graph_evaluated(owner, realm_id, entry_id));
    let current_entry = store
        .current_document_modulator_entry(owner, realm_id)
        .expect("current document modulator entry should exist");
    assert_eq!(
        current_entry
            .document_modulator
            .module_entry_state(entry_id),
        crate::document_module_graph::ModuleMapEntryState::Evaluated
    );
    assert!(!store.mark_dynamic_module_graph_evaluated(owner, stale_realm_id, entry_id));
    let current_entry_after_stale = store
        .current_document_modulator_entry(owner, realm_id)
        .expect("current document modulator entry should still exist");
    assert_eq!(current_entry_after_stale.realm_id, realm_id);
    assert_eq!(
        current_entry_after_stale
            .document_modulator
            .module_entry_state(entry_id),
        crate::document_module_graph::ModuleMapEntryState::Evaluated
    );
}

#[test]
fn dynamic_import_source_wasm_record_lookup_stays_in_owner_store() {
    let mut store = ChildDocumentModulatorStore::default();
    let owner =
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(2), DocumentId(3))
            .document_owner();
    let realm_id = FrameRealmId(4);
    let stale_realm_id = FrameRealmId(5);
    let root_url =
        Url::parse("https://child-module-graph.test/source-root.js").expect("source root url");
    let root_key = ModuleMapKey::java_script(root_url);
    let entry_id = {
        let mut document_modulator = store.take_or_create_document_modulator(owner, realm_id);
        let disposition = document_modulator.start_or_join_module_fetch(root_key.clone());
        store.restore_document_modulator_without_owner_events(owner, realm_id, document_modulator);
        disposition.entry_id()
    };

    let stale_error = store
        .dynamic_module_source_wasm_record(owner, stale_realm_id, entry_id, "./source.wasm")
        .expect_err("stale realm should not read a dynamic import source record");
    assert_eq!(
        stale_error.message(),
        "child dynamic import has no current document modulator"
    );
    let current_entry_after_stale = store
        .current_document_modulator_entry(owner, realm_id)
        .expect("stale lookup should not materialize another modulator entry");
    assert_eq!(current_entry_after_stale.realm_id, realm_id);

    let not_wasm_error = store
        .dynamic_module_source_wasm_record(owner, realm_id, entry_id, "./source.wasm")
        .expect_err("JavaScript module entry should not resolve as a wasm source import");
    assert_eq!(
        not_wasm_error.message(),
        "source-phase dynamic import `./source.wasm` is not a WebAssembly module"
    );
    assert_eq!(
        not_wasm_error.error_constructor(),
        Some(crate::types::ScriptErrorConstructorKind::SyntaxError)
    );
}
