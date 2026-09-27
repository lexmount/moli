use super::*;
use crate::frame_owner_model::FrameDocumentModuleScriptTerminalTask;

#[test]
fn module_terminal_queue_followup_merges_queued_work() {
    let mut followup = FrameDocumentModuleTerminalQueueFollowup::none();

    assert!(!followup.made_progress());
    followup.merge(FrameDocumentModuleTerminalQueueFollowup::module_script_terminal_queued());
    followup.merge(FrameDocumentModuleTerminalQueueFollowup::modulepreload_event_action_queued());
    followup.merge(FrameDocumentModuleTerminalQueueFollowup::dynamic_import_owner_action_queued());
    followup.merge(FrameDocumentModuleTerminalQueueFollowup::dynamic_import_wait_retained());
    followup.merge(FrameDocumentModuleTerminalQueueFollowup::dynamic_import_job_resumed());
    followup.merge(FrameDocumentModuleTerminalQueueFollowup::terminal_warning_recorded());

    assert!(followup.made_progress());
    assert!(followup.module_script_terminal_was_queued());
    assert!(followup.modulepreload_event_action_was_queued());
    assert!(followup.dynamic_import_owner_action_was_queued());
    assert!(followup.dynamic_import_wait_was_retained());
    assert!(followup.dynamic_import_job_was_resumed());
    assert!(followup.terminal_warning_was_recorded());
}

#[test]
fn module_terminal_queue_followup_reports_warning_progress() {
    let empty = FrameDocumentModuleTerminalQueueFollowup::terminal_warning_from_recorded(false);
    assert!(!empty.made_progress());
    assert!(!empty.terminal_warning_was_recorded());

    let warning = FrameDocumentModuleTerminalQueueFollowup::terminal_warning_from_recorded(true);
    assert!(warning.made_progress());
    assert!(warning.terminal_warning_was_recorded());
}

#[test]
fn module_terminal_queue_followup_reports_dynamic_import_wait_progress() {
    let followup = FrameDocumentModuleTerminalQueueFollowup::dynamic_import_wait_retained();

    assert!(followup.made_progress());
    assert!(followup.dynamic_import_wait_was_retained());
    assert!(!followup.dynamic_import_owner_action_was_queued());
    assert!(!followup.dynamic_import_job_was_resumed());
}

#[test]
fn module_terminal_queue_followup_reports_dynamic_import_job_resume_progress() {
    let followup = FrameDocumentModuleTerminalQueueFollowup::dynamic_import_job_resumed();

    assert!(followup.made_progress());
    assert!(followup.dynamic_import_job_was_resumed());
    assert!(!followup.dynamic_import_owner_action_was_queued());
    assert!(!followup.dynamic_import_wait_was_retained());
}

#[test]
fn module_terminal_batch_keeps_terminal_warnings_out_of_task_lane() {
    let task_owner =
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(7), LocalWindowId(8), DocumentId(9));
    let realm_id = FrameRealmId(10);
    let key = ModuleMapKey::java_script(
        Url::parse("https://module-owner-event-batch.test/root.js").expect("module url"),
    );
    let mut batch = FrameDocumentModuleTerminalBatch::default();

    batch.push_warning(FrameDocumentModuleTerminalWarningRecord::new(
        task_owner,
        realm_id,
        FrameDocumentModuleTerminalWarning::ParserRootTerminalWithoutOwnerWork {
            key: key.clone(),
            successful: false,
            parser_root_client_count: 2,
        },
    ));

    assert_eq!(
        batch.len(),
        0,
        "terminal warnings are diagnostics, not module-script terminal tasks"
    );
    let warnings = batch.into_warnings();
    assert_eq!(warnings.len(), 1);
    let (warning_owner, warning_realm_id, warning) = warnings
        .into_iter()
        .next()
        .expect("terminal warning")
        .into_parts();
    assert_eq!(warning_owner, task_owner);
    assert_eq!(warning_realm_id, realm_id);
    let FrameDocumentModuleTerminalWarning::ParserRootTerminalWithoutOwnerWork {
        key: warning_key,
        successful,
        parser_root_client_count,
    } = warning;
    assert_eq!(warning_key, key);
    assert!(!successful);
    assert_eq!(parser_root_client_count, 2);
}

#[test]
fn module_terminal_batch_keeps_dynamic_import_actions_out_of_event_task_lane() {
    let mut batch = FrameDocumentModuleTerminalBatch::default();

    batch.push_dynamic_import_owner_action(dynamic_import_terminal_prepared_action());

    assert_eq!(
        batch.len(),
        0,
        "dynamic import owner actions have their own task source, not module-script terminal payloads"
    );
    assert_eq!(batch.into_dynamic_import_owner_actions().len(), 1);
}

#[test]
fn module_terminal_batch_keeps_modulepreload_terminals_out_of_event_task_lane() {
    let mut batch = FrameDocumentModuleTerminalBatch::default();
    let task_owner =
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(7), LocalWindowId(8), DocumentId(9));
    let realm_id = FrameRealmId(10);

    batch.push_modulepreload_terminal_work(
        FrameDocumentModulepreloadTerminalWork::from_link_error_parts(
            realm_id,
            modulepreload_link_client(task_owner, DomHandle::new(12)),
        ),
    );

    assert_eq!(
        batch.len(),
        0,
        "modulepreload terminal work binds lifecycle state before entering its event task source"
    );
    assert_eq!(batch.into_modulepreload_terminal_works().len(), 1);
}

fn modulepreload_event_action() -> FrameDocumentModulepreloadEventAction {
    let task_owner =
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(7), LocalWindowId(8), DocumentId(9));
    let realm_id = FrameRealmId(10);
    FrameDocumentModulepreloadTerminalWork::from_link_error_parts(
        realm_id,
        modulepreload_link_client(task_owner, DomHandle::new(12)),
    )
    .into_event_action()
}

#[derive(Default)]
struct FakeModulepreloadEventActionHooks {
    fail_dispatch: bool,
    dispatch_calls: usize,
    failed_records: usize,
}

impl FrameDocumentModulepreloadEventActionHooks for FakeModulepreloadEventActionHooks {
    fn dispatch_modulepreload_event(
        &mut self,
        _owner: FrameDocumentTaskOwner,
        _realm_id: FrameRealmId,
        _link_handle: DomHandle,
        _successful: bool,
    ) -> Result<(), String> {
        self.dispatch_calls += 1;
        if self.fail_dispatch {
            Err("modulepreload dispatch failed".to_owned())
        } else {
            Ok(())
        }
    }

    fn record_modulepreload_event_dispatch_failed(
        &mut self,
        _action: &FrameDocumentModulepreloadEventAction,
        _error: &str,
    ) {
        self.failed_records += 1;
    }
}

#[test]
fn modulepreload_event_action_runner_reports_dispatched_event() {
    let mut runner = FrameDocumentModulepreloadEventActionRunner::new(
        FakeModulepreloadEventActionHooks::default(),
    );

    let outcome = runner.run_event_action(modulepreload_event_action());
    let hooks = runner.into_hooks();

    assert!(outcome.event_was_dispatched());
    assert!(!outcome.event_dispatch_was_failed());
    assert_eq!(hooks.dispatch_calls, 1);
    assert_eq!(hooks.failed_records, 0);
}

#[test]
fn modulepreload_event_action_runner_records_dispatch_failure() {
    let hooks = FakeModulepreloadEventActionHooks {
        fail_dispatch: true,
        ..FakeModulepreloadEventActionHooks::default()
    };
    let mut runner = FrameDocumentModulepreloadEventActionRunner::new(hooks);

    let outcome = runner.run_event_action(modulepreload_event_action());
    let hooks = runner.into_hooks();

    assert!(outcome.event_dispatch_was_failed());
    assert!(!outcome.event_was_dispatched());
    assert_eq!(hooks.dispatch_calls, 1);
    assert_eq!(hooks.failed_records, 1);
}

#[test]
fn module_script_terminal_followup_merges_fetch_and_ready_outcomes() {
    let mut followup = FrameDocumentModuleScriptTerminalFollowup::module_dependency_fetch_queued();
    followup.merge(FrameDocumentModuleScriptTerminalFollowup::document_script_ready_queued());
    followup.merge(FrameDocumentModuleScriptTerminalFollowup::module_script_wait_retained());

    assert!(followup.made_progress());
    assert!(followup.module_dependency_fetch_was_queued());
    assert!(followup.document_script_ready_was_queued());
    assert!(followup.module_script_wait_was_retained());
}

