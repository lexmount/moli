use super::{ChildBrowsingContextEntry, JsContextHost};
use crate::{
    detached_event_target::dispatch_detached_simple_event,
    document_runtime::DomHandle,
    document_script_scheduler::{
        DocumentScriptReadyActionDispatchRoute, FrameDocumentClassicReadyWork,
        FrameDocumentClassicScriptSchedulerWork, FrameDocumentClassicSourceFailureWork,
        FrameParserDeferredScriptKind, FrameParserDeferredScriptOrderEntry,
        ParserOrderedModuleTerminalState, ParserPendingScriptId, ParserPendingScriptKey,
    },
    frame_owner_model::{
        ChildDocumentAsyncClassicScriptLoadDelay, DocumentId, DocumentLoadDelayTokenId,
        FrameClassicDocumentScriptExecutionAction, FrameClassicDocumentScriptExecutionStart,
        FrameDocumentClassicDeferredCompletionApplication,
        FrameDocumentClassicParserResumeApplication,
        FrameDocumentClassicParserResumeCompletionAction,
        FrameDocumentClassicParserResumeSkipReason, FrameDocumentClassicPrepareApplication,
        FrameDocumentClassicPrepareDropReason, FrameDocumentClassicScriptCompletionAction,
        FrameDocumentClassicScriptCompletionTarget, FrameDocumentClassicScriptExecutionFinish,
        FrameDocumentClassicScriptScheduling, FrameDocumentClassicScriptSourceLoadClient,
        FrameDocumentClassicScriptSourceLoadTask, FrameDocumentClassicScriptTarget,
        FrameDocumentClassicSourceFailureReportApplication,
        FrameDocumentClassicSourceFailureReportSkipReason,
        FrameDocumentExternalClassicScriptExecution,
        FrameDocumentExternalClassicScriptExecutionAction, FrameDocumentScriptElementEvent,
        FrameDocumentTaskOwner, FrameDocumentUnboundScriptWork, FrameRealmId, FrameRequestId,
        FrameRequestKind, FrameScriptJob, LocalWindowId, PendingChildExternalClassicDocumentScript,
        frame_script_job_kind_from_parser_classic_ready_kind,
    },
    page_task_queue::RendererPageChildClassicScriptSourceLoadTarget,
    parser_script::action::{
        ParserPendingClassicScriptExecution, ParserPendingClassicScriptNotification,
    },
    planning::{PreparedScript, ScriptSource},
    types::{
        ChildClassicScriptLoadCompletion, ChildClassicScriptNetworkAttribution,
        SubresourceResourceType,
    },
};

mod document_state;
mod prepare;
mod scheduler_bridge;

pub(crate) use crate::frame_owner_model::FrameDocumentOwner;
pub(in crate::native_bridge::context_host) use document_state::ChildClassicScriptDocumentState;
pub(in crate::native_bridge::context_host) use prepare::ChildParserClassicScriptCandidate;
use prepare::prepare_child_parser_classic_script;

#[derive(Clone, Copy, Debug)]
enum ChildParserDeferredDelayAction {
    Retain,
    Release(DocumentLoadDelayTokenId),
}

#[derive(Clone, Copy)]
enum ChildParserDeferredFollowupMode {
    Queue,
    ReturnReadyClassic,
}

#[derive(Debug, Clone)]
pub(crate) struct ChildClassicScriptLoadCompletionApplication {
    pub(crate) scheduler_work: Option<FrameDocumentClassicScriptSchedulerWork>,
    pub(crate) queued_document_script_ready: bool,
    pub(crate) queued_document_lifecycle: bool,
}

#[derive(Debug, Clone)]
pub(in crate::native_bridge::context_host) struct QueuedChildParserClassicScript {
    pub(crate) ready_work: Option<FrameDocumentClassicScriptSchedulerWork>,
}

#[derive(Debug, Clone)]
pub(in crate::native_bridge::context_host) struct PendingChildExternalClassicDocumentScriptLoad {
    child_handle: DomHandle,
    owner: FrameDocumentTaskOwner,
    realm_id: Option<FrameRealmId>,
    owner_document_id: DocumentId,
    owner_request_id: FrameRequestId,
    script_handle: DomHandle,
    load_delay: ChildDocumentAsyncClassicScriptLoadDelay,
    script: PreparedScript,
}

impl ChildBrowsingContextEntry {
    fn enter_child_parser_script_nesting(&mut self) {
        self.classic_script_document_state
            .enter_parser_script_nesting();
    }

    fn exit_child_parser_script_nesting(&mut self) {
        self.classic_script_document_state
            .exit_parser_script_nesting();
    }

    fn is_executing_child_parser_script(&self) -> bool {
        self.classic_script_document_state
            .is_executing_parser_script()
    }

    fn push_child_current_script(&mut self, script_handle: DomHandle) {
        self.classic_script_document_state
            .push_current_script(script_handle);
    }

    fn pop_child_current_script(&mut self, script_handle: DomHandle) {
        self.classic_script_document_state
            .pop_current_script(script_handle);
    }

    fn current_child_script(&self) -> Option<DomHandle> {
        self.classic_script_document_state.current_script()
    }
}

impl JsContextHost {
    fn child_classic_script_network_attribution(
        &self,
        child_handle: DomHandle,
        owner: FrameDocumentTaskOwner,
        request_url: url::Url,
    ) -> Option<ChildClassicScriptNetworkAttribution> {
        let snapshot = self.frame_owner_current_child_snapshot(child_handle)?;
        let current_owner = FrameDocumentTaskOwner::new(
            snapshot.scheduler_lane_id,
            snapshot.local_window_id,
            snapshot.document_id,
        );
        if current_owner != owner {
            return None;
        }
        Some(ChildClassicScriptNetworkAttribution {
            frame_id: Some(snapshot.frame_id.0),
            document_url: snapshot.document_url,
            request_url,
        })
    }

    pub(crate) fn enter_child_parser_script_nesting(
        &mut self,
        child_handle: DomHandle,
        owner: FrameDocumentTaskOwner,
    ) -> bool {
        if self.current_child_document_task_owner(child_handle) != Some(owner) {
            return false;
        }
        let Some(entry) = self.child_browsing_contexts.get_mut(&child_handle) else {
            return false;
        };
        entry.enter_child_parser_script_nesting();
        true
    }

    pub(crate) fn exit_child_parser_script_nesting(
        &mut self,
        child_handle: DomHandle,
        owner: FrameDocumentTaskOwner,
    ) {
        if self.current_child_document_task_owner(child_handle) != Some(owner) {
            return;
        }
        if let Some(entry) = self.child_browsing_contexts.get_mut(&child_handle) {
            entry.exit_child_parser_script_nesting();
        }
    }

    pub(crate) fn child_document_is_executing_parser_script(
        &self,
        document_handle: DomHandle,
    ) -> bool {
        let Some(child_handle) =
            self.child_browsing_context_host_for_document_handle(document_handle)
        else {
            return false;
        };
        self.child_browsing_contexts
            .get(&child_handle)
            .is_some_and(ChildBrowsingContextEntry::is_executing_child_parser_script)
    }

    pub(crate) fn push_frame_script_job_current_script(
        &mut self,
        job: &FrameScriptJob,
    ) -> Option<(DomHandle, DomHandle)> {
        let script_handle = job.current_script?;
        let child_handle = self.frame_owner_child_handle_for_script_job(job)?;
        let entry = self.child_browsing_contexts.get_mut(&child_handle)?;
        entry.push_child_current_script(script_handle);
        Some((child_handle, script_handle))
    }

    pub(crate) fn pop_child_current_script(&mut self, token: (DomHandle, DomHandle)) {
        let (child_handle, script_handle) = token;
        if let Some(entry) = self.child_browsing_contexts.get_mut(&child_handle) {
            entry.pop_child_current_script(script_handle);
        }
    }

    pub(crate) fn child_current_script_handle_for_document(
        &self,
        document_handle: DomHandle,
    ) -> Option<DomHandle> {
        let child_handle = self.child_browsing_context_host_for_document_handle(document_handle)?;
        self.child_browsing_contexts
            .get(&child_handle)
            .and_then(ChildBrowsingContextEntry::current_child_script)
    }

