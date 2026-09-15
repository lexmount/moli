use anyhow::{Result, anyhow, bail};
use moli_shared_worker::SharedWorkerInstanceId;
use std::cell::RefCell;
use std::pin::pin;
use std::rc::Rc;
use std::time::Instant;
use url::Url;

use super::{AsyncSubresourceCommandExecution, ScriptVm};

mod document_completion;
mod fetch_dispatch;
mod intercepted_fetch;
mod pending_fetch;
mod streaming;

use crate::RendererSyntheticResponseBody;
use crate::content_security_policy::ContentSecurityPolicyRedirectStatus;
use crate::document_runtime::DocumentRuntime;
use crate::frame_owner_model::FrameDocumentModuleTerminalQueueFollowup;
use crate::native_bridge::{JsContextHost, OwnerDispatchScope};
use crate::network::ResourceRequestClient;
use crate::runtime::{
    AuthorizedCurrentChildDocumentLoadCompletion, AuthorizedCurrentChildModuleFetchCompletion,
    AuthorizedCurrentPopupClassicScriptLoadCompletion,
    AuthorizedCurrentPopupDocumentLoadCompletion, CurrentChildDocumentLoadApplication,
};
use crate::types::{
    AsyncSubresourceFetchCompletion, AsyncSubresourceFetchEvent,
    AsyncSubresourceFetchResponseFilter, AsyncSubresourceFetchResult,
    ChildBlockingStylesheetLoadCompletion, ChildClassicScriptLoadCompletion,
    ChildDocumentLoadCompletion, ChildModuleDependencyFetchCompletion,
    ChildModulepreloadFetchCompletion, ChildParserModuleRootFetchCompletion, DedicatedWorkerId,
    NetworkBodySourceId, PendingSubresourceAuthInfo, PendingSubresourceAuthState,
    PendingSubresourceContinuation, PendingSubresourceContinueEvent,
    PendingSubresourceContinueOutcome, PendingSubresourceFetchInfo, PendingSubresourceFetchState,
    PendingSubresourceResponseInfo, PendingSubresourceResponseState,
    PopupClassicScriptLoadCompletion, PopupDocumentLoadCompletion, RunningSubresourceFetchState,
    StreamingSubresourceFetchState, SubresourceNetworkRecord, SubresourceNetworkRequestHandle,
    SubresourceRequestInitiatorType, SubresourceResourceType, SubresourceResponseBody,
    SubresourceResponseBodyWriter,
};
use crate::util::v8_string;

#[derive(Clone, Copy)]
enum WorkerOwnedFetchTarget {
    Dedicated {
        worker_id: DedicatedWorkerId,
        fetch_id: u32,
    },
    Shared {
        instance_id: SharedWorkerInstanceId,
        fetch_id: u32,
    },
}

impl WorkerOwnedFetchTarget {
    fn from_continuation(continuation: &PendingSubresourceContinuation) -> Option<Self> {
        match continuation {
            PendingSubresourceContinuation::WorkerFetch {
                worker_id,
                fetch_id,
            } => Some(Self::Dedicated {
                worker_id: *worker_id,
                fetch_id: *fetch_id,
            }),
            PendingSubresourceContinuation::SharedWorkerFetch {
                instance_id,
                fetch_id,
            } => Some(Self::Shared {
                instance_id: *instance_id,
                fetch_id: *fetch_id,
            }),
            _ => None,
        }
    }

    fn fetch_id(self) -> u32 {
        match self {
            Self::Dedicated { fetch_id, .. } | Self::Shared { fetch_id, .. } => fetch_id,
        }
    }

    fn continuation(self) -> PendingSubresourceContinuation {
        match self {
            Self::Dedicated {
                worker_id,
                fetch_id,
            } => PendingSubresourceContinuation::WorkerFetch {
                worker_id,
                fetch_id,
            },
            Self::Shared {
                instance_id,
                fetch_id,
            } => PendingSubresourceContinuation::SharedWorkerFetch {
                instance_id,
                fetch_id,
            },
        }
    }

