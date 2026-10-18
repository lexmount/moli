use super::*;
use crate::frame_owner_model::FrameDocumentModuleScriptTerminalTask;

#[test]
fn response_modulepreload_joins_the_module_map_without_manufacturing_link_events() {
    use crate::frame_owner_model::FrameDocumentModulepreloadSource;
    let mut store = ChildDocumentModulatorStore::default();
    let owner =
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(2), DocumentId(3));
    let realm_id = FrameRealmId(4);
    let request = modulepreload_request("header-entry.mjs");
    let source = FrameDocumentModulepreloadSource::ResponseHeader {
        child_handle: DomHandle::new(1),
        owner,
    };
    let task =
        FrameDocumentModulepreloadFetchTask::from_source_parts(realm_id, source, request.clone());
    let duplicate = task.clone();
    let FrameDocumentModulepreloadStartAction::ScheduleFetch {
        target,
        link_handle,
        load_id,
        ..
    } = store.start_modulepreload_fetch_task(task)
    else {
        panic!("header modulepreload must reserve its own Document module entry");
    };
    assert_eq!(target.task_owner(), owner);
    assert_eq!(target.realm_id(), realm_id);
    assert_eq!(target.child_handle(), DomHandle::new(1));
    assert!(link_handle.is_none());
    assert!(matches!(
        store.start_modulepreload_fetch_task(duplicate.clone()),
        FrameDocumentModulepreloadStartAction::JoinedFetching {
            link_handle: None,
            ..
        }
    ));
    let link_handle = DomHandle::new(77);
    let link = FrameDocumentModulepreloadFetchTask::from_modulepreload_fetch_parts(
        realm_id,
        modulepreload_link_client(owner, link_handle),
        request.clone(),
    );
    assert!(
        matches!(store.start_modulepreload_fetch_task(link), FrameDocumentModulepreloadStartAction::JoinedFetching { link_handle: Some(actual), .. } if actual == link_handle)
    );
    let inflight = store
        .take_modulepreload_graph_fetch(owner.document_owner(), realm_id, load_id)
        .unwrap();
    let batch = store
        .finish_modulepreload_fetch(
            owner,
            realm_id,
            inflight,
            Ok(ModuleGraphFetchedSource::new(
                request.source_url().clone(),
                false,
                ModuleSource::text("export const value = 1;"),
            )),
        )
        .unwrap();
    let events = batch.into_modulepreload_terminal_works();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].link_handle(), link_handle);
    assert!(events[0].successful());
    assert!(matches!(
        store.start_modulepreload_fetch_task(duplicate),
        FrameDocumentModulepreloadStartAction::JoinedTerminalSuccess {
            link_handle: None,
            ..
        }
    ));
    assert!(
        store
            .take_ready_document_modulator_terminal_batches(owner, realm_id)
            .is_empty()
    );
}

#[test]
fn response_modulepreload_awaiting_a_realm_retains_exact_document_and_realm_guards() {
    use crate::frame_owner_model::{
        FrameDocumentModulepreloadMaterializedWork, FrameDocumentModulepreloadSource,
        FrameDocumentModulepreloadWorkAwaitingRealm,
    };
    let owner =
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(2), DocumentId(3));
    let realm_id = FrameRealmId(4);
    let source = FrameDocumentModulepreloadSource::ResponseHeader {
        child_handle: DomHandle::new(1),
        owner,
    };
    let mut work = FrameDocumentModulepreloadWorkAwaitingRealm::fetch_start(
        None,
        source,
        modulepreload_request("header-pending.mjs"),
    );
    assert_eq!(work.owner(), owner);
    assert_eq!(work.child_handle(), DomHandle::new(1));
    assert!(work.link_handle().is_none());
    work.bind_first_established_realm(realm_id);
    work.bind_first_established_realm(FrameRealmId(5));
    assert!(
        work.clone()
            .into_materialized_work(FrameRealmId(5))
            .is_none()
    );
    let FrameDocumentModulepreloadMaterializedWork::FetchStart(task) =
        work.into_materialized_work(realm_id).unwrap()
    else {
        panic!("header hint cannot manufacture a link error event");
    };
    assert_eq!(
        task.target(),
        ChildDocumentModuleFetchTarget::new(DomHandle::new(1), owner, realm_id)
    );
    assert!(task.client().is_none());
}