    pub(in crate::native_bridge::context_host) fn queue_child_external_classic_document_script_for_current_document(
        &mut self,
        handle: DomHandle,
        document_handle: DomHandle,
        script_handle: DomHandle,
        mut script: PreparedScript,
    ) -> bool {
        if !matches!(script.source, ScriptSource::External) {
            return false;
        }
        if !self.child_browsing_contexts.contains_key(&handle)
            || self.child_browsing_context_document_handle(handle) != Some(document_handle)
            || self.dom_host().owner_document_handle(script_handle) != Some(document_handle)
        {
            return false;
        }
        let Some((owner, realm_id)) = self
            .frame_owner_store
            .current_child_document_task_owner_reserved_realm(handle)
        else {
            return false;
        };
        let Some(loader) = self.document_resource_loader_for_owner(owner) else {
            return false;
        };
        let Some(network_attribution) =
            self.child_classic_script_network_attribution(handle, owner, script.url.clone())
        else {
            return false;
        };
        let Some((owner_document_id, owner_request_id)) = self
            .frame_owner_store
            .begin_child_frame_request(handle, FrameRequestKind::ClassicScript)
        else {
            return false;
        };
        if owner_document_id != owner.document_id {
            let _ = self
                .frame_owner_store
                .finish_document_request(owner_document_id, owner_request_id);
            return false;
        }
        let Some(load_delay) = self
            .frame_owner_store
            .acquire_current_child_async_classic_script_load_delay(handle, owner)
        else {
            let _ = self
                .frame_owner_store
                .finish_document_request(owner_document_id, owner_request_id);
            return false;
        };
        let load_id = self.next_child_classic_script_load_id;
        self.next_child_classic_script_load_id =
            self.next_child_classic_script_load_id.wrapping_add(1);
        script.node_id = crate::dom::NodeId::new(script_handle.index());
        let script_for_load = script.clone();
        self.pending_child_external_classic_document_scripts.insert(
            load_id,
            PendingChildExternalClassicDocumentScriptLoad {
                child_handle: handle,
                owner,
                realm_id: Some(realm_id),
                owner_document_id,
                owner_request_id,
                script_handle,
                load_delay,
                script,
            },
        );
        let _ = self
            .dom_host_mut()
            .set_script_already_started(script_handle, true);
        let document_character_set = self
            .frame_owner_current_child_snapshot(handle)
            .and_then(|snapshot| {
                self.child_browsing_context_character_set_for_document_handle(
                    snapshot.document_handle,
                )
                .map(str::to_owned)
            })
            .unwrap_or_else(|| self.document_character_set().to_owned());
        let completion_tx = self.resource_completion_tx.clone();
        let task_loader = loader.clone();
        loader.spawn_resource_task(async move {
            let outcome =
                crate::planning::load_prepared_script_source_outcome_with_document_character_set(
                    &script_for_load,
                    task_loader.request_client(),
                    Some(&document_character_set),
                    None,
                    task_loader.task_runner(),
                )
                .await;
            let _ = completion_tx.send_child_classic_script(ChildClassicScriptLoadCompletion {
                owner,
                load_id,
                handle,
                script_handle,
                result: outcome.source_result,
                network_result: outcome.network_result,
                network_attribution,
            });
        });
        true
    }

    fn apply_child_external_classic_document_script_load_completion(
        &mut self,
        completion: &ChildClassicScriptLoadCompletion,
    ) -> Option<ChildClassicScriptLoadCompletionApplication> {
        if self
            .pending_child_external_classic_document_scripts
            .get(&completion.load_id)?
            .owner
            != completion.owner
        {
            return None;
        }
        let pending = self
            .pending_child_external_classic_document_scripts
            .remove(&completion.load_id)?;
        let _ = self
            .frame_owner_store
            .finish_document_request(pending.owner_document_id, pending.owner_request_id);
        if pending.child_handle != completion.handle
            || pending.script_handle != completion.script_handle
        {
            return Some(ChildClassicScriptLoadCompletionApplication {
                scheduler_work: None,
                queued_document_script_ready: false,
                queued_document_lifecycle: self.settle_child_async_classic_script_load_delay(
                    pending.child_handle,
                    pending.owner,
                    pending.load_delay,
                ),
            });
        }
        let realm_currentness = pending.realm_id.map(|realm_id| {
            self.frame_owner_store
                .child_document_task_owner_realm_currentness(
                    pending.child_handle,
                    pending.owner,
                    realm_id,
                )
        });
        let owner_current = realm_currentness
            .is_some_and(crate::frame_owner_model::FrameDocumentTaskRealmCurrentness::names_current_document_realm);
        if let Some(network_result) = completion.network_result.as_deref() {
            if owner_current {
                self.record_get_subresource_network_result(
                    completion.network_attribution.frame_id.clone(),
                    completion.network_attribution.document_url.clone(),
                    completion.network_attribution.request_url.clone(),
                    SubresourceResourceType::Script,
                    network_result,
                );
            } else {
                self.record_historical_child_classic_script_network_result(completion);
            }
        }
        let document_script_admission = if owner_current {
            let work = PendingChildExternalClassicDocumentScript {
                child_handle: pending.child_handle,
                owner: pending.owner,
                realm_id: pending.realm_id,
                script_handle: pending.script_handle,
                load_delay: pending.load_delay,
                source_result: completion.result.clone(),
                script_url: pending.script.url,
                script_base_url: pending.script.base_url,
            };
            self.queue_ready_child_external_classic_document_script(work)
        } else {
            None
        };
        let queued_document_script_ready = document_script_admission
            .is_some_and(crate::frame_owner_model::FrameDocumentScriptWorkAdmission::is_runnable);
        let queued_document_lifecycle = if document_script_admission.is_some() {
            false
        } else {
            self.settle_child_async_classic_script_load_delay(
                pending.child_handle,
                pending.owner,
                pending.load_delay,
            )
        };
        Some(ChildClassicScriptLoadCompletionApplication {
            scheduler_work: None,
            queued_document_script_ready,
            queued_document_lifecycle,
        })
    }

    fn queue_ready_child_external_classic_document_script(
        &mut self,
        work: PendingChildExternalClassicDocumentScript,
    ) -> Option<crate::frame_owner_model::FrameDocumentScriptWorkAdmission> {
        self.queue_child_document_script_work_with_realm_prerequisite(
            FrameDocumentUnboundScriptWork::ExternalClassic(work),
        )
    }

    pub(crate) fn child_external_classic_script_execution_action_for_owner(
        &self,
        work: &PendingChildExternalClassicDocumentScript,
        realm_id: FrameRealmId,
    ) -> Option<FrameDocumentExternalClassicScriptExecutionAction> {
        let current_realm_id = self
            .frame_owner_store
            .current_materialized_realm_id_for_document_task_owner(work.owner);
        if current_realm_id != Some(realm_id) || work.realm_id.is_some_and(|id| id != realm_id) {
            return None;
        }
        let execution = match &work.source_result {
            Ok(source) => FrameDocumentExternalClassicScriptExecution::script_job(
                self.frame_owner_store
                    .child_external_classic_script_job_for_owner(
                        work.child_handle,
                        work.owner.local_window_id,
                        work.owner.document_id,
                        Some(work.script_handle),
                        work.script_url.clone(),
                        work.script_base_url.clone(),
                        source.clone(),
                    )?,
            ),
            Err(message) => {
                FrameDocumentExternalClassicScriptExecution::source_failure(message.clone())
            }
        };
        Some(FrameDocumentExternalClassicScriptExecutionAction::new(
            work.execution_target(realm_id),
            execution,
        ))
    }

    pub(crate) fn settle_child_async_classic_script_load_delay(
        &mut self,
        child_handle: DomHandle,
        owner: FrameDocumentTaskOwner,
        load_delay: ChildDocumentAsyncClassicScriptLoadDelay,
    ) -> bool {
        let Some(token) = load_delay.token() else {
            return false;
        };
        if !self
            .frame_owner_store
            .release_async_classic_script_load_delay(owner, token)
        {
            return false;
        }
        self.queue_child_document_complete_lifecycle_if_ready(child_handle)
    }