    fn unavailable_message(self) -> String {
        match self {
            Self::Dedicated {
                worker_id,
                fetch_id,
            } => format!("worker `{worker_id}` is not available for pending fetch `{fetch_id}`"),
            Self::Shared {
                instance_id,
                fetch_id,
            } => format!(
                "shared worker `{}` is not available for pending fetch `{fetch_id}`",
                instance_id.as_u64()
            ),
        }
    }
}

fn document_connect_csp_redirect_failure_message<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context_host: &Rc<RefCell<JsContextHost>>,
    pending: &PendingSubresourceFetchState,
    final_url: &Url,
) -> Option<String> {
    if !matches!(
        pending.info.resource_type,
        SubresourceResourceType::EventSource
            | SubresourceResourceType::Fetch
            | SubresourceResourceType::Xhr
    ) {
        return None;
    }

    if let Some(fetch) = pending.continuation.window_fetch() {
        if fetch.redirect_csp_state().was_checked(final_url) {
            return None;
        }
        let redirect_status = ContentSecurityPolicyRedirectStatus::FollowedRedirect;
        if let Some(mut violation) = fetch.connect_policy().report_only_violation(
            &pending.info.document_url,
            final_url,
            redirect_status,
        ) {
            report_window_fetch_csp_redirect_violation(
                scope,
                context_host,
                fetch.csp_report_context(),
                &pending.info.url,
                &mut violation,
            );
        }
        let mut violation = fetch.connect_policy().enforce_violation(
            &pending.info.document_url,
            final_url,
            redirect_status,
        )?;
        report_window_fetch_csp_redirect_violation(
            scope,
            context_host,
            fetch.csp_report_context(),
            &pending.info.url,
            &mut violation,
        );
        return Some(
            crate::document_runtime::document_content_security_policy_error_message(
                &violation, "fetch",
            ),
        );
    }

    let redirect_status = ContentSecurityPolicyRedirectStatus::FollowedRedirect;
    let violation = context_host
        .borrow_mut()
        .check_document_connect_csp_for_owner_with_redirect_status(
            scope,
            pending.execution_context.dispatch_scope(),
            &pending.info.document_url,
            final_url,
            redirect_status,
        )
        .into_blocking_violation()?;
    let operation = match pending.info.resource_type {
        SubresourceResourceType::EventSource => "EventSource",
        SubresourceResourceType::Xhr => "XMLHttpRequest",
        _ => "fetch",
    };
    Some(
        crate::document_runtime::document_content_security_policy_error_message(
            &violation, operation,
        ),
    )
}

fn report_window_fetch_csp_redirect_violation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context_host: &Rc<RefCell<JsContextHost>>,
    report_context: &crate::network_host::WindowCspReportRequestContext,
    request_url: &Url,
    violation: &mut crate::document_runtime::DocumentContentSecurityPolicyViolation,
) {
    // Redirect checks create a new violation without an executing script. Report
    // the initial request URL, never the potentially private redirect target.
    violation.blocked_uri = crate::content_security_policy::csp_url_for_report(request_url);
    crate::content_security_policy::ContentSecurityPolicySourceLocation::default()
        .apply_to(violation);
    crate::network_host::send_content_security_policy_violation_report_from_window_context(
        &mut context_host.borrow_mut(),
        report_context,
        violation,
    );
    let host_ptr: *mut JsContextHost = context_host.as_ref().as_ptr();
    context_host
        .borrow_mut()
        .dispatch_document_connect_csp_violation_event_for_exact_owner_without_report_best_effort(
            scope,
            host_ptr,
            report_context.identity(),
            violation,
        );
}

