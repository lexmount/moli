use super::*;

fn parser_root_pending_script_id(
    owner: FrameDocumentTaskOwner,
    handle: usize,
    url: &Url,
) -> crate::document_script_scheduler::ParserPendingScriptId<FrameDocumentOwner> {
    crate::document_script_scheduler::ParserPendingScriptId::new(
        owner.document_owner(),
        &parser_root_script(handle, url),
    )
}

#[test]
fn compiled_parser_root_with_dependencies_retains_shared_tree_job() {
    let mut store = ChildDocumentModulatorStore::default();
    let owner =
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(2), DocumentId(3));
    let document_owner = owner.document_owner();
    let realm_id = FrameRealmId(4);
    let root_url = Url::parse("https://child-module-graph.test/root.js").expect("root url");
    let root_key = ModuleMapKey::java_script(root_url.clone());
    let entry_id = {
        let mut document_modulator =
            store.take_or_create_document_modulator(document_owner, realm_id);
        let disposition = document_modulator.start_or_join_module_fetch(root_key.clone());
        let _ = store.restore_document_modulator(owner, realm_id, document_modulator);
        disposition.entry_id()
    };

    let pending_script_id = parser_root_pending_script_id(owner, 8, &root_url);
    let tree_id = store.record_compiled_parser_root(
        owner,
        realm_id,
        pending_script_id,
        parser_root_script(8, &root_url),
        DomHandle::new(8),
        root_key.clone(),
        root_url.clone(),
        entry_id,
        root_key.clone(),
        vec![ModuleRequestRecord::new(
            "./dep.js",
            ModuleAttributesKey::empty(),
            ModuleImportPhase::Evaluation,
        )],
        ModuleFetchMetadata::default(),
        parser_root_load_delay_token(8),
    );
    let dep_url = Url::parse("https://child-module-graph.test/dep.js").expect("dep url");
    let dep_key = ModuleMapKey::java_script(dep_url.clone());
    {
        let mut document_modulator =
            store.take_or_create_document_modulator(document_owner, realm_id);
        document_modulator.start_or_join_module_fetch(dep_key.clone());
        let _ = store.restore_document_modulator(owner, realm_id, document_modulator);
    }
    let tree_client = module_tree::SingleModuleClientToken {
        tree_id,
        sequence: 1,
    };
    let fetch = NativeModuleGraphFetchRequest::new_tree_dependency_for_test(
        dep_url.clone(),
        root_url,
        ModuleFetchMetadata::default(),
        ModuleKind::JavaScript,
        tree_client,
        dep_key.clone(),
        root_key.clone(),
        entry_id,
        "./dep.js".to_owned(),
        ModuleImportPhase::Evaluation,
    );
    let mut resume = store
        .take_parser_module_tree_job(document_owner, realm_id, tree_id)
        .expect("compiled parser root should retain its tree job");
    let fetch_tasks = store
        .record_parser_module_tree_fetches(&mut resume, vec![fetch])
        .expect("dependency fetch task should be emitted from shared tree job");
    assert_eq!(fetch_tasks.len(), 1);
    store.restore_parser_module_tree_job(resume);

    let graph = store
        .current_document_modulator_entry(document_owner, realm_id)
        .expect("child document modulator entry should exist");
    assert!(
        graph
            .document_modulator
            .has_parser_module_tree_job_for_test(tree_id),
        "pending parser root should keep its shared NativeModuleTreeJob state"
    );
    assert_eq!(
        fetch_tasks[0].client().tree_client(),
        tree_client,
        "dependency fetch should be emitted as a typed owner task, not stored in the child document modulator store"
    );
    let terminal_tasks = store.finish_parser_module_dependency_fetch(
        document_owner,
        realm_id,
        fetch_tasks[0].clone(),
        FrameDocumentModuleFetchTerminalResult::Failed("network failed".to_owned()),
    );
    assert_eq!(terminal_tasks.len(), 1);
    let terminal_batch = terminal_tasks
        .into_iter()
        .next()
        .expect("dependency completion should queue terminal batch");
    assert_eq!(terminal_batch.owner(), owner);
    assert_eq!(terminal_batch.realm_id(), realm_id);
    let mut terminal_tasks = terminal_batch.into_payload();
    assert_eq!(terminal_tasks.len(), 1);
    let FrameDocumentModuleScriptTerminalTask::Dependency(terminal_work) = terminal_tasks
        .pop()
        .expect("dependency completion should queue one terminal work")
    else {
        panic!("expected dependency terminal task");
    };
    let (work_owner, work_realm_id, work_key, work_client, work_fetch_request, work_result) =
        terminal_work.into_terminal_parts();
    assert_eq!(work_owner, owner);
    assert_eq!(work_realm_id, realm_id);
    assert_eq!(work_key, dep_key);
    assert_eq!(work_client.tree_client(), tree_client);
    assert_eq!(work_fetch_request.source_url(), &dep_url);
    assert!(matches!(
        work_result,
        FrameDocumentModuleFetchTerminalResult::Failed(_)
    ));
}