#[test]
fn modulepreload_start_action_reserves_fetch_in_modulator_store() {
    let mut store = ChildDocumentModulatorStore::default();
    let owner =
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(2), DocumentId(3));
    let document_owner = owner.document_owner();
    let realm_id = FrameRealmId(4);
    let link_handle = DomHandle::new(77);
    let request = modulepreload_request("modulepreload-start.js");
    let key = request.module_key().clone();
    let task = FrameDocumentModulepreloadFetchTask::from_modulepreload_fetch_parts(
        realm_id,
        modulepreload_link_client(owner, link_handle),
        request.clone(),
    );

    let FrameDocumentModulepreloadStartAction::ScheduleFetch {
        target,
        link_handle: action_link_handle,
        key: action_key,
        load_id,
        request: fetch_request,
    } = store.start_modulepreload_fetch_task(task)
    else {
        panic!("first modulepreload task should schedule a graph fetch");
    };

    assert_eq!(target.child_handle(), DomHandle::new(1));
    assert_eq!(target.task_owner(), owner);
    assert_eq!(target.realm_id(), realm_id);
    assert_eq!(action_link_handle, Some(link_handle));
    assert_eq!(action_key, key);
    assert_eq!(fetch_request.source_url(), request.source_url());
    let inflight = store
        .take_modulepreload_graph_fetch(document_owner, realm_id, load_id)
        .expect("scheduled modulepreload fetch should be reserved in the owner store");
    assert_eq!(inflight, request);
}

#[derive(Default)]
struct FakeModulepreloadStartHooks {
    terminal_followup: FrameDocumentModuleTerminalQueueFollowup,
    terminal_followup_calls: usize,
    scheduled_fetch_calls: usize,
    joined_fetching_records: usize,
    joined_terminal_success_records: usize,
    joined_terminal_failure_records: usize,
}

impl FrameDocumentModulepreloadStartActionHooks for FakeModulepreloadStartHooks {
    fn post_current_document_modulator_terminals(
        &mut self,
        _owner: FrameDocumentOwner,
        _realm_id: FrameRealmId,
    ) -> FrameDocumentModuleTerminalQueueFollowup {
        self.terminal_followup_calls += 1;
        self.terminal_followup
    }

    fn schedule_modulepreload_fetch(
        &mut self,
        _target: ChildDocumentModuleFetchTarget,
        _link_handle: Option<DomHandle>,
        _key: ModuleMapKey,
        _load_id: u64,
        _request: Box<NativeModuleGraphFetchRequest>,
    ) {
        self.scheduled_fetch_calls += 1;
    }

    fn record_joined_fetching(
        &mut self,
        _owner: FrameDocumentOwner,
        _realm_id: FrameRealmId,
        _link_handle: Option<DomHandle>,
        _key: &ModuleMapKey,
    ) {
        self.joined_fetching_records += 1;
    }

    fn record_joined_terminal_success(
        &mut self,
        _owner: FrameDocumentOwner,
        _realm_id: FrameRealmId,
        _link_handle: Option<DomHandle>,
        _key: &ModuleMapKey,
    ) {
        self.joined_terminal_success_records += 1;
    }

    fn record_joined_terminal_failure(
        &mut self,
        _owner: FrameDocumentOwner,
        _realm_id: FrameRealmId,
        _link_handle: Option<DomHandle>,
        _key: &ModuleMapKey,
    ) {
        self.joined_terminal_failure_records += 1;
    }
}

