use std::pin::pin;
use std::sync::Arc;
use std::time::Instant;
use tracing::debug;
use url::Url;

use super::ScriptVm;
use super::eval_exec::RuntimePendingWorkFlushOutcome;
use super::native_module::RuntimeModuleScriptGraphStart;
use super::post_parse::dynamic_script_execute_is_runnable_before_dom_content_loaded;
use super::runtime_script_continuation::RuntimeScriptOwnerAdvance;
use crate::document_runtime::{
    DocumentProcessingAction, DomHandle, FollowupPageTaskDisposition, PostParseOwnerDriverStep,
    RuntimeScriptWorkPauseKind,
};
use crate::document_script_scheduler::{DocumentScriptExecutionLane, PageOwnedDocumentScriptWork};
#[cfg(test)]
use crate::dom::NodeId;
use crate::dynamic_script_owner::{
    DynamicModuleScriptContinuationWork, DynamicScriptOwnerId, DynamicScriptPageTaskClaim,
    DynamicScriptRunnable, DynamicScriptServiceWorkerContext,
};
use crate::frame_owner_model::{
    FrameDocumentTaskOwner, MainDocumentScriptLoadDelayKind, MainDocumentScriptLoadDelayLease,
    MainDocumentScriptLoadDelayRelease,
};
use crate::host::{
    RuntimeScriptAdmission, ScriptEventKind, ScriptHandleSource, ScriptPageTaskExecutionKind,
};
use crate::module_script_continuation::{
    ModuleScriptContinuation, ModuleScriptEvaluationContinuation, ModuleScriptEvaluationUpdate,
};
use crate::native_bridge::JsContextHost;
use crate::network::ResourceRequestClient;
use crate::page_task_queue::PageTaskQueue;
use crate::page_task_queue::{
    PostParseLifecycleQueueStats, PostParseLifecycleWork, PostParsePageOwnedWork,
    post_parse_lifecycle_queue_stats,
};
use crate::planning::PreparedScript;
use crate::script_vm::{
    ImmediateRuntimeScriptWorkSignal, ParserFinishDomContentLoadedTask,
    ParserFinishDomContentLoadedWork, PostParseDriverStep, PostParseLifecycleAdvance,
    PostParseLifecycleCompletionAction, PostParseLifecycleDriver, PostParseLifecycleRound,
    PostParsePageOwnedTask, PostParseProcessingAction, PostParseProcessingStep,
    PostParseRuntimeDriverStep, PostParseTaskExecutionToken, ReadyPostParseAction,
    select_post_parse_driver_step,
};
#[cfg(test)]
use crate::script_vm::{MainDocumentLifecycleBody, NonScriptPageTaskExecutionOutcome};
use crate::types::{ScriptExecutionReport, SubresourceResourceType};
use crate::types::{ScriptKind, ScriptMode, ScriptSourceKind, SubresourceRequestInitiatorType};

fn preload_like_resource_initiator_type(resource_type: SubresourceResourceType) -> &'static str {
    match resource_type {
        SubresourceResourceType::Script => "link",
        SubresourceResourceType::Image => "img",
        SubresourceResourceType::Audio => "audio",
        SubresourceResourceType::Media => "video",
        SubresourceResourceType::Video => "video",
        SubresourceResourceType::TextTrack => "track",
        SubresourceResourceType::Fetch
        | SubresourceResourceType::EventSource
        | SubresourceResourceType::Font
        | SubresourceResourceType::Ping
        | SubresourceResourceType::CspReport
        | SubresourceResourceType::Dictionary
        | SubresourceResourceType::Manifest
        | SubresourceResourceType::Xhr
        | SubresourceResourceType::WebSocket
        | SubresourceResourceType::Stylesheet => "link",
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RuntimeScriptFailureTerminalBodySettlement {
    activity: crate::script_vm::ScriptTerminalBodyActivity,
    load_gate_release: MainDocumentScriptLoadDelayRelease,
}

/// Bounded result of settling the runtime-module failures produced by one
/// already-selected graph terminal.
///
/// It deliberately separates event-dispatch activity from the load-gate
/// transition. The ResourceCompletion task needs the former to choose its
/// checkpoint follow-up and the latter to prime lifecycle only after that
/// checkpoint; neither fact is scheduler metadata.
#[must_use = "module failure settlement determines resource-task completion"]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RuntimeOwnedModuleFailureBodySettlement {
    activity: crate::script_vm::ScriptTerminalBodyActivity,
    lifecycle_unblocked_owner: Option<FrameDocumentTaskOwner>,
}

impl RuntimeOwnedModuleFailureBodySettlement {
    pub(crate) const fn none() -> Self {
        Self {
            activity: crate::script_vm::ScriptTerminalBodyActivity::NoEventDispatch,
            lifecycle_unblocked_owner: None,
        }
    }

    pub(crate) const fn activity(self) -> crate::script_vm::ScriptTerminalBodyActivity {
        self.activity
    }

    pub(crate) const fn lifecycle_unblocked_owner(self) -> Option<FrameDocumentTaskOwner> {
        self.lifecycle_unblocked_owner
    }
}
impl ScriptVm {
    pub(crate) fn admit_main_document_runtime_script_task(
        &mut self,
        loader: &ResourceRequestClient,
        task_runner: crate::network::RendererResourceTaskRunner,
        admission: RuntimeScriptAdmission,
    ) {
        let document_character_set = self.document_runtime.document_character_set().to_owned();
        let service_worker_context = {
            let host = self._context_host.borrow();
            DynamicScriptServiceWorkerContext {
                browser_context_runtime: host.browser_context_runtime(),
                client_id: host.service_worker_client_id_for_window_fetch(None),
                document_url: self.document_runtime.document_url().clone(),
            }
        };
        self.document_runtime
            .runtime_script_work_mut()
            .dynamic_scripts
            .enqueue_admission(
                loader,
                task_runner,
                admission,
                Some(&document_character_set),
                Some(&service_worker_context),
            );
        self.handle_runtime_script_work_at_explicit_boundary(true);
    }

    pub(super) fn handle_runtime_script_work_at_explicit_boundary(
        &mut self,
        enqueue_continuation_now: bool,
    ) {
        self.pause_runtime_script_work_for_stable_continuation();
        if enqueue_continuation_now && self.runtime_script_work_should_signal_immediate_progress() {
            self.enqueue_runtime_script_work_continuation_if_ready();
        }
    }

    pub(super) fn resume_runtime_script_work_after_deferred_page_tasks(&mut self) {
        self.document_runtime
            .runtime_script_work_mut()
            .dynamic_scripts
            .disable_continuation_enqueue();
        self.document_runtime
            .runtime_script_work_mut()
            .resume_after_deferred_page_tasks();
    }

    pub(super) fn runtime_script_work_has_immediately_runnable_work(&mut self) -> bool {
        self.document_runtime.accept_ready_runtime_script_events();
        self.document_runtime
            .runtime_script_work_mut()
            .has_immediately_runnable_work()
    }

    pub(crate) fn prepared_script_uses_runtime_owned_page_task_execution(
        &self,
        script: &PreparedScript,
    ) -> bool {
        if script.kind == ScriptKind::ImportMap && script.source_kind == ScriptSourceKind::Inline {
            return false;
        }
        script.host_script_handle.as_deref().and_then(|handle| {
            self.document_runtime
                .script_handle_page_task_execution_kind(handle)
        }) == Some(ScriptPageTaskExecutionKind::RuntimeOwned)
    }

    pub(super) fn plan_script_load_lifecycle_work_for_prepared_script(
        &mut self,
        script: &PreparedScript,
    ) -> Option<PostParseLifecycleWork> {
        if !self
            .document_runtime
            .script_event_requires_dispatch_for_script(ScriptEventKind::Load, script)
        {
            return None;
        }
        let handle =
            self.required_host_script_handle_for_observable_script_followup(script, "script load")?;
        self.document_runtime
            .plan_script_event_task_for_script(ScriptEventKind::Load, script, handle)
            .map(PostParseLifecycleWork::DispatchScriptEvent)
    }

    fn claim_runtime_script_page_owned_execution(
        &mut self,
        id: DynamicScriptOwnerId,
        script: PreparedScript,
        source_network_result: Option<crate::types::SharedNavigationResponseResult>,
    ) -> PostParsePageOwnedWork {
        let runtime_script_claim = self
            .document_runtime
            .runtime_script_work_mut()
            .dynamic_scripts
            .claim_page_owned_execution(id)
            .expect(
                "an admitted runtime script must transfer its exact load-delay lease to the page-owned execution task",
            );
        PostParsePageOwnedWork::document_script_work(PageOwnedDocumentScriptWork::Script {
            lane: DocumentScriptExecutionLane::AsyncPhase,
            script: Box::new(script),
            runtime_script_claim: Some(runtime_script_claim),
            source_network_result,
            load_delay_binding: None,
        })
    }

    pub(super) fn emit_ready_runtime_page_owned_work(
        &mut self,
        mut enqueue_work: impl FnMut(PostParsePageOwnedWork),
    ) -> Option<RuntimeScriptOwnerAdvance> {
        self.refresh_script_vm_local_document_state();
        let mut advance = None;
        let dom_content_loaded_dispatched = self.document_runtime.dom_content_loaded_dispatched();
        let runnable = self
            .document_runtime
            .runtime_script_work_mut()
            .dynamic_scripts
            .next_runnable_script_matching(|script| {
                dom_content_loaded_dispatched
                    || dynamic_script_execute_is_runnable_before_dom_content_loaded(
                        &self.document_runtime,
                        script,
                    )
            });
        match runnable {
            Some(DynamicScriptRunnable::Execute {
                id,
                script,
                source_network_result,
            }) if self.prepared_script_uses_runtime_owned_page_task_execution(&script)
                && script.kind == crate::types::ScriptKind::Module =>
            {
                match self.start_runtime_module_script_graph_for_owner(&script, id) {
                    RuntimeModuleScriptGraphStart::Started(actions) => {
                        if let Some(network_result) = source_network_result.as_deref() {
                            self.record_script_subresource_network_result(
                                script.initiator_url.clone(),
                                script.url.clone(),
                                network_result,
                            );
                        }
                        self.commit_runtime_module_graph_start_actions(actions);
                        advance = Some(RuntimeScriptOwnerAdvance::StartedModuleGraph);
                    }
                    RuntimeModuleScriptGraphStart::NotModuleScript => {
                        // `data:` module scripts deliberately use the ordinary
                        // prepared-script executor. They still become a
                        // concrete DocumentScript task; the runtime
                        // continuation must not execute them as a fallback.
                        let work = self.claim_runtime_script_page_owned_execution(
                            id,
                            script,
                            source_network_result,
                        );
                        enqueue_work(work);
                        advance = Some(RuntimeScriptOwnerAdvance::PublishedDocumentScript);
                    }
                }
            }
            Some(DynamicScriptRunnable::Execute {
                id,
                script,
                source_network_result,
            }) if self.prepared_script_uses_runtime_owned_page_task_execution(&script) => {
                let work = self.claim_runtime_script_page_owned_execution(
                    id,
                    script,
                    source_network_result,
                );
                enqueue_work(work);
                advance = Some(RuntimeScriptOwnerAdvance::PublishedDocumentScript);
            }
            Some(DynamicScriptRunnable::Execute {
                id,
                script,
                source_network_result,
            }) => {
                let _ = (id, source_network_result);
                panic!(
                    "production DynamicScriptOwner admissions must carry runtime-owned Page execution identity: {}",
                    script.url
                );
            }
            Some(DynamicScriptRunnable::ContinueModuleScriptGraph { id, continuation }) => {
                self.document_runtime
                    .runtime_script_work_mut()
                    .dynamic_scripts
                    .requeue_module_script_graph_ready_front(id, continuation);
                advance = Some(RuntimeScriptOwnerAdvance::PublishedModuleContinuation);
            }
            Some(DynamicScriptRunnable::ContinueModuleScriptEvaluation { id, evaluation }) => {
                self.document_runtime
                    .runtime_script_work_mut()
                    .dynamic_scripts
                    .requeue_module_script_evaluation_ready_front(id, evaluation);
                advance = Some(RuntimeScriptOwnerAdvance::PublishedModuleContinuation);
            }
            Some(DynamicScriptRunnable::DispatchError {
                id,
                script,
                message,
                module_failure_policy,
                source_network_result,
                error_constructor,
                ..
            }) if self.prepared_script_uses_runtime_owned_page_task_execution(&script) => {
                self.record_runtime_warning(format_args!(
                    "dynamic script load failed for `{}`: {message}",
                    script.url
                ));
                let runtime_script_claim = self
                    .document_runtime
                    .runtime_script_work_mut()
                    .dynamic_scripts
                    .claim_page_owned_execution(id)
                    .expect(
                        "an admitted runtime source failure must transfer its exact load-delay lease to the page-owned terminal task",
                    );
                enqueue_work(PostParsePageOwnedWork::document_script_work(
                    PageOwnedDocumentScriptWork::AsyncSourceFailure {
                        lane: crate::document_script_scheduler::DocumentScriptSourceFailureLane::AsyncPhase,
                        script: Box::new(script),
                        failure: crate::document_script_scheduler::PageOwnedDocumentScriptSourceFailure::runtime_terminal(
                            message,
                            module_failure_policy,
                            error_constructor,
                        ),
                        source_network_result,
                        runtime_script_claim: Some(runtime_script_claim),
                        load_delay_binding: None,
                    },
                ));
                advance = Some(RuntimeScriptOwnerAdvance::PublishedSourceFailure);
            }
            Some(DynamicScriptRunnable::DispatchError {
                id,
                script,
                message,
                kind,
                module_failure_policy,
                source_network_result,
                error_constructor,
            }) => {
                let _ = (
                    id,
                    message,
                    kind,
                    module_failure_policy,
                    source_network_result,
                    error_constructor,
                );
                panic!(
                    "production DynamicScriptOwner failures must carry runtime-owned Page execution identity: {}",
                    script.url
                );
            }
            None => {}
        };
        advance
    }

