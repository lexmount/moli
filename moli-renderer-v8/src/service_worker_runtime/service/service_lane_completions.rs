use super::*;

impl ServiceWorkerRuntimeService {
    pub(in crate::service_worker_runtime) fn enqueue_worker_start_completed(
        &self,
        version_id: ServiceWorkerVersionId,
        run: RendererServiceWorkerRunIdentity,
        final_script_url: String,
        script_resource: ServiceWorkerScriptResource,
        fetch_handler_type: ServiceWorkerFetchHandlerType,
    ) {
        self.enqueue_service_lane_completion(
            ServiceWorkerRuntimeCompletion::version_start_completed(
                self.downgrade(),
                version_id,
                run,
                final_script_url,
                script_resource,
                fetch_handler_type,
            ),
        );
        self.signal_service_lane_wake();
    }

    pub(in crate::service_worker_runtime) fn enqueue_worker_start_failed(
        &self,
        version_id: ServiceWorkerVersionId,
        run: RendererServiceWorkerRunIdentity,
        failure: ServiceWorkerVersionStartFailure,
    ) {
        self.enqueue_service_lane_completion(ServiceWorkerRuntimeCompletion::version_start_failed(
            self.downgrade(),
            version_id,
            run,
            failure,
        ));
        self.signal_service_lane_wake();
    }

    pub(in crate::service_worker_runtime) fn enqueue_imported_script_loaded(
        &self,
        registration_id: ServiceWorkerRegistrationId,
        version_id: ServiceWorkerVersionId,
        run: RendererServiceWorkerRunIdentity,
        resource: WorkerScriptResource,
    ) {
        self.enqueue_service_lane_completion(
            ServiceWorkerRuntimeCompletion::imported_script_loaded(
                self.downgrade(),
                registration_id,
                version_id,
                run,
                resource,
            ),
        );
        self.signal_service_lane_wake();
    }

    pub(in crate::service_worker_runtime) fn enqueue_main_script_update_check_completed(
        &self,
        registration_id: ServiceWorkerRegistrationId,
        result: ServiceWorkerScriptUpdateCheckCompletion,
    ) {
        self.enqueue_service_lane_completion(
            ServiceWorkerRuntimeCompletion::main_script_update_check_completed(
                self.downgrade(),
                registration_id,
                result,
            ),
        );
        self.signal_service_lane_wake();
    }

    pub(in crate::service_worker_runtime) fn enqueue_lifecycle_event_completed(
        &self,
        completion: ServiceWorkerLifecycleCompletion,
    ) {
        self.enqueue_service_lane_completion(
            ServiceWorkerRuntimeCompletion::lifecycle_event_completed(self.downgrade(), completion),
        );
        self.signal_service_lane_wake();
    }

    pub(in crate::service_worker_runtime) fn enqueue_fetch_event_completed(
        &self,
        completion: ServiceWorkerFetchCompletion,
    ) {
        self.enqueue_service_lane_completion(
            ServiceWorkerRuntimeCompletion::fetch_event_completed(self.downgrade(), completion),
        );
        self.signal_service_lane_wake();
    }

    pub(in crate::service_worker_runtime) fn enqueue_fetch_stream_started(
        &self,
        started: ServiceWorkerFetchStreamStarted,
    ) {
        self.enqueue_service_lane_completion(ServiceWorkerRuntimeCompletion::fetch_stream_started(
            self.downgrade(),
            started,
        ));
        self.signal_service_lane_wake();
    }

    pub(in crate::service_worker_runtime) fn enqueue_fetch_stream_chunk(
        &self,
        chunk: ServiceWorkerFetchStreamChunk,
    ) {
        self.enqueue_service_lane_completion(ServiceWorkerRuntimeCompletion::fetch_stream_chunk(
            self.downgrade(),
            chunk,
        ));
        self.signal_service_lane_wake();
    }

    pub(in crate::service_worker_runtime) fn enqueue_message_event_completed(
        &self,
        completion: ServiceWorkerMessageCompletion,
    ) {
        self.enqueue_service_lane_completion(
            ServiceWorkerRuntimeCompletion::message_event_completed(self.downgrade(), completion),
        );
        self.signal_service_lane_wake();
    }