#[test]
fn modulepreload_start_action_runner_schedules_fetch() {
    let mut store = ChildDocumentModulatorStore::default();
    let owner =
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(2), DocumentId(3));
    let realm_id = FrameRealmId(4);
    let link_handle = DomHandle::new(77);
    let request = modulepreload_request("modulepreload-runner-start.js");
    let task = FrameDocumentModulepreloadFetchTask::from_modulepreload_fetch_parts(
        realm_id,
        modulepreload_link_client(owner, link_handle),
        request,
    );
    let action = store.start_modulepreload_fetch_task(task);
    let mut runner =
        FrameDocumentModulepreloadStartActionRunner::new(FakeModulepreloadStartHooks::default());

    let outcome = runner.run_start_action(action);
    let hooks = runner.into_hooks();

    assert!(outcome.fetch_was_scheduled());
    assert!(!outcome.terminal_followup_was_queued());
    assert_eq!(hooks.terminal_followup_calls, 1);
    assert_eq!(hooks.scheduled_fetch_calls, 1);
    assert_eq!(hooks.joined_terminal_success_records, 0);
}

#[test]
fn modulepreload_start_action_runner_posts_terminal_followup() {
    let owner =
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(2), DocumentId(3));
    let document_owner = owner.document_owner();
    let realm_id = FrameRealmId(4);
    let link_handle = DomHandle::new(77);
    let request = modulepreload_request("modulepreload-runner-terminal.js");
    let key = request.module_key().clone();
    let action = FrameDocumentModulepreloadStartAction::JoinedTerminalSuccess {
        owner: document_owner,
        realm_id,
        link_handle: Some(link_handle),
        key,
    };
    let hooks = FakeModulepreloadStartHooks {
        terminal_followup:
            FrameDocumentModuleTerminalQueueFollowup::modulepreload_event_action_queued(),
        ..FakeModulepreloadStartHooks::default()
    };
    let mut runner = FrameDocumentModulepreloadStartActionRunner::new(hooks);

    let outcome = runner.run_start_action(action);
    let hooks = runner.into_hooks();

    assert!(outcome.terminal_followup_was_queued());
    assert!(outcome.joined_terminal_success_was_recorded());
    assert_eq!(hooks.terminal_followup_calls, 1);
    assert_eq!(hooks.scheduled_fetch_calls, 0);
    assert_eq!(hooks.joined_terminal_success_records, 1);
}

struct FakeModulepreloadCompletionHooks {
    finish_result: FrameDocumentModulepreloadFetchFinishResult,
    finished_owner: Option<FrameDocumentTaskOwner>,
    finished_realm: Option<FrameRealmId>,
    finish_calls: usize,
    queued_batches: usize,
    missing_modulator_records: usize,
    finished_records: usize,
}

impl FakeModulepreloadCompletionHooks {
    fn finished() -> Self {
        Self {
            finish_result: FrameDocumentModulepreloadFetchFinishResult::Finished(
                FrameDocumentModuleTerminalBatch::default(),
            ),
            finished_owner: None,
            finished_realm: None,
            finish_calls: 0,
            queued_batches: 0,
            missing_modulator_records: 0,
            finished_records: 0,
        }
    }

    fn missing_modulator() -> Self {
        Self {
            finish_result: FrameDocumentModulepreloadFetchFinishResult::MissingDocumentModulator,
            finished_owner: None,
            finished_realm: None,
            finish_calls: 0,
            queued_batches: 0,
            missing_modulator_records: 0,
            finished_records: 0,
        }
    }
}