    pub(crate) fn note_runtime_owned_module_script_graph_fetch_suspended(
        &mut self,
        id: DynamicScriptOwnerId,
        load_ids: Vec<u64>,
        joined_clients: Vec<moli_module_script_tree::SingleModuleClientToken>,
        continuation: Box<ModuleScriptContinuation>,
    ) {
        self.document_runtime
            .runtime_script_work_mut()
            .dynamic_scripts
            .note_module_script_graph_fetch_suspended(id, load_ids, joined_clients, continuation);
    }

    pub(crate) fn take_runtime_owned_module_script_graph_pending_fetch(
        &mut self,
        load_id: u64,
    ) -> Option<ModuleScriptContinuation> {
        self.document_runtime
            .runtime_script_work_mut()
            .dynamic_scripts
            .take_module_script_graph_pending_fetch(load_id)
    }

    pub(crate) fn take_runtime_owned_module_script_graph_pending_joined_client(
        &mut self,
        client: moli_module_script_tree::SingleModuleClientToken,
    ) -> Option<ModuleScriptContinuation> {
        self.document_runtime
            .runtime_script_work_mut()
            .dynamic_scripts
            .take_module_script_graph_pending_joined_client(client)
    }

    pub(crate) fn restore_runtime_owned_module_script_graph_pending_continuation(
        &mut self,
        id: DynamicScriptOwnerId,
        continuation: Box<ModuleScriptContinuation>,
    ) -> bool {
        self.document_runtime
            .runtime_script_work_mut()
            .dynamic_scripts
            .restore_module_script_graph_pending_continuation(id, continuation)
    }

    #[cfg(test)]
    pub(crate) fn has_pending_runtime_owned_module_script_graph(&mut self) -> bool {
        self.document_runtime
            .runtime_script_work_mut()
            .dynamic_scripts
            .has_pending_module_script_graph()
    }

    pub(crate) fn note_runtime_owned_module_script_graph_ready(
        &mut self,
        id: DynamicScriptOwnerId,
        continuation: Box<ModuleScriptContinuation>,
    ) -> bool {
        let ready = self
            .document_runtime
            .runtime_script_work_mut()
            .dynamic_scripts
            .note_module_script_graph_ready(id, continuation);
        if ready {
            let _ = self.enqueue_runtime_owned_module_continuation();
        }
        ready
    }

    pub(crate) fn note_runtime_owned_module_script_evaluation_suspended(
        &mut self,
        id: DynamicScriptOwnerId,
        evaluation: Box<ModuleScriptEvaluationContinuation>,
    ) {
        self.document_runtime
            .runtime_script_work_mut()
            .dynamic_scripts
            .note_module_script_evaluation_suspended(id, evaluation);
    }

    pub(crate) fn enqueue_runtime_owned_module_continuation(&mut self) -> bool {
        let Some(document_owner) = self.current_main_document_task_owner() else {
            self.record_runtime_warning(format_args!(
                "runtime module continuation became ready without a current main Document owner"
            ));
            return false;
        };
        if self.queued_main_document_module_continuation_owner == Some(document_owner) {
            return true;
        }
        let queued = self
            .page_runtime_wake_tx
            .bind_main_document_runtime_continuation(document_owner)
            .send_runtime_module_continuation()
            .is_ok();
        if queued {
            self.queued_main_document_module_continuation_owner = Some(document_owner);
        } else {
            self.record_runtime_warning(format_args!(
                "main-Document runtime route closed before module continuation admission"
            ));
        }
        queued
    }

    pub(crate) fn begin_runtime_owned_module_continuation_turn(
        &mut self,
        document_owner: crate::frame_owner_model::FrameDocumentTaskOwner,
    ) {
        if self.queued_main_document_module_continuation_owner == Some(document_owner) {
            self.queued_main_document_module_continuation_owner = None;
        }
    }

    pub(crate) fn mark_runtime_owned_module_script_evaluation_fulfilled(
        &mut self,
        reaction_id: u64,
    ) -> Option<ModuleScriptEvaluationUpdate> {
        self.document_runtime
            .runtime_script_work_mut()
            .dynamic_scripts
            .mark_module_script_evaluation_fulfilled(reaction_id)
    }

    pub(crate) fn mark_runtime_owned_module_script_evaluation_rejected(
        &mut self,
        reaction_id: u64,
        reason: String,
        error_constructor: Option<crate::types::ScriptErrorConstructorKind>,
    ) -> Option<ModuleScriptEvaluationUpdate> {
        self.document_runtime
            .runtime_script_work_mut()
            .dynamic_scripts
            .mark_module_script_evaluation_rejected(reaction_id, reason, error_constructor)
    }

    #[cfg(test)]
    pub(crate) fn has_pending_runtime_owned_module_script_evaluation(&mut self) -> bool {
        self.document_runtime
            .runtime_script_work_mut()
            .dynamic_scripts
            .has_pending_module_script_evaluation()
    }

    pub(crate) fn take_ready_runtime_owned_module_script_continuation_work(
        &mut self,
    ) -> Option<DynamicModuleScriptContinuationWork> {
        self.refresh_script_vm_local_document_state();
        self.document_runtime
            .runtime_script_work_mut()
            .dynamic_scripts
            .take_ready_module_script_continuation()
    }

    pub(super) fn send_ready_runtime_tasks(&mut self) -> Option<RuntimeScriptOwnerAdvance> {
        let mut work_items = Vec::new();
        let materialized = self.emit_ready_runtime_page_owned_work(|work| {
            work_items.push(work);
        });
        for work in work_items {
            self.enqueue_main_document_runtime_work(work);
        }
        if self.has_ready_runtime_owned_module_owner_actions() {
            let _ = self.enqueue_runtime_owned_module_continuation();
        }
        materialized
    }

    fn has_main_document_runtime_route(&self) -> bool {
        self.page_runtime_wake_tx.has_main_document_runtime_route()
    }

    /// Publish concrete runtime follow-ups produced by one selected
    /// post-parse action.
    ///
    /// This is an action-settlement boundary, not a generic readiness scan:
    /// lifecycle polling and protocol waits must never call it to discover
    /// hidden work. Runtime/module producers outside this boundary publish
    /// their own typed continuation directly.
    fn publish_post_parse_action_runtime_followups(&mut self) {
        debug_assert!(
            self.has_main_document_runtime_route(),
            "post-parse action settlement requires the stable Page route"
        );
        if self.has_ready_runtime_owned_module_owner_actions() {
            let _ = self.enqueue_runtime_owned_module_continuation();
        }
        self.enqueue_runtime_script_signal_if_needed();
    }

    async fn flush_pending_runtime_script_work_until_generation_stable(
        &mut self,
        loader: &ResourceRequestClient,
        initial_wait_for_dynamic_loads: bool,
        yield_after_one_runnable: bool,
        pause_kind_on_yield: Option<RuntimeScriptWorkPauseKind>,
    ) -> std::result::Result<RuntimePendingWorkFlushOutcome, String> {
        let mut wait_for_dynamic_loads = initial_wait_for_dynamic_loads;
        loop {
            let generation_before = self.document_runtime.runtime_reset_generation();
            let outcome = self
                .flush_pending_work_with_turn_budget(
                    loader,
                    wait_for_dynamic_loads,
                    yield_after_one_runnable,
                )
                .await?;
            if self.document_runtime.runtime_reset_generation() != generation_before {
                // A script replaced the Document while this bounded flush was
                // running. Reconcile the new exact runtime owner before
                // publishing an outcome for the post-parse caller.
                wait_for_dynamic_loads = true;
                continue;
            }
            if let Some(pause_kind) = pause_kind_on_yield
                && yield_after_one_runnable
                && self.has_pending_runtime_script_work()
                && !self
                    .document_runtime
                    .runtime_script_work_mut()
                    .is_paused_for_deferred_page_tasks()
            {
                match pause_kind {
                    RuntimeScriptWorkPauseKind::StablePageTurnContinuation => {
                        self.pause_runtime_script_work_for_stable_continuation();
                    }
                }
            }
            return Ok(outcome);
        }
    }

    fn prepare_post_parse_round_page_owned_work(
        &mut self,
        work: Vec<PostParsePageOwnedWork>,
    ) -> Vec<PostParsePageOwnedWork> {
        if self.document_runtime.take_post_parse_schedule_rebuild() {
            return self.prepare_rebuilt_post_parse_round_page_owned_work();
        }
        let mut work = self.prepare_main_document_lifecycle_page_owned_work(work);
        self.bind_post_parse_page_owned_script_handles(&mut work);
        work
    }

    fn rebuild_post_parse_round_page_owned_work(&mut self) -> Option<Vec<PostParsePageOwnedWork>> {
        self.document_runtime
            .take_post_parse_schedule_rebuild()
            .then(|| self.prepare_rebuilt_post_parse_round_page_owned_work())
    }

    fn prepare_rebuilt_post_parse_round_page_owned_work(&mut self) -> Vec<PostParsePageOwnedWork> {
        // document.open() already replaced the old Document's owner source.
        // Do not clear it again here: the replacement live parser can publish
        // new-owner work before this invalidation is observed.
        self.refresh_script_vm_local_document_state();
        let mut work = self.prepare_main_document_lifecycle_page_owned_work(Vec::new());
        self.bind_post_parse_page_owned_script_handles(&mut work);
        work
    }

    fn prepare_main_document_lifecycle_page_owned_work(
        &mut self,
        mut work: Vec<PostParsePageOwnedWork>,
    ) -> Vec<PostParsePageOwnedWork> {
        let Some(owner) = self.current_main_document_task_owner() else {
            self.record_runtime_warning(format_args!(
                "main post-parse lifecycle has no current document owner"
            ));
            return work;
        };
        // The live document.open()/Page.setDocumentContent parser accepts
        // defer-like scripts as it encounters them, but unlike phase one it
        // has no separate parser-to-phase-two handoff. Finalize that same
        // parser-owned queue here, at the common EOF-to-lifecycle boundary.
        // The legacy pending-document path already seals and arms its queue
        // while projecting handoffs, so do not create a second marker for it.
        if self.document_runtime.main_parser_deferred_scripts_owner() != Some(owner) {
            if let Err(error) = self.start_pending_main_parser_deferred_scripts() {
                self.record_runtime_warning(format_args!(
                    "failed to start replacement parser-deferred scripts: {error}"
                ));
            }
            if let Some(marker) = self.seal_main_parser_deferred_scripts(owner) {
                work.push(marker);
            }
        }
        let interactive = self.finish_current_main_document_parsing(owner);
        let mut work = self
            .document_runtime
            .prepare_post_parse_lifecycle_page_owned_work(owner, work);
        if let Some(interactive) = interactive {
            tracing::debug!(
                ?owner,
                "queued document-owned main interactive transition before deferred scripts"
            );
            work.insert(
                0,
                PostParsePageOwnedWork::main_document_interactive(interactive),
            );
        }
        work
    }

    pub(super) fn rebuild_post_parse_round_page_owned_work_if_invalidated(
        &mut self,
    ) -> Option<Vec<PostParsePageOwnedWork>> {
        self.rebuild_post_parse_round_page_owned_work()
    }

    pub(super) async fn restart_post_parse_lifecycle_round_if_invalidated(
        &mut self,
        page_task_queue: &mut PageTaskQueue,
        report: &mut ScriptExecutionReport,
    ) -> bool {
        let Some(work) = self.rebuild_post_parse_round_page_owned_work_if_invalidated() else {
            return false;
        };
        page_task_queue.clear_document_owned_tasks();
        let _ = self.begin_post_parse_lifecycle_round(page_task_queue, report, work);
        true
    }

    pub(crate) async fn start_post_parse_lifecycle_round(
        &mut self,
        stage: crate::renderer::PageVmInitStage,
        page_task_queue: &mut PageTaskQueue,
        report: &mut ScriptExecutionReport,
        work: Vec<PostParsePageOwnedWork>,
    ) -> PostParseLifecycleDriver {
        let work = self.prepare_post_parse_round_page_owned_work(work);
        let round = self.begin_post_parse_lifecycle_round(page_task_queue, report, work);
        PostParseLifecycleDriver::new(stage, round)
    }

