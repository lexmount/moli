use super::*;

impl ServiceWorkerRuntimeService {
    pub(super) fn finish_client_message(&self, message: ServiceWorkerClientMessage) {
        let delivery = {
            let state = self.inner.state.lock();
            let Some(client) = state.live_clients.get(&message.target_client_id) else {
                return;
            };
            let Some(version) = state.versions.get(&message.source_version_id) else {
                return;
            };
            (
                client.window_completion_target(),
                client.endpoint.clone(),
                version.script_url.clone(),
                version.lifecycle_state.as_str(),
            )
        };
        let (target, endpoint, source_script_url, source_state) = delivery;
        endpoint.send_client_message(
            target,
            message.source_version_id,
            source_script_url,
            source_state,
            message.payload,
        );
    }

    pub(super) fn finish_worker_message(&self, message: ServiceWorkerWorkerMessage) {
        let start = {
            let mut state = self.inner.state.lock();
            let Some(source_version) = state.versions.get(&message.source_version_id) else {
                return;
            };
            if source_version.lifecycle_state == ServiceWorkerVersionLifecycleState::Redundant {
                return;
            }
            let source_registration_id = source_version.registration_id;
            let source_origin = moli_url::origin_ascii_serialization(&source_version.script_url);
            let Some(source_worker) = service_worker_version_snapshot(Some(source_version)) else {
                return;
            };
            let Some(target_version) = state.versions.get(&message.target_version_id) else {
                return;
            };
            let target_registration_id = target_version.registration_id;
            if source_registration_id != target_registration_id {
                return;
            }
            let Some(registration) = state.registrations.get(&target_registration_id) else {
                return;
            };
            if !registration.references_version(message.source_version_id)
                || !registration.references_version(message.target_version_id)
            {
                return;
            }
            let scope_url = registration.scope_url.clone();
            let registration_storage_key = registration.storage_key.clone();
            let Some(target_version) = state.versions.get_mut(&message.target_version_id) else {
                return;
            };
            if target_version.lifecycle_state == ServiceWorkerVersionLifecycleState::Redundant {
                return;
            }
            let event_id =
                ServiceWorkerEventId(self.inner.next_event_id.fetch_add(1, Ordering::Relaxed));
            let event = ServiceWorkerMessageEvent {
                event_id,
                owner: target_version.run_owner(),
                source_client_id: None,
                source_client_url: None,
                source_client_snapshot: None,
                source_worker: Some(source_worker),
                source_origin,
                payload: message.payload,
                window_interaction_allowed: false,
            };
            self.start_message_event_locked(
                &mut state,
                target_registration_id,
                scope_url,
                registration_storage_key,
                event,
            )
        };
        match start {
            ServiceWorkerMessageStart::Dispatch(dispatch) => {
                let (host, event) = *dispatch;
                self.dispatch_message_event(host, event);
            }
            ServiceWorkerMessageStart::Start(launch) => {
                self.start_queued_launch(*launch);
            }
            ServiceWorkerMessageStart::Queued | ServiceWorkerMessageStart::Dropped => {}
        }
    }

    pub(super) fn finish_client_query(
        &self,
        version_id: ServiceWorkerVersionId,
        run: RendererServiceWorkerRunIdentity,
        result: ServiceWorkerClientQueryResult,
    ) {
        let host = {
            let state = self.inner.state.lock();
            let Some(version) = state.versions.get(&version_id) else {
                return;
            };
            if version.run != run {
                return;
            }
            // Evaluation can query Clients before bootstrap completion marks
            // the version Running. Its host already has the worker handle.
            match &version.running_state {
                ServiceWorkerVersionRunningState::Starting { host }
                | ServiceWorkerVersionRunningState::Running { host } => Some(host.clone()),
                ServiceWorkerVersionRunningState::Stopped => None,
            }
        };
        if let Some(host) = host {
            host.dispatch_client_query_result(result);
        }
    }