#[test]
fn parser_module_graph_evaluated_mark_stays_in_owner_store() {
    let mut store = ChildDocumentModulatorStore::default();
    let owner =
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(2), DocumentId(3));
    let document_owner = owner.document_owner();
    let realm_id = FrameRealmId(4);
    let stale_realm_id = FrameRealmId(5);
    let root_url = Url::parse("https://child-module-graph.test/root.js").expect("root url");
    let root_key = ModuleMapKey::java_script(root_url.clone());
    let entry_id = {
        let mut document_modulator =
            store.take_or_create_document_modulator(document_owner, realm_id);
        let disposition = document_modulator.start_or_join_module_fetch(root_key.clone());
        let _ = store.restore_document_modulator(owner, realm_id, document_modulator);
        disposition.entry_id()
    };
    store.record_compiled_parser_root(
        owner,
        realm_id,
        parser_root_pending_script_id(owner, 8, &root_url),
        parser_root_script(8, &root_url),
        DomHandle::new(8),
        root_key.clone(),
        root_url,
        entry_id,
        root_key,
        Vec::new(),
        ModuleFetchMetadata::default(),
        parser_root_load_delay_token(8),
    );

    assert!(store.mark_module_graph_evaluated(document_owner, realm_id, entry_id));
    let current_entry = store
        .current_document_modulator_entry(document_owner, realm_id)
        .expect("current document modulator entry should exist");
    assert_eq!(
        current_entry
            .document_modulator
            .module_entry_state(entry_id),
        crate::document_module_graph::ModuleMapEntryState::Evaluated
    );
    assert!(!store.mark_module_graph_evaluated(document_owner, stale_realm_id, entry_id));
    let current_entry_after_stale = store
        .current_document_modulator_entry(document_owner, realm_id)
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
fn dependency_fetch_without_document_modulator_entry_fails_without_materializing_entry() {
    let mut store = ChildDocumentModulatorStore::default();
    let owner =
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(2), DocumentId(3));
    let document_owner = owner.document_owner();
    let realm_id = FrameRealmId(4);
    let root_url = Url::parse("https://child-module-graph.test/root.js").expect("root url");
    let root_key = ModuleMapKey::java_script(root_url.clone());
    let entry_id = {
        let mut document_modulator =
            store.take_or_create_document_modulator(document_owner, realm_id);
        let disposition = document_modulator.start_or_join_module_fetch(root_key.clone());
        let _ = store.restore_document_modulator(owner, realm_id, document_modulator);
        disposition.entry_id()
    };
    let tree_id = store.record_compiled_parser_root(
        owner,
        realm_id,
        parser_root_pending_script_id(owner, 8, &root_url),
        parser_root_script(8, &root_url),
        DomHandle::new(8),
        root_key.clone(),
        root_url.clone(),
        entry_id,
        root_key,
        vec![ModuleRequestRecord::new(
            "./dep.js",
            ModuleAttributesKey::empty(),
            ModuleImportPhase::Evaluation,
        )],
        ModuleFetchMetadata::default(),
        parser_root_load_delay_token(8),
    );
    let mut resume = store
        .take_parser_module_tree_job(document_owner, realm_id, tree_id)
        .expect("compiled parser root should retain its tree job");
    store.clear();
    let source_url =
        Url::parse("https://child-module-graph.test/malformed-dep.js").expect("source url");
    let fetch = NativeModuleGraphFetchRequest::new_for_test(
        source_url.clone(),
        source_url,
        ModuleFetchMetadata::default(),
        ModuleKind::JavaScript,
    );

    let error = store
        .record_parser_module_tree_fetches(&mut resume, vec![fetch])
        .expect_err("missing current document modulator entry should fail before fetch conversion");

    assert_eq!(
        error.message(),
        "child parser module dependency fetch had no current document modulator entry"
    );
    assert!(
        store.documents.is_empty(),
        "missing document modulator entry should not materialize child document modulator state"
    );
}

