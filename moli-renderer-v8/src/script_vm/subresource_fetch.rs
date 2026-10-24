use anyhow::{Result, anyhow};
use std::cell::RefCell;
use std::pin::pin;
use std::rc::Rc;
use std::time::Instant;
use url::Url;

use super::ScriptVm;
#[cfg(any(test, feature = "devtools"))]
use {
    super::AsyncSubresourceCommandExecution,
    crate::types::{PendingSubresourceAuthResponse, PendingSubresourceResponseDetails},
};

mod beacon;
mod document_completion;
mod fetch_dispatch;
mod intercepted_fetch;
mod intercepted_redirects;
mod pending_fetch;
mod response_preload;
mod streaming;

use crate::content_security_policy::ContentSecurityPolicyRedirectStatus;
use crate::document_runtime::DocumentRuntime;
use crate::frame_owner_model::FrameDocumentModuleTerminalQueueFollowup;
use crate::native_bridge::{OwnerDispatchScope, PageScriptHost, PendingTextTrackLoadTerminal};
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
    ChildModulepreloadFetchCompletion, ChildParserModuleRootFetchCompletion, NetworkBodySourceId,
    PendingSubresourceAuthInfo, PendingSubresourceAuthState, PendingSubresourceContinuation,
    PendingSubresourceContinueEvent, PendingSubresourceFetchInfo, PendingSubresourceFetchState,
    PendingSubresourceResponseInfo, PendingSubresourceResponseState,
    PopupClassicScriptLoadCompletion, PopupDocumentLoadCompletion, RunningSubresourceFetchState,
    StreamingSubresourceFetchState, SubresourceNetworkRequestHandle,
    SubresourceRequestInitiatorType, SubresourceResourceType, SubresourceResponseBody,
};
use crate::util::v8_string;

fn validate_pending_window_fetch_integrity(
    pending: &PendingSubresourceFetchState,
    method: &str,
    response: &crate::protocol_types::NavigationResponse,
    response_filter: Option<&AsyncSubresourceFetchResponseFilter>,
) -> std::result::Result<(), String> {
    let Some(fetch) = pending
        .continuation
        .window_fetch()
        .filter(|fetch| !fetch.integrity().is_empty())
    else {
        return Ok(());
    };
    let filter = response_filter.cloned().unwrap_or_else(|| {
        crate::network_host::FetchResponseRequest {
            method,
            mode: pending.request_mode,
            redirect_mode: fetch.redirect_mode(),
        }
        .network_response_filter(
            &pending.request_origin,
            &response.head(),
            pending.credentials_mode,
        )
    });
    crate::network_host::validate_fetch_response_integrity(
        fetch.integrity(),
        method,
        response.status,
        &filter,
        response.body_bytes(),
    )
}

fn document_connect_csp_redirect_failure_message<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context_host: &Rc<RefCell<PageScriptHost>>,
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

    let (policy, redirect_state, report_context) = pending.continuation.csp_request_policy()?;
    if redirect_state.was_checked(final_url) {
        return None;
    }
    let operation = match pending.info.resource_type {
        SubresourceResourceType::EventSource => "EventSource",
        SubresourceResourceType::Xhr => "XMLHttpRequest",
        _ => "fetch",
    };
    // Synthetic responses still need validation against the accepting Document,
    // while the producer already checked every real redirect before sending it.
    let (report_only, enforced) = policy
        .check_redirect(&pending.info.document_url, final_url)
        .into_violations();
    let mut failure = None;
    for (is_enforced, mut violation) in report_only
        .into_iter()
        .map(|violation| (false, violation))
        .chain(enforced.into_iter().map(|violation| (true, violation)))
    {
        report_window_csp_redirect_violation(
            scope,
            context_host,
            report_context,
            &pending.info.url,
            &mut violation,
        );
        if is_enforced && failure.is_none() {
            failure = Some(
                crate::document_runtime::document_content_security_policy_error_message(
                    &violation, operation,
                ),
            );
        }
    }
    failure
}

fn report_window_csp_redirect_violation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context_host: &Rc<RefCell<PageScriptHost>>,
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
    let host_ptr: *mut PageScriptHost = context_host.as_ref().as_ptr();
    context_host
        .borrow_mut()
        .dispatch_document_connect_csp_violation_event_for_exact_owner_without_report_best_effort(
            scope,
            host_ptr,
            report_context.identity(),
            violation,
        );
}