    pub(super) fn finish_client_query_requested(
        &self,
        query: ServiceWorkerClientQuery,
        run: RendererServiceWorkerRunIdentity,
    ) {
        let version_id = query.version_id;
        let Some((result, windows)) = self.query_clients_with_window_targets(&query, &run) else {
            return;
        };
        if windows.is_empty() {
            self.finish_client_query(version_id, run, result);
            return;
        }
        let accumulator =
            crate::service_worker_runtime::client_query::ServiceWorkerClientQueryAccumulator::new(
                self.downgrade(),
                ServiceWorkerRunOwner::new(version_id, run),
                result,
                windows.len(),
            );
        for (target, sender) in windows {
            let reply = accumulator.reply_for(target.client_id);
            let _ = sender.send_service_worker_window_client_state_request(
                crate::types::ServiceWorkerWindowClientStateRequest { target, reply },
            );
        }
    }

    pub(super) fn finish_client_navigate_requested(
        &self,
        navigate: ServiceWorkerClientNavigate,
        run: RendererServiceWorkerRunIdentity,
    ) {
        let delivery = {
            let state = self.inner.state.lock();
            let Some(source_version) = state.versions.get(&navigate.source_version_id) else {
                return;
            };
            if source_version.run != run {
                return;
            }
            if source_version.lifecycle_state == ServiceWorkerVersionLifecycleState::Redundant {
                return;
            }
            let Some(source_registration) =
                state.registrations.get(&source_version.registration_id)
            else {
                return;
            };
            if source_registration.active_version_id != Some(navigate.source_version_id) {
                return;
            }
            let Some(target_client) = state.live_clients.get(&navigate.target_client_id) else {
                let result = ServiceWorkerClientNavigateResult {
                    request_id: navigate.request_id,
                    result: Err(ServiceWorkerClientNavigateError::type_error(
                        "The client was not found.",
                    )),
                };
                drop(state);
                self.finish_client_navigate(navigate.source_version_id, run, result);
                return;
            };
            if target_client.client_type != ServiceWorkerClientType::Window {
                let result = ServiceWorkerClientNavigateResult {
                    request_id: navigate.request_id,
                    result: Err(ServiceWorkerClientNavigateError::type_error(
                        "The client is not a window client.",
                    )),
                };
                drop(state);
                self.finish_client_navigate(navigate.source_version_id, run, result);
                return;
            }
            if !service_worker_scope_matches_url(
                &source_registration.scope_url,
                &target_client.document_url,
            ) {
                let result = ServiceWorkerClientNavigateResult {
                    request_id: navigate.request_id,
                    result: Err(ServiceWorkerClientNavigateError::type_error(
                        "The client is outside the service worker scope.",
                    )),
                };
                drop(state);
                self.finish_client_navigate(navigate.source_version_id, run, result);
                return;
            }
            if !source_registration
                .controlled_client_ids
                .contains(&navigate.target_client_id)
            {
                let result = ServiceWorkerClientNavigateResult {
                    request_id: navigate.request_id,
                    result: Err(ServiceWorkerClientNavigateError::type_error(
                        "This service worker is not the client's active service worker.".to_owned(),
                    )),
                };
                drop(state);
                self.finish_client_navigate(navigate.source_version_id, run, result);
                return;
            }
            (
                target_client
                    .window_completion_target()
                    .expect("window client navigate target"),
                target_client
                    .endpoint
                    .page_task_sender()
                    .expect("window client should have a page completion endpoint"),
            )
        };
        let (target, completion_tx) = delivery;
        let _ = completion_tx.send_service_worker_client_navigate_request(
            ServiceWorkerClientNavigateRequestCompletion {
                target,
                request_id: navigate.request_id,
                source_version_id: navigate.source_version_id,
                source_run: run,
                url: navigate.url,
            },
        );
    }

    pub(super) fn finish_client_navigate_completed(
        &self,
        completion: ServiceWorkerClientNavigateCompletion,
    ) {
        self.finish_client_navigate(
            completion.source_version_id,
            completion.source_run,
            ServiceWorkerClientNavigateResult {
                request_id: completion.request_id,
                result: completion.result,
            },
        );
    }