#[test]
fn ready_parser_root_releases_shared_tree_job() {
    let mut store = ChildDocumentModulatorStore::default();
    let owner =
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(2), DocumentId(3));
    let document_owner = owner.document_owner();
    let realm_id = FrameRealmId(4);
    let root_url = Url::parse("https://child-module-graph.test/root.js").expect("root url");
    let root_key = ModuleMapKey::java_script(root_url.clone());
    let entry_id = {
        let mut document_modulator =
            store.take_or_create_document_modulator(document_owner, realm_id);
        let disposition = document_modulator.start_or_join_module_fetch(root_key.clone());
        let _ = store.restore_document_modulator(owner, realm_id, document_modulator);
        disposition.entry_id()
    };

    let tree_id = store.record_compiled_parser_root(
        owner,
        realm_id,
        parser_root_pending_script_id(owner, 8, &root_url),
        parser_root_script(8, &root_url),
        DomHandle::new(8),
        root_key.clone(),
        root_url.clone(),
        entry_id,
        root_key,
        Vec::new(),
        ModuleFetchMetadata::default(),
        parser_root_load_delay_token(8),
    );
    let resume = store
        .take_parser_module_tree_job(document_owner, realm_id, tree_id)
        .expect("compiled parser root should retain its tree job");
    let ready = module_script_graph_ready_work_from_tree_job(
        resume,
        ModuleGraphHandle {
            root_entry: entry_id,
            entries: vec![entry_id, ModuleEntryId::from_raw(entry_id.raw() + 1)],
        },
    );
    assert_eq!(ready.entry_id(), entry_id);
    assert_eq!(
        ready.graph().entries,
        vec![entry_id, ModuleEntryId::from_raw(entry_id.raw() + 1)],
        "graph-ready work should retain the full completed graph for pending-script execution"
    );
    assert_eq!(ready.dependency_count(), 1);
    let graph = store
        .current_document_modulator_entry(document_owner, realm_id)
        .expect("child document modulator entry should exist");
    assert!(
        !graph
            .document_modulator
            .has_parser_module_tree_job_for_test(ready.tree_id()),
        "graph-ready parser root should release its shared NativeModuleTreeJob state"
    );
}

