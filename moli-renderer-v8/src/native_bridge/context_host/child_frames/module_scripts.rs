use super::JsContextHost;
use crate::{
    detached_event_target::dispatch_detached_simple_event,
    document_runtime::DomHandle,
    document_script_scheduler::ParserPendingScriptId,
    frame_owner_model::{
        ChildDocumentModuleFetchTarget, FrameDocumentModuleClientReservation,
        FrameDocumentModuleDependencyFetchTask, FrameDocumentModuleFetchClientStart,
        FrameDocumentModuleFetchDisposition, FrameDocumentModulepreloadFetchTask,
        FrameDocumentModulepreloadMaterializedWork, FrameDocumentModulepreloadTerminalWork,
        FrameDocumentModulepreloadWorkAwaitingRealm, FrameDocumentParserModuleRootFetchStart,
        FrameDocumentParserModuleRootStartTask, FrameDocumentTaskOwner, FrameRealmId,
        FrameRequestKind,
    },
    module_runtime::{
        ModuleFetchMetadata, ModuleGraphFetchedSource, ModuleMapKey, NativeModuleGraphFetchRequest,
        NativeModuleSingleFetchRequest,
    },
    page_task_queue::RendererPageChildParserModuleRootStartTarget,
    planning::PreparedScript,
    types::{
        ChildModuleDependencyFetchCompletion, ChildModuleFetchNetworkAttribution,
        ChildParserModuleRootFetchCompletion, ScriptMode,
    },
};

impl JsContextHost {
    pub(crate) fn register_current_child_document_import_map(
        &mut self,
        child_handle: DomHandle,
        document_handle: DomHandle,
        source: &str,
        base_url: &url::Url,
    ) -> Result<(), String> {
        self.frame_owner_store
            .register_current_child_document_import_map(
                child_handle,
                document_handle,
                source,
                base_url,
            )
    }

    pub(crate) fn resolve_frame_document_module_specifier(
        &mut self,
        owner: crate::frame_owner_model::FrameDocumentOwner,
        realm_id: FrameRealmId,
        specifier: &str,
        base_url: &url::Url,
    ) -> Result<url::Url, String> {
        self.frame_owner_store
            .resolve_frame_document_module_specifier(owner, realm_id, specifier, base_url)
    }

    pub(crate) fn resolve_frame_document_module_integrity(
        &self,
        owner: crate::frame_owner_model::FrameDocumentOwner,
        realm_id: FrameRealmId,
        url: &url::Url,
    ) -> Option<String> {
        self.frame_owner_store
            .resolve_frame_document_module_integrity(owner, realm_id, url)
    }

    pub(crate) fn current_child_document_module_fetch_target(
        &self,
        child_handle: DomHandle,
    ) -> Option<ChildDocumentModuleFetchTarget> {
        let snapshot = self.frame_owner_current_child_snapshot(child_handle)?;
        Some(ChildDocumentModuleFetchTarget::new(
            child_handle,
            FrameDocumentTaskOwner::new(
                snapshot.scheduler_lane_id,
                snapshot.local_window_id,
                snapshot.document_id,
            ),
            snapshot.realm_id?,
        ))
    }

    pub(crate) fn cancel_child_document_script_work(&mut self, handle: DomHandle) {
        if let Some(owner) = self
            .frame_owner_store
            .current_child_document_task_owner(handle)
        {
            self.cancel_child_document_script_work_for_owner(handle, owner);
        } else {
            let _ = self.retire_child_document_script_ready_tasks_for_handle(handle);
            self.pending_child_modulepreload_work_awaiting_realm
                .retain(|task| task.child_handle() != handle);
            self.cancel_child_classic_document_script_work(handle);
        }
    }

    #[cfg(test)]
    pub(crate) fn cancel_child_document_script_work_if_current(
        &mut self,
        handle: DomHandle,
        owner: FrameDocumentTaskOwner,
    ) -> bool {
        if !self
            .frame_owner_store
            .child_document_task_owner_is_current(handle, owner)
        {
            return false;
        }
        self.cancel_child_document_script_work_for_owner(handle, owner);
        true
    }