#[test]
fn dynamic_import_terminal_outcome_merges_resume_results() {
    let mut outcome = FrameDocumentDynamicImportTerminalOutcome::terminal_work_consumed();
    outcome.merge(FrameDocumentDynamicImportTerminalOutcome::owner_action_queued());
    outcome.merge(FrameDocumentDynamicImportTerminalOutcome::waiting_fetch_scheduled());
    outcome.merge(FrameDocumentDynamicImportTerminalOutcome::waiting_fetch_missing_loader());
    outcome.merge(FrameDocumentDynamicImportTerminalOutcome::source_import_resolved());
    outcome.merge(FrameDocumentDynamicImportTerminalOutcome::evaluation_import_resolved());
    outcome.merge(FrameDocumentDynamicImportTerminalOutcome::evaluation_import_pending());
    outcome.merge(FrameDocumentDynamicImportTerminalOutcome::dynamic_import_rejected());

    assert!(outcome.made_progress());
    assert!(outcome.owner_action_was_queued());
    assert!(outcome.waiting_fetch_was_scheduled());
    assert!(outcome.waiting_fetch_missing_loader_was_recorded());
    assert!(outcome.source_import_was_resolved());
    assert!(outcome.evaluation_import_was_continued());
    assert!(outcome.evaluation_import_was_resolved());
    assert!(outcome.evaluation_import_was_pending());
    assert!(outcome.dynamic_import_was_rejected());
}

#[test]
fn module_script_terminal_outcome_tracks_module_script_followups() {
    let mut outcome = FrameDocumentModuleScriptTerminalOutcome::consumed_terminal_batch();

    outcome.merge_module_script_terminal_followup(
        FrameDocumentModuleScriptTerminalFollowup::module_dependency_fetch_queued(),
    );
    outcome.merge_module_script_terminal_followup(
        FrameDocumentModuleScriptTerminalFollowup::document_script_ready_queued(),
    );
    outcome.merge_module_script_terminal_followup(
        FrameDocumentModuleScriptTerminalFollowup::module_script_wait_retained(),
    );

    assert!(outcome.made_progress());
    assert!(
        outcome
            .module_script_terminal_followup()
            .module_dependency_fetch_was_queued()
    );
    assert!(
        outcome
            .module_script_terminal_followup()
            .document_script_ready_was_queued()
    );
    assert!(
        outcome
            .module_script_terminal_followup()
            .module_script_wait_was_retained()
    );
}

fn dynamic_import_terminal_prepared_action() -> FrameDocumentDynamicImportTerminalPreparedAction {
    FrameDocumentDynamicImportTerminalPreparedAction::from_terminal_work(
        dynamic_import_terminal_work(),
    )
}

fn dynamic_import_terminal_work() -> FrameDocumentDynamicImportTerminalWork {
    let task_owner =
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(7), LocalWindowId(8), DocumentId(9));
    let realm_id = FrameRealmId(10);
    let key = ModuleMapKey::java_script(
        Url::parse("https://dynamic-import-terminal-runner.test/root.mjs")
            .expect("module url should parse"),
    );
    let client = NativeDynamicImportSingleModuleClient::new(
        module_tree::SingleModuleClientToken {
            tree_id: module_tree::ModuleTreeId(11),
            sequence: 12,
        },
        module_tree::ModuleImportPhase::Evaluation,
    );
    FrameDocumentDynamicImportTerminalWork::from_terminal_parts(task_owner, realm_id, key, client)
}

fn child_dynamic_scheduled_fetch(load_id: u64) -> DynamicModuleScheduledFetch {
    DynamicModuleScheduledFetch::new(load_id, dynamic_fetch_request("next.js"), None)
}

fn child_dynamic_owner_terminal_fetch(load_id: u64) -> DynamicModuleScheduledFetch {
    let owner = FrameDocumentOwner::new(LocalWindowId(8), DocumentId(9));
    let entry_id = FrameDocumentModuleClientEntryId::from_raw(24);
    let key = ModuleMapKey::java_script(
        Url::parse("https://dynamic-import-terminal-runner.test/already-fetched.mjs")
            .expect("module url should parse"),
    );
    let owner_start = owner_module_fetch_client_start_with_disposition(
        owner,
        key,
        FrameDocumentModuleFetchDisposition::AlreadyFetched(entry_id),
    );
    DynamicModuleScheduledFetch::new(
        load_id,
        dynamic_fetch_request("already-fetched.js"),
        Some(owner_start),
    )
}

fn dynamic_import_waiting_action(
    fetch_action: ChildDynamicModuleFetchAction,
) -> FrameDocumentDynamicImportOwnerAction {
    FrameDocumentDynamicImportOwnerAction::waiting(
        FrameDocumentOwner::new(LocalWindowId(8), DocumentId(9)),
        FrameRealmId(10),
        fetch_action,
    )
}

fn owner_module_fetch_client_start(
    owner: FrameDocumentOwner,
    key: ModuleMapKey,
) -> FrameDocumentModuleFetchClientStart {
    let entry_id = FrameDocumentModuleClientEntryId::from_raw(21);
    owner_module_fetch_client_start_with_disposition(
        owner,
        key,
        FrameDocumentModuleFetchDisposition::StartedFetch(entry_id),
    )
}

fn owner_module_fetch_client_start_with_disposition(
    owner: FrameDocumentOwner,
    key: ModuleMapKey,
    fetch_disposition: FrameDocumentModuleFetchDisposition,
) -> FrameDocumentModuleFetchClientStart {
    FrameDocumentModuleFetchClientStart::new(
        owner,
        FrameRequestId(22),
        FrameRequestKind::ModuleDependency,
        key,
        FrameDocumentModuleClientRegistration::new(
            fetch_disposition.entry_id(),
            FrameDocumentModuleClientId::from_raw(23),
            fetch_disposition,
        ),
    )
}

fn dynamic_import_owner_module_fetch_completion_action()
-> FrameDocumentDynamicImportTerminalPreparedAction {
    let task_owner =
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(7), LocalWindowId(8), DocumentId(9));
    let owner = task_owner.document_owner();
    let realm_id = FrameRealmId(10);
    let tree_client = module_tree::SingleModuleClientToken {
        tree_id: module_tree::ModuleTreeId(31),
        sequence: 32,
    };
    let source_url =
        Url::parse("https://child-module-graph.test/app/owner-module-fetch.mjs").unwrap();
    let key = ModuleMapKey::java_script(source_url.clone());
    let request = dynamic_tree_fetch_request("owner-module-fetch.mjs", tree_client);
    let owner_start = owner_module_fetch_client_start(owner, key);
    let mut store = ChildDocumentModulatorStore::default();
    let scheduled = store.suspend_dynamic_module_import_fetches(
        owner,
        realm_id,
        vec![request],
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
        source_url,
        false,
        ModuleSource::text("export const value = 1;".to_owned()),
    ));
    FrameDocumentDynamicImportTerminalPreparedAction::from_fetch_completion(
        task_owner,
        realm_id,
        load_id,
        FrameDocumentDynamicImportOwnerAction::OwnerModuleFetchCompleted {
            load_id,
            settle: ChildDynamicModuleOwnerFetchCompletionSettlementAction::new(
                owner_start,
                source,
            ),
            restore: ChildDynamicModuleCompletedFetchRestoreAction::new(owner, realm_id, inflight),
        },
    )
}

fn dynamic_import_ready_action() -> FrameDocumentDynamicImportOwnerAction {
    dynamic_import_ready_action_with_phase(ModuleImportPhase::Evaluation)
}

fn dynamic_import_ready_action_with_phase(
    phase: ModuleImportPhase,
) -> FrameDocumentDynamicImportOwnerAction {
    FrameDocumentDynamicImportOwnerAction::ready(NativeDynamicModuleImportReady {
        job: NativeModuleGraphJob::dynamic_import(pending_dynamic_module_import_with_phase(phase)),
        graph: ModuleGraphHandle {
            root_entry: ModuleEntryId::for_test(42),
            entries: vec![ModuleEntryId::for_test(42)],
        },
    })
}

fn dynamic_import_graph_failed_action() -> FrameDocumentDynamicImportOwnerAction {
    FrameDocumentDynamicImportOwnerAction::graph_advance_failed(
        NativeModuleGraphJob::dynamic_import(pending_dynamic_module_import()),
        ModuleLoadError::new(
            ModuleLoadStage::Resolve,
            "forced dynamic import graph failure",
        ),
    )
}