#[test]
fn parser_tree_advance_complete_builds_graph_ready_action() {
    let mut store = ChildDocumentModulatorStore::default();
    let owner =
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(2), DocumentId(3));
    let document_owner = owner.document_owner();
    let realm_id = FrameRealmId(4);
    let root_url = Url::parse("https://child-module-graph.test/root.js").expect("root url");
    let root_key = ModuleMapKey::java_script(root_url.clone());
    let entry_id = {
        let mut document_modulator =
            store.take_or_create_document_modulator(document_owner, realm_id);
        let disposition = document_modulator.start_or_join_module_fetch(root_key.clone());
        let _ = store.restore_document_modulator(owner, realm_id, document_modulator);
        disposition.entry_id()
    };
    let pending_script_id = parser_root_pending_script_id(owner, 8, &root_url);
    let tree_id = store.record_compiled_parser_root(
        owner,
        realm_id,
        pending_script_id,
        parser_root_script(8, &root_url),
        DomHandle::new(8),
        root_key.clone(),
        root_url,
        entry_id,
        root_key.clone(),
        Vec::new(),
        ModuleFetchMetadata::default(),
        parser_root_load_delay_token(8),
    );
    let resume = store
        .take_parser_module_tree_job(document_owner, realm_id, tree_id)
        .expect("compiled parser root should retain its tree job");

    let action = frame_document_parser_module_tree_advance_action(
        document_owner,
        realm_id,
        tree_id,
        resume,
        Ok(NativeModuleGraphJobAdvance::Complete(ModuleGraphHandle {
            root_entry: entry_id,
            entries: vec![entry_id],
        })),
    );

    let FrameDocumentParserModuleTreeAdvanceAction::NotifyGraphReady(work) = action else {
        panic!("complete parser tree advance should produce graph-ready work");
    };
    assert_eq!(work.owner(), owner);
    assert_eq!(work.realm_id(), realm_id);
    assert_eq!(work.pending_script_id(), pending_script_id);
    assert_eq!(work.tree_id(), tree_id);
    assert_eq!(work.entry_id(), entry_id);
    assert_eq!(work.request_key(), &root_key);
}

#[test]
fn parser_tree_advance_failure_builds_graph_failed_action() {
    let mut store = ChildDocumentModulatorStore::default();
    let owner =
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(2), DocumentId(3));
    let document_owner = owner.document_owner();
    let realm_id = FrameRealmId(4);
    let root_url = Url::parse("https://child-module-graph.test/root.js").expect("root url");
    let root_key = ModuleMapKey::java_script(root_url.clone());
    let entry_id = {
        let mut document_modulator =
            store.take_or_create_document_modulator(document_owner, realm_id);
        let disposition = document_modulator.start_or_join_module_fetch(root_key.clone());
        let _ = store.restore_document_modulator(owner, realm_id, document_modulator);
        disposition.entry_id()
    };
    let pending_script_id = parser_root_pending_script_id(owner, 8, &root_url);
    let tree_id = store.record_compiled_parser_root(
        owner,
        realm_id,
        pending_script_id,
        parser_root_script(8, &root_url),
        DomHandle::new(8),
        root_key.clone(),
        root_url,
        entry_id,
        root_key.clone(),
        Vec::new(),
        ModuleFetchMetadata::default(),
        parser_root_load_delay_token(8),
    );
    let resume = store
        .take_parser_module_tree_job(document_owner, realm_id, tree_id)
        .expect("compiled parser root should retain its tree job");

    let action = frame_document_parser_module_tree_advance_action(
        document_owner,
        realm_id,
        tree_id,
        resume,
        Err(ModuleLoadError::new(
            ModuleLoadStage::Fetch,
            "advance failed",
        )),
    );

    let FrameDocumentParserModuleTreeAdvanceAction::NotifyGraphFailed { trace, work } = action
    else {
        panic!("failed parser tree advance should produce graph-failed work");
    };
    assert_eq!(
        trace,
        FrameDocumentParserModuleTreeAdvanceFailureTrace::OwnerLaneAdvance
    );
    assert_eq!(work.owner(), owner);
    assert_eq!(work.realm_id(), realm_id);
    assert_eq!(work.pending_script_id(), pending_script_id);
    assert_eq!(work.tree_id(), Some(tree_id));
    assert_eq!(work.request_key(), &root_key);
    assert_eq!(work.error().message(), "advance failed");
}

