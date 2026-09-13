use super::*;
use crate::network::{ResourceResponseFailure, ResourceResponseResult};
use crate::worker::network_transfer::WorkerResourceTransfer;

pub(super) struct WorkerImportScriptSource {
    pub(super) final_url: Url,
    pub(super) source: String,
    pub(super) muted_errors: bool,
    resource: Option<crate::worker::WorkerScriptResource>,
}

pub(super) fn resolve_import_script_url(
    state: Rc<RefCell<WorkerGlobalState>>,
    input: &str,
) -> Result<Url, WorkerImportScriptError> {
    let base_url = state.borrow().current_script_url.clone();
    let mut url = Url::parse(input)
        .or_else(|_| {
            base_url
                .as_ref()
                .ok_or(url::ParseError::RelativeUrlWithoutBase)
                .and_then(|base| base.join(input))
        })
        .map_err(|_| {
            WorkerImportScriptError::syntax(format!(
                "Failed to execute 'importScripts': invalid URL `{input}`."
            ))
        })?;
    match url.scheme() {
        "http" | "https" | "data" | "blob" => {}
        scheme => {
            return Err(WorkerImportScriptError::network(format!(
                "Failed to execute 'importScripts': URL scheme `{scheme}` is not allowed."
            )));
        }
    }
    url.set_fragment(None);
    Ok(url)
}

pub(super) fn materialize_worker_import_source(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    script_url: &Url,
) -> Result<WorkerImportScriptSource, WorkerImportScriptError> {
    let network = {
        let state = state.borrow();
        WorkerResourceTransfer::start_script(
            state.global_kind.network(),
            state.parent_tx.network_observer(),
            script_url,
            state.current_script_url.as_ref().unwrap_or(script_url),
        )
        .ok_or_else(|| WorkerImportScriptError::network("worker is shutting down"))?
    };
    let result = (|| {
        if let Some(violation) = {
            let state = state.borrow();
            state.current_script_url.as_ref().and_then(|protected_url| {
                worker_content_security_policy_report_only_violation(
                    &state,
                    protected_url,
                    script_url,
                    crate::content_security_policy::ContentSecurityPolicyResourceKind::WorkerScript,
                )
            })
        } {
            dispatch_worker_content_security_policy_violation_event_for_state(
                scope, state, &violation,
            );
        }
        if let Some(violation) = {
            let state = state.borrow();
            state.current_script_url.as_ref().and_then(|protected_url| {
                worker_content_security_policy_violation(
                    &state,
                    protected_url,
                    script_url,
                    crate::content_security_policy::ContentSecurityPolicyResourceKind::WorkerScript,
                )
            })
        } {
            dispatch_worker_content_security_policy_violation_event_for_state(
                scope, state, &violation,
            );
            let message = worker_content_security_policy_error_message(&violation, "importScripts");
            return Err(WorkerImportScriptError::network(message));
        }
        let response = match script_url.scheme() {
            "data" => {
                // Keep importScripts' strict data-URL UTF-8/error behavior. The
                // shared local resolver also retains the exact bytes for observers.
                decode_data_url_script_source(script_url, "Failed to execute 'importScripts'")
                    .map_err(WorkerImportScriptError::network)?;
                crate::network_host::local_url_response(script_url).ok_or_else(|| {
                    WorkerImportScriptError::network(format!(
                        "Failed to execute 'importScripts': invalid data URL `{script_url}`."
                    ))
                })?
            }
            "blob" => crate::network_host::local_url_response(script_url).ok_or_else(|| {
                WorkerImportScriptError::network(format!(
                    "Failed to execute 'importScripts': blob URL `{}` is unavailable.",
                    script_url
                ))
            })?,
            "http" | "https" => {
                let (loader, initiator_url, referrer_policy, network_partition_key) = {
                    let state = state.borrow();
                    (
                        state.loader.clone(),
                        state.current_script_url.clone(),
                        state.referrer_policy.clone(),
                        state.network_partition_key.clone(),
                    )
                };
                return fetch_worker_import_source_blocking(
                    loader,
                    script_url.clone(),
                    initiator_url,
                    referrer_policy,
                    network_partition_key,
                    &network,
                )
                .inspect(|source| {
                    if let Some(resource) = source.resource.clone() {
                        report_service_worker_imported_script_loaded(state, resource);
                    }
                })
                .map_err(WorkerImportScriptError::network);
            }
            scheme => {
                return Err(WorkerImportScriptError::network(format!(
                    "Failed to execute 'importScripts': URL scheme `{scheme}` is not allowed."
                )));
            }
        };
        crate::worker::ensure_worker_script_mime_acceptable(
            script_url,
            &response.headers,
            response.body_text().as_bytes(),
        )
        .map_err(WorkerImportScriptError::network)?;
        network.response_completed(&response);
        let (head, source, _) = response.into_parts();
        Ok(WorkerImportScriptSource {
            final_url: head.final_url,
            source,
            muted_errors: false,
            resource: None,
        })
    })();
    result.inspect_err(|error| {
        if let WorkerImportScriptError::DomException { message, .. } = error {
            network.failed(&ResourceResponseFailure::Request(message.clone()));
        }
    })
}

