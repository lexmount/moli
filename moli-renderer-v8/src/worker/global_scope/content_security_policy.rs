use std::cell::RefCell;
use std::rc::Rc;

use url::Url;

use crate::content_security_policy::{
    ContentSecurityPolicy, ContentSecurityPolicyDisposition, ContentSecurityPolicyNonUrlKind,
    ContentSecurityPolicyRedirectStatus, ContentSecurityPolicyResourceKind,
    ContentSecurityPolicyUrlViolation, ContentSecurityPolicyViolationEventFields,
    content_security_policy_non_url_violation_with_source,
    content_security_policy_trusted_types_policy_violation_with_disposition_and_reporting_endpoints,
    content_security_policy_trusted_types_sink_violation_with_disposition_and_reporting_endpoints,
    content_security_policy_url_violation_for_checked_url_with_redirect_status_disposition_and_reporting_endpoints,
    create_security_policy_violation_event, current_script_violation_location,
    send_content_security_policy_reports,
};
use crate::context_bootstrap::dispatch_simple_event_target_event;
use crate::network::loads::{ResourceLoadDisposition, ResourceLoadKind};
use crate::protocol_types::PendingSubresourceFetchInfo;
use crate::service_worker_runtime::service_worker_fetch_request_metadata;
use crate::types::SubresourceResourceType;
use moli_fetch::{BrowserRequestMetadata, Request};
use crate::worker::WorkerMessage;

use super::{WorkerGlobalState, get_worker_state, request_body_text};

pub(super) fn worker_policy_snapshot(
    state: &WorkerGlobalState,
) -> crate::content_security_policy::InheritedContentSecurityPolicy {
    state
        .content_security_policy_snapshot
        .clone()
        .unwrap_or_else(
            || crate::content_security_policy::InheritedContentSecurityPolicy {
                self_url: state.current_script_url.clone(),
                header_policies: state.content_security_policies.clone(),
                meta_policies: Vec::new(),
                report_only_policies: state.content_security_report_only_policies.clone(),
                reporting_endpoints: state.content_security_reporting_endpoints.clone(),
            },
        )
}

fn worker_policy_url<'a>(state: &'a WorkerGlobalState, protected_url: &'a Url) -> &'a Url {
    state
        .content_security_policy_snapshot
        .as_ref()
        .and_then(|policy| policy.self_url.as_ref())
        .unwrap_or(protected_url)
}

pub(super) fn worker_policy_violation(
    protected_url: &Url,
    report_uri_enabled: bool,
    mut violation: ContentSecurityPolicyUrlViolation,
) -> ContentSecurityPolicyUrlViolation {
    violation.document_uri = crate::content_security_policy::csp_url_for_report(protected_url);
    violation.source_file = crate::content_security_policy::csp_url_for_report(protected_url);
    if !report_uri_enabled {
        violation.report_uri_endpoints.clear();
    }
    violation
}

fn worker_policies(
    state: &WorkerGlobalState,
    disposition: ContentSecurityPolicyDisposition,
) -> impl Iterator<Item = (&ContentSecurityPolicy, bool)> {
    let (headers, meta) = match (state.content_security_policy_snapshot.as_ref(), disposition) {
        (Some(policy), ContentSecurityPolicyDisposition::Enforce) => (
            policy.header_policies.as_slice(),
            policy.meta_policies.as_slice(),
        ),
        (Some(policy), ContentSecurityPolicyDisposition::Report) => {
            (policy.report_only_policies.as_slice(), &[][..])
        }
        (None, ContentSecurityPolicyDisposition::Enforce) => {
            (state.content_security_policies.as_slice(), &[][..])
        }
        (None, ContentSecurityPolicyDisposition::Report) => (
            state.content_security_report_only_policies.as_slice(),
            &[][..],
        ),
    };
    headers
        .iter()
        .map(|policy| (policy, true))
        .chain(meta.iter().map(|policy| (policy, false)))
}

pub(super) fn dispatch_worker_content_security_policy_violation_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    request_client: &crate::network::context::WorkerResourceLoader,
    violation: &ContentSecurityPolicyUrlViolation,
) {
    let global = scope.get_current_context().global(scope);
    let Some(event) = create_worker_content_security_policy_violation_event(scope, violation)
    else {
        return;
    };
    let fields = ContentSecurityPolicyViolationEventFields::from_url_violation(violation);
    send_content_security_policy_reports(
        request_client.content_security_policy_reports(),
        request_client.request_client(),
        moli_url::WebOrigin::from_serialized(&violation.document_uri),
        &fields,
        &violation.report_uri_endpoints,
        &violation.report_to_endpoints,
    );
    dispatch_simple_event_target_event(scope, global, "securitypolicyviolation", event);
}