#[derive(Default)]
struct FakeParserModuleTreeAdvanceHooks {
    queue_dependency_effects: usize,
    restore_waiting_effects: usize,
    graph_ready_effects: usize,
    graph_failed_effects: usize,
    fail_dependency_conversion: bool,
    fail_dependency_route: bool,
    queue_dependency_fetch: bool,
    last_failed_trace: Option<FrameDocumentParserModuleTreeAdvanceFailureTrace>,
}

impl FrameDocumentParserModuleTreeAdvanceHooks for FakeParserModuleTreeAdvanceHooks {
    fn queue_dependency_fetches(
        &mut self,
        _document_owner: FrameDocumentOwner,
        _realm_id: FrameRealmId,
        _tree_id: module_tree::ModuleTreeId,
        resume: Box<NativeParserModuleTreeJobResume>,
        _fetches: Vec<NativeModuleGraphFetchRequest>,
    ) -> FrameDocumentParserModuleTreeAdvanceDependencyFetchResult {
        self.queue_dependency_effects += 1;
        if self.fail_dependency_conversion {
            return FrameDocumentParserModuleTreeAdvanceDependencyFetchResult::DependencyFetchStartFailed {
                trace: FrameDocumentParserModuleTreeAdvanceFailureTrace::DependencyFetchTaskConversion,
                work: Box::new(module_script_graph_failed_work_from_tree_job(
                    *resume,
                    ModuleLoadError::new(ModuleLoadStage::Fetch, "conversion failed"),
                )),
            };
        }
        if self.fail_dependency_route {
            return FrameDocumentParserModuleTreeAdvanceDependencyFetchResult::DependencyFetchStartFailed {
                trace: FrameDocumentParserModuleTreeAdvanceFailureTrace::DependencyFetchStartRoute,
                work: Box::new(module_script_graph_failed_work_from_tree_job(
                    *resume,
                    ModuleLoadError::new(ModuleLoadStage::Fetch, "stable route closed"),
                )),
            };
        }
        if self.queue_dependency_fetch {
            FrameDocumentParserModuleTreeAdvanceDependencyFetchResult::Followup(
                FrameDocumentModuleScriptTerminalFollowup::module_dependency_fetch_queued(),
            )
        } else {
            FrameDocumentParserModuleTreeAdvanceDependencyFetchResult::Followup(
                FrameDocumentModuleScriptTerminalFollowup::module_script_wait_retained(),
            )
        }
    }

    fn restore_waiting(&mut self, _resume: Box<NativeParserModuleTreeJobResume>) {
        self.restore_waiting_effects += 1;
    }

    fn notify_graph_ready(
        &mut self,
        _work: Box<crate::document_script_scheduler::DocumentModuleGraphReadyWork>,
    ) -> FrameDocumentModuleScriptTerminalFollowup {
        self.graph_ready_effects += 1;
        FrameDocumentModuleScriptTerminalFollowup::document_script_ready_queued()
    }

    fn notify_graph_failed(
        &mut self,
        trace: FrameDocumentParserModuleTreeAdvanceFailureTrace,
        _work: Box<crate::document_script_scheduler::DocumentModuleGraphFailedWork>,
    ) -> FrameDocumentModuleScriptTerminalFollowup {
        self.graph_failed_effects += 1;
        self.last_failed_trace = Some(trace);
        FrameDocumentModuleScriptTerminalFollowup::document_script_ready_queued()
    }
}