    pub(crate) fn dispatch_child_script_element_event<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        event: FrameDocumentScriptElementEvent,
    ) -> bool {
        let Some(snapshot) = self.frame_owner_current_child_snapshot(event.child_handle) else {
            return false;
        };
        if snapshot.local_window_id != event.owner.local_window_id
            || snapshot.document_id != event.owner.document_id
        {
            return false;
        }
        if self.dom_host().owner_document_handle(event.script_handle)
            != Some(snapshot.document_handle)
        {
            return false;
        }
        let _ = self.child_browsing_context_document_wrapper(scope, event.child_handle);
        let host_ptr = self as *mut JsContextHost;
        let Some(target) =
            self.native_bridge_mut()
                .wrap_handle(scope, host_ptr, event.script_handle)
        else {
            return false;
        };
        dispatch_detached_simple_event(scope, target, event.kind.event_type(), false, false, false)
    }

    pub(crate) fn current_child_classic_source_load_target(
        &self,
        expected: RendererPageChildClassicScriptSourceLoadTarget,
    ) -> Option<RendererPageChildClassicScriptSourceLoadTarget> {
        if !self.frame_owner_store.child_document_task_owner_is_current(
            expected.child_handle(),
            expected.document_owner(),
        ) || self
            .frame_owner_store
            .current_reserved_realm_id_for_document_task_owner(expected.document_owner())
            != Some(expected.realm_id())
        {
            return None;
        }
        let client = self.child_classic_script_source_load_client_for_owner(
            expected.child_handle(),
            expected.document_owner().document_owner(),
        )?;
        (client.metadata().script_handle() == expected.script_handle()).then_some(expected)
    }

    pub(crate) fn start_current_child_classic_source_load_task(
        &mut self,
        task: FrameDocumentClassicScriptSourceLoadTask,
    ) -> crate::frame_owner_model::FrameDocumentClassicScriptSourceLoadStartOutcome {
        use crate::frame_owner_model::FrameDocumentClassicScriptSourceLoadStartOutcome;

        let task_owner = task.owner();
        let client = task.client().clone();
        let client_target = *client.target();
        let child_handle = client_target.child_handle();
        if !self.child_browsing_contexts.contains_key(&child_handle) {
            let _ = self.fail_child_classic_source_load_before_start(
                &task,
                "child browsing context disappeared before classic source fetch start",
            );
            return FrameDocumentClassicScriptSourceLoadStartOutcome::RejectedBeforeNetworkStart;
        }
        let owner = client_target.owner();
        if task_owner.document_owner() != owner
            || !self.frame_parser_classic_scripts.has_runner(owner)
        {
            let _ = self.fail_child_classic_source_load_before_start(
                &task,
                "child parser classic runner disappeared before source fetch start",
            );
            return FrameDocumentClassicScriptSourceLoadStartOutcome::RejectedBeforeNetworkStart;
        }
        let Some(loader) = self.document_resource_loader_for_owner(task_owner) else {
            let _ = self.fail_child_classic_source_load_before_start(
                &task,
                "child classic source fetch lost its exact Document loader",
            );
            return FrameDocumentClassicScriptSourceLoadStartOutcome::RejectedBeforeNetworkStart;
        };
        let Some(network_attribution) = self.child_classic_script_network_attribution(
            child_handle,
            task_owner,
            client.script_url().clone(),
        ) else {
            let _ = self.fail_child_classic_source_load_before_start(
                &task,
                "child classic source fetch lost its exact network attribution",
            );
            return FrameDocumentClassicScriptSourceLoadStartOutcome::RejectedBeforeNetworkStart;
        };
        let Some((owner_document_id, owner_request_id)) = self
            .frame_owner_store
            .begin_child_frame_request(child_handle, FrameRequestKind::ClassicScript)
        else {
            let _ = self.fail_child_classic_source_load_before_start(
                &task,
                "child classic source fetch could not admit its Document request",
            );
            return FrameDocumentClassicScriptSourceLoadStartOutcome::RejectedBeforeNetworkStart;
        };
        if owner_document_id != task_owner.document_id {
            let _ = self
                .frame_owner_store
                .finish_document_request(owner_document_id, owner_request_id);
            let _ = self.fail_child_classic_source_load_before_start(
                &task,
                "child classic source fetch request named another Document",
            );
            return FrameDocumentClassicScriptSourceLoadStartOutcome::RejectedBeforeNetworkStart;
        }
        let load_id = self.next_child_classic_script_load_id;
        self.next_child_classic_script_load_id =
            self.next_child_classic_script_load_id.wrapping_add(1);
        let Some(request) = self.frame_parser_classic_scripts.begin_external_load(
            owner,
            &client,
            load_id,
            task_owner,
            owner_request_id,
        ) else {
            let _ = self
                .frame_owner_store
                .finish_document_request(owner_document_id, owner_request_id);
            let _ = self.fail_child_classic_source_load_before_start(
                &task,
                "child classic PendingScript changed before source fetch start",
            );
            return FrameDocumentClassicScriptSourceLoadStartOutcome::RejectedBeforeNetworkStart;
        };
        self.spawn_child_classic_source_load_request(request, loader, network_attribution);
        FrameDocumentClassicScriptSourceLoadStartOutcome::NetworkRequestStarted
    }

    pub(crate) fn fail_child_classic_source_load_before_start(
        &mut self,
        task: &FrameDocumentClassicScriptSourceLoadTask,
        error: &str,
    ) -> bool {
        let owner = task.owner();
        if !self
            .frame_owner_store
            .child_document_task_owner_is_current(task.child_handle(), owner)
            || self
                .frame_owner_store
                .current_reserved_realm_id_for_document_task_owner(owner)
                != Some(task.realm_id())
        {
            return false;
        }
        self.fail_child_classic_source_load_client_before_start(
            task.child_handle(),
            owner,
            task.client(),
            error,
        )
    }

    pub(in crate::native_bridge::context_host) fn fail_child_classic_source_load_client_before_start(
        &mut self,
        child_handle: DomHandle,
        owner: FrameDocumentTaskOwner,
        client: &FrameDocumentClassicScriptSourceLoadClient,
        error: &str,
    ) -> bool {
        if self
            .frame_owner_store
            .current_child_document_task_owner(child_handle)
            != Some(owner)
            || client.target().child_handle() != child_handle
            || client.target().owner() != owner.document_owner()
        {
            return false;
        }
        if !self
            .frame_parser_classic_scripts
            .fail_external_pending_before_load(owner.document_owner(), client, error.to_owned())
        {
            return false;
        }
        if let Some(work) =
            self.take_child_classic_script_scheduler_work_for_current_document(child_handle)
        {
            self.child_document_script_schedulers
                .notify_parser_classic_next_owner_action(work);
            self.admit_runnable_child_document_script_tasks();
        }
        true
    }

    fn spawn_child_classic_source_load_request(
        &mut self,
        request: crate::frame_owner_model::FrameDocumentClassicScriptSourceLoadRequest,
        loader: crate::network::context::DocumentResourceLoader,
        network_attribution: ChildClassicScriptNetworkAttribution,
    ) {
        let request_target = *request.target();
        let document_character_set = self
            .frame_owner_current_child_snapshot(request_target.child_handle())
            .and_then(|snapshot| {
                self.child_browsing_context_character_set_for_document_handle(
                    snapshot.document_handle,
                )
                .map(str::to_owned)
            })
            .unwrap_or_else(|| self.document_character_set().to_owned());
        let child_handle = request_target.child_handle();
        let (_request_target, source_load) = request.into_parts();
        let (source_identity, input) = source_load.into_parts();
        let completion_tx = self.resource_completion_tx.clone();
        let script_handle = source_identity.metadata().script_handle();
        let script_for_load = input.into_script();
        let load_id = source_identity
            .load_id()
            .expect("child classic external source load request must carry a load id");
        if let Some(outcome) = crate::planning::immediate_external_script_source_load_outcome(
            &script_for_load,
            Some(&document_character_set),
        ) {
            let application =
                self.apply_child_classic_script_load_completion(ChildClassicScriptLoadCompletion {
                    owner: request_target.task_owner(),
                    load_id,
                    handle: child_handle,
                    script_handle,
                    result: outcome.source_result,
                    network_result: outcome.network_result,
                    network_attribution,
                });
            if let Some(work) = application.and_then(|application| application.scheduler_work) {
                self.child_document_script_schedulers
                    .notify_parser_classic_next_owner_action(work);
                self.admit_runnable_child_document_script_tasks();
            }
            return;
        }
        let task_loader = loader.clone();
        loader.spawn_resource_task(async move {
            let outcome =
                crate::planning::load_prepared_script_source_outcome_with_document_character_set(
                    &script_for_load,
                    task_loader.request_client(),
                    Some(&document_character_set),
                    None,
                    task_loader.task_runner(),
                )
                .await;
            let _ = completion_tx.send_child_classic_script(ChildClassicScriptLoadCompletion {
                owner: request_target.task_owner(),
                load_id,
                handle: child_handle,
                script_handle,
                result: outcome.source_result,
                network_result: outcome.network_result,
                network_attribution,
            });
        });
    }

    fn child_classic_script_source_load_client_for_owner(
        &self,
        handle: DomHandle,
        owner: FrameDocumentOwner,
    ) -> Option<FrameDocumentClassicScriptSourceLoadClient> {
        if !self.child_browsing_contexts.contains_key(&handle) {
            return None;
        }
        let client = self
            .frame_parser_classic_scripts
            .source_load_client(owner, handle)?;
        if client.target().owner() == owner {
            return Some(client);
        }
        None
    }

    pub(crate) fn apply_child_classic_script_load_completion(
        &mut self,
        completion: ChildClassicScriptLoadCompletion,
    ) -> Option<ChildClassicScriptLoadCompletionApplication> {
        if let Some(application) =
            self.apply_child_external_classic_document_script_load_completion(&completion)
        {
            return Some(application);
        }
        let Some(current_task_owner) = self
            .frame_owner_store
            .current_child_document_task_owner(completion.handle)
        else {
            self.record_historical_child_classic_script_network_result(&completion);
            return None;
        };
        if current_task_owner != completion.owner {
            self.record_historical_child_classic_script_network_result(&completion);
            return None;
        }
        let current_owner = current_task_owner.document_owner();
        let owner = self
            .frame_parser_classic_scripts
            .external_load_owner(current_owner, &completion)?;
        let owner_target = *owner.target();
        let owner_current = self.frame_owner_store.document_request_is_current(
            owner_target.owner_document_id(),
            owner_target.owner_request_id(),
            FrameRequestKind::ClassicScript,
        );
        let _ = self.frame_owner_store.finish_document_request(
            owner_target.owner_document_id(),
            owner_target.owner_request_id(),
        );
        let (_owner_target, source_load) = owner.into_parts();
        if let Some(network_result) = completion.network_result.as_deref() {
            self.record_get_subresource_network_result(
                completion.network_attribution.frame_id.clone(),
                completion.network_attribution.document_url.clone(),
                completion.network_attribution.request_url.clone(),
                SubresourceResourceType::Script,
                network_result,
            );
        }
        let handle = completion.handle;
        let source_result = source_load.into_source_result(completion.result);
        let notification = self
            .frame_parser_classic_scripts
            .notify_external_source_result(current_owner, source_result, owner_current)?;
        match notification {
            ParserPendingClassicScriptNotification::SourceReady
            | ParserPendingClassicScriptNotification::SourceFailed => {
                let scheduler_work =
                    self.take_child_classic_script_scheduler_work_for_current_document(handle);
                let queued_document_script_ready = scheduler_work.is_none()
                    && self.queue_next_child_parser_deferred_script_if_ready(
                        handle,
                        self.frame_owner_store
                            .current_child_document_task_owner(handle)?,
                    );
                Some(ChildClassicScriptLoadCompletionApplication {
                    scheduler_work,
                    queued_document_script_ready,
                    queued_document_lifecycle: false,
                })
            }
        }
    }

    pub(crate) fn record_historical_child_classic_script_network_result(
        &mut self,
        completion: &ChildClassicScriptLoadCompletion,
    ) -> bool {
        let Some(network_result) = completion.network_result.as_deref() else {
            return false;
        };
        self.record_historical_get_subresource_network_result_with_initiator(
            completion.network_attribution.frame_id.clone(),
            completion.network_attribution.document_url.clone(),
            completion.network_attribution.request_url.clone(),
            SubresourceResourceType::Script,
            crate::types::SubresourceRequestInitiatorType::Script,
            network_result,
        );
        true
    }
}