fn detached_window_fetch_csp_redirect_failure_message(
    context_host: &Rc<RefCell<JsContextHost>>,
    pending: &PendingSubresourceFetchState,
    final_url: &Url,
) -> Option<String> {
    let fetch = pending.continuation.window_fetch()?;
    if fetch.redirect_csp_state().was_checked(final_url) {
        return None;
    }
    let redirect_status = ContentSecurityPolicyRedirectStatus::FollowedRedirect;
    if let Some(mut violation) = fetch.connect_policy().report_only_violation(
        &pending.info.document_url,
        final_url,
        redirect_status,
    ) {
        violation.blocked_uri =
            crate::content_security_policy::csp_url_for_report(&pending.info.url);
        crate::content_security_policy::ContentSecurityPolicySourceLocation::default()
            .apply_to(&mut violation);
        crate::network_host::send_content_security_policy_violation_report_from_window_context(
            &mut context_host.borrow_mut(),
            fetch.csp_report_context(),
            &violation,
        );
    }
    let mut violation = fetch.connect_policy().enforce_violation(
        &pending.info.document_url,
        final_url,
        redirect_status,
    )?;
    violation.blocked_uri = crate::content_security_policy::csp_url_for_report(&pending.info.url);
    crate::content_security_policy::ContentSecurityPolicySourceLocation::default()
        .apply_to(&mut violation);
    crate::network_host::send_content_security_policy_violation_report_from_window_context(
        &mut context_host.borrow_mut(),
        fetch.csp_report_context(),
        &violation,
    );
    Some(
        crate::document_runtime::document_content_security_policy_error_message(
            &violation, "fetch",
        ),
    )
}

fn apply_media_subresource_terminal(
    scope: &mut v8::PinScope<'_, '_>,
    context_host: &Rc<RefCell<JsContextHost>>,
    media_handle: crate::document_runtime::DomHandle,
    sequence: crate::native_bridge::MediaLoadSequenceId,
    internal_id: u64,
    successful: bool,
) {
    let followup = context_host
        .borrow_mut()
        .complete_pending_media_load_network_request_if_matches(
            media_handle,
            sequence,
            internal_id,
            successful,
        );
    let host_ptr: *mut JsContextHost = context_host.as_ref().as_ptr();
    crate::native_bridge::element::queue_media_load_network_terminal_followup(
        scope,
        host_ptr,
        media_handle,
        sequence,
        followup,
    );
}

enum ImageSubresourceTerminal<'a> {
    Response {
        response: &'a crate::protocol_types::NavigationResponse,
        encoded: &'a moli_parkable_image::ParkableImage,
    },
    Failure,
}

impl ImageSubresourceTerminal<'_> {
    fn resource_performance_entry(
        &self,
        request_url: &url::Url,
    ) -> crate::context_bootstrap::ResourcePerformanceEntry {
        match self {
            Self::Response { response, encoded } => {
                crate::context_bootstrap::ResourcePerformanceEntry::from_network_response_with_body_size(
                    request_url.as_str(),
                    "img",
                    None,
                    response,
                    encoded.len(),
                )
            }
            Self::Failure => {
                crate::context_bootstrap::ResourcePerformanceEntry::from_network_failure(
                    request_url.as_str(),
                    "img",
                    None,
                )
            }
        }
    }
}

fn apply_image_subresource_terminal(
    scope: &mut v8::PinScope<'_, '_>,
    context_host: &Rc<RefCell<JsContextHost>>,
    image_handle: crate::document_runtime::DomHandle,
    sequence: crate::native_bridge::ImageLoadEventId,
    internal_id: u64,
    request_url: &url::Url,
    terminal: ImageSubresourceTerminal<'_>,
) {
    let (accepted, followup) = match &terminal {
        ImageSubresourceTerminal::Response { response, encoded } => {
            let descriptor =
                crate::network_host::image_response_descriptor_from_parkable(response, encoded);
            let completion = context_host
                .borrow_mut()
                .complete_pending_image_load_network_response_if_matches(
                    image_handle,
                    sequence,
                    internal_id,
                    descriptor,
                    (*encoded).clone(),
                );
            (completion.accepted(), completion.followup())
        }
        ImageSubresourceTerminal::Failure => {
            let followup = context_host
                .borrow_mut()
                .complete_pending_image_load_network_request_if_matches(
                    image_handle,
                    sequence,
                    internal_id,
                    false,
                );
            (followup.is_some(), followup)
        }
    };
    if accepted {
        crate::context_bootstrap::record_resource_performance_entry(
            scope,
            terminal.resource_performance_entry(request_url),
        );
    }
    let host_ptr: *mut JsContextHost = context_host.as_ref().as_ptr();
    crate::native_bridge::element::queue_image_load_network_terminal_followup(
        host_ptr,
        image_handle,
        sequence,
        followup,
    );
}

