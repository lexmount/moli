//! Window-client request bodies for browser-context ServiceWorker tasks.
//!
//! These operations update navigation, focus, or popup state and publish their
//! typed ServiceWorker-side result. They do not dispatch a Page callback and
//! never own the selected task's microtask checkpoint.

use anyhow::Result;

use super::{ScriptVm, ServiceWorkerInternalBodyEffect};
use crate::service_worker_runtime::ServiceWorkerClientNavigateError;
use crate::types::{
    ServiceWorkerClientFocusRequestCompletion, ServiceWorkerClientNavigateRequestCompletion,
    ServiceWorkerClientsOpenWindowRequestCompletion,
    ServiceWorkerNotificationActionNavigateRequestCompletion,
};

impl ScriptVm {
    pub(crate) fn apply_service_worker_client_navigate_request_body(
        &mut self,
        completion: ServiceWorkerClientNavigateRequestCompletion,
    ) -> Result<ServiceWorkerInternalBodyEffect> {
        let owner = self
            ._context_host
            .borrow()
            .service_worker_window_client_completion_owner(completion.target);
        let Some(owner) = owner else {
            self._context_host
                .borrow()
                .browser_context_runtime()
                .service_worker_runtime()
                .enqueue_client_navigate_completed(
                    crate::types::ServiceWorkerClientNavigateCompletion {
                        request_id: completion.request_id,
                        source_version_id: completion.source_version_id,
                        source_run: completion.source_run,
                        result: Err(ServiceWorkerClientNavigateError::type_error(
                            "The client was not found.",
                        )),
                    },
                );
            return Ok(ServiceWorkerInternalBodyEffect::ExactTargetUnavailable);
        };

        let browser_context_runtime = self._context_host.borrow().browser_context_runtime();
        match owner.dispatch_scope() {
            crate::native_bridge::OwnerDispatchScope::Child(child_handle) => {
                let request_id = completion.request_id;
                let source_version_id = completion.source_version_id;
                let source_run = completion.source_run;
                let url = completion.url.clone();
                self.with_default_context_scope(move |scope, host_ptr| {
                    let result = unsafe { &mut *host_ptr }
                        .record_pending_service_worker_child_client_navigation(
                            scope,
                            child_handle,
                            url,
                            crate::types::ServiceWorkerClientNavigateContinuation {
                                request_id,
                                source_version_id,
                                source_run: source_run.clone(),
                            },
                        );
                    if let Err(error) = result {
                        browser_context_runtime
                            .service_worker_runtime()
                            .enqueue_client_navigate_completed(
                                crate::types::ServiceWorkerClientNavigateCompletion {
                                    request_id,
                                    source_version_id,
                                    source_run,
                                    result: Err(error),
                                },
                            );
                    }
                    Ok(())
                })?;
                return Ok(ServiceWorkerInternalBodyEffect::InternalActionApplied);
            }
            crate::native_bridge::OwnerDispatchScope::LightweightPopup(_) => {
                browser_context_runtime
                    .service_worker_runtime()
                    .enqueue_client_navigate_completed(
                        crate::types::ServiceWorkerClientNavigateCompletion {
                            request_id: completion.request_id,
                            source_version_id: completion.source_version_id,
                            source_run: completion.source_run,
                            result: Err(ServiceWorkerClientNavigateError::type_error(
                                "The client was not found.",
                            )),
                        },
                    );
                return Ok(ServiceWorkerInternalBodyEffect::InternalActionApplied);
            }
            crate::native_bridge::OwnerDispatchScope::Top => {}
        }
        if self
            ._context_host
            .borrow()
            .has_pending_location_navigation()
        {
            browser_context_runtime
                .service_worker_runtime()
                .enqueue_client_navigate_completed(
                    crate::types::ServiceWorkerClientNavigateCompletion {
                        request_id: completion.request_id,
                        source_version_id: completion.source_version_id,
                        source_run: completion.source_run,
                        result: Err(ServiceWorkerClientNavigateError::type_error(
                            "The client is already navigating.",
                        )),
                    },
                );
            return Ok(ServiceWorkerInternalBodyEffect::InternalActionApplied);
        }
        let source_version_id = completion.source_version_id;
        let request_id = completion.request_id;
        let url = completion.url.clone();
        self._context_host
            .borrow_mut()
            .record_pending_service_worker_client_navigation(
                url,
                crate::types::ServiceWorkerClientNavigateContinuation {
                    request_id,
                    source_version_id,
                    source_run: completion.source_run,
                },
            );
        Ok(ServiceWorkerInternalBodyEffect::InternalActionApplied)
    }