    pub(in crate::native_bridge::context_host) fn cancel_child_document_script_work_for_owner(
        &mut self,
        handle: DomHandle,
        owner: FrameDocumentTaskOwner,
    ) {
        let document_owner = owner.document_owner();
        let _ = self
            .frame_owner_store
            .release_all_document_script_load_delays(owner);
        self.frame_parser_deferred_script_order
            .remove_document(document_owner);
        self.child_document_script_schedulers
            .remove_document(document_owner);
        let _ = self.retire_child_document_script_ready_tasks_for_owner(owner);
        let _ = self.frame_parser_classic_scripts.remove(document_owner);
        self.child_document_parsers.clear(document_owner);
        self.pending_child_modulepreload_work_awaiting_realm
            .retain(|task| task.child_handle() != handle || task.owner() != owner);
        self.cancel_child_classic_document_script_work(handle);
    }

    /// Parser and runtime insertions share the module graph pipeline. Only
    /// ModuleDefer reserves parser order and delays DOMContentLoaded.
    pub(in crate::native_bridge::context_host) fn queue_child_module_script_for_current_document(
        &mut self,
        handle: DomHandle,
        script_handle: DomHandle,
        blocking_stylesheet_signatures: std::collections::HashSet<
            crate::stylesheet_blocking::DocumentBlockingStylesheetSignature,
        >,
        script: PreparedScript,
    ) -> bool {
        if !self.child_browsing_contexts.contains_key(&handle) {
            return false;
        }
        let Some(owner) = self
            .frame_owner_store
            .current_child_document_task_owner(handle)
        else {
            return false;
        };
        let parser_ordered = script.mode == ScriptMode::ModuleDefer;
        let pending_script_id = ParserPendingScriptId::from_key(
            owner.document_owner(),
            crate::document_script_scheduler::ParserPendingScriptKey::from_script(&script),
        );
        if self
            .child_document_script_schedulers
            .has_module_script(pending_script_id)
        {
            tracing::debug!(
                owner = ?owner,
                parser_position = pending_script_id.parser_position(),
                script_node_id = ?pending_script_id.script_node_id(),
                script_url = %script.url,
                "joined duplicate child parser module handoff without another lifecycle delay"
            );
            return true;
        }
        let load_delay_token = if parser_ordered {
            self.frame_owner_store
                .acquire_current_child_parser_deferred_script_load_delay(handle, owner)
                .map(Into::into)
        } else {
            self.frame_owner_store
                .acquire_current_child_async_module_script_load_delay(handle, owner)
        };
        let Some(load_delay_token) = load_delay_token else {
            return false;
        };
        let (registered_pending_script_id, accepted, queued_ready_work) = if parser_ordered {
            match self
                .child_document_script_schedulers
                .accept_parser_ordered_module_script(
                    owner.document_owner(),
                    &script,
                    blocking_stylesheet_signatures,
                ) {
                Some(pending_script_id) => (pending_script_id, true, false),
                None => (pending_script_id, false, false),
            }
        } else {
            let watch = self
                .child_document_script_schedulers
                .register_and_watch_module_script(owner.document_owner(), &script);
            (
                watch.pending_script_id(),
                watch.watched(),
                watch.queued_ready_work(),
            )
        };
        debug_assert_eq!(registered_pending_script_id, pending_script_id);
        if !accepted {
            let _ = self
                .child_document_script_schedulers
                .discard_module_script(pending_script_id);
            let _ = self.release_child_module_script_load_delay(
                owner,
                load_delay_token,
                parser_ordered,
            );
            tracing::warn!(
                owner = ?owner,
                parser_position = pending_script_id.parser_position(),
                script_node_id = ?pending_script_id.script_node_id(),
                script_url = %script.url,
                "child parser module PendingScript could not enter its selected owner state"
            );
            return false;
        }
        let order_registered = parser_ordered
            && self.frame_parser_deferred_script_order.register(
                owner.document_owner(),
                crate::document_script_scheduler::FrameParserDeferredScriptOrderEntry::module(
                    pending_script_id,
                ),
            );
        if parser_ordered && !order_registered {
            let _ = self
                .child_document_script_schedulers
                .discard_module_script(pending_script_id);
            let _ = self.release_child_module_script_load_delay(
                owner,
                load_delay_token,
                parser_ordered,
            );
            tracing::warn!(
                owner = ?owner,
                parser_position = pending_script_id.parser_position(),
                script_node_id = ?pending_script_id.script_node_id(),
                script_url = %script.url,
                "rejecting child module-defer without an exact parser-order slot"
            );
            return false;
        }
        if queued_ready_work {
            self.admit_runnable_child_document_script_tasks();
        }
        tracing::debug!(
            owner = ?owner,
            script_node_id = ?script.node_id,
            script_url = %script.url,
            parser_position = pending_script_id.parser_position(),
            accepted,
            parser_ordered,
            ?load_delay_token,
            order_registered,
            "child parser module PendingScript registered at parser handoff"
        );
        let root_start = FrameDocumentParserModuleRootStartTask::from_parser_script_parts(
            handle,
            owner,
            pending_script_id,
            script_handle,
            script,
            load_delay_token,
        );
        let Some(realm_request) =
            self.request_child_frame_realm_materialization_for_owner(handle, owner)
        else {
            self.rollback_child_parser_module_root_start(&root_start);
            tracing::error!(
                owner = ?owner,
                parser_position = pending_script_id.parser_position(),
                script_node_id = ?pending_script_id.script_node_id(),
                "rolled back child parser module acceptance after realm admission failed"
            );
            return false;
        };
        let target = RendererPageChildParserModuleRootStartTarget::new(
            handle,
            owner,
            realm_request.realm_id(),
        );
        if let Err(error) = self
            .page_child_frame_task_sender()
            .send_parser_module_root_start(target, root_start)
        {
            let root_start = error.into_task();
            self.rollback_child_parser_module_root_start(&root_start);
            tracing::error!(
                owner = ?owner,
                parser_position = pending_script_id.parser_position(),
                script_node_id = ?pending_script_id.script_node_id(),
                "rolled back child parser module acceptance after Page route closure"
            );
            return false;
        }
        true
    }

