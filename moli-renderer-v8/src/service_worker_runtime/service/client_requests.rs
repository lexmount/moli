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
                version_id: message.target_version_id,
                run: target_version.run.clone(),
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
            match &version.running_state {
                ServiceWorkerVersionRunningState::Running { host } => Some(host.clone()),
                ServiceWorkerVersionRunningState::Starting { .. }
                | ServiceWorkerVersionRunningState::Stopped => None,
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
        let result = self.query_clients(&query);
        self.finish_client_query(version_id, run, result);
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
            if source_version.lifecycle_state == ServiceWorkerVersionLifecycleState::Redundant {
                return;
            }
            let Some(source_registration) =
                state.registrations.get(&source_version.registration_id)
            else {
                return;
            };
            if source_registration.active_version_id != Some(focus.source_version_id) {
                return;
            }
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
            if !service_worker_scope_matches_url(
                &source_registration.scope_url,
                &target_client.document_url,
            ) {
                let result = ServiceWorkerClientFocusResult {
                    request_id: focus.request_id,
                    result: Err(ServiceWorkerClientFocusError::not_found()),
                };
                drop(state);
                self.finish_client_focus(focus.source_version_id, run, result);
                return;
            }
            if !source_registration
                .controlled_client_ids
                .contains(&focus.target_client_id)
            {
                let result = ServiceWorkerClientFocusResult {
                    request_id: focus.request_id,
                    result: Err(ServiceWorkerClientFocusError::type_error(
                        "This service worker is not the client's active service worker.".to_owned(),
                    )),
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
        Ok(Some(service_worker_client_snapshot(
            source_registration,
            client,
        )))
    }

    pub(crate) fn client_focus_result_for_current_window_client(
        &self,
        source_version_id: ServiceWorkerVersionId,
        client_id: ServiceWorkerClientId,
    ) -> Result<ServiceWorkerClientSnapshot, ServiceWorkerClientFocusError> {
        let mut state = self.inner.state.lock();
        let Some(source_version) = state.versions.get(&source_version_id) else {
            return Err(ServiceWorkerClientFocusError::not_found());
        };
        let registration_id = source_version.registration_id;
        let script_url = source_version.script_url.clone();
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
        if !moli_url::same_origin(&script_url, &client.document_url) {
            return Err(ServiceWorkerClientFocusError::not_found());
        }
        if !client.execution_ready {
            return Err(ServiceWorkerClientFocusError::not_found());
        }
        if client.discarded_or_frozen {
            return Err(ServiceWorkerClientFocusError::inactive());
        }
        for client in state.live_clients.values_mut() {
            client.focused = false;
        }
        let Some(client) = state.live_clients.get_mut(&client_id) else {
            return Err(ServiceWorkerClientFocusError::not_found());
        };
        client.focused = true;
        Ok(service_worker_client_snapshot_with_controlled(
            client, controlled,
        ))
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