    pub(crate) fn apply_service_worker_client_focus_request_body(
        &mut self,
        completion: ServiceWorkerClientFocusRequestCompletion,
    ) -> Result<ServiceWorkerInternalBodyEffect> {
        let owner_is_current = self
            ._context_host
            .borrow()
            .service_worker_window_client_completion_owner(completion.target)
            .is_some();
        if !owner_is_current {
            self._context_host
                .borrow()
                .browser_context_runtime()
                .service_worker_runtime()
                .enqueue_client_focus_completed(crate::types::ServiceWorkerClientFocusCompletion {
                    request_id: completion.request_id,
                    source_version_id: completion.source_version_id,
                    source_run: completion.source_run,
                    result: Err(crate::runtime::ServiceWorkerClientFocusError::not_found()),
                });
            return Ok(ServiceWorkerInternalBodyEffect::ExactTargetUnavailable);
        }

        let browser_context_runtime = self._context_host.borrow().browser_context_runtime();
        let result = browser_context_runtime
            .service_worker_runtime()
            .client_focus_result_for_current_window_client(
                completion.source_version_id,
                completion.target.client_id,
            );
        browser_context_runtime
            .service_worker_runtime()
            .enqueue_client_focus_completed(crate::types::ServiceWorkerClientFocusCompletion {
                request_id: completion.request_id,
                source_version_id: completion.source_version_id,
                source_run: completion.source_run,
                result,
            });
        Ok(ServiceWorkerInternalBodyEffect::InternalActionApplied)
    }

    pub(crate) fn apply_service_worker_clients_open_window_request_body(
        &mut self,
        completion: ServiceWorkerClientsOpenWindowRequestCompletion,
    ) -> Result<ServiceWorkerInternalBodyEffect> {
        let host_owner = self
            ._context_host
            .borrow()
            .service_worker_window_client_completion_owner(completion.host);
        let Some(host_owner) = host_owner else {
            self._context_host
                .borrow()
                .browser_context_runtime()
                .service_worker_runtime()
                .enqueue_clients_open_window_completed(
                    crate::types::ServiceWorkerClientsOpenWindowCompletion {
                        request_id: completion.request_id,
                        source_version_id: completion.source_version_id,
                        source_run: completion.source_run,
                        result: Err(
                            crate::runtime::ServiceWorkerClientsOpenWindowError::type_error(
                                "No live window client is available to host openWindow().",
                            ),
                        ),
                    },
                );
            return Ok(ServiceWorkerInternalBodyEffect::ExactTargetUnavailable);
        };
        if !matches!(completion.url.scheme(), "http" | "https") {
            self._context_host
                .borrow()
                .browser_context_runtime()
                .service_worker_runtime()
                .enqueue_clients_open_window_completed(
                    crate::types::ServiceWorkerClientsOpenWindowCompletion {
                        request_id: completion.request_id,
                        source_version_id: completion.source_version_id,
                        source_run: completion.source_run,
                        result: Err(
                            crate::runtime::ServiceWorkerClientsOpenWindowError::type_error(
                                format!("'{}' cannot be opened.", completion.url.as_str()),
                            ),
                        ),
                    },
                );
            return Ok(ServiceWorkerInternalBodyEffect::InternalActionApplied);
        }

        let host_scope = host_owner.dispatch_scope();
        let Some(creator_base_url) = self
            ._context_host
            .borrow_mut()
            .service_worker_window_request_context(host_scope)
            .map(|context| context.document_url().clone())
        else {
            self._context_host
                .borrow()
                .browser_context_runtime()
                .service_worker_runtime()
                .enqueue_clients_open_window_completed(
                    crate::types::ServiceWorkerClientsOpenWindowCompletion {
                        request_id: completion.request_id,
                        source_version_id: completion.source_version_id,
                        source_run: completion.source_run,
                        result: Err(
                            crate::runtime::ServiceWorkerClientsOpenWindowError::type_error(
                                "No live window client is available to host openWindow().",
                            ),
                        ),
                    },
                );
            return Ok(ServiceWorkerInternalBodyEffect::ExactTargetUnavailable);
        };
        let url = completion.url.to_string();
        self.with_default_context_scope(|scope, host_ptr| {
            let host = unsafe { &mut *host_ptr };
            let previous_owner_context = host_scope.enter(scope);
            let popup_id = host
                .open_lightweight_popup_window(
                    scope,
                    host_ptr,
                    None,
                    None,
                    "_blank",
                    &url,
                    creator_base_url.clone(),
                    crate::document_runtime::DocumentPolicyContainer::default(),
                )
                .map(|opened_popup| opened_popup.popup_id);
            let session_storage_store = popup_id
                .and_then(|popup_id| host.lightweight_popup_session_storage_store(popup_id));
            let initial_empty_document_storage_key = popup_id.and_then(|popup_id| {
                host.lightweight_popup_initial_empty_document_storage_key(popup_id)
            });
            host.record_pending_popup_activation(
                crate::RendererPendingPopupActivation::browser_context(
                    popup_id,
                    url.clone(),
                    "_blank".to_owned(),
                )
                .with_initial_auxiliary_state(
                    session_storage_store,
                    initial_empty_document_storage_key,
                ),
                None,
            );
            if let Some(popup_id) = popup_id {
                host.begin_service_worker_clients_open_window_popup(
                    popup_id,
                    completion.url.clone(),
                    completion.request_id,
                    completion.source_version_id,
                    completion.source_run,
                );
                host_scope.restore(scope, previous_owner_context);
                return Ok(());
            }
            host.browser_context_runtime()
                .service_worker_runtime()
                .enqueue_clients_open_window_completed(
                    crate::types::ServiceWorkerClientsOpenWindowCompletion {
                        request_id: completion.request_id,
                        source_version_id: completion.source_version_id,
                        source_run: completion.source_run,
                        result: Ok(None),
                    },
                );
            host_scope.restore(scope, previous_owner_context);
            Ok(())
        })?;
        Ok(ServiceWorkerInternalBodyEffect::InternalActionApplied)
    }