fn parser_tree_resume_for_advance_runner() -> (
    FrameDocumentTaskOwner,
    FrameDocumentOwner,
    FrameRealmId,
    module_tree::ModuleTreeId,
    NativeParserModuleTreeJobResume,
    ModuleEntryId,
    ModuleMapKey,
) {
    let mut store = ChildDocumentModulatorStore::default();
    let owner =
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(2), DocumentId(3));
    let document_owner = owner.document_owner();
    let realm_id = FrameRealmId(4);
    let root_url = Url::parse("https://child-module-graph.test/root.js").expect("root url");
    let root_key = ModuleMapKey::java_script(root_url.clone());
    let entry_id = {
        let mut document_modulator =
            store.take_or_create_document_modulator(document_owner, realm_id);
        let disposition = document_modulator.start_or_join_module_fetch(root_key.clone());
        let _ = store.restore_document_modulator(owner, realm_id, document_modulator);
        disposition.entry_id()
    };
    let tree_id = store.record_compiled_parser_root(
        owner,
        realm_id,
        parser_root_pending_script_id(owner, 8, &root_url),
        parser_root_script(8, &root_url),
        DomHandle::new(8),
        root_key.clone(),
        root_url,
        entry_id,
        root_key.clone(),
        Vec::new(),
        ModuleFetchMetadata::default(),
        parser_root_load_delay_token(8),
    );
    let resume = store
        .take_parser_module_tree_job(document_owner, realm_id, tree_id)
        .expect("compiled parser root should retain its tree job");
    (
        owner,
        document_owner,
        realm_id,
        tree_id,
        resume,
        entry_id,
        root_key,
    )
}

#[test]
fn parser_tree_advance_runner_routes_graph_ready_followup() {
    let (_owner, document_owner, realm_id, tree_id, resume, entry_id, _root_key) =
        parser_tree_resume_for_advance_runner();
    let action = frame_document_parser_module_tree_advance_action(
        document_owner,
        realm_id,
        tree_id,
        resume,
        Ok(NativeModuleGraphJobAdvance::Complete(ModuleGraphHandle {
            root_entry: entry_id,
            entries: vec![entry_id],
        })),
    );
    let mut runner = FrameDocumentParserModuleTreeAdvanceRunner::new(
        FakeParserModuleTreeAdvanceHooks::default(),
    );

    let followup = runner.run_tree_advance_action(action);
    let hooks = runner.into_hooks();

    assert!(followup.document_script_ready_was_queued());
    assert_eq!(hooks.graph_ready_effects, 1);
}

#[test]
fn parser_tree_advance_runner_reports_retained_wait_followup() {
    let (_owner, _document_owner, _realm_id, _tree_id, resume, _entry_id, _root_key) =
        parser_tree_resume_for_advance_runner();
    let action = FrameDocumentParserModuleTreeAdvanceAction::RestoreWaiting {
        resume: Box::new(resume),
    };
    let mut runner = FrameDocumentParserModuleTreeAdvanceRunner::new(
        FakeParserModuleTreeAdvanceHooks::default(),
    );

    let followup = runner.run_tree_advance_action(action);
    let hooks = runner.into_hooks();

    assert!(!followup.document_script_ready_was_queued());
    assert!(!followup.module_dependency_fetch_was_queued());
    assert!(followup.module_script_wait_was_retained());
    assert_eq!(hooks.restore_waiting_effects, 1);
}

#[test]
fn parser_tree_advance_runner_reports_dependency_fetch_followup() {
    let (_owner, document_owner, realm_id, tree_id, resume, _entry_id, _root_key) =
        parser_tree_resume_for_advance_runner();
    let action = FrameDocumentParserModuleTreeAdvanceAction::QueueDependencyFetches {
        document_owner,
        realm_id,
        tree_id,
        resume: Box::new(resume),
        fetches: Vec::new(),
    };
    let mut runner =
        FrameDocumentParserModuleTreeAdvanceRunner::new(FakeParserModuleTreeAdvanceHooks {
            queue_dependency_fetch: true,
            ..FakeParserModuleTreeAdvanceHooks::default()
        });

    let followup = runner.run_tree_advance_action(action);
    let hooks = runner.into_hooks();

    assert!(followup.module_dependency_fetch_was_queued());
    assert!(!followup.document_script_ready_was_queued());
    assert_eq!(hooks.queue_dependency_effects, 1);
    assert_eq!(hooks.graph_failed_effects, 0);
}