fn apply_text_track_subresource_terminal(
    scope: &mut v8::PinScope<'_, '_>,
    context_host: &Rc<RefCell<JsContextHost>>,
    track_handle: crate::document_runtime::DomHandle,
    sequence: crate::native_bridge::TextTrackLoadSequenceId,
    internal_id: u64,
    result: Result<String, String>,
) {
    let followup = context_host
        .borrow_mut()
        .complete_pending_text_track_network_if_matches(
            track_handle,
            sequence,
            internal_id,
            result,
        );
    let host_ptr: *mut JsContextHost = context_host.as_ref().as_ptr();
    crate::native_bridge::element::queue_text_track_terminal_followup(
        scope,
        host_ptr,
        track_handle,
        sequence,
        followup,
    );
}

fn apply_stylesheet_subresource_terminal(
    context_host: &Rc<RefCell<JsContextHost>>,
    binding: crate::frame_owner_model::StylesheetSubresourceLoadDelayBinding,
) {
    let _ = context_host
        .borrow_mut()
        .settle_stylesheet_subresource_load_delay(binding);
}

fn enter_subresource_owner_async_scope<'s>(
    context_host: &Rc<RefCell<JsContextHost>>,
    scope: &mut v8::PinScope<'s, '_>,
    owner: OwnerDispatchScope,
) -> Option<v8::Local<'s, v8::Value>> {
    match owner {
        OwnerDispatchScope::Top => None,
        OwnerDispatchScope::Child(handle) => {
            let child_context_exists = context_host
                .borrow()
                .frame_owner_frame_id_for_child_handle(handle)
                .is_some();
            child_context_exists.then(|| {
                context_host
                    .borrow_mut()
                    .enter_child_async_continuation_scope(scope, handle)
            })
        }
        OwnerDispatchScope::LightweightPopup(popup_id) => {
            let popup_context_exists = context_host
                .borrow()
                .lightweight_popup_request_base_url(scope, popup_id)
                .is_some();
            popup_context_exists.then(|| {
                crate::native_bridge::enter_active_lightweight_popup_scope(scope, popup_id)
            })
        }
    }
}

/// Leave the Window attribution installed until the selected resource task's
/// checkpoint. Promise reactions created by Fetch/XHR completion must observe
/// the same child or popup Window as the body that settled them.
fn defer_subresource_owner_async_scope<'s>(
    context_host: &Rc<RefCell<JsContextHost>>,
    scope: &mut v8::PinScope<'s, '_>,
    owner: OwnerDispatchScope,
    previous: Option<v8::Local<'s, v8::Value>>,
) {
    match (owner, previous) {
        (OwnerDispatchScope::Child(_), Some(previous)) => {
            crate::native_bridge::defer_active_child_window_restore(scope, previous);
            context_host
                .borrow_mut()
                .defer_child_subresource_request_scope_pop_after_microtasks();
        }
        (OwnerDispatchScope::LightweightPopup(_), Some(previous)) => {
            crate::native_bridge::defer_active_lightweight_popup_restore(scope, previous);
        }
        _ => {}
    }
}