impl ChildBrowsingContextEntry {
    pub(super) fn clear_child_classic_script_document_state(&mut self) {
        self.classic_script_document_state.clear();
    }
}

impl JsContextHost {
    pub(in crate::native_bridge::context_host) fn install_empty_child_classic_script_runner_for_current_document(
        &mut self,
        handle: DomHandle,
        owner_local_window_id: LocalWindowId,
        owner_document_id: DocumentId,
    ) -> bool {
        let owner = FrameDocumentOwner::new(owner_local_window_id, owner_document_id);
        if !self.child_browsing_contexts.contains_key(&handle) {
            return false;
        }
        self.frame_parser_classic_scripts.install_empty(owner);
        true
    }

    pub(in crate::native_bridge::context_host) fn push_child_parser_classic_script_for_current_document(
        &mut self,
        handle: DomHandle,
        document_handle: DomHandle,
        parser_script: ChildParserClassicScriptCandidate,
    ) -> Option<QueuedChildParserClassicScript> {
        let task_owner = self
            .frame_owner_store
            .current_child_document_task_owner(handle)?;
        let runner_owner = task_owner.document_owner();
        if !self.frame_parser_classic_scripts.has_runner(runner_owner) {
            return None;
        }
        if !self
            .frame_owner_store
            .child_document_owner_is_current(handle, runner_owner)
        {
            return None;
        }
        if !self
            .frame_parser_classic_scripts
            .accepts_owner_document_handle(runner_owner, document_handle)
        {
            return None;
        }
        let scheduling = parser_script.scheduling();
        let deferred_loader = (scheduling == FrameDocumentClassicScriptScheduling::Deferred)
            .then(|| self.document_resource_loader_for_owner(task_owner))
            .flatten();
        if scheduling == FrameDocumentClassicScriptScheduling::Deferred && deferred_loader.is_none()
        {
            return None;
        }
        let deferred_order_key = (scheduling == FrameDocumentClassicScriptScheduling::Deferred)
            .then(|| parser_script.pending_script_key());
        let load_delay_token = if scheduling == FrameDocumentClassicScriptScheduling::Deferred {
            Some(
                self.frame_owner_store
                    .acquire_current_child_parser_deferred_script_load_delay(handle, task_owner)?,
            )
        } else {
            None
        };
        let Some(pending_script) = prepare_child_parser_classic_script(
            self.dom_host_mut(),
            task_owner,
            document_handle,
            parser_script,
            load_delay_token,
        ) else {
            if let Some(token) = load_delay_token {
                let _ = self
                    .frame_owner_store
                    .release_parser_deferred_script_load_delay(task_owner, token);
            }
            return None;
        };
        if scheduling == FrameDocumentClassicScriptScheduling::Deferred {
            let loader = deferred_loader?;
            let network_attribution = pending_script
                .runner_external_pending_script_url()
                .cloned()
                .and_then(|request_url| {
                    self.child_classic_script_network_attribution(handle, task_owner, request_url)
                });
            let Some(network_attribution) = network_attribution else {
                let _ = self
                    .frame_owner_store
                    .release_parser_deferred_script_load_delay(
                        task_owner,
                        load_delay_token.expect("deferred classic must own a load-delay token"),
                    );
                return None;
            };
            let Some((owner_document_id, owner_request_id)) = self
                .frame_owner_store
                .begin_child_frame_request(handle, FrameRequestKind::ClassicScript)
            else {
                let _ = self
                    .frame_owner_store
                    .release_parser_deferred_script_load_delay(
                        task_owner,
                        load_delay_token.expect("deferred classic must own a load-delay token"),
                    );
                return None;
            };
            if owner_document_id != runner_owner.document_id {
                let _ = self
                    .frame_owner_store
                    .finish_document_request(owner_document_id, owner_request_id);
                let _ = self
                    .frame_owner_store
                    .release_parser_deferred_script_load_delay(
                        task_owner,
                        load_delay_token.expect("deferred classic must own a load-delay token"),
                    );
                return None;
            }
            let load_id = self.next_child_classic_script_load_id;
            self.next_child_classic_script_load_id =
                self.next_child_classic_script_load_id.wrapping_add(1);
            let Some(request) = self
                .frame_parser_classic_scripts
                .push_deferred_external_script_and_begin_load(
                    runner_owner,
                    handle,
                    pending_script,
                    load_id,
                    task_owner,
                    owner_request_id,
                )
            else {
                let _ = self
                    .frame_owner_store
                    .finish_document_request(owner_document_id, owner_request_id);
                let _ = self
                    .frame_owner_store
                    .release_parser_deferred_script_load_delay(
                        task_owner,
                        load_delay_token.expect("deferred classic must own a load-delay token"),
                    );
                return None;
            };
            let order_key = deferred_order_key
                .expect("deferred child parser classic must carry a preparation-time key");
            let order_registered = self.frame_parser_deferred_script_order.register(
                runner_owner,
                crate::document_script_scheduler::FrameParserDeferredScriptOrderEntry::classic(
                    order_key,
                ),
            );
            if !order_registered {
                let _ = self
                    .frame_parser_classic_scripts
                    .discard_current_deferred_script_if_key(runner_owner, order_key);
                let _ = self
                    .frame_owner_store
                    .finish_document_request(owner_document_id, owner_request_id);
                let _ = self
                    .frame_owner_store
                    .release_parser_deferred_script_load_delay(
                        task_owner,
                        load_delay_token.expect("deferred classic must own a load-delay token"),
                    );
                tracing::warn!(
                    owner = ?runner_owner,
                    parser_position = order_key.parser_position(),
                    script_node_id = ?order_key.script_node_id(),
                    "rejecting child classic defer without an exact parser-order slot"
                );
                return None;
            }
            tracing::debug!(
                owner = ?runner_owner,
                parser_position = order_key.parser_position(),
                script_node_id = ?order_key.script_node_id(),
                ?load_delay_token,
                order_registered,
                "child parser classic PendingScript registered in cross-kind defer order"
            );
            self.spawn_child_classic_source_load_request(request, loader, network_attribution);
            return Some(QueuedChildParserClassicScript { ready_work: None });
        }
        let pushed = self
            .frame_parser_classic_scripts
            .push_prepared_parser_script(runner_owner, document_handle, pending_script);
        if !pushed {
            return None;
        }
        let ready_work = self.take_child_classic_script_scheduler_work_for_current_document(handle);
        if ready_work.is_none() {
            // A parser-blocking classic can be non-runnable for two different
            // reasons. External work must publish its concrete fetch-start;
            // inline or stylesheet-blocked work only needs to retain the
            // parser and make its exact execution realm durable. Treating a
            // missing external client as admission failure would let the
            // parser run to EOF before the retained script executes.
            let has_external_source = self
                .child_classic_script_source_load_client_for_owner(handle, runner_owner)
                .is_some();
            let admitted = if has_external_source {
                self.queue_child_classic_script_source_load_task(handle)
            } else {
                self.request_child_frame_realm_materialization_for_owner(handle, task_owner)
                    .is_some()
            };
            if !admitted {
                return None;
            }
        }
        Some(QueuedChildParserClassicScript { ready_work })
    }