impl FrameDocumentModulepreloadFetchCompletionHooks for FakeModulepreloadCompletionHooks {
    fn finish_modulepreload_fetch(
        &mut self,
        owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
        _request: NativeModuleSingleFetchRequest,
        _source: Result<ModuleGraphFetchedSource, ModuleLoadError>,
    ) -> FrameDocumentModulepreloadFetchFinishResult {
        self.finish_calls += 1;
        self.finished_owner = Some(owner);
        self.finished_realm = Some(realm_id);
        std::mem::replace(
            &mut self.finish_result,
            FrameDocumentModulepreloadFetchFinishResult::MissingDocumentModulator,
        )
    }

    fn queue_module_terminal_batch(
        &mut self,
        _batch: FrameDocumentModuleTerminalBatch,
    ) -> FrameDocumentModuleTerminalQueueFollowup {
        self.queued_batches += 1;
        FrameDocumentModuleTerminalQueueFollowup::modulepreload_event_action_queued()
    }

    fn record_missing_modulepreload_modulator(
        &mut self,
        _owner: FrameDocumentOwner,
        _realm_id: FrameRealmId,
        _load_id: u64,
    ) {
        self.missing_modulator_records += 1;
    }

    fn record_modulepreload_completion_finished(
        &mut self,
        _owner: FrameDocumentTaskOwner,
        _realm_id: FrameRealmId,
        _load_id: u64,
        _key: &ModuleMapKey,
    ) {
        self.finished_records += 1;
    }
}

fn modulepreload_completion_action(
    owner: FrameDocumentTaskOwner,
    realm_id: FrameRealmId,
) -> FrameDocumentModulepreloadFetchCompletionAction {
    let request = modulepreload_request("modulepreload-runner-complete.js");
    let source = Ok(ModuleGraphFetchedSource::new(
        request.source_url().clone(),
        false,
        ModuleSource::text("export const value = 1;".to_owned()),
    ));
    FrameDocumentModulepreloadFetchCompletionAction::new(owner, realm_id, 55, request, source)
}

#[test]
fn modulepreload_fetch_completion_runner_finishes_and_queues_terminal_batch() {
    let task_owner =
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(2), DocumentId(3));
    let realm_id = FrameRealmId(4);
    let hooks = FakeModulepreloadCompletionHooks::finished();
    let mut runner = FrameDocumentModulepreloadFetchCompletionRunner::new(hooks);

    let outcome =
        runner.run_completion_action(modulepreload_completion_action(task_owner, realm_id));
    let hooks = runner.into_hooks();

    assert!(outcome.fetch_was_finished());
    assert!(outcome.terminal_followup_was_queued());
    assert_eq!(hooks.finish_calls, 1);
    assert_eq!(hooks.finished_owner, Some(task_owner));
    assert_eq!(hooks.finished_realm, Some(realm_id));
    assert_eq!(hooks.queued_batches, 1);
    assert_eq!(hooks.finished_records, 1);
}

#[test]
fn modulepreload_fetch_completion_runner_records_missing_modulator() {
    let task_owner =
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(2), DocumentId(3));
    let realm_id = FrameRealmId(4);
    let hooks = FakeModulepreloadCompletionHooks::missing_modulator();
    let mut runner = FrameDocumentModulepreloadFetchCompletionRunner::new(hooks);

    let outcome =
        runner.run_completion_action(modulepreload_completion_action(task_owner, realm_id));
    let hooks = runner.into_hooks();

    assert!(outcome.missing_document_modulator_was_recorded());
    assert!(!outcome.terminal_followup_was_queued());
    assert_eq!(hooks.finish_calls, 1);
    assert_eq!(hooks.finished_owner, Some(task_owner));
    assert_eq!(hooks.finished_realm, Some(realm_id));
    assert_eq!(hooks.queued_batches, 0);
    assert_eq!(hooks.missing_modulator_records, 1);
}