fn dispatch_streaming_event_source_messages<'s>(
    context_host: &Rc<RefCell<JsContextHost>>,
    scope: &mut v8::PinScope<'s, '_>,
    event_source: v8::Local<'s, v8::Object>,
    request_handle: Option<SubresourceNetworkRequestHandle>,
    messages: &[crate::network_host::EventSourceMessage],
) {
    for message in messages {
        if crate::network_host::event_source_ready_state(scope, event_source)
            == crate::network_host::EVENT_SOURCE_CLOSED
        {
            break;
        }
        if let Some(handle) = request_handle {
            context_host
                .borrow_mut()
                .record_subresource_event_source_message_received(
                    crate::types::SubresourceEventSourceMessageReceived::new(
                        handle,
                        message.event_name.clone(),
                        message.event_id.clone(),
                        message.data.clone(),
                    ),
                );
        }
        crate::network_host::dispatch_event_source_message(scope, event_source, message);
    }
}

/// V8/Window activity produced by one async-subresource body.
///
/// This value is created only after the body has run. It is not a queued task
/// policy: the enclosing carrier consumes it to decide whether that carrier
/// actually entered a Window realm and therefore owns a completion. A selected
/// Networking task may additionally reconcile child records; a Fetch command
/// owns only its explicit command-end checkpoint. Those carrier-specific
/// effects must not be inferred in this body layer.
#[must_use = "async-subresource body activity determines task completion"]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AsyncSubresourceFetchBodyActivity {
    NoWindowRealmEntered,
    WindowRealmEntered,
}

fn window_subresource_owner_is_current(
    context_host: &Rc<RefCell<JsContextHost>>,
    pending: &PendingSubresourceFetchState,
) -> bool {
    pending
        .execution_context
        .window_request_target()
        .is_none_or(|target| {
            context_host
                .borrow()
                .window_execution_context_owner_is_current(target.owner(), target.dispatch_scope())
        })
}

fn window_subresource_realm_is_current(
    context_host: &Rc<RefCell<JsContextHost>>,
    scope: &mut v8::PinScope<'_, '_>,
    pending: &PendingSubresourceFetchState,
) -> bool {
    // The entered V8 context proves only that the persistent handle still
    // exists. Registry resolution additionally proves that this exact realm
    // token still belongs to its original LocalWindow and access-policy
    // registration; a replacement realm must not inherit the completion.
    let Some(binding) = pending.execution_context.window_realm_binding() else {
        return true;
    };
    crate::native_bridge::current_runtime_observable_context_token(scope)
        == Some(binding.realm_token())
        && binding.is_current(&context_host.borrow())
}

#[derive(Clone, Copy)]
enum WorkerOwnedXhrTarget {
    Dedicated {
        worker_id: DedicatedWorkerId,
        xhr_id: u32,
    },
    Shared {
        instance_id: SharedWorkerInstanceId,
        xhr_id: u32,
    },
}

impl WorkerOwnedXhrTarget {
    fn from_continuation(continuation: &PendingSubresourceContinuation) -> Option<Self> {
        match continuation {
            PendingSubresourceContinuation::WorkerXhr { worker_id, xhr_id } => {
                Some(Self::Dedicated {
                    worker_id: *worker_id,
                    xhr_id: *xhr_id,
                })
            }
            PendingSubresourceContinuation::SharedWorkerXhr {
                instance_id,
                xhr_id,
            } => Some(Self::Shared {
                instance_id: *instance_id,
                xhr_id: *xhr_id,
            }),
            _ => None,
        }
    }

    fn xhr_id(self) -> u32 {
        match self {
            Self::Dedicated { xhr_id, .. } | Self::Shared { xhr_id, .. } => xhr_id,
        }
    }

    fn continuation(self) -> PendingSubresourceContinuation {
        match self {
            Self::Dedicated { worker_id, xhr_id } => {
                PendingSubresourceContinuation::WorkerXhr { worker_id, xhr_id }
            }
            Self::Shared {
                instance_id,
                xhr_id,
            } => PendingSubresourceContinuation::SharedWorkerXhr {
                instance_id,
                xhr_id,
            },
        }
    }