    pub(crate) fn clear_child_parser_classic_runner_for_current_document(
        &mut self,
        handle: DomHandle,
    ) {
        if let Some(owner) = self.frame_owner_store.current_child_document_owner(handle) {
            self.frame_parser_deferred_script_order
                .remove_document(owner);
            let _ = self.frame_parser_classic_scripts.remove(owner);
        }
    }

    pub(in crate::native_bridge::context_host) fn take_child_classic_script_scheduler_work_for_current_document(
        &mut self,
        handle: DomHandle,
    ) -> Option<FrameDocumentClassicScriptSchedulerWork> {
        let (owner, realm_id) = self
            .frame_owner_store
            .current_child_document_task_owner_materialized_realm(handle)?;
        let document_owner = owner.document_owner();
        if !self.frame_parser_classic_scripts.has_runner(document_owner) {
            return None;
        }
        let stylesheet_blocked = self
            .frame_parser_classic_scripts
            .current_parser_blocking_stylesheet_signatures(document_owner)
            .is_some_and(|signatures| {
                self.frame_document_blocking_stylesheets
                    .blocks_signatures(document_owner, signatures)
            });
        if stylesheet_blocked {
            tracing::debug!(
                child_handle = ?handle,
                owner = ?owner,
                signatures = ?self
                    .frame_parser_classic_scripts
                    .current_parser_blocking_stylesheet_signatures(document_owner),
                "retaining child parser-blocking classic behind stylesheet readiness"
            );
            return None;
        }
        let work = self
            .frame_parser_classic_scripts
            .next_parser_blocking_task(document_owner, handle, owner, Some(realm_id), true)?;
        let route = work.dispatch_route();
        (self.child_classic_document_script_ready_runner_owner_is_current(&route)
            && self.frame_document_ready_route_task_is_current(&route))
        .then_some(work)
    }

    pub(crate) fn queue_current_child_parser_blocking_script_if_ready(
        &mut self,
        child_handle: DomHandle,
    ) -> bool {
        let Some(work) =
            self.take_child_classic_script_scheduler_work_for_current_document(child_handle)
        else {
            return false;
        };
        self.child_document_script_schedulers
            .notify_parser_classic_next_owner_action(work);
        self.admit_runnable_child_document_script_tasks();
        tracing::debug!(
            child_handle = ?child_handle,
            "promoted child parser-blocking classic after stylesheet/source readiness"
        );
        true
    }

    pub(crate) fn take_child_parser_deferred_classic_work_if_ready(
        &mut self,
        child_handle: DomHandle,
        owner: FrameDocumentTaskOwner,
        head: FrameParserDeferredScriptOrderEntry,
    ) -> Option<FrameDocumentClassicScriptSchedulerWork> {
        if head.kind() != FrameParserDeferredScriptKind::Classic
            || !self
                .frame_owner_store
                .current_child_document_allows_deferred_script_execution(child_handle, owner)
        {
            return None;
        }
        let document_owner = owner.document_owner();
        let current_key = self
            .frame_parser_classic_scripts
            .current_deferred_script_key(document_owner);
        if current_key != Some(head.key()) {
            tracing::warn!(
                child_handle = ?child_handle,
                owner = ?owner,
                parser_position = head.key().parser_position(),
                script_node_id = ?head.key().script_node_id(),
                current_classic_key = ?current_key,
                "child parser-deferred classic order head does not match classic PendingScript head"
            );
            return None;
        }
        let stylesheet_blocked = self
            .frame_parser_classic_scripts
            .current_deferred_stylesheet_signatures(document_owner)
            .is_some_and(|signatures| {
                self.frame_document_blocking_stylesheets
                    .blocks_signatures(document_owner, signatures)
            });
        if stylesheet_blocked {
            tracing::debug!(
                child_handle = ?child_handle,
                owner = ?owner,
                parser_position = head.key().parser_position(),
                script_node_id = ?head.key().script_node_id(),
                signatures = ?self
                    .frame_parser_classic_scripts
                    .current_deferred_stylesheet_signatures(document_owner),
                "retaining child parser-deferred classic head behind stylesheet readiness"
            );
            return None;
        }
        if !self
            .frame_parser_deferred_script_order
            .mark_head_in_flight(document_owner, head)
        {
            return None;
        }
        let realm_id = self
            .frame_owner_store
            .current_reserved_realm_id_for_document_task_owner(owner);
        let Some(work) = self.frame_parser_classic_scripts.next_deferred_task(
            document_owner,
            child_handle,
            owner,
            realm_id,
            true,
        ) else {
            let restored = self
                .frame_parser_deferred_script_order
                .restore_in_flight_head(document_owner, head);
            debug_assert!(restored, "unpromoted classic defer head must be restorable");
            tracing::debug!(
                child_handle = ?child_handle,
                owner = ?owner,
                parser_position = head.key().parser_position(),
                script_node_id = ?head.key().script_node_id(),
                "retaining child parser-deferred classic head because source is not terminal"
            );
            return None;
        };
        Some(work)
    }

    pub(crate) fn queue_next_child_parser_deferred_script_if_ready(
        &mut self,
        child_handle: DomHandle,
        owner: FrameDocumentTaskOwner,
    ) -> bool {
        self.admit_next_child_parser_deferred_script_if_ready(child_handle, owner)
    }

    pub(in crate::native_bridge::context_host) fn admit_next_child_parser_deferred_script_if_ready(
        &mut self,
        child_handle: DomHandle,
        owner: FrameDocumentTaskOwner,
    ) -> bool {
        let document_owner = owner.document_owner();
        if !self
            .frame_owner_store
            .current_child_document_allows_deferred_script_execution(child_handle, owner)
        {
            if let Some(head) = self.frame_parser_deferred_script_order.head(document_owner) {
                tracing::debug!(
                    child_handle = ?child_handle,
                    owner = ?owner,
                    parser_position = head.key().parser_position(),
                    script_node_id = ?head.key().script_node_id(),
                    kind = ?head.kind(),
                    "retaining child parser-deferred head until document becomes interactive"
                );
            }
            return false;
        }
        let Some(head) = self
            .frame_parser_deferred_script_order
            .pending_head(document_owner)
        else {
            return false;
        };
        let realm_id = self
            .frame_owner_store
            .current_reserved_realm_id_for_document_task_owner(owner);
        match head.kind() {
            FrameParserDeferredScriptKind::Classic => {
                let Some(work) = self.take_child_parser_deferred_classic_work_if_ready(
                    child_handle,
                    owner,
                    head,
                ) else {
                    return false;
                };
                self.child_document_script_schedulers
                    .notify_parser_classic_next_owner_action(work);
            }
            FrameParserDeferredScriptKind::Module => {
                let pending_script_id = head
                    .pending_module_script_id(document_owner)
                    .expect("module order head must carry a module PendingScript id");
                let stylesheet_blocked = self
                    .child_document_script_schedulers
                    .parser_ordered_module_blocking_stylesheet_signatures(pending_script_id)
                    .is_some_and(|signatures| {
                        self.frame_document_blocking_stylesheets
                            .blocks_signatures(document_owner, signatures)
                    });
                if stylesheet_blocked {
                    tracing::debug!(
                        child_handle = ?child_handle,
                        owner = ?owner,
                        parser_position = head.key().parser_position(),
                        script_node_id = ?head.key().script_node_id(),
                        signatures = ?self
                            .child_document_script_schedulers
                            .parser_ordered_module_blocking_stylesheet_signatures(
                                pending_script_id
                            ),
                        "retaining child parser-deferred module head behind stylesheet readiness"
                    );
                    return false;
                }
                match self
                    .child_document_script_schedulers
                    .prepare_parser_ordered_module_terminal(pending_script_id)
                {
                    ParserOrderedModuleTerminalState::Ready => {}
                    ParserOrderedModuleTerminalState::Waiting => {
                        tracing::debug!(
                            child_handle = ?child_handle,
                            owner = ?owner,
                            parser_position = head.key().parser_position(),
                            script_node_id = ?head.key().script_node_id(),
                            "watching child parser-deferred module head for its graph terminal"
                        );
                        return false;
                    }
                    ParserOrderedModuleTerminalState::Missing => {
                        tracing::warn!(
                            child_handle = ?child_handle,
                            owner = ?owner,
                            parser_position = head.key().parser_position(),
                            script_node_id = ?head.key().script_node_id(),
                            "child parser-deferred module head lost its PendingScript"
                        );
                        return false;
                    }
                }
                if !self
                    .frame_parser_deferred_script_order
                    .mark_head_in_flight(document_owner, head)
                {
                    return false;
                }
                if !self
                    .child_document_script_schedulers
                    .promote_parser_ordered_module_terminal(pending_script_id)
                {
                    let restored = self
                        .frame_parser_deferred_script_order
                        .restore_in_flight_head(document_owner, head);
                    debug_assert!(restored, "unpromoted module defer head must be restorable");
                    tracing::warn!(
                        child_handle = ?child_handle,
                        owner = ?owner,
                        parser_position = head.key().parser_position(),
                        script_node_id = ?head.key().script_node_id(),
                        "ready child parser-deferred module terminal could not be promoted"
                    );
                    return false;
                }
            }
        }

        let admitted = self.admit_runnable_child_document_script_tasks();
        tracing::debug!(
            child_handle = ?child_handle,
            owner = ?owner,
            realm_id = ?realm_id,
            parser_position = head.key().parser_position(),
            script_node_id = ?head.key().script_node_id(),
            kind = ?head.kind(),
            "claimed child parser-deferred document-order head for DocumentScriptReady"
        );
        admitted != 0
    }

