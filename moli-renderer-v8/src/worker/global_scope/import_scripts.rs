use super::*;
use crate::content_security_policy::{
    ContentSecurityPolicyRedirectStatus, ContentSecurityPolicyResourceKind,
};
use moli_url::WebOrigin;

pub(super) struct WorkerImportScriptSource {
    pub(super) final_url: Url,
    pub(super) source: String,
    pub(super) muted_errors: bool,
    redirect_urls: Vec<Url>,
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
    check_import_script_csp(
        scope,
        state,
        script_url,
        script_url,
        ContentSecurityPolicyRedirectStatus::NoRedirect,
    )?;
    match script_url.scheme() {
        "data" => {
            let source =
                decode_data_url_script_source(script_url, "Failed to execute 'importScripts'")
                    .map_err(WorkerImportScriptError::network)?;
            let mime_type =
                moli_web_mime::data_url_mime_type(script_url.as_str()).ok_or_else(|| {
                    WorkerImportScriptError::network(format!(
                        "Failed to execute 'importScripts': invalid data URL `{script_url}`."
                    ))
                })?;
            ensure_worker_import_script_mime_acceptable(script_url, &mime_type, source.as_bytes())?;
            Ok(WorkerImportScriptSource {
                final_url: script_url.clone(),
                source,
                muted_errors: false,
                redirect_urls: Vec::new(),
                resource: None,
            })
        }
        "blob" => {
            let (body, mime_type) = crate::blob::object_url_body_and_type(script_url.as_str())
                .ok_or_else(|| {
                    WorkerImportScriptError::network(format!(
                        "Failed to execute 'importScripts': blob URL `{}` is unavailable.",
                        script_url
                    ))
                })?;
            ensure_worker_import_script_mime_acceptable(script_url, &mime_type, body.as_bytes())?;
            Ok(WorkerImportScriptSource {
                final_url: script_url.clone(),
                source: body,
                muted_errors: false,
                redirect_urls: Vec::new(),
                resource: None,
            })
        }
        "http" | "https" => {
            let (loader, initiator_url, referrer_policy, network_partition_key, policy_context) = {
                let state = state.borrow();
                (
                    state.loader.clone(),
                    state.current_script_url.clone(),
                    state.referrer_policy.clone(),
                    state.network_partition_key.clone(),
                    state.policy_context,
                )
            };
            let source = fetch_worker_import_source_blocking(
                loader,
                script_url.clone(),
                initiator_url,
                referrer_policy,
                network_partition_key,
                policy_context,
            )
            .map_err(WorkerImportScriptError::network)?;
            for checked_url in &source.redirect_urls {
                check_import_script_csp(
                    scope,
                    state,
                    script_url,
                    checked_url,
                    ContentSecurityPolicyRedirectStatus::FollowedRedirect,
                )?;
            }
            if let Some(resource) = source.resource.clone() {
                report_service_worker_imported_script_loaded(state, resource);
            }
            Ok(source)
        }
        scheme => Err(WorkerImportScriptError::network(format!(
            "Failed to execute 'importScripts': URL scheme `{scheme}` is not allowed."
        ))),
    }
}

fn check_import_script_csp(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    request_url: &Url,
    checked_url: &Url,
    redirect_status: ContentSecurityPolicyRedirectStatus,
) -> Result<(), WorkerImportScriptError> {
    let (report, enforce) = {
        let state = state.borrow();
        let Some(protected_url) = state.current_script_url.as_ref() else {
            return Ok(());
        };
        (
            worker_content_security_policy_report_only_violation_for_checked_url_with_redirect_status(
                &state, protected_url, checked_url, request_url, ContentSecurityPolicyResourceKind::WorkerScript, redirect_status,
            ),
            worker_content_security_policy_violation_for_checked_url_with_redirect_status(
                &state, protected_url, checked_url, request_url, ContentSecurityPolicyResourceKind::WorkerScript, redirect_status,
            ),
        )
    };
    if let Some(violation) = report {
        dispatch_worker_content_security_policy_violation_event_for_state(scope, state, &violation);
    }
    if let Some(violation) = enforce {
        dispatch_worker_content_security_policy_violation_event_for_state(scope, state, &violation);
        return Err(WorkerImportScriptError::network(
            worker_content_security_policy_error_message(&violation, "importScripts"),
        ));
    }
    Ok(())
}

fn ensure_worker_import_script_mime_acceptable(
    script_url: &Url,
    mime_type: &str,
    body: &[u8],
) -> Result<(), WorkerImportScriptError> {
    let headers = [("Content-Type".to_owned(), mime_type.to_owned())];
    crate::worker::ensure_worker_script_mime_acceptable(script_url, &headers, body)
        .map_err(WorkerImportScriptError::network)
}