    pub(super) fn finish_client_focus_requested(
        &self,
        focus: ServiceWorkerClientFocus,
        run: RendererServiceWorkerRunIdentity,
    ) {
        let delivery = {
            let state = self.inner.state.lock();
            let Some(source_version) = state.versions.get(&focus.source_version_id) else {
                return;
            };
            if source_version.run != run {
                return;
            }
            let Some(source_registration) =
                state.registrations.get(&source_version.registration_id)
            else {
                return;
            };
            let Some(target_client) = state.live_clients.get(&focus.target_client_id) else {
                let result = ServiceWorkerClientFocusResult {
                    request_id: focus.request_id,
                    result: Err(ServiceWorkerClientFocusError::not_found()),
                };
                drop(state);
                self.finish_client_focus(focus.source_version_id, run, result);
                return;
            };
            if target_client.client_type != ServiceWorkerClientType::Window {
                let result = ServiceWorkerClientFocusResult {
                    request_id: focus.request_id,
                    result: Err(ServiceWorkerClientFocusError::not_found()),
                };
                drop(state);
                self.finish_client_focus(focus.source_version_id, run, result);
                return;
            }
            // WindowClient.focus is allowed for uncontrolled and out-of-scope
            // Windows. Its boundary is the complete native storage key, also
            // covering inherited about:blank/srcdoc origins and partitions.
            if target_client.storage_key != source_registration.storage_key {
                let result = ServiceWorkerClientFocusResult {
                    request_id: focus.request_id,
                    result: Err(ServiceWorkerClientFocusError::not_found()),
                };
                drop(state);
                self.finish_client_focus(focus.source_version_id, run, result);
                return;
            }
            if !target_client.execution_ready {
                let result = ServiceWorkerClientFocusResult {
                    request_id: focus.request_id,
                    result: Err(ServiceWorkerClientFocusError::not_found()),
                };
                drop(state);
                self.finish_client_focus(focus.source_version_id, run, result);
                return;
            }
            if target_client.discarded_or_frozen {
                let result = ServiceWorkerClientFocusResult {
                    request_id: focus.request_id,
                    result: Err(ServiceWorkerClientFocusError::inactive()),
                };
                drop(state);
                self.finish_client_focus(focus.source_version_id, run, result);
                return;
            }
            (
                target_client
                    .window_completion_target()
                    .expect("window client focus target"),
                target_client
                    .endpoint
                    .page_task_sender()
                    .expect("window client should have a page completion endpoint"),
            )
        };
        let (target, completion_tx) = delivery;
        let _ = completion_tx.send_service_worker_client_focus_request(
            ServiceWorkerClientFocusRequestCompletion {
                target,
                request_id: focus.request_id,
                source_version_id: focus.source_version_id,
                source_run: run,
            },
        );
    }

    pub(super) fn finish_client_focus_completed(
        &self,
        completion: ServiceWorkerClientFocusCompletion,
    ) {
        self.finish_client_focus(
            completion.source_version_id,
            completion.source_run,
            ServiceWorkerClientFocusResult {
                request_id: completion.request_id,
                result: completion.result,
            },
        );
    }

    pub(super) fn finish_clients_open_window_requested(
        &self,
        open_window: ServiceWorkerClientsOpenWindow,
        run: RendererServiceWorkerRunIdentity,
    ) {
        let delivery = {
            let state = self.inner.state.lock();
            let Some(source_version) = state.versions.get(&open_window.source_version_id) else {
                return;
            };
            if source_version.run != run {
                return;
            }
            if source_version.lifecycle_state == ServiceWorkerVersionLifecycleState::Redundant {
                return;
            }
            let Some(source_registration) =
                state.registrations.get(&source_version.registration_id)
            else {
                return;
            };
            if source_registration.active_version_id != Some(open_window.source_version_id) {
                return;
            }
            let target_client = source_registration
                .controlled_client_ids
                .iter()
                .filter_map(|client_id| state.live_clients.get(client_id))
                .filter(|client| client.client_type == ServiceWorkerClientType::Window)
                .filter(|client| client.execution_ready && !client.discarded_or_frozen)
                .min_by_key(|client| client.id.as_u64());
            let Some(target_client) = target_client else {
                let result = ServiceWorkerClientsOpenWindowResult {
                    request_id: open_window.request_id,
                    result: Err(ServiceWorkerClientsOpenWindowError::type_error(
                        "No live window client is available to host openWindow().",
                    )),
                };
                drop(state);
                self.finish_clients_open_window(open_window.source_version_id, run, result);
                return;
            };
            (
                target_client
                    .window_completion_target()
                    .expect("window client openWindow host"),
                target_client
                    .endpoint
                    .page_task_sender()
                    .expect("window client should have a page completion endpoint"),
            )
        };
        let (host, completion_tx) = delivery;
        let _ = completion_tx.send_service_worker_clients_open_window_request(
            ServiceWorkerClientsOpenWindowRequestCompletion {
                host,
                request_id: open_window.request_id,
                source_version_id: open_window.source_version_id,
                source_run: run,
                url: open_window.url,
            },
        );
    }

