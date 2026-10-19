use super::*;
use crate::content_security_policy::{
    ContentSecurityPolicyDisposition, ContentSecurityPolicyRedirectStatus,
    ContentSecurityPolicyResourceKind,
    content_security_policy_url_violation_for_checked_url_with_redirect_status_disposition_and_reporting_endpoints,
    current_script_violation_location,
};
use crate::worker::WorkerMessage;
use moli_url::WebOrigin;

pub(super) struct WorkerImportScriptSource {
    pub(super) final_url: Url,
    pub(super) source: String,
    pub(super) muted_errors: bool,
    resource: Option<crate::worker::WorkerScriptResource>,
}

fn resolve_import_script_url(
    state: Rc<RefCell<WorkerGlobalState>>,
    input: &str,
) -> Result<Url, WorkerImportScriptError> {
    let base_url = state.borrow().current_script_url.clone();
    let url = Url::parse(input)
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
    Ok(url)
}

fn materialize_worker_import_source(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    script_url: &Url,
) -> Result<WorkerImportScriptSource, WorkerImportScriptError> {
    let csp = WorkerImportScriptCsp::capture(scope, state, script_url);
    if let Some(csp) = &csp {
        csp.check_url(script_url, ContentSecurityPolicyRedirectStatus::NoRedirect)
            .map_err(WorkerImportScriptError::network)?;
    }
    let timing = crate::context_bootstrap::FetchResourceTiming::for_initiator_type("other");
    let mut fetch_url = script_url.clone();
    fetch_url.set_fragment(None);
    match script_url.scheme() {
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
                final_url: script_url.clone(),
                source,
                muted_errors: false,
                resource: None,
            })
        }
        "blob" => {
            let (body, mime_type) = crate::blob::object_url_body_and_type(fetch_url.as_str())
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
            let fetched = fetch_worker_import_source_blocking(
                loader,
                WorkerImportScriptRequest {
                    script_url: script_url.clone(),
                    initiator_url,
                    origin: worker_global_origin(scope).unwrap_or(WebOrigin::Opaque),
                    referrer_policy,
                    network_partition_key,
                    policy_context,
                    timing,
                    redirect_check: csp.map(|csp| {
                        moli_fetch::RequestRedirectCheck::new(move |checked_url| {
                            csp.check_url(
                                checked_url,
                                ContentSecurityPolicyRedirectStatus::FollowedRedirect,
                            )
                        })
                    }),
                },
            )
            .map_err(WorkerImportScriptError::network)?;
            // Publish before evaluating the source, and before propagating
            // HTTP/MIME/network errors to the caller of importScripts().
            crate::context_bootstrap::record_resource_performance_entry(scope, fetched.timing);
            let source = fetched.script.map_err(WorkerImportScriptError::network)?;
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

// Redirect checks run on the network thread. Keep the caller's location and
// an owned policy snapshot, and send violations back to the Worker's task queue.
struct WorkerImportScriptCsp {
    enforced_policies: Vec<String>,
    report_only_policies: Vec<String>,
    reporting_endpoints: crate::content_security_policy::ContentSecurityPolicyReportingEndpoints,
    protected_url: Url,
    request_url: Url,
    location: Option<(String, i32, i32)>,
    wake_tx: tokio::sync::mpsc::UnboundedSender<WorkerMessage>,
}

impl WorkerImportScriptCsp {
    fn capture(
        scope: &mut v8::PinScope<'_, '_>,
        state: &Rc<RefCell<WorkerGlobalState>>,
        request_url: &Url,
    ) -> Option<Self> {
        let (enforced_policies, report_only_policies, reporting_endpoints, protected_url, wake_tx) = {
            let state = state.borrow();
            (
                state.content_security_policies.clone(),
                state.content_security_report_only_policies.clone(),
                state.content_security_reporting_endpoints.clone(),
                state.current_script_url.clone()?,
                state.worker_wake_tx.clone(),
            )
        };
        if enforced_policies.is_empty() && report_only_policies.is_empty() {
            return None;
        }
        Some(Self {
            enforced_policies,
            report_only_policies,
            reporting_endpoints,
            protected_url,
            request_url: request_url.clone(),
            location: current_script_violation_location(scope),
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
            let policies = match disposition {
                ContentSecurityPolicyDisposition::Enforce => &self.enforced_policies,
                ContentSecurityPolicyDisposition::Report => &self.report_only_policies,
            };
            for policy in policies {
                let Some(violation) = content_security_policy_url_violation_for_checked_url_with_redirect_status_disposition_and_reporting_endpoints(
                    std::slice::from_ref(policy),
                    &self.protected_url,
                    checked_url,
                    &self.request_url,
                    ContentSecurityPolicyResourceKind::WorkerScript,
                    redirect_status,
                    disposition,
                    &self.reporting_endpoints,
                ) else {
                    continue;
                };
                let mut violation = violation;
                violation.document_uri =
                    crate::content_security_policy::csp_url_for_report(&self.protected_url);
                violation.source_file =
                    crate::content_security_policy::csp_url_for_report(&self.protected_url);
                if let Some((source_file, line_number, column_number)) = &self.location {
                    violation.source_file = source_file.clone();
                    violation.line_number = *line_number;
                    violation.column_number = *column_number;
                }
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

struct WorkerImportScriptRequest {
    script_url: Url,
    initiator_url: Option<Url>,
    origin: WebOrigin,
    referrer_policy: Option<String>,
    network_partition_key: Option<String>,
    policy_context: crate::types::SubresourcePolicyContext,
    redirect_check: Option<moli_fetch::RequestRedirectCheck>,
    timing: crate::context_bootstrap::FetchResourceTiming,
}

struct WorkerImportScriptFetch {
    script: Result<WorkerImportScriptSource, String>,
    timing: crate::context_bootstrap::ResourcePerformanceEntry,
}

fn fetch_worker_import_source_blocking(
    loader: crate::network::context::WorkerResourceLoader,
    request: WorkerImportScriptRequest,
) -> Result<WorkerImportScriptFetch, String> {
    let WorkerImportScriptRequest {
        script_url,
        initiator_url,
        origin,
        referrer_policy,
        network_partition_key,
        policy_context,
        redirect_check,
        timing,
    } = request;
    let request_url = script_url.clone();
    let mut fetch_url = script_url.clone();
    fetch_url.set_fragment(None);
    let mut request = moli_fetch::Request::new("GET", fetch_url.as_str(), None, vec![])
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
    request = request.with_request_origin(origin.clone());
    if let Some(ref initiator_url) = initiator_url {
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
    let timing = Arc::new(parking_lot::Mutex::new(timing));
    let header_timing = timing.clone();
    let header_cancel = cancel_handle.clone();
    let response = loader
        .request_client()
        .fetch_text_for_worker_blocking_boundary_with_response_start(
            request,
            cancel_handle.clone(),
            move || {
                let mut timing = header_timing.lock();
                timing.response_started();
                timing.observe_native_timing(header_cancel.response_timing());
            },
        )
        .map_err(|error| format!("failed to fetch worker import `{script_url}`: {error}"));
    timing
        .lock()
        .observe_native_timing(cancel_handle.response_timing());
    load.finish();
    let response = match response {
        Ok(response) => response,
        Err(error) => {
            return Ok(WorkerImportScriptFetch {
                script: Err(error),
                timing: timing.lock().failure(&request_url),
            });
        }
    };
    let response_time_ms = response_started_at
        .elapsed()
        .as_millis()
        .min(u64::MAX as u128) as u64;
    // Classic imports use no-cors. Every cross-origin hop taints errors,
    // even when a later redirect returns to the worker origin.
    let head = response.head();
    let muted_errors = head.url_list().has_cross_origin_url(&origin);
    let security_result = validate_fetch_response_security_policy(
        origin.clone(),
        &head,
        RequestMode::NoCors,
        RequestCredentialsMode::SameOrigin,
        policy_context,
    );
    let performance_entry = if security_result.is_ok() {
        timing.lock().classic_script_response(
            &request_url,
            &origin,
            &head,
            response.body_bytes().len(),
        )
    } else {
        timing.lock().failure(&request_url)
    };
    let response_validation = security_result.and_then(|()| {
        moli_fetch::ensure_http_status_success(response.final_url.as_str(), response.status, false)
            .map_err(|error| error.to_string())?;
        crate::worker::ensure_worker_script_mime_acceptable(
            &response.final_url,
            &response.headers,
            response.body_bytes(),
        )
    });
    if let Err(error) = response_validation {
        let error = if muted_errors {
            format!(
                "Failed to execute 'importScripts': The script at '{script_url}' failed to load."
            )
        } else {
            error
        };
        return Ok(WorkerImportScriptFetch {
            script: Err(error),
            timing: performance_entry,
        });
    }
    let (head, body, body_bytes) = response.into_parts();
    let resource = crate::worker::WorkerScriptResource::from_response_parts(
        request_url,
        &head,
        &body_bytes,
        response_time_ms,
    );
    Ok(WorkerImportScriptFetch {
        script: Ok(WorkerImportScriptSource {
            final_url: head.final_url,
            source: body,
            muted_errors,
            resource: Some(resource),
        }),
        timing: performance_entry,
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

fn evaluate_worker_script(
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

pub(in crate::worker) const WORKER_EXCEPTION_SOURCE_SLOT: &str = "__moliWorkerExceptionSource";

pub(in crate::worker) const WORKER_EXCEPTION_LINE_SLOT: &str = "__moliWorkerExceptionLine";

pub(in crate::worker) const WORKER_EXCEPTION_COLUMN_SLOT: &str = "__moliWorkerExceptionColumn";

enum WorkerImportScriptError {
    DomException { name: &'static str, message: String },
    Exception(v8::Global<v8::Value>),
}

impl WorkerImportScriptError {
    fn syntax(message: impl Into<String>) -> Self {
        Self::DomException {
            name: "SyntaxError",
            message: message.into(),
        }
    }

    fn network(message: impl Into<String>) -> Self {
        Self::DomException {
            name: "NetworkError",
            message: message.into(),
        }
    }

    fn error<'s>(scope: &mut v8::PinScope<'s, '_>, message: impl Into<String>) -> Self {
        let message = message.into();
        let value = v8_string(scope, &message)
            .map(|value| v8::Exception::error(scope, value))
            .unwrap_or_else(|| v8::Exception::error(scope, v8::String::empty(scope)));
        Self::Exception(v8::Global::new(scope, value))
    }

    fn throw(self, scope: &mut v8::PinScope<'_, '_>) {
        match self {
            Self::DomException { name, message } => {
                let exception = worker_dom_exception_value(scope, &message, name);
                scope.throw_exception(exception);
            }
            Self::Exception(value) => {
                let value = v8::Local::new(scope, value);
                scope.throw_exception(value);
            }
        }
    }
}

struct PreparedWorkerImportScript {
    final_url: Url,
    source: Option<String>,
    muted_errors: bool,
}

fn annotate_worker_exception_location<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    exception: v8::Local<'s, v8::Value>,
    message: Option<v8::Local<'s, v8::Message>>,
) {
    let Ok(object) = v8::Local::<v8::Object>::try_from(exception) else {
        return;
    };
    let source = message
        .and_then(|message| message.get_script_resource_name(scope))
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .filter(|value| !value.is_empty());
    if let Some(source) = source
        && let Some(value) = v8_string(scope, &source)
    {
        set_worker_exception_location_if_missing(
            scope,
            object,
            WORKER_EXCEPTION_SOURCE_SLOT,
            value.into(),
        );
    }
    if let Some(line) = message.and_then(|message| message.get_line_number(scope)) {
        let value = v8::Number::new(scope, line as f64);
        set_worker_exception_location_if_missing(
            scope,
            object,
            WORKER_EXCEPTION_LINE_SLOT,
            value.into(),
        );
    }
    if let Some(column) = message.and_then(|message| message.get_start_column().checked_add(1)) {
        let value = v8::Number::new(scope, column as f64);
        set_worker_exception_location_if_missing(
            scope,
            object,
            WORKER_EXCEPTION_COLUMN_SLOT,
            value.into(),
        );
    }
}

fn set_worker_exception_location_if_missing<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &str,
    value: v8::Local<'s, v8::Value>,
) {
    if get_private_value(scope, object, slot).is_none() {
        set_private_value(scope, object, slot, value);
    }
}

pub(super) fn worker_import_scripts_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(state) = get_worker_state(scope) else {
        return;
    };
    if state.borrow().script_kind == crate::worker::thread::WorkerScriptKind::Module {
        throw_type_error(scope, "Module scripts don't support importScripts().");
        return;
    }
    let require_trusted_types_for_script =
        crate::content_security_policy::content_security_policy_requires_trusted_types_for_script(
            &state.borrow().content_security_policies,
        );
    let mut prepared = Vec::with_capacity(args.length() as usize);
    for i in 0..args.length() {
        let Some(specifier) = crate::context_bootstrap::trusted_script_url_string_or_throw(
            scope,
            args.get(i),
            crate::content_security_policy::TrustedTypesForScriptRequirements::enforced_only(
                require_trusted_types_for_script,
            ),
            "WorkerGlobalScope importScripts",
            "importScripts",
        ) else {
            return;
        };
        let resolved_url = match resolve_import_script_url(state.clone(), &specifier) {
            Ok(url) => url,
            Err(error) => {
                error.throw(scope);
                return;
            }
        };
        let source = if matches!(resolved_url.scheme(), "data" | "blob") {
            match materialize_worker_import_source(scope, &state, &resolved_url) {
                Ok(import_source) => Some(import_source.source),
                Err(error) => {
                    error.throw(scope);
                    return;
                }
            }
        } else {
            None
        };
        prepared.push(PreparedWorkerImportScript {
            final_url: resolved_url,
            source,
            muted_errors: false,
        });
    }
    for mut script in prepared {
        let request_url = script.final_url.clone();
        if script.source.is_none() {
            let import_source =
                match materialize_worker_import_source(scope, &state, &script.final_url) {
                    Ok(result) => result,
                    Err(error) => {
                        error.throw(scope);
                        return;
                    }
                };
            script.final_url = import_source.final_url;
            script.source = Some(import_source.source);
            script.muted_errors = import_source.muted_errors;
        }
        let source = script.source.as_deref().unwrap_or_default();
        if let Err(error) = evaluate_worker_script(
            scope,
            &request_url,
            &script.final_url,
            source,
            script.muted_errors,
        ) {
            error.throw(scope);
            return;
        }
    }
}