#[test]
fn modulepreload_start_action_joins_fetching_entry_in_modulator_store() {
    let mut store = ChildDocumentModulatorStore::default();
    let owner =
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(2), DocumentId(3));
    let document_owner = owner.document_owner();
    let realm_id = FrameRealmId(4);
    let first_link = DomHandle::new(77);
    let joined_link = DomHandle::new(78);
    let request = modulepreload_request("modulepreload-joined.js");
    let key = request.module_key().clone();
    let first_task = FrameDocumentModulepreloadFetchTask::from_modulepreload_fetch_parts(
        realm_id,
        modulepreload_link_client(owner, first_link),
        request.clone(),
    );
    let joined_task = FrameDocumentModulepreloadFetchTask::from_modulepreload_fetch_parts(
        realm_id,
        modulepreload_link_client(owner, joined_link),
        request,
    );

    assert!(matches!(
        store.start_modulepreload_fetch_task(first_task),
        FrameDocumentModulepreloadStartAction::ScheduleFetch { .. }
    ));
    let FrameDocumentModulepreloadStartAction::JoinedFetching {
        owner: action_owner,
        realm_id: action_realm_id,
        link_handle,
        key: action_key,
    } = store.start_modulepreload_fetch_task(joined_task)
    else {
        panic!("second modulepreload task for a fetching key should join the existing fetch");
    };

    assert_eq!(action_owner, document_owner);
    assert_eq!(action_realm_id, realm_id);
    assert_eq!(link_handle, Some(joined_link));
    assert_eq!(action_key, key);
}

#[test]
fn modulepreload_start_action_joins_terminal_entry_as_terminal_work() {
    let mut store = ChildDocumentModulatorStore::default();
    let owner =
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(2), DocumentId(3));
    let document_owner = owner.document_owner();
    let realm_id = FrameRealmId(4);
    let link_handle = DomHandle::new(77);
    let request = modulepreload_request("modulepreload-terminal.js");
    let key = request.module_key().clone();
    let mut document_modulator = store.take_or_create_document_modulator(document_owner, realm_id);
    assert!(matches!(
        document_modulator.start_or_join_fetch(key.clone()),
        ModuleMapFetchDisposition::StartedFetch(_)
    ));
    document_modulator.insert_fetched_source(
        key.clone(),
        ModuleSource::text("export const value = 1;".to_owned()),
    );
    let tasks = store.restore_document_modulator(owner, realm_id, document_modulator);
    assert!(
        tasks.is_empty(),
        "the terminal entry has no clients before the modulepreload task joins"
    );
    let task = FrameDocumentModulepreloadFetchTask::from_modulepreload_fetch_parts(
        realm_id,
        modulepreload_link_client(owner, link_handle),
        request,
    );

    let FrameDocumentModulepreloadStartAction::JoinedTerminalSuccess {
        owner: action_owner,
        realm_id: action_realm_id,
        link_handle: action_link_handle,
        key: action_key,
    } = store.start_modulepreload_fetch_task(task)
    else {
        panic!("modulepreload task for a fetched entry should join the terminal result");
    };
    assert_eq!(action_owner, document_owner);
    assert_eq!(action_realm_id, realm_id);
    assert_eq!(action_link_handle, Some(link_handle));
    assert_eq!(action_key, key);

    let batch = store.take_ready_document_modulator_terminal_batches(owner, realm_id);
    assert_eq!(
        batch.len(),
        0,
        "terminal modulepreload joins should not queue module owner-event work"
    );
    let works = batch.into_modulepreload_terminal_works();
    assert_eq!(works.len(), 1);
    let work = works
        .into_iter()
        .next()
        .expect("terminal join should produce one modulepreload terminal work");
    assert_eq!(work.owner(), owner);
    assert_eq!(work.realm_id(), realm_id);
    assert_eq!(work.key(), Some(&key));
    assert_eq!(work.link_handle(), link_handle);
    assert!(work.successful());
}