pub(super) fn dispatch_worker_content_security_policy_violation_event_for_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    violation: &ContentSecurityPolicyUrlViolation,
) {
    let global = scope.get_current_context().global(scope);
    let Some(event) = create_worker_content_security_policy_violation_event(scope, violation)
    else {
        return;
    };
    send_worker_content_security_policy_reports_for_state(state, violation);
    dispatch_simple_event_target_event(scope, global, "securitypolicyviolation", event);
}

pub(super) fn dispatch_worker_trusted_types_sink_violation_event_for_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    sink: &str,
    sample: &str,
) {
    let violations = {
        let state_ref = state.borrow();
        let Some(protected_url) = state_ref.current_script_url.as_ref() else {
            return;
        };
        trusted_types_policies(&state_ref)
            .filter_map(|(policy, disposition, report_uri_enabled)| {
                content_security_policy_trusted_types_sink_violation_with_disposition_and_reporting_endpoints(
                    policy,
                    worker_policy_url(&state_ref, protected_url),
                    sink,
                    sample,
                    disposition,
                    &state_ref.content_security_reporting_endpoints,
                ).map(|violation| worker_policy_violation(protected_url, report_uri_enabled, violation))
            })
            .collect()
    };
    queue_worker_trusted_types_violations(scope, state, violations);
}

pub(super) fn allows_worker_trusted_type_policy_name_for_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    policy_name: &str,
    is_duplicate: bool,
) -> bool {
    let violations: Vec<_> = {
        let state_ref = state.borrow();
        let Some(protected_url) = state_ref.current_script_url.as_ref() else {
            return true;
        };
        trusted_types_policies(&state_ref)
            .filter_map(|(policy, disposition, report_uri_enabled)| {
                content_security_policy_trusted_types_policy_violation_with_disposition_and_reporting_endpoints(
                    policy,
                    worker_policy_url(&state_ref, protected_url),
                    policy_name,
                    is_duplicate,
                    disposition,
                    &state_ref.content_security_reporting_endpoints,
                ).map(|violation| worker_policy_violation(protected_url, report_uri_enabled, violation))
            })
            .collect()
    };
    let allowed = !violations
        .iter()
        .any(|violation| violation.disposition == ContentSecurityPolicyDisposition::Enforce);
    queue_worker_trusted_types_violations(scope, state, violations);
    allowed
}

fn trusted_types_policies(
    state: &WorkerGlobalState,
) -> impl Iterator<
    Item = (
        &ContentSecurityPolicy,
        ContentSecurityPolicyDisposition,
        bool,
    ),
> {
    [
        ContentSecurityPolicyDisposition::Enforce,
        ContentSecurityPolicyDisposition::Report,
    ]
    .into_iter()
    .flat_map(move |disposition| {
        worker_policies(state, disposition)
            .map(move |(policy, report_uri_enabled)| (policy, disposition, report_uri_enabled))
    })
}

fn queue_worker_trusted_types_violations(
    scope: &mut v8::PinScope<'_, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    violations: Vec<ContentSecurityPolicyUrlViolation>,
) {
    if violations.is_empty() {
        return;
    }
    // Capture the caller before returning from the sink, but deliver events
    // in a later task. In particular, a listener must not reenter createPolicy
    // before the original call has recorded the newly created policy name.
    let location = current_script_violation_location(scope);
    let wake_tx = state.borrow().worker_wake_tx.clone();
    for mut violation in violations {
        if let Some((source_file, line_number, column_number)) = &location {
            violation.source_file.clone_from(source_file);
            violation.line_number = *line_number;
            violation.column_number = *column_number;
        }
        let _ = wake_tx.send(WorkerMessage::DispatchContentSecurityPolicyViolation(
            Box::new(violation),
        ));
    }
}

fn create_worker_content_security_policy_violation_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    violation: &ContentSecurityPolicyUrlViolation,
) -> Option<v8::Local<'s, v8::Object>> {
    let fields = ContentSecurityPolicyViolationEventFields::from_url_violation(violation);
    crate::content_security_policy::notify_content_security_policy_reporting_observers(
        scope, &fields,
    );
    create_security_policy_violation_event(scope, &fields)
}