    pub(crate) fn apply_service_worker_notification_action_navigate_request_body(
        &mut self,
        completion: ServiceWorkerNotificationActionNavigateRequestCompletion,
    ) -> Result<ServiceWorkerInternalBodyEffect> {
        let host_owner = self
            ._context_host
            .borrow()
            .service_worker_window_client_completion_owner(completion.host);
        let Some(host_owner) = host_owner else {
            return Ok(ServiceWorkerInternalBodyEffect::ExactTargetUnavailable);
        };
        let host_scope = host_owner.dispatch_scope();
        let Some(creator_base_url) = self
            ._context_host
            .borrow_mut()
            .service_worker_window_request_context(host_scope)
            .map(|context| context.document_url().clone())
        else {
            return Ok(ServiceWorkerInternalBodyEffect::ExactTargetUnavailable);
        };

        let url = completion.url.to_string();
        self.with_default_context_scope(|scope, host_ptr| {
            let host = unsafe { &mut *host_ptr };
            let previous_owner_context = host_scope.enter(scope);
            let popup_id = host
                .open_lightweight_popup_window(
                    scope,
                    host_ptr,
                    None,
                    None,
                    "_blank",
                    &url,
                    creator_base_url.clone(),
                    crate::document_runtime::DocumentPolicyContainer::default(),
                )
                .map(|opened_popup| opened_popup.popup_id);
            let session_storage_store = popup_id
                .and_then(|popup_id| host.lightweight_popup_session_storage_store(popup_id));
            let initial_empty_document_storage_key = popup_id.and_then(|popup_id| {
                host.lightweight_popup_initial_empty_document_storage_key(popup_id)
            });
            host.record_pending_popup_activation(
                crate::RendererPendingPopupActivation::browser_context(
                    popup_id,
                    url.clone(),
                    "_blank".to_owned(),
                )
                .with_initial_auxiliary_state(
                    session_storage_store,
                    initial_empty_document_storage_key,
                ),
                None,
            );
            host_scope.restore(scope, previous_owner_context);
            Ok(())
        })?;
        Ok(ServiceWorkerInternalBodyEffect::InternalActionApplied)
    }
}