    pub(crate) fn resume_post_parse_lifecycle_driver_for_existing_queue(
        &self,
        stage: crate::renderer::PageVmInitStage,
    ) -> PostParseLifecycleDriver {
        let round = PostParseLifecycleRound {
            queue_stats: PostParseLifecycleQueueStats::default(),
            phase_started: Instant::now(),
        };
        PostParseLifecycleDriver::new(stage, round)
    }

    pub(super) async fn prepare_post_parse_task_execution(
        &mut self,
        page_task_queue: &mut PageTaskQueue,
        report: &mut ScriptExecutionReport,
        token: PostParseTaskExecutionToken,
    ) -> bool {
        self.restart_post_parse_lifecycle_round_if_task_allows_invalidation(
            page_task_queue,
            report,
            token,
        )
        .await
    }

    async fn restart_post_parse_lifecycle_round_if_task_allows_invalidation(
        &mut self,
        page_task_queue: &mut PageTaskQueue,
        report: &mut ScriptExecutionReport,
        token: PostParseTaskExecutionToken,
    ) -> bool {
        token.allows_invalidation_restart()
            && self
                .restart_post_parse_lifecycle_round_if_invalidated(page_task_queue, report)
                .await
    }

    async fn complete_post_parse_page_owned_task(
        &mut self,
        page_task_queue: &mut PageTaskQueue,
        report: &mut ScriptExecutionReport,
        task: PostParsePageOwnedTask,
    ) -> Option<PostParseLifecycleCompletionAction> {
        let token = task.completion.token;
        if self
            .restart_post_parse_lifecycle_round_if_task_allows_invalidation(
                page_task_queue,
                report,
                token,
            )
            .await
        {
            // A same-document DCL handler can invalidate the post-parse round by
            // scheduling post-DCL work. Restart the round so stale tasks are not
            // reused, but do not make a DCL-boundary wait consume the new
            // post-DCL backlog.
            if matches!(
                token.boundary_completion,
                Some(PostParseLifecycleCompletionAction::ReturnAtStage(
                    "DOMContentLoaded"
                ))
            ) && self.document_runtime.dom_content_loaded_dispatched()
            {
                return token.boundary_completion;
            }
            return None;
        }
        if token.requires_runtime_followup_publication {
            self.finish_executed_post_parse_action(page_task_queue, true);
        }
        token.boundary_completion
    }

    pub(super) async fn finish_completed_post_parse_page_owned_task_or_continue(
        &mut self,
        page_task_queue: &mut PageTaskQueue,
        report: &mut ScriptExecutionReport,
        completed_task: Option<PostParsePageOwnedTask>,
    ) -> std::result::Result<Option<PostParseLifecycleAdvance>, String> {
        let Some(task) = completed_task else {
            return Ok(None);
        };
        let completion_action = self
            .complete_post_parse_page_owned_task(page_task_queue, report, task)
            .await;
        Ok(completion_action.map(PostParseLifecycleAdvance::Complete))
    }

    fn take_exact_domcontentloaded_after_main_parser_finish(
        &mut self,
        page_task_queue: &mut PageTaskQueue,
        owner: FrameDocumentTaskOwner,
    ) -> Option<PostParsePageOwnedWork> {
        if self.current_main_document_task_owner() != Some(owner) {
            tracing::debug!(
                ?owner,
                current_owner = ?self.current_main_document_task_owner(),
                "discarded stale parser-finish permit before DOMContentLoaded claim"
            );
            return None;
        }
        if self
            ._context_host
            .borrow()
            .current_main_document_domcontentloaded_transition_is_ready(owner)
            != Some(true)
        {
            tracing::debug!(
                ?owner,
                "parser-finish permit found no ready DOMContentLoaded transition"
            );
            return None;
        }

        let exact_dcl_is_front = page_task_queue.post_parse_front().is_some_and(|work| {
            work.is_domcontentloaded_task() && work.main_document_lifecycle_owner() == Some(owner)
        });
        if !exact_dcl_is_front {
            tracing::debug!(
                ?owner,
                "parser-finish permit found no exact DOMContentLoaded queue successor"
            );
            return None;
        }
        Some(
            page_task_queue
                .post_parse_pop_front()
                .expect("an observed exact DOMContentLoaded successor must remain queued"),
        )
    }

    /// Claim the exact parse-time DOMContentLoaded successor of a drained
    /// main-parser queue. Queue inspection and removal remain inside the
    /// lifecycle authority; the caller receives only already-claimed work.
    pub(crate) fn claim_parse_time_domcontentloaded_after_main_parser_finish(
        &mut self,
        page_task_queue: &mut PageTaskQueue,
        owner: FrameDocumentTaskOwner,
    ) -> Option<ParserFinishDomContentLoadedWork> {
        if self.has_pending_location_navigation() {
            return None;
        }
        let work =
            self.take_exact_domcontentloaded_after_main_parser_finish(page_task_queue, owner)?;
        let PostParsePageOwnedWork::Lifecycle(work) = work else {
            unreachable!("an exact DOMContentLoaded successor must be lifecycle work");
        };
        Some(ParserFinishDomContentLoadedWork::new(owner, *work))
    }

    /// Consume the completed parser task and claim only its exact, already
    /// queued DOMContentLoaded successor.
    ///
    /// This is intentionally not a lifecycle poll. It neither scans for work
    /// nor considers runtime candidates: the caller already holds a one-shot
    /// parser-finish permit, and this method accepts exactly one same-Document
    /// DCL action from the existing authoritative queue.
    pub(crate) async fn claim_domcontentloaded_after_main_parser_finish(
        &mut self,
        page_task_queue: &mut PageTaskQueue,
        report: &mut ScriptExecutionReport,
        driver: PostParseLifecycleDriver,
        completed_parser_task: PostParsePageOwnedTask,
        owner: FrameDocumentTaskOwner,
    ) -> std::result::Result<Option<ParserFinishDomContentLoadedTask>, String> {
        let boundary_completion = self
            .complete_post_parse_page_owned_task(page_task_queue, report, completed_parser_task)
            .await;
        if boundary_completion.is_some() {
            return Err(
                "a parser-deferred task unexpectedly completed the lifecycle boundary".to_owned(),
            );
        }

        let Some(work) =
            self.take_exact_domcontentloaded_after_main_parser_finish(page_task_queue, owner)
        else {
            return Ok(None);
        };
        let execution =
            driver.task_execution_for_action(PostParseProcessingAction::from_page_owned_work(work));
        if self
            .prepare_post_parse_task_execution(page_task_queue, report, execution.token)
            .await
        {
            tracing::debug!(
                ?owner,
                "parser-finish DOMContentLoaded claim invalidated the lifecycle round"
            );
            return Ok(None);
        }
        Ok(Some(ParserFinishDomContentLoadedTask::new(
            owner,
            execution.into_page_owned_task(),
        )))
    }

    pub(super) fn begin_post_parse_lifecycle_round(
        &mut self,
        page_task_queue: &mut PageTaskQueue,
        report: &mut ScriptExecutionReport,
        work: Vec<PostParsePageOwnedWork>,
    ) -> PostParseLifecycleRound {
        let queue_stats = post_parse_lifecycle_queue_stats(&work);
        let context_host = self._context_host.clone();
        let host_ptr = context_host.as_ref().as_ptr();
        let context_host = context_host.borrow();
        self.document_runtime
            .queue_initial_connected_style_loads_with_source_lookup(host_ptr, |owner| {
                context_host.owner_style_sheet_processing_source(owner)
            });
        self.document_runtime.drain_document_processing_wakes();
        self.document_runtime
            .enqueue_post_parse_lifecycle_page_owned_work(page_task_queue, work, report);
        PostParseLifecycleRound {
            queue_stats,
            phase_started: Instant::now(),
        }
    }

    pub(super) fn bind_post_parse_page_owned_script_handles(
        &mut self,
        work: &mut [PostParsePageOwnedWork],
    ) {
        for item in work {
            if let Some(script) = item.as_script_mut() {
                self.bind_prepared_script_handle_if_needed(script, ScriptHandleSource::ParserOwned);
            }
        }
    }

    pub(crate) fn has_post_domcontentloaded_runtime_work_for_wait(&mut self) -> bool {
        self.refresh_script_vm_local_document_state();
        self.document_runtime.dom_content_loaded_dispatched()
            && self.has_pending_runtime_script_work()
    }

    pub(crate) fn check_main_document_completion(
        &self,
        owner: crate::frame_owner_model::FrameDocumentTaskOwner,
    ) -> Option<bool> {
        self._context_host
            .borrow()
            .current_main_document_complete_transition_is_ready(owner)
    }

    #[cfg(test)]
    pub(crate) fn execute_post_parse_lifecycle_work_best_effort(
        &mut self,
        work: PostParseLifecycleWork,
    ) -> std::result::Result<NonScriptPageTaskExecutionOutcome, String> {
        if let Some(body) = MainDocumentLifecycleBody::from_post_parse_work(&work) {
            return self
                .execute_main_document_lifecycle_body(body)
                .into_legacy_post_parse_outcome();
        }
        let result = self.execute_post_parse_lifecycle_work_best_effort_inner(work);
        self.finish_runtime_turn_with_style_drain(
            crate::style_engine::StyleInvalidationTurnExitBoundary::NonScriptPageTask,
            result,
        )
    }

    #[cfg(test)]
    fn execute_post_parse_lifecycle_work_best_effort_inner(
        &mut self,
        work: PostParseLifecycleWork,
    ) -> std::result::Result<NonScriptPageTaskExecutionOutcome, String> {
        match work {
            PostParseLifecycleWork::SeedDocumentOwnedBlockingStylesheets(inputs) => {
                let Some(owner) = self.current_main_document_task_owner() else {
                    tracing::warn!(
                        input_count = inputs.len(),
                        "dropping stylesheet seed without current main document owner"
                    );
                    return Ok(NonScriptPageTaskExecutionOutcome::None);
                };
                self.accept_main_document_blocking_stylesheet_inputs(owner, &inputs);
                Ok(NonScriptPageTaskExecutionOutcome::None)
            }
            PostParseLifecycleWork::AdvanceMainParserDeferredScripts { .. } => {
                unreachable!(
                    "main parser-deferred adapter work must use async page-owned execution"
                )
            }
            PostParseLifecycleWork::RecordDocumentScriptRun { .. } => {
                Ok(NonScriptPageTaskExecutionOutcome::None)
            }
            PostParseLifecycleWork::DispatchContentSecurityPolicyViolation(task) => {
                self.dispatch_content_security_policy_violation_event_page_task_best_effort(&task);
                self.drain_deferred_page_tasks_best_effort();
                Ok(NonScriptPageTaskExecutionOutcome::None)
            }
            PostParseLifecycleWork::DispatchScriptEvent(task) => {
                self.dispatch_script_event_and_checkpoint_for_test(&task);
                self.drain_deferred_page_tasks_best_effort();
                Ok(NonScriptPageTaskExecutionOutcome::None)
            }
            PostParseLifecycleWork::ReportWindowScriptFailure(task) => {
                self.report_window_script_failure_task_and_checkpoint_for_test(&task);
                self.drain_deferred_page_tasks_best_effort();
                Ok(NonScriptPageTaskExecutionOutcome::None)
            }
            PostParseLifecycleWork::SettleMainDocumentScriptLoadDelay(binding) => {
                let owner = binding.owner();
                let kind = binding.kind();
                let load_delay_token = binding.load_delay_token();
                let settled = self
                    ._context_host
                    .borrow_mut()
                    .release_main_document_script_load_delay(binding);
                debug!(
                    ?owner,
                    ?kind,
                    ?load_delay_token,
                    settled = ?settled,
                            "settled main async script lifecycle binding after observable follow-up"
                );
                Ok(NonScriptPageTaskExecutionOutcome::None)
            }
            PostParseLifecycleWork::ApplyMainDocumentInteractive(_)
            | PostParseLifecycleWork::DispatchDomContentLoaded { .. }
            | PostParseLifecycleWork::DispatchWindowLoad { .. } => unreachable!(
                "main-document lifecycle work must retain its typed body execution fact"
            ),
            PostParseLifecycleWork::CheckMainDocumentCompletion { owner } => {
                unreachable!(
                    "main document completion rechecks must not enter the V8 lifecycle executor: {owner:?}"
                )
            }
            PostParseLifecycleWork::RecordDetachedPostParseRuns(_) => {
                Ok(NonScriptPageTaskExecutionOutcome::None)
            }
            PostParseLifecycleWork::DispatchConnectedStyleLoad(_) => {
                unreachable!(
                    "connected style load lifecycle work must use async page-owned execution path"
                )
            }
        }
    }

    pub(crate) fn finish_runtime_owned_script_success(
        &mut self,
        dynamic_script_owner_id: DynamicScriptOwnerId,
        script: &PreparedScript,
    ) {
        let lease = self
            .document_runtime
            .runtime_script_work_mut()
            .dynamic_scripts
            .finish_script_terminal(dynamic_script_owner_id);
        let Some(lease) = self.exact_runtime_script_terminal_lease_or_warn(
            dynamic_script_owner_id,
            script,
            lease,
        ) else {
            return;
        };
        self.apply_runtime_script_success_terminal(script, lease);
        debug!(
            ?dynamic_script_owner_id,
            url = %script.url,
            "completed runtime-owned script terminal inside its selected action"
        );
    }