    pub(in crate::service_worker_runtime) fn enqueue_notification_event_completed(
        &self,
        completion: ServiceWorkerNotificationCompletion,
    ) {
        self.enqueue_service_lane_completion(
            ServiceWorkerRuntimeCompletion::notification_event_completed(
                self.downgrade(),
                completion,
            ),
        );
        self.signal_service_lane_wake();
    }

    pub(in crate::service_worker_runtime) fn enqueue_push_event_completed(
        &self,
        completion: ServiceWorkerPushCompletion,
    ) {
        self.enqueue_service_lane_completion(ServiceWorkerRuntimeCompletion::push_event_completed(
            self.downgrade(),
            completion,
        ));
        self.signal_service_lane_wake();
    }

    pub(in crate::service_worker_runtime) fn enqueue_sync_event_completed(
        &self,
        completion: ServiceWorkerSyncCompletion,
    ) {
        self.enqueue_service_lane_completion(ServiceWorkerRuntimeCompletion::sync_event_completed(
            self.downgrade(),
            completion,
        ));
        self.signal_service_lane_wake();
    }

    pub(in crate::service_worker_runtime) fn enqueue_periodic_sync_event_completed(
        &self,
        completion: ServiceWorkerPeriodicSyncCompletion,
    ) {
        self.enqueue_service_lane_completion(
            ServiceWorkerRuntimeCompletion::periodic_sync_event_completed(
                self.downgrade(),
                completion,
            ),
        );
        self.signal_service_lane_wake();
    }

    pub(in crate::service_worker_runtime) fn enqueue_show_notification_requested(
        &self,
        request: ServiceWorkerShowNotification,
        run: RendererServiceWorkerRunIdentity,
        source_host: SharedRendererServiceWorkerHost,
    ) {
        self.enqueue_service_lane_completion(
            ServiceWorkerRuntimeCompletion::show_notification_requested(
                self.downgrade(),
                request,
                run,
                source_host,
            ),
        );
        self.signal_service_lane_wake();
    }

    pub(in crate::service_worker_runtime) fn enqueue_get_notifications_requested(
        &self,
        request: ServiceWorkerGetNotifications,
        run: RendererServiceWorkerRunIdentity,
        source_host: SharedRendererServiceWorkerHost,
    ) {
        self.enqueue_service_lane_completion(
            ServiceWorkerRuntimeCompletion::get_notifications_requested(
                self.downgrade(),
                request,
                run,
                source_host,
            ),
        );
        self.signal_service_lane_wake();
    }

    pub(in crate::service_worker_runtime) fn enqueue_sync_registration_requested(
        &self,
        request: ServiceWorkerSyncRegistration,
        run: RendererServiceWorkerRunIdentity,
        source_host: SharedRendererServiceWorkerHost,
    ) {
        self.enqueue_service_lane_completion(
            ServiceWorkerRuntimeCompletion::sync_registration_requested(
                self.downgrade(),
                request,
                run,
                source_host,
            ),
        );
        self.signal_service_lane_wake();
    }

    pub(in crate::service_worker_runtime) fn enqueue_sync_get_tags_requested(
        &self,
        request: ServiceWorkerSyncGetTags,
        run: RendererServiceWorkerRunIdentity,
        source_host: SharedRendererServiceWorkerHost,
    ) {
        self.enqueue_service_lane_completion(
            ServiceWorkerRuntimeCompletion::sync_get_tags_requested(
                self.downgrade(),
                request,
                run,
                source_host,
            ),
        );
        self.signal_service_lane_wake();
    }

    pub(in crate::service_worker_runtime) fn enqueue_periodic_sync_registration_requested(
        &self,
        request: ServiceWorkerPeriodicSyncRegistration,
        run: RendererServiceWorkerRunIdentity,
        source_host: SharedRendererServiceWorkerHost,
    ) {
        self.enqueue_service_lane_completion(
            ServiceWorkerRuntimeCompletion::periodic_sync_registration_requested(
                self.downgrade(),
                request,
                run,
                source_host,
            ),
        );
        self.signal_service_lane_wake();
    }

