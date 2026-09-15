use super::*;
use crate::content_security_policy::{
    ContentSecurityPolicyDisposition, ContentSecurityPolicyRedirectStatus,
    ContentSecurityPolicyResourceKind, ContentSecurityPolicySourceLocation,
    InheritedContentSecurityPolicy,
    content_security_policy_url_violation_for_checked_url_with_redirect_status_disposition_and_reporting_endpoints,
};
use moli_url::WebOrigin;

pub(super) struct WorkerImportScriptSource {
    pub(super) final_url: Url,
    pub(super) source: Arc<str>,
    pub(super) muted_errors: bool,
    resource: Option<crate::worker::WorkerScriptResource>,
}

pub(super) fn resolve_import_script_url(
    state: Rc<RefCell<WorkerGlobalState>>,
    input: &str,
) -> Result<Url, WorkerImportScriptError> {
    let (base_url, service_worker) = {
        let state = state.borrow();
        (
            state.current_script_url.clone(),
            matches!(
                state.global_kind,
                super::super::thread::WorkerGlobalKind::Service { .. }
            ),
        )
    };
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
    if !service_worker {
        url.set_fragment(None);
    }
    Ok(url)
}

pub(super) fn materialize_worker_import_source(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    script_url: &Url,
    blob_entry: Option<(String, String)>,
) -> Result<WorkerImportScriptSource, WorkerImportScriptError> {
    let csp = WorkerImportScriptCsp::capture(scope, state, script_url);
    if let Some(csp) = &csp {
        csp.check_url(script_url, ContentSecurityPolicyRedirectStatus::NoRedirect)
            .map_err(WorkerImportScriptError::network)?;
    }
    let (service_worker, cached, updated, can_import_new) = {
        let state = state.borrow();
        (
            matches!(
                state.global_kind,
                super::super::thread::WorkerGlobalKind::Service { .. }
            ),
            state
                .service_worker_script_resources
                .get(script_url)
                .cloned(),
            state
                .service_worker_updated_script_resources
                .get(script_url)
                .cloned(),
            state.service_worker_can_import_new_scripts,
        )
    };
    if service_worker {
        let from_update_check = cached.is_none() && updated.is_some();
        let cached = match cached.clone() {
            Some(resource) => Some(resource),
            None => updated
                .transpose()
                .map_err(WorkerImportScriptError::network)?,
        };
        if let Some(resource) = &cached
            && let Some(script) = &resource.classic_script
        {
            if let Some(csp) = &csp {
                for url in &script.redirect_urls {
                    csp.check_url(url, ContentSecurityPolicyRedirectStatus::FollowedRedirect)
                        .map_err(WorkerImportScriptError::network)?;
                }
            }
            if from_update_check {
                state
                    .borrow_mut()
                    .service_worker_script_resources
                    .insert(script_url.clone(), resource.clone());
                report_service_worker_imported_script_loaded(state, resource.clone());
            }
            return Ok(WorkerImportScriptSource {
                final_url: resource.final_url.clone(),
                source: script.source.clone(),
                muted_errors: script.muted_errors,
                resource: Some(resource.clone()),
            });
        }
        if !can_import_new && cached.is_none() {
            return Err(WorkerImportScriptError::network(format!(
                "Failed to execute 'importScripts': `{script_url}` was not imported during installation."
            )));
        }
    }
    // The resource map uses the complete request URL. Fragments are excluded
    // only when obtaining the response body from the network or a local URL.
    let mut fetch_url = script_url.clone();
    fetch_url.set_fragment(None);
    let mut source = match fetch_url.scheme() {
        "data" => {
            let source =
                decode_data_url_script_source(&fetch_url, "Failed to execute 'importScripts'")
                    .map_err(WorkerImportScriptError::network)?;
            let mime_type =
                moli_web_mime::data_url_mime_type(fetch_url.as_str()).ok_or_else(|| {
                    WorkerImportScriptError::network(format!(
                        "Failed to execute 'importScripts': invalid data URL `{script_url}`."
                    ))
                })?;
            ensure_worker_import_script_mime_acceptable(script_url, &mime_type, source.as_bytes())?;
            Ok(WorkerImportScriptSource {
                final_url: fetch_url.clone(),
                source: source.into(),
                muted_errors: false,
                resource: None,
            })
        }
        "blob" => {
            let (body, mime_type) = blob_entry.ok_or_else(|| {
                WorkerImportScriptError::network(format!(
                    "Failed to execute 'importScripts': blob URL `{}` is unavailable.",
                    script_url
                ))
            })?;
            ensure_worker_import_script_mime_acceptable(script_url, &mime_type, body.as_bytes())?;
            Ok(WorkerImportScriptSource {
                final_url: fetch_url.clone(),
                source: body.into(),
                muted_errors: false,
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
                fetch_url.clone(),
                initiator_url,
                referrer_policy,
                network_partition_key,
                policy_context,
                csp.map(|csp| {
                    moli_fetch::RequestRedirectCheck::new(move |checked_url| {
                        csp.check_url(
                            checked_url,
                            ContentSecurityPolicyRedirectStatus::FollowedRedirect,
                        )
                    })
                }),
            )
            .map_err(WorkerImportScriptError::network)?;
            Ok(source)
        }
        scheme => Err(WorkerImportScriptError::network(format!(
            "Failed to execute 'importScripts': URL scheme `{scheme}` is not allowed."
        ))),
    }?;
    if service_worker {
        let resource = source
            .resource
            .get_or_insert_with(|| crate::worker::WorkerScriptResource {
                request_url: script_url.clone(),
                final_url: source.final_url.clone(),
                kind: crate::worker::WorkerScriptResourceKind::JavaScript,
                status: 200,
                headers: vec![("Content-Type".into(), "text/javascript".into())],
                body_len: source.source.len(),
                body_sha256: moli_crypto::sha256_hex(source.source.as_bytes()),
                response_time_ms: 0,
                mime_type: Some("text/javascript".into()),
                classic_script: Some(crate::worker::WorkerStoredClassicScript {
                    source: source.source.clone(),
                    muted_errors: source.muted_errors,
                    redirect_urls: Vec::new(),
                }),
            });
        resource.request_url = script_url.clone();
        // Older profiles stored only hashes. Recover their content only when
        // the server still returns the installed bytes, never a new version.
        if cached.as_ref().is_some_and(|cached| {
            cached.body_sha256 != resource.body_sha256 || cached.final_url != resource.final_url
        }) {
            return Err(WorkerImportScriptError::network(format!(
                "Failed to execute 'importScripts': the installed script `{script_url}` is unavailable."
            )));
        }
        state
            .borrow_mut()
            .service_worker_script_resources
            .insert(script_url.clone(), resource.clone());
        report_service_worker_imported_script_loaded(state, resource.clone());
    }
    Ok(source)
}

// Redirect checks run on the network thread. Keep the caller's location and
// an owned policy snapshot, and send violations back to the Worker's task queue.
struct WorkerImportScriptCsp {
    policy: InheritedContentSecurityPolicy,
    protected_url: Url,
    request_url: Url,
    location: ContentSecurityPolicySourceLocation,
    wake_tx: tokio::sync::mpsc::UnboundedSender<WorkerMessage>,
}

impl WorkerImportScriptCsp {
    fn capture(
        scope: &mut v8::PinScope<'_, '_>,
        state: &Rc<RefCell<WorkerGlobalState>>,
        request_url: &Url,
    ) -> Option<Self> {
        let (policy, protected_url, wake_tx) = {
            let state = state.borrow();
            (
                content_security_policy::worker_policy_snapshot(&state),
                state.current_script_url.clone()?,
                state.worker_wake_tx.clone(),
            )
        };
        if policy.header_policies.is_empty()
            && policy.meta_policies.is_empty()
            && policy.report_only_policies.is_empty()
        {
            return None;
        }
        Some(Self {
            policy,
            protected_url,
            request_url: request_url.clone(),
            location: ContentSecurityPolicySourceLocation::capture(scope),
            wake_tx,
        })
    }

    fn check_url(
        &self,
        checked_url: &Url,
        redirect_status: ContentSecurityPolicyRedirectStatus,
    ) -> Result<(), String> {
        let mut failure = None;
        for disposition in [
            ContentSecurityPolicyDisposition::Report,
            ContentSecurityPolicyDisposition::Enforce,
        ] {
            for (policy, report_uri_enabled) in self.policy.policies(disposition) {
                let Some(violation) = content_security_policy_url_violation_for_checked_url_with_redirect_status_disposition_and_reporting_endpoints(
                    std::slice::from_ref(policy),
                    self.policy.self_url.as_ref().unwrap_or(&self.protected_url),
                    checked_url,
                    &self.request_url,
                    ContentSecurityPolicyResourceKind::WorkerScript,
                    redirect_status,
                    disposition,
                    &self.policy.reporting_endpoints,
                ) else {
                    continue;
                };
                let mut violation = content_security_policy::worker_policy_violation(
                    &self.protected_url,
                    report_uri_enabled,
                    violation,
                );
                self.location.apply_to(&mut violation);
                if disposition == ContentSecurityPolicyDisposition::Enforce {
                    failure.get_or_insert_with(|| {
                        worker_content_security_policy_error_message(&violation, "importScripts")
                    });
                }
                let _ = self
                    .wake_tx
                    .send(WorkerMessage::DispatchContentSecurityPolicyViolation(
                        Box::new(violation),
                    ));
            }
        }
        failure.map_or(Ok(()), Err)
    }
}

fn ensure_worker_import_script_mime_acceptable(
    script_url: &Url,
    mime_type: &str,
    body: &[u8],
) -> Result<(), WorkerImportScriptError> {
    let headers = [(
        "Content-Type".to_owned(),
        moli_fetch::header_value_from_byte_string(mime_type)
            .expect("serialized MIME types contain ByteStrings"),
    )];
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
    redirect_check: Option<moli_fetch::RequestRedirectCheck>,
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
    if let Some(redirect_check) = redirect_check {
        request = request.with_redirect_check(redirect_check);
    }
    let request_initiator_url = initiator_url.clone();
    if let Some(ref initiator_url) = request_initiator_url {
        request = request
            .with_initiator_url(initiator_url)
            .with_request_origin(moli_url::WebOrigin::from_url(initiator_url));
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
            validate_fetch_response_security_policy(
                WebOrigin::from_url(initiator_url),
                &response.head(),
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
    let mut resource = crate::worker::WorkerScriptResource::from_response_parts(
        request_url,
        &head,
        &body_bytes,
        response_time_ms,
    );
    let source: Arc<str> = body.into();
    resource.classic_script = Some(crate::worker::WorkerStoredClassicScript {
        source: source.clone(),
        muted_errors,
        redirect_urls: head
            .redirect_chain
            .iter()
            .map(|redirect| redirect.to_url.clone())
            .collect(),
    });
    Ok(WorkerImportScriptSource {
        final_url: head.final_url,
        source,
        muted_errors,
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
    let _ = crate::script_execution::execute_compiled_script(&mut scope, script);
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
    Ok(())
}

// ─── console ────────────────────────────────────────────────────────────────