fn dynamic_import_failed_fetch_action() -> FrameDocumentDynamicImportOwnerAction {
    FrameDocumentDynamicImportOwnerAction::fetch_failed(
        pending_dynamic_module_import(),
        ModuleLoadError::new(
            ModuleLoadStage::Fetch,
            "forced dynamic import fetch failure",
        ),
    )
}

#[derive(Clone, Copy)]
enum FakeDynamicImportTerminalFinish {
    MissingJoinedClient,
    FollowupAction,
    UnexpectedCompleteWarning,
}

#[test]
fn dynamic_import_owner_action_runner_records_missing_terminal_client() {
    let hooks = FakeDynamicImportOwnerActionHooks {
        terminal_finish: FakeDynamicImportTerminalFinish::MissingJoinedClient,
        ..FakeDynamicImportOwnerActionHooks::default()
    };
    let mut runner = FrameDocumentDynamicImportOwnerActionRunner::new(hooks);

    let outcome = runner.run_prepared_action(dynamic_import_terminal_prepared_action());
    let hooks = runner.into_hooks();

    assert!(outcome.terminal_work_was_consumed());
    assert!(outcome.missing_joined_client_was_recorded());
    assert_eq!(hooks.terminal_client_finish_calls, 1);
    assert_eq!(hooks.missing_joined_terminal_client_records, 1);
    let missing = hooks
        .last_missing_joined_terminal_client
        .expect("missing joined terminal client record");
    assert_eq!(
        missing.owner(),
        FrameDocumentOwner::new(LocalWindowId(8), DocumentId(9))
    );
    assert_eq!(missing.realm_id(), FrameRealmId(10));
    assert_eq!(
        missing.tree_client(),
        module_tree::SingleModuleClientToken {
            tree_id: module_tree::ModuleTreeId(11),
            sequence: 12
        }
    );
    assert_eq!(hooks.queued_owner_actions, 0);
    assert_eq!(hooks.unexpected_complete_warnings, 0);
    assert_eq!(hooks.resumed_records, 1);
    assert_eq!(hooks.failed_records, 0);
    assert_eq!(
        hooks.last_resumed_diagnostic,
        Some(
            FrameDocumentDynamicImportOwnerActionDiagnostic::TerminalClient {
                owner: FrameDocumentOwner::new(LocalWindowId(8), DocumentId(9)),
                realm_id: FrameRealmId(10),
                tree_client: module_tree::SingleModuleClientToken {
                    tree_id: module_tree::ModuleTreeId(11),
                    sequence: 12
                },
                import_phase: module_tree::ModuleImportPhase::Evaluation,
                url: Url::parse("https://dynamic-import-terminal-runner.test/root.mjs")
                    .expect("module url should parse"),
            }
        )
    );
}

#[test]
fn dynamic_import_owner_action_runner_materializes_terminal_client_action() {
    let mut runner = FrameDocumentDynamicImportOwnerActionRunner::new(
        FakeDynamicImportOwnerActionHooks::default(),
    );

    let outcome = runner.run_prepared_action(dynamic_import_terminal_prepared_action());
    let hooks = runner.into_hooks();

    assert!(outcome.terminal_work_was_consumed());
    assert!(outcome.owner_action_was_queued());
    assert!(!outcome.source_import_was_resolved());
    assert!(!outcome.evaluation_import_was_continued());
    assert_eq!(hooks.terminal_client_finish_calls, 1);
    assert_eq!(hooks.queued_owner_actions, 1);
    assert_eq!(hooks.unexpected_complete_warnings, 0);
    assert_eq!(hooks.resumed_records, 1);
    assert_eq!(hooks.failed_records, 0);
}

#[test]
fn dynamic_import_owner_action_runner_uses_terminal_followup_queue_result() {
    let hooks = FakeDynamicImportOwnerActionHooks {
        terminal_client_followup: FrameDocumentModuleTerminalQueueFollowup::none(),
        ..FakeDynamicImportOwnerActionHooks::default()
    };
    let mut runner = FrameDocumentDynamicImportOwnerActionRunner::new(hooks);

    let outcome = runner.run_prepared_action(dynamic_import_terminal_prepared_action());
    let hooks = runner.into_hooks();

    assert!(outcome.terminal_work_was_consumed());
    assert!(!outcome.owner_action_was_queued());
    assert_eq!(hooks.terminal_client_finish_calls, 1);
    assert_eq!(hooks.queued_owner_actions, 1);
    assert_eq!(hooks.resumed_records, 1);
    assert_eq!(hooks.failed_records, 0);
}

#[test]
fn dynamic_import_owner_action_runner_preserves_terminal_followup_progress() {
    let mut terminal_client_followup =
        FrameDocumentModuleTerminalQueueFollowup::dynamic_import_wait_retained();
    terminal_client_followup
        .merge(FrameDocumentModuleTerminalQueueFollowup::dynamic_import_job_resumed());
    let hooks = FakeDynamicImportOwnerActionHooks {
        terminal_client_followup,
        ..FakeDynamicImportOwnerActionHooks::default()
    };
    let mut runner = FrameDocumentDynamicImportOwnerActionRunner::new(hooks);

    let outcome = runner.run_prepared_action(dynamic_import_terminal_prepared_action());
    let hooks = runner.into_hooks();

    assert!(outcome.terminal_work_was_consumed());
    assert!(!outcome.owner_action_was_queued());
    assert!(outcome.dynamic_import_wait_was_retained());
    assert!(outcome.dynamic_import_job_was_resumed());
    assert_eq!(hooks.terminal_client_finish_calls, 1);
    assert_eq!(hooks.queued_owner_actions, 1);
    assert_eq!(hooks.resumed_records, 1);
    assert_eq!(hooks.failed_records, 0);
}

struct FakeDynamicImportOwnerActionQueueHooks {
    task_owner: Option<FrameDocumentTaskOwner>,
    queued_owner_actions: usize,
    stale_records: usize,
}

impl FrameDocumentDynamicImportOwnerActionQueueHooks for FakeDynamicImportOwnerActionQueueHooks {
    fn current_dynamic_import_task_owner(
        &mut self,
        _owner: FrameDocumentOwner,
        _realm_id: FrameRealmId,
    ) -> FrameDocumentDynamicImportQueueTaskOwnerResult {
        self.task_owner
            .map(FrameDocumentDynamicImportQueueTaskOwnerResult::Current)
            .unwrap_or(FrameDocumentDynamicImportQueueTaskOwnerResult::Stale)
    }

    fn queue_dynamic_import_owner_actions(
        &mut self,
        actions: Vec<FrameDocumentDynamicImportTerminalPreparedAction>,
    ) -> FrameDocumentModuleTerminalQueueFollowup {
        self.queued_owner_actions += actions.len();
        FrameDocumentModuleTerminalQueueFollowup::dynamic_import_owner_action_queued()
    }

    fn record_stale_dynamic_import_owner_action(
        &mut self,
        _trace: FrameDocumentDynamicImportOwnerActionQueueTrace,
    ) {
        self.stale_records += 1;
    }
}

impl Default for FakeDynamicImportOwnerActionQueueHooks {
    fn default() -> Self {
        Self {
            task_owner: Some(FrameDocumentTaskOwner::new(
                FrameSchedulerLaneId(7),
                LocalWindowId(8),
                DocumentId(9),
            )),
            queued_owner_actions: 0,
            stale_records: 0,
        }
    }
}

#[test]
fn dynamic_import_owner_action_queue_runner_queues_waiting_fetches() {
    let mut runner = FrameDocumentDynamicImportOwnerActionQueueRunner::new(
        FakeDynamicImportOwnerActionQueueHooks::default(),
    );

    let outcome =
        runner.run_queue_request(FrameDocumentDynamicImportOwnerActionQueueRequest::waiting(
            FrameDocumentOwner::new(LocalWindowId(8), DocumentId(9)),
            FrameRealmId(10),
            vec![
                child_dynamic_scheduled_fetch(61).into(),
                child_dynamic_scheduled_fetch(62).into(),
            ],
        ));
    let hooks = runner.into_hooks();

    assert!(outcome.dynamic_import_owner_action_was_queued());
    assert!(!outcome.stale_owner_was_recorded());
    assert_eq!(hooks.queued_owner_actions, 2);
    assert_eq!(hooks.stale_records, 0);
}