    fn unavailable_message(self) -> String {
        match self {
            Self::Dedicated { worker_id, xhr_id } => {
                format!("worker `{worker_id}` is not available for pending xhr `{xhr_id}`")
            }
            Self::Shared {
                instance_id,
                xhr_id,
            } => format!(
                "shared worker `{}` is not available for pending xhr `{xhr_id}`",
                instance_id.as_u64()
            ),
        }
    }
}

#[derive(Clone, Copy)]
enum WorkerOwnedCspReportTarget {
    Dedicated {
        worker_id: DedicatedWorkerId,
        report_id: u32,
    },
    Shared {
        instance_id: SharedWorkerInstanceId,
        report_id: u32,
    },
}

impl WorkerOwnedCspReportTarget {
    fn from_continuation(continuation: &PendingSubresourceContinuation) -> Option<Self> {
        match continuation {
            PendingSubresourceContinuation::WorkerCspReport {
                worker_id,
                report_id,
            } => Some(Self::Dedicated {
                worker_id: *worker_id,
                report_id: *report_id,
            }),
            PendingSubresourceContinuation::SharedWorkerCspReport {
                instance_id,
                report_id,
            } => Some(Self::Shared {
                instance_id: *instance_id,
                report_id: *report_id,
            }),
            _ => None,
        }
    }

    fn report_id(self) -> u32 {
        match self {
            Self::Dedicated { report_id, .. } | Self::Shared { report_id, .. } => report_id,
        }
    }

    fn unavailable_message(self) -> String {
        match self {
            Self::Dedicated {
                worker_id,
                report_id,
            } => format!(
                "worker `{worker_id}` is not available for pending CSP report `{report_id}`"
            ),
            Self::Shared {
                instance_id,
                report_id,
            } => format!(
                "shared worker `{}` is not available for pending CSP report `{report_id}`",
                instance_id.as_u64()
            ),
        }
    }
}

fn with_pending_subresource_record_identity(
    record: SubresourceNetworkRecord,
    request_body_bytes: Option<Vec<u8>>,
    request_handle: Option<SubresourceNetworkRequestHandle>,
) -> SubresourceNetworkRecord {
    let mut record = record.with_request_body_bytes(request_body_bytes);
    if let Some(handle) = request_handle {
        record = record.with_request_handle(handle);
    }
    record
}

impl ScriptVm {}

#[derive(Clone, Copy, Default)]
struct AsyncSubresourceTraceFields {
    event_kind: Option<&'static str>,
    internal_id: Option<u64>,
    body_source_id: Option<NetworkBodySourceId>,
    bytes: Option<usize>,
    continuation_kind: Option<&'static str>,
    resource_type: Option<SubresourceResourceType>,
}

fn async_subresource_trace_fields_for_pending(
    event_kind: &'static str,
    internal_id: u64,
    pending: &PendingSubresourceFetchState,
) -> AsyncSubresourceTraceFields {
    async_subresource_trace_fields_for_pending_with_body(event_kind, internal_id, None, pending)
}

fn async_subresource_trace_fields_for_pending_with_body(
    event_kind: &'static str,
    internal_id: u64,
    body_source_id: Option<NetworkBodySourceId>,
    pending: &PendingSubresourceFetchState,
) -> AsyncSubresourceTraceFields {
    AsyncSubresourceTraceFields {
        event_kind: Some(event_kind),
        internal_id: Some(internal_id),
        body_source_id,
        bytes: None,
        continuation_kind: Some(pending_subresource_continuation_kind(&pending.continuation)),
        resource_type: Some(pending.info.resource_type),
    }
}