#[test]
fn parser_tree_advance_runner_reports_dependency_wait_retained_followup() {
    let (_owner, document_owner, realm_id, tree_id, resume, _entry_id, _root_key) =
        parser_tree_resume_for_advance_runner();
    let action = FrameDocumentParserModuleTreeAdvanceAction::QueueDependencyFetches {
        document_owner,
        realm_id,
        tree_id,
        resume: Box::new(resume),
        fetches: Vec::new(),
    };
    let mut runner = FrameDocumentParserModuleTreeAdvanceRunner::new(
        FakeParserModuleTreeAdvanceHooks::default(),
    );

    let followup = runner.run_tree_advance_action(action);
    let hooks = runner.into_hooks();

    assert!(!followup.module_dependency_fetch_was_queued());
    assert!(!followup.document_script_ready_was_queued());
    assert!(followup.module_script_wait_was_retained());
    assert_eq!(hooks.queue_dependency_effects, 1);
    assert_eq!(hooks.graph_failed_effects, 0);
}

#[test]
fn parser_tree_advance_runner_routes_dependency_conversion_failure_to_graph_failed() {
    let (_owner, document_owner, realm_id, tree_id, resume, _entry_id, _root_key) =
        parser_tree_resume_for_advance_runner();
    let action = FrameDocumentParserModuleTreeAdvanceAction::QueueDependencyFetches {
        document_owner,
        realm_id,
        tree_id,
        resume: Box::new(resume),
        fetches: Vec::new(),
    };
    let mut runner =
        FrameDocumentParserModuleTreeAdvanceRunner::new(FakeParserModuleTreeAdvanceHooks {
            fail_dependency_conversion: true,
            ..FakeParserModuleTreeAdvanceHooks::default()
        });

    let followup = runner.run_tree_advance_action(action);
    let hooks = runner.into_hooks();

    assert!(followup.document_script_ready_was_queued());
    assert_eq!(hooks.queue_dependency_effects, 1);
    assert_eq!(hooks.graph_failed_effects, 1);
    assert_eq!(
        hooks.last_failed_trace,
        Some(FrameDocumentParserModuleTreeAdvanceFailureTrace::DependencyFetchTaskConversion)
    );
}

#[test]
fn parser_tree_advance_runner_routes_dependency_start_failure_to_graph_failed() {
    let (_owner, document_owner, realm_id, tree_id, resume, _entry_id, _root_key) =
        parser_tree_resume_for_advance_runner();
    let action = FrameDocumentParserModuleTreeAdvanceAction::QueueDependencyFetches {
        document_owner,
        realm_id,
        tree_id,
        resume: Box::new(resume),
        fetches: Vec::new(),
    };
    let mut runner =
        FrameDocumentParserModuleTreeAdvanceRunner::new(FakeParserModuleTreeAdvanceHooks {
            fail_dependency_route: true,
            ..FakeParserModuleTreeAdvanceHooks::default()
        });

    let followup = runner.run_tree_advance_action(action);
    let hooks = runner.into_hooks();

    assert!(followup.document_script_ready_was_queued());
    assert_eq!(hooks.queue_dependency_effects, 1);
    assert_eq!(hooks.graph_failed_effects, 1);
    assert_eq!(
        hooks.last_failed_trace,
        Some(FrameDocumentParserModuleTreeAdvanceFailureTrace::DependencyFetchStartRoute)
    );
}