#[test]
fn modulepreload_fetch_completion_settles_source_in_modulator_store() {
    let mut store = ChildDocumentModulatorStore::default();
    let owner =
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(2), DocumentId(3));
    let document_owner = owner.document_owner();
    let realm_id = FrameRealmId(4);
    let link_handle = DomHandle::new(77);
    let request = modulepreload_request("modulepreload-complete.js");
    let key = request.module_key().clone();
    let task = FrameDocumentModulepreloadFetchTask::from_modulepreload_fetch_parts(
        realm_id,
        modulepreload_link_client(owner, link_handle),
        request.clone(),
    );
    let FrameDocumentModulepreloadStartAction::ScheduleFetch { load_id, .. } =
        store.start_modulepreload_fetch_task(task)
    else {
        panic!("modulepreload task should schedule a graph fetch before completion");
    };
    let inflight = store
        .take_modulepreload_graph_fetch(document_owner, realm_id, load_id)
        .expect("scheduled modulepreload request should be in flight");

    let batch = store
        .finish_modulepreload_fetch(
            owner,
            realm_id,
            inflight,
            Ok(ModuleGraphFetchedSource::new(
                request.source_url().clone(),
                false,
                ModuleSource::text("export const value = 1;".to_owned()),
            )),
        )
        .expect("current document modulator should accept modulepreload completion");

    assert_eq!(batch.len(), 0);
    let works = batch.into_modulepreload_terminal_works();
    assert_eq!(works.len(), 1);
    let work = works
        .into_iter()
        .next()
        .expect("successful modulepreload completion should produce terminal work");
    assert_eq!(work.owner(), owner);
    assert_eq!(work.realm_id(), realm_id);
    assert_eq!(work.key(), Some(&key));
    assert_eq!(work.link_handle(), link_handle);
    assert!(work.successful());
}

#[test]
fn modulepreload_fetch_completion_marks_failure_in_modulator_store() {
    let mut store = ChildDocumentModulatorStore::default();
    let owner =
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(2), DocumentId(3));
    let document_owner = owner.document_owner();
    let realm_id = FrameRealmId(4);
    let link_handle = DomHandle::new(77);
    let request = modulepreload_request("modulepreload-failure.js");
    let key = request.module_key().clone();
    let task = FrameDocumentModulepreloadFetchTask::from_modulepreload_fetch_parts(
        realm_id,
        modulepreload_link_client(owner, link_handle),
        request,
    );
    let FrameDocumentModulepreloadStartAction::ScheduleFetch { load_id, .. } =
        store.start_modulepreload_fetch_task(task)
    else {
        panic!("modulepreload task should schedule a graph fetch before completion");
    };
    let inflight = store
        .take_modulepreload_graph_fetch(document_owner, realm_id, load_id)
        .expect("scheduled modulepreload request should be in flight");

    let batch = store
        .finish_modulepreload_fetch(
            owner,
            realm_id,
            inflight,
            Err(ModuleLoadError::new(
                ModuleLoadStage::Fetch,
                "modulepreload failed",
            )),
        )
        .expect("current document modulator should accept modulepreload failure");

    assert_eq!(batch.len(), 0);
    let works = batch.into_modulepreload_terminal_works();
    assert_eq!(works.len(), 1);
    let work = works
        .into_iter()
        .next()
        .expect("failed modulepreload completion should produce terminal work");
    assert_eq!(work.owner(), owner);
    assert_eq!(work.realm_id(), realm_id);
    assert_eq!(work.key(), Some(&key));
    assert_eq!(work.link_handle(), link_handle);
    assert!(!work.successful());
}