    pub(super) fn finish_clients_open_window_completed(
        &self,
        completion: ServiceWorkerClientsOpenWindowCompletion,
    ) {
        self.finish_clients_open_window(
            completion.source_version_id,
            completion.source_run,
            ServiceWorkerClientsOpenWindowResult {
                request_id: completion.request_id,
                result: completion.result,
            },
        );
    }

    pub(crate) fn client_navigate_result_for_current_window_client(
        &self,
        source_version_id: ServiceWorkerVersionId,
        client_id: ServiceWorkerClientId,
        ancestor_origins: Vec<String>,
    ) -> Result<Option<ServiceWorkerClientSnapshot>, ServiceWorkerClientNavigateError> {
        let state = self.inner.state.lock();
        let Some(source_version) = state.versions.get(&source_version_id) else {
            return Err(ServiceWorkerClientNavigateError::type_error(
                "The service worker version was not found.",
            ));
        };
        let Some(source_registration) = state.registrations.get(&source_version.registration_id)
        else {
            return Err(ServiceWorkerClientNavigateError::type_error(
                "The service worker registration was not found.",
            ));
        };
        let Some(client) = state.live_clients.get(&client_id) else {
            return Ok(None);
        };
        if client.client_type != ServiceWorkerClientType::Window {
            return Ok(None);
        }
        if !moli_url::same_origin(&source_version.script_url, &client.document_url) {
            return Ok(None);
        }
        let mut snapshot = service_worker_client_snapshot(source_registration, client);
        snapshot.ancestor_origins = ancestor_origins;
        Ok(Some(snapshot))
    }

    pub(crate) fn client_focus_result_for_current_window_client(
        &self,
        source_version_id: ServiceWorkerVersionId,
        client_id: ServiceWorkerClientId,
        document_owner: Option<crate::window_document_identity::WindowDocumentOwner>,
        native_state: Option<crate::service_worker_runtime::ServiceWorkerWindowClientState>,
    ) -> Result<ServiceWorkerClientSnapshot, ServiceWorkerClientFocusError> {
        let state = self.inner.state.lock();
        let Some(source_version) = state.versions.get(&source_version_id) else {
            return Err(ServiceWorkerClientFocusError::not_found());
        };
        let registration_id = source_version.registration_id;
        let Some(source_registration) = state.registrations.get(&registration_id) else {
            return Err(ServiceWorkerClientFocusError::not_found());
        };
        let controlled = source_registration
            .controlled_client_ids
            .contains(&client_id);
        let Some(client) = state.live_clients.get(&client_id) else {
            return Err(ServiceWorkerClientFocusError::not_found());
        };
        if client.client_type != ServiceWorkerClientType::Window {
            return Err(ServiceWorkerClientFocusError::not_found());
        }
        if client.document_owner != document_owner {
            return Err(ServiceWorkerClientFocusError::not_found());
        }
        if client.storage_key != source_registration.storage_key {
            return Err(ServiceWorkerClientFocusError::not_found());
        }
        if !client.execution_ready {
            return Err(ServiceWorkerClientFocusError::not_found());
        }
        if client.discarded_or_frozen {
            return Err(ServiceWorkerClientFocusError::inactive());
        }
        let Some(native_state) = native_state else {
            return Err(ServiceWorkerClientFocusError::not_found());
        };
        if !native_state.focused {
            return Err(ServiceWorkerClientFocusError::inactive());
        }
        let mut snapshot = service_worker_client_snapshot_with_controlled(client, controlled);
        snapshot.frame_type = native_state.frame_type;
        snapshot.visibility_state = native_state.visibility_state;
        snapshot.focused = native_state.focused;
        snapshot.ancestor_origins = native_state.ancestor_origins;
        Ok(snapshot)
    }