    pub(in crate::service_worker_runtime) fn enqueue_periodic_sync_get_tags_requested(
        &self,
        request: ServiceWorkerPeriodicSyncGetTags,
        run: RendererServiceWorkerRunIdentity,
        source_host: SharedRendererServiceWorkerHost,
    ) {
        self.enqueue_service_lane_completion(
            ServiceWorkerRuntimeCompletion::periodic_sync_get_tags_requested(
                self.downgrade(),
                request,
                run,
                source_host,
            ),
        );
        self.signal_service_lane_wake();
    }

    pub(in crate::service_worker_runtime) fn enqueue_periodic_sync_unregistration_requested(
        &self,
        request: ServiceWorkerPeriodicSyncUnregistration,
        run: RendererServiceWorkerRunIdentity,
        source_host: SharedRendererServiceWorkerHost,
    ) {
        self.enqueue_service_lane_completion(
            ServiceWorkerRuntimeCompletion::periodic_sync_unregistration_requested(
                self.downgrade(),
                request,
                run,
                source_host,
            ),
        );
        self.signal_service_lane_wake();
    }

    pub(in crate::service_worker_runtime) fn enqueue_push_subscribe_requested(
        &self,
        request: ServiceWorkerPushSubscribe,
        run: RendererServiceWorkerRunIdentity,
        source_host: SharedRendererServiceWorkerHost,
    ) {
        self.enqueue_service_lane_completion(
            ServiceWorkerRuntimeCompletion::push_subscribe_requested(
                self.downgrade(),
                request,
                run,
                source_host,
            ),
        );
        self.signal_service_lane_wake();
    }

    pub(in crate::service_worker_runtime) fn enqueue_push_get_subscription_requested(
        &self,
        request: ServiceWorkerPushGetSubscription,
        run: RendererServiceWorkerRunIdentity,
        source_host: SharedRendererServiceWorkerHost,
    ) {
        self.enqueue_service_lane_completion(
            ServiceWorkerRuntimeCompletion::push_get_subscription_requested(
                self.downgrade(),
                request,
                run,
                source_host,
            ),
        );
        self.signal_service_lane_wake();
    }

    pub(in crate::service_worker_runtime) fn enqueue_push_unsubscribe_requested(
        &self,
        request: ServiceWorkerPushUnsubscribe,
        run: RendererServiceWorkerRunIdentity,
        source_host: SharedRendererServiceWorkerHost,
    ) {
        self.enqueue_service_lane_completion(
            ServiceWorkerRuntimeCompletion::push_unsubscribe_requested(
                self.downgrade(),
                request,
                run,
                source_host,
            ),
        );
        self.signal_service_lane_wake();
    }

    pub(in crate::service_worker_runtime) fn enqueue_close_notification_requested(
        &self,
        request: ServiceWorkerCloseNotification,
        run: RendererServiceWorkerRunIdentity,
    ) {
        self.enqueue_service_lane_completion(
            ServiceWorkerRuntimeCompletion::close_notification_requested(
                self.downgrade(),
                request,
                run,
            ),
        );
        self.signal_service_lane_wake();
    }

    pub(crate) fn enqueue_client_message(&self, message: ServiceWorkerClientMessage) {
        self.enqueue_service_lane_completion(ServiceWorkerRuntimeCompletion::client_message(
            self.downgrade(),
            message,
        ));
        self.signal_service_lane_wake();
    }

    pub(crate) fn enqueue_worker_message(&self, message: ServiceWorkerWorkerMessage) {
        self.enqueue_service_lane_completion(ServiceWorkerRuntimeCompletion::worker_message(
            self.downgrade(),
            message,
        ));
        self.signal_service_lane_wake();
    }

    pub(crate) fn enqueue_client_query(
        &self,
        query: ServiceWorkerClientQuery,
        run: RendererServiceWorkerRunIdentity,
    ) {
        self.enqueue_service_lane_completion(ServiceWorkerRuntimeCompletion::client_query(
            self.downgrade(),
            query,
            run,
        ));
        self.signal_service_lane_wake();
    }