#[test]
fn restored_document_modulator_returns_modulepreload_terminal_work() {
    let mut store = ChildDocumentModulatorStore::default();
    let owner =
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(2), DocumentId(3));
    let document_owner = owner.document_owner();
    let realm_id = FrameRealmId(4);
    let key = ModuleMapKey::java_script(
        Url::parse("https://child-module-graph.test/modulepreload.js").expect("module url"),
    );
    let link_handle = DomHandle::new(77);

    let mut document_modulator = store.take_or_create_document_modulator(document_owner, realm_id);
    assert!(matches!(
        document_modulator.start_or_join_fetch(key.clone()),
        ModuleMapFetchDisposition::StartedFetch(_)
    ));
    let link_client = crate::module_runtime::NativeModulepreloadLinkClient::new_for_frame_document(
        link_handle,
        key.clone(),
        modulepreload_link_client(owner, link_handle),
    );
    document_modulator.add_modulepreload_link_client(key.clone(), link_client.clone());
    document_modulator.insert_fetched_source(
        key.clone(),
        ModuleSource::text("export const value = 1;".to_owned()),
    );
    let batch = store.restore_document_modulator(owner, realm_id, document_modulator);

    assert_eq!(
        batch.len(),
        0,
        "modulepreload terminal notification should not return module owner-event work"
    );
    let works = batch.into_modulepreload_terminal_works();
    assert_eq!(works.len(), 1);
    let work = works
        .into_iter()
        .next()
        .expect("child modulepreload terminal work should be returned");
    assert_eq!(work.owner(), owner);
    assert_eq!(work.realm_id(), realm_id);
    assert_eq!(work.key(), Some(&key));
    assert_eq!(work.link_handle(), link_handle);
    assert!(work.successful());
}

#[test]
fn module_script_single_module_client_becomes_terminal_work() {
    let mut store = ChildDocumentModulatorStore::default();
    let owner =
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(2), DocumentId(3));
    let document_owner = owner.document_owner();
    let realm_id = FrameRealmId(4);
    let key = ModuleMapKey::java_script(
        Url::parse("https://child-module-graph.test/module-script-client.js").expect("module url"),
    );
    let tree_client = module_tree::SingleModuleClientToken {
        tree_id: module_tree::ModuleTreeId(9),
        sequence: 12,
    };

    let mut document_modulator = store.take_or_create_document_modulator(document_owner, realm_id);
    assert!(matches!(
        document_modulator.start_or_join_fetch(key.clone()),
        ModuleMapFetchDisposition::StartedFetch(_)
    ));
    document_modulator.add_single_module_fetch_client(
        key.clone(),
        NativeModuleMapSingleModuleClient::module_script(
            tree_client,
            module_tree::ModuleImportPhase::Evaluation,
        ),
    );
    document_modulator.insert_fetched_source(
        key.clone(),
        ModuleSource::text("export const value = 1;".to_owned()),
    );
    let tasks = store.restore_document_modulator(owner, realm_id, document_modulator);

    let task = tasks
        .into_iter()
        .next()
        .expect("terminal notification should be returned");
    assert_eq!(task.owner(), owner);
    assert_eq!(task.realm_id(), realm_id);
    let mut module_script_terminal_tasks = task.into_payload();
    assert_eq!(module_script_terminal_tasks.len(), 1);
    let terminal_work = match module_script_terminal_tasks
        .pop()
        .expect("module-script terminal event should contain one work")
    {
        FrameDocumentModuleScriptTerminalTask::SingleModule(work) => work,
        FrameDocumentModuleScriptTerminalTask::ParserRoot(_) => {
            panic!("expected single-module terminal work")
        }
        FrameDocumentModuleScriptTerminalTask::Dependency(_) => {
            panic!("expected single-module terminal work")
        }
    };

    assert_eq!(terminal_work.owner(), owner);
    assert_eq!(terminal_work.realm_id(), realm_id);
    assert_eq!(terminal_work.key(), &key);
    assert_eq!(terminal_work.client().token(), tree_client);
    assert_eq!(
        terminal_work.client().import_phase(),
        module_tree::ModuleImportPhase::Evaluation
    );
}