    /// Apply the synchronous terminal portion of a selected DocumentScript
    /// body without claiming that the surrounding HTML task has ended.
    ///
    /// The caller owns the task-end checkpoint and lifecycle prime. This keeps
    /// `script -> script reactions -> terminal callback -> terminal reactions`
    /// observable while preventing the terminal helper from becoming a second
    /// completion authority.
    pub(crate) fn finish_claimed_runtime_owned_script_success_body(
        &mut self,
        claim: DynamicScriptPageTaskClaim,
        script: &PreparedScript,
    ) -> crate::script_vm::ScriptTerminalBodyActivity {
        let (dynamic_script_owner_id, lease) = claim.into_parts();
        let activity = self.apply_runtime_script_success_terminal_body(script, lease);
        debug!(
            ?dynamic_script_owner_id,
            url = %script.url,
            ?activity,
            "completed claimed runtime-owned script success body"
        );
        activity
    }

    pub(crate) fn finish_runtime_owned_script_success_body(
        &mut self,
        dynamic_script_owner_id: DynamicScriptOwnerId,
        script: &PreparedScript,
    ) -> crate::script_vm::ScriptTerminalBodyActivity {
        let lease = self
            .document_runtime
            .runtime_script_work_mut()
            .dynamic_scripts
            .finish_script_terminal(dynamic_script_owner_id);
        let Some(lease) = self.exact_runtime_script_terminal_lease_or_warn(
            dynamic_script_owner_id,
            script,
            lease,
        ) else {
            return crate::script_vm::ScriptTerminalBodyActivity::NoEventDispatch;
        };
        self.apply_runtime_script_success_terminal_body(script, lease)
    }

    pub(crate) fn restore_runtime_owned_script_page_task_claim(
        &mut self,
        claim: DynamicScriptPageTaskClaim,
    ) {
        self.document_runtime
            .runtime_script_work_mut()
            .dynamic_scripts
            .restore_page_owned_execution_claim(claim);
    }

    pub(crate) fn finish_claimed_runtime_owned_script_failure_body(
        &mut self,
        claim: DynamicScriptPageTaskClaim,
        script: &PreparedScript,
        message: &str,
        module_failure_policy: Option<crate::host::ModuleFailurePolicy>,
        error_constructor: Option<crate::types::ScriptErrorConstructorKind>,
    ) -> crate::script_vm::ScriptTerminalBodyActivity {
        let (dynamic_script_owner_id, lease) = claim.into_parts();
        let settlement = self.apply_runtime_script_failure_terminal_body(
            script,
            message,
            module_failure_policy,
            error_constructor,
            lease,
        );
        debug!(
            ?dynamic_script_owner_id,
            url = %script.url,
            activity = ?settlement.activity,
            "completed claimed runtime-owned script failure body"
        );
        settlement.activity
    }

    pub(crate) fn cancel_claimed_runtime_owned_script_load_delay_body(
        &mut self,
        claim: DynamicScriptPageTaskClaim,
        script: &PreparedScript,
    ) {
        let (dynamic_script_owner_id, lease) = claim.into_parts();
        self.release_runtime_script_load_delay_lease_body(script, lease);
        debug!(
            ?dynamic_script_owner_id,
            url = %script.url,
            "released claimed runtime-owned script load-delay lease inside selected body"
        );
    }

    pub(super) fn apply_runtime_script_success_terminal(
        &mut self,
        script: &PreparedScript,
        lease: MainDocumentScriptLoadDelayLease,
    ) {
        if let Some(PostParseLifecycleWork::DispatchScriptEvent(task)) =
            self.plan_script_load_lifecycle_work_for_prepared_script(script)
        {
            self.dispatch_runtime_script_terminal_event_and_finish_checkpoint_best_effort(&task);
        }
        self.release_runtime_script_load_delay_lease(script, lease);
    }

    pub(super) fn apply_runtime_script_failure_terminal(
        &mut self,
        script: &PreparedScript,
        message: &str,
        module_failure_policy: Option<crate::host::ModuleFailurePolicy>,
        error_constructor: Option<crate::types::ScriptErrorConstructorKind>,
        lease: MainDocumentScriptLoadDelayLease,
    ) {
        let work = self.document_runtime.plan_script_failure_lifecycle_work(
            script,
            message,
            module_failure_policy,
            error_constructor,
        );
        for terminal in work {
            match terminal {
                PostParseLifecycleWork::DispatchScriptEvent(task) => {
                    self.dispatch_runtime_script_terminal_event_and_finish_checkpoint_best_effort(
                        &task,
                    );
                }
                PostParseLifecycleWork::ReportWindowScriptFailure(task) => {
                    self.report_runtime_script_failure_terminal_and_finish_checkpoint_best_effort(
                        &task,
                    );
                }
                other => unreachable!(
                    "runtime script failure planning produced non-terminal work: {other:?}"
                ),
            }
        }
        self.release_runtime_script_load_delay_lease(script, lease);
    }

    fn apply_runtime_script_success_terminal_body(
        &mut self,
        script: &PreparedScript,
        lease: MainDocumentScriptLoadDelayLease,
    ) -> crate::script_vm::ScriptTerminalBodyActivity {
        let activity = if let Some(PostParseLifecycleWork::DispatchScriptEvent(task)) =
            self.plan_script_load_lifecycle_work_for_prepared_script(script)
        {
            self.dispatch_script_event_body_best_effort(&task);
            crate::script_vm::ScriptTerminalBodyActivity::EventDispatchAttempted
        } else {
            crate::script_vm::ScriptTerminalBodyActivity::NoEventDispatch
        };
        self.release_runtime_script_load_delay_lease_body(script, lease);
        activity
    }

    fn apply_runtime_script_failure_terminal_body(
        &mut self,
        script: &PreparedScript,
        message: &str,
        module_failure_policy: Option<crate::host::ModuleFailurePolicy>,
        error_constructor: Option<crate::types::ScriptErrorConstructorKind>,
        lease: MainDocumentScriptLoadDelayLease,
    ) -> RuntimeScriptFailureTerminalBodySettlement {
        let work = self.document_runtime.plan_script_failure_lifecycle_work(
            script,
            message,
            module_failure_policy,
            error_constructor,
        );
        let mut activity = crate::script_vm::ScriptTerminalBodyActivity::NoEventDispatch;
        for terminal in work {
            match terminal {
                PostParseLifecycleWork::DispatchScriptEvent(task) => {
                    self.dispatch_script_event_body_best_effort(&task);
                    activity = crate::script_vm::ScriptTerminalBodyActivity::EventDispatchAttempted;
                }
                PostParseLifecycleWork::ReportWindowScriptFailure(task) => {
                    self.report_window_error_body_best_effort(
                        &task.message,
                        task.filename.as_deref(),
                        task.error_constructor,
                    );
                    activity = crate::script_vm::ScriptTerminalBodyActivity::EventDispatchAttempted;
                }
                other => unreachable!(
                    "runtime script failure planning produced non-terminal work: {other:?}"
                ),
            }
        }
        let load_gate_release = self.release_runtime_script_load_delay_lease_body(script, lease);
        RuntimeScriptFailureTerminalBodySettlement {
            activity,
            load_gate_release,
        }
    }

    pub(super) fn release_runtime_script_load_delay_lease(
        &mut self,
        script: &PreparedScript,
        lease: MainDocumentScriptLoadDelayLease,
    ) {
        let release = self.release_runtime_script_load_delay_lease_body(script, lease);
        self.prime_lifecycle_after_script_load_gate_release(release);
    }

    fn release_runtime_script_load_delay_lease_body(
        &mut self,
        script: &PreparedScript,
        lease: MainDocumentScriptLoadDelayLease,
    ) -> MainDocumentScriptLoadDelayRelease {
        let owner = lease.owner();
        let kind = lease.kind();
        let load_delay_token = lease.load_delay_token();
        let release = self
            ._context_host
            .borrow_mut()
            .release_main_document_script_load_delay(lease);
        debug!(
            ?owner,
            ?kind,
            ?load_delay_token,
            ?release,
            script_node_id = ?script.node_id,
            script_url = %script.url,
            "released runtime script load-delay lease after observable terminal body"
        );
        release
    }

    pub(super) fn exact_runtime_script_terminal_lease_or_warn(
        &mut self,
        dynamic_script_owner_id: DynamicScriptOwnerId,
        script: &PreparedScript,
        lease: Option<MainDocumentScriptLoadDelayLease>,
    ) -> Option<MainDocumentScriptLoadDelayLease> {
        if lease.is_none() {
            tracing::warn!(
                ?dynamic_script_owner_id,
                script_node_id = ?script.node_id,
                script_url = %script.url,
                "ignored runtime script terminal without its exact load-delay lease"
            );
        }
        lease
    }

    fn prime_lifecycle_after_script_load_gate_release(
        &mut self,
        release: MainDocumentScriptLoadDelayRelease,
    ) {
        if release == MainDocumentScriptLoadDelayRelease::BecameUnblocked {
            self.prime_document_lifecycle_processing_and_record_stylesheet_network_results();
        }
    }

    pub(crate) fn accept_main_document_script_load_delay_binding(
        &mut self,
        owner: FrameDocumentTaskOwner,
        kind: MainDocumentScriptLoadDelayKind,
    ) -> Option<MainDocumentScriptLoadDelayLease> {
        self._context_host
            .borrow_mut()
            .acquire_current_main_document_script_load_delay(owner, kind)
    }

    pub(crate) fn cancel_runtime_owned_script_load_delay_body(
        &mut self,
        script: &PreparedScript,
        dynamic_script_owner_id: DynamicScriptOwnerId,
    ) {
        let lease = self
            .document_runtime
            .runtime_script_work_mut()
            .dynamic_scripts
            .finish_script_terminal(dynamic_script_owner_id);
        let Some(lease) = lease else {
            return;
        };
        let release = self.release_runtime_script_load_delay_lease_body(script, lease);
        debug!(
            ?dynamic_script_owner_id,
            ?release,
            url = %script.url,
            "released dropped runtime-owned script load-delay lease inside selected body"
        );
    }

    pub(crate) fn finish_runtime_owned_script_failure(
        &mut self,
        dynamic_script_owner_id: Option<DynamicScriptOwnerId>,
        script: &PreparedScript,
        message: &str,
    ) {
        self.finish_runtime_owned_script_failure_with_kind(
            dynamic_script_owner_id,
            script,
            message,
            crate::dynamic_script_owner::DynamicScriptOwner::legacy_message_failure_kind(
                script, message,
            ),
            None,
            None,
        );
    }

    pub(crate) fn finish_runtime_owned_script_failure_body_with_kind(
        &mut self,
        dynamic_script_owner_id: Option<DynamicScriptOwnerId>,
        script: &PreparedScript,
        message: &str,
        kind: crate::dynamic_script_owner::DynamicScriptFailureKind,
        module_failure_policy: Option<crate::host::ModuleFailurePolicy>,
        error_constructor: Option<crate::types::ScriptErrorConstructorKind>,
    ) -> crate::script_vm::ScriptTerminalBodyActivity {
        if let Some(id) = dynamic_script_owner_id {
            if kind == crate::dynamic_script_owner::DynamicScriptFailureKind::Immediate {
                let lease = self
                    .document_runtime
                    .runtime_script_work_mut()
                    .dynamic_scripts
                    .finish_script_terminal(id);
                let Some(lease) =
                    self.exact_runtime_script_terminal_lease_or_warn(id, script, lease)
                else {
                    return crate::script_vm::ScriptTerminalBodyActivity::NoEventDispatch;
                };
                return self
                    .apply_runtime_script_failure_terminal_body(
                        script,
                        message,
                        module_failure_policy,
                        error_constructor,
                        lease,
                    )
                    .activity;
            }
            self.document_runtime
                .runtime_script_work_mut()
                .dynamic_scripts
                .note_script_failed_with_kind_and_error_constructor(
                    id,
                    script,
                    message.to_owned(),
                    kind,
                    module_failure_policy,
                    error_constructor,
                );
            self.enqueue_immediate_runtime_script_work_if_needed();
            return crate::script_vm::ScriptTerminalBodyActivity::NoEventDispatch;
        }

        if kind == crate::dynamic_script_owner::DynamicScriptFailureKind::Immediate {
            let dispatch_current_script_error_immediately = script.source_kind
                == ScriptSourceKind::External
                && self.prepared_script_uses_runtime_owned_page_task_execution(script);
            if dispatch_current_script_error_immediately {
                let activity = self.dispatch_current_prepared_script_error_body_best_effort(
                    script,
                    message,
                    module_failure_policy,
                    error_constructor,
                );
                self.enqueue_immediate_runtime_script_work_if_needed();
                return activity;
            }
            match self.enqueue_script_failure_lifecycle_work_for_prepared_script(
                script,
                message,
                module_failure_policy,
                error_constructor,
            ) {
                Ok(FollowupPageTaskDisposition::Skipped) => {}
                Ok(
                    disposition @ (FollowupPageTaskDisposition::Deferred
                    | FollowupPageTaskDisposition::Enqueued),
                ) => {
                    if self.pause_runtime_script_work_at_followup_task_boundary(disposition) {
                        return crate::script_vm::ScriptTerminalBodyActivity::NoEventDispatch;
                    }
                }
                Err(error) => {
                    self.record_runtime_warning(format_args!(
                        "script failure page-task enqueue failed for `{}`: {error}",
                        script.url
                    ));
                }
            }
            self.enqueue_immediate_runtime_script_work_if_needed();
            return crate::script_vm::ScriptTerminalBodyActivity::NoEventDispatch;
        }

        panic!("runtime-owned deferrable script failure should carry dynamic owner id");
    }