fn window_fetch_redirect_policy_was_checked(
    pending: &PendingSubresourceFetchState,
    head: &moli_fetch::ResponseHead,
) -> bool {
    // Intercepted hops were admitted before their next pause. A later network
    // redirect still needs its final-target check; do not report the same
    // captured-policy violation again when a fulfilled body is delivered.
    pending
        .continuation
        .window_fetch()
        .and_then(|fetch| fetch.intercepted_request.as_deref())
        .is_some_and(|request| {
            !request.redirect_chain().is_empty()
                && request.url == head.final_url
                && request.redirect_chain().len() == head.redirect_chain.len()
        })
}

fn detached_window_fetch_csp_redirect_failure_message(
    context_host: &Rc<RefCell<PageScriptHost>>,
    pending: &PendingSubresourceFetchState,
    final_url: &Url,
) -> Option<String> {
    let (policy, redirect_state, report_context) = pending.continuation.csp_request_policy()?;
    if redirect_state.was_checked(final_url) {
        return None;
    }
    let (reports, enforced) = policy
        .check_url(
            &pending.info.document_url,
            final_url,
            ContentSecurityPolicyRedirectStatus::FollowedRedirect,
        )
        .into_violations();
    let error = enforced.first().map(|violation| {
        crate::document_runtime::document_content_security_policy_error_message(violation, "fetch")
    });
    for mut violation in reports.into_iter().chain(enforced) {
        violation.blocked_uri =
            crate::content_security_policy::csp_url_for_report(&pending.info.url);
        crate::content_security_policy::ContentSecurityPolicySourceLocation::default()
            .apply_to(&mut violation);
        crate::network_host::send_content_security_policy_violation_report_from_window_context(
            &mut context_host.borrow_mut(),
            report_context,
            &violation,
        );
    }
    error
}

enum MediaSubresourceTerminal<'a> {
    Response {
        response: &'a crate::protocol_types::NavigationResponse,
        origin_clean: bool,
    },
    Failure,
}

fn apply_media_subresource_terminal(
    scope: &mut v8::PinScope<'_, '_>,
    context_host: &Rc<RefCell<PageScriptHost>>,
    media_handle: crate::document_runtime::DomHandle,
    sequence: crate::native_bridge::MediaLoadSequenceId,
    internal_id: u64,
    terminal: MediaSubresourceTerminal<'_>,
) {
    let followup = {
        let mut host = context_host.borrow_mut();
        let successful = match terminal {
            MediaSubresourceTerminal::Failure => false,
            MediaSubresourceTerminal::Response {
                response,
                origin_clean,
            } => {
                if !crate::network_host::media_response_status_is_successful(response.status)
                    || response.body_bytes().is_empty()
                {
                    false
                } else {
                    match host.begin_media_resource(
                        scope,
                        media_handle,
                        sequence,
                        response.body_bytes(),
                        origin_clean,
                        Some(internal_id),
                    ) {
                        Ok(true) => return,
                        Ok(false) => true,
                        Err(_) => false,
                    }
                }
            }
        };
        host.complete_pending_media_load_network_request_if_matches(
            media_handle,
            sequence,
            internal_id,
            successful,
        )
    };
    let host_ptr: *mut PageScriptHost = context_host.as_ref().as_ptr();
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
        origin_clean: bool,
    },
    Failure,
    PreloadFailure,
}

impl ImageSubresourceTerminal<'_> {
    fn resource_performance_entry(
        &self,
        request_url: &url::Url,
        initiator_type: &'static str,
    ) -> crate::context_bootstrap::ResourcePerformanceEntry {
        match self {
            Self::Response { response, encoded, .. } => {
                crate::context_bootstrap::ResourcePerformanceEntry::from_network_response_with_body_size(
                    request_url.as_str(),
                    initiator_type,
                    None,
                    response,
                    encoded.len(),
                )
            }
            Self::Failure | Self::PreloadFailure => {
                crate::context_bootstrap::ResourcePerformanceEntry::from_network_failure(
                    request_url.as_str(),
                    initiator_type,
                    None,
                )
            }
        }
    }
}

