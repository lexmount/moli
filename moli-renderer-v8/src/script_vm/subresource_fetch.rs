use anyhow::{Result, anyhow, bail};
use moli_shared_worker::SharedWorkerInstanceId;
use std::cell::RefCell;
use std::pin::pin;
use std::rc::Rc;
use std::time::Instant;
use url::Url;

use super::{AsyncSubresourceCommandExecution, ScriptVm};
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
    AsyncSubresourceFetchResponseFilter, ChildBlockingStylesheetLoadCompletion,
    ChildClassicScriptLoadCompletion, ChildDocumentLoadCompletion,
    ChildModuleDependencyFetchCompletion, ChildModulepreloadFetchCompletion,
    ChildParserModuleRootFetchCompletion, DedicatedWorkerId, NetworkBodySourceId,
    PendingSubresourceAuthInfo, PendingSubresourceAuthState, PendingSubresourceContinuation,
    PendingSubresourceContinueEvent, PendingSubresourceContinueOutcome,
    PendingSubresourceFetchInfo, PendingSubresourceFetchState, PendingSubresourceResponseInfo,
    PendingSubresourceResponseState, PopupClassicScriptLoadCompletion, PopupDocumentLoadCompletion,
    RunningSubresourceFetchState, StreamingSubresourceFetchState, SubresourceNetworkRecord,
    SubresourceNetworkRequestHandle, SubresourceRequestInitiatorType, SubresourceResourceType,
    SubresourceResponseBody, SubresourceResponseBodyWriter,
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
        let redirect_status = ContentSecurityPolicyRedirectStatus::FollowedRedirect;
        if let Some(violation) = fetch.connect_policy().report_only_violation(
            &pending.info.document_url,
            final_url,
            redirect_status,
        ) {
            report_window_fetch_csp_redirect_violation(
                scope,
                context_host,
                fetch.csp_report_context(),
                &violation,
            );
        }
        let violation = fetch.connect_policy().enforce_violation(
            &pending.info.document_url,
            final_url,
            redirect_status,
        )?;
        report_window_fetch_csp_redirect_violation(
            scope,
            context_host,
            fetch.csp_report_context(),
            &violation,
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
    violation: &crate::document_runtime::DocumentContentSecurityPolicyViolation,
) {
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
    let redirect_status = ContentSecurityPolicyRedirectStatus::FollowedRedirect;
    if let Some(violation) = fetch.connect_policy().report_only_violation(
        &pending.info.document_url,
        final_url,
        redirect_status,
    ) {
        crate::network_host::send_content_security_policy_violation_report_from_window_context(
            &mut context_host.borrow_mut(),
            fetch.csp_report_context(),
            &violation,
        );
    }
    let violation = fetch.connect_policy().enforce_violation(
        &pending.info.document_url,
        final_url,
        redirect_status,
    )?;
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
    Response(&'a crate::protocol_types::NavigationResponse),
    Failure,
}

impl ImageSubresourceTerminal<'_> {
    fn resource_performance_entry(
        &self,
        request_url: &url::Url,
    ) -> crate::context_bootstrap::ResourcePerformanceEntry {
        match self {
            Self::Response(response) => {
                crate::context_bootstrap::ResourcePerformanceEntry::from_network_response(
                    request_url.as_str(),
                    "img",
                    None,
                    response,
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
        ImageSubresourceTerminal::Response(response) => {
            let descriptor = crate::network_host::image_response_descriptor(response);
            let completion = context_host
                .borrow_mut()
                .complete_pending_image_load_network_response_if_matches(
                    image_handle,
                    sequence,
                    internal_id,
                    descriptor,
                    response.body_bytes(),
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
                .child_browsing_context_request_scope(handle)
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

impl ScriptVm {
    pub(crate) fn should_intercept_parser_script_source_fetch(
        &self,
        script: &crate::planning::PreparedScript,
    ) -> bool {
        script.source_kind == crate::types::ScriptSourceKind::External
            && matches!(script.url.scheme(), "http" | "https")
            && self
                ._context_host
                .borrow()
                .should_intercept_subresource(SubresourceResourceType::Script)
    }

    pub(crate) fn start_parser_script_source_fetch_interception(
        &mut self,
        script: crate::planning::PreparedScript,
        request_client: ResourceRequestClient,
        task_runner: crate::network::RendererResourceTaskRunner,
        browser_context_runtime: crate::runtime::RendererBrowserContextRuntime,
        document_character_set: Option<String>,
    ) -> crate::planning::SharedScriptSourceLoad {
        // Main-parser source completion is routed by the exact parser
        // continuation registered on the returned load. The interception
        // request itself is frozen into this Page turn's concrete output
        // journal; it is never parked in browser-global state for a later
        // protocol snapshot to rediscover.
        let (load, completer) =
            crate::planning::SharedScriptSourceLoad::pending_with_owner_wake(None);
        let (info, continuation) = browser_context_runtime.prepare_detached_parser_script_fetch(
            PendingSubresourceFetchInfo {
                internal_id: 0,
                network_request_handle: None,
                frame_id: self.root_frame_id.clone(),
                document_url: script.initiator_url.clone(),
                url: script.url.clone(),
                websocket_socket_id: None,
                method: "GET".to_owned(),
                request_headers: Vec::new(),
                request_body: None,
                request_body_bytes: None,
                resource_type: SubresourceResourceType::Script,
                request_cookie_report: None,
            },
            script,
            request_client,
            task_runner,
            document_character_set,
            completer,
        );
        let source_document = self
            ._context_host
            .borrow()
            .root_document_lifecycle_identity()
            .expect("parser fetch interception requires an active root Document");
        let appended = self._context_host.borrow().append_live_turn_owner_action(
            crate::runtime::RendererOwnerAction::DetachedParserScriptFetchPause {
                source_document,
                info: Box::new(info),
                continuation,
            },
        );
        assert!(
            appended,
            "parser fetch interception requires a concrete Page output journal"
        );
        load
    }

    fn continue_worker_owned_fetch(
        &self,
        target: WorkerOwnedFetchTarget,
        request: crate::worker::WorkerPendingFetchContinue,
    ) -> bool {
        match target {
            WorkerOwnedFetchTarget::Dedicated { worker_id, .. } => self
                ._context_host
                .borrow_mut()
                .continue_worker_fetch(worker_id, request),
            WorkerOwnedFetchTarget::Shared { instance_id, .. } => self
                ._context_host
                .borrow_mut()
                .continue_shared_worker_fetch(instance_id, request),
        }
    }

    fn continue_worker_owned_xhr(
        &self,
        target: WorkerOwnedXhrTarget,
        request: crate::worker::WorkerPendingXhrContinue,
    ) -> bool {
        match target {
            WorkerOwnedXhrTarget::Dedicated { worker_id, .. } => self
                ._context_host
                .borrow_mut()
                .continue_worker_xhr(worker_id, request),
            WorkerOwnedXhrTarget::Shared { instance_id, .. } => self
                ._context_host
                .borrow_mut()
                .continue_shared_worker_xhr(instance_id, request),
        }
    }

    fn continue_worker_owned_csp_report(
        &self,
        target: WorkerOwnedCspReportTarget,
        request: crate::worker::WorkerPendingFetchContinue,
    ) -> bool {
        match target {
            WorkerOwnedCspReportTarget::Dedicated { worker_id, .. } => self
                ._context_host
                .borrow_mut()
                .continue_worker_csp_report(worker_id, request),
            WorkerOwnedCspReportTarget::Shared { instance_id, .. } => self
                ._context_host
                .borrow_mut()
                .continue_shared_worker_csp_report(instance_id, request),
        }
    }

    fn fail_worker_owned_fetch(
        &self,
        target: WorkerOwnedFetchTarget,
        request: crate::worker::WorkerPendingFetchContinue,
        error_text: String,
    ) -> bool {
        match target {
            WorkerOwnedFetchTarget::Dedicated { worker_id, .. } => self
                ._context_host
                .borrow_mut()
                .fail_worker_fetch(worker_id, request, error_text),
            WorkerOwnedFetchTarget::Shared { instance_id, .. } => self
                ._context_host
                .borrow_mut()
                .fail_shared_worker_fetch(instance_id, request, error_text),
        }
    }

    fn fail_worker_owned_xhr(
        &self,
        target: WorkerOwnedXhrTarget,
        request: crate::worker::WorkerPendingXhrContinue,
        error_text: String,
    ) -> bool {
        match target {
            WorkerOwnedXhrTarget::Dedicated { worker_id, .. } => self
                ._context_host
                .borrow_mut()
                .fail_worker_xhr(worker_id, request, error_text),
            WorkerOwnedXhrTarget::Shared { instance_id, .. } => self
                ._context_host
                .borrow_mut()
                .fail_shared_worker_xhr(instance_id, request, error_text),
        }
    }

    fn fail_worker_owned_csp_report(
        &self,
        target: WorkerOwnedCspReportTarget,
        request: crate::worker::WorkerPendingFetchContinue,
        error_text: String,
    ) -> bool {
        match target {
            WorkerOwnedCspReportTarget::Dedicated { worker_id, .. } => self
                ._context_host
                .borrow_mut()
                .fail_worker_csp_report(worker_id, request, error_text),
            WorkerOwnedCspReportTarget::Shared { instance_id, .. } => self
                ._context_host
                .borrow_mut()
                .fail_shared_worker_csp_report(instance_id, request, error_text),
        }
    }

    fn fail_worker_owned_fetch_auth(
        &self,
        target: WorkerOwnedFetchTarget,
        request: crate::worker::WorkerPendingFetchContinue,
        error_text: String,
    ) -> bool {
        match target {
            WorkerOwnedFetchTarget::Dedicated { worker_id, .. } => self
                ._context_host
                .borrow_mut()
                .fail_worker_fetch_auth(worker_id, request, error_text),
            WorkerOwnedFetchTarget::Shared { instance_id, .. } => self
                ._context_host
                .borrow_mut()
                .fail_shared_worker_fetch_auth(instance_id, request, error_text),
        }
    }

    fn fail_worker_owned_xhr_auth(
        &self,
        target: WorkerOwnedXhrTarget,
        request: crate::worker::WorkerPendingXhrContinue,
        error_text: String,
    ) -> bool {
        match target {
            WorkerOwnedXhrTarget::Dedicated { worker_id, .. } => self
                ._context_host
                .borrow_mut()
                .fail_worker_xhr_auth(worker_id, request, error_text),
            WorkerOwnedXhrTarget::Shared { instance_id, .. } => self
                ._context_host
                .borrow_mut()
                .fail_shared_worker_xhr_auth(instance_id, request, error_text),
        }
    }

    fn continue_worker_owned_fetch_response(
        &self,
        target: WorkerOwnedFetchTarget,
        request: crate::worker::WorkerPendingFetchContinue,
        response_code: Option<u16>,
        response_headers: Option<Vec<(String, String)>>,
    ) -> bool {
        match target {
            WorkerOwnedFetchTarget::Dedicated { worker_id, .. } => self
                ._context_host
                .borrow_mut()
                .continue_worker_fetch_response(
                    worker_id,
                    request,
                    response_code,
                    response_headers,
                ),
            WorkerOwnedFetchTarget::Shared { instance_id, .. } => self
                ._context_host
                .borrow_mut()
                .continue_shared_worker_fetch_response(
                    instance_id,
                    request,
                    response_code,
                    response_headers,
                ),
        }
    }

    fn continue_worker_owned_xhr_response(
        &self,
        target: WorkerOwnedXhrTarget,
        request: crate::worker::WorkerPendingXhrContinue,
        response_code: Option<u16>,
        response_headers: Option<Vec<(String, String)>>,
    ) -> bool {
        match target {
            WorkerOwnedXhrTarget::Dedicated { worker_id, .. } => self
                ._context_host
                .borrow_mut()
                .continue_worker_xhr_response(worker_id, request, response_code, response_headers),
            WorkerOwnedXhrTarget::Shared { instance_id, .. } => self
                ._context_host
                .borrow_mut()
                .continue_shared_worker_xhr_response(
                    instance_id,
                    request,
                    response_code,
                    response_headers,
                ),
        }
    }

    fn fail_worker_owned_fetch_response(
        &self,
        target: WorkerOwnedFetchTarget,
        request: crate::worker::WorkerPendingFetchContinue,
        error_text: String,
    ) -> bool {
        match target {
            WorkerOwnedFetchTarget::Dedicated { worker_id, .. } => self
                ._context_host
                .borrow_mut()
                .fail_worker_fetch_response(worker_id, request, error_text),
            WorkerOwnedFetchTarget::Shared { instance_id, .. } => self
                ._context_host
                .borrow_mut()
                .fail_shared_worker_fetch_response(instance_id, request, error_text),
        }
    }

    fn fail_worker_owned_xhr_response(
        &self,
        target: WorkerOwnedXhrTarget,
        request: crate::worker::WorkerPendingXhrContinue,
        error_text: String,
    ) -> bool {
        match target {
            WorkerOwnedXhrTarget::Dedicated { worker_id, .. } => self
                ._context_host
                .borrow_mut()
                .fail_worker_xhr_response(worker_id, request, error_text),
            WorkerOwnedXhrTarget::Shared { instance_id, .. } => self
                ._context_host
                .borrow_mut()
                .fail_shared_worker_xhr_response(instance_id, request, error_text),
        }
    }

    fn fulfill_worker_owned_fetch(
        &self,
        target: WorkerOwnedFetchTarget,
        request: crate::worker::WorkerPendingFetchContinue,
        response_code: u16,
        response_headers: Vec<(String, String)>,
        response_body: RendererSyntheticResponseBody,
    ) -> bool {
        match target {
            WorkerOwnedFetchTarget::Dedicated { worker_id, .. } => {
                self._context_host.borrow_mut().fulfill_worker_fetch(
                    worker_id,
                    request,
                    response_code,
                    response_headers,
                    response_body,
                )
            }
            WorkerOwnedFetchTarget::Shared { instance_id, .. } => {
                self._context_host.borrow_mut().fulfill_shared_worker_fetch(
                    instance_id,
                    request,
                    response_code,
                    response_headers,
                    response_body,
                )
            }
        }
    }

    fn fulfill_worker_owned_xhr(
        &self,
        target: WorkerOwnedXhrTarget,
        request: crate::worker::WorkerPendingXhrContinue,
        response_code: u16,
        response_headers: Vec<(String, String)>,
        response_body: RendererSyntheticResponseBody,
    ) -> bool {
        match target {
            WorkerOwnedXhrTarget::Dedicated { worker_id, .. } => {
                self._context_host.borrow_mut().fulfill_worker_xhr(
                    worker_id,
                    request,
                    response_code,
                    response_headers,
                    response_body,
                )
            }
            WorkerOwnedXhrTarget::Shared { instance_id, .. } => {
                self._context_host.borrow_mut().fulfill_shared_worker_xhr(
                    instance_id,
                    request,
                    response_code,
                    response_headers,
                    response_body,
                )
            }
        }
    }

    fn fulfill_worker_owned_csp_report(
        &self,
        target: WorkerOwnedCspReportTarget,
        request: crate::worker::WorkerPendingFetchContinue,
        response_code: u16,
        response_headers: Vec<(String, String)>,
        response_body: RendererSyntheticResponseBody,
    ) -> bool {
        match target {
            WorkerOwnedCspReportTarget::Dedicated { worker_id, .. } => {
                self._context_host.borrow_mut().fulfill_worker_csp_report(
                    worker_id,
                    request,
                    response_code,
                    response_headers,
                    response_body,
                )
            }
            WorkerOwnedCspReportTarget::Shared { instance_id, .. } => self
                ._context_host
                .borrow_mut()
                .fulfill_shared_worker_csp_report(
                    instance_id,
                    request,
                    response_code,
                    response_headers,
                    response_body,
                ),
        }
    }

    fn fulfill_worker_owned_fetch_response(
        &self,
        target: WorkerOwnedFetchTarget,
        request: crate::worker::WorkerPendingFetchContinue,
        response_code: u16,
        response_headers: Vec<(String, String)>,
        response_body: RendererSyntheticResponseBody,
    ) -> bool {
        match target {
            WorkerOwnedFetchTarget::Dedicated { worker_id, .. } => self
                ._context_host
                .borrow_mut()
                .fulfill_worker_fetch_response(
                    worker_id,
                    request,
                    response_code,
                    response_headers,
                    response_body,
                ),
            WorkerOwnedFetchTarget::Shared { instance_id, .. } => self
                ._context_host
                .borrow_mut()
                .fulfill_shared_worker_fetch_response(
                    instance_id,
                    request,
                    response_code,
                    response_headers,
                    response_body,
                ),
        }
    }

    fn fulfill_worker_owned_xhr_response(
        &self,
        target: WorkerOwnedXhrTarget,
        request: crate::worker::WorkerPendingXhrContinue,
        response_code: u16,
        response_headers: Vec<(String, String)>,
        response_body: RendererSyntheticResponseBody,
    ) -> bool {
        match target {
            WorkerOwnedXhrTarget::Dedicated { worker_id, .. } => {
                self._context_host.borrow_mut().fulfill_worker_xhr_response(
                    worker_id,
                    request,
                    response_code,
                    response_headers,
                    response_body,
                )
            }
            WorkerOwnedXhrTarget::Shared { instance_id, .. } => self
                ._context_host
                .borrow_mut()
                .fulfill_shared_worker_xhr_response(
                    instance_id,
                    request,
                    response_code,
                    response_headers,
                    response_body,
                ),
        }
    }

    pub(crate) fn continue_pending_subresource_fetch_body(
        &mut self,
        internal_id: u64,
        url: Option<Url>,
        method: Option<String>,
        body: Option<Option<String>>,
        headers: Option<Vec<(String, String)>>,
        intercept_response: bool,
        handle_auth_requests: bool,
    ) -> Result<AsyncSubresourceCommandExecution<PendingSubresourceContinueOutcome>> {
        let pending = self
            ._context_host
            .borrow_mut()
            .take_pending_subresource_fetch(internal_id)
            .ok_or_else(|| anyhow!("unknown pending subresource fetch `{internal_id}`"))?;
        let PendingSubresourceFetchState {
            info,
            load,
            execution_context,
            credentials_mode,
            request_mode,
            network_partition_key,
            policy_context,
            continuation,
            deferred_request_started,
        } = pending;
        let pending = match continuation {
            PendingSubresourceContinuation::WebSocket(connection) => {
                let request_url = url.unwrap_or_else(|| info.url.clone());
                let headers_overridden = headers.is_some();
                let request_headers = headers.unwrap_or_else(|| info.request_headers.clone());
                self._context_host
                    .borrow_mut()
                    .start_pending_websocket_connection(
                        connection,
                        request_url,
                        request_headers,
                        headers_overridden,
                        intercept_response,
                    )
                    .map_err(|error| anyhow!(error))?;
                return Ok(AsyncSubresourceCommandExecution::without_window_realm(
                    PendingSubresourceContinueOutcome::Started,
                ));
            }
            continuation @ (PendingSubresourceContinuation::WorkerFetch { .. }
            | PendingSubresourceContinuation::SharedWorkerFetch { .. }) => {
                let target = WorkerOwnedFetchTarget::from_continuation(&continuation)
                    .expect("fetch continuation target");
                let fetch_id = target.fetch_id();
                let request_url = url.unwrap_or_else(|| info.url.clone());
                let request_method = method.unwrap_or_else(|| info.method.clone());
                let request_body = body.unwrap_or_else(|| info.request_body.clone());
                let request_headers = headers.unwrap_or_else(|| info.request_headers.clone());
                let continued = self.continue_worker_owned_fetch(
                    target,
                    crate::worker::WorkerPendingFetchContinue {
                        fetch_id,
                        internal_id,
                        network_request_handle: info.network_request_handle,
                        url: request_url.clone(),
                        method: request_method.clone(),
                        body: request_body.clone(),
                        headers: request_headers.clone(),
                        intercept_response,
                        handle_auth_requests,
                        auth: None,
                    },
                );
                if !continued {
                    bail!(target.unavailable_message());
                }
                if intercept_response || handle_auth_requests {
                    self._context_host
                        .borrow_mut()
                        .record_in_flight_worker_subresource_fetch(
                            crate::types::InFlightWorkerSubresourceFetchState {
                                pending: PendingSubresourceFetchState {
                                    info,
                                    load,
                                    execution_context,
                                    credentials_mode,
                                    request_mode,
                                    network_partition_key: network_partition_key.clone(),
                                    policy_context,
                                    continuation: target.continuation(),
                                    deferred_request_started,
                                },
                                request_url,
                                request_method,
                                request_headers,
                                request_body,
                            },
                        );
                }
                return Ok(AsyncSubresourceCommandExecution::without_window_realm(
                    PendingSubresourceContinueOutcome::Started,
                ));
            }
            continuation @ (PendingSubresourceContinuation::WorkerXhr { .. }
            | PendingSubresourceContinuation::SharedWorkerXhr { .. }) => {
                let target = WorkerOwnedXhrTarget::from_continuation(&continuation)
                    .expect("xhr continuation target");
                let xhr_id = target.xhr_id();
                let request_url = url.unwrap_or_else(|| info.url.clone());
                let request_method = method.unwrap_or_else(|| info.method.clone());
                let request_body = body.unwrap_or_else(|| info.request_body.clone());
                let request_headers = headers.unwrap_or_else(|| info.request_headers.clone());
                let continued = self.continue_worker_owned_xhr(
                    target,
                    crate::worker::WorkerPendingXhrContinue {
                        xhr_id,
                        internal_id,
                        network_request_handle: info.network_request_handle,
                        url: request_url.clone(),
                        method: request_method.clone(),
                        body: request_body.clone(),
                        headers: request_headers.clone(),
                        intercept_response,
                        handle_auth_requests,
                        auth: None,
                    },
                );
                if !continued {
                    bail!(target.unavailable_message());
                }
                if intercept_response || handle_auth_requests {
                    self._context_host
                        .borrow_mut()
                        .record_in_flight_worker_subresource_fetch(
                            crate::types::InFlightWorkerSubresourceFetchState {
                                pending: PendingSubresourceFetchState {
                                    info,
                                    load,
                                    execution_context,
                                    credentials_mode,
                                    request_mode,
                                    network_partition_key: network_partition_key.clone(),
                                    policy_context,
                                    continuation: target.continuation(),
                                    deferred_request_started,
                                },
                                request_url,
                                request_method,
                                request_headers,
                                request_body,
                            },
                        );
                }
                return Ok(AsyncSubresourceCommandExecution::without_window_realm(
                    PendingSubresourceContinueOutcome::Started,
                ));
            }
            continuation @ (PendingSubresourceContinuation::WorkerCspReport { .. }
            | PendingSubresourceContinuation::SharedWorkerCspReport { .. }) => {
                let target = WorkerOwnedCspReportTarget::from_continuation(&continuation)
                    .expect("CSP report continuation target");
                let report_id = target.report_id();
                let request_url = url.unwrap_or_else(|| info.url.clone());
                let request_method = method.unwrap_or_else(|| info.method.clone());
                let request_body = body.unwrap_or_else(|| info.request_body.clone());
                let request_headers = headers.unwrap_or_else(|| info.request_headers.clone());
                let continued = self.continue_worker_owned_csp_report(
                    target,
                    crate::worker::WorkerPendingFetchContinue {
                        fetch_id: report_id,
                        internal_id,
                        network_request_handle: info.network_request_handle,
                        url: request_url,
                        method: request_method,
                        body: request_body,
                        headers: request_headers,
                        intercept_response: false,
                        handle_auth_requests: false,
                        auth: None,
                    },
                );
                if !continued {
                    bail!(target.unavailable_message());
                }
                return Ok(AsyncSubresourceCommandExecution::without_window_realm(
                    PendingSubresourceContinueOutcome::Started,
                ));
            }
            PendingSubresourceContinuation::CspReport { client_id } => {
                let request_url = url.unwrap_or_else(|| info.url.clone());
                let request_method = method.unwrap_or_else(|| info.method.clone());
                let request_body_bytes = match &body {
                    Some(Some(body)) => Some(body.as_bytes().to_vec()),
                    Some(None) => None,
                    None => info.request_body_bytes.clone(),
                };
                let request_body = body.unwrap_or_else(|| info.request_body.clone());
                let request_headers = headers.unwrap_or_else(|| info.request_headers.clone());
                let pending = PendingSubresourceFetchState {
                    info,
                    load,
                    execution_context,
                    credentials_mode,
                    request_mode,
                    network_partition_key,
                    policy_context,
                    continuation: PendingSubresourceContinuation::CspReport { client_id },
                    deferred_request_started,
                };
                if !self._context_host.borrow().network_offline() {
                    let maybe_pending = self.continue_csp_report_via_service_worker(
                        pending,
                        client_id,
                        request_url.clone(),
                        request_method.clone(),
                        request_headers.clone(),
                        request_body.clone(),
                        request_body_bytes,
                    )?;
                    let Some(pending) = maybe_pending else {
                        return Ok(AsyncSubresourceCommandExecution::without_window_realm(
                            PendingSubresourceContinueOutcome::Started,
                        ));
                    };
                    return self.continue_pending_subresource_fetch_via_loader(
                        pending,
                        request_url,
                        request_method,
                        request_headers,
                        request_body,
                        intercept_response,
                        handle_auth_requests,
                    );
                }
                return self.continue_pending_subresource_fetch_via_loader(
                    pending,
                    request_url,
                    request_method,
                    request_headers,
                    request_body,
                    intercept_response,
                    handle_auth_requests,
                );
            }
            continuation => PendingSubresourceFetchState {
                info,
                load,
                execution_context,
                credentials_mode,
                request_mode,
                network_partition_key,
                policy_context,
                continuation,
                deferred_request_started,
            },
        };
        let request_url = url.unwrap_or_else(|| pending.info.url.clone());
        let request_method = method.unwrap_or_else(|| pending.info.method.clone());
        let request_body = body.unwrap_or_else(|| pending.info.request_body.clone());
        let request_headers = headers.unwrap_or_else(|| pending.info.request_headers.clone());
        self.continue_pending_subresource_fetch_via_loader(
            pending,
            request_url,
            request_method,
            request_headers,
            request_body,
            intercept_response,
            handle_auth_requests,
        )
    }

    fn continue_pending_subresource_fetch_via_loader(
        &mut self,
        pending: PendingSubresourceFetchState,
        request_url: Url,
        request_method: String,
        request_headers: Vec<(String, String)>,
        request_body: Option<String>,
        intercept_response: bool,
        handle_auth_requests: bool,
    ) -> Result<AsyncSubresourceCommandExecution<PendingSubresourceContinueOutcome>> {
        let internal_id = pending.info.internal_id;
        if pending.load.network_offline() {
            let activity = self.resolve_pending_subresource_fetch_body(
                pending,
                request_url,
                request_method,
                request_headers,
                request_body,
                None,
                false,
                None,
                None,
                Err("Network emulation offline".to_owned()),
            )?;
            return Ok(AsyncSubresourceCommandExecution::after_body(
                PendingSubresourceContinueOutcome::Started,
                activity,
            )
            .with_post_checkpoint_event(PendingSubresourceContinueEvent::Completed {
                internal_id,
            }));
        }
        // Request interception may resume after the initiating Document has
        // navigated. The lease retains the request-time frozen client; looking
        // up the ambient Page loader here would silently rebind policy/backend
        // to a newer Document identity.
        let loader = pending.load.request_client();
        let mut request = moli_fetch::Request::new(
            &request_method,
            request_url.as_str(),
            request_body.clone(),
            request_headers.clone(),
        )?
        .with_initiator_url(&pending.info.document_url)
        .with_request_mode(pending.request_mode)
        .with_credentials_mode(pending.credentials_mode)
        .with_network_partition_key(pending.network_partition_key.clone())
        .with_subframe_context(pending.info.frame_id.is_some());
        request = match pending.info.resource_type {
            SubresourceResourceType::Script
            | SubresourceResourceType::Stylesheet
            | SubresourceResourceType::Image
            | SubresourceResourceType::Font
            | SubresourceResourceType::Audio
            | SubresourceResourceType::Video
            | SubresourceResourceType::Media
            | SubresourceResourceType::TextTrack
            | SubresourceResourceType::Ping
            | SubresourceResourceType::CspReport
            | SubresourceResourceType::Dictionary => {
                match crate::network::request_resource_type_for_subresource(
                    pending.info.resource_type,
                ) {
                    Some(resource_type) => request.with_resource_type(resource_type),
                    None => request,
                }
            }
            SubresourceResourceType::Fetch => {
                request.with_browser_request_metadata(moli_fetch::BrowserRequestMetadata::Fetch)
            }
            SubresourceResourceType::Manifest => request
                .with_resource_type(moli_fetch::RequestResourceType::Manifest)
                .with_browser_request_metadata(moli_fetch::BrowserRequestMetadata::Manifest),
            SubresourceResourceType::EventSource => request
                .with_browser_request_metadata(moli_fetch::BrowserRequestMetadata::EventSource)
                .with_cache_mode(moli_fetch::RequestCacheMode::NoStore)
                .without_request_timeout(),
            SubresourceResourceType::Xhr => {
                request.with_browser_request_metadata(moli_fetch::BrowserRequestMetadata::Xhr)
            }
            SubresourceResourceType::WebSocket => request,
        };
        let cancel_handle = moli_fetch::FetchCancelHandle::new();
        pending.load.attach_cancel_handle(cancel_handle.clone());
        self.spawn_running_subresource_fetch(
            loader,
            request,
            RunningSubresourceFetchState {
                pending,
                request_url,
                request_method,
                request_headers,
                request_body,
                intercept_response,
                handle_auth_requests,
                initial_auth_network_request_headers: None,
            },
            Some(cancel_handle),
        );
        Ok(AsyncSubresourceCommandExecution::without_window_realm(
            PendingSubresourceContinueOutcome::Started,
        ))
    }

    fn continue_csp_report_via_service_worker(
        &mut self,
        pending: PendingSubresourceFetchState,
        client_id: crate::service_worker_runtime::ServiceWorkerClientId,
        request_url: Url,
        request_method: String,
        request_headers: Vec<(String, String)>,
        request_body: Option<String>,
        request_body_bytes: Option<Vec<u8>>,
    ) -> Result<Option<PendingSubresourceFetchState>> {
        if self
            ._context_host
            .borrow()
            .service_worker_controller_for_fetch(
                client_id,
                &pending.info.document_url,
                &request_url,
            )
            .is_none()
        {
            return Ok(Some(pending));
        }

        let mut request = moli_fetch::Request::new_bytes(
            &request_method,
            request_url.as_str(),
            request_body_bytes,
            request_headers.clone(),
        )?
        .with_initiator_url(&pending.info.document_url)
        .with_resource_type(moli_fetch::RequestResourceType::CspReport)
        .with_request_mode(pending.request_mode)
        .with_credentials_mode(pending.credentials_mode)
        .with_network_partition_key(pending.network_partition_key.clone())
        .with_redirect_mode(moli_fetch::RequestRedirectMode::Error)
        .with_subframe_context(pending.info.frame_id.is_some());
        request.priority_hints.fetch_priority = None;

        let cancel_handle = moli_fetch::FetchCancelHandle::new();
        pending.load.attach_cancel_handle(cancel_handle.clone());
        let internal_id = pending.info.internal_id;
        let policy_context = pending.policy_context;
        let request_cookie_report = pending.info.request_cookie_report.clone();
        let document_url = pending.info.document_url.clone();
        let frame_id = pending.info.frame_id.clone();
        let completion_tx = self._context_host.borrow().resource_completion_sender();
        let request_client = pending.load.request_client();
        let resource_task_runner = pending.load.task_runner();
        let dispatch = crate::service_worker_runtime::ServiceWorkerFetchDispatch {
            internal_id,
            request: self._context_host.borrow().service_worker_fetch_request(
                client_id,
                request.url.clone(),
                request.method.clone(),
                request.request_headers.clone(),
                request.body.clone(),
                crate::service_worker_runtime::ServiceWorkerRequestDestination::Report,
                request.request_mode,
                request.credentials_mode,
                request.redirect_mode,
                request.priority_hints.fetch_priority,
                crate::service_worker_runtime::service_worker_fetch_request_metadata(&request),
            ),
            request_body_text: request_body.clone(),
            cors_preflight_request_headers: Vec::new(),
            request_cookie_report,
            network_context: crate::types::AsyncSubresourceNetworkContext {
                frame_id,
                document_url,
                resource_type: SubresourceResourceType::CspReport,
                policy_context,
            },
            completion_tx,
            request_client,
            resource_task_runner,
            cancel_handle,
            direct_completion_tx: None,
        };

        self._context_host
            .borrow_mut()
            .restore_pending_subresource_fetch(pending);
        if self
            ._context_host
            .borrow()
            .dispatch_service_worker_fetch(dispatch)
        {
            return Ok(None);
        }

        let _ = self
            ._context_host
            .borrow()
            .resource_completion_sender()
            .send_async_subresource(AsyncSubresourceFetchCompletion {
                internal_id,
                request_url,
                request_method,
                request_headers,
                request_body,
                response_status_text: None,
                skip_fetch_security_validation: true,
                response_filter: Default::default(),
                network_error_text: None,
                result: Err("service worker csp report fetch dispatch failed".to_owned()),
            });
        Ok(None)
    }

    pub(crate) fn continue_pending_subresource_auth_body(
        &mut self,
        internal_id: u64,
        auth: crate::SubresourceAuthCredentials,
    ) -> Result<AsyncSubresourceCommandExecution<PendingSubresourceContinueOutcome>> {
        let pending = self
            ._context_host
            .borrow_mut()
            .take_pending_subresource_auth(internal_id)
            .ok_or_else(|| anyhow!("unknown pending subresource auth `{internal_id}`"))?;
        let PendingSubresourceAuthState {
            pending: pending_fetch,
            request_url,
            request_method,
            request_headers: original_request_headers,
            request_body,
            intercept_response,
            initial_network_request_headers,
            response: _,
        } = pending;
        if let Some(target) = WorkerOwnedFetchTarget::from_continuation(&pending_fetch.continuation)
        {
            let request_headers = original_request_headers;
            let continued = self.continue_worker_owned_fetch(
                target,
                crate::worker::WorkerPendingFetchContinue {
                    fetch_id: target.fetch_id(),
                    internal_id,
                    network_request_handle: pending_fetch.info.network_request_handle,
                    url: request_url.clone(),
                    method: request_method.clone(),
                    body: request_body.clone(),
                    headers: request_headers.clone(),
                    intercept_response,
                    handle_auth_requests: true,
                    auth: Some(auth),
                },
            );
            if !continued {
                bail!(target.unavailable_message());
            }
            self._context_host
                .borrow_mut()
                .record_in_flight_worker_subresource_fetch(
                    crate::types::InFlightWorkerSubresourceFetchState {
                        pending: pending_fetch,
                        request_url,
                        request_method,
                        request_headers,
                        request_body,
                    },
                );
            return Ok(AsyncSubresourceCommandExecution::without_window_realm(
                PendingSubresourceContinueOutcome::Started,
            ));
        }
        if let Some(target) = WorkerOwnedXhrTarget::from_continuation(&pending_fetch.continuation) {
            let request_headers = original_request_headers;
            let continued = self.continue_worker_owned_xhr(
                target,
                crate::worker::WorkerPendingXhrContinue {
                    xhr_id: target.xhr_id(),
                    internal_id,
                    network_request_handle: pending_fetch.info.network_request_handle,
                    url: request_url.clone(),
                    method: request_method.clone(),
                    body: request_body.clone(),
                    headers: request_headers.clone(),
                    intercept_response,
                    handle_auth_requests: true,
                    auth: Some(auth),
                },
            );
            if !continued {
                bail!(target.unavailable_message());
            }
            self._context_host
                .borrow_mut()
                .record_in_flight_worker_subresource_fetch(
                    crate::types::InFlightWorkerSubresourceFetchState {
                        pending: pending_fetch,
                        request_url,
                        request_method,
                        request_headers,
                        request_body,
                    },
                );
            return Ok(AsyncSubresourceCommandExecution::without_window_realm(
                PendingSubresourceContinueOutcome::Started,
            ));
        }
        let loader = pending_fetch.load.request_client();
        let mut request = moli_fetch::Request::new(
            &request_method,
            request_url.as_str(),
            request_body.clone(),
            original_request_headers.clone(),
        )?
        .with_initiator_url(&pending_fetch.info.document_url)
        .with_request_mode(pending_fetch.request_mode)
        .with_credentials_mode(pending_fetch.credentials_mode)
        .with_auth(auth.into())
        .with_subframe_context(pending_fetch.info.frame_id.is_some());
        request = match pending_fetch.info.resource_type {
            SubresourceResourceType::Script
            | SubresourceResourceType::Stylesheet
            | SubresourceResourceType::Image
            | SubresourceResourceType::Font
            | SubresourceResourceType::Audio
            | SubresourceResourceType::Video
            | SubresourceResourceType::Media
            | SubresourceResourceType::TextTrack
            | SubresourceResourceType::Ping
            | SubresourceResourceType::CspReport
            | SubresourceResourceType::Dictionary => {
                match crate::network::request_resource_type_for_subresource(
                    pending_fetch.info.resource_type,
                ) {
                    Some(resource_type) => request.with_resource_type(resource_type),
                    None => request,
                }
            }
            SubresourceResourceType::Fetch => {
                request.with_browser_request_metadata(moli_fetch::BrowserRequestMetadata::Fetch)
            }
            SubresourceResourceType::Manifest => request
                .with_resource_type(moli_fetch::RequestResourceType::Manifest)
                .with_browser_request_metadata(moli_fetch::BrowserRequestMetadata::Manifest),
            SubresourceResourceType::EventSource => request
                .with_browser_request_metadata(moli_fetch::BrowserRequestMetadata::EventSource)
                .with_cache_mode(moli_fetch::RequestCacheMode::NoStore)
                .without_request_timeout(),
            SubresourceResourceType::Xhr => {
                request.with_browser_request_metadata(moli_fetch::BrowserRequestMetadata::Xhr)
            }
            SubresourceResourceType::WebSocket => request,
        };
        let request_headers = original_request_headers;
        if self._context_host.borrow().network_offline() {
            let activity = self.resolve_pending_subresource_fetch_body(
                pending_fetch,
                request_url,
                request_method,
                request_headers,
                request_body,
                None,
                false,
                None,
                None,
                Err("Network emulation offline".to_owned()),
            )?;
            return Ok(AsyncSubresourceCommandExecution::after_body(
                PendingSubresourceContinueOutcome::Started,
                activity,
            )
            .with_post_checkpoint_event(PendingSubresourceContinueEvent::Completed {
                internal_id,
            }));
        }
        let cancel_handle = moli_fetch::FetchCancelHandle::new();
        pending_fetch
            .load
            .attach_cancel_handle(cancel_handle.clone());
        self.spawn_running_subresource_fetch(
            loader,
            request,
            RunningSubresourceFetchState {
                pending: pending_fetch,
                request_url,
                request_method,
                request_headers,
                request_body,
                intercept_response,
                handle_auth_requests: true,
                initial_auth_network_request_headers: initial_network_request_headers,
            },
            Some(cancel_handle),
        );
        Ok(AsyncSubresourceCommandExecution::without_window_realm(
            PendingSubresourceContinueOutcome::Started,
        ))
    }

    pub(crate) fn fail_pending_subresource_auth_body(
        &mut self,
        internal_id: u64,
        error_text: String,
    ) -> Result<AsyncSubresourceCommandExecution<()>> {
        let pending = self
            ._context_host
            .borrow_mut()
            .take_pending_subresource_auth(internal_id)
            .ok_or_else(|| anyhow!("unknown pending subresource auth `{internal_id}`"))?;
        if let Some(target) =
            WorkerOwnedFetchTarget::from_continuation(&pending.pending.continuation)
        {
            let result = Err(error_text.clone());
            self.record_subresource_fetch_network_result(
                &pending.pending,
                &pending.request_url,
                &pending.request_method,
                &pending.request_headers,
                &pending.request_body,
                &result,
            );
            let failed = self.fail_worker_owned_fetch_auth(
                target,
                crate::worker::WorkerPendingFetchContinue {
                    fetch_id: target.fetch_id(),
                    internal_id,
                    network_request_handle: pending.pending.info.network_request_handle,
                    url: pending.request_url,
                    method: pending.request_method,
                    body: pending.request_body,
                    headers: pending.request_headers,
                    intercept_response: false,
                    handle_auth_requests: false,
                    auth: None,
                },
                error_text,
            );
            if !failed {
                bail!(target.unavailable_message());
            }
            return Ok(AsyncSubresourceCommandExecution::without_window_realm(()));
        }
        if let Some(target) = WorkerOwnedXhrTarget::from_continuation(&pending.pending.continuation)
        {
            let result = Err(error_text.clone());
            self.record_subresource_fetch_network_result(
                &pending.pending,
                &pending.request_url,
                &pending.request_method,
                &pending.request_headers,
                &pending.request_body,
                &result,
            );
            let failed = self.fail_worker_owned_xhr_auth(
                target,
                crate::worker::WorkerPendingXhrContinue {
                    xhr_id: target.xhr_id(),
                    internal_id,
                    network_request_handle: pending.pending.info.network_request_handle,
                    url: pending.request_url,
                    method: pending.request_method,
                    body: pending.request_body,
                    headers: pending.request_headers,
                    intercept_response: false,
                    handle_auth_requests: false,
                    auth: None,
                },
                error_text,
            );
            if !failed {
                bail!(target.unavailable_message());
            }
            return Ok(AsyncSubresourceCommandExecution::without_window_realm(()));
        }
        let activity = self.resolve_pending_subresource_fetch_body(
            pending.pending,
            pending.request_url,
            pending.request_method,
            pending.request_headers,
            pending.request_body,
            None,
            false,
            None,
            None,
            Err(error_text),
        )?;
        Ok(AsyncSubresourceCommandExecution::after_body((), activity))
    }

    pub(crate) fn cancel_pending_subresource_auth_body(
        &mut self,
        internal_id: u64,
    ) -> Result<AsyncSubresourceCommandExecution<()>> {
        let pending = self
            ._context_host
            .borrow_mut()
            .take_pending_subresource_auth(internal_id)
            .ok_or_else(|| anyhow!("unknown pending subresource auth `{internal_id}`"))?;
        let PendingSubresourceAuthState {
            pending,
            request_url,
            request_method,
            request_headers,
            request_body,
            intercept_response,
            initial_network_request_headers,
            response,
        } = pending;
        let response = match initial_network_request_headers {
            Some(headers) => response.with_network_request_headers(Some(headers)),
            None => response,
        };
        let response_info = PendingSubresourceResponseInfo {
            internal_id,
            url: request_url.clone(),
            final_url: response.final_url.clone(),
            method: request_method.clone(),
            request_headers: request_headers.clone(),
            request_body: request_body.clone(),
            resource_type: pending.info.resource_type,
            request_cookie_report: response.request_cookie_report.clone(),
            network_request_headers: response
                .network_request_headers()
                .map(|headers| headers.to_vec()),
            response_status: response.status,
            response_headers: response.headers.clone(),
            response_body: SubresourceResponseBody::from_navigation_response(&response),
            from_cache: response.from_cache,
        };
        self._context_host
            .borrow_mut()
            .record_pending_subresource_response(PendingSubresourceResponseState {
                pending,
                request_url,
                request_method,
                request_headers,
                request_body,
                response,
            });
        if intercept_response {
            self._context_host
                .borrow_mut()
                .record_pending_subresource_continue_event(
                    PendingSubresourceContinueEvent::ResponsePaused(response_info),
                );
            return Ok(AsyncSubresourceCommandExecution::without_window_realm(()));
        }
        self.continue_pending_subresource_response_body(internal_id, None, None)
    }

    pub(crate) fn fail_pending_subresource_fetch_body(
        &mut self,
        internal_id: u64,
        error_text: String,
    ) -> Result<AsyncSubresourceCommandExecution<()>> {
        let pending = self
            ._context_host
            .borrow_mut()
            .take_pending_subresource_fetch(internal_id)
            .ok_or_else(|| anyhow!("unknown pending subresource fetch `{internal_id}`"))?;
        let PendingSubresourceFetchState {
            info,
            load,
            execution_context,
            credentials_mode,
            request_mode,
            network_partition_key,
            policy_context,
            continuation,
            deferred_request_started,
        } = pending;
        let pending = match continuation {
            PendingSubresourceContinuation::WebSocket(connection) => {
                self._context_host
                    .borrow_mut()
                    .fail_pending_websocket_connection(connection, error_text)
                    .map_err(|error| anyhow!(error))?;
                return Ok(AsyncSubresourceCommandExecution::without_window_realm(()));
            }
            continuation @ (PendingSubresourceContinuation::WorkerFetch { .. }
            | PendingSubresourceContinuation::SharedWorkerFetch { .. }) => {
                let target = WorkerOwnedFetchTarget::from_continuation(&continuation)
                    .expect("fetch continuation target");
                let failed = self.fail_worker_owned_fetch(
                    target,
                    crate::worker::WorkerPendingFetchContinue {
                        fetch_id: target.fetch_id(),
                        internal_id: 0,
                        network_request_handle: info.network_request_handle,
                        url: info.url.clone(),
                        method: info.method.clone(),
                        body: info.request_body.clone(),
                        headers: info.request_headers.clone(),
                        intercept_response: false,
                        handle_auth_requests: false,
                        auth: None,
                    },
                    error_text.clone(),
                );
                if !failed {
                    bail!(target.unavailable_message());
                }
                let request_body_bytes = info.request_body_bytes.clone();
                let request_handle = info.network_request_handle;
                let network_record = with_pending_subresource_record_identity(
                    crate::types::SubresourceNetworkRecord::failure(
                        info.frame_id,
                        info.document_url,
                        info.url,
                        info.method,
                        info.request_headers,
                        info.request_body,
                        info.resource_type,
                        error_text,
                    ),
                    request_body_bytes,
                    request_handle,
                );
                self._context_host
                    .borrow_mut()
                    .record_subresource_network(network_record);
                return Ok(AsyncSubresourceCommandExecution::without_window_realm(()));
            }
            continuation @ (PendingSubresourceContinuation::WorkerXhr { .. }
            | PendingSubresourceContinuation::SharedWorkerXhr { .. }) => {
                let target = WorkerOwnedXhrTarget::from_continuation(&continuation)
                    .expect("xhr continuation target");
                let failed = self.fail_worker_owned_xhr(
                    target,
                    crate::worker::WorkerPendingXhrContinue {
                        xhr_id: target.xhr_id(),
                        internal_id: 0,
                        network_request_handle: info.network_request_handle,
                        url: info.url.clone(),
                        method: info.method.clone(),
                        body: info.request_body.clone(),
                        headers: info.request_headers.clone(),
                        intercept_response: false,
                        handle_auth_requests: false,
                        auth: None,
                    },
                    error_text.clone(),
                );
                if !failed {
                    bail!(target.unavailable_message());
                }
                let request_body_bytes = info.request_body_bytes.clone();
                let request_handle = info.network_request_handle;
                let network_record = with_pending_subresource_record_identity(
                    crate::types::SubresourceNetworkRecord::failure(
                        info.frame_id,
                        info.document_url,
                        info.url,
                        info.method,
                        info.request_headers,
                        info.request_body,
                        info.resource_type,
                        error_text,
                    ),
                    request_body_bytes,
                    request_handle,
                );
                self._context_host
                    .borrow_mut()
                    .record_subresource_network(network_record);
                return Ok(AsyncSubresourceCommandExecution::without_window_realm(()));
            }
            continuation @ (PendingSubresourceContinuation::WorkerCspReport { .. }
            | PendingSubresourceContinuation::SharedWorkerCspReport { .. }) => {
                let target = WorkerOwnedCspReportTarget::from_continuation(&continuation)
                    .expect("CSP report continuation target");
                let failed = self.fail_worker_owned_csp_report(
                    target,
                    crate::worker::WorkerPendingFetchContinue {
                        fetch_id: target.report_id(),
                        internal_id: 0,
                        network_request_handle: info.network_request_handle,
                        url: info.url.clone(),
                        method: info.method.clone(),
                        body: info.request_body.clone(),
                        headers: info.request_headers.clone(),
                        intercept_response: false,
                        handle_auth_requests: false,
                        auth: None,
                    },
                    error_text.clone(),
                );
                if !failed {
                    bail!(target.unavailable_message());
                }
                let request_body_bytes = info.request_body_bytes.clone();
                let request_handle = info.network_request_handle;
                let network_record = with_pending_subresource_record_identity(
                    crate::types::SubresourceNetworkRecord::failure(
                        info.frame_id,
                        info.document_url,
                        info.url,
                        info.method,
                        info.request_headers,
                        info.request_body,
                        info.resource_type,
                        error_text,
                    ),
                    request_body_bytes,
                    request_handle,
                );
                self._context_host
                    .borrow_mut()
                    .record_subresource_network(network_record);
                return Ok(AsyncSubresourceCommandExecution::without_window_realm(()));
            }
            continuation => PendingSubresourceFetchState {
                info,
                load,
                execution_context,
                credentials_mode,
                request_mode,
                network_partition_key,
                policy_context,
                continuation,
                deferred_request_started,
            },
        };
        let info = pending.info.clone();
        let activity = self.resolve_pending_subresource_fetch_body(
            pending,
            info.url,
            info.method,
            info.request_headers,
            info.request_body,
            None,
            false,
            None,
            None,
            Err(error_text),
        )?;
        Ok(AsyncSubresourceCommandExecution::after_body((), activity))
    }

    pub(crate) fn fulfill_pending_subresource_fetch_body(
        &mut self,
        internal_id: u64,
        response_code: u16,
        response_headers: Vec<(String, String)>,
        response_body: RendererSyntheticResponseBody,
    ) -> Result<AsyncSubresourceCommandExecution<()>> {
        let pending = self
            ._context_host
            .borrow_mut()
            .take_pending_subresource_fetch(internal_id)
            .ok_or_else(|| anyhow!("unknown pending subresource fetch `{internal_id}`"))?;
        let PendingSubresourceFetchState {
            info,
            load,
            execution_context,
            credentials_mode,
            request_mode,
            network_partition_key,
            policy_context,
            continuation,
            deferred_request_started,
        } = pending;
        let pending = match continuation {
            PendingSubresourceContinuation::WebSocket(connection) => {
                if response_code != 101 {
                    self._context_host
                        .borrow_mut()
                        .fail_pending_websocket_connection(
                            connection,
                            format!(
                                "Fetch.fulfillRequest WebSocket response must use status 101, got {response_code}"
                            ),
                        )
                        .map_err(|error| anyhow!(error))?;
                    return Ok(AsyncSubresourceCommandExecution::without_window_realm(()));
                }
                let request_url = info.url.clone();
                let request_headers = info.request_headers.clone();
                self._context_host
                    .borrow_mut()
                    .fulfill_pending_websocket_connection(
                        connection,
                        request_url,
                        request_headers,
                        response_code,
                        response_headers.clone(),
                    )
                    .map_err(|error| anyhow!(error))?;
                let _ = response_body;
                return Ok(AsyncSubresourceCommandExecution::without_window_realm(()));
            }
            continuation @ (PendingSubresourceContinuation::WorkerFetch { .. }
            | PendingSubresourceContinuation::SharedWorkerFetch { .. }) => {
                let target = WorkerOwnedFetchTarget::from_continuation(&continuation)
                    .expect("fetch continuation target");
                let request_url = info.url.clone();
                let request_method = info.method.clone();
                let request_headers = info.request_headers.clone();
                let request_body = info.request_body.clone();
                let validation = if request_mode == moli_fetch::RequestMode::NoCors {
                    Ok(())
                } else {
                    crate::network_host::validate_cors_response(
                        &info.document_url,
                        &info.url,
                        &response_headers,
                        credentials_mode,
                    )
                };
                match validation {
                    Ok(()) => {
                        let fulfilled = self.fulfill_worker_owned_fetch(
                            target,
                            crate::worker::WorkerPendingFetchContinue {
                                fetch_id: target.fetch_id(),
                                internal_id: 0,
                                network_request_handle: info.network_request_handle,
                                url: request_url.clone(),
                                method: request_method.clone(),
                                body: request_body.clone(),
                                headers: request_headers.clone(),
                                intercept_response: false,
                                handle_auth_requests: false,
                                auth: None,
                            },
                            response_code,
                            response_headers.clone(),
                            response_body.clone(),
                        );
                        if !fulfilled {
                            bail!(target.unavailable_message());
                        }
                        let request_body_bytes = info.request_body_bytes.clone();
                        let request_handle = info.network_request_handle;
                        let network_record = with_pending_subresource_record_identity(
                            crate::types::SubresourceNetworkRecord::success_with_body(
                                info.frame_id,
                                info.document_url,
                                request_url,
                                request_method,
                                request_headers,
                                request_body,
                                info.resource_type,
                                info.request_cookie_report,
                                Vec::new(),
                                info.url,
                                response_code,
                                response_headers,
                                response_body.into_subresource_response_body(),
                                Vec::new(),
                            ),
                            request_body_bytes,
                            request_handle,
                        );
                        self._context_host
                            .borrow_mut()
                            .record_subresource_network(network_record);
                    }
                    Err(message) => {
                        let failed = self.fail_worker_owned_fetch(
                            target,
                            crate::worker::WorkerPendingFetchContinue {
                                fetch_id: target.fetch_id(),
                                internal_id: 0,
                                network_request_handle: info.network_request_handle,
                                url: request_url.clone(),
                                method: request_method.clone(),
                                body: request_body.clone(),
                                headers: request_headers.clone(),
                                intercept_response: false,
                                handle_auth_requests: false,
                                auth: None,
                            },
                            message.clone(),
                        );
                        if !failed {
                            bail!(target.unavailable_message());
                        }
                        let request_body_bytes = info.request_body_bytes.clone();
                        let request_handle = info.network_request_handle;
                        let network_record = with_pending_subresource_record_identity(
                            crate::types::SubresourceNetworkRecord::failure(
                                info.frame_id,
                                info.document_url,
                                request_url,
                                request_method,
                                request_headers,
                                request_body,
                                info.resource_type,
                                message,
                            ),
                            request_body_bytes,
                            request_handle,
                        );
                        self._context_host
                            .borrow_mut()
                            .record_subresource_network(network_record);
                    }
                }
                return Ok(AsyncSubresourceCommandExecution::without_window_realm(()));
            }
            continuation @ (PendingSubresourceContinuation::WorkerXhr { .. }
            | PendingSubresourceContinuation::SharedWorkerXhr { .. }) => {
                let target = WorkerOwnedXhrTarget::from_continuation(&continuation)
                    .expect("xhr continuation target");
                let request_url = info.url.clone();
                let request_method = info.method.clone();
                let request_headers = info.request_headers.clone();
                let request_body = info.request_body.clone();
                let validation = crate::network_host::validate_cors_response(
                    &info.document_url,
                    &info.url,
                    &response_headers,
                    credentials_mode,
                );
                match validation {
                    Ok(()) => {
                        let fulfilled = self.fulfill_worker_owned_xhr(
                            target,
                            crate::worker::WorkerPendingXhrContinue {
                                xhr_id: target.xhr_id(),
                                internal_id: 0,
                                network_request_handle: info.network_request_handle,
                                url: request_url.clone(),
                                method: request_method.clone(),
                                body: request_body.clone(),
                                headers: request_headers.clone(),
                                intercept_response: false,
                                handle_auth_requests: false,
                                auth: None,
                            },
                            response_code,
                            response_headers.clone(),
                            response_body.clone(),
                        );
                        if !fulfilled {
                            bail!(target.unavailable_message());
                        }
                        let request_body_bytes = info.request_body_bytes.clone();
                        let request_handle = info.network_request_handle;
                        let network_record = with_pending_subresource_record_identity(
                            crate::types::SubresourceNetworkRecord::success_with_body(
                                info.frame_id,
                                info.document_url,
                                request_url,
                                request_method,
                                request_headers,
                                request_body,
                                info.resource_type,
                                info.request_cookie_report,
                                Vec::new(),
                                info.url,
                                response_code,
                                response_headers,
                                response_body.into_subresource_response_body(),
                                Vec::new(),
                            ),
                            request_body_bytes,
                            request_handle,
                        );
                        self._context_host
                            .borrow_mut()
                            .record_subresource_network(network_record);
                    }
                    Err(message) => {
                        let failed = self.fail_worker_owned_xhr(
                            target,
                            crate::worker::WorkerPendingXhrContinue {
                                xhr_id: target.xhr_id(),
                                internal_id: 0,
                                network_request_handle: info.network_request_handle,
                                url: request_url.clone(),
                                method: request_method.clone(),
                                body: request_body.clone(),
                                headers: request_headers.clone(),
                                intercept_response: false,
                                handle_auth_requests: false,
                                auth: None,
                            },
                            message.clone(),
                        );
                        if !failed {
                            bail!(target.unavailable_message());
                        }
                        let request_body_bytes = info.request_body_bytes.clone();
                        let request_handle = info.network_request_handle;
                        let network_record = with_pending_subresource_record_identity(
                            crate::types::SubresourceNetworkRecord::failure(
                                info.frame_id,
                                info.document_url,
                                request_url,
                                request_method,
                                request_headers,
                                request_body,
                                info.resource_type,
                                message,
                            ),
                            request_body_bytes,
                            request_handle,
                        );
                        self._context_host
                            .borrow_mut()
                            .record_subresource_network(network_record);
                    }
                }
                return Ok(AsyncSubresourceCommandExecution::without_window_realm(()));
            }
            continuation @ (PendingSubresourceContinuation::WorkerCspReport { .. }
            | PendingSubresourceContinuation::SharedWorkerCspReport { .. }) => {
                let target = WorkerOwnedCspReportTarget::from_continuation(&continuation)
                    .expect("CSP report continuation target");
                let request_url = info.url.clone();
                let request_method = info.method.clone();
                let request_headers = info.request_headers.clone();
                let request_body = info.request_body.clone();
                let fulfilled = self.fulfill_worker_owned_csp_report(
                    target,
                    crate::worker::WorkerPendingFetchContinue {
                        fetch_id: target.report_id(),
                        internal_id: 0,
                        network_request_handle: info.network_request_handle,
                        url: request_url.clone(),
                        method: request_method.clone(),
                        body: request_body.clone(),
                        headers: request_headers.clone(),
                        intercept_response: false,
                        handle_auth_requests: false,
                        auth: None,
                    },
                    response_code,
                    response_headers.clone(),
                    response_body.clone(),
                );
                if !fulfilled {
                    bail!(target.unavailable_message());
                }
                let request_body_bytes = info.request_body_bytes.clone();
                let request_handle = info.network_request_handle;
                let network_record = with_pending_subresource_record_identity(
                    crate::types::SubresourceNetworkRecord::success_with_body(
                        info.frame_id,
                        info.document_url,
                        request_url,
                        request_method,
                        request_headers,
                        request_body,
                        info.resource_type,
                        info.request_cookie_report,
                        Vec::new(),
                        info.url,
                        response_code,
                        response_headers,
                        response_body.into_subresource_response_body(),
                        Vec::new(),
                    ),
                    request_body_bytes,
                    request_handle,
                );
                self._context_host
                    .borrow_mut()
                    .record_subresource_network(network_record);
                return Ok(AsyncSubresourceCommandExecution::without_window_realm(()));
            }
            continuation => PendingSubresourceFetchState {
                info,
                load,
                execution_context,
                credentials_mode,
                request_mode,
                network_partition_key,
                policy_context,
                continuation,
                deferred_request_started,
            },
        };
        let info = pending.info.clone();
        let activity = self.resolve_pending_subresource_fetch_body(
            pending,
            info.url.clone(),
            info.method,
            info.request_headers,
            info.request_body,
            None,
            false,
            None,
            None,
            Ok(
                response_body.into_navigation_response(moli_fetch::ResponseHead {
                    final_url: info.url,
                    status: response_code,
                    headers: response_headers,
                    request_cookie_report: info.request_cookie_report,
                    cookie_set_reports: Vec::new(),
                    redirected: false,
                    redirect_chain: Vec::new(),
                    from_cache: false,
                    negotiated_http_version: None,
                }),
            ),
        )?;
        Ok(AsyncSubresourceCommandExecution::after_body((), activity))
    }

    pub(super) fn record_subresource_fetch_network_result(
        &mut self,
        pending: &PendingSubresourceFetchState,
        request_url: &Url,
        request_method: &str,
        request_headers: &[(String, String)],
        request_body: &Option<String>,
        result: &std::result::Result<crate::protocol_types::NavigationResponse, String>,
    ) {
        match result {
            Ok(response) => {
                let request_cookie_report = response
                    .request_cookie_report
                    .clone()
                    .or_else(|| pending.info.request_cookie_report.clone());
                let network_record = with_pending_subresource_record_identity(
                    crate::types::SubresourceNetworkRecord::success_with_body(
                        pending.info.frame_id.clone(),
                        pending.info.document_url.clone(),
                        request_url.clone(),
                        request_method.to_owned(),
                        request_headers.to_vec(),
                        request_body.clone(),
                        pending.info.resource_type,
                        request_cookie_report,
                        response.redirect_chain.clone().into_iter().collect(),
                        response.final_url.clone(),
                        response.status,
                        response.headers.clone(),
                        SubresourceResponseBody::from_navigation_response(response),
                        response.cookie_set_reports.clone(),
                    )
                    .with_from_cache(response.from_cache)
                    .with_negotiated_http_version(response.negotiated_http_version)
                    .with_network_request_headers(
                        response
                            .network_request_headers()
                            .map(|headers| headers.to_vec()),
                    ),
                    pending.info.request_body_bytes.clone(),
                    pending.info.network_request_handle,
                );
                self._context_host
                    .borrow_mut()
                    .record_subresource_network(network_record);
            }
            Err(error_text) => {
                let network_error_text =
                    if crate::network_host::is_cors_policy_failure_message(error_text) {
                        crate::network_host::FAILED_ERROR_TEXT.to_owned()
                    } else {
                        error_text.clone()
                    };
                let network_record = with_pending_subresource_record_identity(
                    crate::types::SubresourceNetworkRecord::failure(
                        pending.info.frame_id.clone(),
                        pending.info.document_url.clone(),
                        request_url.clone(),
                        request_method.to_owned(),
                        request_headers.to_vec(),
                        request_body.clone(),
                        pending.info.resource_type,
                        network_error_text,
                    ),
                    pending.info.request_body_bytes.clone(),
                    pending.info.network_request_handle,
                );
                self._context_host
                    .borrow_mut()
                    .record_subresource_network(network_record);
            }
        }
    }

    pub(crate) fn continue_pending_subresource_response_body(
        &mut self,
        internal_id: u64,
        response_code: Option<u16>,
        response_headers: Option<Vec<(String, String)>>,
    ) -> Result<AsyncSubresourceCommandExecution<()>> {
        let pending_websocket_response = {
            self._context_host
                .borrow_mut()
                .take_pending_websocket_response(internal_id)
        };
        if let Some(pending) = pending_websocket_response {
            if response_code.is_some_and(|status| status != 101) {
                self._context_host
                    .borrow_mut()
                    .fail_websocket_handshake_response(
                        pending,
                        format!(
                            "Fetch.continueResponse WebSocket response must use status 101, got {}",
                            response_code.unwrap()
                        ),
                    )
                    .map_err(|error| anyhow!(error))?;
                bail!(
                    "Fetch.continueResponse WebSocket response must use status 101, got {}",
                    response_code.unwrap()
                );
            }
            self._context_host
                .borrow_mut()
                .continue_websocket_handshake_response(pending, response_code, response_headers)
                .map_err(|error| anyhow!(error))?;
            return Ok(AsyncSubresourceCommandExecution::without_window_realm(()));
        }
        let pending = self
            ._context_host
            .borrow_mut()
            .take_pending_subresource_response(internal_id)
            .ok_or_else(|| anyhow!("unknown pending subresource response `{internal_id}`"))?;
        if let Some(target) =
            WorkerOwnedFetchTarget::from_continuation(&pending.pending.continuation)
        {
            let response = crate::protocol_types::NavigationResponse::with_status_headers_from(
                &pending.response,
                response_code.unwrap_or(pending.response.status),
                response_headers
                    .clone()
                    .unwrap_or_else(|| pending.response.headers.clone()),
            );
            let result = if pending.pending.request_mode == moli_fetch::RequestMode::NoCors {
                Ok(response)
            } else {
                crate::network_host::validate_cors_response(
                    &pending.pending.info.document_url,
                    &response.final_url,
                    &response.headers,
                    pending.pending.credentials_mode,
                )
                .map(|()| response)
            };
            self.record_subresource_fetch_network_result(
                &pending.pending,
                &pending.request_url,
                &pending.request_method,
                &pending.request_headers,
                &pending.request_body,
                &result,
            );
            let request = crate::worker::WorkerPendingFetchContinue {
                fetch_id: target.fetch_id(),
                internal_id,
                network_request_handle: pending.pending.info.network_request_handle,
                url: pending.request_url,
                method: pending.request_method,
                body: pending.request_body,
                headers: pending.request_headers,
                intercept_response: false,
                handle_auth_requests: false,
                auth: None,
            };
            match result {
                Ok(_) => {
                    let continued = self.continue_worker_owned_fetch_response(
                        target,
                        request,
                        response_code,
                        response_headers,
                    );
                    if !continued {
                        bail!(target.unavailable_message());
                    }
                }
                Err(message) => {
                    let failed = self.fail_worker_owned_fetch_response(target, request, message);
                    if !failed {
                        bail!(target.unavailable_message());
                    }
                }
            }
            return Ok(AsyncSubresourceCommandExecution::without_window_realm(()));
        }
        if let Some(target) = WorkerOwnedXhrTarget::from_continuation(&pending.pending.continuation)
        {
            let response = crate::protocol_types::NavigationResponse::with_status_headers_from(
                &pending.response,
                response_code.unwrap_or(pending.response.status),
                response_headers
                    .clone()
                    .unwrap_or_else(|| pending.response.headers.clone()),
            );
            let result = if pending.pending.request_mode == moli_fetch::RequestMode::NoCors {
                Ok(response)
            } else {
                crate::network_host::validate_cors_response(
                    &pending.pending.info.document_url,
                    &response.final_url,
                    &response.headers,
                    pending.pending.credentials_mode,
                )
                .map(|()| response)
            };
            self.record_subresource_fetch_network_result(
                &pending.pending,
                &pending.request_url,
                &pending.request_method,
                &pending.request_headers,
                &pending.request_body,
                &result,
            );
            let request = crate::worker::WorkerPendingXhrContinue {
                xhr_id: target.xhr_id(),
                internal_id,
                network_request_handle: pending.pending.info.network_request_handle,
                url: pending.request_url,
                method: pending.request_method,
                body: pending.request_body,
                headers: pending.request_headers,
                intercept_response: false,
                handle_auth_requests: false,
                auth: None,
            };
            match result {
                Ok(_) => {
                    let continued = self.continue_worker_owned_xhr_response(
                        target,
                        request,
                        response_code,
                        response_headers,
                    );
                    if !continued {
                        bail!(target.unavailable_message());
                    }
                }
                Err(message) => {
                    let failed = self.fail_worker_owned_xhr_response(target, request, message);
                    if !failed {
                        bail!(target.unavailable_message());
                    }
                }
            }
            return Ok(AsyncSubresourceCommandExecution::without_window_realm(()));
        }
        let response = crate::protocol_types::NavigationResponse::with_status_headers_from(
            &pending.response,
            response_code.unwrap_or(pending.response.status),
            response_headers.unwrap_or_else(|| pending.response.headers.clone()),
        );
        let activity = self.resolve_pending_subresource_fetch_body(
            pending.pending,
            pending.request_url,
            pending.request_method,
            pending.request_headers,
            pending.request_body,
            None,
            false,
            None,
            None,
            Ok(response),
        )?;
        Ok(AsyncSubresourceCommandExecution::after_body((), activity))
    }

    pub(crate) fn fail_pending_subresource_response_body(
        &mut self,
        internal_id: u64,
        error_text: String,
    ) -> Result<AsyncSubresourceCommandExecution<()>> {
        let pending_websocket_response = {
            self._context_host
                .borrow_mut()
                .take_pending_websocket_response(internal_id)
        };
        if let Some(pending) = pending_websocket_response {
            self._context_host
                .borrow_mut()
                .fail_websocket_handshake_response(pending, error_text)
                .map_err(|error| anyhow!(error))?;
            return Ok(AsyncSubresourceCommandExecution::without_window_realm(()));
        }
        let pending = self
            ._context_host
            .borrow_mut()
            .take_pending_subresource_response(internal_id)
            .ok_or_else(|| anyhow!("unknown pending subresource response `{internal_id}`"))?;
        if let Some(target) =
            WorkerOwnedFetchTarget::from_continuation(&pending.pending.continuation)
        {
            let result = Err(error_text.clone());
            self.record_subresource_fetch_network_result(
                &pending.pending,
                &pending.request_url,
                &pending.request_method,
                &pending.request_headers,
                &pending.request_body,
                &result,
            );
            let failed = self.fail_worker_owned_fetch_response(
                target,
                crate::worker::WorkerPendingFetchContinue {
                    fetch_id: target.fetch_id(),
                    internal_id,
                    network_request_handle: pending.pending.info.network_request_handle,
                    url: pending.request_url,
                    method: pending.request_method,
                    body: pending.request_body,
                    headers: pending.request_headers,
                    intercept_response: false,
                    handle_auth_requests: false,
                    auth: None,
                },
                error_text,
            );
            if !failed {
                bail!(target.unavailable_message());
            }
            return Ok(AsyncSubresourceCommandExecution::without_window_realm(()));
        }
        if let Some(target) = WorkerOwnedXhrTarget::from_continuation(&pending.pending.continuation)
        {
            let result = Err(error_text.clone());
            self.record_subresource_fetch_network_result(
                &pending.pending,
                &pending.request_url,
                &pending.request_method,
                &pending.request_headers,
                &pending.request_body,
                &result,
            );
            let request = crate::worker::WorkerPendingXhrContinue {
                xhr_id: target.xhr_id(),
                internal_id,
                network_request_handle: pending.pending.info.network_request_handle,
                url: pending.request_url,
                method: pending.request_method,
                body: pending.request_body,
                headers: pending.request_headers,
                intercept_response: false,
                handle_auth_requests: false,
                auth: None,
            };
            let failed = self.fail_worker_owned_xhr_response(target, request, error_text);
            if !failed {
                bail!(target.unavailable_message());
            }
            return Ok(AsyncSubresourceCommandExecution::without_window_realm(()));
        }
        let activity = self.resolve_pending_subresource_fetch_body(
            pending.pending,
            pending.request_url,
            pending.request_method,
            pending.request_headers,
            pending.request_body,
            None,
            false,
            None,
            None,
            Err(error_text),
        )?;
        Ok(AsyncSubresourceCommandExecution::after_body((), activity))
    }

    pub(crate) fn fulfill_pending_subresource_response_body(
        &mut self,
        internal_id: u64,
        response_code: u16,
        response_headers: Vec<(String, String)>,
        response_body: RendererSyntheticResponseBody,
    ) -> Result<AsyncSubresourceCommandExecution<()>> {
        let pending_websocket_response = {
            self._context_host
                .borrow_mut()
                .take_pending_websocket_response(internal_id)
        };
        if let Some(pending) = pending_websocket_response {
            if response_code != 101 {
                self._context_host
                    .borrow_mut()
                    .fail_websocket_handshake_response(
                        pending,
                        format!(
                            "Fetch.fulfillRequest WebSocket response must use status 101, got {response_code}"
                        ),
                    )
                    .map_err(|error| anyhow!(error))?;
                bail!(
                    "Fetch.fulfillRequest WebSocket response must use status 101, got {response_code}"
                );
            }
            let _ = response_body;
            self._context_host
                .borrow_mut()
                .continue_websocket_handshake_response(
                    pending,
                    Some(response_code),
                    Some(response_headers),
                )
                .map_err(|error| anyhow!(error))?;
            return Ok(AsyncSubresourceCommandExecution::without_window_realm(()));
        }
        let pending = self
            ._context_host
            .borrow_mut()
            .take_pending_subresource_response(internal_id)
            .ok_or_else(|| anyhow!("unknown pending subresource response `{internal_id}`"))?;
        if let Some(target) =
            WorkerOwnedFetchTarget::from_continuation(&pending.pending.continuation)
        {
            let response = response_body.clone_as_navigation_response(moli_fetch::ResponseHead {
                final_url: pending.response.final_url.clone(),
                status: response_code,
                headers: response_headers.clone(),
                request_cookie_report: pending.response.request_cookie_report.clone(),
                cookie_set_reports: Vec::new(),
                redirected: false,
                redirect_chain: Vec::new(),
                from_cache: pending.response.from_cache,
                negotiated_http_version: pending.response.negotiated_http_version,
            });
            let result = if pending.pending.request_mode == moli_fetch::RequestMode::NoCors {
                Ok(response)
            } else {
                crate::network_host::validate_cors_response(
                    &pending.pending.info.document_url,
                    &response.final_url,
                    &response.headers,
                    pending.pending.credentials_mode,
                )
                .map(|()| response)
            };
            self.record_subresource_fetch_network_result(
                &pending.pending,
                &pending.request_url,
                &pending.request_method,
                &pending.request_headers,
                &pending.request_body,
                &result,
            );
            let request = crate::worker::WorkerPendingFetchContinue {
                fetch_id: target.fetch_id(),
                internal_id,
                network_request_handle: pending.pending.info.network_request_handle,
                url: pending.request_url,
                method: pending.request_method,
                body: pending.request_body,
                headers: pending.request_headers,
                intercept_response: false,
                handle_auth_requests: false,
                auth: None,
            };
            if let Err(message) = result {
                let failed = self.fail_worker_owned_fetch_response(target, request, message);
                if !failed {
                    bail!(target.unavailable_message());
                }
                return Ok(AsyncSubresourceCommandExecution::without_window_realm(()));
            }
            let fulfilled = self.fulfill_worker_owned_fetch_response(
                target,
                request,
                response_code,
                response_headers,
                response_body,
            );
            if !fulfilled {
                bail!(target.unavailable_message());
            }
            return Ok(AsyncSubresourceCommandExecution::without_window_realm(()));
        }
        if let Some(target) = WorkerOwnedXhrTarget::from_continuation(&pending.pending.continuation)
        {
            let response = response_body.clone_as_navigation_response(moli_fetch::ResponseHead {
                final_url: pending.response.final_url.clone(),
                status: response_code,
                headers: response_headers.clone(),
                request_cookie_report: pending.response.request_cookie_report.clone(),
                cookie_set_reports: Vec::new(),
                redirected: false,
                redirect_chain: Vec::new(),
                from_cache: pending.response.from_cache,
                negotiated_http_version: pending.response.negotiated_http_version,
            });
            let result = if pending.pending.request_mode == moli_fetch::RequestMode::NoCors {
                Ok(response)
            } else {
                crate::network_host::validate_cors_response(
                    &pending.pending.info.document_url,
                    &response.final_url,
                    &response.headers,
                    pending.pending.credentials_mode,
                )
                .map(|()| response)
            };
            self.record_subresource_fetch_network_result(
                &pending.pending,
                &pending.request_url,
                &pending.request_method,
                &pending.request_headers,
                &pending.request_body,
                &result,
            );
            let request = crate::worker::WorkerPendingXhrContinue {
                xhr_id: target.xhr_id(),
                internal_id,
                network_request_handle: pending.pending.info.network_request_handle,
                url: pending.request_url,
                method: pending.request_method,
                body: pending.request_body,
                headers: pending.request_headers,
                intercept_response: false,
                handle_auth_requests: false,
                auth: None,
            };
            if let Err(message) = result {
                let failed = self.fail_worker_owned_xhr_response(target, request, message);
                if !failed {
                    bail!(target.unavailable_message());
                }
                return Ok(AsyncSubresourceCommandExecution::without_window_realm(()));
            }
            let fulfilled = self.fulfill_worker_owned_xhr_response(
                target,
                request,
                response_code,
                response_headers,
                response_body,
            );
            if !fulfilled {
                bail!(target.unavailable_message());
            }
            return Ok(AsyncSubresourceCommandExecution::without_window_realm(()));
        }
        let activity = self.resolve_pending_subresource_fetch_body(
            pending.pending,
            pending.request_url,
            pending.request_method,
            pending.request_headers,
            pending.request_body,
            None,
            false,
            None,
            None,
            Ok(
                response_body.into_navigation_response(moli_fetch::ResponseHead {
                    final_url: pending.response.final_url,
                    status: response_code,
                    headers: response_headers,
                    request_cookie_report: pending.response.request_cookie_report,
                    cookie_set_reports: Vec::new(),
                    redirected: false,
                    redirect_chain: Vec::new(),
                    from_cache: pending.response.from_cache,
                    negotiated_http_version: pending.response.negotiated_http_version,
                }),
            ),
        )?;
        Ok(AsyncSubresourceCommandExecution::after_body((), activity))
    }

    /// Standalone ScriptVm compatibility turn for low-level producer tests.
    /// Page behavior must use `PageVm`'s command coordinator instead.
    #[cfg(test)]
    pub(crate) fn continue_pending_subresource_fetch(
        &mut self,
        internal_id: u64,
        url: Option<Url>,
        method: Option<String>,
        body: Option<Option<String>>,
        headers: Option<Vec<(String, String)>>,
        intercept_response: bool,
        handle_auth_requests: bool,
    ) -> Result<PendingSubresourceContinueOutcome> {
        let execution = self.continue_pending_subresource_fetch_body(
            internal_id,
            url,
            method,
            body,
            headers,
            intercept_response,
            handle_auth_requests,
        )?;
        self.finish_async_subresource_command_for_test(execution)
    }

    /// Standalone ScriptVm compatibility turn for low-level producer tests.
    /// Page behavior must use `PageVm`'s command coordinator instead.
    #[cfg(test)]
    pub(crate) fn fulfill_pending_subresource_fetch(
        &mut self,
        internal_id: u64,
        response_code: u16,
        response_headers: Vec<(String, String)>,
        response_body: RendererSyntheticResponseBody,
    ) -> Result<()> {
        let execution = self.fulfill_pending_subresource_fetch_body(
            internal_id,
            response_code,
            response_headers,
            response_body,
        )?;
        self.finish_async_subresource_command_for_test(execution)
    }

    #[cfg(test)]
    pub(super) fn eval_in_isolated_context(
        &mut self,
        execution_context_id: i64,
        source: &str,
    ) -> Result<String> {
        let (context_ptr, sync_child_records): (*const v8::Global<v8::Context>, bool) = {
            let world = self
                .page_isolated_world_contexts
                .context(execution_context_id)
                .ok_or_else(|| {
                    anyhow!("unknown isolated execution context `{execution_context_id}`")
                })?;
            (&world.context as *const _, world.child_handle.is_some())
        };
        self.eval_string_in_context_ptr_runtime_turn(context_ptr, source, sync_child_records)
    }

    pub(super) fn exec_in_isolated_context(
        &mut self,
        execution_context_id: i64,
        source: &str,
    ) -> Result<()> {
        let (context_ptr, sync_child_records): (*const v8::Global<v8::Context>, bool) = {
            let world = self
                .page_isolated_world_contexts
                .context(execution_context_id)
                .ok_or_else(|| {
                    anyhow!("unknown isolated execution context `{execution_context_id}`")
                })?;
            (&world.context as *const _, world.child_handle.is_some())
        };
        self.exec_in_context_ptr_runtime_turn(
            context_ptr,
            source,
            None,
            0,
            true,
            sync_child_records,
        )
    }

    fn resolve_network_only_subresource_fetch(
        &mut self,
        mut pending: PendingSubresourceFetchState,
        request_url: Url,
        request_method: String,
        request_headers: Vec<(String, String)>,
        request_body: Option<String>,
        response_status_text: Option<String>,
        skip_fetch_security_validation: bool,
        network_error_text: Option<String>,
        result: std::result::Result<crate::protocol_types::NavigationResponse, String>,
    ) -> Result<()> {
        let detached_window_fetch = pending.continuation.is_detached_window_fetch();
        debug_assert!(
            detached_window_fetch || pending.execution_context.is_window_network_only(),
            "network-only completion must be an accepted fire-and-forget request or detached Fetch"
        );
        self._context_host
            .borrow_mut()
            .record_deferred_pending_subresource_request_started(&mut pending);

        let result = if detached_window_fetch {
            result.and_then(|response| {
                if !response.redirect_chain.is_empty()
                    && let Some(message) = detached_window_fetch_csp_redirect_failure_message(
                        &self._context_host,
                        &pending,
                        &response.final_url,
                    )
                {
                    return Err(message);
                }
                if !skip_fetch_security_validation {
                    crate::network_host::validate_fetch_response_security_policy_with_body(
                        &pending.info.document_url,
                        &response.final_url,
                        &response.headers,
                        response.body_bytes(),
                        pending.request_mode,
                        pending.credentials_mode,
                        pending.policy_context,
                    )?;
                }
                Ok(response)
            })
        } else {
            result
        };

        match result {
            Ok(response) => {
                let network_request_headers = response
                    .network_request_headers()
                    .map(|headers| headers.to_vec());
                let request_cookie_report = response
                    .request_cookie_report
                    .clone()
                    .or_else(|| pending.info.request_cookie_report.clone());
                let response_body = SubresourceResponseBody::from_navigation_response(&response);
                let mut network_record = crate::types::SubresourceNetworkRecord::success_with_body(
                    pending.info.frame_id.clone(),
                    pending.info.document_url.clone(),
                    request_url,
                    request_method,
                    request_headers,
                    request_body,
                    pending.info.resource_type,
                    request_cookie_report,
                    response.redirect_chain,
                    response.final_url,
                    response.status,
                    response.headers,
                    response_body,
                    response.cookie_set_reports,
                )
                .with_from_cache(response.from_cache)
                .with_negotiated_http_version(response.negotiated_http_version)
                .with_network_request_headers(network_request_headers)
                .with_request_initiator_type(SubresourceRequestInitiatorType::Script)
                .with_request_body_bytes(pending.info.request_body_bytes.clone());
                if let Some(status_text) = response_status_text.as_deref() {
                    network_record = network_record.with_response_status_text(status_text);
                }
                if let Some(handle) = pending.info.network_request_handle {
                    network_record = network_record.with_request_handle(handle);
                }
                self._context_host
                    .borrow_mut()
                    .record_subresource_network(network_record);
            }
            Err(error_text) => {
                let error_text = network_error_text.as_deref().unwrap_or(&error_text);
                let mut network_record = crate::types::SubresourceNetworkRecord::failure(
                    pending.info.frame_id.clone(),
                    pending.info.document_url.clone(),
                    request_url,
                    request_method,
                    request_headers,
                    request_body,
                    pending.info.resource_type,
                    error_text.to_owned(),
                )
                .with_request_initiator_type(SubresourceRequestInitiatorType::Script)
                .with_request_body_bytes(pending.info.request_body_bytes.clone());
                if let Some(handle) = pending.info.network_request_handle {
                    network_record = network_record.with_request_handle(handle);
                }
                self._context_host
                    .borrow_mut()
                    .record_subresource_network(network_record);
            }
        }
        tracing::debug!(
            internal_id = pending.info.internal_id,
            detached_owner = ?pending.execution_context.detached_window_fetch_identity(),
            accepted_context = ?pending.execution_context.window_network_only_identity(),
            accepted_document = ?pending.execution_context.window_document_network_only_identity(),
            "completed network-only subresource without entering V8"
        );
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn resolve_pending_event_source_fetch(
        &mut self,
        mut pending: PendingSubresourceFetchState,
        request_url: Url,
        request_method: String,
        request_headers: Vec<(String, String)>,
        request_body: Option<String>,
        response_status_text: Option<String>,
        skip_fetch_security_validation: bool,
        network_error_text: Option<String>,
        result: std::result::Result<crate::protocol_types::NavigationResponse, String>,
    ) -> Result<AsyncSubresourceFetchBodyActivity> {
        self._context_host
            .borrow_mut()
            .record_deferred_pending_subresource_request_started(&mut pending);
        let context_host = self._context_host.clone();
        self.renderer_document_isolate
            .with_entered_renderer_document_isolate(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = v8::Local::new(
                    scope,
                    pending
                        .execution_context
                        .context_global()
                        .expect("EventSource completion must retain its V8 context"),
                );
                let scope = &mut v8::ContextScope::new(scope, context);
                if !window_subresource_realm_is_current(&context_host, scope, &pending) {
                    return Ok(AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered);
                }
                let owner_async_scope = enter_subresource_owner_async_scope(
                    &context_host,
                    scope,
                    pending.execution_context.dispatch_scope(),
                );
                let event_source = match &pending.continuation {
                    PendingSubresourceContinuation::EventSource(event_source) => {
                        v8::Local::new(scope, event_source)
                    }
                    _ => unreachable!("EventSource completion requires EventSource continuation"),
                };
                let request_handle = pending.info.network_request_handle;

                match result {
                    Err(error_text) => {
                        let error_text = network_error_text.unwrap_or(error_text);
                        let mut record = crate::types::SubresourceNetworkRecord::failure(
                            pending.info.frame_id.clone(),
                            pending.info.document_url.clone(),
                            request_url,
                            request_method,
                            request_headers,
                            request_body,
                            SubresourceResourceType::EventSource,
                            error_text,
                        );
                        if let Some(handle) = request_handle {
                            record = record.with_request_handle(handle);
                        }
                        context_host
                            .borrow_mut()
                            .record_subresource_network(record);
                        crate::network_host::fail_event_source_connection(
                            scope,
                            event_source,
                            crate::network_host::EventSourceTerminalMode::Reconnect,
                        );
                    }
                    Ok(response) => {
                        let security_error = if !response.redirect_chain.is_empty() {
                            document_connect_csp_redirect_failure_message(
                                scope,
                                &context_host,
                                &pending,
                                &response.final_url,
                            )
                        } else {
                            None
                        }
                        .or_else(|| {
                            (!skip_fetch_security_validation)
                                .then(|| {
                                    crate::network_host::validate_fetch_response_security_policy_with_body(
                                        &pending.info.document_url,
                                        &response.final_url,
                                        &response.headers,
                                        response.body_bytes(),
                                        pending.request_mode,
                                        pending.credentials_mode,
                                        pending.policy_context,
                                    )
                                    .err()
                                })
                                .flatten()
                        });
                        if let Some(error_text) = security_error {
                            let mut record = crate::types::SubresourceNetworkRecord::failure(
                                pending.info.frame_id.clone(),
                                pending.info.document_url.clone(),
                                request_url,
                                request_method,
                                request_headers,
                                request_body,
                                SubresourceResourceType::EventSource,
                                error_text,
                            );
                            if let Some(handle) = request_handle {
                                record = record.with_request_handle(handle);
                            }
                            context_host
                                .borrow_mut()
                                .record_subresource_network(record);
                            crate::network_host::fail_event_source_connection(
                                scope,
                                event_source,
                                crate::network_host::EventSourceTerminalMode::Close,
                            );
                        } else {
                            let head = response.head();
                            if let Some(handle) = request_handle {
                                context_host
                                    .borrow_mut()
                                    .record_subresource_response_started(
                                        crate::types::SubresourceResponseStarted::new(
                                            handle,
                                            response.redirect_chain.clone(),
                                            response.final_url.clone(),
                                            response.status,
                                            response.headers.clone(),
                                            response.cookie_set_reports.clone(),
                                        )
                                        .with_status_text(response_status_text)
                                        .with_from_cache(response.from_cache)
                                        .with_negotiated_http_version(
                                            response.negotiated_http_version,
                                        )
                                        .with_network_request_headers(
                                            response
                                                .network_request_headers()
                                                .map(|headers| headers.to_vec()),
                                        ),
                                    );
                            }
                            if let Some(error_text) =
                                crate::network_host::event_source_response_error(&head)
                            {
                                if let Some(handle) = request_handle {
                                    context_host
                                        .borrow_mut()
                                        .record_subresource_body_finished(
                                            crate::types::SubresourceBodyFinished::failed(
                                                handle, error_text,
                                            ),
                                        );
                                }
                                crate::network_host::fail_event_source_connection(
                                    scope,
                                    event_source,
                                    crate::network_host::EventSourceTerminalMode::Close,
                                );
                            } else {
                                crate::network_host::open_event_source_connection(
                                    scope,
                                    event_source,
                                    &response.final_url,
                                );
                                let bytes = response.body_bytes();
                                if let Some(handle) = request_handle
                                    && !bytes.is_empty()
                                {
                                    context_host.borrow_mut().record_subresource_data_received(
                                        crate::types::SubresourceDataReceived::new(
                                            handle,
                                            bytes.len(),
                                            bytes.len(),
                                        ),
                                    );
                                }
                                let mut parser = crate::network_host::EventSourceParser::new(
                                    crate::network_host::event_source_last_event_id(
                                        scope,
                                        event_source,
                                    ),
                                    crate::network_host::event_source_reconnect_delay_ms(
                                        scope,
                                        event_source,
                                    ),
                                );
                                if crate::network_host::event_source_ready_state(scope, event_source)
                                    != crate::network_host::EVENT_SOURCE_CLOSED
                                {
                                    let messages = parser.push(bytes);
                                    dispatch_streaming_event_source_messages(
                                        &context_host,
                                        scope,
                                        event_source,
                                        request_handle,
                                        &messages,
                                    );
                                }
                                if crate::network_host::event_source_ready_state(scope, event_source)
                                    != crate::network_host::EVENT_SOURCE_CLOSED
                                {
                                    crate::network_host::update_event_source_stream_state(
                                        scope,
                                        event_source,
                                        parser.last_event_id(),
                                        parser.reconnect_delay_ms(),
                                    );
                                }
                                if let Some(handle) = request_handle {
                                    context_host.borrow_mut().record_subresource_body_finished(
                                        crate::types::SubresourceBodyFinished::ready_after_streaming(
                                            handle,
                                            SubresourceResponseBody::from_navigation_response(
                                                &response,
                                            ),
                                        ),
                                    );
                                }
                                if crate::network_host::event_source_ready_state(scope, event_source)
                                    != crate::network_host::EVENT_SOURCE_CLOSED
                                {
                                    crate::network_host::fail_event_source_connection(
                                        scope,
                                        event_source,
                                        crate::network_host::EventSourceTerminalMode::Reconnect,
                                    );
                                }
                            }
                        }
                    }
                }
                defer_subresource_owner_async_scope(
                    &context_host,
                    scope,
                    pending.execution_context.dispatch_scope(),
                    owner_async_scope,
                );
                Ok(AsyncSubresourceFetchBodyActivity::WindowRealmEntered)
            })
    }

    fn resolve_pending_subresource_fetch_body(
        &mut self,
        mut pending: PendingSubresourceFetchState,
        request_url: Url,
        request_method: String,
        request_headers: Vec<(String, String)>,
        request_body: Option<String>,
        response_status_text: Option<String>,
        skip_fetch_security_validation: bool,
        response_filter: Option<AsyncSubresourceFetchResponseFilter>,
        network_error_text: Option<String>,
        result: std::result::Result<crate::protocol_types::NavigationResponse, String>,
    ) -> Result<AsyncSubresourceFetchBodyActivity> {
        let trace_started = moli_trace::cdp_runtime_trace_enabled().then(Instant::now);
        let trace_fields = async_subresource_trace_fields_for_pending(
            "completion",
            pending.info.internal_id,
            &pending,
        );
        trace_async_subresource_stage(
            "async_subresource_resolve_pending_start",
            trace_fields,
            trace_started,
        );
        if pending.continuation.is_detached_window_fetch()
            || pending.execution_context.is_window_network_only()
        {
            return self
                .resolve_network_only_subresource_fetch(
                    pending,
                    request_url,
                    request_method,
                    request_headers,
                    request_body,
                    response_status_text,
                    skip_fetch_security_validation,
                    network_error_text,
                    result,
                )
                .map(|()| AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered);
        }
        if !window_subresource_owner_is_current(&self._context_host, &pending) {
            tracing::debug!(
                internal_id = pending.info.internal_id,
                owner = ?pending.execution_context.window_request_target().map(crate::native_bridge::WindowTaskTarget::owner),
                "discarded subresource completion for retired Window execution context"
            );
            return Ok(AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered);
        }
        if pending.continuation.is_window_event_source() {
            return self.resolve_pending_event_source_fetch(
                pending,
                request_url,
                request_method,
                request_headers,
                request_body,
                response_status_text,
                skip_fetch_security_validation,
                network_error_text,
                result,
            );
        }
        self._context_host
            .borrow_mut()
            .record_deferred_pending_subresource_request_started(&mut pending);
        let context_host = self._context_host.clone();
        let mut completed_web_font = None;
        let result = self.renderer_document_isolate.with_entered_renderer_document_isolate(|isolate| {
            let scope = pin!(v8::HandleScope::new(isolate));
            let scope = &mut scope.init();
            let context = v8::Local::new(
                scope,
                pending
                    .execution_context
                    .context_global()
                    .expect("active subresource completion must retain its V8 context"),
            );
            let scope = &mut v8::ContextScope::new(scope, context);
            if !window_subresource_realm_is_current(&context_host, scope, &pending) {
                tracing::debug!(
                    internal_id = pending.info.internal_id,
                    expected_realm = ?pending.execution_context.realm_token(),
                    "discarded subresource completion for retired V8 realm"
                );
                return Ok(AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered);
            }
            trace_async_subresource_stage(
                "async_subresource_context_entered",
                trace_fields,
                trace_started,
            );
            let owner_async_scope =
                enter_subresource_owner_async_scope(
                    &context_host,
                    scope,
                    pending.execution_context.dispatch_scope(),
                );

            let security_started = moli_trace::cdp_runtime_trace_enabled().then(Instant::now);
            let mut opaque_response_blocked = false;
            let result = result.and_then(|response| {
                if !response.redirect_chain.is_empty()
                    && let Some(message) = document_connect_csp_redirect_failure_message(
                        scope,
                        &context_host,
                        &pending,
                        &response.final_url,
                    )
                {
                    return Err(message);
                }
                if !skip_fetch_security_validation
                    && matches!(
                    pending.info.resource_type,
                    SubresourceResourceType::Audio
                        | SubresourceResourceType::Fetch
                        | SubresourceResourceType::Font
                        | SubresourceResourceType::Image
                        | SubresourceResourceType::Media
                        | SubresourceResourceType::TextTrack
                        | SubresourceResourceType::Video
                        | SubresourceResourceType::Xhr
                ) {
                    let validation = crate::network_host::validate_fetch_response_security_policy_with_body_classified(
                        &pending.info.document_url,
                        &response.final_url,
                        &response.headers,
                        response.body_bytes(),
                        pending.request_mode,
                        pending.credentials_mode,
                        pending.policy_context,
                    );
                    match validation {
                        Ok(()) => {}
                        Err(crate::network_host::FetchResponseSecurityViolation::OpaqueResponseBlocked(_))
                            if pending.continuation.is_window_fetch() =>
                        {
                            opaque_response_blocked = true;
                        }
                        Err(violation) => return Err(violation.into_message()),
                    }
                }
                Ok(response)
            });
            trace_async_subresource_stage(
                "async_subresource_security_checked",
                trace_fields,
                security_started,
            );

            let request_initiator_type = pending.continuation.request_initiator_type();
            match result {
                Ok(response) => {
                    let response_status = response.status;
                    let request_cookie_report = response
                        .request_cookie_report
                        .clone()
                        .or_else(|| pending.info.request_cookie_report.clone());
                    let record_started =
                        moli_trace::cdp_runtime_trace_enabled().then(Instant::now);
                    let mut network_record = if opaque_response_blocked {
                        crate::types::SubresourceNetworkRecord::failure(
                            pending.info.frame_id.clone(),
                            pending.info.document_url.clone(),
                            request_url.clone(),
                            request_method,
                            request_headers,
                            request_body,
                            pending.info.resource_type,
                            crate::network_host::ABORTED_ERROR_TEXT.to_owned(),
                        )
                        .with_request_initiator_type(request_initiator_type)
                        .with_request_body_bytes(pending.info.request_body_bytes.clone())
                    } else {
                        crate::types::SubresourceNetworkRecord::success_with_body(
                            pending.info.frame_id.clone(),
                            pending.info.document_url.clone(),
                            request_url.clone(),
                            request_method,
                            request_headers,
                            request_body,
                            pending.info.resource_type,
                            request_cookie_report,
                            response.redirect_chain.clone().into_iter().collect(),
                            response.final_url.clone(),
                            response.status,
                            response.headers.clone(),
                            SubresourceResponseBody::from_navigation_response(&response),
                            response.cookie_set_reports.clone(),
                        )
                        .with_from_cache(response.from_cache)
                        .with_negotiated_http_version(response.negotiated_http_version)
                        .with_network_request_headers(
                            response.network_request_headers().map(|headers| headers.to_vec()),
                        )
                        .with_request_initiator_type(request_initiator_type)
                        .with_request_body_bytes(pending.info.request_body_bytes.clone())
                    };
                    if !opaque_response_blocked
                        && let Some(status_text) = response_status_text.as_deref()
                    {
                        network_record =
                            network_record.with_response_status_text(status_text);
                    }
                    if let Some(handle) = pending.info.network_request_handle {
                        network_record = network_record.with_request_handle(handle);
                    }
                    context_host
                        .borrow_mut()
                        .record_subresource_network(network_record);
                    trace_async_subresource_stage(
                        "async_subresource_network_recorded",
                        trace_fields,
                        record_started,
                    );
                    let mut observable_response = response;
                    if matches!(
                        pending.info.resource_type,
                        SubresourceResourceType::Fetch | SubresourceResourceType::Xhr
                    ) {
                        observable_response.headers =
                            crate::network_host::filter_cors_exposed_response_headers(
                                &pending.info.document_url,
                                &observable_response.final_url,
                                &observable_response.headers,
                                pending.credentials_mode,
                            );
                    }
                    let continuation_started =
                        moli_trace::cdp_runtime_trace_enabled().then(Instant::now);
                    match pending.continuation {
                        PendingSubresourceContinuation::Fetch(fetch) => {
                            let resolver = fetch
                                .into_resolver()
                                .expect("detached keepalive completion is handled before V8 entry");
                            let resolver = v8::Local::new(scope, &resolver);
                            let (head, body) = observable_response.into_body();
                            let body = if opaque_response_blocked {
                                moli_fetch::ResponseBody::materialized_bytes(Vec::new())
                            } else {
                                body
                            };
                            let response_filter = opaque_response_blocked
                                .then_some(AsyncSubresourceFetchResponseFilter::Opaque)
                                .or(response_filter);
                            let response_obj =
                                crate::network_host::build_fetch_response_object_from_body_source_for_request_mode_with_filter(
                                    scope,
                                    &pending.info.document_url,
                                    pending.request_mode,
                                    head,
                                    body,
                                    response_filter,
                                );
                            if let Some(status_text) = response_status_text.as_deref() {
                                crate::network_host::set_response_slot_string(
                                    scope,
                                    response_obj,
                                    crate::network_host::RESPONSE_STATUS_TEXT_SLOT,
                                    status_text,
                                );
                            }
                            resolver.resolve(scope, response_obj.into());
                        }
                        PendingSubresourceContinuation::Xhr(xhr) => {
                            let xhr = v8::Local::new(scope, &xhr);
                            let (head, body) = observable_response.into_body();
                            crate::network_host::apply_xhr_response_body_source_with_status_text(
                                scope,
                                xhr,
                                head,
                                body,
                                response_status_text.as_deref(),
                            );
                        }
                        PendingSubresourceContinuation::Image {
                            image_handle,
                            sequence,
                            ..
                        } => apply_image_subresource_terminal(
                            scope,
                            &context_host,
                            image_handle,
                            sequence,
                            pending.info.internal_id,
                            &pending.info.url,
                            ImageSubresourceTerminal::Response(&observable_response),
                        ),
                        PendingSubresourceContinuation::Media {
                            media_handle,
                            sequence,
                        } => apply_media_subresource_terminal(
                            scope,
                            &context_host,
                            media_handle,
                            sequence,
                            pending.info.internal_id,
                            crate::network_host::media_response_status_is_successful(
                                response_status,
                            ),
                        ),
                        PendingSubresourceContinuation::TextTrack {
                            track_handle,
                            sequence,
                        } => apply_text_track_subresource_terminal(
                            scope,
                            &context_host,
                            track_handle,
                            sequence,
                            pending.info.internal_id,
                            crate::network_host::text_track_response_result(
                                response_status,
                                observable_response.body_text(),
                            ),
                        ),
                        PendingSubresourceContinuation::StylesheetSubresource {
                            binding,
                            web_font,
                            css_image,
                        } => {
                            if let Some(identity) = css_image.as_ref() {
                                let descriptor =
                                    crate::network_host::image_response_descriptor(
                                        &observable_response,
                                    );
                                let _ = context_host
                                    .borrow_mut()
                                    .complete_stylesheet_css_image_response(
                                        identity,
                                        descriptor,
                                        observable_response.body_bytes(),
                                    );
                            }
                            if binding.child_handle().is_none() {
                                completed_web_font = web_font.map(|font| {
                                    if (200..=299).contains(&response_status)
                                        && !opaque_response_blocked
                                    {
                                        crate::css_resource_urls::CompletedStylesheetWebFont::response(
                                            font,
                                            observable_response.clone_body_bytes(),
                                        )
                                    } else {
                                        crate::css_resource_urls::CompletedStylesheetWebFont::failure(
                                            font,
                                        )
                                    }
                                });
                            }
                            apply_stylesheet_subresource_terminal(&context_host, binding);
                        }
                        PendingSubresourceContinuation::Beacon
                        | PendingSubresourceContinuation::CspReport { .. }
                        | PendingSubresourceContinuation::EventSource(_)
                        | PendingSubresourceContinuation::WebSocket(_)
                        | PendingSubresourceContinuation::WorkerFetch { .. }
                        | PendingSubresourceContinuation::WorkerXhr { .. }
                        | PendingSubresourceContinuation::WorkerCspReport { .. }
                        | PendingSubresourceContinuation::SharedWorkerFetch { .. }
                        | PendingSubresourceContinuation::SharedWorkerXhr { .. }
                        | PendingSubresourceContinuation::SharedWorkerCspReport { .. } => {}
                    }
                    trace_async_subresource_stage(
                        "async_subresource_continuation_delivered",
                        trace_fields,
                        continuation_started,
                    );
                }
                Err(error_text) => {
                    let network_error_text = network_error_text
                        .as_deref()
                        .or_else(|| {
                            crate::network_host::is_cors_policy_failure_message(&error_text)
                                .then_some(crate::network_host::FAILED_ERROR_TEXT)
                        })
                        .unwrap_or(&error_text);
                    let record_started =
                        moli_trace::cdp_runtime_trace_enabled().then(Instant::now);
                    let mut network_record = crate::types::SubresourceNetworkRecord::failure(
                        pending.info.frame_id.clone(),
                        pending.info.document_url.clone(),
                        request_url,
                        request_method,
                        request_headers,
                        request_body,
                        pending.info.resource_type,
                        network_error_text.to_owned(),
                    )
                    .with_request_initiator_type(request_initiator_type);
                    if let Some(handle) = pending.info.network_request_handle {
                        network_record = network_record.with_request_handle(handle);
                    }
                    context_host
                        .borrow_mut()
                        .record_subresource_network(network_record);
                    trace_async_subresource_stage(
                        "async_subresource_network_recorded",
                        trace_fields,
                        record_started,
                    );
                    let continuation_started =
                        moli_trace::cdp_runtime_trace_enabled().then(Instant::now);
                    match pending.continuation {
                        PendingSubresourceContinuation::Fetch(fetch) => {
                            let resolver = fetch
                                .into_resolver()
                                .expect("detached keepalive failure is handled before V8 entry");
                            let resolver = v8::Local::new(scope, &resolver);
                            let exception = v8_string(scope, &error_text)
                                .map(|message| v8::Exception::type_error(scope, message))
                                .unwrap_or_else(|| v8::undefined(scope).into());
                            resolver.reject(scope, exception);
                        }
                        PendingSubresourceContinuation::Xhr(xhr) => {
                            let xhr = v8::Local::new(scope, &xhr);
                            crate::network_host::apply_xhr_failure(scope, xhr);
                        }
                        PendingSubresourceContinuation::Image {
                            image_handle,
                            sequence,
                            ..
                        } => apply_image_subresource_terminal(
                            scope,
                            &context_host,
                            image_handle,
                            sequence,
                            pending.info.internal_id,
                            &pending.info.url,
                            ImageSubresourceTerminal::Failure,
                        ),
                        PendingSubresourceContinuation::Media {
                            media_handle,
                            sequence,
                        } => apply_media_subresource_terminal(
                            scope,
                            &context_host,
                            media_handle,
                            sequence,
                            pending.info.internal_id,
                            false,
                        ),
                        PendingSubresourceContinuation::TextTrack {
                            track_handle,
                            sequence,
                        } => apply_text_track_subresource_terminal(
                            scope,
                            &context_host,
                            track_handle,
                            sequence,
                            pending.info.internal_id,
                            Err(error_text.clone()),
                        ),
                        PendingSubresourceContinuation::StylesheetSubresource {
                            binding,
                            web_font,
                            css_image,
                        } => {
                            if let Some(identity) = css_image.as_ref() {
                                let _ = context_host
                                    .borrow_mut()
                                    .fail_stylesheet_css_image(identity);
                            }
                            if binding.child_handle().is_none() {
                                completed_web_font = web_font.map(
                                    crate::css_resource_urls::CompletedStylesheetWebFont::failure,
                                );
                            }
                            apply_stylesheet_subresource_terminal(&context_host, binding);
                        }
                        PendingSubresourceContinuation::Beacon
                        | PendingSubresourceContinuation::CspReport { .. }
                        | PendingSubresourceContinuation::EventSource(_)
                        | PendingSubresourceContinuation::WebSocket(_)
                        | PendingSubresourceContinuation::WorkerFetch { .. }
                        | PendingSubresourceContinuation::WorkerXhr { .. }
                        | PendingSubresourceContinuation::WorkerCspReport { .. }
                        | PendingSubresourceContinuation::SharedWorkerFetch { .. }
                        | PendingSubresourceContinuation::SharedWorkerXhr { .. }
                        | PendingSubresourceContinuation::SharedWorkerCspReport { .. } => {}
                    }
                    trace_async_subresource_stage(
                        "async_subresource_continuation_delivered",
                        trace_fields,
                        continuation_started,
                    );
                }
            }

            defer_subresource_owner_async_scope(
                &context_host,
                scope,
                pending.execution_context.dispatch_scope(),
                owner_async_scope,
            );
            Ok(AsyncSubresourceFetchBodyActivity::WindowRealmEntered)
        });
        if let Some(web_font) = completed_web_font {
            self.complete_document_web_font(web_font);
        }
        trace_async_subresource_stage(
            "async_subresource_resolve_pending_done",
            trace_fields,
            trace_started,
        );
        result
    }

    pub(super) fn spawn_running_subresource_fetch(
        &mut self,
        request_client: ResourceRequestClient,
        request: moli_fetch::Request,
        state: RunningSubresourceFetchState,
        cancel_handle: Option<moli_fetch::FetchCancelHandle>,
    ) {
        let task_runner = state.pending.load.task_runner();
        let internal_id = state.pending.info.internal_id;
        let request_url = state.request_url.clone();
        let request_method = state.request_method.clone();
        let request_headers = state.request_headers.clone();
        let request_body = state.request_body.clone();
        let completion_tx = self._context_host.borrow().resource_completion_sender();
        {
            let mut host = self._context_host.borrow_mut();
            host.begin_active_subresource_request();
            host.record_running_subresource_fetch(state);
        }
        task_runner.spawn(async move {
            let result =
                crate::network_host::fetch_browser_subresource_with_preflight_and_network_metadata(
                    request_client,
                    request,
                    cancel_handle,
                )
                .await
                .map(|observed| {
                    let (response, request_observation) = observed.into_parts();
                    crate::protocol_types::NavigationResponse::from(response)
                        .with_network_request_headers(
                            request_observation.map(|observation| observation.into_headers()),
                        )
                });
            let _ = completion_tx.send_async_subresource(AsyncSubresourceFetchCompletion {
                internal_id,
                request_url,
                request_method,
                request_headers,
                request_body,
                response_status_text: None,
                skip_fetch_security_validation: false,
                response_filter: None,
                network_error_text: None,
                result,
            });
        });
    }

    fn complete_running_subresource_fetch_body(
        &mut self,
        running: RunningSubresourceFetchState,
        response_status_text: Option<String>,
        skip_fetch_security_validation: bool,
        response_filter: Option<AsyncSubresourceFetchResponseFilter>,
        network_error_text: Option<String>,
        result: std::result::Result<crate::protocol_types::NavigationResponse, String>,
    ) -> Result<AsyncSubresourceFetchBodyActivity> {
        let RunningSubresourceFetchState {
            pending,
            request_url,
            request_method,
            request_headers,
            request_body,
            intercept_response,
            handle_auth_requests,
            initial_auth_network_request_headers,
        } = running;
        let internal_id = pending.info.internal_id;
        let resource_type = pending.info.resource_type;
        let trace_started = moli_trace::cdp_runtime_trace_enabled().then(Instant::now);
        let trace_fields =
            async_subresource_trace_fields_for_pending("completion", internal_id, &pending);
        trace_async_subresource_stage(
            "async_subresource_complete_running_start",
            trace_fields,
            trace_started,
        );

        let activity = match result {
            Ok(response) => {
                let response = if let Some(headers) = initial_auth_network_request_headers.clone() {
                    response.with_network_request_headers(Some(headers))
                } else {
                    response
                };
                if handle_auth_requests
                    && matches!(response.status, 401 | 407)
                    && let Some(challenge) =
                        crate::network_host::extract_subresource_auth_challenge(&response.headers)
                {
                    let info = PendingSubresourceAuthInfo {
                        internal_id,
                        url: request_url.clone(),
                        method: request_method.clone(),
                        request_headers: request_headers.clone(),
                        request_body: request_body.clone(),
                        resource_type,
                        request_cookie_report: response.request_cookie_report.clone(),
                        network_request_headers: response
                            .network_request_headers()
                            .map(|headers| headers.to_vec()),
                        challenge,
                        intercept_response,
                        response_final_url: response.final_url.clone(),
                        response_status: response.status,
                        response_headers: response.headers.clone(),
                        response_body: SubresourceResponseBody::from_navigation_response(&response),
                        response_from_cache: response.from_cache,
                    };
                    trace_async_subresource_stage(
                        "async_subresource_complete_running_auth_required",
                        trace_fields,
                        trace_started,
                    );
                    self._context_host
                        .borrow_mut()
                        .record_pending_subresource_auth(PendingSubresourceAuthState {
                            pending,
                            request_url,
                            request_method,
                            request_headers,
                            request_body,
                            intercept_response,
                            initial_network_request_headers: initial_auth_network_request_headers
                                .or_else(|| {
                                    response
                                        .network_request_headers()
                                        .map(|headers| headers.to_vec())
                                }),
                            response,
                        });
                    self._context_host
                        .borrow_mut()
                        .record_pending_subresource_continue_event(
                            PendingSubresourceContinueEvent::AuthRequired(info),
                        );
                    return Ok(AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered);
                }

                if intercept_response {
                    trace_async_subresource_stage(
                        "async_subresource_complete_running_response_paused",
                        trace_fields,
                        trace_started,
                    );
                    let info = PendingSubresourceResponseInfo {
                        internal_id,
                        url: request_url.clone(),
                        final_url: response.final_url.clone(),
                        method: request_method.clone(),
                        request_headers: request_headers.clone(),
                        request_body: request_body.clone(),
                        resource_type,
                        request_cookie_report: response.request_cookie_report.clone(),
                        network_request_headers: response
                            .network_request_headers()
                            .map(|headers| headers.to_vec()),
                        response_status: response.status,
                        response_headers: response.headers.clone(),
                        response_body: SubresourceResponseBody::from_navigation_response(&response),
                        from_cache: response.from_cache,
                    };
                    self._context_host
                        .borrow_mut()
                        .record_pending_subresource_response(PendingSubresourceResponseState {
                            pending,
                            request_url,
                            request_method,
                            request_headers,
                            request_body,
                            response,
                        });
                    self._context_host
                        .borrow_mut()
                        .record_pending_subresource_continue_event(
                            PendingSubresourceContinueEvent::ResponsePaused(info),
                        );
                    return Ok(AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered);
                }

                let activity = self.resolve_pending_subresource_fetch_body(
                    pending,
                    request_url,
                    request_method,
                    request_headers,
                    request_body,
                    response_status_text.clone(),
                    skip_fetch_security_validation,
                    response_filter,
                    network_error_text.clone(),
                    Ok(response),
                );
                trace_async_subresource_stage(
                    "async_subresource_complete_running_done",
                    trace_fields,
                    trace_started,
                );
                activity?
            }
            Err(error) => {
                let activity = self.resolve_pending_subresource_fetch_body(
                    pending,
                    request_url,
                    request_method,
                    request_headers,
                    request_body,
                    response_status_text,
                    skip_fetch_security_validation,
                    response_filter,
                    network_error_text,
                    Err(error),
                );
                trace_async_subresource_stage(
                    "async_subresource_complete_running_done",
                    trace_fields,
                    trace_started,
                );
                activity?
            }
        };
        self._context_host
            .borrow_mut()
            .record_pending_subresource_continue_event(
                PendingSubresourceContinueEvent::Completed { internal_id },
            );
        Ok(activity)
    }

    /// Standalone ScriptVm test turn: apply one terminal and immediately submit
    /// the checkpoint that production owns in the selected Networking task.
    #[cfg(test)]
    pub(crate) fn complete_async_subresource_fetch(
        &mut self,
        completion: AsyncSubresourceFetchCompletion,
    ) -> Result<()> {
        let activity = self.complete_async_subresource_fetch_body(completion)?;
        self.finish_async_subresource_body_checkpoint_for_test(activity)
    }

    fn complete_async_subresource_fetch_body(
        &mut self,
        completion: AsyncSubresourceFetchCompletion,
    ) -> Result<AsyncSubresourceFetchBodyActivity> {
        let trace_started = moli_trace::cdp_runtime_trace_enabled().then(Instant::now);
        let internal_id = completion.internal_id;
        trace_async_subresource_stage(
            "async_subresource_complete_start",
            AsyncSubresourceTraceFields {
                event_kind: Some("completion"),
                internal_id: Some(internal_id),
                ..AsyncSubresourceTraceFields::default()
            },
            trace_started,
        );
        let running = {
            self._context_host
                .borrow_mut()
                .take_running_subresource_fetch(completion.internal_id)
        };
        if let Some(running) = running {
            let trace_fields = async_subresource_trace_fields_for_pending(
                "completion",
                internal_id,
                &running.pending,
            );
            trace_async_subresource_stage(
                "async_subresource_complete_running",
                trace_fields,
                trace_started,
            );
            self._context_host
                .borrow_mut()
                .finish_active_subresource_request();
            let result = self.complete_running_subresource_fetch_body(
                running,
                completion.response_status_text,
                completion.skip_fetch_security_validation,
                completion.response_filter,
                completion.network_error_text,
                completion.result,
            );
            trace_async_subresource_stage(
                "async_subresource_complete_done",
                trace_fields,
                trace_started,
            );
            return result;
        }
        let Some(pending) = self
            ._context_host
            .borrow_mut()
            .take_pending_subresource_fetch(completion.internal_id)
        else {
            trace_async_subresource_stage(
                "async_subresource_complete_missing",
                AsyncSubresourceTraceFields {
                    event_kind: Some("completion"),
                    internal_id: Some(internal_id),
                    ..AsyncSubresourceTraceFields::default()
                },
                trace_started,
            );
            return Ok(AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered);
        };
        let trace_fields =
            async_subresource_trace_fields_for_pending("completion", internal_id, &pending);
        trace_async_subresource_stage(
            "async_subresource_complete_pending",
            trace_fields,
            trace_started,
        );
        let activity = self.resolve_pending_subresource_fetch_body(
            pending,
            completion.request_url,
            completion.request_method,
            completion.request_headers,
            completion.request_body,
            completion.response_status_text,
            completion.skip_fetch_security_validation,
            completion.response_filter,
            completion.network_error_text,
            completion.result,
        )?;
        trace_async_subresource_stage(
            "async_subresource_complete_done",
            trace_fields,
            trace_started,
        );
        Ok(activity)
    }

    pub(crate) fn complete_async_subresource_fetch_event_body(
        &mut self,
        event: AsyncSubresourceFetchEvent,
    ) -> Result<AsyncSubresourceFetchBodyActivity> {
        let trace_started = moli_trace::cdp_runtime_trace_enabled().then(Instant::now);
        let trace_fields = async_subresource_trace_fields_for_event(&event);
        trace_async_subresource_stage("async_subresource_event_start", trace_fields, trace_started);
        let result = match event {
            AsyncSubresourceFetchEvent::Completion(completion) => {
                self.complete_async_subresource_fetch_body(*completion)
            }
            AsyncSubresourceFetchEvent::ObservedNetworkRecord(record) => {
                self._context_host
                    .borrow_mut()
                    .record_subresource_network(*record);
                Ok(AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered)
            }
            AsyncSubresourceFetchEvent::StreamingStarted(started) => {
                self.start_streaming_async_subresource_fetch_body(*started)
            }
            AsyncSubresourceFetchEvent::StreamingChunk(chunk) => Ok(self
                .append_streaming_async_subresource_fetch_chunk_body(
                    chunk.body_source_id,
                    chunk.bytes,
                )),
            AsyncSubresourceFetchEvent::StreamingFinished(finished) => self
                .finish_streaming_async_subresource_fetch_body(
                    finished.internal_id,
                    finished.body_source_id,
                    finished.result,
                ),
        };
        trace_async_subresource_stage("async_subresource_event_done", trace_fields, trace_started);
        result
    }

    pub(crate) fn async_subresource_fetch_event_target_is_current(
        &self,
        target: crate::types::AsyncSubresourceFetchEventTarget,
    ) -> bool {
        self._context_host
            .borrow()
            .async_subresource_fetch_event_target_is_current(target)
    }

    fn start_network_only_subresource_stream(
        &mut self,
        mut pending: PendingSubresourceFetchState,
        started: crate::types::AsyncSubresourceStreamingStarted,
    ) -> Result<()> {
        let detached_window_fetch = pending.continuation.is_detached_window_fetch();
        debug_assert!(
            detached_window_fetch || pending.execution_context.is_window_network_only(),
            "network-only stream must be an accepted fire-and-forget request or detached Fetch"
        );
        self._context_host
            .borrow_mut()
            .record_deferred_pending_subresource_request_started(&mut pending);

        let security_error = detached_window_fetch
            .then(|| {
                if !started.head.redirect_chain.is_empty() {
                    detached_window_fetch_csp_redirect_failure_message(
                        &self._context_host,
                        &pending,
                        &started.head.final_url,
                    )
                } else {
                    None
                }
                .or_else(|| {
                    crate::network_host::validate_fetch_response_security_policy(
                        &pending.info.document_url,
                        &started.head.final_url,
                        &started.head.headers,
                        pending.request_mode,
                        pending.credentials_mode,
                        pending.policy_context,
                    )
                    .err()
                })
            })
            .flatten();

        if let Some(error_text) = security_error {
            pending.load.cancel();
            let mut network_record = crate::types::SubresourceNetworkRecord::failure(
                pending.info.frame_id.clone(),
                pending.info.document_url.clone(),
                started.request_url,
                started.request_method,
                started.request_headers,
                started.request_body,
                pending.info.resource_type,
                error_text,
            )
            .with_request_initiator_type(SubresourceRequestInitiatorType::Script)
            .with_request_body_bytes(pending.info.request_body_bytes.clone());
            if let Some(handle) = pending.info.network_request_handle {
                network_record = network_record.with_request_handle(handle);
            }
            self._context_host
                .borrow_mut()
                .record_subresource_network(network_record);
            self._context_host
                .borrow_mut()
                .record_pending_subresource_continue_event(
                    PendingSubresourceContinueEvent::Completed {
                        internal_id: started.internal_id,
                    },
                );
            return Ok(());
        }

        let detached_identity = pending.execution_context.detached_window_fetch_identity();
        let accepted_context = pending.execution_context.window_network_only_identity();
        let accepted_document = pending
            .execution_context
            .window_document_network_only_identity();
        self._context_host
            .borrow_mut()
            .record_streaming_subresource_fetch(StreamingSubresourceFetchState {
                pending,
                request_url: started.request_url,
                request_method: started.request_method,
                request_headers: started.request_headers,
                request_body: started.request_body,
                body_source_id: started.body_source_id,
                head: started.head,
                network_request_headers: started.network_request_headers,
                body_writer: SubresourceResponseBodyWriter::default(),
                event_source_parser: None,
                xhr_response: None,
            });
        tracing::debug!(
            internal_id = started.internal_id,
            ?detached_identity,
            ?accepted_context,
            ?accepted_document,
            "continued network-only subresource streaming without a V8 body source"
        );
        Ok(())
    }

    /// Standalone ScriptVm test turn for a streaming-start terminal.
    #[cfg(test)]
    pub(super) fn start_streaming_async_subresource_fetch(
        &mut self,
        started: crate::types::AsyncSubresourceStreamingStarted,
    ) -> Result<()> {
        let activity = self.start_streaming_async_subresource_fetch_body(started)?;
        self.finish_async_subresource_body_checkpoint_for_test(activity)
    }

    fn start_streaming_async_subresource_fetch_body(
        &mut self,
        started: crate::types::AsyncSubresourceStreamingStarted,
    ) -> Result<AsyncSubresourceFetchBodyActivity> {
        let trace_started = moli_trace::cdp_runtime_trace_enabled().then(Instant::now);
        let trace_fields = AsyncSubresourceTraceFields {
            event_kind: Some("streaming_started"),
            internal_id: Some(started.internal_id),
            body_source_id: Some(started.body_source_id),
            ..AsyncSubresourceTraceFields::default()
        };
        trace_async_subresource_stage(
            "async_subresource_streaming_start_begin",
            trace_fields,
            trace_started,
        );
        let Some(mut pending) = self
            ._context_host
            .borrow_mut()
            .take_pending_subresource_fetch(started.internal_id)
        else {
            trace_async_subresource_stage(
                "async_subresource_streaming_start_missing",
                trace_fields,
                trace_started,
            );
            return Ok(AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered);
        };
        let trace_fields = async_subresource_trace_fields_for_pending_with_body(
            "streaming_started",
            started.internal_id,
            Some(started.body_source_id),
            &pending,
        );
        if pending.continuation.is_detached_window_fetch()
            || pending.execution_context.is_window_network_only()
        {
            return self
                .start_network_only_subresource_stream(pending, started)
                .map(|()| AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered);
        }
        if !window_subresource_owner_is_current(&self._context_host, &pending) {
            pending.load.cancel();
            self._context_host
                .borrow_mut()
                .record_pending_subresource_continue_event(
                    PendingSubresourceContinueEvent::Completed {
                        internal_id: started.internal_id,
                    },
                );
            tracing::debug!(
                internal_id = started.internal_id,
                owner = ?pending.execution_context.window_request_target().map(crate::native_bridge::WindowTaskTarget::owner),
                "discarded streaming subresource start for retired Window execution context"
            );
            return Ok(AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered);
        }
        self._context_host
            .borrow_mut()
            .record_deferred_pending_subresource_request_started(&mut pending);

        let pending_context_ptr: *const v8::Global<v8::Context> = pending
            .execution_context
            .context_global()
            .expect("active streaming subresource must retain its V8 context");
        let result = self.renderer_document_isolate.with_entered_renderer_document_isolate(|isolate| {
            let scope = pin!(v8::HandleScope::new(isolate));
            let scope = &mut scope.init();
            // SAFETY: `pending_context_ptr` points into `pending.execution_context`, which stays
            // alive until the streaming state is recorded after this non-escaping closure returns.
            let context = unsafe { v8::Local::new(scope, &*pending_context_ptr) };
            let scope = &mut v8::ContextScope::new(scope, context);
            if !window_subresource_realm_is_current(&self._context_host, scope, &pending) {
                pending.load.cancel();
                self._context_host
                    .borrow_mut()
                    .record_pending_subresource_continue_event(
                        PendingSubresourceContinueEvent::Completed {
                            internal_id: started.internal_id,
                        },
                    );
                tracing::debug!(
                    internal_id = started.internal_id,
                    expected_realm = ?pending.execution_context.realm_token(),
                    "discarded streaming subresource start for retired V8 realm"
                );
                return Ok(AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered);
            }
            trace_async_subresource_stage(
                "async_subresource_streaming_context_entered",
                trace_fields,
                trace_started,
            );
            let owner_async_scope =
                enter_subresource_owner_async_scope(
                    &self._context_host,
                    scope,
                    pending.execution_context.dispatch_scope(),
                );

            let streaming_start_error = if !started.head.redirect_chain.is_empty() {
                document_connect_csp_redirect_failure_message(
                    scope,
                    &self._context_host,
                    &pending,
                    &started.head.final_url,
                )
            } else {
                None
            }
            .or_else(|| {
                matches!(
                    pending.info.resource_type,
                    SubresourceResourceType::EventSource
                        | SubresourceResourceType::Fetch
                        | SubresourceResourceType::Xhr
                )
                .then(|| {
                    crate::network_host::validate_fetch_response_security_policy(
                        &pending.info.document_url,
                        &started.head.final_url,
                        &started.head.headers,
                        pending.request_mode,
                        pending.credentials_mode,
                        pending.policy_context,
                    )
                    .err()
                })
                .flatten()
            });
            if let Some(error_text) = streaming_start_error {
                pending.load.cancel();
                let network_error_text =
                    if crate::network_host::is_cors_policy_failure_message(&error_text) {
                        crate::network_host::FAILED_ERROR_TEXT.to_owned()
                    } else {
                        error_text.clone()
                    };
                let mut network_record = crate::types::SubresourceNetworkRecord::failure(
                    pending.info.frame_id.clone(),
                    pending.info.document_url.clone(),
                    started.request_url.clone(),
                    started.request_method.clone(),
                    started.request_headers.clone(),
                    started.request_body.clone(),
                    pending.info.resource_type,
                    network_error_text,
                )
                .with_request_body_bytes(pending.info.request_body_bytes.clone());
                if let Some(handle) = pending.info.network_request_handle {
                    network_record = network_record.with_request_handle(handle);
                }
                self._context_host
                    .borrow_mut()
                    .record_subresource_network(network_record);
                match &pending.continuation {
                    PendingSubresourceContinuation::Fetch(fetch) => {
                        let resolver = fetch
                            .resolver()
                            .expect("detached keepalive stream is handled before V8 entry");
                        let resolver = v8::Local::new(scope, resolver);
                        let exception = v8_string(scope, &error_text)
                            .map(|message| v8::Exception::type_error(scope, message))
                            .unwrap_or_else(|| v8::undefined(scope).into());
                        resolver.reject(scope, exception);
                    }
                    PendingSubresourceContinuation::Xhr(xhr) => {
                        let xhr = v8::Local::new(scope, xhr);
                        crate::network_host::apply_xhr_failure(scope, xhr);
                    }
                    PendingSubresourceContinuation::EventSource(event_source) => {
                        let event_source = v8::Local::new(scope, event_source);
                        crate::network_host::fail_event_source_connection(
                            scope,
                            event_source,
                            crate::network_host::EventSourceTerminalMode::Close,
                        );
                    }
                    PendingSubresourceContinuation::Image {
                        image_handle,
                        sequence,
                        ..
                    } => apply_image_subresource_terminal(
                        scope,
                        &self._context_host,
                        *image_handle,
                        *sequence,
                        started.internal_id,
                        &started.request_url,
                        ImageSubresourceTerminal::Failure,
                    ),
                    PendingSubresourceContinuation::Media {
                        media_handle,
                        sequence,
                    } => apply_media_subresource_terminal(
                        scope,
                        &self._context_host,
                        *media_handle,
                        *sequence,
                        started.internal_id,
                        false,
                    ),
                    PendingSubresourceContinuation::TextTrack {
                        track_handle,
                        sequence,
                    } => apply_text_track_subresource_terminal(
                        scope,
                        &self._context_host,
                        *track_handle,
                        *sequence,
                        started.internal_id,
                        Err(error_text.clone()),
                    ),
                    PendingSubresourceContinuation::StylesheetSubresource { binding, .. } => {
                        apply_stylesheet_subresource_terminal(&self._context_host, *binding);
                    }
                    PendingSubresourceContinuation::Beacon
                    | PendingSubresourceContinuation::CspReport { .. }
                    | PendingSubresourceContinuation::WebSocket(_)
                    | PendingSubresourceContinuation::WorkerFetch { .. }
                    | PendingSubresourceContinuation::WorkerXhr { .. }
                    | PendingSubresourceContinuation::WorkerCspReport { .. }
                    | PendingSubresourceContinuation::SharedWorkerFetch { .. }
                    | PendingSubresourceContinuation::SharedWorkerXhr { .. }
                    | PendingSubresourceContinuation::SharedWorkerCspReport { .. } => {}
                }
                defer_subresource_owner_async_scope(
                    &self._context_host,
                    scope,
                    pending.execution_context.dispatch_scope(),
                    owner_async_scope,
                );
                self._context_host
                    .borrow_mut()
                    .record_pending_subresource_continue_event(
                        PendingSubresourceContinueEvent::Completed {
                            internal_id: started.internal_id,
                        },
                    );
                return Ok(AsyncSubresourceFetchBodyActivity::WindowRealmEntered);
            }

            if pending.continuation.is_window_event_source()
                && let Some(error_text) =
                    crate::network_host::event_source_response_error(&started.head)
            {
                pending.load.cancel();
                if let Some(handle) = pending.info.network_request_handle {
                    self._context_host
                        .borrow_mut()
                        .record_subresource_response_started(
                            crate::types::SubresourceResponseStarted::new(
                                handle,
                                started
                                    .head
                                    .redirect_chain
                                    .clone()
                                    .into_iter()
                                    .map(Into::into)
                                    .collect(),
                                started.head.final_url.clone(),
                                started.head.status,
                                started.head.headers.clone(),
                                started.head.cookie_set_reports.clone(),
                            )
                            .with_from_cache(started.head.from_cache)
                            .with_negotiated_http_version(
                                started.head.negotiated_http_version,
                            )
                            .with_network_request_headers(
                                started.network_request_headers.clone(),
                            ),
                        );
                    self._context_host
                        .borrow_mut()
                        .record_subresource_body_finished(
                            crate::types::SubresourceBodyFinished::failed(handle, error_text),
                        );
                }
                if let PendingSubresourceContinuation::EventSource(event_source) =
                    &pending.continuation
                {
                    let event_source = v8::Local::new(scope, event_source);
                    crate::network_host::fail_event_source_connection(
                        scope,
                        event_source,
                        crate::network_host::EventSourceTerminalMode::Close,
                    );
                }
                defer_subresource_owner_async_scope(
                    &self._context_host,
                    scope,
                    pending.execution_context.dispatch_scope(),
                    owner_async_scope,
                );
                self._context_host
                    .borrow_mut()
                    .record_pending_subresource_continue_event(
                        PendingSubresourceContinueEvent::Completed {
                            internal_id: started.internal_id,
                        },
                    );
                return Ok(AsyncSubresourceFetchBodyActivity::WindowRealmEntered);
            }

            let mut observable_head = started.head.clone();
            if matches!(
                pending.info.resource_type,
                SubresourceResourceType::Fetch | SubresourceResourceType::Xhr
            ) {
                observable_head.headers = crate::network_host::filter_cors_exposed_response_headers(
                    &pending.info.document_url,
                    &observable_head.final_url,
                    &observable_head.headers,
                    pending.credentials_mode,
                );
            }

            if matches!(&pending.continuation, PendingSubresourceContinuation::Xhr(_))
                && let Some(handle) = pending.info.network_request_handle
            {
                self._context_host
                    .borrow_mut()
                    .record_subresource_response_started(
                        crate::types::SubresourceResponseStarted::new(
                            handle,
                            started
                                .head
                                .redirect_chain
                                .clone()
                                .into_iter()
                                .map(Into::into)
                                .collect(),
                            started.head.final_url.clone(),
                            started.head.status,
                            started.head.headers.clone(),
                            started.head.cookie_set_reports.clone(),
                        )
                        .with_from_cache(started.head.from_cache)
                        .with_negotiated_http_version(started.head.negotiated_http_version)
                        .with_network_request_headers(started.network_request_headers.clone()),
                    );
            }

            if let PendingSubresourceContinuation::Xhr(xhr) = &pending.continuation {
                let xhr = v8::Local::new(scope, xhr);
                let pending_owner = pending.execution_context.dispatch_scope();
                let xhr_response = crate::types::XhrStreamingResponseState::new(
                    &observable_head.headers,
                );
                self._context_host
                    .borrow_mut()
                    .record_streaming_subresource_fetch(StreamingSubresourceFetchState {
                        pending,
                        request_url: started.request_url.clone(),
                        request_method: started.request_method.clone(),
                        request_headers: started.request_headers.clone(),
                        request_body: started.request_body.clone(),
                        body_source_id: started.body_source_id,
                        head: started.head.clone(),
                        network_request_headers: started.network_request_headers.clone(),
                        body_writer: SubresourceResponseBodyWriter::default(),
                        event_source_parser: None,
                        xhr_response: Some(xhr_response),
                    });
                let remains_current =
                    crate::network_host::apply_xhr_streaming_response_head(
                        scope,
                        xhr,
                        &observable_head,
                        started.internal_id,
                    );
                if !remains_current {
                    // Registering the stream before dispatching state 2 lets abort()
                    // retire the transport synchronously. open() and isolate
                    // termination also invalidate the old XHR generation; retire
                    // any state that remains after those callbacks return.
                    let _ = self
                        ._context_host
                        .borrow_mut()
                        .abort_subresource_fetch(started.internal_id);
                }
                defer_subresource_owner_async_scope(
                    &self._context_host,
                    scope,
                    pending_owner,
                    owner_async_scope,
                );
                return Ok(AsyncSubresourceFetchBodyActivity::WindowRealmEntered);
            }

            let mut event_source_parser = None;
            let mut event_source_to_open = None;
            match &pending.continuation {
                PendingSubresourceContinuation::Fetch(fetch) => {
                    let resolver = fetch
                        .resolver()
                        .expect("detached keepalive stream is handled before V8 entry");
                    let resolver = v8::Local::new(scope, resolver);
                    let response_obj = crate::network_host::build_fetch_response_object_from_stream_for_request_mode(
                        scope,
                        &pending.info.document_url,
                        pending.request_mode,
                        observable_head,
                        started.body_source_id,
                    );
                    resolver.resolve(scope, response_obj.into());
                }
                PendingSubresourceContinuation::Image {
                    image_handle,
                    sequence,
                    ..
                } => apply_image_subresource_terminal(
                    scope,
                    &self._context_host,
                    *image_handle,
                    *sequence,
                    started.internal_id,
                    &started.request_url,
                    // Image requests require the complete body for MIME sniffing and decode.
                    // Production image transports are buffered, so a streaming head is an
                    // invalid terminal rather than a successful image response.
                    ImageSubresourceTerminal::Failure,
                ),
                PendingSubresourceContinuation::Media {
                    media_handle,
                    sequence,
                } => apply_media_subresource_terminal(
                    scope,
                    &self._context_host,
                    *media_handle,
                    *sequence,
                    started.internal_id,
                    crate::network_host::media_response_status_is_successful(started.head.status),
                ),
                PendingSubresourceContinuation::TextTrack {
                    track_handle,
                    sequence,
                } => apply_text_track_subresource_terminal(
                    scope,
                    &self._context_host,
                    *track_handle,
                    *sequence,
                    started.internal_id,
                    Err("text-track response unexpectedly used streaming transport".to_owned()),
                ),
                PendingSubresourceContinuation::StylesheetSubresource { binding, .. } => {
                    apply_stylesheet_subresource_terminal(&self._context_host, *binding);
                }
                PendingSubresourceContinuation::EventSource(event_source) => {
                    if let Some(handle) = pending.info.network_request_handle {
                        self._context_host
                            .borrow_mut()
                            .record_subresource_response_started(
                                crate::types::SubresourceResponseStarted::new(
                                    handle,
                                    started
                                        .head
                                        .redirect_chain
                                        .clone()
                                        .into_iter()
                                        .map(Into::into)
                                        .collect(),
                                    started.head.final_url.clone(),
                                    started.head.status,
                                    started.head.headers.clone(),
                                    started.head.cookie_set_reports.clone(),
                                )
                                .with_from_cache(started.head.from_cache)
                                .with_negotiated_http_version(
                                    started.head.negotiated_http_version,
                                )
                                .with_network_request_headers(
                                    started.network_request_headers.clone(),
                                ),
                            );
                    }
                    let event_source = v8::Local::new(scope, event_source);
                    let last_event_id =
                        crate::network_host::event_source_last_event_id(scope, event_source);
                    let reconnect_delay_ms =
                        crate::network_host::event_source_reconnect_delay_ms(scope, event_source);
                    event_source_parser = Some(crate::network_host::EventSourceParser::new(
                        last_event_id,
                        reconnect_delay_ms,
                    ));
                    event_source_to_open = Some(event_source);
                }
                PendingSubresourceContinuation::Beacon
                | PendingSubresourceContinuation::CspReport { .. }
                | PendingSubresourceContinuation::Xhr(_)
                | PendingSubresourceContinuation::WebSocket(_)
                | PendingSubresourceContinuation::WorkerFetch { .. }
                | PendingSubresourceContinuation::WorkerXhr { .. }
                | PendingSubresourceContinuation::WorkerCspReport { .. }
                | PendingSubresourceContinuation::SharedWorkerFetch { .. }
                | PendingSubresourceContinuation::SharedWorkerXhr { .. }
                | PendingSubresourceContinuation::SharedWorkerCspReport { .. } => {}
            }

            let pending_owner = pending.execution_context.dispatch_scope();
            self._context_host.borrow_mut().record_streaming_subresource_fetch(
                StreamingSubresourceFetchState {
                    pending,
                    request_url: started.request_url.clone(),
                    request_method: started.request_method.clone(),
                    request_headers: started.request_headers.clone(),
                    request_body: started.request_body.clone(),
                    body_source_id: started.body_source_id,
                    head: started.head.clone(),
                    network_request_headers: started.network_request_headers.clone(),
                    body_writer: SubresourceResponseBodyWriter::default(),
                    event_source_parser,
                    xhr_response: None,
                },
            );
            if let Some(event_source) = event_source_to_open {
                crate::network_host::open_event_source_connection(
                    scope,
                    event_source,
                    &started.head.final_url,
                );
            }

            defer_subresource_owner_async_scope(
                &self._context_host,
                scope,
                pending_owner,
                owner_async_scope,
            );
            Ok(AsyncSubresourceFetchBodyActivity::WindowRealmEntered)
        });
        trace_async_subresource_stage(
            "async_subresource_streaming_start_isolate_done",
            trace_fields,
            trace_started,
        );
        result
    }

    /// Standalone ScriptVm test turn for one streaming body chunk.
    #[cfg(test)]
    pub(super) fn append_streaming_async_subresource_fetch_chunk(
        &mut self,
        body_source_id: NetworkBodySourceId,
        bytes: Vec<u8>,
    ) {
        let activity =
            self.append_streaming_async_subresource_fetch_chunk_body(body_source_id, bytes);
        self.finish_async_subresource_body_checkpoint_for_test(activity)
            .expect("streaming chunk test task checkpoint should complete");
    }

    fn append_streaming_async_subresource_fetch_chunk_body(
        &mut self,
        body_source_id: NetworkBodySourceId,
        bytes: Vec<u8>,
    ) -> AsyncSubresourceFetchBodyActivity {
        let trace_started = moli_trace::cdp_runtime_trace_enabled().then(Instant::now);
        let bytes_len = bytes.len();
        let trace_fields = AsyncSubresourceTraceFields {
            event_kind: Some("streaming_chunk"),
            body_source_id: Some(body_source_id),
            bytes: Some(bytes_len),
            ..AsyncSubresourceTraceFields::default()
        };
        trace_async_subresource_stage(
            "async_subresource_streaming_chunk_start",
            trace_fields,
            trace_started,
        );
        let is_xhr = self
            ._context_host
            .borrow()
            .streaming_subresource_is_xhr(body_source_id);
        if is_xhr {
            let context_host = self._context_host.clone();
            let entered = self
                .renderer_document_isolate
                .with_entered_renderer_document_isolate(|isolate| {
                    let scope = pin!(v8::HandleScope::new(isolate));
                    let scope = &mut scope.init();
                    let Some(delivery) = context_host.borrow_mut().append_streaming_xhr_chunk(
                        scope,
                        body_source_id,
                        &bytes,
                    ) else {
                        return Ok(false);
                    };
                    if let Some(handle) = delivery.request_handle {
                        context_host.borrow_mut().record_subresource_data_received(
                            crate::types::SubresourceDataReceived::new(
                                handle, bytes_len, bytes_len,
                            ),
                        );
                    }
                    let context = delivery.context;
                    let xhr = delivery.xhr;
                    let dispatch_scope = delivery.dispatch_scope;
                    let realm_token = delivery.realm_token;
                    let internal_id = delivery.internal_id;
                    let scope = &mut v8::ContextScope::new(scope, context);
                    if realm_token.is_some_and(|expected| {
                        crate::native_bridge::current_runtime_observable_context_token(scope)
                            != Some(expected)
                    }) {
                        let _ = context_host
                            .borrow_mut()
                            .abort_subresource_fetch(internal_id);
                        return Ok(false);
                    }
                    let previous =
                        enter_subresource_owner_async_scope(&context_host, scope, dispatch_scope);
                    let remains_current = crate::network_host::apply_xhr_streaming_response_chunk(
                        scope,
                        xhr,
                        internal_id,
                        &delivery.decoded_text,
                        delivery.loaded,
                        delivery.total,
                    );
                    defer_subresource_owner_async_scope(
                        &context_host,
                        scope,
                        dispatch_scope,
                        previous,
                    );
                    if !remains_current {
                        let _ = context_host
                            .borrow_mut()
                            .abort_subresource_fetch(internal_id);
                    }
                    Ok(true)
                })
                .unwrap_or(false);
            trace_async_subresource_stage(
                "async_subresource_streaming_chunk_done",
                trace_fields,
                trace_started,
            );
            return if entered {
                AsyncSubresourceFetchBodyActivity::WindowRealmEntered
            } else {
                AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered
            };
        }
        let is_event_source = self
            ._context_host
            .borrow()
            .streaming_subresource_is_event_source(body_source_id);
        if is_event_source {
            let entered = self
                .renderer_document_isolate
                .with_entered_renderer_document_isolate(|isolate| {
                    let scope = pin!(v8::HandleScope::new(isolate));
                    let scope = &mut scope.init();
                    let Some(delivery) = self
                        ._context_host
                        .borrow_mut()
                        .append_streaming_event_source_chunk(scope, body_source_id, &bytes)
                    else {
                        return Ok(false);
                    };
                    if let Some(handle) = delivery.request_handle {
                        self._context_host
                            .borrow_mut()
                            .record_subresource_data_received(
                                crate::types::SubresourceDataReceived::new(
                                    handle, bytes_len, bytes_len,
                                ),
                            );
                    }
                    let context = delivery.context;
                    let event_source = delivery.event_source;
                    let scope = &mut v8::ContextScope::new(scope, context);
                    dispatch_streaming_event_source_messages(
                        &self._context_host,
                        scope,
                        event_source,
                        delivery.request_handle,
                        &delivery.messages,
                    );
                    Ok(true)
                })
                .unwrap_or(false);
            trace_async_subresource_stage(
                "async_subresource_streaming_chunk_done",
                trace_fields,
                trace_started,
            );
            return if entered {
                AsyncSubresourceFetchBodyActivity::WindowRealmEntered
            } else {
                AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered
            };
        }
        let body_binding = self
            ._context_host
            .borrow()
            .streaming_subresource_body_binding_by_body_source_id(body_source_id);
        let append_started = moli_trace::cdp_runtime_trace_enabled().then(Instant::now);
        self._context_host
            .borrow_mut()
            .append_streaming_subresource_body(body_source_id, &bytes);
        trace_async_subresource_stage(
            "async_subresource_streaming_chunk_appended",
            trace_fields,
            append_started,
        );
        let mut activity = AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered;
        if let Some((context_ptr, dispatch_scope, realm_token)) = body_binding {
            let enqueue_started = moli_trace::cdp_runtime_trace_enabled().then(Instant::now);
            let context_host = self._context_host.clone();
            let entered = self
                .renderer_document_isolate
                .with_entered_renderer_document_isolate(|isolate| {
                    let scope = pin!(v8::HandleScope::new(isolate));
                    let scope = &mut scope.init();
                    let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                    let scope = &mut v8::ContextScope::new(scope, context);
                    if realm_token.is_some_and(|expected| {
                        crate::native_bridge::current_runtime_observable_context_token(scope)
                            != Some(expected)
                    }) {
                        return Ok(false);
                    }
                    let previous =
                        enter_subresource_owner_async_scope(&context_host, scope, dispatch_scope);
                    // The CDP capture already borrowed this chunk into its
                    // body-writer above, so the original Vec can move into the
                    // Web-visible stream without cloning the full chunk.
                    crate::network_host::enqueue_pending_network_body_chunk(
                        scope,
                        body_source_id,
                        bytes,
                    );
                    defer_subresource_owner_async_scope(
                        &context_host,
                        scope,
                        dispatch_scope,
                        previous,
                    );
                    Ok(true)
                })
                .unwrap_or(false);
            if entered {
                activity = AsyncSubresourceFetchBodyActivity::WindowRealmEntered;
            }
            trace_async_subresource_stage(
                "async_subresource_streaming_chunk_enqueued",
                trace_fields,
                enqueue_started,
            );
        }
        trace_async_subresource_stage(
            "async_subresource_streaming_chunk_done",
            trace_fields,
            trace_started,
        );
        activity
    }

    fn finish_network_only_subresource_stream(
        &mut self,
        streaming: StreamingSubresourceFetchState,
        internal_id: u64,
        result: std::result::Result<(), String>,
    ) -> Result<()> {
        debug_assert!(
            streaming.pending.continuation.is_detached_window_fetch()
                || streaming.pending.execution_context.is_window_network_only(),
            "network-only stream must be an accepted fire-and-forget request or detached Fetch"
        );
        let detached_identity = streaming
            .pending
            .execution_context
            .detached_window_fetch_identity();
        let accepted_context = streaming
            .pending
            .execution_context
            .window_network_only_identity();
        let accepted_document = streaming
            .pending
            .execution_context
            .window_document_network_only_identity();
        match result {
            Ok(()) => {
                let request_cookie_report = streaming
                    .head
                    .request_cookie_report
                    .clone()
                    .or_else(|| streaming.pending.info.request_cookie_report.clone());
                let response_body = streaming.body_writer.finish();
                let mut network_record = crate::types::SubresourceNetworkRecord::success_with_body(
                    streaming.pending.info.frame_id.clone(),
                    streaming.pending.info.document_url.clone(),
                    streaming.request_url,
                    streaming.request_method,
                    streaming.request_headers,
                    streaming.request_body,
                    streaming.pending.info.resource_type,
                    request_cookie_report,
                    streaming
                        .head
                        .redirect_chain
                        .into_iter()
                        .map(Into::into)
                        .collect(),
                    streaming.head.final_url,
                    streaming.head.status,
                    streaming.head.headers,
                    response_body,
                    streaming.head.cookie_set_reports,
                )
                .with_from_cache(streaming.head.from_cache)
                .with_negotiated_http_version(streaming.head.negotiated_http_version)
                .with_network_request_headers(streaming.network_request_headers)
                .with_request_initiator_type(SubresourceRequestInitiatorType::Script)
                .with_request_body_bytes(streaming.pending.info.request_body_bytes.clone());
                if let Some(handle) = streaming.pending.info.network_request_handle {
                    network_record = network_record.with_request_handle(handle);
                }
                self._context_host
                    .borrow_mut()
                    .record_subresource_network(network_record);
            }
            Err(_) => {
                let network_error_text = crate::network_host::ABORTED_ERROR_TEXT.to_owned();
                if let Some(handle) = streaming.pending.info.network_request_handle {
                    let partial_body = streaming.body_writer.finish();
                    self._context_host
                        .borrow_mut()
                        .record_subresource_response_started(
                            crate::types::SubresourceResponseStarted::new(
                                handle,
                                streaming
                                    .head
                                    .redirect_chain
                                    .into_iter()
                                    .map(Into::into)
                                    .collect(),
                                streaming.head.final_url,
                                streaming.head.status,
                                streaming.head.headers,
                                streaming.head.cookie_set_reports,
                            )
                            .with_from_cache(streaming.head.from_cache)
                            .with_negotiated_http_version(streaming.head.negotiated_http_version)
                            .with_network_request_headers(streaming.network_request_headers),
                        );
                    self._context_host
                        .borrow_mut()
                        .record_subresource_body_finished(
                            crate::types::SubresourceBodyFinished::failed_with_partial_body(
                                handle,
                                network_error_text,
                                partial_body,
                            ),
                        );
                } else {
                    self._context_host.borrow_mut().record_subresource_network(
                        crate::types::SubresourceNetworkRecord::failure(
                            streaming.pending.info.frame_id.clone(),
                            streaming.pending.info.document_url.clone(),
                            streaming.request_url,
                            streaming.request_method,
                            streaming.request_headers,
                            streaming.request_body,
                            streaming.pending.info.resource_type,
                            network_error_text,
                        )
                        .with_request_initiator_type(SubresourceRequestInitiatorType::Script)
                        .with_request_body_bytes(streaming.pending.info.request_body_bytes.clone()),
                    );
                }
            }
        }
        self._context_host
            .borrow_mut()
            .record_pending_subresource_continue_event(
                PendingSubresourceContinueEvent::Completed { internal_id },
            );
        tracing::debug!(
            internal_id,
            ?detached_identity,
            ?accepted_context,
            ?accepted_document,
            "finished network-only subresource without entering V8"
        );
        Ok(())
    }

    /// Standalone ScriptVm test turn for a streaming-finish terminal.
    #[cfg(test)]
    pub(super) fn finish_streaming_async_subresource_fetch(
        &mut self,
        internal_id: u64,
        body_source_id: NetworkBodySourceId,
        result: std::result::Result<(), String>,
    ) -> Result<()> {
        let activity = self.finish_streaming_async_subresource_fetch_body(
            internal_id,
            body_source_id,
            result,
        )?;
        self.finish_async_subresource_body_checkpoint_for_test(activity)
    }

    fn finish_streaming_async_subresource_fetch_body(
        &mut self,
        internal_id: u64,
        body_source_id: NetworkBodySourceId,
        result: std::result::Result<(), String>,
    ) -> Result<AsyncSubresourceFetchBodyActivity> {
        let trace_started = moli_trace::cdp_runtime_trace_enabled().then(Instant::now);
        let trace_fields = AsyncSubresourceTraceFields {
            event_kind: Some("streaming_finished"),
            internal_id: Some(internal_id),
            body_source_id: Some(body_source_id),
            ..AsyncSubresourceTraceFields::default()
        };
        trace_async_subresource_stage(
            "async_subresource_streaming_finish_start",
            trace_fields,
            trace_started,
        );
        let Some(mut streaming) = self
            ._context_host
            .borrow_mut()
            .take_streaming_subresource_fetch(internal_id)
        else {
            trace_async_subresource_stage(
                "async_subresource_streaming_finish_missing",
                trace_fields,
                trace_started,
            );
            return Ok(AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered);
        };
        let trace_fields = async_subresource_trace_fields_for_pending_with_body(
            "streaming_finished",
            internal_id,
            Some(body_source_id),
            &streaming.pending,
        );
        if streaming.pending.continuation.is_detached_window_fetch()
            || streaming.pending.execution_context.is_window_network_only()
        {
            return self
                .finish_network_only_subresource_stream(streaming, internal_id, result)
                .map(|()| AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered);
        }
        if !window_subresource_owner_is_current(&self._context_host, &streaming.pending) {
            self._context_host
                .borrow_mut()
                .record_pending_subresource_continue_event(
                    PendingSubresourceContinueEvent::Completed { internal_id },
                );
            tracing::debug!(
                internal_id,
                owner = ?streaming.pending.execution_context.window_request_target().map(crate::native_bridge::WindowTaskTarget::owner),
                "discarded streaming subresource finish for retired Window execution context"
            );
            return Ok(AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered);
        }

        let context_host = self._context_host.clone();
        let result = self
            .renderer_document_isolate
            .with_entered_renderer_document_isolate(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = v8::Local::new(
                    scope,
                    streaming
                        .pending
                        .execution_context
                        .context_global()
                        .expect("active streaming finish must retain its V8 context"),
                );
                let scope = &mut v8::ContextScope::new(scope, context);
                if !window_subresource_realm_is_current(&context_host, scope, &streaming.pending) {
                    context_host
                        .borrow_mut()
                        .record_pending_subresource_continue_event(
                            PendingSubresourceContinueEvent::Completed { internal_id },
                        );
                    tracing::debug!(
                        internal_id,
                        expected_realm = ?streaming.pending.execution_context.realm_token(),
                        "discarded streaming subresource finish for retired V8 realm"
                    );
                    return Ok(AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered);
                }
                trace_async_subresource_stage(
                    "async_subresource_streaming_finish_context_entered",
                    trace_fields,
                    trace_started,
                );
                let pending_owner = streaming.pending.execution_context.dispatch_scope();
                let owner_async_scope =
                    enter_subresource_owner_async_scope(&context_host, scope, pending_owner);
                if streaming.pending.continuation.is_window_event_source() {
                    let event_source = match &streaming.pending.continuation {
                        PendingSubresourceContinuation::EventSource(event_source) => {
                            v8::Local::new(scope, event_source)
                        }
                        _ => unreachable!("EventSource continuation was checked above"),
                    };
                    if let Some(parser) = streaming.event_source_parser.take() {
                        crate::network_host::update_event_source_stream_state(
                            scope,
                            event_source,
                            parser.last_event_id(),
                            parser.reconnect_delay_ms(),
                        );
                    }
                    let response_body = streaming.body_writer.finish();
                    if let Some(handle) = streaming.pending.info.network_request_handle {
                        let body = match result {
                            Ok(()) => crate::types::SubresourceBodyFinished::ready_after_streaming(
                                handle,
                                response_body,
                            ),
                            Err(error_text) => {
                                crate::types::SubresourceBodyFinished::failed_with_partial_body(
                                    handle,
                                    error_text,
                                    response_body,
                                )
                            }
                        };
                        context_host
                            .borrow_mut()
                            .record_subresource_body_finished(body);
                    }
                    if crate::network_host::event_source_ready_state(scope, event_source)
                        != crate::network_host::EVENT_SOURCE_CLOSED
                    {
                        crate::network_host::fail_event_source_connection(
                            scope,
                            event_source,
                            crate::network_host::EventSourceTerminalMode::Reconnect,
                        );
                    }
                    defer_subresource_owner_async_scope(
                        &context_host,
                        scope,
                        pending_owner,
                        owner_async_scope,
                    );
                    context_host
                        .borrow_mut()
                        .record_pending_subresource_continue_event(
                            PendingSubresourceContinueEvent::Completed { internal_id },
                        );
                    return Ok(AsyncSubresourceFetchBodyActivity::WindowRealmEntered);
                }
                match result {
                    Ok(()) => {
                        let request_cookie_report = streaming
                            .head
                            .request_cookie_report
                            .clone()
                            .or_else(|| streaming.pending.info.request_cookie_report.clone());
                        let finish_body_started =
                            moli_trace::cdp_runtime_trace_enabled().then(Instant::now);
                        let response_body = streaming.body_writer.finish();
                        trace_async_subresource_stage(
                            "async_subresource_streaming_body_finished",
                            trace_fields,
                            finish_body_started,
                        );
                        let xhr_delivery_body = if matches!(
                            &streaming.pending.continuation,
                            PendingSubresourceContinuation::Xhr(_)
                        ) {
                            let materialize_started =
                                moli_trace::cdp_runtime_trace_enabled().then(Instant::now);
                            match response_body.try_materialized_body() {
                                Ok(body) => {
                                    trace_async_subresource_stage(
                                        "async_subresource_streaming_xhr_body_materialized",
                                        trace_fields,
                                        materialize_started,
                                    );
                                    Some(body)
                                }
                                Err(error) => {
                                    let error_text = format!(
                                        "failed to materialize streaming XHR body: {error}"
                                    );
                                    crate::network_host::error_pending_network_body_stream(
                                        scope,
                                        body_source_id,
                                        error_text.clone(),
                                    );
                                    let request_handle =
                                        streaming.pending.info.network_request_handle;
                                    let mut network_record =
                                        crate::types::SubresourceNetworkRecord::failure(
                                            streaming.pending.info.frame_id.clone(),
                                            streaming.pending.info.document_url.clone(),
                                            streaming.request_url,
                                            streaming.request_method,
                                            streaming.request_headers,
                                            streaming.request_body,
                                            streaming.pending.info.resource_type,
                                            error_text,
                                        )
                                        .with_request_body_bytes(
                                            streaming.pending.info.request_body_bytes.clone(),
                                        );
                                    if let Some(handle) = request_handle {
                                        network_record = network_record.with_request_handle(handle);
                                    }
                                    context_host
                                        .borrow_mut()
                                        .record_subresource_network(network_record);
                                    if let PendingSubresourceContinuation::Xhr(xhr) =
                                        streaming.pending.continuation
                                    {
                                        let xhr = v8::Local::new(scope, &xhr);
                                        crate::network_host::apply_xhr_failure(scope, xhr);
                                    }
                                    context_host
                                        .borrow_mut()
                                        .record_pending_subresource_continue_event(
                                            PendingSubresourceContinueEvent::Completed {
                                                internal_id,
                                            },
                                        );
                                    defer_subresource_owner_async_scope(
                                        &context_host,
                                        scope,
                                        pending_owner,
                                        owner_async_scope,
                                    );
                                    return Ok(
                                        AsyncSubresourceFetchBodyActivity::WindowRealmEntered,
                                    );
                                }
                            }
                        } else {
                            None
                        };
                        let close_started =
                            moli_trace::cdp_runtime_trace_enabled().then(Instant::now);
                        crate::network_host::close_pending_network_body_stream(
                            scope,
                            body_source_id,
                        );
                        trace_async_subresource_stage(
                            "async_subresource_stream_closed",
                            trace_fields,
                            close_started,
                        );
                        let record_started =
                            moli_trace::cdp_runtime_trace_enabled().then(Instant::now);
                        let request_handle = streaming.pending.info.network_request_handle;
                        if matches!(
                            &streaming.pending.continuation,
                            PendingSubresourceContinuation::Xhr(_)
                        ) && let Some(handle) = request_handle
                        {
                            context_host.borrow_mut().record_subresource_body_finished(
                                crate::types::SubresourceBodyFinished::ready_after_streaming(
                                    handle,
                                    response_body,
                                ),
                            );
                        } else {
                            let mut network_record =
                                crate::types::SubresourceNetworkRecord::success_with_body(
                                    streaming.pending.info.frame_id.clone(),
                                    streaming.pending.info.document_url.clone(),
                                    streaming.request_url,
                                    streaming.request_method,
                                    streaming.request_headers,
                                    streaming.request_body,
                                    streaming.pending.info.resource_type,
                                    request_cookie_report,
                                    streaming
                                        .head
                                        .redirect_chain
                                        .clone()
                                        .into_iter()
                                        .map(Into::into)
                                        .collect(),
                                    streaming.head.final_url.clone(),
                                    streaming.head.status,
                                    streaming.head.headers.clone(),
                                    response_body,
                                    streaming.head.cookie_set_reports.clone(),
                                )
                                .with_from_cache(streaming.head.from_cache)
                                .with_negotiated_http_version(
                                    streaming.head.negotiated_http_version,
                                )
                                .with_network_request_headers(
                                    streaming.network_request_headers.clone(),
                                )
                                .with_request_body_bytes(
                                    streaming.pending.info.request_body_bytes.clone(),
                                );
                            if let Some(handle) = request_handle {
                                network_record = network_record.with_request_handle(handle);
                            }
                            context_host
                                .borrow_mut()
                                .record_subresource_network(network_record);
                        }
                        trace_async_subresource_stage(
                            "async_subresource_streaming_network_recorded",
                            trace_fields,
                            record_started,
                        );
                        if let PendingSubresourceContinuation::Xhr(xhr) =
                            streaming.pending.continuation
                            && let Some(response_body) = xhr_delivery_body
                        {
                            let xhr_started =
                                moli_trace::cdp_runtime_trace_enabled().then(Instant::now);
                            let xhr = v8::Local::new(scope, &xhr);
                            let mut observable_head = streaming.head;
                            observable_head.headers =
                                crate::network_host::filter_cors_exposed_response_headers(
                                    &streaming.pending.info.document_url,
                                    &observable_head.final_url,
                                    &observable_head.headers,
                                    streaming.pending.credentials_mode,
                                );
                            // XHR still exposes a complete response at DONE. Keep the
                            // network transfer and CDP record streaming/spooled, and
                            // materialize only at this Web-visible completion boundary.
                            crate::network_host::apply_xhr_streaming_response_body_source(
                                scope,
                                xhr,
                                observable_head,
                                response_body,
                                internal_id,
                            );
                            trace_async_subresource_stage(
                                "async_subresource_streaming_xhr_delivered",
                                trace_fields,
                                xhr_started,
                            );
                        }
                    }
                    Err(error_text) => {
                        let error_started =
                            moli_trace::cdp_runtime_trace_enabled().then(Instant::now);
                        crate::network_host::error_pending_network_body_stream(
                            scope,
                            body_source_id,
                            error_text.clone(),
                        );
                        let network_error_text = crate::network_host::ABORTED_ERROR_TEXT.to_owned();
                        let request_handle = streaming.pending.info.network_request_handle;
                        if let Some(handle) = request_handle {
                            let partial_body = streaming.body_writer.finish();
                            if !matches!(
                                &streaming.pending.continuation,
                                PendingSubresourceContinuation::Xhr(_)
                            ) {
                                context_host
                                    .borrow_mut()
                                    .record_subresource_response_started(
                                        crate::types::SubresourceResponseStarted::new(
                                            handle,
                                            streaming
                                                .head
                                                .redirect_chain
                                                .clone()
                                                .into_iter()
                                                .map(Into::into)
                                                .collect(),
                                            streaming.head.final_url.clone(),
                                            streaming.head.status,
                                            streaming.head.headers.clone(),
                                            streaming.head.cookie_set_reports.clone(),
                                        )
                                        .with_from_cache(streaming.head.from_cache)
                                        .with_negotiated_http_version(
                                            streaming.head.negotiated_http_version,
                                        )
                                        .with_network_request_headers(
                                            streaming.network_request_headers.clone(),
                                        ),
                                    );
                            }
                            context_host.borrow_mut().record_subresource_body_finished(
                                crate::types::SubresourceBodyFinished::failed_with_partial_body(
                                    handle,
                                    network_error_text,
                                    partial_body,
                                ),
                            );
                        } else {
                            context_host.borrow_mut().record_subresource_network(
                                crate::types::SubresourceNetworkRecord::failure(
                                    streaming.pending.info.frame_id.clone(),
                                    streaming.pending.info.document_url.clone(),
                                    streaming.request_url,
                                    streaming.request_method,
                                    streaming.request_headers,
                                    streaming.request_body,
                                    streaming.pending.info.resource_type,
                                    network_error_text,
                                )
                                .with_request_body_bytes(
                                    streaming.pending.info.request_body_bytes.clone(),
                                ),
                            );
                        }
                        trace_async_subresource_stage(
                            "async_subresource_streaming_error_recorded",
                            trace_fields,
                            error_started,
                        );
                    }
                }
                context_host
                    .borrow_mut()
                    .record_pending_subresource_continue_event(
                        PendingSubresourceContinueEvent::Completed { internal_id },
                    );
                defer_subresource_owner_async_scope(
                    &context_host,
                    scope,
                    pending_owner,
                    owner_async_scope,
                );
                Ok(AsyncSubresourceFetchBodyActivity::WindowRealmEntered)
            });
        trace_async_subresource_stage(
            "async_subresource_streaming_finish_done",
            trace_fields,
            trace_started,
        );
        result
    }

    #[cfg(test)]
    pub(crate) fn current_document_write_external_script_fetch_target(
        &self,
    ) -> Option<crate::types::DocumentWriteExternalScriptFetchTarget> {
        let target = self
            .document_runtime
            .pending_document_write_external_script_fetch_target()?;
        self.document_write_external_script_fetch_target_is_current(target)
            .then_some(target)
    }

    pub(crate) fn document_write_external_script_fetch_target_is_current(
        &self,
        expected: crate::types::DocumentWriteExternalScriptFetchTarget,
    ) -> bool {
        self.document_runtime
            .has_document_write_external_script_fetch_target(expected)
            && self.current_main_document_task_owner() == Some(expected.task_owner())
    }

    pub(crate) fn apply_current_document_write_external_script_load_completion(
        &mut self,
        completion: crate::runtime::AuthorizedCurrentDocumentWriteExternalScriptLoadCompletion,
    ) -> Result<crate::document_runtime::DocumentWriteExternalScriptLoadApplication> {
        let completion = completion.into_completion();
        let document_runtime: *mut DocumentRuntime = &mut *self.document_runtime;
        self.with_default_context_scope(|scope, host_ptr| {
            Ok(unsafe { &mut *document_runtime }
                .complete_document_write_external_script_load(scope, host_ptr, completion))
        })
    }

    pub(crate) fn apply_current_child_document_load_completion(
        &mut self,
        authorization: AuthorizedCurrentChildDocumentLoadCompletion,
    ) -> Result<CurrentChildDocumentLoadApplication> {
        self.apply_current_child_document_load_completion_inner(authorization.into_completion())
    }

    fn apply_current_child_document_load_completion_inner(
        &mut self,
        completion: ChildDocumentLoadCompletion,
    ) -> Result<CurrentChildDocumentLoadApplication> {
        let context_host = self._context_host.clone();
        let application = self.with_default_context_scope(move |scope, _host_ptr| {
            Ok(context_host
                .borrow_mut()
                .apply_current_child_document_load_completion(scope, completion))
        })?;
        let application = match application {
            crate::native_bridge::ChildDocumentLoadApplication::Applied {
                followup,
                body_activity,
            } => (followup.map(|application| *application), body_activity),
            crate::native_bridge::ChildDocumentLoadApplication::SupersededDuringApplication {
                completion,
                body_activity,
            } => {
                let historical_network_recorded =
                    self.record_historical_child_document_load_network(&completion);
                self.apply_pending_child_document_owner_retirements();
                return Ok(
                    CurrentChildDocumentLoadApplication::SupersededDuringApplication {
                        historical_network_recorded,
                        body_activity,
                    },
                );
            }
        };
        self.apply_pending_child_document_owner_retirements();
        let (application, body_activity) = application;
        let Some(application) = application else {
            return Ok(CurrentChildDocumentLoadApplication::Applied { body_activity });
        };
        let (work, parser_stop_action, owner_transition) = application.into_followups();
        if let Some(transition) = owner_transition {
            self.apply_child_document_owner_transition(transition);
        }
        if let Some(action) = parser_stop_action {
            super::child_document_lifecycle::ChildDocumentLifecycleOwner::new(self)
                .notify_parser_stop_action(action);
        }
        if let Some(work) = work {
            super::child_document_script_scheduler::ChildDocumentScriptSchedulerOwner::new(self)
                .notify_parser_classic_next_owner_action(work);
        }
        Ok(CurrentChildDocumentLoadApplication::Applied { body_activity })
    }

    pub(crate) fn current_child_document_navigation_fetch_target(
        &self,
        child_handle: crate::document_runtime::DomHandle,
    ) -> Option<crate::frame_owner_model::ChildDocumentNavigationFetchTarget> {
        self._context_host
            .borrow()
            .current_child_document_navigation_fetch_target(child_handle)
    }

    pub(crate) fn record_historical_child_document_load_network(
        &mut self,
        completion: &ChildDocumentLoadCompletion,
    ) -> bool {
        self._context_host
            .borrow_mut()
            .record_historical_child_document_load_network(completion)
    }

    pub(crate) fn discard_stale_child_document_load_completion(
        &mut self,
        target: crate::frame_owner_model::ChildDocumentNavigationFetchTarget,
    ) {
        self._context_host
            .borrow_mut()
            .discard_stale_child_document_load_completion(target);
    }

    pub(crate) fn apply_child_blocking_stylesheet_load_completion_from_page_turn(
        &mut self,
        completion: ChildBlockingStylesheetLoadCompletion,
    ) -> Result<()> {
        let context_host = self._context_host.clone();
        self.with_default_context_scope(move |scope, _host_ptr| {
            context_host
                .borrow_mut()
                .apply_child_blocking_stylesheet_load_completion(scope, completion);
            Ok(())
        })
    }

    pub(crate) fn record_historical_child_blocking_stylesheet_network_results(
        &mut self,
        completion: &ChildBlockingStylesheetLoadCompletion,
    ) {
        self._context_host
            .borrow_mut()
            .record_historical_child_blocking_stylesheet_network_results(completion);
    }

    pub(crate) fn current_child_document_task_owner(
        &self,
        child_handle: crate::document_runtime::DomHandle,
    ) -> Option<crate::frame_owner_model::FrameDocumentTaskOwner> {
        self._context_host
            .borrow()
            .current_child_document_task_owner(child_handle)
    }

    pub(crate) fn current_child_document_module_fetch_target(
        &self,
        child_handle: crate::document_runtime::DomHandle,
    ) -> Option<crate::frame_owner_model::ChildDocumentModuleFetchTarget> {
        self._context_host
            .borrow()
            .current_child_document_module_fetch_target(child_handle)
    }

    pub(crate) fn apply_child_classic_script_load_completion_from_page_turn(
        &mut self,
        completion: ChildClassicScriptLoadCompletion,
    ) -> Result<()> {
        let context_host = self._context_host.clone();
        let application = self.with_default_context_scope(move |_scope, _host_ptr| {
            Ok(context_host
                .borrow_mut()
                .apply_child_classic_script_load_completion(completion))
        })?;
        if let Some(application) = application {
            if let Some(work) = application.scheduler_work {
                super::child_document_script_scheduler::ChildDocumentScriptSchedulerOwner::new(
                    self,
                )
                .notify_parser_classic_next_owner_action(work);
                return Ok(());
            }
            let _ = application.queued_document_script_ready;
            let _ = application.queued_document_lifecycle;
        }
        Ok(())
    }

    pub(crate) fn record_historical_child_classic_script_network_result(
        &mut self,
        completion: &ChildClassicScriptLoadCompletion,
    ) -> bool {
        self._context_host
            .borrow_mut()
            .record_historical_child_classic_script_network_result(completion)
    }

    /// Applies a parser-module terminal only after the Page owner has proved
    /// that its complete exact target is current.
    pub(crate) fn apply_current_child_parser_module_root_fetch_completion(
        &mut self,
        authorization: AuthorizedCurrentChildModuleFetchCompletion<
            ChildParserModuleRootFetchCompletion,
        >,
    ) -> FrameDocumentModuleTerminalQueueFollowup {
        self.finish_child_parser_module_root_fetch_completion(authorization.into_completion())
    }

    fn finish_child_parser_module_root_fetch_completion(
        &mut self,
        completion: ChildParserModuleRootFetchCompletion,
    ) -> FrameDocumentModuleTerminalQueueFollowup {
        let applied = self
            ._context_host
            .borrow_mut()
            .finish_child_parser_module_root_fetch_request(&completion);
        if applied {
            return self.apply_child_parser_module_root_fetch_completion_to_owner(completion);
        }
        FrameDocumentModuleTerminalQueueFollowup::none()
    }

    /// Applies a dependency terminal only after the Page owner has proved
    /// that its complete exact target is current.
    pub(crate) fn apply_current_child_module_dependency_fetch_completion(
        &mut self,
        authorization: AuthorizedCurrentChildModuleFetchCompletion<
            ChildModuleDependencyFetchCompletion,
        >,
    ) -> FrameDocumentModuleTerminalQueueFollowup {
        self.finish_child_module_dependency_fetch_completion(authorization.into_completion())
    }

    /// Applies a child `modulepreload` terminal only after the Page owner has
    /// proved its complete exact target is current.
    pub(crate) fn apply_current_child_modulepreload_fetch_completion(
        &mut self,
        authorization: AuthorizedCurrentChildModuleFetchCompletion<
            ChildModulepreloadFetchCompletion,
        >,
    ) -> FrameDocumentModuleTerminalQueueFollowup {
        super::child_module_fetch::ChildModuleFetchOwner::new(self)
            .apply_current_modulepreload_fetch_completion(authorization.into_completion())
    }

    fn finish_child_module_dependency_fetch_completion(
        &mut self,
        completion: ChildModuleDependencyFetchCompletion,
    ) -> FrameDocumentModuleTerminalQueueFollowup {
        let applied = self
            ._context_host
            .borrow_mut()
            .finish_child_module_dependency_fetch_request(&completion);
        if applied {
            return self.apply_child_module_dependency_fetch_completion_to_owner(completion);
        }
        FrameDocumentModuleTerminalQueueFollowup::none()
    }

    pub(crate) fn record_current_child_module_fetch_network_result(
        &mut self,
        attribution: &crate::types::ChildModuleFetchNetworkAttribution,
        network_result: Option<&crate::types::SharedNavigationResponseResult>,
    ) -> bool {
        let Some(network_result) = network_result else {
            return false;
        };
        self._context_host
            .borrow_mut()
            .record_current_child_module_fetch_network_result(attribution, network_result.as_ref());
        true
    }

    pub(crate) fn record_historical_child_module_fetch_network_result(
        &mut self,
        attribution: &crate::types::ChildModuleFetchNetworkAttribution,
        network_result: Option<&crate::types::SharedNavigationResponseResult>,
    ) -> bool {
        let Some(network_result) = network_result else {
            return false;
        };
        self._context_host
            .borrow_mut()
            .record_historical_child_module_fetch_network_result(
                attribution,
                network_result.as_ref(),
            );
        true
    }

    #[cfg(test)]
    pub(crate) fn complete_popup_document_load(
        &mut self,
        completion: PopupDocumentLoadCompletion,
    ) -> Result<()> {
        let target = completion.target();
        if self.current_lightweight_popup_document_fetch_target(target.load_id()) != Some(target) {
            return Ok(());
        }
        let _ = self.apply_popup_document_load_completion_inner(completion)?;
        Ok(())
    }

    pub(crate) fn current_lightweight_popup_document_fetch_target(
        &self,
        load_id: u64,
    ) -> Option<crate::native_bridge::LightweightPopupDocumentFetchTarget> {
        self._context_host
            .borrow()
            .current_lightweight_popup_document_fetch_target(load_id)
    }

    pub(crate) fn apply_current_popup_document_load_completion(
        &mut self,
        authorization: AuthorizedCurrentPopupDocumentLoadCompletion,
    ) -> Result<crate::native_bridge::PopupDocumentLoadApplication> {
        self.apply_popup_document_load_completion_inner(authorization.into_completion())
    }

    fn apply_popup_document_load_completion_inner(
        &mut self,
        completion: PopupDocumentLoadCompletion,
    ) -> Result<crate::native_bridge::PopupDocumentLoadApplication> {
        let context_host = self._context_host.clone();
        self.with_default_context_scope(move |scope, _host_ptr| {
            let application = context_host
                .borrow_mut()
                .apply_lightweight_popup_document_load_completion(scope, completion);
            Ok(application)
        })
    }

    pub(crate) fn current_lightweight_popup_classic_script_fetch_target(
        &self,
        load_id: u64,
    ) -> Option<crate::native_bridge::LightweightPopupClassicScriptFetchTarget> {
        self._context_host
            .borrow()
            .current_lightweight_popup_classic_script_fetch_target(load_id)
    }

    pub(crate) fn discard_stale_lightweight_popup_classic_script_completion(
        &mut self,
        target: crate::native_bridge::LightweightPopupClassicScriptFetchTarget,
    ) {
        self._context_host
            .borrow_mut()
            .discard_stale_lightweight_popup_classic_script_completion(target);
    }

    pub(crate) fn apply_current_popup_classic_script_load_completion(
        &mut self,
        authorization: AuthorizedCurrentPopupClassicScriptLoadCompletion,
    ) -> Result<crate::native_bridge::PopupClassicScriptLoadApplication> {
        let completion: PopupClassicScriptLoadCompletion = authorization.into_completion();
        let context_host = self._context_host.clone();
        self.with_default_context_scope(move |scope, _host_ptr| {
            Ok(context_host
                .borrow_mut()
                .apply_lightweight_popup_classic_script_load_completion(scope, completion))
        })
    }
}

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
        PendingSubresourceContinuation::Xhr(_) => "xhr",
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