pub(super) fn fetch_worker_import_source_blocking(
    loader: crate::network::context::WorkerResourceLoader,
    script_url: Url,
    initiator_url: Option<Url>,
    referrer_policy: Option<String>,
    network_partition_key: Option<String>,
    policy_context: crate::types::SubresourcePolicyContext,
) -> Result<WorkerImportScriptSource, String> {
    let request_url = script_url.clone();
    let mut request = moli_fetch::Request::new("GET", script_url.as_str(), None, vec![])
        .map_err(|error| error.to_string())?
        .with_page_network_policy()
        .with_request_mode(RequestMode::NoCors)
        .with_credentials_mode(RequestCredentialsMode::SameOrigin)
        .with_script_fetch_metadata(moli_fetch::ScriptFetchRequestMetadata {
            document_referrer_policy: referrer_policy,
            ..moli_fetch::ScriptFetchRequestMetadata::default()
        })
        .with_network_partition_key(network_partition_key);
    let request_initiator_url = initiator_url.clone();
    if let Some(ref initiator_url) = request_initiator_url {
        request = request.with_initiator_url(initiator_url);
    }
    let response_started_at = Instant::now();
    let cancel_handle = FetchCancelHandle::new();
    let load = loader
        .register_load(
            ResourceLoadKind::Script,
            ResourceLoadDisposition::Ordinary,
            Some(cancel_handle.clone()),
        )
        .ok_or_else(|| "worker is shutting down".to_owned())?;
    let response = loader
        .request_client()
        .fetch_text_for_worker_blocking_boundary_with_cancel(request, cancel_handle)
        .map_err(|error| format!("failed to fetch worker import `{script_url}`: {error}"));
    load.finish();
    let response = response?;
    let response_time_ms = response_started_at
        .elapsed()
        .as_millis()
        .min(u64::MAX as u128) as u64;
    // Classic imported scripts use no-cors, unlike the worker's top-level
    // same-origin fetch and module CORS fetches. Any cross-origin response in
    // the URL chain taints the result, even if it redirects back to the worker.
    let muted_errors = request_initiator_url.as_ref().is_some_and(|initiator_url| {
        !moli_url::same_origin(initiator_url, &request_url)
            || !moli_url::same_origin(initiator_url, &response.final_url)
            || response.redirect_chain.iter().any(|redirect| {
                !moli_url::same_origin(initiator_url, &redirect.from_url)
                    || !moli_url::same_origin(initiator_url, &redirect.to_url)
            })
    });
    let response_validation = (|| {
        moli_fetch::ensure_http_status_success(response.final_url.as_str(), response.status, false)
            .map_err(|error| error.to_string())?;
        crate::worker::ensure_worker_script_mime_acceptable(
            &response.final_url,
            &response.headers,
            response.body_bytes(),
        )?;
        if let Some(initiator_url) = &request_initiator_url {
            validate_fetch_response_security_policy_for_origin(
                initiator_url,
                &WebOrigin::from_url(initiator_url),
                &response.final_url,
                &response.headers,
                RequestMode::NoCors,
                RequestCredentialsMode::SameOrigin,
                policy_context,
            )?;
        }
        Ok::<_, String>(())
    })();
    response_validation.map_err(|error| {
        if muted_errors {
            format!(
                "Failed to execute 'importScripts': The script at '{script_url}' failed to load."
            )
        } else {
            error
        }
    })?;
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
        muted_errors,
        redirect_urls: head
            .redirect_chain
            .into_iter()
            .map(|redirect| redirect.to_url)
            .collect(),
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
    request_url: &Url,
    script_url: &Url,
    script_source: &str,
    muted_errors: bool,
) -> Result<(), WorkerImportScriptError> {
    let source = v8::String::new(scope, script_source).ok_or_else(|| {
        WorkerImportScriptError::error(
            scope,
            format!("failed to allocate worker source for `{script_url}`"),
        )
    })?;
    let name = v8::String::new(scope, script_url.as_str()).expect("worker script origin");
    let sanitized_base = Url::parse("about:blank").expect("valid sanitized script base");
    let base_url = if muted_errors {
        &sanitized_base
    } else {
        script_url
    };
    let host_defined_options = crate::util::script_host_defined_options_with_fetch_metadata(
        scope,
        base_url,
        None,
        false,
        muted_errors,
        Some(request_url),
    );
    let origin = v8::ScriptOrigin::new(
        scope,
        name.into(),
        0,
        0,
        false,
        -1,
        None,
        muted_errors,
        false,
        false,
        host_defined_options,
    );
    let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
    let mut scope = try_catch.init();
    let Some(script) = v8::Script::compile(&scope, source, Some(&origin)) else {
        if muted_errors {
            return Err(WorkerImportScriptError::network(
                "Failed to execute 'importScripts': A cross-origin script failed to execute."
                    .to_owned(),
            ));
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
    let _ = script.run(&scope);
    if scope.has_caught() {
        if muted_errors {
            return Err(WorkerImportScriptError::network(
                "Failed to execute 'importScripts': A cross-origin script failed to execute."
                    .to_owned(),
            ));
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
}

// ─── console ────────────────────────────────────────────────────────────────