    /// Records a deferrable module-graph failure without publishing a second
    /// concrete runtime continuation yet.
    ///
    /// The selected graph-terminal action owns a bounded settlement pass after
    /// all of its failures have been recorded. Keeping publication out of this
    /// method lets that pass preserve DynamicScriptOwner's lane ordering while
    /// still dispatching every now-runnable error in the same owner action.
    pub(crate) fn record_runtime_owned_module_failure_for_selected_action(
        &mut self,
        dynamic_script_owner_id: DynamicScriptOwnerId,
        script: &PreparedScript,
        message: String,
        kind: crate::dynamic_script_owner::DynamicScriptFailureKind,
        module_failure_policy: Option<crate::host::ModuleFailurePolicy>,
        error_constructor: Option<crate::types::ScriptErrorConstructorKind>,
    ) {
        debug_assert!(
            kind.is_deferrable_module(),
            "selected-action recording is only for ordered module graph failures"
        );
        self.document_runtime
            .runtime_script_work_mut()
            .dynamic_scripts
            .note_script_failed_with_kind_and_error_constructor(
                dynamic_script_owner_id,
                script,
                message,
                kind,
                module_failure_policy,
                error_constructor,
            );
    }

    /// Settles only error terminals carried by the currently selected owner
    /// action. DynamicScriptOwner remains the ordering authority; encountering
    /// any unrelated head stops this pass instead of draining it.
    fn take_runtime_owned_module_failure_terminal_for_selected_action(
        &mut self,
        action_owner_ids: &[DynamicScriptOwnerId],
    ) -> Option<(
        crate::dynamic_script_owner::DynamicScriptFailureTerminal,
        Option<MainDocumentScriptLoadDelayLease>,
    )> {
        let terminal = self
            .document_runtime
            .runtime_script_work_mut()
            .dynamic_scripts
            .take_runnable_failure_terminal_for_action(action_owner_ids)?;
        self.record_runtime_warning(format_args!(
            "dynamic script load failed for `{}`: {}",
            terminal.script.url, terminal.message
        ));
        if let Some(network_result) = terminal.source_network_result.as_deref() {
            self.record_script_subresource_network_result(
                terminal.script.initiator_url.clone(),
                terminal.script.url.clone(),
                network_result,
            );
        }
        let lease = self
            .document_runtime
            .runtime_script_work_mut()
            .dynamic_scripts
            .finish_script_terminal(terminal.id);
        let lease =
            self.exact_runtime_script_terminal_lease_or_warn(terminal.id, &terminal.script, lease);
        Some((terminal, lease))
    }

    pub(crate) fn settle_runtime_owned_module_failures_for_selected_action(
        &mut self,
        action_owner_ids: &[DynamicScriptOwnerId],
    ) -> usize {
        let mut settled = 0;
        loop {
            let Some((terminal, lease)) = self
                .take_runtime_owned_module_failure_terminal_for_selected_action(action_owner_ids)
            else {
                break;
            };
            let crate::dynamic_script_owner::DynamicScriptFailureTerminal {
                id,
                script,
                message,
                kind,
                module_failure_policy,
                source_network_result: _,
                error_constructor,
            } = terminal;
            if let Some(lease) = lease {
                self.apply_runtime_script_failure_terminal(
                    &script,
                    &message,
                    module_failure_policy,
                    error_constructor,
                    lease,
                );
            }
            debug!(
                dynamic_script_owner_id = ?id,
                url = %script.url,
                ?kind,
                "settled runtime module failure inside its selected graph-terminal action"
            );
            settled += 1;
        }
        settled
    }

    /// Body-only counterpart used by the selected ResourceCompletion task.
    /// It preserves DynamicScriptOwner ordering, but neither performs a
    /// microtask checkpoint nor primes lifecycle. Those obligations are
    /// returned to the unique Page task-end coordinator.
    pub(crate) fn settle_runtime_owned_module_failures_for_selected_action_body(
        &mut self,
        action_owner_ids: &[DynamicScriptOwnerId],
    ) -> RuntimeOwnedModuleFailureBodySettlement {
        let mut settlement = RuntimeOwnedModuleFailureBodySettlement::none();
        loop {
            let Some((terminal, lease)) = self
                .take_runtime_owned_module_failure_terminal_for_selected_action(action_owner_ids)
            else {
                break;
            };
            let crate::dynamic_script_owner::DynamicScriptFailureTerminal {
                id,
                script,
                message,
                kind,
                module_failure_policy,
                source_network_result: _,
                error_constructor,
            } = terminal;
            if let Some(lease) = lease {
                let lease_owner = lease.owner();
                let terminal = self.apply_runtime_script_failure_terminal_body(
                    &script,
                    &message,
                    module_failure_policy,
                    error_constructor,
                    lease,
                );
                if terminal.activity
                    == crate::script_vm::ScriptTerminalBodyActivity::EventDispatchAttempted
                {
                    settlement.activity =
                        crate::script_vm::ScriptTerminalBodyActivity::EventDispatchAttempted;
                }
                if matches!(
                    terminal.load_gate_release,
                    MainDocumentScriptLoadDelayRelease::BecameUnblocked
                ) {
                    if let Some(previous_owner) = settlement.lifecycle_unblocked_owner {
                        assert_eq!(
                            previous_owner, lease_owner,
                            "one selected module graph terminal cannot unblock two main Documents"
                        );
                    }
                    settlement.lifecycle_unblocked_owner = Some(lease_owner);
                }
            }
            debug!(
                dynamic_script_owner_id = ?id,
                url = %script.url,
                ?kind,
                "settled runtime module failure body inside its selected graph-terminal action"
            );
        }
        settlement
    }

    pub(crate) fn finish_runtime_owned_script_failure_with_kind(
        &mut self,
        dynamic_script_owner_id: Option<DynamicScriptOwnerId>,
        script: &PreparedScript,
        message: &str,
        kind: crate::dynamic_script_owner::DynamicScriptFailureKind,
        module_failure_policy: Option<crate::host::ModuleFailurePolicy>,
        error_constructor: Option<crate::types::ScriptErrorConstructorKind>,
    ) {
        if let Some(id) = dynamic_script_owner_id {
            if kind == crate::dynamic_script_owner::DynamicScriptFailureKind::Immediate {
                let lease = self
                    .document_runtime
                    .runtime_script_work_mut()
                    .dynamic_scripts
                    .finish_script_terminal(id);
                if let Some(lease) =
                    self.exact_runtime_script_terminal_lease_or_warn(id, script, lease)
                {
                    self.apply_runtime_script_failure_terminal(
                        script,
                        message,
                        module_failure_policy,
                        error_constructor,
                        lease,
                    );
                }
                return;
            }
            self.document_runtime
                .runtime_script_work_mut()
                .dynamic_scripts
                .note_script_failed_with_kind_and_error_constructor(
                    id,
                    script,
                    message.to_owned(),
                    kind,
                    module_failure_policy,
                    error_constructor,
                );
            debug!(
                dynamic_script_owner_id = ?id,
                url = %script.url,
                ?kind,
                "queued runtime-owned script failure follow-up"
            );
            self.enqueue_immediate_runtime_script_work_if_needed();
            return;
        }
        if kind == crate::dynamic_script_owner::DynamicScriptFailureKind::Immediate {
            let dispatch_current_script_error_immediately = script.source_kind
                == ScriptSourceKind::External
                && self.prepared_script_uses_runtime_owned_page_task_execution(script);
            if dispatch_current_script_error_immediately {
                self.dispatch_unclaimed_runtime_script_failure_and_finish_terminal_best_effort(
                    script,
                    message,
                    module_failure_policy,
                    error_constructor,
                );
                self.enqueue_immediate_runtime_script_work_if_needed();
                return;
            }
            match self.enqueue_script_failure_lifecycle_work_for_prepared_script(
                script,
                message,
                module_failure_policy,
                error_constructor,
            ) {
                Ok(FollowupPageTaskDisposition::Skipped) => {}
                Ok(
                    disposition @ (FollowupPageTaskDisposition::Deferred
                    | FollowupPageTaskDisposition::Enqueued),
                ) => {
                    if self.pause_runtime_script_work_at_followup_task_boundary(disposition) {
                        return;
                    }
                }
                Err(error) => {
                    self.record_runtime_warning(format_args!(
                        "script failure page-task enqueue failed for `{}`: {error}",
                        script.url
                    ));
                }
            }
            self.enqueue_immediate_runtime_script_work_if_needed();
            return;
        }

        panic!("runtime-owned deferrable script failure should carry dynamic owner id");
    }

    pub(super) fn publish_post_parse_action_followups(
        &mut self,
        _page_task_queue: &mut PageTaskQueue,
    ) {
        self.publish_post_parse_action_runtime_followups();
    }

    pub(super) fn finish_executed_post_parse_action(
        &mut self,
        page_task_queue: &mut PageTaskQueue,
        requires_runtime_followup_publication: bool,
    ) {
        if !requires_runtime_followup_publication {
            return;
        }
        self.publish_post_parse_action_followups(page_task_queue);
    }

    fn poll_post_parse_runtime_driver_step(
        &mut self,
        page_task_queue: &mut PageTaskQueue,
    ) -> PostParseRuntimeDriverStep {
        if self.has_post_domcontentloaded_runtime_backlog(page_task_queue) {
            PostParseRuntimeDriverStep::PendingBacklog
        } else {
            PostParseRuntimeDriverStep::Idle
        }
    }

    pub(super) fn poll_next_post_parse_driver_step(
        &mut self,
        page_task_queue: &mut PageTaskQueue,
    ) -> PostParseDriverStep {
        let has_post_domcontentloaded_load_delaying_runtime_work =
            self.has_post_domcontentloaded_load_delaying_runtime_work();
        self.prime_document_lifecycle_processing_and_record_stylesheet_network_results();
        let owner_step = self
            .document_runtime
            .poll_next_post_parse_owner_driver_step(
                page_task_queue,
                has_post_domcontentloaded_load_delaying_runtime_work,
            );
        self.record_ready_stylesheet_network_results();
        if let PostParseOwnerDriverStep::Ready(action) = owner_step {
            return self.select_ready_owner_post_parse_driver_step(*action, page_task_queue);
        }
        let runtime_step = self.poll_post_parse_runtime_driver_step(page_task_queue);
        select_post_parse_driver_step(owner_step, runtime_step)
    }

    fn select_ready_owner_post_parse_driver_step(
        &mut self,
        action: DocumentProcessingAction,
        page_task_queue: &mut PageTaskQueue,
    ) -> PostParseDriverStep {
        let PostParseProcessingAction {
            work,
            reached_boundary,
            invalidation_policy,
        } = PostParseProcessingAction::from_document_processing_action(action);
        if !(work.is_domcontentloaded_task() || work.is_window_load_task()) {
            return PostParseDriverStep::Ready(Box::new(ReadyPostParseAction::Processing(
                Box::new(PostParseProcessingAction {
                    work,
                    reached_boundary,
                    invalidation_policy,
                }),
            )));
        }
        if work.is_domcontentloaded_task()
            && let Some(owner) = work.main_document_lifecycle_owner()
            && self
                ._context_host
                .borrow()
                .current_main_document_domcontentloaded_transition_is_ready(owner)
                == Some(false)
        {
            tracing::debug!(
                ?owner,
                "main DOMContentLoaded delivery remains blocked by document lifecycle"
            );
            page_task_queue.enqueue_front_post_parse_work_preserving_order(vec![work]);
            return self.select_runtime_post_parse_step_while_owner_is_blocked(page_task_queue);
        }
        let is_window_load_task = work.is_window_load_task();
        if is_window_load_task
            && let Some(owner) = work.main_document_lifecycle_owner()
            && self
                ._context_host
                .borrow()
                .current_main_document_complete_transition_is_ready(owner)
                == Some(false)
        {
            tracing::debug!(
                ?owner,
                "main window-load delivery remains blocked by document lifecycle"
            );
            page_task_queue.enqueue_front_post_parse_work_preserving_order(vec![work]);
            return self.select_runtime_post_parse_step_while_owner_is_blocked(page_task_queue);
        }
        let runtime_step = if is_window_load_task {
            self.poll_post_parse_load_delaying_runtime_driver_step(page_task_queue)
        } else {
            self.poll_post_parse_runtime_driver_step(page_task_queue)
        };
        match runtime_step {
            PostParseRuntimeDriverStep::PendingBacklog if is_window_load_task => {
                page_task_queue.enqueue_front_post_parse_work_preserving_order(vec![work]);
                PostParseDriverStep::AwaitProgress
            }
            PostParseRuntimeDriverStep::PendingBacklog | PostParseRuntimeDriverStep::Idle => {
                PostParseDriverStep::Ready(Box::new(ReadyPostParseAction::Processing(Box::new(
                    PostParseProcessingAction {
                        work,
                        reached_boundary,
                        invalidation_policy,
                    },
                ))))
            }
        }
    }