    pub(crate) fn queue_next_child_parser_deferred_script_for_document_realm(
        &mut self,
        owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
    ) -> bool {
        let Some(snapshot) = self
            .frame_owner_store
            .current_child_owner_snapshot_for_realm(realm_id)
        else {
            tracing::debug!(
                owner = ?owner,
                realm_id = ?realm_id,
                "dropping child parser-deferred wake for a retired realm"
            );
            return false;
        };
        let current_owner = FrameDocumentTaskOwner::new(
            snapshot.scheduler_lane_id,
            snapshot.local_window_id,
            snapshot.document_id,
        );
        if current_owner != owner {
            tracing::debug!(
                owner = ?owner,
                current_owner = ?current_owner,
                realm_id = ?realm_id,
                "dropping stale child parser-deferred wake"
            );
            return false;
        }
        self.queue_next_child_parser_deferred_script_if_ready(snapshot.owner_handle, owner)
    }

    pub(crate) fn complete_child_deferred_classic_script(
        &mut self,
        target: FrameDocumentClassicScriptCompletionTarget,
    ) -> FrameDocumentClassicDeferredCompletionApplication {
        if target.scheduling() != FrameDocumentClassicScriptScheduling::Deferred
            || !self
                .frame_owner_store
                .child_document_task_owner_realm_currentness(
                    target.child_handle(),
                    target.task_owner(),
                    target.realm_id(),
                )
                .names_current_document_realm()
        {
            return FrameDocumentClassicDeferredCompletionApplication::new(None, false, false);
        }
        let Some(key) = target.pending_script_key() else {
            tracing::warn!(
                child_handle = ?target.child_handle(),
                owner = ?target.task_owner(),
                "child classic defer completion has no preparation-time PendingScript key"
            );
            return FrameDocumentClassicDeferredCompletionApplication::new(None, false, false);
        };
        self.complete_child_parser_deferred_order_entry(
            target.child_handle(),
            target.task_owner(),
            FrameParserDeferredScriptOrderEntry::classic(key),
            ChildParserDeferredDelayAction::Release(
                target
                    .load_delay_token()
                    .expect("accepted classic defer must retain its lifecycle delay token"),
            ),
            ChildParserDeferredFollowupMode::ReturnReadyClassic,
        )
    }

    pub(crate) fn cancel_child_deferred_classic_ready_work(
        &mut self,
        target: crate::frame_owner_model::FrameDocumentClassicScriptReadyTarget,
        script_handle: DomHandle,
    ) -> FrameDocumentClassicDeferredCompletionApplication {
        if target.scheduling() != FrameDocumentClassicScriptScheduling::Deferred
            || !self
                .frame_owner_store
                .child_document_task_owner_is_current(target.child_handle(), target.task_owner())
        {
            return FrameDocumentClassicDeferredCompletionApplication::new(None, false, false);
        }
        let Some(key) = target.pending_script_key() else {
            return FrameDocumentClassicDeferredCompletionApplication::new(None, false, false);
        };
        if key.script_node_id() != script_handle
            || !self
                .frame_parser_classic_scripts
                .discard_current_deferred_script_if_key(target.task_owner().document_owner(), key)
        {
            tracing::warn!(
                child_handle = ?target.child_handle(),
                owner = ?target.task_owner(),
                parser_position = key.parser_position(),
                script_node_id = ?key.script_node_id(),
                ?script_handle,
                "could not cancel the exact in-flight child classic defer PendingScript"
            );
            return FrameDocumentClassicDeferredCompletionApplication::new(None, false, false);
        }
        tracing::debug!(
            child_handle = ?target.child_handle(),
            owner = ?target.task_owner(),
            parser_position = key.parser_position(),
            script_node_id = ?key.script_node_id(),
            "disposed child classic defer PendingScript without execution"
        );
        self.complete_child_parser_deferred_order_entry(
            target.child_handle(),
            target.task_owner(),
            FrameParserDeferredScriptOrderEntry::classic(key),
            ChildParserDeferredDelayAction::Release(
                target
                    .load_delay_token()
                    .expect("accepted classic defer must retain its lifecycle delay token"),
            ),
            ChildParserDeferredFollowupMode::Queue,
        )
    }

    pub(crate) fn complete_child_deferred_classic_terminal_without_event(
        &mut self,
        target: crate::frame_owner_model::FrameDocumentClassicScriptSourceFailureTarget,
        script_handle: DomHandle,
    ) -> FrameDocumentClassicDeferredCompletionApplication {
        if target.scheduling() != FrameDocumentClassicScriptScheduling::Deferred
            || !self
                .frame_owner_store
                .child_document_task_owner_is_current(target.child_handle(), target.task_owner())
        {
            return FrameDocumentClassicDeferredCompletionApplication::new(None, false, false);
        }
        let Some(key) = target.pending_script_key() else {
            return FrameDocumentClassicDeferredCompletionApplication::new(None, false, false);
        };
        if key.script_node_id() != script_handle {
            return FrameDocumentClassicDeferredCompletionApplication::new(None, false, false);
        }
        self.complete_child_parser_deferred_order_entry(
            target.child_handle(),
            target.task_owner(),
            FrameParserDeferredScriptOrderEntry::classic(key),
            ChildParserDeferredDelayAction::Release(
                target
                    .load_delay_token()
                    .expect("accepted classic defer must retain its lifecycle delay token"),
            ),
            ChildParserDeferredFollowupMode::Queue,
        )
    }

    pub(crate) fn complete_child_parser_deferred_module_script_for_document_realm(
        &mut self,
        owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
        pending_script_id: ParserPendingScriptId<FrameDocumentOwner>,
    ) -> FrameDocumentClassicDeferredCompletionApplication {
        if pending_script_id.owner() != owner.document_owner() {
            return FrameDocumentClassicDeferredCompletionApplication::new(None, false, false);
        }
        let Some(snapshot) = self
            .frame_owner_store
            .current_child_owner_snapshot_for_realm(realm_id)
        else {
            return FrameDocumentClassicDeferredCompletionApplication::new(None, false, false);
        };
        let current_owner = FrameDocumentTaskOwner::new(
            snapshot.scheduler_lane_id,
            snapshot.local_window_id,
            snapshot.document_id,
        );
        if current_owner != owner {
            return FrameDocumentClassicDeferredCompletionApplication::new(None, false, false);
        }
        self.complete_child_parser_deferred_order_entry(
            snapshot.owner_handle,
            owner,
            FrameParserDeferredScriptOrderEntry::module(pending_script_id),
            ChildParserDeferredDelayAction::Retain,
            ChildParserDeferredFollowupMode::Queue,
        )
    }

    pub(crate) fn cancel_current_child_parser_deferred_module_script(
        &mut self,
        owner: FrameDocumentTaskOwner,
        pending_script_key: ParserPendingScriptKey,
    ) -> FrameDocumentClassicDeferredCompletionApplication {
        let Some(current_realm_id) = self
            .frame_owner_store
            .current_reserved_realm_id_for_document_task_owner(owner)
        else {
            return FrameDocumentClassicDeferredCompletionApplication::new(None, false, false);
        };
        self.complete_child_parser_deferred_module_script_for_document_realm(
            owner,
            current_realm_id,
            ParserPendingScriptId::from_key(owner.document_owner(), pending_script_key),
        )
    }