pub(super) fn fetch_worker_import_source_blocking(
    loader: crate::network::context::WorkerResourceLoader,
    script_url: Url,
    initiator_url: Option<Url>,
    referrer_policy: Option<String>,
    network_partition_key: Option<String>,
    network: &Arc<WorkerResourceTransfer>,
) -> Result<WorkerImportScriptSource, String> {
    let request_url = script_url.clone();
    let mut request = moli_fetch::Request::new("GET", script_url.as_str(), None, vec![])
        .map_err(|error| error.to_string())?
        .with_page_network_policy()
        .with_network_partition_key(network_partition_key);
    let request_initiator_url = initiator_url.clone();
    if let Some(ref initiator_url) = request_initiator_url {
        request = request
            .with_initiator_url(initiator_url)
            .with_request_origin(moli_url::WebOrigin::from_url(initiator_url));
    }
    if let Some(referrer_policy) = referrer_policy {
        request = request.with_script_fetch_metadata(moli_fetch::ScriptFetchRequestMetadata {
            document_referrer_policy: Some(referrer_policy),
            ..moli_fetch::ScriptFetchRequestMetadata::default()
        });
    }
    let response_started_at = Instant::now();
    let load = loader
        .register_load(
            ResourceLoadKind::Script,
            ResourceLoadDisposition::Ordinary,
            None,
        )
        .ok_or_else(|| "worker is shutting down".to_owned())?;
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    let callback_network = network.clone();
    load.request_client()
        .fetch_script_text_callback_with_load(
            request,
            load,
            network.clone(),
            move |result: ResourceResponseResult| {
                if let Err(error) = send.send(result) {
                    callback_network.complete(&error.0);
                }
            },
        )
        .map_err(|error| format!("failed to fetch worker import `{script_url}`: {error}"))?;
    let response = receive
        .recv()
        .map_err(|_| "worker import producer stopped".to_owned())?
        .inspect_err(|error| network.failed(error))
        .map_err(|error| format!("failed to fetch worker import `{script_url}`: {error}"))?;
    let response_time_ms = response_started_at
        .elapsed()
        .as_millis()
        .min(u64::MAX as u128) as u64;
    moli_fetch::ensure_http_status_success(response.final_url.as_str(), response.status, false)
        .map_err(|error| error.to_string())?;
    crate::worker::ensure_worker_script_mime_acceptable(
        &response.final_url,
        &response.headers,
        response.body_bytes(),
    )?;
    if request_initiator_url.as_ref().is_some_and(|initiator_url| {
        matches!(initiator_url.scheme(), "http" | "https")
            && !moli_url::same_origin(initiator_url, &response.final_url)
    }) {
        return Err(format!(
            "Failed to execute 'importScripts' on 'WorkerGlobalScope': The script at '{}' failed to load.",
            response.final_url
        ));
    }
    network.response_completed(&response);
    let (head, body, body_bytes) = response.into_parts();
    let resource = crate::worker::WorkerScriptResource::from_response_parts(
        request_url,
        &head,
        &body_bytes,
        response_time_ms,
    );
    Ok(WorkerImportScriptSource {
        final_url: head.final_url,
        source: body,
        muted_errors: false,
        resource: Some(resource),
    })
}

fn report_service_worker_imported_script_loaded(
    state: &Rc<RefCell<WorkerGlobalState>>,
    resource: crate::worker::WorkerScriptResource,
) {
    let state = state.borrow();
    let WorkerGlobalKind::Service {
        registration_id,
        version_id,
        ..
    } = &state.global_kind
    else {
        return;
    };
    let _ = state
        .parent_tx
        .send(WorkerToParentMessage::ServiceWorkerImportedScriptLoaded {
            registration_id: *registration_id,
            version_id: *version_id,
            resource,
        });
}

pub(super) fn evaluate_worker_script(
    scope: &mut v8::PinScope<'_, '_>,
    state: Rc<RefCell<WorkerGlobalState>>,
    script_url: &Url,
    script_source: &str,
    muted_errors: bool,
) -> Result<(), WorkerImportScriptError> {
    let previous_url = {
        let mut state = state.borrow_mut();
        state.current_script_url.replace(script_url.clone())
    };
    let outcome = (|| {
        let source = v8::String::new(scope, script_source).ok_or_else(|| {
            WorkerImportScriptError::error(
                scope,
                format!("failed to allocate worker source for `{script_url}`"),
            )
        })?;
        let origin = create_script_origin(scope, script_url.as_str());
        let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
        let mut scope = try_catch.init();
        let Some(script) = v8::Script::compile(&scope, source, Some(&origin)) else {
            if muted_errors {
                return Err(WorkerImportScriptError::network(format!(
                    "Failed to execute 'importScripts' on 'WorkerGlobalScope': The script at '{script_url}' failed to load."
                )));
            }
            let error = scope
                .exception()
                .map(|value| {
                    let message = scope.message();
                    annotate_worker_exception_location(&mut scope, value, message);
                    WorkerImportScriptError::Exception(v8::Global::new(&scope, value))
                })
                .unwrap_or_else(|| {
                    WorkerImportScriptError::error(
                        &mut scope,
                        format!("failed to compile `{script_url}`"),
                    )
                });
            return Err(error);
        };
        let _ = crate::script_execution::execute_compiled_script(&mut scope, script);
        if scope.has_caught() {
            if muted_errors {
                return Err(WorkerImportScriptError::network(format!(
                    "Failed to execute 'importScripts' on 'WorkerGlobalScope': The script at '{script_url}' failed to load."
                )));
            }
            let error = scope
                .exception()
                .map(|value| {
                    let message = scope.message();
                    annotate_worker_exception_location(&mut scope, value, message);
                    WorkerImportScriptError::Exception(v8::Global::new(&scope, value))
                })
                .unwrap_or_else(|| {
                    WorkerImportScriptError::error(
                        &mut scope,
                        format!("failed to execute `{script_url}`"),
                    )
                });
            return Err(error);
        }
        scope.perform_microtask_checkpoint();
        crate::context_bootstrap::run_end_of_microtask_checkpoint_tasks(&mut scope);
        Ok(())
    })();
    state.borrow_mut().current_script_url = previous_url;
    outcome
}

// ─── console ────────────────────────────────────────────────────────────────