#[test]
fn dynamic_import_owner_action_queue_runner_records_stale_owner() {
    let hooks = FakeDynamicImportOwnerActionQueueHooks {
        task_owner: None,
        ..FakeDynamicImportOwnerActionQueueHooks::default()
    };
    let mut runner = FrameDocumentDynamicImportOwnerActionQueueRunner::new(hooks);

    let outcome = runner.run_queue_request(
        FrameDocumentDynamicImportOwnerActionQueueRequest::continuation(
            FrameDocumentTaskOwner::new(FrameSchedulerLaneId(7), LocalWindowId(8), DocumentId(9))
                .document_owner(),
            FrameRealmId(10),
            dynamic_import_ready_action(),
        ),
    );
    let hooks = runner.into_hooks();

    assert!(outcome.stale_owner_was_recorded());
    assert!(!outcome.dynamic_import_owner_action_was_queued());
    assert_eq!(hooks.stale_records, 1);
    assert_eq!(hooks.queued_owner_actions, 0);
}

#[test]
fn dynamic_import_owner_action_queue_runner_reports_empty_waiting_fetches() {
    let mut runner = FrameDocumentDynamicImportOwnerActionQueueRunner::new(
        FakeDynamicImportOwnerActionQueueHooks::default(),
    );

    let outcome =
        runner.run_queue_request(FrameDocumentDynamicImportOwnerActionQueueRequest::waiting(
            FrameDocumentOwner::new(LocalWindowId(8), DocumentId(9)),
            FrameRealmId(10),
            Vec::new(),
        ));
    let hooks = runner.into_hooks();

    assert!(outcome.dynamic_import_wait_was_retained());
    assert!(!outcome.dynamic_import_owner_action_was_queued());
    assert!(!outcome.stale_owner_was_recorded());
    assert_eq!(hooks.queued_owner_actions, 0);
    assert_eq!(hooks.stale_records, 0);
}

struct FakeDynamicImportOwnerActionHooks {
    terminal_finish: FakeDynamicImportTerminalFinish,
    fail_unexpected_complete_warning: bool,
    owner_module_fetch_completion_settle_result: bool,
    completed_owner_fetch_restore_result: bool,
    finish_without_network_settle_result: bool,
    scheduled_fetch_restore_result: bool,
    source_ready_result: FrameDocumentDynamicImportSourceReadyResult,
    evaluation_ready_result: FrameDocumentDynamicImportEvaluationReadyResult,
    reject_result: FrameDocumentDynamicImportRejectResult,
    waiting_fetch_schedule_result: FrameDocumentDynamicImportWaitingFetchScheduleResult,
    terminal_client_followup: FrameDocumentModuleTerminalQueueFollowup,
    terminal_client_finish_calls: usize,
    owner_module_fetch_completion_settles: usize,
    completed_owner_fetch_restores: usize,
    finish_without_network_calls: usize,
    restored_scheduled_fetch_calls: usize,
    missing_joined_terminal_client_records: usize,
    last_missing_joined_terminal_client:
        Option<FrameDocumentDynamicImportMissingJoinedTerminalClient>,
    missing_joined_terminal_fetch_records: usize,
    queued_owner_actions: usize,
    scheduled_fetch_calls: usize,
    last_scheduled_fetch_target: Option<(FrameDocumentOwner, FrameRealmId)>,
    source_import_calls: usize,
    evaluation_import_calls: usize,
    unexpected_complete_warnings: usize,
    last_unexpected_complete_warning: Option<FrameDocumentDynamicImportOwnerActionDiagnostic>,
    dynamic_import_rejection_calls: usize,
    last_restored_terminal_fetch_target: Option<(FrameDocumentOwner, FrameRealmId)>,
    resumed_records: usize,
    last_resumed_diagnostic: Option<FrameDocumentDynamicImportOwnerActionDiagnostic>,
    failed_records: usize,
    last_failed_diagnostic: Option<FrameDocumentDynamicImportOwnerActionDiagnostic>,
}

impl FrameDocumentDynamicImportOwnerActionHooks for FakeDynamicImportOwnerActionHooks {
    fn finish_terminal_client(
        &mut self,
        _action: FrameDocumentDynamicImportTerminalClientAction,
    ) -> Result<FrameDocumentDynamicImportTerminalClientFinishResult, String> {
        self.terminal_client_finish_calls += 1;
        let result = match self.terminal_finish {
            FakeDynamicImportTerminalFinish::MissingJoinedClient => {
                FrameDocumentDynamicImportTerminalClientFinishResult::MissingJoinedClient
            }
            FakeDynamicImportTerminalFinish::FollowupAction => {
                FrameDocumentDynamicImportTerminalClientFinishResult::followup_action(
                    dynamic_import_ready_action(),
                )
            }
            FakeDynamicImportTerminalFinish::UnexpectedCompleteWarning => {
                FrameDocumentDynamicImportTerminalClientFinishResult::RestoredAfterUnexpectedComplete
            }
        };
        Ok(result)
    }

    fn queue_owner_action_followups(
        &mut self,
        actions: Vec<FrameDocumentDynamicImportTerminalPreparedAction>,
    ) -> Result<FrameDocumentModuleTerminalQueueFollowup, String> {
        self.queued_owner_actions += actions.len();
        Ok(self.terminal_client_followup)
    }

    fn record_missing_joined_terminal_client(
        &mut self,
        missing: FrameDocumentDynamicImportMissingJoinedTerminalClient,
    ) -> Result<(), String> {
        self.missing_joined_terminal_client_records += 1;
        self.last_missing_joined_terminal_client = Some(missing);
        Ok(())
    }

    fn settle_owner_module_fetch_completion(
        &mut self,
        _action: ChildDynamicModuleOwnerFetchCompletionSettlementAction,
    ) -> Result<FrameDocumentDynamicImportOwnerFetchSettlementResult, String> {
        self.owner_module_fetch_completion_settles += 1;
        Ok(
            FrameDocumentDynamicImportOwnerFetchSettlementResult::from_settled(
                self.owner_module_fetch_completion_settle_result,
            ),
        )
    }

    fn restore_completed_owner_module_fetch_as_joined_terminal_client(
        &mut self,
        _restore: ChildDynamicModuleCompletedFetchRestoreAction,
    ) -> Result<FrameDocumentDynamicImportJoinedFetchRestoreResult, String> {
        self.completed_owner_fetch_restores += 1;
        Ok(
            FrameDocumentDynamicImportJoinedFetchRestoreResult::from_restored(
                self.completed_owner_fetch_restore_result,
            ),
        )
    }

    fn finish_owner_module_fetch_without_network(
        &mut self,
        _action: ChildDynamicModuleOwnerFetchWithoutNetworkSettlementAction,
    ) -> Result<FrameDocumentDynamicImportOwnerFetchSettlementResult, String> {
        self.finish_without_network_calls += 1;
        Ok(
            FrameDocumentDynamicImportOwnerFetchSettlementResult::from_settled(
                self.finish_without_network_settle_result,
            ),
        )
    }

    fn restore_scheduled_fetch_as_joined_terminal_client(
        &mut self,
        action: FrameDocumentDynamicImportOwnerTerminalRestoreAction,
    ) -> Result<FrameDocumentDynamicImportJoinedFetchRestoreResult, String> {
        self.restored_scheduled_fetch_calls += 1;
        self.last_restored_terminal_fetch_target = Some((action.owner(), action.realm_id()));
        Ok(
            FrameDocumentDynamicImportJoinedFetchRestoreResult::from_restored(
                self.scheduled_fetch_restore_result,
            ),
        )
    }

    fn schedule_waiting_fetch(
        &mut self,
        action: FrameDocumentDynamicImportWaitingFetchScheduleAction,
    ) -> Result<FrameDocumentDynamicImportWaitingFetchScheduleResult, String> {
        self.scheduled_fetch_calls += 1;
        self.last_scheduled_fetch_target = Some((action.owner(), action.realm_id()));
        Ok(self.waiting_fetch_schedule_result)
    }

    fn record_missing_joined_terminal_fetch(
        &mut self,
        _missing: FrameDocumentDynamicImportMissingJoinedTerminalFetch,
    ) -> Result<(), String> {
        self.missing_joined_terminal_fetch_records += 1;
        Ok(())
    }

    fn resolve_ready_source_import(
        &mut self,
        _action: FrameDocumentDynamicImportSourceReadyAction,
    ) -> Result<FrameDocumentDynamicImportSourceReadyResult, String> {
        self.source_import_calls += 1;
        Ok(self.source_ready_result)
    }

    fn continue_ready_evaluation_import(
        &mut self,
        _action: FrameDocumentDynamicImportEvaluationReadyAction,
    ) -> Result<FrameDocumentDynamicImportEvaluationReadyResult, String> {
        self.evaluation_import_calls += 1;
        Ok(self.evaluation_ready_result)
    }