fn send_worker_content_security_policy_reports_for_state(
    state: &Rc<RefCell<WorkerGlobalState>>,
    violation: &ContentSecurityPolicyUrlViolation,
) {
    let fields = ContentSecurityPolicyViolationEventFields::from_url_violation(violation);
    let requests = state
        .borrow()
        .loader
        .content_security_policy_reports()
        .requests(
            &fields,
            &violation.report_uri_endpoints,
            &violation.report_to_endpoints,
        );
    for request in requests {
        send_worker_content_security_policy_report_for_state(state, request);
    }
}

fn send_worker_content_security_policy_report_for_state(
    state: &Rc<RefCell<WorkerGlobalState>>,
    request: Request,
) {
    if !matches!(request.url.scheme(), "http" | "https") {
        return;
    }
    let (loader, document_url, partition, policy_context, runtime, client_id) = {
        let state = state.borrow();
        let Some(document_url) = state.current_script_url.clone() else {
            return;
        };
        (
            state.loader.clone(),
            document_url,
            state.network_partition_key.clone(),
            state.policy_context,
            state.service_worker_runtime.clone(),
            state.service_worker_client_id,
        )
    };
    let request = request
        .with_initiator_url(&document_url)
        .with_request_origin(moli_url::WebOrigin::from_url(&document_url))
        .with_network_partition_key(partition)
        .with_browser_request_metadata(BrowserRequestMetadata::Fetch);
    let cancel = moli_fetch::FetchCancelHandle::new();
    let Some(load) = loader.register_load(
        ResourceLoadKind::CspReport,
        ResourceLoadDisposition::Keepalive,
        Some(cancel.clone()),
    ) else {
        return;
    };
    let info = PendingSubresourceFetchInfo {
        internal_id: crate::network::interception_control::next_subresource_request_id(),
        network_request_handle: Some(
            crate::network::interception_control::next_subresource_network_handle(),
        ),
        frame_id: None,
        document_url,
        url: request.url.clone(),
        websocket_socket_id: None,
        method: request.method.clone(),
        request_headers: request.interception_request_headers(),
        request_body: request_body_text(&request.body),
        request_body_bytes: request.body.clone(),
        resource_type: SubresourceResourceType::CspReport,
        request_cookie_report: None,
    };
    let interception = state
        .borrow()
        .interception_context
        .clone()
        .expect("Worker resource output");
    let metadata = service_worker_fetch_request_metadata(&request);
    let runner = load.task_runner();
    runner.spawn(async move {
        crate::network_host::controlled_resource::ControlledResourceLoad::new(
            interception,
            request,
            info,
            load,
            cancel,
            crate::types::SubresourceRequestInitiatorType::Script,
        )
        .with_fetch_policy(policy_context)
        .with_service_worker(runtime, client_id, metadata)
        .run()
        .await;
    });
}

#[allow(clippy::too_many_arguments)]
pub(super) fn worker_content_security_policy_violation(
    state: &WorkerGlobalState,
    protected_url: &Url,
    request_url: &Url,
    kind: ContentSecurityPolicyResourceKind,
) -> Option<ContentSecurityPolicyUrlViolation> {
    worker_content_security_policy_violation_with_redirect_status(
        state,
        protected_url,
        request_url,
        kind,
        ContentSecurityPolicyRedirectStatus::NoRedirect,
    )
}

pub(super) fn worker_compilation_content_security_policy_violations(
    state: &WorkerGlobalState,
    protected_url: &Url,
    kind: ContentSecurityPolicyNonUrlKind,
    source: Option<&str>,
    disposition: ContentSecurityPolicyDisposition,
) -> Vec<ContentSecurityPolicyUrlViolation> {
    worker_policies(state, disposition)
        .filter_map(|(policy, report_uri_enabled)| {
            content_security_policy_non_url_violation_with_source(
                policy,
                worker_policy_url(state, protected_url),
                kind,
                source,
                disposition,
                &state.content_security_reporting_endpoints,
            )
            .map(|violation| worker_policy_violation(protected_url, report_uri_enabled, violation))
        })
        .collect()
}