    fn select_runtime_post_parse_step_while_owner_is_blocked(
        &mut self,
        page_task_queue: &mut PageTaskQueue,
    ) -> PostParseDriverStep {
        match self.poll_post_parse_runtime_driver_step(page_task_queue) {
            PostParseRuntimeDriverStep::PendingBacklog | PostParseRuntimeDriverStep::Idle => {
                PostParseDriverStep::AwaitProgress
            }
        }
    }

    fn poll_post_parse_load_delaying_runtime_driver_step(
        &mut self,
        page_task_queue: &mut PageTaskQueue,
    ) -> PostParseRuntimeDriverStep {
        let _ = page_task_queue;
        if self.has_post_domcontentloaded_load_delaying_runtime_work() {
            PostParseRuntimeDriverStep::PendingBacklog
        } else {
            PostParseRuntimeDriverStep::Idle
        }
    }

    pub(super) async fn next_post_parse_processing_step(
        &mut self,
        loader: &ResourceRequestClient,
        page_task_queue: &mut PageTaskQueue,
        report: &mut ScriptExecutionReport,
    ) -> std::result::Result<PostParseProcessingStep, String> {
        loop {
            if self
                .restart_post_parse_lifecycle_round_if_invalidated(page_task_queue, report)
                .await
            {
                continue;
            }
            if !self.document_runtime.dom_content_loaded_dispatched()
                && !self
                    .document_runtime
                    .has_parser_owned_pre_domcontentloaded_page_tasks()
                && self.runtime_script_work_has_pre_domcontentloaded_dynamic_candidate()
            {
                self.resume_runtime_script_work_after_deferred_page_tasks();
                match self
                    .flush_pending_runtime_script_work_until_generation_stable(
                        loader, true, false, None,
                    )
                    .await?
                {
                    RuntimePendingWorkFlushOutcome::Complete => continue,
                    RuntimePendingWorkFlushOutcome::WaitingForSource => {
                        return Ok(PostParseProcessingStep::AwaitProgress);
                    }
                }
            }
            match self.poll_next_post_parse_driver_step(page_task_queue) {
                PostParseDriverStep::Ready(action) => match *action {
                    ReadyPostParseAction::Processing(action) => {
                        return Ok(PostParseProcessingStep::Action(action));
                    }
                },
                PostParseDriverStep::NeedsContinuation => {
                    return Ok(PostParseProcessingStep::NeedsContinuation);
                }
                PostParseDriverStep::AwaitProgress => {
                    return Ok(PostParseProcessingStep::AwaitProgress);
                }
                PostParseDriverStep::Idle => {
                    return Ok(PostParseProcessingStep::Idle);
                }
            }
        }
    }

    pub(super) async fn next_post_parse_lifecycle_advance_from_driver(
        &mut self,
        loader: &ResourceRequestClient,
        page_task_queue: &mut PageTaskQueue,
        report: &mut ScriptExecutionReport,
        driver: PostParseLifecycleDriver,
    ) -> std::result::Result<PostParseLifecycleAdvance, String> {
        loop {
            match self
                .next_post_parse_processing_step(loader, page_task_queue, report)
                .await?
            {
                PostParseProcessingStep::Action(action) => {
                    let execution = driver.task_execution_for_action(*action);
                    if self
                        .prepare_post_parse_task_execution(page_task_queue, report, execution.token)
                        .await
                    {
                        continue;
                    }
                    return Ok(PostParseLifecycleAdvance::PageOwnedTask(Box::new(
                        execution.into_page_owned_task(),
                    )));
                }
                PostParseProcessingStep::NeedsContinuation => {
                    return Ok(PostParseLifecycleAdvance::NeedsContinuation);
                }
                PostParseProcessingStep::AwaitProgress => {
                    return Ok(PostParseLifecycleAdvance::AwaitProgress);
                }
                PostParseProcessingStep::Idle => {
                    return Ok(PostParseLifecycleAdvance::Complete(
                        driver.idle_completion_action(),
                    ));
                }
            }
        }
    }

    pub(crate) async fn advance_post_parse_lifecycle(
        &mut self,
        loader: &ResourceRequestClient,
        page_task_queue: &mut PageTaskQueue,
        report: &mut ScriptExecutionReport,
        driver: PostParseLifecycleDriver,
        completed_task: Option<PostParsePageOwnedTask>,
    ) -> std::result::Result<PostParseLifecycleAdvance, String> {
        if let Some(advance) = self
            .finish_completed_post_parse_page_owned_task_or_continue(
                page_task_queue,
                report,
                completed_task,
            )
            .await?
        {
            return Ok(advance);
        }
        self.next_post_parse_lifecycle_advance_from_driver(loader, page_task_queue, report, driver)
            .await
    }

    pub(super) fn has_post_domcontentloaded_runtime_backlog(
        &mut self,
        page_task_queue: &mut PageTaskQueue,
    ) -> bool {
        let _ = page_task_queue;
        self.has_pending_native_module_job()
            || self.has_pending_stable_runtime_script_continuation()
            || (self.document_runtime.dom_content_loaded_dispatched()
                && self.has_pending_runtime_script_work())
    }

    fn has_post_domcontentloaded_load_delaying_runtime_work(&mut self) -> bool {
        if !self.document_runtime.dom_content_loaded_dispatched() {
            return false;
        }
        self.refresh_script_vm_local_document_state();
        self.current_main_document_task_owner()
            .and_then(|owner| self.current_main_document_has_async_script_load_delay(owner))
            .unwrap_or(false)
    }

    pub(super) fn has_pending_runtime_script_work(&mut self) -> bool {
        self.document_runtime
            .runtime_script_work_mut()
            .has_pending_work()
    }

    pub(crate) fn current_main_document_has_async_script_load_delay(
        &self,
        owner: crate::frame_owner_model::FrameDocumentTaskOwner,
    ) -> Option<bool> {
        self._context_host
            .borrow()
            .current_main_document_has_async_script_load_delay(owner)
    }

    fn has_pending_stable_runtime_script_continuation(&mut self) -> bool {
        self.document_runtime
            .runtime_script_work_mut()
            .is_paused_for_deferred_page_tasks()
            && self.document_runtime.runtime_script_work_mut().pause_kind()
                == Some(RuntimeScriptWorkPauseKind::StablePageTurnContinuation)
            && !self.document_runtime.runtime_script_work_mut().is_idle()
    }

    pub(crate) fn has_runnable_runtime_script_work_now(&mut self) -> bool {
        self.refresh_script_vm_local_document_state();
        let has_stable_continuation = self.has_pending_stable_runtime_script_continuation();
        if self.document_runtime.dom_content_loaded_dispatched() {
            return (has_stable_continuation || self.has_pending_runtime_script_work())
                && self.runtime_script_work_has_immediately_runnable_work();
        }
        has_stable_continuation
            && self
                .runtime_script_work_has_immediately_runnable_dynamic_work_before_domcontentloaded()
    }

    fn prepared_dynamic_script_targets_pre_domcontentloaded_lane(
        &self,
        script: &PreparedScript,
    ) -> bool {
        let Some(handle) = script.host_script_handle.as_deref() else {
            return false;
        };
        self.document_runtime.script_handle_source(handle) == ScriptHandleSource::DocumentWriteOwned
            && self.document_runtime.script_handle_followup_lane(handle)
                == Some(crate::document_runtime::DeferredPageTaskLane::PreDomContentLoaded)
    }

    fn runtime_script_work_has_pre_domcontentloaded_dynamic_candidate(&mut self) -> bool {
        self.document_runtime.accept_ready_runtime_script_events();
        self.document_runtime
            .runtime_script_work_mut()
            .dynamic_scripts
            .has_script_matching(|script| {
                self.prepared_dynamic_script_targets_pre_domcontentloaded_lane(script)
            })
    }

    fn runtime_script_work_has_immediately_runnable_dynamic_work_before_domcontentloaded(
        &mut self,
    ) -> bool {
        self.document_runtime.accept_ready_runtime_script_events();
        self.document_runtime
            .runtime_script_work_mut()
            .dynamic_scripts
            .has_immediately_runnable_work_matching(|script| {
                dynamic_script_execute_is_runnable_before_dom_content_loaded(
                    &self.document_runtime,
                    script,
                )
            })
    }

    fn prepare_runtime_script_work_signal(&mut self) -> Option<ImmediateRuntimeScriptWorkSignal> {
        (self.has_pending_runtime_script_work()
            && self.arm_runtime_script_work_continuation_if_needed()
            && self.runtime_script_work_should_signal_immediate_progress())
        .then_some(ImmediateRuntimeScriptWorkSignal::StablePageTurnContinuation)
    }

    pub(crate) fn enqueue_immediate_runtime_script_work_if_needed(&mut self) {
        self.enqueue_runtime_script_signal_if_needed();
    }

    pub(super) fn enqueue_runtime_script_signal_if_needed(&mut self) {
        if let Some(signal) = self.prepare_runtime_script_work_signal() {
            self.enqueue_immediate_runtime_script_work_signal(signal);
        }
    }

    pub(super) fn arm_runtime_script_work_continuation_if_needed(&mut self) -> bool {
        if self.document_runtime.runtime_script_work_mut().is_idle() {
            return false;
        }
        self.pause_runtime_script_work_for_stable_continuation();
        true
    }

    #[cfg(test)]
    pub(super) fn test_pending_runtime_source_load_script(&self) -> PreparedScript {
        PreparedScript {
            position: 0,
            node_id: NodeId::new(1),
            kind: ScriptKind::Classic,
            mode: ScriptMode::Async,
            source_kind: ScriptSourceKind::External,
            fetch_metadata: crate::planning::ScriptFetchMetadata::default(),
            source: moli_parser::ScriptSource::External,
            url: Url::parse("https://example.com/pending-runtime.js").unwrap(),
            base_url: Url::parse("https://example.com/pending-runtime.js").unwrap(),
            initiator_url: Url::parse("https://example.com/").unwrap(),
            host_script_handle: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn enqueue_test_pending_runtime_source_load(&mut self) {
        let script = self.test_pending_runtime_source_load_script();
        self.document_runtime
            .runtime_script_work_mut()
            .dynamic_scripts
            .enqueue_loading_script_for_test(script);
    }

    /// Install a ready runtime-script producer without publishing its Page
    /// continuation.
    ///
    /// The fixture carries the same exact Document load-delay lease and
    /// runtime-owned script identity as production. A later selected callback
    /// completion must therefore publish the real
    /// `RuntimeScriptContinuation`; the fixture never executes the script
    /// through a test-only path.
    #[cfg(test)]
    pub(crate) fn enqueue_test_ready_runtime_script_followup(&mut self) {
        let owner = self
            .current_main_document_task_owner()
            .expect("ready runtime-script fixture requires a current main Document");
        let load_delay_binding = self
            .accept_main_document_script_load_delay_binding(
                owner,
                MainDocumentScriptLoadDelayKind::Classic,
            )
            .expect("ready runtime-script fixture requires exact lifecycle ownership");
        let node_id = self
            .document_runtime
            .dom_host_mut()
            .create_element("script");
        let host_script_handle = format!("test-runtime-followup-{}", node_id.index());
        self.document_runtime
            .bind_runtime_owned_script_handle_for_node(node_id, &host_script_handle);
        let script = PreparedScript {
            position: node_id.index(),
            node_id,
            kind: ScriptKind::Classic,
            mode: ScriptMode::Async,
            source_kind: ScriptSourceKind::Inline,
            fetch_metadata: crate::planning::ScriptFetchMetadata::default(),
            source: moli_parser::ScriptSource::Inline(String::new()),
            url: Url::parse("data:text/javascript,").expect("test runtime-script URL"),
            base_url: self.document_runtime.document_url().clone(),
            initiator_url: self.document_runtime.document_url().clone(),
            host_script_handle: Some(host_script_handle),
        };
        self.document_runtime
            .runtime_script_work_mut()
            .dynamic_scripts
            .enqueue_ready_script_with_load_delay_for_test(script, load_delay_binding);
        // Match the production handoff: the concrete producer is ready, but
        // it remains parked behind a stable continuation until the enclosing
        // selected callback completes. Without this pause, the callback's
        // legacy host-turn flush could execute the script directly and the
        // fixture would not prove typed continuation publication.
        self.handle_runtime_script_work_at_explicit_boundary(false);
    }

    fn pause_runtime_script_work_for_stable_continuation(&mut self) {
        self.document_runtime
            .runtime_script_work_mut()
            .pause_for_deferred_page_tasks(RuntimeScriptWorkPauseKind::StablePageTurnContinuation);
        let Some(document_owner) = self.current_main_document_task_owner() else {
            self.record_runtime_warning(format_args!(
                "cannot arm runtime script continuation without a current main Document owner"
            ));
            return;
        };
        let continuation = self
            .page_runtime_wake_tx
            .bind_main_document_runtime_continuation(document_owner);
        self.document_runtime
            .runtime_script_work_mut()
            .dynamic_scripts
            .enable_continuation_enqueue(continuation);
    }

    pub(super) fn runtime_script_work_should_signal_immediate_progress(&mut self) -> bool {
        self.document_runtime.accept_ready_runtime_script_events();
        if self.document_runtime.runtime_script_work_mut().is_idle() {
            return false;
        }
        if self.document_runtime.dom_content_loaded_dispatched() {
            return self.runtime_script_work_has_immediately_runnable_work();
        }
        self.runtime_script_work_has_immediately_runnable_dynamic_work_before_domcontentloaded()
    }

    pub(crate) fn dispatch_connected_style_load(
        &mut self,
        ready: crate::document_runtime::ReadyConnectedStyleLoad,
    ) -> bool {
        let parser_blocking_link_event = self
            .document_runtime
            .ready_connected_style_load_is_parser_blocking_link_event(&ready);
        let context_ptr: *const v8::Global<v8::Context> = &self.page_default_context;
        let context_host = self._context_host.clone();
        let document_runtime = &mut self.document_runtime;
        let dispatched = self
            .renderer_document_isolate
            .with_renderer_document_isolate_mut(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                let scope = &mut v8::ContextScope::new(scope, context);
                let host_ptr: *mut JsContextHost = (*context_host).as_ptr();
                if ready.load_event_binding().is_some_and(|binding| {
                    !unsafe { &*host_ptr }.main_style_load_event_is_current(binding)
                }) {
                    return false;
                }
                document_runtime.dispatch_pending_style_load(scope, host_ptr, ready)
            });
        if parser_blocking_link_event {
            self.document_runtime
                .release_main_parser_after_parser_blocking_link_event_if_ready();
        }
        dispatched
    }

    pub(crate) fn apply_pending_stylesheet_source_css_projections(&mut self) {
        let context_ptr: *const v8::Global<v8::Context> = &self.page_default_context;
        let context_host = self._context_host.clone();
        let document_runtime = &mut self.document_runtime;
        self.renderer_document_isolate
            .with_renderer_document_isolate_mut(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                let scope = &mut v8::ContextScope::new(scope, context);
                let host_ptr: *mut JsContextHost = (*context_host).as_ptr();
                document_runtime.apply_pending_stylesheet_source_css_projections(scope, host_ptr);
            });
    }