    fn rollback_child_parser_module_root_start(
        &mut self,
        task: &FrameDocumentParserModuleRootStartTask,
    ) {
        let owner = task.owner();
        let pending_script_id = task.pending_script_id();
        let parser_ordered = task.script().mode == ScriptMode::ModuleDefer;
        if parser_ordered {
            let _ = self.frame_parser_deferred_script_order.remove_pending(
                owner.document_owner(),
                crate::document_script_scheduler::FrameParserDeferredScriptOrderEntry::module(
                    pending_script_id,
                ),
            );
        }
        let _ = self
            .child_document_script_schedulers
            .discard_module_script(pending_script_id);
        let _ = self.release_child_module_script_load_delay(
            owner,
            task.client().load_delay_token(),
            parser_ordered,
        );
    }

    pub(crate) fn release_child_module_script_load_delay(
        &mut self,
        owner: FrameDocumentTaskOwner,
        load_delay: crate::frame_owner_model::ChildDocumentModuleScriptLoadDelay,
        parser_ordered: bool,
    ) -> bool {
        let Some(load_delay_token) = load_delay.token() else {
            return false;
        };
        if parser_ordered {
            self.frame_owner_store
                .release_parser_deferred_script_load_delay(owner, load_delay_token)
        } else {
            self.frame_owner_store
                .release_async_module_script_load_delay(owner, load_delay_token)
        }
    }

    pub(crate) fn current_child_parser_module_root_start_target(
        &self,
        target: RendererPageChildParserModuleRootStartTarget,
    ) -> Option<RendererPageChildParserModuleRootStartTarget> {
        if !self
            .frame_owner_store
            .child_document_task_owner_is_current(target.child_handle(), target.document_owner())
        {
            return None;
        }
        let realm_id = self
            .frame_owner_store
            .current_materialized_realm_id_for_document_task_owner(target.document_owner())?;
        if realm_id != target.realm_id() {
            return None;
        }
        Some(target)
    }

    pub(crate) fn settle_stale_child_parser_module_root_start(
        &mut self,
        task: &FrameDocumentParserModuleRootStartTask,
    ) {
        self.rollback_child_parser_module_root_start(task);
    }