    pub(crate) fn enqueue_client_navigate(
        &self,
        navigate: ServiceWorkerClientNavigate,
        run: RendererServiceWorkerRunIdentity,
    ) {
        self.enqueue_service_lane_completion(ServiceWorkerRuntimeCompletion::client_navigate(
            self.downgrade(),
            navigate,
            run,
        ));
        self.signal_service_lane_wake();
    }

    pub(crate) fn enqueue_client_focus(
        &self,
        focus: ServiceWorkerClientFocus,
        run: RendererServiceWorkerRunIdentity,
    ) {
        self.enqueue_service_lane_completion(ServiceWorkerRuntimeCompletion::client_focus(
            self.downgrade(),
            focus,
            run,
        ));
        self.signal_service_lane_wake();
    }

    pub(crate) fn enqueue_clients_open_window(
        &self,
        open_window: ServiceWorkerClientsOpenWindow,
        run: RendererServiceWorkerRunIdentity,
    ) {
        self.enqueue_service_lane_completion(ServiceWorkerRuntimeCompletion::clients_open_window(
            self.downgrade(),
            open_window,
            run,
        ));
        self.signal_service_lane_wake();
    }

    pub(crate) fn enqueue_client_navigate_completed(
        &self,
        completion: ServiceWorkerClientNavigateCompletion,
    ) {
        self.enqueue_service_lane_completion(
            ServiceWorkerRuntimeCompletion::client_navigate_completed(self.downgrade(), completion),
        );
        self.signal_service_lane_wake();
    }

    pub(crate) fn enqueue_client_focus_completed(
        &self,
        completion: ServiceWorkerClientFocusCompletion,
    ) {
        self.enqueue_service_lane_completion(
            ServiceWorkerRuntimeCompletion::client_focus_completed(self.downgrade(), completion),
        );
        self.signal_service_lane_wake();
    }

    pub(crate) fn enqueue_clients_open_window_completed(
        &self,
        completion: ServiceWorkerClientsOpenWindowCompletion,
    ) {
        self.enqueue_service_lane_completion(
            ServiceWorkerRuntimeCompletion::clients_open_window_completed(
                self.downgrade(),
                completion,
            ),
        );
        self.signal_service_lane_wake();
    }

    pub(super) fn enqueue_worker_idle_timeout(&self, timeout: ServiceWorkerIdleTimeout) {
        self.enqueue_service_lane_completion(ServiceWorkerRuntimeCompletion::idle_timeout(
            self.downgrade(),
            timeout,
        ));
        self.signal_service_lane_wake();
    }

    pub(crate) fn enqueue_skip_waiting_requested(
        &self,
        registration_id: ServiceWorkerRegistrationId,
        version_id: ServiceWorkerVersionId,
    ) {
        self.enqueue_service_lane_completion(
            ServiceWorkerRuntimeCompletion::skip_waiting_requested(
                self.downgrade(),
                registration_id,
                version_id,
            ),
        );
        self.signal_service_lane_wake();
    }

    pub(crate) fn enqueue_clients_claim_requested(
        &self,
        registration_id: ServiceWorkerRegistrationId,
        version_id: ServiceWorkerVersionId,
    ) {
        self.enqueue_service_lane_completion(
            ServiceWorkerRuntimeCompletion::clients_claim_requested(
                self.downgrade(),
                registration_id,
                version_id,
            ),
        );
        self.signal_service_lane_wake();
    }
}

impl WeakServiceWorkerRuntimeService {
    pub(in crate::service_worker_runtime) fn upgrade(&self) -> Option<ServiceWorkerRuntimeService> {
        self.inner
            .upgrade()
            .map(|inner| ServiceWorkerRuntimeService { inner })
    }