fn apply_image_subresource_terminal(
    scope: &mut v8::PinScope<'_, '_>,
    context_host: &Rc<RefCell<PageScriptHost>>,
    image_handle: crate::document_runtime::DomHandle,
    sequence: crate::native_bridge::ImageLoadEventId,
    internal_id: u64,
    request_url: &url::Url,
    terminal: ImageSubresourceTerminal<'_>,
) {
    let initiator_type = context_host
        .borrow()
        .pending_image_load_event(image_handle)
        .filter(|pending| pending.id() == sequence)
        .map(|pending| pending.resource_timing_initiator_type());
    let (accepted, followup) = match &terminal {
        ImageSubresourceTerminal::Response {
            response,
            encoded,
            origin_clean,
        } => {
            let descriptor = crate::network_host::image_response_descriptor_from_parkable(
                response,
                encoded,
                *origin_clean,
            );
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
        ImageSubresourceTerminal::Failure | ImageSubresourceTerminal::PreloadFailure => {
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
    if accepted
        && !matches!(terminal, ImageSubresourceTerminal::PreloadFailure)
        && let Some(initiator_type) = initiator_type
    {
        crate::context_bootstrap::record_resource_performance_entry(
            scope,
            terminal.resource_performance_entry(request_url, initiator_type),
        );
    }
    let host_ptr: *mut PageScriptHost = context_host.as_ref().as_ptr();
    crate::native_bridge::element::queue_image_load_network_terminal_followup(
        host_ptr,
        image_handle,
        sequence,
        followup,
    );
}

fn apply_text_track_subresource_terminal(
    scope: &mut v8::PinScope<'_, '_>,
    context_host: &Rc<RefCell<PageScriptHost>>,
    track_handle: crate::document_runtime::DomHandle,
    sequence: crate::native_bridge::TextTrackLoadSequenceId,
    internal_id: u64,
    terminal: PendingTextTrackLoadTerminal,
) {
    let followup = context_host
        .borrow_mut()
        .complete_pending_text_track_network_if_matches(
            track_handle,
            sequence,
            internal_id,
            terminal,
        );
    let host_ptr: *mut PageScriptHost = context_host.as_ref().as_ptr();
    crate::native_bridge::element::queue_text_track_terminal_followup(
        scope,
        host_ptr,
        track_handle,
        sequence,
        followup,
    );
}

fn apply_stylesheet_subresource_terminal(
    context_host: &Rc<RefCell<PageScriptHost>>,
    binding: crate::frame_owner_model::StylesheetSubresourceLoadDelayBinding,
) {
    let _ = context_host
        .borrow_mut()
        .settle_stylesheet_subresource_load_delay(binding);
}

fn enter_subresource_owner_async_scope<'s>(
    context_host: &Rc<RefCell<PageScriptHost>>,
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
    context_host: &Rc<RefCell<PageScriptHost>>,
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
    context_host: &Rc<RefCell<PageScriptHost>>,
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
    context_host: &Rc<RefCell<PageScriptHost>>,
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
    context_host: &Rc<RefCell<PageScriptHost>>,
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
        AsyncSubresourceFetchEvent::ResponseLinks(_) => AsyncSubresourceTraceFields {
            event_kind: Some("response_links"),
            ..AsyncSubresourceTraceFields::default()
        },
        AsyncSubresourceFetchEvent::DocumentAbort { internal_id } => AsyncSubresourceTraceFields {
            event_kind: Some("document_abort"),
            internal_id: Some(*internal_id),
            ..AsyncSubresourceTraceFields::default()
        },
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
        AsyncSubresourceFetchEvent::InterceptedXhrStreamingStarted { started, .. } => {
            AsyncSubresourceTraceFields {
                event_kind: Some("intercepted_xhr_stream_start"),
                internal_id: Some(started.internal_id),
                body_source_id: Some(started.body_source_id),
                ..AsyncSubresourceTraceFields::default()
            }
        }
        AsyncSubresourceFetchEvent::InterceptedXhrCompleted(completed) => {
            AsyncSubresourceTraceFields {
                event_kind: Some("intercepted_xhr_complete"),
                internal_id: Some(completed.info.internal_id),
                ..AsyncSubresourceTraceFields::default()
            }
        }
        AsyncSubresourceFetchEvent::InterceptionOutputReady(_) => AsyncSubresourceTraceFields {
            event_kind: Some("native_interception_output"),
            ..Default::default()
        },
        AsyncSubresourceFetchEvent::ObservedNetworkRecord(record) => AsyncSubresourceTraceFields {
            event_kind: Some("observed_network_record"),
            resource_type: Some(record.resource_type()),
            ..AsyncSubresourceTraceFields::default()
        },
        AsyncSubresourceFetchEvent::RequestExtraInfo { internal_id, .. } => {
            AsyncSubresourceTraceFields {
                event_kind: Some("request_extra_info"),
                internal_id: Some(*internal_id),
                ..Default::default()
            }
        }
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
        PendingSubresourceContinuation::FontFace(_) => "font_face",
        PendingSubresourceContinuation::StylesheetSubresource { .. } => "stylesheet_subresource",
        PendingSubresourceContinuation::Dictionary(_) => "dictionary",
        PendingSubresourceContinuation::ResponsePreload(_) => "response_preload",
        PendingSubresourceContinuation::Beacon(_) => "beacon",
        PendingSubresourceContinuation::CspReport { .. } => "csp_report",
        PendingSubresourceContinuation::Xhr { .. } => "xhr",
        PendingSubresourceContinuation::WebSocket(..) => "websocket",
        PendingSubresourceContinuation::ChildDocument(_) => "child_document",
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