    pub(crate) fn begin_current_child_module_root_fetch_client(
        &mut self,
        reservation: FrameDocumentModuleClientReservation,
    ) -> Option<FrameDocumentModuleFetchClientStart> {
        self.frame_owner_store
            .begin_reserved_current_child_module_fetch_client(
                reservation,
                FrameRequestKind::ModuleRoot,
            )
    }

    pub(crate) fn settle_current_child_dynamic_import_owner_module_fetch(
        &mut self,
        start: &FrameDocumentModuleFetchClientStart,
        _result: Result<ModuleGraphFetchedSource, String>,
    ) -> bool {
        self.frame_owner_store
            .settle_current_document_module_fetch_request(
                start.owner(),
                start.request_id(),
                start.request_kind(),
            )
    }

    pub(in crate::native_bridge::context_host) fn queue_child_modulepreload_link_error_for_current_document(
        &mut self,
        handle: DomHandle,
        link_handle: DomHandle,
    ) -> bool {
        if !self.child_browsing_contexts.contains_key(&handle) {
            return false;
        }
        let Some(owner) = self
            .frame_owner_store
            .current_child_document_task_owner(handle)
        else {
            return false;
        };
        let Some(client) = self
            .frame_owner_store
            .accept_current_child_modulepreload_link(handle, owner, link_handle)
        else {
            return false;
        };

        let expected_realm_id = self
            .frame_owner_store
            .current_reserved_realm_id_for_document_task_owner(owner);
        if self
            .frame_owner_store
            .current_materialized_realm_id_for_document_task_owner(owner)
            .is_none()
        {
            return self.queue_child_modulepreload_work_awaiting_realm(
                FrameDocumentModulepreloadWorkAwaitingRealm::link_error(expected_realm_id, client),
            );
        }
        let Some(realm_id) = expected_realm_id else {
            tracing::warn!(
                ?handle,
                ?owner,
                ?link_handle,
                "child modulepreload error found a materialized context without an exact realm"
            );
            return false;
        };
        self.route_child_modulepreload_link_error(
            FrameDocumentModulepreloadTerminalWork::from_link_error_parts(realm_id, client),
        )
    }

    pub(in crate::native_bridge::context_host) fn queue_child_modulepreload_fetch_for_current_document(
        &mut self,
        handle: DomHandle,
        link_handle: DomHandle,
        request: NativeModuleSingleFetchRequest,
    ) -> bool {
        if !self.child_browsing_contexts.contains_key(&handle) {
            return false;
        }
        let Some(owner) = self
            .frame_owner_store
            .current_child_document_task_owner(handle)
        else {
            return false;
        };
        let Some(client) = self
            .frame_owner_store
            .accept_current_child_modulepreload_link(handle, owner, link_handle)
        else {
            return false;
        };

        let expected_realm_id = self
            .frame_owner_store
            .current_reserved_realm_id_for_document_task_owner(owner);
        if self
            .frame_owner_store
            .current_materialized_realm_id_for_document_task_owner(owner)
            .is_none()
        {
            return self.queue_child_modulepreload_work_awaiting_realm(
                FrameDocumentModulepreloadWorkAwaitingRealm::fetch_start(
                    expected_realm_id,
                    client,
                    request,
                ),
            );
        }

        let Some(realm_id) = expected_realm_id else {
            tracing::warn!(
                ?handle,
                ?owner,
                ?link_handle,
                "child modulepreload found a materialized context without an exact realm"
            );
            return false;
        };
        self.route_child_modulepreload_start(
            FrameDocumentModulepreloadFetchTask::from_modulepreload_fetch_parts(
                realm_id, client, request,
            ),
        )
    }

    fn queue_child_modulepreload_work_awaiting_realm(
        &mut self,
        work: FrameDocumentModulepreloadWorkAwaitingRealm,
    ) -> bool {
        let handle = work.child_handle();
        let owner = work.owner();
        let previous_len = self.pending_child_modulepreload_work_awaiting_realm.len();
        self.pending_child_modulepreload_work_awaiting_realm
            .push_back(work);
        if self
            .request_child_frame_realm_materialization_for_owner(handle, owner)
            .is_some_and(|request| {
                matches!(
                    request,
                    crate::frame_owner_model::FrameRealmMaterializationRequest::NewlyQueued { .. }
                        | crate::frame_owner_model::FrameRealmMaterializationRequest::AlreadyQueued { .. }
                )
            })
        {
            return true;
        }
        self.pending_child_modulepreload_work_awaiting_realm
            .truncate(previous_len);
        false
    }