    pub(in crate::service_worker_runtime) fn finish_worker_start_completed(
        &self,
        version_id: ServiceWorkerVersionId,
        run: RendererServiceWorkerRunIdentity,
        final_script_url: String,
        script_resource: Option<ServiceWorkerScriptResource>,
        fetch_handler_type: ServiceWorkerFetchHandlerType,
    ) {
        let Some(service) = self.upgrade() else {
            return;
        };
        service.finish_worker_start_completed_with_script_resource(
            version_id,
            run,
            final_script_url,
            script_resource,
            fetch_handler_type,
        );
    }

    pub(in crate::service_worker_runtime) fn finish_worker_start_failed(
        &self,
        version_id: ServiceWorkerVersionId,
        run: RendererServiceWorkerRunIdentity,
        failure: ServiceWorkerVersionStartFailure,
    ) {
        let Some(service) = self.upgrade() else {
            return;
        };
        service.finish_worker_start_failed(version_id, run, failure);
    }

    pub(in crate::service_worker_runtime) fn finish_imported_script_loaded(
        &self,
        registration_id: ServiceWorkerRegistrationId,
        version_id: ServiceWorkerVersionId,
        run: RendererServiceWorkerRunIdentity,
        resource: WorkerScriptResource,
    ) {
        let Some(service) = self.upgrade() else {
            return;
        };
        service.finish_imported_script_loaded(registration_id, version_id, run, resource);
    }

    pub(in crate::service_worker_runtime) fn finish_main_script_update_check_completed(
        &self,
        registration_id: ServiceWorkerRegistrationId,
        result: ServiceWorkerScriptUpdateCheckCompletion,
    ) {
        let Some(service) = self.upgrade() else {
            return;
        };
        service.finish_main_script_update_check_completed(registration_id, result);
    }

    pub(in crate::service_worker_runtime) fn finish_lifecycle_event_completed(
        &self,
        completion: ServiceWorkerLifecycleCompletion,
    ) {
        let Some(service) = self.upgrade() else {
            return;
        };
        service.finish_lifecycle_event_completed(completion);
    }

    pub(in crate::service_worker_runtime) fn finish_fetch_event_completed(
        &self,
        completion: ServiceWorkerFetchCompletion,
    ) {
        let Some(service) = self.upgrade() else {
            return;
        };
        service.finish_fetch_event_completed(completion);
    }

    pub(in crate::service_worker_runtime) fn finish_fetch_stream_started(
        &self,
        started: ServiceWorkerFetchStreamStarted,
    ) {
        let Some(service) = self.upgrade() else {
            return;
        };
        service.finish_fetch_stream_started(started);
    }

    pub(in crate::service_worker_runtime) fn finish_fetch_stream_chunk(
        &self,
        chunk: ServiceWorkerFetchStreamChunk,
    ) {
        let Some(service) = self.upgrade() else {
            return;
        };
        service.finish_fetch_stream_chunk(chunk);
    }

    pub(in crate::service_worker_runtime) fn finish_message_event_completed(
        &self,
        completion: ServiceWorkerMessageCompletion,
    ) {
        let Some(service) = self.upgrade() else {
            return;
        };
        service.finish_message_event_completed(completion);
    }

    pub(in crate::service_worker_runtime) fn finish_notification_event_completed(
        &self,
        completion: ServiceWorkerNotificationCompletion,
    ) {
        let Some(service) = self.upgrade() else {
            return;
        };
        service.finish_notification_event_completed(completion);
    }

    pub(in crate::service_worker_runtime) fn finish_push_event_completed(
        &self,
        completion: ServiceWorkerPushCompletion,
    ) {
        let Some(service) = self.upgrade() else {
            return;
        };
        service.finish_push_event_completed(completion);
    }

    pub(in crate::service_worker_runtime) fn finish_sync_event_completed(
        &self,
        completion: ServiceWorkerSyncCompletion,
    ) {
        let Some(service) = self.upgrade() else {
            return;
        };
        service.finish_sync_event_completed(completion);
    }

    pub(in crate::service_worker_runtime) fn finish_periodic_sync_event_completed(
        &self,
        completion: ServiceWorkerPeriodicSyncCompletion,
    ) {
        let Some(service) = self.upgrade() else {
            return;
        };
        service.finish_periodic_sync_event_completed(completion);
    }