    pub(crate) fn dispatch_preload_like_link_error_event(&mut self, handle: DomHandle) -> bool {
        let context_ptr: *const v8::Global<v8::Context> = &self.page_default_context;
        let context_host = self._context_host.clone();
        let document_runtime = &mut self.document_runtime;
        self.renderer_document_isolate
            .with_renderer_document_isolate_mut(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                let scope = &mut v8::ContextScope::new(scope, context);
                let host_ptr: *mut JsContextHost = (*context_host).as_ptr();
                document_runtime.dispatch_preload_like_link_error_event(scope, host_ptr, handle);
            });
        true
    }

    pub(crate) fn bind_prepared_script_handle_if_needed(
        &mut self,
        script: &mut PreparedScript,
        source: ScriptHandleSource,
    ) {
        let allow_missing_handle = script.kind == ScriptKind::Classic
            && script.source_kind == ScriptSourceKind::Inline
            && script.mode == ScriptMode::Normal;
        if script.host_script_handle.is_none() && !allow_missing_handle {
            let handle = match source {
                ScriptHandleSource::ParserOwned => self
                    .document_runtime
                    .bind_parser_owned_script_handle_for_node(script.node_id),
                ScriptHandleSource::DocumentWriteOwned => self
                    .document_runtime
                    .bind_document_write_owned_script_handle_for_node(script.node_id),
                ScriptHandleSource::Unknown | ScriptHandleSource::RuntimeOwned => {
                    panic!("unsupported prepared script binding source: {source:?}")
                }
            };
            script.host_script_handle = Some(handle);
        }
        if matches!(source, ScriptHandleSource::DocumentWriteOwned)
            && !matches!(script.mode, ScriptMode::Normal)
        {
            let _ = self
                .document_runtime
                .mark_script_already_started_by_node_id(script.node_id);
        }
        if let Some(handle) = script.host_script_handle.as_deref() {
            self.document_runtime.set_script_handle_followup_lane(
                handle,
                crate::host::HostScriptScheduler::followup_lane_for_script(source, script.mode),
            );
            if matches!(source, ScriptHandleSource::DocumentWriteOwned)
                && (script.kind == ScriptKind::Module
                    || matches!(
                        script.mode,
                        ScriptMode::Async
                            | ScriptMode::InOrder
                            | ScriptMode::ImportMapInOrder
                            | ScriptMode::ModuleInOrder
                    ))
            {
                self.document_runtime
                    .set_script_handle_waits_until_dom_content_loaded(handle);
            }
        }
    }

    pub(crate) fn prime_document_lifecycle_processing_and_record_stylesheet_network_results(
        &mut self,
    ) {
        let prime_result = self
            .document_runtime
            .prime_document_lifecycle_processing_for_owner(self._context_host.as_ref().as_ptr());
        self.schedule_connected_modulepreloads_from_prime_result(prime_result);
        self.record_ready_stylesheet_network_results();
        self.reconcile_document_web_fonts_for_layout();
    }

    pub(crate) fn schedule_connected_modulepreloads_from_prime_result(
        &mut self,
        prime_result: crate::document_runtime::ConnectedStyleLoadPrimeResult,
    ) {
        let (modulepreload_starts, runtime_warnings) = prime_result.into_parts();
        for warning in runtime_warnings {
            self.record_runtime_warning(format_args!("{warning}"));
        }
        for start in modulepreload_starts {
            let (request, link_client) = start.into_parts();
            if let Err(error) =
                self.register_native_modulepreload_link_for_owner(request, link_client)
            {
                tracing::debug!(
                    "connected JS modulepreload failed before fetch scheduling: {}",
                    error
                );
            }
        }
    }

    pub(in crate::script_vm) fn queue_linked_stylesheet_import_csp_violations(
        &mut self,
        import_urls: impl IntoIterator<Item = url::Url>,
    ) {
        let violations = import_urls
            .into_iter()
            .flat_map(|url| {
                let (report_only, enforced) = self
                    .document_runtime
                    .style_element_request_csp_check(
                        &url,
                        crate::content_security_policy::ContentSecurityPolicyStyleElementRequest {
                            nonce: None,
                        },
                    )
                    .into_violations();
                [report_only, enforced].into_iter().flatten()
            })
            .collect::<Vec<_>>();
        if violations.is_empty() {
            return;
        }

        let context_ptr: *const v8::Global<v8::Context> = &self.page_default_context;
        let context_host = self._context_host.clone();
        let document_runtime = &mut self.document_runtime;
        self.renderer_document_isolate
            .with_renderer_document_isolate_mut(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                let scope = &mut v8::ContextScope::new(scope, context);
                let host_ptr: *mut JsContextHost = (*context_host).as_ptr();
                for violation in violations {
                    document_runtime.queue_content_security_policy_violation_event_best_effort(
                        scope, host_ptr, &violation,
                    );
                }
            });
    }

    pub(crate) fn record_ready_stylesheet_network_results(&mut self) {
        let results = self
            .document_runtime
            .take_ready_stylesheet_network_results();
        let client_terminals = self
            .document_runtime
            .take_ready_stylesheet_link_client_terminals();
        let blocking_import_graphs = self
            .document_runtime
            .take_ready_blocking_style_import_graphs();
        if results.is_empty() && client_terminals.is_empty() && blocking_import_graphs.is_empty() {
            return;
        }
        let optional_resource_fetch_mask = self
            .document_runtime
            .current_document_resource_loader()
            .map_or(
                crate::protocol_types::OptionalResourceFetchMask::NONE,
                |loader| loader.request_client().optional_resource_fetch_mask(),
            );
        let mut performance_entries = Vec::new();
        let mut css_subresources = Vec::new();
        let mut linked_stylesheet_import_urls = Vec::new();
        let mut linked_stylesheet_import_loads = Vec::new();
        let mut import_graph_results: Vec<(
            crate::document_runtime::ConnectedStyleImportRoot,
            Vec<crate::live_stylesheet::LiveStylesheetImportResponse>,
        )> = Vec::new();
        let mut host = self._context_host.borrow_mut();
        for network_result in results {
            let crate::document_runtime::ConnectedLoadNetworkResult {
                stylesheet_fetch,
                blocking_operation: _,
                source_operation: _,
                import_roots,
                document_url,
                request_url,
                source_owners,
                resource_type,
                start_unix_millis,
                origin_clean,
                result,
            } = network_result;
            let is_import_graph_result = !import_roots.is_empty();
            let import_roots = import_roots
                .into_iter()
                .filter(|root| {
                    host.live_stylesheet(root.stylesheet_id)
                        .is_some_and(|stylesheet| root.matches_stylesheet(&stylesheet))
                })
                .collect::<Vec<_>>();
            let has_stylesheet_install_authority = if is_import_graph_result {
                !import_roots.is_empty()
            } else {
                !source_owners.is_empty()
            };
            performance_entries.push(
                crate::context_bootstrap::ResourcePerformanceEntry::from_network_result(
                    request_url.as_str(),
                    preload_like_resource_initiator_type(resource_type),
                    start_unix_millis,
                    &result,
                ),
            );
            if resource_type == SubresourceResourceType::Stylesheet && stylesheet_fetch.is_none() {
                let validated_response = result
                    .as_ref()
                    .ok()
                    .filter(|response| (200..=299).contains(&response.status))
                    .and_then(|response| {
                        crate::stylesheet_blocking::validate_stylesheet_response(
                            &request_url,
                            response.clone(),
                        )
                        .ok()
                    });
                let (stylesheet_text, stylesheet_base_url) = validated_response
                    .as_ref()
                    .map(|response| (response.body_text().to_owned(), response.final_url.clone()))
                    .unwrap_or_else(|| {
                        let final_url = result
                            .as_ref()
                            .ok()
                            .map(|response| response.final_url.clone())
                            .unwrap_or_else(|| request_url.clone());
                        (String::new(), final_url)
                    });
                if is_import_graph_result {
                    let response = crate::live_stylesheet::LiveStylesheetImportResponse {
                        request_url: request_url.clone(),
                        response_url: stylesheet_base_url.clone(),
                        css_text: stylesheet_text.clone(),
                        successful: validated_response.is_some(),
                        origin_clean,
                    };
                    for root in import_roots {
                        if let Some((_, responses)) =
                            import_graph_results.iter_mut().find(|(candidate, _)| {
                                candidate.owner == root.owner
                                    && candidate.stylesheet_id == root.stylesheet_id
                                    && candidate.contents_revision == root.contents_revision
                                    && candidate.import_generation == root.import_generation
                            })
                        {
                            responses.push(response.clone());
                        } else {
                            import_graph_results.push((root, vec![response.clone()]));
                        }
                    }
                }
                if !is_import_graph_result {
                    for owner in source_owners.iter().copied() {
                        let Some(prepared) = host.prepare_linked_stylesheet_resource(
                            owner,
                            &stylesheet_text,
                            stylesheet_base_url.clone(),
                            request_url.clone(),
                            origin_clean,
                        ) else {
                            continue;
                        };
                        host.install_linked_stylesheet(
                            crate::document_runtime::InstallLinkedStylesheet::from_prepared(
                                owner,
                                request_url.clone(),
                                prepared,
                            ),
                        );
                    }
                }
                // A stale completion remains observable as Network and
                // Performance data, but cannot derive new requests in the
                // current document after its exact owner/root authority was
                // revoked.
                if has_stylesheet_install_authority && validated_response.is_some() {
                    for resource in crate::css_resource_urls::stylesheet_load_blocking_resources(
                        &stylesheet_text,
                        &stylesheet_base_url,
                        optional_resource_fetch_mask,
                    ) {
                        let Some(binding) =
                            host.accept_current_main_stylesheet_subresource_load_delay()
                        else {
                            tracing::debug!(
                                url = %resource.request_url(),
                                kind = ?resource.kind(),
                                "skipping stylesheet subresource for stale main document owner"
                            );
                            continue;
                        };
                        css_subresources.push((binding, resource));
                    }
                }
            }
            host.record_get_subresource_network_result_with_initiator(
                None,
                document_url,
                request_url,
                resource_type,
                SubresourceRequestInitiatorType::Parser,
                &result,
            );
        }
        for (root, responses) in import_graph_results {
            if host
                .install_live_stylesheet_import_graph(root.clone(), &responses)
                .is_some()
            {
                let _ =
                    host.refresh_live_stylesheet_after_import_graph(root.owner, root.stylesheet_id);
            }
        }
        for blocking_import_graph in blocking_import_graphs {
            let (operation, roots, graph, mut successful) = blocking_import_graph.into_parts();
            let expected_root_count = roots.len();
            let roots = roots
                .into_iter()
                .filter(|root| {
                    host.live_stylesheet(root.stylesheet_id)
                        .is_some_and(|stylesheet| root.matches_stylesheet(&stylesheet))
                })
                .collect::<Vec<_>>();
            successful &= !roots.is_empty() && roots.len() == expected_root_count;
            let responses = graph
                .network_results()
                .iter()
                .map(|result| {
                    let terminal = result.terminal();
                    let physical_response = terminal.physical().as_result().ok();
                    let ready_response = terminal.ready_response();
                    crate::live_stylesheet::LiveStylesheetImportResponse {
                        request_url: result.request_url().clone(),
                        response_url: physical_response
                            .as_ref()
                            .map(|response| response.final_url.clone())
                            .unwrap_or_else(|| result.request_url().clone()),
                        css_text: ready_response
                            .map(|response| response.body_text().to_owned())
                            .unwrap_or_default(),
                        successful: ready_response.is_some(),
                        origin_clean: terminal.origin_clean().unwrap_or(false),
                    }
                })
                .collect::<Vec<_>>();
            if !roots.is_empty() {
                for response in &responses {
                    if !response.successful {
                        continue;
                    }
                    for resource in crate::css_resource_urls::stylesheet_load_blocking_resources(
                        &response.css_text,
                        &response.response_url,
                        optional_resource_fetch_mask,
                    ) {
                        let Some(binding) =
                            host.accept_current_main_stylesheet_subresource_load_delay()
                        else {
                            continue;
                        };
                        css_subresources.push((binding, resource));
                    }
                }
            }
            for root in roots {
                match host.install_live_stylesheet_import_graph(root.clone(), &responses) {
                    Some(graph_successful) => {
                        successful &= graph_successful;
                        let _ = host.refresh_live_stylesheet_after_import_graph(
                            root.owner,
                            root.stylesheet_id,
                        );
                    }
                    None => successful = false,
                }
            }
            self.document_runtime
                .complete_ready_blocking_style_import_graph(&operation, successful);
        }
        for client in client_terminals {
            let load = client.load();
            if !load.installs_stylesheet() {
                continue;
            }
            let terminal = client.terminal();
            // Blink creates the owner's CSSStyleSheet even when the resource
            // failed or was cancelled. An unusable terminal therefore installs
            // an empty source; its existing link task still reports the error.
            let ready_response = terminal.ready_response();
            let stylesheet_text = ready_response
                .map(|response| response.body_text().to_owned())
                .unwrap_or_default();
            let stylesheet_base_url = match terminal.physical() {
                crate::stylesheet_blocking::StylesheetPhysicalOutcome::Response(response) => {
                    response.final_url.clone()
                }
                crate::stylesheet_blocking::StylesheetPhysicalOutcome::NetworkError(_) => {
                    load.request_url().clone()
                }
            };
            let request_url = load.request_url().clone();
            let prepared = host.prepare_linked_stylesheet_resource(
                load.owner(),
                &stylesheet_text,
                stylesheet_base_url.clone(),
                request_url.clone(),
                terminal.origin_clean().unwrap_or(false),
            );
            if let Some(prepared) = prepared.as_ref() {
                host.install_linked_stylesheet(
                    crate::document_runtime::InstallLinkedStylesheet::from_prepared(
                        load.owner(),
                        request_url.clone(),
                        prepared.clone(),
                    ),
                );
            }
            if !load.fetch().claim_dependent_resource_start() {
                continue;
            }
            let import_urls = ready_response
                .and(prepared.as_ref())
                .map(|prepared| prepared.import_urls().to_vec())
                .unwrap_or_default();
            linked_stylesheet_import_loads.push((Arc::clone(load), import_urls.clone()));
            if ready_response.is_none() {
                // Only a usable response may contribute CSS text, imports, or
                // other dependent resources. The empty owner sheet is CSSOM-only.
                continue;
            }
            if request_url.scheme() != "data" {
                linked_stylesheet_import_urls.extend(import_urls);
            }
            for resource in crate::css_resource_urls::stylesheet_load_blocking_resources(
                &stylesheet_text,
                &stylesheet_base_url,
                optional_resource_fetch_mask,
            ) {
                let Some(binding) = host.accept_current_main_stylesheet_subresource_load_delay()
                else {
                    tracing::debug!(
                        url = %resource.request_url(),
                        kind = ?resource.kind(),
                        "skipping stylesheet subresource for stale main document owner"
                    );
                    continue;
                };
                css_subresources.push((binding, resource));
            }
        }
        drop(host);
        let host_ptr: *mut JsContextHost = (*self._context_host).as_ptr();
        for (load, urls) in linked_stylesheet_import_loads {
            self.document_runtime
                .prime_network_stylesheet_import_loads(load, urls, host_ptr);
        }
        self.queue_linked_stylesheet_import_csp_violations(linked_stylesheet_import_urls);
        self.apply_pending_stylesheet_source_css_projections();
        self.start_stylesheet_subresource_fetches(css_subresources);
        self.record_resource_performance_entries(performance_entries);
    }

    fn record_resource_performance_entries(
        &mut self,
        entries: Vec<crate::context_bootstrap::ResourcePerformanceEntry>,
    ) {
        if entries.is_empty() {
            return;
        }
        if let Err(error) = self.with_default_context_scope(|scope, _host_ptr| {
            for entry in entries {
                crate::context_bootstrap::record_resource_performance_entry(scope, entry);
            }
            Ok(())
        }) {
            self.record_runtime_warning(format_args!(
                "failed to record resource performance entries: {error}"
            ));
        }
    }

    pub(super) fn start_stylesheet_subresource_fetches(
        &mut self,
        resources: Vec<(
            crate::frame_owner_model::StylesheetSubresourceLoadDelayBinding,
            crate::css_resource_urls::StylesheetLoadBlockingResource,
        )>,
    ) {
        if resources.is_empty() {
            return;
        }
        let started = Instant::now();
        let resource_count = resources.len();
        let context_host = self._context_host.clone();
        let retain_css_images = context_host.borrow().layout_policy().uses_real_layout();
        let mut admitted = Vec::with_capacity(resources.len());
        let mut duplicate_bindings = Vec::new();
        for (binding, resource) in resources {
            match resource.kind() {
                crate::css_resource_urls::StylesheetLoadBlockingResourceKind::Font
                    if binding.child_handle().is_none() =>
                {
                    match self
                        ._context_host
                        .borrow()
                        .admit_document_web_font(resource)
                    {
                        Some(resource) => admitted.push((binding, resource, None)),
                        None => duplicate_bindings.push(binding),
                    }
                }
                crate::css_resource_urls::StylesheetLoadBlockingResourceKind::Image
                    if retain_css_images =>
                {
                    let resolved_url = resource.request_url().as_str().to_owned();
                    match self
                        ._context_host
                        .borrow_mut()
                        .admit_stylesheet_css_image(binding, resolved_url)
                    {
                        crate::native_bridge::CssImageResourceAdmission::Fetch(identity) => {
                            admitted.push((binding, resource, Some(identity)));
                        }
                        crate::native_bridge::CssImageResourceAdmission::Reused => {
                            duplicate_bindings.push(binding);
                        }
                        crate::native_bridge::CssImageResourceAdmission::Untracked => {
                            admitted.push((binding, resource, None));
                        }
                    }
                }
                _ => admitted.push((binding, resource, None)),
            }
        }
        if !duplicate_bindings.is_empty() {
            let mut host = context_host.borrow_mut();
            for binding in duplicate_bindings {
                host.settle_stylesheet_subresource_load_delay(binding);
            }
        }
        let result = self.with_default_context_scope(move |scope, _host_ptr| {
            let mut host = context_host.borrow_mut();
            let mut local_web_fonts = Vec::new();
            for (binding, resource, css_image) in admitted {
                let request_url = resource.request_url().clone();
                let kind = resource.kind();
                let failed_css_image = css_image.clone();
                let failed_web_font = (binding.child_handle().is_none()
                    && kind == crate::css_resource_urls::StylesheetLoadBlockingResourceKind::Font)
                    .then(|| {
                        resource
                            .web_font()
                            .cloned()
                            .map(crate::css_resource_urls::CompletedStylesheetWebFont::failure)
                    })
                    .flatten();
                match crate::network_host::start_stylesheet_subresource_fetch(
                    scope, &mut host, binding, resource, css_image,
                ) {
                    Ok(crate::network_host::StylesheetSubresourceFetchStart::WebFontSettled(
                        web_font,
                    )) => local_web_fonts.push(web_font),
                    Ok(
                        crate::network_host::StylesheetSubresourceFetchStart::Pending
                        | crate::network_host::StylesheetSubresourceFetchStart::Settled,
                    ) => {}
                    Err(error) => {
                        if let Some(identity) = failed_css_image.as_ref() {
                            let _ = host.fail_stylesheet_css_image(identity);
                        }
                        let settlement = host.settle_stylesheet_subresource_load_delay(binding);
                        local_web_fonts.extend(failed_web_font);
                        tracing::warn!(
                            url = %request_url,
                            ?kind,
                            owner = ?binding.owner(),
                            settled = settlement.settled(),
                            %error,
                            "stylesheet subresource failed before network scheduling"
                        );
                    }
                }
            }
            Ok(local_web_fonts)
        });
        match result {
            Ok(web_fonts) => {
                for web_font in web_fonts {
                    self.complete_document_web_font(web_font);
                }
            }
            Err(error) => self.record_runtime_warning(format_args!(
                "failed to enter stylesheet subresource request scope: {error}"
            )),
        }
        debug!(
            resource_count,
            elapsed_ms = started.elapsed().as_millis(),
            "started owner-bound stylesheet subresource requests"
        );
    }

    pub(crate) fn record_script_subresource_network_result(
        &mut self,
        document_url: Url,
        request_url: Url,
        result: &std::result::Result<crate::protocol_types::NavigationResponse, String>,
    ) {
        self.record_script_subresource_network_result_with_initiator(
            document_url,
            request_url,
            SubresourceRequestInitiatorType::Parser,
            result,
        );
    }

    pub(crate) fn record_script_subresource_network_result_with_initiator(
        &mut self,
        document_url: Url,
        request_url: Url,
        request_initiator_type: SubresourceRequestInitiatorType,
        result: &std::result::Result<crate::protocol_types::NavigationResponse, String>,
    ) {
        let performance_entry =
            crate::context_bootstrap::ResourcePerformanceEntry::from_network_result(
                request_url.as_str(),
                "script",
                None,
                result,
            );
        self._context_host
            .borrow_mut()
            .record_get_subresource_network_result_with_initiator(
                None,
                document_url,
                request_url,
                SubresourceResourceType::Script,
                request_initiator_type,
                result,
            );
        self.record_resource_performance_entries(vec![performance_entry]);
    }

    pub(crate) fn record_historical_script_subresource_network_result(
        &mut self,
        document_url: Url,
        request_url: Url,
        result: &std::result::Result<crate::protocol_types::NavigationResponse, String>,
    ) {
        self._context_host
            .borrow_mut()
            .record_historical_get_subresource_network_result_with_initiator(
                None,
                document_url,
                request_url,
                SubresourceResourceType::Script,
                SubresourceRequestInitiatorType::Parser,
                result,
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stylesheet_image_resources_require_the_image_bit() {
        let stylesheet_base_url =
            Url::parse("https://example.test/assets/app.css").expect("stylesheet url");
        let stylesheet_text = "#hero { background-image: url(hero.png); }";

        assert!(
            crate::css_resource_urls::stylesheet_load_blocking_resources(
                stylesheet_text,
                &stylesheet_base_url,
                crate::protocol_types::OptionalResourceFetchMask::NONE,
            )
            .is_empty()
        );

        let resources = crate::css_resource_urls::stylesheet_load_blocking_resources(
            stylesheet_text,
            &stylesheet_base_url,
            crate::protocol_types::OptionalResourceFetchMask::IMAGE,
        );

        assert_eq!(resources.len(), 1);
        assert_eq!(
            resources[0].request_url().as_str(),
            "https://example.test/assets/hero.png"
        );
        assert_eq!(
            resources[0].kind(),
            crate::css_resource_urls::StylesheetLoadBlockingResourceKind::Image
        );
    }

    #[test]
    fn stylesheet_font_face_resources_require_the_font_bit() {
        let stylesheet_base_url =
            Url::parse("https://example.test/assets/app.css").expect("stylesheet url");
        let stylesheet_text = r#"
            @font-face {
                font-family: Demo;
                src: local("Demo"),
                     url(fonts/demo.woff2) format("woff2"),
                     url("/fonts/demo.woff") format("woff");
            }
            body { background-image: url(hero.png); }
        "#;

        let resources = crate::css_resource_urls::stylesheet_load_blocking_resources(
            stylesheet_text,
            &stylesheet_base_url,
            crate::protocol_types::OptionalResourceFetchMask::FONT,
        );

        assert_eq!(resources.len(), 1);
        assert_eq!(
            resources[0].request_url().as_str(),
            "https://example.test/assets/fonts/demo.woff2"
        );
        assert_eq!(
            resources[0].kind(),
            crate::css_resource_urls::StylesheetLoadBlockingResourceKind::Font
        );
    }
}