    pub(in crate::native_bridge::context_host) fn bind_pending_child_modulepreload_work_to_first_realm(
        &mut self,
        handle: DomHandle,
        owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
    ) {
        for work in &mut self.pending_child_modulepreload_work_awaiting_realm {
            if work.child_handle() == handle && work.owner() == owner {
                work.bind_first_established_realm(realm_id);
            }
        }
    }

    pub(crate) fn promote_child_modulepreload_work_after_realm_materialization(
        &mut self,
        handle: DomHandle,
        owner: FrameDocumentTaskOwner,
    ) -> usize {
        let current_realm_id = self
            .frame_owner_store
            .current_child_document_task_owner_materialized_realm(handle)
            .filter(|(current_owner, _)| *current_owner == owner)
            .map(|(_, realm_id)| realm_id);
        let mut retained = std::collections::VecDeque::new();
        let mut ready = Vec::new();
        for work in std::mem::take(&mut self.pending_child_modulepreload_work_awaiting_realm) {
            if work.child_handle() == handle && work.owner() == owner {
                ready.push(work);
            } else {
                retained.push_back(work);
            }
        }
        self.pending_child_modulepreload_work_awaiting_realm = retained;

        let Some(realm_id) = current_realm_id else {
            if !ready.is_empty() {
                tracing::debug!(
                    ?handle,
                    ?owner,
                    discarded = ready.len(),
                    "discarded modulepreload work after its child Document was replaced during realm materialization"
                );
            }
            return 0;
        };

        let mut promoted = 0;
        for work in ready {
            let expected_realm_id = work.expected_realm_id();
            let link_handle = work.link_handle();
            let request_url = work
                .request()
                .map(|request| request.module_key().url().clone());
            let Some(work) = work.into_materialized_work(realm_id) else {
                tracing::debug!(
                    ?handle,
                    ?owner,
                    ?expected_realm_id,
                    ?realm_id,
                    ?link_handle,
                    ?request_url,
                    "discarded modulepreload work after its pre-existing realm was replaced"
                );
                continue;
            };
            let routed = match work {
                FrameDocumentModulepreloadMaterializedWork::FetchStart(task) => {
                    self.route_child_modulepreload_start(*task)
                }
                FrameDocumentModulepreloadMaterializedWork::LinkError(work) => {
                    self.route_child_modulepreload_link_error(work)
                }
            };
            promoted += usize::from(routed);
        }
        promoted
    }

    pub(crate) fn discard_child_modulepreload_work_awaiting_realm(
        &mut self,
        handle: DomHandle,
        owner: FrameDocumentTaskOwner,
    ) -> usize {
        let original_len = self.pending_child_modulepreload_work_awaiting_realm.len();
        self.pending_child_modulepreload_work_awaiting_realm
            .retain(|work| work.child_handle() != handle || work.owner() != owner);
        original_len - self.pending_child_modulepreload_work_awaiting_realm.len()
    }

    #[cfg(test)]
    pub(crate) fn pending_child_modulepreload_work_awaiting_realm_for_test(&self) -> usize {
        self.pending_child_modulepreload_work_awaiting_realm.len()
    }

    fn route_child_modulepreload_link_error(
        &mut self,
        work: FrameDocumentModulepreloadTerminalWork,
    ) -> bool {
        let Some(action) = self.accept_child_modulepreload_terminal_event(work) else {
            return false;
        };
        self.route_child_modulepreload_event_action(action)
    }

    fn route_child_modulepreload_start(
        &mut self,
        task: FrameDocumentModulepreloadFetchTask,
    ) -> bool {
        let target = task.target();
        let link_handle = task.link_handle();
        let sender = self.page_modulepreload_start_sender();
        match sender.send(task) {
            Ok(()) => true,
            Err(_) => {
                tracing::debug!(
                    ?target,
                    ?link_handle,
                    "discarded child modulepreload start after its stable Page route closed"
                );
                false
            }
        }
    }