    fn record_restored_after_unexpected_complete(
        &mut self,
        diagnostic: FrameDocumentDynamicImportOwnerActionDiagnostic,
    ) -> Result<(), String> {
        self.unexpected_complete_warnings += 1;
        self.last_unexpected_complete_warning = Some(diagnostic);
        if self.fail_unexpected_complete_warning {
            Err("unexpected complete warning failed".to_owned())
        } else {
            Ok(())
        }
    }

    fn reject_dynamic_import(
        &mut self,
        _action: FrameDocumentDynamicImportRejectAction,
    ) -> Result<FrameDocumentDynamicImportRejectResult, String> {
        self.dynamic_import_rejection_calls += 1;
        Ok(self.reject_result)
    }

    fn record_action_resumed(
        &mut self,
        diagnostic: FrameDocumentDynamicImportOwnerActionDiagnostic,
    ) {
        self.resumed_records += 1;
        self.last_resumed_diagnostic = Some(diagnostic);
    }

    fn record_action_failed(
        &mut self,
        diagnostic: FrameDocumentDynamicImportOwnerActionDiagnostic,
        _error: &str,
    ) {
        self.failed_records += 1;
        self.last_failed_diagnostic = Some(diagnostic);
    }
}

impl Default for FakeDynamicImportOwnerActionHooks {
    fn default() -> Self {
        Self {
            terminal_finish: FakeDynamicImportTerminalFinish::FollowupAction,
            owner_module_fetch_completion_settle_result: true,
            completed_owner_fetch_restore_result: true,
            finish_without_network_settle_result: true,
            scheduled_fetch_restore_result: true,
            source_ready_result: FrameDocumentDynamicImportSourceReadyResult::Resolved,
            evaluation_ready_result: FrameDocumentDynamicImportEvaluationReadyResult::Pending,
            reject_result: FrameDocumentDynamicImportRejectResult::Rejected,
            waiting_fetch_schedule_result:
                FrameDocumentDynamicImportWaitingFetchScheduleResult::Scheduled,
            terminal_client_followup:
                FrameDocumentModuleTerminalQueueFollowup::dynamic_import_owner_action_queued(),
            fail_unexpected_complete_warning: false,
            terminal_client_finish_calls: 0,
            owner_module_fetch_completion_settles: 0,
            completed_owner_fetch_restores: 0,
            finish_without_network_calls: 0,
            restored_scheduled_fetch_calls: 0,
            missing_joined_terminal_client_records: 0,
            last_missing_joined_terminal_client: None,
            missing_joined_terminal_fetch_records: 0,
            queued_owner_actions: 0,
            scheduled_fetch_calls: 0,
            last_scheduled_fetch_target: None,
            source_import_calls: 0,
            evaluation_import_calls: 0,
            unexpected_complete_warnings: 0,
            last_unexpected_complete_warning: None,
            dynamic_import_rejection_calls: 0,
            last_restored_terminal_fetch_target: None,
            resumed_records: 0,
            last_resumed_diagnostic: None,
            failed_records: 0,
            last_failed_diagnostic: None,
        }
    }
}

#[test]
fn dynamic_import_owner_action_runner_records_unexpected_complete_terminal_warning() {
    let hooks = FakeDynamicImportOwnerActionHooks {
        terminal_finish: FakeDynamicImportTerminalFinish::UnexpectedCompleteWarning,
        ..FakeDynamicImportOwnerActionHooks::default()
    };
    let mut action_runner = FrameDocumentDynamicImportOwnerActionRunner::new(hooks);

    let outcome = action_runner.run_prepared_action(dynamic_import_terminal_prepared_action());
    let hooks = action_runner.into_hooks();

    assert!(outcome.terminal_work_was_consumed());
    assert!(outcome.unexpected_complete_was_recorded());
    assert_eq!(hooks.unexpected_complete_warnings, 1);
    assert_eq!(hooks.queued_owner_actions, 0);
    assert_eq!(hooks.resumed_records, 1);
    assert_eq!(hooks.failed_records, 0);
    assert_eq!(
        hooks.last_unexpected_complete_warning,
        Some(
            FrameDocumentDynamicImportOwnerActionDiagnostic::TerminalClient {
                owner: FrameDocumentOwner::new(LocalWindowId(8), DocumentId(9)),
                realm_id: FrameRealmId(10),
                tree_client: module_tree::SingleModuleClientToken {
                    tree_id: module_tree::ModuleTreeId(11),
                    sequence: 12
                },
                import_phase: module_tree::ModuleImportPhase::Evaluation,
                url: Url::parse("https://dynamic-import-terminal-runner.test/root.mjs")
                    .expect("module url should parse"),
            }
        )
    );
}

#[test]
fn dynamic_import_owner_action_runner_restores_completed_owner_module_fetch() {
    let owner_action = dynamic_import_owner_module_fetch_completion_action();
    let mut action_runner = FrameDocumentDynamicImportOwnerActionRunner::new(
        FakeDynamicImportOwnerActionHooks::default(),
    );

    let outcome = action_runner.run_prepared_action(owner_action);
    let hooks = action_runner.into_hooks();

    assert!(outcome.owner_module_fetch_was_settled());
    assert!(outcome.owner_module_fetch_was_restored());
    assert_eq!(hooks.owner_module_fetch_completion_settles, 1);
    assert_eq!(hooks.completed_owner_fetch_restores, 1);
    assert_eq!(hooks.missing_joined_terminal_fetch_records, 0);
    assert_eq!(hooks.resumed_records, 1);
    assert_eq!(hooks.failed_records, 0);
}

#[test]
fn dynamic_import_owner_action_runner_preserves_missing_owner_module_fetch_settlement() {
    let owner_action = dynamic_import_owner_module_fetch_completion_action();
    let hooks = FakeDynamicImportOwnerActionHooks {
        owner_module_fetch_completion_settle_result: false,
        ..FakeDynamicImportOwnerActionHooks::default()
    };
    let mut action_runner = FrameDocumentDynamicImportOwnerActionRunner::new(hooks);

    let outcome = action_runner.run_prepared_action(owner_action);
    let hooks = action_runner.into_hooks();

    assert!(!outcome.owner_module_fetch_was_settled());
    assert!(outcome.owner_module_fetch_was_restored());
    assert_eq!(hooks.owner_module_fetch_completion_settles, 1);
    assert_eq!(hooks.completed_owner_fetch_restores, 1);
    assert_eq!(hooks.missing_joined_terminal_fetch_records, 0);
    assert_eq!(hooks.resumed_records, 1);
    assert_eq!(hooks.failed_records, 0);
}

#[test]
fn dynamic_import_owner_action_runner_records_missing_completed_owner_module_fetch_restore() {
    let owner_action = dynamic_import_owner_module_fetch_completion_action();
    let hooks = FakeDynamicImportOwnerActionHooks {
        completed_owner_fetch_restore_result: false,
        ..FakeDynamicImportOwnerActionHooks::default()
    };
    let mut action_runner = FrameDocumentDynamicImportOwnerActionRunner::new(hooks);

    let outcome = action_runner.run_prepared_action(owner_action);
    let hooks = action_runner.into_hooks();

    assert!(outcome.owner_module_fetch_was_settled());
    assert!(outcome.missing_joined_terminal_fetch_was_recorded());
    assert_eq!(hooks.owner_module_fetch_completion_settles, 1);
    assert_eq!(hooks.completed_owner_fetch_restores, 1);
    assert_eq!(hooks.missing_joined_terminal_fetch_records, 1);
    assert_eq!(hooks.resumed_records, 1);
    assert_eq!(hooks.failed_records, 0);
}