#[test]
fn modulepreload_link_error_event_builds_terminal_work_without_module_key() {
    let owner =
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(2), DocumentId(3));
    let realm_id = FrameRealmId(4);
    let link_handle = DomHandle::new(77);

    let terminal_work = FrameDocumentModulepreloadTerminalWork::from_link_error_parts(
        realm_id,
        modulepreload_link_client(owner, link_handle),
    );

    assert_eq!(terminal_work.owner(), owner);
    assert_eq!(terminal_work.realm_id(), realm_id);
    assert_eq!(terminal_work.key(), None);
    assert_eq!(terminal_work.link_handle(), link_handle);
    assert!(!terminal_work.successful());
}

#[test]
fn stale_document_modulator_restore_does_not_replace_current_realm() {
    let mut store = ChildDocumentModulatorStore::default();
    let task_owner =
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(2), DocumentId(3));
    let owner = task_owner.document_owner();
    let old_realm_id = FrameRealmId(4);
    let new_realm_id = FrameRealmId(5);
    let old_key = ModuleMapKey::java_script(
        Url::parse("https://child-module-graph.test/old.js").expect("old module url"),
    );
    let new_key = ModuleMapKey::java_script(
        Url::parse("https://child-module-graph.test/new.js").expect("new module url"),
    );

    let mut old_modulator = store.take_or_create_document_modulator(owner, old_realm_id);
    assert!(matches!(
        old_modulator.start_or_join_fetch(old_key.clone()),
        ModuleMapFetchDisposition::StartedFetch(_)
    ));

    let mut new_modulator = store.take_or_create_document_modulator(owner, new_realm_id);
    assert!(matches!(
        new_modulator.start_or_join_fetch(new_key.clone()),
        ModuleMapFetchDisposition::StartedFetch(_)
    ));
    assert!(
        store
            .restore_document_modulator(task_owner, new_realm_id, new_modulator)
            .is_empty(),
        "restoring the current replacement realm should not emit terminal work"
    );

    assert!(
        store
            .restore_document_modulator(task_owner, old_realm_id, old_modulator)
            .is_empty(),
        "stale old realm restore must be dropped instead of replacing the current graph"
    );
    assert!(
        store
            .take_current_document_modulator(owner, old_realm_id)
            .is_none(),
        "old realm must not be re-created by stale restore"
    );

    let mut current_modulator = store
        .take_current_document_modulator(owner, new_realm_id)
        .expect("replacement realm should remain current");
    assert!(
        matches!(
            current_modulator.start_or_join_fetch(new_key.clone()),
            ModuleMapFetchDisposition::JoinedFetching(_)
        ),
        "replacement realm graph should survive stale restore"
    );
    assert!(
        matches!(
            current_modulator.start_or_join_fetch(old_key.clone()),
            ModuleMapFetchDisposition::StartedFetch(_)
        ),
        "stale old realm graph must not overwrite replacement graph"
    );
}

#[test]
fn document_replacement_preserves_same_execution_context_modulator() {
    let mut store = ChildDocumentModulatorStore::default();
    let local_window_id = LocalWindowId(2);
    let original_owner =
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), local_window_id, DocumentId(3));
    let replacement_owner =
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), local_window_id, DocumentId(4));
    let realm_id = FrameRealmId(5);
    let key = ModuleMapKey::java_script(
        Url::parse("https://child-module-graph.test/preserved.js").expect("module url"),
    );

    let mut modulator =
        store.take_or_create_document_modulator(original_owner.document_owner(), realm_id);
    assert!(matches!(
        modulator.start_or_join_fetch(key.clone()),
        ModuleMapFetchDisposition::StartedFetch(_)
    ));
    store.restore_document_modulator_without_owner_events(
        original_owner.document_owner(),
        realm_id,
        modulator,
    );

    let mut replacement_modulator = store
        .take_current_document_modulator(replacement_owner.document_owner(), realm_id)
        .expect("document.open replacement should retain the LocalWindow module map");
    assert!(matches!(
        replacement_modulator.start_or_join_fetch(key),
        ModuleMapFetchDisposition::JoinedFetching(_)
    ));
}