fn async_subresource_trace_fields_for_event(
    event: &AsyncSubresourceFetchEvent,
) -> AsyncSubresourceTraceFields {
    match event {
        AsyncSubresourceFetchEvent::ContentSecurityPolicyViolation { .. } => {
            AsyncSubresourceTraceFields {
                event_kind: Some("csp_violation"),
                ..AsyncSubresourceTraceFields::default()
            }
        }
        AsyncSubresourceFetchEvent::Upload { internal_id, .. } => AsyncSubresourceTraceFields {
            event_kind: Some("upload"),
            internal_id: Some(*internal_id),
            ..AsyncSubresourceTraceFields::default()
        },
        AsyncSubresourceFetchEvent::Completion(completion) => AsyncSubresourceTraceFields {
            event_kind: Some("completion"),
            internal_id: Some(completion.internal_id),
            ..AsyncSubresourceTraceFields::default()
        },
        AsyncSubresourceFetchEvent::ObservedNetworkRecord(record) => AsyncSubresourceTraceFields {
            event_kind: Some("observed_network_record"),
            resource_type: Some(record.resource_type()),
            ..AsyncSubresourceTraceFields::default()
        },
        AsyncSubresourceFetchEvent::StreamingStarted(started) => AsyncSubresourceTraceFields {
            event_kind: Some("streaming_started"),
            internal_id: Some(started.internal_id),
            body_source_id: Some(started.body_source_id),
            ..AsyncSubresourceTraceFields::default()
        },
        AsyncSubresourceFetchEvent::StreamingChunk(chunk) => AsyncSubresourceTraceFields {
            event_kind: Some("streaming_chunk"),
            body_source_id: Some(chunk.body_source_id),
            bytes: Some(chunk.bytes.len()),
            ..AsyncSubresourceTraceFields::default()
        },
        AsyncSubresourceFetchEvent::StreamingFinished(finished) => AsyncSubresourceTraceFields {
            event_kind: Some("streaming_finished"),
            internal_id: Some(finished.internal_id),
            body_source_id: Some(finished.body_source_id),
            ..AsyncSubresourceTraceFields::default()
        },
    }
}

fn pending_subresource_continuation_kind(
    continuation: &PendingSubresourceContinuation,
) -> &'static str {
    match continuation {
        PendingSubresourceContinuation::EventSource(_) => "event_source",
        PendingSubresourceContinuation::Fetch(_) => "fetch",
        PendingSubresourceContinuation::Image { .. } => "image",
        PendingSubresourceContinuation::Media { .. } => "media",
        PendingSubresourceContinuation::TextTrack { .. } => "text_track",
        PendingSubresourceContinuation::StylesheetSubresource { .. } => "stylesheet_subresource",
        PendingSubresourceContinuation::Beacon => "beacon",
        PendingSubresourceContinuation::CspReport { .. } => "csp_report",
        PendingSubresourceContinuation::Xhr { .. } => "xhr",
        PendingSubresourceContinuation::WebSocket(_) => "websocket",
        PendingSubresourceContinuation::WorkerFetch { .. } => "worker_fetch",
        PendingSubresourceContinuation::WorkerXhr { .. } => "worker_xhr",
        PendingSubresourceContinuation::WorkerCspReport { .. } => "worker_csp_report",
        PendingSubresourceContinuation::SharedWorkerFetch { .. } => "shared_worker_fetch",
        PendingSubresourceContinuation::SharedWorkerXhr { .. } => "shared_worker_xhr",
        PendingSubresourceContinuation::SharedWorkerCspReport { .. } => "shared_worker_csp_report",
    }
}

fn trace_async_subresource_stage(
    stage: &'static str,
    fields: AsyncSubresourceTraceFields,
    started: Option<Instant>,
) {
    if let Some(started) = started {
        tracing::info!(
            target: "moli_cdp_runtime",
            stage,
            event_kind = ?fields.event_kind,
            internal_id = ?fields.internal_id,
            body_source_id = ?fields.body_source_id,
            bytes = ?fields.bytes,
            continuation_kind = ?fields.continuation_kind,
            resource_type = ?fields.resource_type,
            elapsed_us = %started.elapsed().as_micros(),
        );
    }
}