#[test]
fn dynamic_import_owner_action_runner_records_action_failure() {
    let hooks = FakeDynamicImportOwnerActionHooks {
        terminal_finish: FakeDynamicImportTerminalFinish::UnexpectedCompleteWarning,
        fail_unexpected_complete_warning: true,
        ..FakeDynamicImportOwnerActionHooks::default()
    };
    let mut action_runner = FrameDocumentDynamicImportOwnerActionRunner::new(hooks);

    let outcome = action_runner.run_prepared_action(dynamic_import_terminal_prepared_action());
    let hooks = action_runner.into_hooks();

    assert!(outcome.resume_failure_was_recorded());
    assert_eq!(hooks.terminal_client_finish_calls, 1);
    assert_eq!(hooks.unexpected_complete_warnings, 1);
    assert_eq!(hooks.queued_owner_actions, 0);
    assert_eq!(hooks.resumed_records, 0);
    assert_eq!(hooks.failed_records, 1);
    assert_eq!(
        hooks.last_failed_diagnostic,
        Some(
            FrameDocumentDynamicImportOwnerActionDiagnostic::TerminalClient {
                owner: FrameDocumentOwner::new(LocalWindowId(8), DocumentId(9)),
                realm_id: FrameRealmId(10),
                tree_client: module_tree::SingleModuleClientToken {
                    tree_id: module_tree::ModuleTreeId(11),
                    sequence: 12
                },
                import_phase: module_tree::ModuleImportPhase::Evaluation,
                url: Url::parse("https://dynamic-import-terminal-runner.test/root.mjs")
                    .expect("module url should parse"),
            }
        )
    );
}

#[test]
fn dynamic_import_owner_action_runner_executes_exactly_one_waiting_fetch() {
    let action = dynamic_import_waiting_action(child_dynamic_scheduled_fetch(41).into());
    let mut action_runner = FrameDocumentDynamicImportOwnerActionRunner::new(
        FakeDynamicImportOwnerActionHooks::default(),
    );

    let outcome = action_runner
        .run_owner_action(action)
        .expect("waiting fetch action should run");
    let hooks = action_runner.into_hooks();

    assert!(outcome.waiting_fetch_was_scheduled());
    assert_eq!(hooks.scheduled_fetch_calls, 1);
    assert_eq!(
        hooks.last_scheduled_fetch_target,
        Some((
            FrameDocumentOwner::new(LocalWindowId(8), DocumentId(9)),
            FrameRealmId(10)
        ))
    );
    assert_eq!(hooks.finish_without_network_calls, 0);
    assert_eq!(hooks.restored_scheduled_fetch_calls, 0);
    assert_eq!(hooks.missing_joined_terminal_fetch_records, 0);
}

#[test]
fn dynamic_import_owner_action_runner_reports_missing_waiting_fetch_loader() {
    let action = dynamic_import_waiting_action(child_dynamic_scheduled_fetch(41).into());
    let hooks = FakeDynamicImportOwnerActionHooks {
        waiting_fetch_schedule_result:
            FrameDocumentDynamicImportWaitingFetchScheduleResult::MissingLoader,
        ..FakeDynamicImportOwnerActionHooks::default()
    };
    let mut action_runner = FrameDocumentDynamicImportOwnerActionRunner::new(hooks);

    let outcome = action_runner
        .run_owner_action(action)
        .expect("missing loader is a schedule outcome, not an owner-action failure");
    let hooks = action_runner.into_hooks();

    assert!(!outcome.waiting_fetch_was_scheduled());
    assert!(outcome.waiting_fetch_missing_loader_was_recorded());
    assert!(!outcome.resume_failure_was_recorded());
    assert_eq!(hooks.scheduled_fetch_calls, 1);
    assert_eq!(
        hooks.last_scheduled_fetch_target,
        Some((
            FrameDocumentOwner::new(LocalWindowId(8), DocumentId(9)),
            FrameRealmId(10)
        ))
    );
    assert_eq!(hooks.failed_records, 0);
}

#[test]
fn dynamic_import_owner_action_runner_drops_waiting_fetch_with_stale_producer() {
    let action = dynamic_import_waiting_action(child_dynamic_scheduled_fetch(42).into());
    let hooks = FakeDynamicImportOwnerActionHooks {
        waiting_fetch_schedule_result:
            FrameDocumentDynamicImportWaitingFetchScheduleResult::StaleOwner,
        ..FakeDynamicImportOwnerActionHooks::default()
    };
    let mut action_runner = FrameDocumentDynamicImportOwnerActionRunner::new(hooks);

    let outcome = action_runner
        .run_owner_action(action)
        .expect("a stale producer is a bounded owner-action outcome");
    let hooks = action_runner.into_hooks();

    assert!(!outcome.waiting_fetch_was_scheduled());
    assert!(outcome.stale_owner_was_dropped());
    assert!(!outcome.resume_failure_was_recorded());
    assert_eq!(hooks.scheduled_fetch_calls, 1);
    assert_eq!(hooks.failed_records, 0);
}

#[test]
fn dynamic_import_owner_action_runner_restores_owner_terminal_fetch_actions() {
    let action = dynamic_import_waiting_action(ChildDynamicModuleFetchAction::from(
        child_dynamic_owner_terminal_fetch(43),
    ));
    let mut action_runner = FrameDocumentDynamicImportOwnerActionRunner::new(
        FakeDynamicImportOwnerActionHooks::default(),
    );

    let outcome = action_runner
        .run_owner_action(action)
        .expect("owner terminal fetch action should run");
    let hooks = action_runner.into_hooks();

    assert!(outcome.owner_module_fetch_was_settled());
    assert!(outcome.owner_module_fetch_was_restored());
    assert_eq!(hooks.scheduled_fetch_calls, 0);
    assert_eq!(hooks.finish_without_network_calls, 1);
    assert_eq!(hooks.restored_scheduled_fetch_calls, 1);
    assert_eq!(
        hooks.last_restored_terminal_fetch_target,
        Some((
            FrameDocumentOwner::new(LocalWindowId(8), DocumentId(9)),
            FrameRealmId(10)
        ))
    );
    assert_eq!(hooks.missing_joined_terminal_fetch_records, 0);
}

#[test]
fn dynamic_import_owner_action_runner_preserves_missing_owner_terminal_without_network_settlement()
{
    let action = dynamic_import_waiting_action(ChildDynamicModuleFetchAction::from(
        child_dynamic_owner_terminal_fetch(43),
    ));
    let hooks = FakeDynamicImportOwnerActionHooks {
        finish_without_network_settle_result: false,
        ..FakeDynamicImportOwnerActionHooks::default()
    };
    let mut action_runner = FrameDocumentDynamicImportOwnerActionRunner::new(hooks);

    let outcome = action_runner
        .run_owner_action(action)
        .expect("owner terminal fetch action should run");
    let hooks = action_runner.into_hooks();

    assert!(!outcome.owner_module_fetch_was_settled());
    assert!(outcome.owner_module_fetch_was_restored());
    assert_eq!(hooks.scheduled_fetch_calls, 0);
    assert_eq!(hooks.finish_without_network_calls, 1);
    assert_eq!(hooks.restored_scheduled_fetch_calls, 1);
    assert_eq!(hooks.missing_joined_terminal_fetch_records, 0);
}

#[test]
fn dynamic_import_owner_action_runner_records_missing_scheduled_terminal_fetch_restore() {
    let action = dynamic_import_waiting_action(ChildDynamicModuleFetchAction::from(
        child_dynamic_owner_terminal_fetch(43),
    ));
    let hooks = FakeDynamicImportOwnerActionHooks {
        scheduled_fetch_restore_result: false,
        ..FakeDynamicImportOwnerActionHooks::default()
    };
    let mut action_runner = FrameDocumentDynamicImportOwnerActionRunner::new(hooks);

    let outcome = action_runner
        .run_owner_action(action)
        .expect("owner terminal fetch action should run");
    let hooks = action_runner.into_hooks();

    assert!(outcome.owner_module_fetch_was_settled());
    assert!(!outcome.owner_module_fetch_was_restored());
    assert!(outcome.missing_joined_terminal_fetch_was_recorded());
    assert_eq!(hooks.finish_without_network_calls, 1);
    assert_eq!(hooks.restored_scheduled_fetch_calls, 1);
    assert_eq!(hooks.missing_joined_terminal_fetch_records, 1);
}

#[test]
fn dynamic_import_owner_action_runner_reports_ready_import_continuation() {
    let mut action_runner = FrameDocumentDynamicImportOwnerActionRunner::new(
        FakeDynamicImportOwnerActionHooks::default(),
    );

    let outcome = action_runner
        .run_owner_action(dynamic_import_ready_action())
        .expect("ready dynamic import should continue");
    let hooks = action_runner.into_hooks();

    assert!(outcome.evaluation_import_was_continued());
    assert!(outcome.evaluation_import_was_pending());
    assert!(!outcome.evaluation_import_was_resolved());
    assert!(!outcome.dynamic_import_was_rejected());
    assert!(!outcome.source_import_was_resolved());
    assert_eq!(hooks.evaluation_import_calls, 1);
    assert_eq!(hooks.source_import_calls, 0);
    assert_eq!(hooks.dynamic_import_rejection_calls, 0);
}