    pub(in crate::service_worker_runtime) fn finish_show_notification_requested(
        &self,
        request: ServiceWorkerShowNotification,
        run: RendererServiceWorkerRunIdentity,
        source_host: SharedRendererServiceWorkerHost,
    ) {
        let Some(service) = self.upgrade() else {
            return;
        };
        service.finish_show_notification_requested(request, run, source_host);
    }

    pub(in crate::service_worker_runtime) fn finish_get_notifications_requested(
        &self,
        request: ServiceWorkerGetNotifications,
        run: RendererServiceWorkerRunIdentity,
        source_host: SharedRendererServiceWorkerHost,
    ) {
        let Some(service) = self.upgrade() else {
            return;
        };
        service.finish_get_notifications_requested(request, run, source_host);
    }

    pub(in crate::service_worker_runtime) fn finish_sync_registration_requested(
        &self,
        request: ServiceWorkerSyncRegistration,
        run: RendererServiceWorkerRunIdentity,
        source_host: SharedRendererServiceWorkerHost,
    ) {
        let Some(service) = self.upgrade() else {
            return;
        };
        service.finish_sync_registration_requested(request, run, source_host);
    }

    pub(in crate::service_worker_runtime) fn finish_sync_get_tags_requested(
        &self,
        request: ServiceWorkerSyncGetTags,
        run: RendererServiceWorkerRunIdentity,
        source_host: SharedRendererServiceWorkerHost,
    ) {
        let Some(service) = self.upgrade() else {
            return;
        };
        service.finish_sync_get_tags_requested(request, run, source_host);
    }

    pub(in crate::service_worker_runtime) fn finish_periodic_sync_registration_requested(
        &self,
        request: ServiceWorkerPeriodicSyncRegistration,
        run: RendererServiceWorkerRunIdentity,
        source_host: SharedRendererServiceWorkerHost,
    ) {
        let Some(service) = self.upgrade() else {
            return;
        };
        service.finish_periodic_sync_registration_requested(request, run, source_host);
    }

    pub(in crate::service_worker_runtime) fn finish_periodic_sync_get_tags_requested(
        &self,
        request: ServiceWorkerPeriodicSyncGetTags,
        run: RendererServiceWorkerRunIdentity,
        source_host: SharedRendererServiceWorkerHost,
    ) {
        let Some(service) = self.upgrade() else {
            return;
        };
        service.finish_periodic_sync_get_tags_requested(request, run, source_host);
    }

    pub(in crate::service_worker_runtime) fn finish_periodic_sync_unregistration_requested(
        &self,
        request: ServiceWorkerPeriodicSyncUnregistration,
        run: RendererServiceWorkerRunIdentity,
        source_host: SharedRendererServiceWorkerHost,
    ) {
        let Some(service) = self.upgrade() else {
            return;
        };
        service.finish_periodic_sync_unregistration_requested(request, run, source_host);
    }

    pub(in crate::service_worker_runtime) fn finish_push_subscribe_requested(
        &self,
        request: ServiceWorkerPushSubscribe,
        run: RendererServiceWorkerRunIdentity,
        source_host: SharedRendererServiceWorkerHost,
    ) {
        let Some(service) = self.upgrade() else {
            return;
        };
        service.finish_push_subscribe_requested(request, run, source_host);
    }

    pub(in crate::service_worker_runtime) fn finish_push_get_subscription_requested(
        &self,
        request: ServiceWorkerPushGetSubscription,
        run: RendererServiceWorkerRunIdentity,
        source_host: SharedRendererServiceWorkerHost,
    ) {
        let Some(service) = self.upgrade() else {
            return;
        };
        service.finish_push_get_subscription_requested(request, run, source_host);
    }

    pub(in crate::service_worker_runtime) fn finish_push_unsubscribe_requested(
        &self,
        request: ServiceWorkerPushUnsubscribe,
        run: RendererServiceWorkerRunIdentity,
        source_host: SharedRendererServiceWorkerHost,
    ) {
        let Some(service) = self.upgrade() else {
            return;
        };
        service.finish_push_unsubscribe_requested(request, run, source_host);
    }