    pub(crate) fn window_client_focus_reply(
        &self,
        completion: crate::types::ServiceWorkerClientFocusRequestCompletion,
    ) -> crate::service_worker_runtime::ServiceWorkerWindowClientStateReply {
        let service = self.downgrade();
        crate::service_worker_runtime::ServiceWorkerWindowClientStateReply::new(
            completion.target.client_id,
            move |native_state| {
                let Some(service) = service.upgrade() else {
                    return;
                };
                let result = service.client_focus_result_for_current_window_client(
                    completion.source_version_id,
                    completion.target.client_id,
                    Some(completion.target.document_owner),
                    native_state,
                );
                service.enqueue_client_focus_completed(
                    crate::types::ServiceWorkerClientFocusCompletion {
                        request_id: completion.request_id,
                        source_version_id: completion.source_version_id,
                        source_run: completion.source_run,
                        result,
                    },
                );
            },
        )
    }

    pub(crate) fn window_client_focus_currentness(
        &self,
        completion: &crate::types::ServiceWorkerClientFocusRequestCompletion,
    ) -> Box<dyn Fn() -> bool + Send + Sync> {
        let service = self.downgrade();
        let version_id = completion.source_version_id;
        let run = completion.source_run.clone();
        let target = completion.target;
        Box::new(move || {
            let Some(service) = service.upgrade() else {
                return false;
            };
            let state = service.inner.state.lock();
            let Some(version) = state.versions.get(&version_id) else {
                return false;
            };
            if version.run != run {
                return false;
            }
            let Some(registration) = state.registrations.get(&version.registration_id) else {
                return false;
            };
            state
                .live_clients
                .get(&target.client_id)
                .is_some_and(|client| {
                    client.window_completion_target() == Some(target)
                        && client.execution_ready
                        && !client.discarded_or_frozen
                        && client.storage_key == registration.storage_key
                })
        })
    }

    fn finish_client_navigate(
        &self,
        version_id: ServiceWorkerVersionId,
        run: RendererServiceWorkerRunIdentity,
        result: ServiceWorkerClientNavigateResult,
    ) {
        let host = {
            let state = self.inner.state.lock();
            let Some(version) = state.versions.get(&version_id) else {
                return;
            };
            if version.run != run {
                return;
            }
            match &version.running_state {
                ServiceWorkerVersionRunningState::Running { host } => Some(host.clone()),
                ServiceWorkerVersionRunningState::Starting { .. }
                | ServiceWorkerVersionRunningState::Stopped => None,
            }
        };
        if let Some(host) = host {
            host.dispatch_client_navigate_result(result);
        }
    }

    fn finish_client_focus(
        &self,
        version_id: ServiceWorkerVersionId,
        run: RendererServiceWorkerRunIdentity,
        result: ServiceWorkerClientFocusResult,
    ) {
        let host = {
            let state = self.inner.state.lock();
            let Some(version) = state.versions.get(&version_id) else {
                return;
            };
            if version.run != run {
                return;
            }
            match &version.running_state {
                ServiceWorkerVersionRunningState::Running { host } => Some(host.clone()),
                ServiceWorkerVersionRunningState::Starting { .. }
                | ServiceWorkerVersionRunningState::Stopped => None,
            }
        };
        if let Some(host) = host {
            host.dispatch_client_focus_result(result);
        }
    }

    fn finish_clients_open_window(
        &self,
        version_id: ServiceWorkerVersionId,
        run: RendererServiceWorkerRunIdentity,
        result: ServiceWorkerClientsOpenWindowResult,
    ) {
        let host = {
            let state = self.inner.state.lock();
            let Some(version) = state.versions.get(&version_id) else {
                return;
            };
            if version.run != run {
                return;
            }
            match &version.running_state {
                ServiceWorkerVersionRunningState::Running { host } => Some(host.clone()),
                ServiceWorkerVersionRunningState::Starting { .. }
                | ServiceWorkerVersionRunningState::Stopped => None,
            }
        };
        if let Some(host) = host {
            host.dispatch_clients_open_window_result(result);
        }
    }
}