#[test]
fn dynamic_import_owner_action_runner_reports_ready_import_resolution() {
    let hooks = FakeDynamicImportOwnerActionHooks {
        evaluation_ready_result: FrameDocumentDynamicImportEvaluationReadyResult::Resolved,
        ..FakeDynamicImportOwnerActionHooks::default()
    };
    let mut action_runner = FrameDocumentDynamicImportOwnerActionRunner::new(hooks);

    let outcome = action_runner
        .run_owner_action(dynamic_import_ready_action())
        .expect("ready dynamic import should resolve");
    let hooks = action_runner.into_hooks();

    assert!(outcome.evaluation_import_was_continued());
    assert!(outcome.evaluation_import_was_resolved());
    assert!(!outcome.evaluation_import_was_pending());
    assert!(!outcome.dynamic_import_was_rejected());
    assert_eq!(hooks.evaluation_import_calls, 1);
    assert_eq!(hooks.source_import_calls, 0);
    assert_eq!(hooks.dynamic_import_rejection_calls, 0);
}

#[test]
fn dynamic_import_owner_action_runner_reports_ready_import_rejection() {
    let hooks = FakeDynamicImportOwnerActionHooks {
        evaluation_ready_result: FrameDocumentDynamicImportEvaluationReadyResult::Rejected,
        ..FakeDynamicImportOwnerActionHooks::default()
    };
    let mut action_runner = FrameDocumentDynamicImportOwnerActionRunner::new(hooks);

    let outcome = action_runner
        .run_owner_action(dynamic_import_ready_action())
        .expect("ready dynamic import should reject");
    let hooks = action_runner.into_hooks();

    assert!(!outcome.evaluation_import_was_continued());
    assert!(!outcome.evaluation_import_was_resolved());
    assert!(!outcome.evaluation_import_was_pending());
    assert!(outcome.dynamic_import_was_rejected());
    assert_eq!(hooks.evaluation_import_calls, 1);
    assert_eq!(hooks.source_import_calls, 0);
    assert_eq!(hooks.dynamic_import_rejection_calls, 0);
}

#[test]
fn dynamic_import_owner_action_runner_reports_source_import_resolution() {
    let mut action_runner = FrameDocumentDynamicImportOwnerActionRunner::new(
        FakeDynamicImportOwnerActionHooks::default(),
    );

    let outcome = action_runner
        .run_owner_action(dynamic_import_ready_action_with_phase(
            ModuleImportPhase::Source,
        ))
        .expect("source-ready dynamic import should resolve source import");
    let hooks = action_runner.into_hooks();

    assert!(outcome.source_import_was_resolved());
    assert!(!outcome.evaluation_import_was_continued());
    assert_eq!(hooks.source_import_calls, 1);
    assert_eq!(hooks.evaluation_import_calls, 0);
    assert_eq!(hooks.dynamic_import_rejection_calls, 0);
}

#[test]
fn dynamic_import_owner_action_runner_reports_source_import_rejection() {
    let hooks = FakeDynamicImportOwnerActionHooks {
        source_ready_result: FrameDocumentDynamicImportSourceReadyResult::Rejected,
        ..FakeDynamicImportOwnerActionHooks::default()
    };
    let mut action_runner = FrameDocumentDynamicImportOwnerActionRunner::new(hooks);

    let outcome = action_runner
        .run_owner_action(dynamic_import_ready_action_with_phase(
            ModuleImportPhase::Source,
        ))
        .expect("source-ready dynamic import should reject source import");
    let hooks = action_runner.into_hooks();

    assert!(!outcome.source_import_was_resolved());
    assert!(outcome.dynamic_import_was_rejected());
    assert!(!outcome.evaluation_import_was_continued());
    assert_eq!(hooks.source_import_calls, 1);
    assert_eq!(hooks.evaluation_import_calls, 0);
    assert_eq!(hooks.dynamic_import_rejection_calls, 0);
}

#[test]
fn dynamic_import_owner_action_runner_reports_stale_owner_without_settlement() {
    let hooks = FakeDynamicImportOwnerActionHooks {
        source_ready_result: FrameDocumentDynamicImportSourceReadyResult::DroppedStaleOwner,
        ..FakeDynamicImportOwnerActionHooks::default()
    };
    let mut action_runner = FrameDocumentDynamicImportOwnerActionRunner::new(hooks);
    let outcome = action_runner
        .run_owner_action(dynamic_import_ready_action_with_phase(
            ModuleImportPhase::Source,
        ))
        .expect("stale source-ready action should be consumed");
    assert!(outcome.made_progress());
    assert!(outcome.stale_owner_was_dropped());
    assert!(!outcome.source_import_was_resolved());
    assert!(!outcome.source_import_was_rejected());

    let hooks = FakeDynamicImportOwnerActionHooks {
        evaluation_ready_result: FrameDocumentDynamicImportEvaluationReadyResult::DroppedStaleOwner,
        ..FakeDynamicImportOwnerActionHooks::default()
    };
    let mut action_runner = FrameDocumentDynamicImportOwnerActionRunner::new(hooks);
    let outcome = action_runner
        .run_owner_action(dynamic_import_ready_action())
        .expect("stale evaluation-ready action should be consumed");
    assert!(outcome.made_progress());
    assert!(outcome.stale_owner_was_dropped());
    assert!(!outcome.evaluation_import_was_resolved());
    assert!(!outcome.evaluation_import_was_pending());
    assert!(!outcome.evaluation_import_was_rejected());

    let hooks = FakeDynamicImportOwnerActionHooks {
        reject_result: FrameDocumentDynamicImportRejectResult::DroppedStaleOwner,
        ..FakeDynamicImportOwnerActionHooks::default()
    };
    let mut action_runner = FrameDocumentDynamicImportOwnerActionRunner::new(hooks);
    let outcome = action_runner
        .run_owner_action(dynamic_import_graph_failed_action())
        .expect("stale rejection action should be consumed");
    assert!(outcome.made_progress());
    assert!(outcome.stale_owner_was_dropped());
    assert!(!outcome.dynamic_import_was_rejected());
}

#[test]
fn dynamic_import_owner_action_runner_reports_graph_failure_rejection() {
    let mut action_runner = FrameDocumentDynamicImportOwnerActionRunner::new(
        FakeDynamicImportOwnerActionHooks::default(),
    );

    let outcome = action_runner
        .run_owner_action(dynamic_import_graph_failed_action())
        .expect("graph failure should reject dynamic import");
    let hooks = action_runner.into_hooks();

    assert!(outcome.dynamic_import_was_rejected());
    assert_eq!(hooks.dynamic_import_rejection_calls, 1);
    assert_eq!(hooks.source_import_calls, 0);
    assert_eq!(hooks.evaluation_import_calls, 0);
}

#[test]
fn dynamic_import_owner_action_runner_reports_failed_fetch_rejection() {
    let mut action_runner = FrameDocumentDynamicImportOwnerActionRunner::new(
        FakeDynamicImportOwnerActionHooks::default(),
    );

    let outcome = action_runner
        .run_owner_action(dynamic_import_failed_fetch_action())
        .expect("failed fetch should reject dynamic import");
    let hooks = action_runner.into_hooks();

    assert!(outcome.dynamic_import_was_rejected());
    assert_eq!(hooks.dynamic_import_rejection_calls, 1);
    assert_eq!(hooks.source_import_calls, 0);
    assert_eq!(hooks.evaluation_import_calls, 0);
}

#[test]
fn module_script_terminal_batch_task_carries_terminal_work_directly() {
    let task_owner =
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(7), LocalWindowId(8), DocumentId(9));
    let realm_id = FrameRealmId(10);
    let key = ModuleMapKey::java_script(
        Url::parse("https://module-script-terminal-task.test/module.js").expect("module url"),
    );
    let client = NativeModuleScriptSingleModuleClient::new(
        module_tree::SingleModuleClientToken {
            tree_id: module_tree::ModuleTreeId(12),
            sequence: 13,
        },
        module_tree::ModuleImportPhase::Evaluation,
    );
    let work = FrameDocumentModuleScriptTerminalWork::from_terminal_parts(
        task_owner, realm_id, key, client,
    );
    let task = FrameDocumentModuleScriptTerminalBatchTask::new(
        task_owner,
        realm_id,
        vec![FrameDocumentModuleScriptTerminalTask::single_module(work)],
    );

    assert_eq!(task.owner(), task_owner);
    assert_eq!(task.realm_id(), realm_id);
    let tasks = task.into_payload();
    assert_eq!(tasks.len(), 1);
}