    fn complete_child_parser_deferred_order_entry(
        &mut self,
        child_handle: DomHandle,
        owner: FrameDocumentTaskOwner,
        entry: FrameParserDeferredScriptOrderEntry,
        delay_action: ChildParserDeferredDelayAction,
        followup_mode: ChildParserDeferredFollowupMode,
    ) -> FrameDocumentClassicDeferredCompletionApplication {
        if !self
            .frame_owner_store
            .child_document_task_owner_is_current(child_handle, owner)
            || self
                .frame_parser_deferred_script_order
                .in_flight_head(owner.document_owner())
                != Some(entry)
            || !self
                .frame_parser_deferred_script_order
                .release_in_flight_head(owner.document_owner(), entry)
        {
            return FrameDocumentClassicDeferredCompletionApplication::new(None, false, false);
        }
        if let ChildParserDeferredDelayAction::Release(load_delay_token) = delay_action {
            if !self
                .frame_owner_store
                .release_parser_deferred_script_load_delay(owner, load_delay_token)
            {
                tracing::error!(
                    ?child_handle,
                    ?owner,
                    ?entry,
                    ?load_delay_token,
                    "parser-deferred completion could not release its exact lifecycle delay"
                );
            } else {
                tracing::debug!(
                    ?child_handle,
                    ?owner,
                    ?entry,
                    ?load_delay_token,
                    "released parser-deferred lifecycle delay after exact order completion"
                );
            }
        }
        let scheduler_work = match followup_mode {
            ChildParserDeferredFollowupMode::Queue => None,
            ChildParserDeferredFollowupMode::ReturnReadyClassic => self
                .frame_parser_deferred_script_order
                .pending_head(owner.document_owner())
                .and_then(|head| {
                    self.take_child_parser_deferred_classic_work_if_ready(child_handle, owner, head)
                }),
        };
        let queued_document_script_ready = scheduler_work.is_none()
            && self.queue_next_child_parser_deferred_script_if_ready(child_handle, owner);
        let domcontentloaded_queued = scheduler_work.is_none()
            && !queued_document_script_ready
            && self.queue_child_document_domcontentloaded_if_ready(child_handle, owner);
        FrameDocumentClassicDeferredCompletionApplication::new(
            scheduler_work,
            queued_document_script_ready,
            domcontentloaded_queued,
        )
        .with_order_slot_released()
    }

    pub(crate) fn prepare_child_classic_script_execution(
        &mut self,
        ready: FrameDocumentClassicReadyWork,
    ) -> FrameDocumentClassicPrepareApplication {
        let target = *ready.target();
        let owner = target.task_owner().document_owner();
        let script_handle = ready.script_handle();
        let script_url = ready.script_url().clone();
        let child_handle = target.child_handle();
        let owner_current = self
            .frame_owner_store
            .current_child_document_owner(child_handle)
            .is_some_and(|current| current == owner);
        let runner_owner_current = self.frame_parser_classic_scripts.has_runner(owner);
        if !runner_owner_current {
            tracing::debug!(
                child_handle = ?child_handle,
                owner = ?owner,
                script_handle = ?script_handle,
                "dropping child classic ready work because runner owner is stale"
            );
            tracing::warn!(
                child_handle = ?child_handle,
                script_handle = ?script_handle,
                url = %script_url,
                "child classic script no longer has a current execution owner"
            );
            return FrameDocumentClassicPrepareApplication::dropped(
                FrameDocumentClassicPrepareDropReason::StaleRunnerOwner,
            );
        }
        if self.dom_host().owner_document_handle(script_handle)
            != Some(target.original_owner_document_handle())
        {
            let disposed = self.frame_parser_classic_scripts.dispose_ready_script(
                owner,
                child_handle,
                target.task_owner(),
                target.realm_id(),
                target.scheduling(),
                target.pending_script_key(),
                script_handle,
                owner_current,
            );
            if let Some(completion) = disposed {
                tracing::debug!(
                    child_handle = ?child_handle,
                    script_handle = ?script_handle,
                    url = %script_url,
                    "child classic script element no longer belongs to its original owner document"
                );
                return FrameDocumentClassicPrepareApplication::started(
                    FrameClassicDocumentScriptExecutionStart::Complete(Box::new(completion)),
                );
            }
            tracing::warn!(
                child_handle = ?child_handle,
                script_handle = ?script_handle,
                url = %script_url,
                "child classic script no longer has a current execution owner"
            );
            return FrameDocumentClassicPrepareApplication::dropped(
                FrameDocumentClassicPrepareDropReason::MovedFromOriginalDocumentWithoutCompletion,
            );
        }
        if !owner_current {
            let began = self
                .frame_parser_classic_scripts
                .begin_ready_execution(
                    owner,
                    child_handle,
                    target.task_owner(),
                    target.realm_id(),
                    target.scheduling(),
                    target.pending_script_key(),
                    script_handle,
                    false,
                )
                .is_some();
            debug_assert!(
                !began,
                "stale child parser script owner must not enter execution"
            );
            tracing::warn!(
                child_handle = ?child_handle,
                script_handle = ?script_handle,
                url = %script_url,
                "child classic script no longer has a current execution owner"
            );
            return FrameDocumentClassicPrepareApplication::dropped(
                FrameDocumentClassicPrepareDropReason::StaleDocumentOwner,
            );
        }
        let parser_resume_permit = (target.scheduling()
            == FrameDocumentClassicScriptScheduling::ParserBlocking)
            .then(|| {
                self.child_document_parsers
                    .parser_script_resume_permit(owner, script_handle)
            })
            .flatten();
        if target.scheduling() == FrameDocumentClassicScriptScheduling::ParserBlocking
            && parser_resume_permit.is_none()
        {
            return FrameDocumentClassicPrepareApplication::dropped(
                FrameDocumentClassicPrepareDropReason::StaleParserSuspension,
            );
        }
        let Some(execution_entry) = self.frame_parser_classic_scripts.begin_ready_execution(
            owner,
            child_handle,
            target.task_owner(),
            target.realm_id(),
            target.scheduling(),
            target.pending_script_key(),
            script_handle,
            owner_current,
        ) else {
            tracing::warn!(
                child_handle = ?child_handle,
                script_handle = ?script_handle,
                url = %script_url,
                "child classic script no longer has a current execution owner"
            );
            return FrameDocumentClassicPrepareApplication::dropped(
                FrameDocumentClassicPrepareDropReason::BeginExecutionUnavailable,
            );
        };
        if let Some(permit) = parser_resume_permit
            && self
                .child_document_parsers
                .resume_parser_script_for_execution(owner, permit)
                != Some(true)
        {
            tracing::debug!(
                child_handle = ?child_handle,
                owner = ?owner,
                script_handle = ?script_handle,
                ?permit,
                "dropping child parser script with a stale parser suspension permit"
            );
            self.child_document_parsers.clear(owner);
            return FrameDocumentClassicPrepareApplication::dropped(
                FrameDocumentClassicPrepareDropReason::StaleParserSuspension,
            );
        }
        let (target, execution, executable) = execution_entry.into_parts();
        let Some(action) = self.child_classic_script_execution_action(
            target,
            execution,
            executable.into_prepared_script(),
        ) else {
            tracing::warn!(
                child_handle = ?child_handle,
                script_handle = ?script_handle,
                url = %script_url,
                "child classic script no longer has a current execution owner"
            );
            return FrameDocumentClassicPrepareApplication::dropped(
                FrameDocumentClassicPrepareDropReason::ExecutionActionUnavailable,
            );
        };
        FrameDocumentClassicPrepareApplication::started(
            FrameClassicDocumentScriptExecutionStart::Execute(Box::new(action)),
        )
    }

    fn child_classic_script_execution_action(
        &self,
        target: FrameDocumentClassicScriptTarget,
        execution: ParserPendingClassicScriptExecution,
        script: PreparedScript,
    ) -> Option<FrameClassicDocumentScriptExecutionAction> {
        let Some(realm_id) = target.realm_id() else {
            tracing::warn!(
                child_handle = ?target.child_handle(),
                owner = ?target.owner(),
                script_handle = ?execution.metadata.script_handle(),
                "child classic execution target has no materialized FrameRealm"
            );
            return None;
        };
        let kind = frame_script_job_kind_from_parser_classic_ready_kind(execution.ready_kind);
        let script_handle = execution.metadata.script_handle();
        let PreparedScript {
            url: script_url,
            base_url: script_base_url,
            fetch_metadata,
            source,
            ..
        } = script;
        let source = match source {
            ScriptSource::Inline(source) | ScriptSource::Loaded(source) => source,
            ScriptSource::LoadedBinary { source, .. } => source,
            ScriptSource::External => {
                unreachable!("frame classic execution action must have materialized source")
            }
        };
        let script_nonce = fetch_metadata.nonce;
        let script_integrity = fetch_metadata.integrity;
        let mut job = self
            .frame_owner_store
            .child_prepared_classic_script_job_for_owner(
                target.child_handle(),
                target.owner().local_window_id,
                target.owner().document_id,
                kind,
                Some(script_handle),
                script_url.clone(),
                script_base_url.clone(),
                script_nonce,
                source,
            )?;
        job.script_integrity = script_integrity;
        let finish = FrameDocumentClassicScriptExecutionFinish {
            child_handle: target.child_handle(),
            owner: target.owner(),
            task_owner: target.task_owner(),
            realm_id,
            script_handle,
            script_url,
            script_base_url,
            scheduling: target.scheduling(),
            pending_script_key: target.pending_script_key(),
            load_delay_token: target.load_delay_token(),
        };
        Some(FrameClassicDocumentScriptExecutionAction::new(job, finish))
    }