    pub(crate) fn finish_child_owner_module_fetch_without_network(
        &mut self,
        start: &FrameDocumentModuleFetchClientStart,
    ) -> bool {
        self.frame_owner_store
            .finish_document_request(start.owner().document_id, start.request_id())
    }

    pub(crate) fn begin_current_child_module_dependency_fetch_client(
        &mut self,
        reservation: FrameDocumentModuleClientReservation,
    ) -> Option<FrameDocumentModuleFetchClientStart> {
        self.frame_owner_store
            .begin_reserved_current_child_module_fetch_client(
                reservation,
                FrameRequestKind::ModuleDependency,
            )
    }

    pub(crate) fn capture_child_module_fetch_producer_for_child(
        &self,
        child_handle: DomHandle,
        owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
        request_url: url::Url,
    ) -> Option<(
        ChildDocumentModuleFetchTarget,
        ChildModuleFetchNetworkAttribution,
    )> {
        let expected = ChildDocumentModuleFetchTarget::new(child_handle, owner, realm_id);
        let (current, network_attribution) =
            self.capture_current_child_module_fetch_producer(child_handle, request_url)?;
        (current == expected).then_some((current, network_attribution))
    }

    /// Captures executable identity and Network attribution from one current
    /// child snapshot. The Page arbiter may use the target for authorization;
    /// the attribution remains protocol metadata and never grants execution.
    pub(crate) fn capture_current_child_module_fetch_producer(
        &self,
        child_handle: DomHandle,
        request_url: url::Url,
    ) -> Option<(
        ChildDocumentModuleFetchTarget,
        ChildModuleFetchNetworkAttribution,
    )> {
        let snapshot = self.frame_owner_current_child_snapshot(child_handle)?;
        let target = ChildDocumentModuleFetchTarget::new(
            child_handle,
            FrameDocumentTaskOwner::new(
                snapshot.scheduler_lane_id,
                snapshot.local_window_id,
                snapshot.document_id,
            ),
            snapshot.realm_id?,
        );
        let network_attribution = ChildModuleFetchNetworkAttribution::parser(
            Some(snapshot.frame_id.0),
            snapshot.document_url,
            request_url,
        );
        Some((target, network_attribution))
    }

    pub(crate) fn capture_child_module_fetch_network_attribution(
        &self,
        target: ChildDocumentModuleFetchTarget,
        request_url: url::Url,
    ) -> Option<ChildModuleFetchNetworkAttribution> {
        let snapshot = self.frame_owner_current_child_snapshot(target.child_handle())?;
        let owner = target.task_owner();
        if snapshot.scheduler_lane_id != owner.scheduler_lane_id
            || snapshot.local_window_id != owner.local_window_id
            || snapshot.document_id != owner.document_id
            || snapshot.realm_id != Some(target.realm_id())
        {
            return None;
        }
        Some(ChildModuleFetchNetworkAttribution::parser(
            Some(snapshot.frame_id.0),
            snapshot.document_url,
            request_url,
        ))
    }

    pub(crate) fn current_child_module_fetch_target_for_realm(
        &self,
        owner: FrameDocumentTaskOwner,
        realm_id: FrameRealmId,
    ) -> Option<ChildDocumentModuleFetchTarget> {
        let snapshot = self.frame_owner_current_child_snapshot_for_realm(realm_id)?;
        if snapshot.scheduler_lane_id != owner.scheduler_lane_id
            || snapshot.local_window_id != owner.local_window_id
            || snapshot.document_id != owner.document_id
        {
            return None;
        }
        Some(ChildDocumentModuleFetchTarget::new(
            snapshot.owner_handle,
            owner,
            realm_id,
        ))
    }

    pub(crate) fn capture_child_dynamic_import_fetch_producer(
        &self,
        owner: crate::frame_owner_model::FrameDocumentOwner,
        realm_id: FrameRealmId,
        request_url: url::Url,
    ) -> Option<(
        ChildDocumentModuleFetchTarget,
        ChildModuleFetchNetworkAttribution,
    )> {
        let snapshot = self.frame_owner_current_child_snapshot_for_realm(realm_id)?;
        if snapshot.local_window_id != owner.local_window_id
            || snapshot.document_id != owner.document_id
        {
            return None;
        }
        let task_owner = FrameDocumentTaskOwner::new(
            snapshot.scheduler_lane_id,
            snapshot.local_window_id,
            snapshot.document_id,
        );
        Some((
            ChildDocumentModuleFetchTarget::new(snapshot.owner_handle, task_owner, realm_id),
            ChildModuleFetchNetworkAttribution::dynamic_import(
                Some(snapshot.frame_id.0),
                snapshot.document_url,
                request_url,
            ),
        ))
    }