#[derive(Default)]
struct FakeModuleScriptTerminalHooks {
    parser_root_terminal_calls: usize,
    module_script_terminal_calls: usize,
    dependency_terminal_calls: usize,
}

impl FrameDocumentModuleScriptTerminalHooks for FakeModuleScriptTerminalHooks {
    fn handle_parser_root_terminal(
        &mut self,
        _work: Box<FrameDocumentParserRootTerminalWork>,
    ) -> FrameDocumentModuleScriptTerminalFollowup {
        self.parser_root_terminal_calls += 1;
        FrameDocumentModuleScriptTerminalFollowup::document_script_ready_queued()
    }

    fn handle_single_module_terminal(
        &mut self,
        _work: FrameDocumentModuleScriptTerminalWork,
    ) -> FrameDocumentModuleScriptTerminalFollowup {
        self.module_script_terminal_calls += 1;
        FrameDocumentModuleScriptTerminalFollowup::document_script_ready_queued()
    }

    fn handle_dependency_terminal(
        &mut self,
        _work: Box<FrameDocumentModuleDependencyTerminalWork>,
    ) -> FrameDocumentModuleScriptTerminalFollowup {
        self.dependency_terminal_calls += 1;
        FrameDocumentModuleScriptTerminalFollowup::module_dependency_fetch_queued()
    }
}

#[test]
fn module_script_terminal_runner_dispatches_parser_root_terminal_batch() {
    let task_owner =
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(7), LocalWindowId(8), DocumentId(9));
    let realm_id = FrameRealmId(10);
    let root_url =
        Url::parse("https://module-owner-event-runner.test/root.js").expect("module url");
    let key = ModuleMapKey::java_script(root_url.clone());
    let fetched = crate::frame_owner_model::FrameDocumentParserRootTerminalResult::Fetched(
        ModuleGraphFetchedSource::new(
            root_url.clone(),
            false,
            ModuleSource::text("export const value = 1;".to_owned()),
        ),
    );
    let works = vec![
        FrameDocumentParserRootTerminalWork::from_terminal_parts(
            task_owner,
            realm_id,
            key.clone(),
            FrameDocumentParserRootTerminalClient::new(parser_root_client(31, &root_url)),
            fetched.clone(),
        ),
        FrameDocumentParserRootTerminalWork::from_terminal_parts(
            task_owner,
            realm_id,
            key,
            FrameDocumentParserRootTerminalClient::new(parser_root_client(32, &root_url)),
            fetched,
        ),
    ];
    let task = FrameDocumentModuleScriptTerminalBatchTask::new(
        task_owner,
        realm_id,
        works
            .into_iter()
            .map(FrameDocumentModuleScriptTerminalTask::parser_root)
            .collect(),
    );
    let mut runner =
        FrameDocumentModuleScriptTerminalRunner::new(FakeModuleScriptTerminalHooks::default());

    let outcome = runner.run_terminal_batch_task(task);

    assert!(outcome.made_progress());
    assert!(
        outcome
            .module_script_terminal_followup()
            .document_script_ready_was_queued()
    );
    assert_eq!(runner.into_hooks().parser_root_terminal_calls, 2);
}

#[test]
fn module_script_terminal_runner_dispatches_module_script_terminal_work() {
    let task_owner =
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(7), LocalWindowId(8), DocumentId(9));
    let realm_id = FrameRealmId(10);
    let key = ModuleMapKey::java_script(
        Url::parse("https://module-owner-event-runner.test/module-script.js").expect("module url"),
    );
    let client = NativeModuleScriptSingleModuleClient::new(
        module_tree::SingleModuleClientToken {
            tree_id: module_tree::ModuleTreeId(11),
            sequence: 12,
        },
        module_tree::ModuleImportPhase::Evaluation,
    );
    let work = FrameDocumentModuleScriptTerminalWork::from_terminal_parts(
        task_owner, realm_id, key, client,
    );
    let task = FrameDocumentModuleScriptTerminalBatchTask::new(
        task_owner,
        realm_id,
        vec![FrameDocumentModuleScriptTerminalTask::single_module(work)],
    );
    let mut runner =
        FrameDocumentModuleScriptTerminalRunner::new(FakeModuleScriptTerminalHooks::default());

    let outcome = runner.run_terminal_batch_task(task);
    let hooks = runner.into_hooks();

    assert!(outcome.made_progress());
    assert!(
        outcome
            .module_script_terminal_followup()
            .document_script_ready_was_queued()
    );
    assert_eq!(hooks.module_script_terminal_calls, 1);
}

#[test]
fn module_script_terminal_runner_dispatches_dependency_terminal_work() {
    let task_owner =
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(7), LocalWindowId(8), DocumentId(9));
    let realm_id = FrameRealmId(10);
    let parent_url =
        Url::parse("https://module-owner-event-runner.test/root.js").expect("parent url");
    let parent_key = ModuleMapKey::java_script(parent_url);
    let dependency_url =
        Url::parse("https://module-owner-event-runner.test/dep.js").expect("dependency url");
    let dependency_key = ModuleMapKey::java_script(dependency_url);
    let tree_client = module_tree::SingleModuleClientToken {
        tree_id: module_tree::ModuleTreeId(11),
        sequence: 12,
    };
    let parent_entry_id = ModuleEntryId::from_raw(13);
    let dependency_client = FrameDocumentStaticDependencyModuleClient::new(
        parent_entry_id,
        parent_key,
        "./dep.js".to_owned(),
        ModuleImportPhase::Evaluation,
        tree_client,
    );
    let entry_id = FrameDocumentModuleClientEntryId::from_raw(14);
    let reservation = FrameDocumentModuleClientReservation::new(
        task_owner.document_owner(),
        dependency_key.clone(),
        FrameDocumentModuleClientRegistration::new(
            entry_id,
            FrameDocumentModuleClientId::from_raw(15),
            FrameDocumentModuleFetchDisposition::StartedFetch(entry_id),
        ),
    );
    let fetch_task = FrameDocumentModuleDependencyFetchTask::from_dependency_fetch_parts(
        task_owner,
        realm_id,
        dependency_key,
        dependency_client,
        reservation,
        dynamic_tree_fetch_request("dep.js", tree_client),
    );
    let work = FrameDocumentModuleDependencyTerminalWork::from_fetch_task_result(
        fetch_task,
        FrameDocumentModuleFetchTerminalResult::Failed("dependency failed".to_owned()),
    );
    let task = FrameDocumentModuleScriptTerminalBatchTask::new(
        task_owner,
        realm_id,
        vec![FrameDocumentModuleScriptTerminalTask::dependency(work)],
    );
    let mut runner =
        FrameDocumentModuleScriptTerminalRunner::new(FakeModuleScriptTerminalHooks::default());

    let outcome = runner.run_terminal_batch_task(task);
    let hooks = runner.into_hooks();

    assert!(outcome.made_progress());
    assert!(
        outcome
            .module_script_terminal_followup()
            .module_dependency_fetch_was_queued()
    );
    assert_eq!(hooks.dependency_terminal_calls, 1);
}

#[test]
fn module_script_terminal_runner_dispatches_module_script_terminals() {
    let task_owner =
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(7), LocalWindowId(8), DocumentId(9));
    let realm_id = FrameRealmId(10);
    let key = ModuleMapKey::java_script(
        Url::parse("https://module-owner-event-runner.test/module-script.js").expect("module url"),
    );
    let module_script_client = NativeModuleScriptSingleModuleClient::new(
        module_tree::SingleModuleClientToken {
            tree_id: module_tree::ModuleTreeId(11),
            sequence: 12,
        },
        module_tree::ModuleImportPhase::Evaluation,
    );
    let module_script_work = FrameDocumentModuleScriptTerminalWork::from_terminal_parts(
        task_owner,
        realm_id,
        key.clone(),
        module_script_client,
    );
    let task = FrameDocumentModuleScriptTerminalBatchTask::new(
        task_owner,
        realm_id,
        vec![FrameDocumentModuleScriptTerminalTask::single_module(
            module_script_work,
        )],
    );
    let mut runner =
        FrameDocumentModuleScriptTerminalRunner::new(FakeModuleScriptTerminalHooks::default());

    let outcome = runner.run_terminal_batch_task(task);
    let hooks = runner.into_hooks();

    assert!(outcome.made_progress());
    assert!(
        outcome
            .module_script_terminal_followup()
            .document_script_ready_was_queued()
    );
    assert_eq!(hooks.module_script_terminal_calls, 1);
}