fn worker_url_policy_violation(
    state: &WorkerGlobalState,
    protected_url: &Url,
    checked_url: &Url,
    blocked_url: &Url,
    kind: ContentSecurityPolicyResourceKind,
    redirect_status: ContentSecurityPolicyRedirectStatus,
    disposition: ContentSecurityPolicyDisposition,
) -> Option<ContentSecurityPolicyUrlViolation> {
    worker_policies(state, disposition).find_map(|(policy, report_uri_enabled)| {
        content_security_policy_url_violation_for_checked_url_with_redirect_status_disposition_and_reporting_endpoints(
            std::slice::from_ref(policy),
            worker_policy_url(state, protected_url),
            checked_url,
            blocked_url,
            kind,
            redirect_status,
            disposition,
            &state.content_security_reporting_endpoints,
        ).map(|violation| worker_policy_violation(protected_url, report_uri_enabled, violation))
    })
}

pub(super) fn worker_content_security_policy_violation_with_redirect_status(
    state: &WorkerGlobalState,
    protected_url: &Url,
    request_url: &Url,
    kind: ContentSecurityPolicyResourceKind,
    redirect_status: ContentSecurityPolicyRedirectStatus,
) -> Option<ContentSecurityPolicyUrlViolation> {
    worker_url_policy_violation(
        state,
        protected_url,
        request_url,
        request_url,
        kind,
        redirect_status,
        ContentSecurityPolicyDisposition::Enforce,
    )
}

pub(super) fn worker_content_security_policy_report_only_violation(
    state: &WorkerGlobalState,
    protected_url: &Url,
    request_url: &Url,
    kind: ContentSecurityPolicyResourceKind,
) -> Option<ContentSecurityPolicyUrlViolation> {
    worker_content_security_policy_report_only_violation_with_redirect_status(
        state,
        protected_url,
        request_url,
        kind,
        ContentSecurityPolicyRedirectStatus::NoRedirect,
    )
}

pub(super) fn worker_content_security_policy_report_only_violation_with_redirect_status(
    state: &WorkerGlobalState,
    protected_url: &Url,
    request_url: &Url,
    kind: ContentSecurityPolicyResourceKind,
    redirect_status: ContentSecurityPolicyRedirectStatus,
) -> Option<ContentSecurityPolicyUrlViolation> {
    worker_url_policy_violation(
        state,
        protected_url,
        request_url,
        request_url,
        kind,
        redirect_status,
        ContentSecurityPolicyDisposition::Report,
    )
}

pub(super) fn dispatch_worker_content_security_policy_report_only_violation_for_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    protected_url: &Url,
    request_url: &Url,
    kind: ContentSecurityPolicyResourceKind,
) {
    dispatch_worker_content_security_policy_report_only_violation_with_redirect_status_for_state(
        scope,
        state,
        protected_url,
        request_url,
        kind,
        ContentSecurityPolicyRedirectStatus::NoRedirect,
    );
}

pub(super) fn dispatch_worker_content_security_policy_report_only_violation_with_redirect_status_for_state<
    's,
>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    protected_url: &Url,
    request_url: &Url,
    kind: ContentSecurityPolicyResourceKind,
    redirect_status: ContentSecurityPolicyRedirectStatus,
) {
    let violation = {
        let state_ref = state.borrow();
        worker_content_security_policy_report_only_violation_with_redirect_status(
            &state_ref,
            protected_url,
            request_url,
            kind,
            redirect_status,
        )
    };
    if let Some(violation) = violation {
        dispatch_worker_content_security_policy_violation_event_for_state(scope, state, &violation);
    }
}

pub(super) fn worker_content_security_policy_error_message(
    violation: &ContentSecurityPolicyUrlViolation,
    operation: &'static str,
) -> String {
    format!(
        "{operation}: blocked by Content Security Policy for `{}`.",
        violation.blocked_uri
    )
}

pub(in crate::worker) fn dispatch_worker_csp_violation_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    loader: &crate::network::context::WorkerResourceLoader,
    violation: &crate::content_security_policy::ContentSecurityPolicyUrlViolation,
) {
    self::dispatch_worker_content_security_policy_violation_event(scope, loader, violation);
}

pub(crate) fn worker_allows_trusted_type_policy_name_by_csp(
    scope: &mut v8::PinScope<'_, '_>,
    name: &str,
    is_duplicate: bool,
) -> Option<bool> {
    let state = get_worker_state(scope)?;
    Some(self::allows_worker_trusted_type_policy_name_for_state(
        scope,
        &state,
        name,
        is_duplicate,
    ))
}