    pub(crate) fn finish_executing_child_classic_script(
        &mut self,
        finish: FrameDocumentClassicScriptExecutionFinish,
    ) -> Option<FrameDocumentClassicScriptCompletionAction> {
        let expected_realm_id = finish.realm_id;
        let current_realm_id = self
            .frame_owner_current_child_snapshot(finish.child_handle)
            .and_then(|snapshot| snapshot.realm_id);
        if current_realm_id != Some(expected_realm_id) {
            tracing::debug!(
                child_handle = ?finish.child_handle,
                owner = ?finish.owner,
                expected_realm_id = ?expected_realm_id,
                current_realm_id = ?current_realm_id,
                "dropping child classic execution finish for stale FrameRealm"
            );
            self.cancel_child_deferred_classic_execution_finish(&finish);
            return None;
        }
        let owner_current = self
            .frame_owner_store
            .current_child_document_owner(finish.child_handle)
            .is_some_and(|current| current == finish.owner);
        let completion = self.frame_parser_classic_scripts.finish_executing(
            finish.owner,
            finish.child_handle,
            finish.task_owner,
            Some(finish.realm_id),
            finish.scheduling,
            finish.pending_script_key,
            finish.script_handle,
            owner_current,
        );
        if completion.is_none() {
            self.cancel_child_deferred_classic_execution_finish(&finish);
        }
        completion
    }

    fn cancel_child_deferred_classic_execution_finish(
        &mut self,
        finish: &FrameDocumentClassicScriptExecutionFinish,
    ) -> FrameDocumentClassicDeferredCompletionApplication {
        if finish.scheduling != FrameDocumentClassicScriptScheduling::Deferred
            || !self
                .frame_owner_store
                .child_document_task_owner_is_current(finish.child_handle, finish.task_owner)
        {
            return FrameDocumentClassicDeferredCompletionApplication::new(None, false, false);
        }
        let Some(key) = finish.pending_script_key else {
            return FrameDocumentClassicDeferredCompletionApplication::new(None, false, false);
        };
        if key.script_node_id() != finish.script_handle
            || !self
                .frame_parser_classic_scripts
                .discard_current_deferred_script_if_key(finish.owner, key)
        {
            return FrameDocumentClassicDeferredCompletionApplication::new(None, false, false);
        }
        self.complete_child_parser_deferred_order_entry(
            finish.child_handle,
            finish.task_owner,
            FrameParserDeferredScriptOrderEntry::classic(key),
            ChildParserDeferredDelayAction::Release(
                finish
                    .load_delay_token
                    .expect("accepted classic defer must retain its lifecycle delay token"),
            ),
            ChildParserDeferredFollowupMode::Queue,
        )
    }

    pub(crate) fn report_child_classic_script_source_failure(
        &mut self,
        failed: FrameDocumentClassicSourceFailureWork,
    ) -> FrameDocumentClassicSourceFailureReportApplication {
        let (target, _failure, script_element_event) = failed.into_parts();
        let child_handle = target.child_handle();
        let owner = target.task_owner().document_owner();
        let runner_owner_current = self.frame_parser_classic_scripts.has_runner(owner);
        if !runner_owner_current {
            tracing::debug!(
                child_handle = ?child_handle,
                owner = ?owner,
                "dropping child classic source failure because runner owner is stale"
            );
            return FrameDocumentClassicSourceFailureReportApplication::skipped(
                FrameDocumentClassicSourceFailureReportSkipReason::StaleRunnerOwner,
            );
        }
        let Some(expected_realm_id) = target.realm_id() else {
            tracing::debug!(
                child_handle = ?child_handle,
                owner = ?owner,
                "dropping child classic source failure because it has no materialized FrameRealm"
            );
            return FrameDocumentClassicSourceFailureReportApplication::skipped(
                FrameDocumentClassicSourceFailureReportSkipReason::MissingCurrentRealm,
            );
        };
        let current_realm_id = self
            .frame_owner_current_child_snapshot(child_handle)
            .and_then(|snapshot| snapshot.realm_id);
        if current_realm_id != Some(expected_realm_id) {
            tracing::debug!(
                child_handle = ?child_handle,
                owner = ?owner,
                expected_realm_id = ?expected_realm_id,
                current_realm_id = ?current_realm_id,
                "dropping child classic source failure because FrameRealm is stale"
            );
            return FrameDocumentClassicSourceFailureReportApplication::skipped(
                FrameDocumentClassicSourceFailureReportSkipReason::StaleRealm,
            );
        }
        FrameDocumentClassicSourceFailureReportApplication::completed(
            FrameDocumentClassicScriptCompletionAction::new(
                FrameDocumentClassicScriptCompletionTarget::new(
                    child_handle,
                    target.task_owner(),
                    expected_realm_id,
                )
                .with_scheduling(target.scheduling())
                .with_pending_script_key(target.pending_script_key())
                .with_load_delay_token(target.load_delay_token()),
                script_element_event,
            ),
        )
    }

    pub(crate) fn resume_child_classic_parser_after_completion(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        action: FrameDocumentClassicParserResumeCompletionAction,
    ) -> FrameDocumentClassicParserResumeApplication {
        let target = action.into_target();
        let child_handle = target.child_handle();
        let owner = target.owner();
        let expected_realm_id = target.realm_id();
        let current_realm_id = self
            .frame_owner_current_child_snapshot(child_handle)
            .and_then(|snapshot| snapshot.realm_id);
        if current_realm_id != Some(expected_realm_id) {
            self.child_document_parsers.clear(owner);
            return FrameDocumentClassicParserResumeApplication::skipped(
                FrameDocumentClassicParserResumeSkipReason::StaleRealm,
            );
        }
        self.resume_live_child_document_parser_after_blocker(scope, child_handle, owner)
    }
}

#[cfg(test)]
mod tests {
    use crate::frame_owner_model::{
        FrameClassicDocumentScriptExecutionStart, FrameDocumentClassicParserResumeApplication,
        FrameDocumentClassicParserResumeSkipReason, FrameDocumentClassicPrepareApplication,
        FrameDocumentClassicPrepareDropReason, FrameDocumentClassicSourceFailureReportApplication,
        FrameDocumentClassicSourceFailureReportSkipReason,
    };

    #[test]
    fn child_classic_parser_resume_application_tracks_lifecycle_followup() {
        let skipped = FrameDocumentClassicParserResumeApplication::skipped(
            FrameDocumentClassicParserResumeSkipReason::StaleDocumentOwner,
        );
        assert!(!skipped.parser_was_resumed());
        assert_eq!(
            skipped.skip_reason(),
            Some(FrameDocumentClassicParserResumeSkipReason::StaleDocumentOwner)
        );
        assert!(skipped.into_scheduler_work().is_none());

        let stale_realm = FrameDocumentClassicParserResumeApplication::skipped(
            FrameDocumentClassicParserResumeSkipReason::StaleRealm,
        );
        assert!(!stale_realm.parser_was_resumed());
        assert_eq!(
            stale_realm.skip_reason(),
            Some(FrameDocumentClassicParserResumeSkipReason::StaleRealm)
        );
        assert!(stale_realm.into_scheduler_work().is_none());

        let resumed = FrameDocumentClassicParserResumeApplication::resumed(None);
        assert!(resumed.parser_was_resumed());
        assert_eq!(resumed.skip_reason(), None);
        assert!(resumed.into_scheduler_work().is_none());
    }

    #[test]
    fn child_classic_prepare_application_tracks_drop_reason() {
        let dropped = FrameDocumentClassicPrepareApplication::dropped(
            FrameDocumentClassicPrepareDropReason::StaleRunnerOwner,
        );

        assert_eq!(
            dropped.drop_reason(),
            Some(FrameDocumentClassicPrepareDropReason::StaleRunnerOwner)
        );
        assert!(matches!(
            dropped.into_start(),
            FrameClassicDocumentScriptExecutionStart::Dropped
        ));
    }

    #[test]
    fn child_classic_source_failure_report_application_tracks_stale_runner_skip() {
        let skipped = FrameDocumentClassicSourceFailureReportApplication::skipped(
            FrameDocumentClassicSourceFailureReportSkipReason::StaleRunnerOwner,
        );

        assert_eq!(
            skipped.skip_reason(),
            Some(FrameDocumentClassicSourceFailureReportSkipReason::StaleRunnerOwner)
        );
        assert!(skipped.into_completion().is_none());
    }
}