    pub(crate) fn dispatch_child_modulepreload_link_handle_event<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        owner: FrameDocumentTaskOwner,
        link_handle: DomHandle,
        successful: bool,
    ) -> bool {
        let Some(document_handle) = self.dom_host().owner_document_handle(link_handle) else {
            return false;
        };
        let Some(child_handle) =
            self.child_browsing_context_host_for_document_handle(document_handle)
        else {
            return false;
        };
        let Some(snapshot) = self.frame_owner_current_child_snapshot(child_handle) else {
            return false;
        };
        if snapshot.scheduler_lane_id != owner.scheduler_lane_id
            || snapshot.local_window_id != owner.local_window_id
            || snapshot.document_id != owner.document_id
            || snapshot.document_handle != document_handle
        {
            return false;
        }

        let _ = self.child_browsing_context_document_wrapper(scope, child_handle);
        let host_ptr = self as *mut JsContextHost;
        let Some(target) = self
            .native_bridge_mut()
            .wrap_handle(scope, host_ptr, link_handle)
        else {
            return false;
        };
        let event_type = if successful { "load" } else { "error" };
        dispatch_detached_simple_event(scope, target, event_type, false, false, false)
    }

    pub(crate) fn start_child_module_dependency_fetch(
        &mut self,
        loader: &crate::network::context::DocumentResourceLoader,
        start: FrameDocumentModuleFetchClientStart,
        task: FrameDocumentModuleDependencyFetchTask,
        child_handle: DomHandle,
        network_attribution: ChildModuleFetchNetworkAttribution,
    ) {
        if !matches!(
            start.fetch_disposition(),
            FrameDocumentModuleFetchDisposition::StartedFetch(_)
        ) {
            let _ = self
                .frame_owner_store
                .finish_document_request(start.owner().document_id, start.request_id());
            return;
        }
        let request = task.fetch_request().clone();
        let owner = task.owner();
        debug_assert_eq!(owner.document_owner(), start.owner());
        let request_id = start.request_id();
        let completion_task = task.clone();
        let completion_network_attribution = network_attribution.clone();
        let completion_tx = self.resource_completion_tx.clone();
        tracing::debug!(
            owner = ?owner,
            realm_id = ?task.realm_id(),
            ?child_handle,
            url = %network_attribution.request_url(),
            "starting child module dependency fetch through owner module map"
        );
        let send_completion = move |result, network_result| {
            let _ = completion_tx.send_child_module_dependency_fetch(
                ChildModuleDependencyFetchCompletion::new(
                    child_handle,
                    request_id,
                    completion_task,
                    result,
                    network_result,
                    completion_network_attribution,
                ),
            );
        };
        if let Err(error) = request.fetch_source_for_document(loader, send_completion) {
            let message = error.to_string();
            let _ = self
                .resource_completion_tx
                .send_child_module_dependency_fetch(ChildModuleDependencyFetchCompletion::new(
                    child_handle,
                    request_id,
                    task,
                    Err(message.clone()),
                    Some(std::sync::Arc::new(Err(message))),
                    network_attribution,
                ));
        }
    }

    pub(crate) fn start_child_parser_module_root_fetch(
        &mut self,
        loader: &crate::network::context::DocumentResourceLoader,
        fetch_start: FrameDocumentParserModuleRootFetchStart,
        target: ChildDocumentModuleFetchTarget,
        network_attribution: ChildModuleFetchNetworkAttribution,
    ) {
        if !matches!(
            fetch_start.start.fetch_disposition(),
            FrameDocumentModuleFetchDisposition::StartedFetch(_)
        ) {
            return;
        }
        let request =
            child_parser_module_root_fetch_request(&fetch_start.script, fetch_start.start.key());
        let completion_tx = self.resource_completion_tx.clone();
        debug_assert_eq!(target.child_handle(), fetch_start.child_handle);
        let script_handle = fetch_start.script_handle;
        let realm_id = fetch_start.realm_id;
        let owner = fetch_start.owner;
        debug_assert_eq!(target.task_owner(), owner);
        debug_assert_eq!(target.realm_id(), realm_id);
        debug_assert_eq!(owner.document_owner(), fetch_start.start.owner());
        let request_id = fetch_start.start.request_id();
        let request_key = fetch_start.start.key().clone();
        let completion_request_key = request_key.clone();
        let completion_network_attribution = network_attribution.clone();
        tracing::debug!(
            child_handle = ?target.child_handle(),
            script_handle = ?script_handle,
            owner = ?owner,
            realm_id = ?realm_id,
            url = %network_attribution.request_url(),
            "starting child parser module root fetch through child document modulator reservation"
        );
        let send_completion = move |result, network_result| {
            let _ = completion_tx.send_child_parser_module_root_fetch(
                ChildParserModuleRootFetchCompletion::new(
                    target,
                    request_id,
                    completion_request_key,
                    result,
                    network_result,
                    completion_network_attribution,
                ),
            );
        };
        if let Err(error) = request.fetch_source_for_document(loader, send_completion) {
            let message = error.to_string();
            let _ = self
                .resource_completion_tx
                .send_child_parser_module_root_fetch(ChildParserModuleRootFetchCompletion::new(
                    target,
                    request_id,
                    request_key,
                    Err(message.clone()),
                    Some(std::sync::Arc::new(Err(message))),
                    network_attribution,
                ));
        }
    }

    /// Finishes request bookkeeping after the Page resource turn has already
    /// authorized the completion's exact child/document/realm target.
    ///
    /// This layer intentionally does not rediscover currentness or choose
    /// current versus historical protocol output.
    pub(crate) fn finish_child_parser_module_root_fetch_request(
        &mut self,
        completion: &ChildParserModuleRootFetchCompletion,
    ) -> bool {
        let owner = completion.target().task_owner();
        self.frame_owner_store.document_request_is_current(
            owner.document_id,
            completion.request_id(),
            FrameRequestKind::ModuleRoot,
        ) && self
            .frame_owner_store
            .finish_document_request(owner.document_id, completion.request_id())
    }

    pub(crate) fn finish_child_module_dependency_fetch_request(
        &mut self,
        completion: &ChildModuleDependencyFetchCompletion,
    ) -> bool {
        let owner = completion.target().task_owner();
        self.frame_owner_store.document_request_is_current(
            owner.document_id,
            completion.request_id(),
            FrameRequestKind::ModuleDependency,
        ) && self
            .frame_owner_store
            .finish_document_request(owner.document_id, completion.request_id())
    }

    pub(crate) fn record_current_child_module_fetch_network_result(
        &mut self,
        attribution: &ChildModuleFetchNetworkAttribution,
        network_result: &std::result::Result<crate::protocol_types::NavigationResponse, String>,
    ) {
        self.record_get_subresource_network_result_with_initiator(
            attribution.frame_id().map(str::to_owned),
            attribution.document_url().clone(),
            attribution.request_url().clone(),
            crate::types::SubresourceResourceType::Script,
            attribution.initiator_type(),
            network_result,
        );
    }

    pub(crate) fn record_historical_child_module_fetch_network_result(
        &mut self,
        attribution: &ChildModuleFetchNetworkAttribution,
        network_result: &std::result::Result<crate::protocol_types::NavigationResponse, String>,
    ) {
        self.record_historical_get_subresource_network_result_with_initiator(
            attribution.frame_id().map(str::to_owned),
            attribution.document_url().clone(),
            attribution.request_url().clone(),
            crate::types::SubresourceResourceType::Script,
            attribution.initiator_type(),
            network_result,
        );
    }
}

fn child_parser_module_root_fetch_request(
    script: &PreparedScript,
    key: &ModuleMapKey,
) -> NativeModuleGraphFetchRequest {
    NativeModuleGraphFetchRequest::new(
        key.url().clone(),
        script.initiator_url.clone(),
        ModuleFetchMetadata::from_top_level_script_fetch_metadata(&script.fetch_metadata),
        key.kind(),
    )
}