    pub(in crate::service_worker_runtime) fn finish_close_notification_requested(
        &self,
        request: ServiceWorkerCloseNotification,
        run: RendererServiceWorkerRunIdentity,
    ) {
        let Some(service) = self.upgrade() else {
            return;
        };
        service.finish_close_notification_requested(request, run);
    }

    pub(in crate::service_worker_runtime) fn finish_client_message(
        &self,
        message: ServiceWorkerClientMessage,
    ) {
        let Some(service) = self.upgrade() else {
            return;
        };
        service.finish_client_message(message);
    }

    pub(in crate::service_worker_runtime) fn finish_worker_message(
        &self,
        message: ServiceWorkerWorkerMessage,
    ) {
        let Some(service) = self.upgrade() else {
            return;
        };
        service.finish_worker_message(message);
    }

    pub(in crate::service_worker_runtime) fn finish_client_query_requested(
        &self,
        query: ServiceWorkerClientQuery,
        run: RendererServiceWorkerRunIdentity,
    ) {
        let Some(service) = self.upgrade() else {
            return;
        };
        service.finish_client_query_requested(query, run);
    }

    pub(in crate::service_worker_runtime) fn finish_client_navigate_requested(
        &self,
        navigate: ServiceWorkerClientNavigate,
        run: RendererServiceWorkerRunIdentity,
    ) {
        let Some(service) = self.upgrade() else {
            return;
        };
        service.finish_client_navigate_requested(navigate, run);
    }

    pub(in crate::service_worker_runtime) fn finish_client_navigate_completed(
        &self,
        completion: ServiceWorkerClientNavigateCompletion,
    ) {
        let Some(service) = self.upgrade() else {
            return;
        };
        service.finish_client_navigate_completed(completion);
    }

    pub(in crate::service_worker_runtime) fn finish_client_focus_requested(
        &self,
        focus: ServiceWorkerClientFocus,
        run: RendererServiceWorkerRunIdentity,
    ) {
        let Some(service) = self.upgrade() else {
            return;
        };
        service.finish_client_focus_requested(focus, run);
    }

    pub(in crate::service_worker_runtime) fn finish_client_focus_completed(
        &self,
        completion: ServiceWorkerClientFocusCompletion,
    ) {
        let Some(service) = self.upgrade() else {
            return;
        };
        service.finish_client_focus_completed(completion);
    }

    pub(in crate::service_worker_runtime) fn finish_clients_open_window_requested(
        &self,
        open_window: ServiceWorkerClientsOpenWindow,
        run: RendererServiceWorkerRunIdentity,
    ) {
        let Some(service) = self.upgrade() else {
            return;
        };
        service.finish_clients_open_window_requested(open_window, run);
    }

    pub(in crate::service_worker_runtime) fn finish_clients_open_window_completed(
        &self,
        completion: ServiceWorkerClientsOpenWindowCompletion,
    ) {
        let Some(service) = self.upgrade() else {
            return;
        };
        service.finish_clients_open_window_completed(completion);
    }

    pub(in crate::service_worker_runtime) fn finish_worker_idle_timeout(
        &self,
        timeout: ServiceWorkerIdleTimeout,
    ) {
        let Some(service) = self.upgrade() else {
            return;
        };
        service.finish_worker_idle_timeout(timeout);
    }

    pub(in crate::service_worker_runtime) fn finish_worker_skip_waiting_requested(
        &self,
        registration_id: ServiceWorkerRegistrationId,
        version_id: ServiceWorkerVersionId,
    ) {
        let Some(service) = self.upgrade() else {
            return;
        };
        service.finish_worker_skip_waiting_requested(registration_id, version_id);
    }

    pub(in crate::service_worker_runtime) fn finish_worker_clients_claim_requested(
        &self,
        registration_id: ServiceWorkerRegistrationId,
        version_id: ServiceWorkerVersionId,
    ) {
        let Some(service) = self.upgrade() else {
            return;
        };
        service.finish_worker_clients_claim_requested(registration_id, version_id);
    }
}