pub(crate) fn worker_trusted_types_for_script_requirements(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<crate::content_security_policy::TrustedTypesForScriptRequirements> {
    let state = get_worker_state(scope)?;
    let state = state.borrow();
    Some(
        crate::content_security_policy::TrustedTypesForScriptRequirements::new(
            crate::content_security_policy::content_security_policy_requires_trusted_types_for_script(
                &state.content_security_policies,
            ),
            crate::content_security_policy::content_security_policy_requires_trusted_types_for_script(
                &state.content_security_report_only_policies,
            ),
        ),
    )
}
pub(crate) fn worker_allows_trusted_types_eval(scope: &mut v8::PinScope<'_, '_>) -> Option<bool> {
    Some(
        crate::content_security_policy::content_security_policy_allows_trusted_types_eval(
            &get_worker_state(scope)?.borrow().content_security_policies,
        ),
    )
}

pub(crate) fn dispatch_worker_trusted_types_sink_violation_event(
    scope: &mut v8::PinScope<'_, '_>,
    sink: &str,
    sample: &str,
) {
    let Some(state) = get_worker_state(scope) else {
        return;
    };
    self::dispatch_worker_trusted_types_sink_violation_event_for_state(scope, &state, sink, sample);
}

pub(crate) fn worker_allows_eval_code_generation_by_csp(
    scope: &mut v8::PinScope<'_, '_>,
    allow_trusted_types_eval: bool,
    source: Option<&str>,
) -> Option<bool> {
    use crate::content_security_policy::ContentSecurityPolicyNonUrlKind;
    let kind = if allow_trusted_types_eval {
        ContentSecurityPolicyNonUrlKind::TrustedTypesEval
    } else {
        ContentSecurityPolicyNonUrlKind::Eval
    };
    worker_allows_compilation_by_csp(scope, kind, source)
}

fn worker_allows_compilation_by_csp(
    scope: &mut v8::PinScope<'_, '_>,
    kind: crate::content_security_policy::ContentSecurityPolicyNonUrlKind,
    source: Option<&str>,
) -> Option<bool> {
    let state = get_worker_state(scope)?;
    let (wake_tx, mut report_only_violations, mut enforce_violations) = {
        let state = state.borrow();
        let Some(protected_url) = state.current_script_url.as_ref() else {
            return Some(true);
        };
        (
            state.worker_wake_tx.clone(),
            worker_compilation_content_security_policy_violations(
                &state,
                protected_url,
                kind,
                source,
                crate::content_security_policy::ContentSecurityPolicyDisposition::Report,
            ),
            worker_compilation_content_security_policy_violations(
                &state,
                protected_url,
                kind,
                source,
                crate::content_security_policy::ContentSecurityPolicyDisposition::Enforce,
            ),
        )
    };
    if kind != crate::content_security_policy::ContentSecurityPolicyNonUrlKind::WasmEval
        && (!report_only_violations.is_empty() || !enforce_violations.is_empty())
        && let Some((source_file, line_number, column_number)) =
            crate::content_security_policy::current_script_violation_location(scope)
    {
        for violation in [&mut report_only_violations, &mut enforce_violations]
            .into_iter()
            .flatten()
        {
            violation.source_file.clone_from(&source_file);
            violation.line_number = line_number;
            violation.column_number = column_number;
        }
    }
    let allowed = enforce_violations.is_empty();
    for violation in report_only_violations.into_iter().chain(enforce_violations) {
        let _ = wake_tx.send(
            crate::worker::handle::WorkerMessage::DispatchContentSecurityPolicyViolation(Box::new(
                violation,
            )),
        );
    }
    Some(allowed)
}

pub(in crate::worker) fn dispatch_worker_csp_violation_event_for_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<RefCell<WorkerGlobalState>>,
    violation: &crate::content_security_policy::ContentSecurityPolicyUrlViolation,
) {
    self::dispatch_worker_content_security_policy_violation_event_for_state(
        scope, state, violation,
    );
}

pub(crate) fn worker_allows_wasm_code_generation_by_csp(
    scope: &mut v8::PinScope<'_, '_>,
) -> Option<bool> {
    worker_allows_compilation_by_csp(
        scope,
        crate::content_security_policy::ContentSecurityPolicyNonUrlKind::WasmEval,
        None,
    )
}

impl WorkerGlobalState {
    pub(in crate::worker) fn content_security_policy_snapshot_for_inheritance(
        &self,
    ) -> crate::content_security_policy::InheritedContentSecurityPolicy {
        worker_policy_snapshot(self)
    }
}
