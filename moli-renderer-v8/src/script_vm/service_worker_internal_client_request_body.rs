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

#[derive(Clone)]
struct ServiceWorkerClientsOpenWindowContinuationSeed {
    request_id: u64,
    source_version_id: crate::runtime::ServiceWorkerVersionId,
    source_run: crate::runtime::RendererServiceWorkerRunIdentity,
}

fn canonical_service_worker_auxiliary_url(url: &url::Url) -> Option<url::Url> {
    match url.scheme() {
        "http" | "https" => Some(url.clone()),
        // Chromium canonicalizes every renderer-accepted `about:` URL to the
        // one browser-owned about:blank initial navigation.
        "about" => url::Url::parse("about:blank").ok(),
        _ => None,
    }
}

impl ScriptVm {
    fn record_service_worker_auxiliary_navigation(
        &mut self,
        host_scope: crate::native_bridge::OwnerDispatchScope,
        source_script_url: url::Url,
        destination_url: url::Url,
        completion: Option<ServiceWorkerClientsOpenWindowContinuationSeed>,
    ) -> Result<bool> {
        self.with_default_context_scope(|scope, host_ptr| {
            let host = unsafe { &mut *host_ptr };
            let previous_owner_context = host_scope.enter(scope);
            let recorded = (|| {
                let creation_policy = crate::document_runtime::DocumentPolicyContainer::default()
                    .into_auxiliary_browsing_context_creation_policy()
                    .expect("browser-context auxiliary creation is not document-sandboxed");
                let auxiliary_browsing_context_policy =
                    creation_policy.renderer_auxiliary_browsing_context_policy();
                let Some(pending_auxiliary_page) =
                    host.reserve_pending_browser_context_auxiliary_page()
                else {
                    return false;
                };
                let destination = destination_url.to_string();
                let navigation_referrer = moli_fetch::referrer_header_value(
                    &source_script_url,
                    &destination_url,
                    None,
                    None,
                )
                .unwrap_or_default();
                let document_referrer = moli_fetch::navigation_referrer_value(
                    &source_script_url,
                    &destination_url,
                    None,
                    None,
                )
                .unwrap_or_default();
                let request = crate::RendererTopLevelNavigationRequest::get(destination.clone())
                    .with_source(crate::RendererTopLevelNavigationSource::browser_context(
                        source_script_url.to_string(),
                        None,
                        false,
                    ));
                let mut activation = crate::RendererPendingPopupActivation::browser_context(
                    None,
                    destination,
                    "_blank".to_owned(),
                    crate::RendererPopupDisposition::Foreground,
                )
                .with_navigation_request(request)
                .with_navigation_referrers(navigation_referrer, String::new(), document_referrer)
                .with_pending_auxiliary_page(Some(pending_auxiliary_page))
                .with_auxiliary_browsing_context_policy(auxiliary_browsing_context_policy);
                if let Some(completion) = completion {
                    activation = activation.with_service_worker_clients_open_window_continuation(
                        crate::RendererServiceWorkerClientsOpenWindowContinuation::new(
                            &host.browser_context_runtime(),
                            pending_auxiliary_page.page_reservation().page_id(),
                            completion.request_id,
                            completion.source_version_id,
                            completion.source_run,
                        ),
                    );
                }
                activation = activation.with_new_target_disposition(
                    crate::RendererPopupNewTargetDisposition::FreshUnnamed,
                );
                host.record_pending_popup_activation(activation, None);
                true
            })();
            host_scope.restore(scope, previous_owner_context);
            Ok(recorded)
        })
    }

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
        let Some(destination_url) = canonical_service_worker_auxiliary_url(&completion.url) else {
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
        };

        let host_scope = host_owner.dispatch_scope();
        let host_is_current = self
            ._context_host
            .borrow_mut()
            .service_worker_window_request_context(host_scope)
            .is_some();
        if !host_is_current {
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
        }
        let completion_seed = ServiceWorkerClientsOpenWindowContinuationSeed {
            request_id: completion.request_id,
            source_version_id: completion.source_version_id,
            source_run: completion.source_run.clone(),
        };
        let recorded = match self.record_service_worker_auxiliary_navigation(
            host_scope,
            completion.source_script_url,
            destination_url,
            Some(completion_seed.clone()),
        ) {
            Ok(recorded) => recorded,
            Err(error) => {
                self._context_host
                    .borrow()
                    .browser_context_runtime()
                    .service_worker_runtime()
                    .enqueue_clients_open_window_completed(
                        crate::types::ServiceWorkerClientsOpenWindowCompletion {
                            request_id: completion_seed.request_id,
                            source_version_id: completion_seed.source_version_id,
                            source_run: completion_seed.source_run,
                            result: Ok(None),
                        },
                    );
                return Err(error);
            }
        };
        if !recorded {
            self._context_host
                .borrow()
                .browser_context_runtime()
                .service_worker_runtime()
                .enqueue_clients_open_window_completed(
                    crate::types::ServiceWorkerClientsOpenWindowCompletion {
                        request_id: completion_seed.request_id,
                        source_version_id: completion_seed.source_version_id,
                        source_run: completion_seed.source_run,
                        result: Ok(None),
                    },
                );
        }
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
        let Some(destination_url) = canonical_service_worker_auxiliary_url(&completion.url) else {
            return Ok(ServiceWorkerInternalBodyEffect::InternalActionApplied);
        };
        let host_scope = host_owner.dispatch_scope();
        let host_is_current = self
            ._context_host
            .borrow_mut()
            .service_worker_window_request_context(host_scope)
            .is_some();
        if !host_is_current {
            return Ok(ServiceWorkerInternalBodyEffect::ExactTargetUnavailable);
        }
        self.record_service_worker_auxiliary_navigation(
            host_scope,
            completion.source_script_url,
            destination_url,
            None,
        )?;
        Ok(ServiceWorkerInternalBodyEffect::InternalActionApplied)
    }
}