#[test]
fn stale_parser_module_tree_job_restore_does_not_materialize_document_modulator_entry() {
    let mut store = ChildDocumentModulatorStore::default();
    let owner =
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(2), DocumentId(3));
    let document_owner = owner.document_owner();
    let realm_id = FrameRealmId(4);
    let root_url = Url::parse("https://child-module-graph.test/root.js").expect("root url");
    let root_key = ModuleMapKey::java_script(root_url.clone());
    let entry_id = {
        let mut document_modulator =
            store.take_or_create_document_modulator(document_owner, realm_id);
        let disposition = document_modulator.start_or_join_module_fetch(root_key.clone());
        let _ = store.restore_document_modulator(owner, realm_id, document_modulator);
        disposition.entry_id()
    };

    let tree_id = store.record_compiled_parser_root(
        owner,
        realm_id,
        parser_root_pending_script_id(owner, 8, &root_url),
        parser_root_script(8, &root_url),
        DomHandle::new(8),
        root_key.clone(),
        root_url,
        entry_id,
        root_key,
        Vec::new(),
        ModuleFetchMetadata::default(),
        parser_root_load_delay_token(8),
    );
    let resume = store
        .take_parser_module_tree_job(document_owner, realm_id, tree_id)
        .expect("compiled parser root should retain its tree job");

    store.clear();
    store.restore_parser_module_tree_job(resume);

    assert!(
        store.documents.is_empty(),
        "stale parser tree-job restore must not materialize a child document modulator entry"
    );
}

#[test]
fn parser_module_tree_job_resume_is_document_local() {
    let mut store = ChildDocumentModulatorStore::default();
    let owner =
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(2), DocumentId(3));
    let other_owner =
        FrameDocumentTaskOwner::new(FrameSchedulerLaneId(1), LocalWindowId(4), DocumentId(5));
    let document_owner = owner.document_owner();
    let realm_id = FrameRealmId(6);
    let root_url = Url::parse("https://child-module-graph.test/root.js").expect("root url");
    let root_key = ModuleMapKey::java_script(root_url.clone());
    let entry_id = {
        let mut document_modulator =
            store.take_or_create_document_modulator(document_owner, realm_id);
        let disposition = document_modulator.start_or_join_module_fetch(root_key.clone());
        let _ = store.restore_document_modulator(owner, realm_id, document_modulator);
        disposition.entry_id()
    };

    let tree_id = store.record_compiled_parser_root(
        owner,
        realm_id,
        parser_root_pending_script_id(owner, 8, &root_url),
        parser_root_script(8, &root_url),
        DomHandle::new(8),
        root_key.clone(),
        root_url.clone(),
        entry_id,
        root_key,
        vec![ModuleRequestRecord::new(
            "./dep.js",
            ModuleAttributesKey::empty(),
            ModuleImportPhase::Evaluation,
        )],
        ModuleFetchMetadata::default(),
        parser_root_load_delay_token(8),
    );
    assert!(
        store
            .take_parser_module_tree_job(other_owner.document_owner(), realm_id, tree_id)
            .is_none(),
        "parser tree jobs must not be taken from another child document owner"
    );
    assert!(
        store
            .take_parser_module_tree_job(document_owner, FrameRealmId(99), tree_id)
            .is_none(),
        "parser tree jobs must not be taken from a stale child FrameRealm"
    );

    let resume = store
        .take_parser_module_tree_job(document_owner, realm_id, tree_id)
        .expect("matching child document owner should expose parser tree job");
    assert_eq!(resume.root().owner().document_owner(), document_owner);
    assert_eq!(resume.root().realm_id(), realm_id);
    assert_eq!(resume.root().tree_id(), tree_id);
    assert!(
        !store
            .current_document_modulator_entry(document_owner, realm_id)
            .expect("child document modulator entry should exist")
            .document_modulator
            .has_parser_module_tree_job_for_test(tree_id),
        "taking the tree job should remove it from document-local storage until restored"
    );

    store.restore_parser_module_tree_job(resume);
    assert!(
        store
            .current_document_modulator_entry(document_owner, realm_id)
            .expect("child document modulator entry should exist")
            .document_modulator
            .has_parser_module_tree_job_for_test(tree_id),
        "restoring the resume handle should return the tree job to the same document"
    );
}
