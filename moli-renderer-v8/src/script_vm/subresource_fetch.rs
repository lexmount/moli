use crate::network::{
    PausedResourceResponse, ResourceBodyResponse, ResourceResponseBody, ResourceResponseFailure,
    ResourceResponseHead,
};
use crate::service_worker_runtime::ServiceWorkerFetchResultSender;
use anyhow::{Result, anyhow, bail};
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
    AsyncSubresourceFetchResponseFilter, AsyncSubresourceFetchResult,
    ChildBlockingStylesheetLoadCompletion, ChildClassicScriptLoadCompletion,
    ChildDocumentLoadCompletion, ChildModuleDependencyFetchCompletion,
    ChildModulepreloadFetchCompletion, ChildParserModuleRootFetchCompletion, NetworkBodySourceId,
    PendingSubresourceAuthInfo, PendingSubresourceAuthState, PendingSubresourceContinuation,
    PendingSubresourceContinueEvent, PendingSubresourceContinueOutcome,
    PendingSubresourceFetchInfo, PendingSubresourceFetchState, PendingSubresourceResponseInfo,
    PendingSubresourceResponseState, PopupClassicScriptLoadCompletion, PopupDocumentLoadCompletion,
    RunningSubresourceFetchState, StreamingSubresourceFetchState, SubresourceNetworkRequestHandle,
    SubresourceResourceType, SubresourceResponseBody,
};
use crate::util::v8_string;

fn pending_subresource_request(
    pending: &PendingSubresourceFetchState,
    url: &Url,
    method: &str,
    headers: &moli_fetch::RequestHeaders,
) -> Result<moli_fetch::Request> {
    let request = moli_fetch::Request::new_bytes(
        method,
        url.as_str(),
        pending.info.request_body_bytes.clone(),
        headers.clone(),
    )?
    .with_initiator_url(&pending.info.document_url)
    .with_request_origin(pending.request_origin.clone())
    .with_redirect_headers(pending.redirect_headers.clone())
    .with_request_mode(pending.request_mode)
    .with_credentials_mode(pending.credentials_mode)
    .with_network_partition_key(pending.network_partition_key.clone())
    .with_subframe_context(pending.info.frame_id.is_some());
    let request = match pending.info.resource_type {
        SubresourceResourceType::Script
        | SubresourceResourceType::Stylesheet
        | SubresourceResourceType::Image
        | SubresourceResourceType::Font
        | SubresourceResourceType::Audio
        | SubresourceResourceType::Video
        | SubresourceResourceType::Media
        | SubresourceResourceType::TextTrack
        | SubresourceResourceType::Ping
        | SubresourceResourceType::Dictionary => {
            match crate::network::request_resource_type_for_subresource(pending.info.resource_type)
            {
                Some(resource_type) => request.with_resource_type(resource_type),
                None => request,
            }
        }
        SubresourceResourceType::Fetch => {
            request.with_browser_request_metadata(moli_fetch::BrowserRequestMetadata::Fetch)
        }
        SubresourceResourceType::CspReport => request
            .with_resource_type(moli_fetch::RequestResourceType::CspReport)
            .with_redirect_mode(moli_fetch::RequestRedirectMode::Error),
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
        SubresourceResourceType::Document | SubresourceResourceType::WebSocket => request,
    };
    Ok(match pending.continuation.window_fetch() {
        Some(fetch) => fetch.options.apply(request),
        None => request,
    })
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

    if pending.continuation.window_fetch().is_some() {
        return pending
            .response_stream()
            .window_fetch_policy()
            .expect("Window fetch retains its response policy")
            .check_redirect(final_url, |context, violation| {
                report_window_fetch_csp_redirect_violation(scope, context_host, context, violation);
            });
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
    pending.continuation.window_fetch()?;
    pending
        .response_stream()
        .window_fetch_policy()
        .expect("Window fetch retains its response policy")
        .check_redirect(final_url, |context, violation| {
            crate::network_host::send_content_security_policy_violation_report_from_window_context(
                &mut context_host.borrow_mut(),
                context,
                violation,
            );
        })
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
        let loader = self
            .current_main_document_resource_loader()
            .expect("parser request must retain its active Document resource loader");
        let request = crate::planning::external_script_request(
            &script,
            &loader.fetch_context().request_origin(),
            Some(moli_fetch::RequestResourceType::ParserBlockingScript),
        );
        let Some((request_load, network, started)) = loader.prepare_resource_request(
            &request,
            SubresourceResourceType::Script,
            crate::types::SubresourceRequestInitiatorType::Parser,
        ) else {
            completer.finish(
                crate::planning::external_script_source_load_outcome_from_result(
                    &script,
                    &loader.fetch_context().request_origin(),
                    Err("Document resource owner retired".into()),
                    document_character_set.as_deref(),
                ),
            );
            return load;
        };
        let request_handle = network.handle();
        self._context_host
            .borrow_mut()
            .record_native_resource_observation(started);
        let (info, continuation) = browser_context_runtime.prepare_detached_parser_script_fetch(
            PendingSubresourceFetchInfo {
                internal_id: 0,
                network_request_handle: Some(request_handle),
                frame_id: self.root_frame_id.clone(),
                document_url: script.initiator_url.clone(),
                url: script.url.clone(),
                websocket_socket_id: None,
                method: "GET".to_owned(),
                request_headers: Default::default(),
                request_body: None,
                request_body_bytes: None,
                resource_type: SubresourceResourceType::Script,
                request_cookie_report: None,
            },
            script,
            loader,
            (request_load, network),
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

    pub(crate) fn continue_pending_subresource_fetch_body(
        &mut self,
        internal_id: u64,
        url: Option<Url>,
        method: Option<String>,
        body: Option<Option<String>>,
        headers: Option<moli_fetch::RequestHeaderOverride>,
        intercept_response: bool,
        handle_auth_requests: bool,
    ) -> Result<AsyncSubresourceCommandExecution<PendingSubresourceContinueOutcome>> {
        let mut pending = self
            ._context_host
            .borrow_mut()
            .take_pending_subresource_fetch(internal_id)
            .ok_or_else(|| anyhow!("unknown pending subresource fetch `{internal_id}`"))?;
        let headers = headers.map(|override_headers| match override_headers {
            moli_fetch::RequestHeaderOverride::CurrentRequest {
                headers,
                redirect_headers,
            } => {
                pending.redirect_headers =
                    Some(redirect_headers.unwrap_or_else(|| pending.info.request_headers.clone()));
                headers
            }
            moli_fetch::RequestHeaderOverride::RedirectChain(headers) => {
                pending.redirect_headers = None;
                headers
            }
        });
        if let Some(body) = &body {
            pending.info.request_body = body.clone();
            pending.info.request_body_bytes = body.as_ref().map(|body| body.as_bytes().to_vec());
        }
        if let Some(network) = &pending.network {
            network.configure_interception(intercept_response, handle_auth_requests);
            let info = &mut pending.info;
            let changed = url.as_ref().is_some_and(|url| url != &info.url)
                || method.as_ref().is_some_and(|method| method != &info.method)
                || headers
                    .as_ref()
                    .is_some_and(|headers| headers != &info.request_headers)
                || body.is_some();
            if let Some(url) = &url {
                info.url = url.clone();
            }
            if let Some(method) = &method {
                info.method = method.clone();
            }
            if let Some(headers) = &headers {
                info.request_headers = headers.clone();
            }
            if changed {
                network.network.update_request(|request| {
                    crate::network_host::resource_request_started(
                        request,
                        info,
                        pending.continuation.request_initiator_type(),
                        pending.load.disposition()
                            == crate::network::loads::ResourceLoadDisposition::Keepalive,
                    )
                });
            }
        }
        let PendingSubresourceFetchState {
            redirect_headers,
            request_origin,
            info,
            load,
            execution_context,
            credentials_mode,
            request_mode,
            network_partition_key,
            policy_context,
            continuation,
            network,
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
            PendingSubresourceContinuation::CspReport { client_id } => {
                let request_url = url.unwrap_or_else(|| info.url.clone());
                let request_method = method.unwrap_or_else(|| info.method.clone());
                let request_body = body.unwrap_or_else(|| info.request_body.clone());
                let request_headers = headers.unwrap_or_else(|| info.request_headers.clone());
                let pending = PendingSubresourceFetchState {
                    redirect_headers: redirect_headers.clone(),
                    request_origin,
                    info,
                    load,
                    execution_context,
                    credentials_mode,
                    request_mode,
                    network_partition_key,
                    policy_context,
                    continuation: PendingSubresourceContinuation::CspReport { client_id },
                    network,
                };
                if !self._context_host.borrow().network_offline() {
                    let maybe_pending = self.continue_csp_report_via_service_worker(
                        pending,
                        client_id,
                        request_url.clone(),
                        request_method.clone(),
                        request_headers.clone(),
                        request_body.clone(),
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
                    );
                }
                return self.continue_pending_subresource_fetch_via_loader(
                    pending,
                    request_url,
                    request_method,
                    request_headers,
                    request_body,
                );
            }
            continuation => PendingSubresourceFetchState {
                redirect_headers: redirect_headers.clone(),
                request_origin,
                info,
                load,
                execution_context,
                credentials_mode,
                request_mode,
                network_partition_key,
                policy_context,
                continuation,
                network,
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
        )
    }

    fn continue_pending_subresource_fetch_via_loader(
        &mut self,
        pending: PendingSubresourceFetchState,
        request_url: Url,
        request_method: String,
        request_headers: moli_fetch::RequestHeaders,
        request_body: Option<String>,
    ) -> Result<AsyncSubresourceCommandExecution<PendingSubresourceContinueOutcome>> {
        let internal_id = pending.info.internal_id;
        if pending.load.network_offline() {
            let activity = self.resolve_pending_subresource_fetch_body(
                pending,
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
        let request =
            pending_subresource_request(&pending, &request_url, &request_method, &request_headers)?;
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
        request_headers: moli_fetch::RequestHeaders,
        request_body: Option<String>,
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

        let request =
            pending_subresource_request(&pending, &request_url, &request_method, &request_headers)?;

        let cancel_handle = moli_fetch::FetchCancelHandle::new();
        pending.load.attach_cancel_handle(cancel_handle.clone());
        let internal_id = pending.info.internal_id;
        let policy_context = pending.policy_context;
        let request_cookie_report = pending.info.request_cookie_report.clone();
        let document_url = pending.info.document_url.clone();
        let frame_id = pending.info.frame_id.clone();
        let completion_tx = self._context_host.borrow().resource_completion_sender();
        let response_stream = pending.response_stream().clone();
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
            cors_preflight_request_headers: Vec::new(),
            request_cookie_report,
            network_context: crate::types::AsyncSubresourceNetworkContext {
                frame_id,
                request_origin: pending.request_origin.clone(),
                document_url,
                resource_type: SubresourceResourceType::CspReport,
                policy_context,
            },
            result_tx: ServiceWorkerFetchResultSender::Page {
                completion_tx,
                network: response_stream,
            },
            request_client,
            resource_task_runner,
            cancel_handle: cancel_handle.clone(),
        };

        {
            let mut host = self._context_host.borrow_mut();
            host.begin_active_subresource_request();
            host.record_running_subresource_fetch(RunningSubresourceFetchState {
                pending,
                request_url: request_url.clone(),
                request_method: request_method.clone(),
                request_headers: request_headers.clone(),
                request_body: request_body.clone(),
            });
        }
        self._context_host
            .borrow()
            .dispatch_service_worker_fetch(dispatch);
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
            response,
        } = pending;
        let response = response.discard();
        let loader = pending_fetch.load.request_client();
        let mut request = pending_subresource_request(
            &pending_fetch,
            &request_url,
            &request_method,
            &original_request_headers,
        )?
        .with_auth(auth.into());
        for redirect in &response.redirect_chain {
            request.apply_redirect_status(redirect.status);
        }
        request.url = response.final_url.clone();
        request = request.with_redirect_chain(response.redirect_chain);
        let request_headers = original_request_headers;
        if self._context_host.borrow().network_offline() {
            let activity = self.resolve_pending_subresource_fetch_body(
                pending_fetch,
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
        pending.response.discard();
        let activity = self.resolve_pending_subresource_fetch_body(
            pending.pending,
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
            response,
        } = pending;
        let head = response.body.head();
        let intercept_response = pending.response_stream().intercept_response();
        let response_info = PendingSubresourceResponseInfo {
            internal_id,
            url: request_url.clone(),
            final_url: head.final_url.clone(),
            method: request_method.clone(),
            request_headers: request_headers.clone(),
            request_body: request_body.clone(),
            resource_type: pending.info.resource_type,
            request_cookie_report: head.request_cookie_report.clone(),
            network_request_headers: pending.response_stream().record_request_headers(None),
            response_status: head.status,
            response_headers: head.headers.clone(),
            response_body: response.body.body_source(),
            from_cache: head.from_cache,
        };
        self._context_host
            .borrow_mut()
            .record_pending_subresource_response(PendingSubresourceResponseState {
                pending,
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
            redirect_headers: _,
            request_origin,
            info,
            load,
            execution_context,
            credentials_mode,
            request_mode,
            network_partition_key,
            policy_context,
            continuation,
            network,
        } = pending;
        let pending = match continuation {
            PendingSubresourceContinuation::WebSocket(connection) => {
                self._context_host
                    .borrow_mut()
                    .fail_pending_websocket_connection(connection, error_text)
                    .map_err(|error| anyhow!(error))?;
                return Ok(AsyncSubresourceCommandExecution::without_window_realm(()));
            }
            continuation => PendingSubresourceFetchState {
                redirect_headers: None,
                request_origin,
                info,
                load,
                execution_context,
                credentials_mode,
                request_mode,
                network_partition_key,
                policy_context,
                continuation,
                network,
            },
        };
        let activity = self.resolve_pending_subresource_fetch_body(
            pending,
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
        response_headers: Vec<(String, Vec<u8>)>,
        response_body: RendererSyntheticResponseBody,
    ) -> Result<AsyncSubresourceCommandExecution<()>> {
        let pending = self
            ._context_host
            .borrow_mut()
            .take_pending_subresource_fetch(internal_id)
            .ok_or_else(|| anyhow!("unknown pending subresource fetch `{internal_id}`"))?;
        let PendingSubresourceFetchState {
            redirect_headers: _,
            request_origin,
            info,
            load,
            execution_context,
            credentials_mode,
            request_mode,
            network_partition_key,
            policy_context,
            continuation,
            network,
        } = pending;
        // Request-stage fulfillment has no followed redirects yet. Reuse this
        // complete head for validation and response materialization.
        let head = moli_fetch::ResponseHead {
            final_url: info.url.clone(),
            status: response_code,
            headers: response_headers.clone(),
            request_cookie_report: info.request_cookie_report.clone(),
            cookie_set_reports: Vec::new(),
            redirected: false,
            redirect_chain: Vec::new(),
            from_cache: false,
            negotiated_http_version: None,
        };
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
            continuation => PendingSubresourceFetchState {
                redirect_headers: None,
                request_origin,
                info,
                load,
                execution_context,
                credentials_mode,
                request_mode,
                network_partition_key,
                policy_context,
                continuation,
                network,
            },
        };
        let activity = self.resolve_pending_subresource_fetch_body(
            pending,
            None,
            false,
            None,
            None,
            Ok(response_body.into_navigation_response(head)),
        )?;
        Ok(AsyncSubresourceCommandExecution::after_body((), activity))
    }

    pub(crate) fn continue_pending_subresource_response_body(
        &mut self,
        internal_id: u64,
        response_code: Option<u16>,
        response_headers: Option<Vec<(String, Vec<u8>)>>,
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
        self._context_host
            .borrow_mut()
            .record_running_response(pending.pending);
        pending.response.resume(response_code, response_headers);
        Ok(AsyncSubresourceCommandExecution::without_window_realm(()))
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
        pending.response.discard();
        let activity = self.resolve_pending_subresource_fetch_body(
            pending.pending,
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
        response_headers: Vec<(String, Vec<u8>)>,
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
        let head = pending.response.discard();
        let activity = self.resolve_pending_subresource_fetch_body(
            pending.pending,
            None,
            false,
            None,
            None,
            Ok(
                response_body.into_navigation_response(moli_fetch::ResponseHead {
                    final_url: head.final_url,
                    status: response_code,
                    headers: response_headers,
                    request_cookie_report: head.request_cookie_report,
                    cookie_set_reports: Vec::new(),
                    redirected: false,
                    redirect_chain: Vec::new(),
                    from_cache: head.from_cache,
                    negotiated_http_version: head.negotiated_http_version,
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
        headers: Option<moli_fetch::RequestHeaderOverride>,
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
        response_headers: Vec<(String, Vec<u8>)>,
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
        pending: PendingSubresourceFetchState,
        skip_fetch_security_validation: bool,
        network_error_text: Option<String>,
        result: std::result::Result<crate::protocol_types::NavigationResponse, String>,
    ) -> Result<()> {
        let physical = result.as_ref().ok().cloned();
        let result = if pending.continuation.is_detached_window_fetch() {
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
                        &pending.request_origin,
                        &response.head(),
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
        publish_buffered_subresource_result(
            &self._context_host,
            pending.response_stream(),
            physical.as_ref(),
            &result,
            network_error_text.as_deref(),
        );
        pending.load.finish();
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn resolve_pending_event_source_fetch(
        &mut self,
        pending: PendingSubresourceFetchState,
        response_status_text: Option<String>,
        skip_fetch_security_validation: bool,
        network_error_text: Option<String>,
        result: std::result::Result<crate::protocol_types::NavigationResponse, String>,
    ) -> Result<AsyncSubresourceFetchBodyActivity> {
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
                        pending.network().failed_with(
                            &ResourceResponseFailure::Request(error_text),
                            subresource_turn_observer(&context_host),
                        );
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
                                        &pending.request_origin,
                                        &response.head(),
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
                            publish_buffered_subresource_result(
                                &context_host,
                                pending.response_stream(),
                                Some(&response),
                                &Err(error_text),
                                None,
                            );
                            crate::network_host::fail_event_source_connection(
                                scope,
                                event_source,
                                crate::network_host::EventSourceTerminalMode::Close,
                            );
                        } else {
                            let head = response.head();
                            if let Some(error_text) =
                                crate::network_host::event_source_response_error(&head)
                            {
                                publish_buffered_subresource_result(
                                    &context_host,
                                    pending.response_stream(),
                                    Some(&response),
                                    &Err(error_text),
                                    None,
                                );
                                crate::network_host::fail_event_source_connection(
                                    scope,
                                    event_source,
                                    crate::network_host::EventSourceTerminalMode::Close,
                                );
                            } else {
                                pending.network().body_completed_with(
                                    ResourceResponseHead {
                                        head: response.head(),
                                        status_text: response_status_text.clone(),
                                        network_request_headers: response.network_request_headers().map(<[_]>::to_vec),
                                    },
                                    SubresourceResponseBody::from_navigation_response(&response),
                                    subresource_turn_observer(&context_host),
                                );
                                crate::network_host::open_event_source_connection(
                                    scope,
                                    event_source,
                                    &response.final_url,
                                );
                                let bytes = response.body_bytes();
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
        pending: PendingSubresourceFetchState,
        response_status_text: Option<String>,
        skip_fetch_security_validation: bool,
        response_filter: Option<AsyncSubresourceFetchResponseFilter>,
        network_error_text: Option<String>,
        result: std::result::Result<crate::protocol_types::NavigationResponse, String>,
    ) -> Result<AsyncSubresourceFetchBodyActivity> {
        self.resolve_pending_subresource_fetch_completion(
            pending,
            response_status_text,
            skip_fetch_security_validation,
            response_filter,
            network_error_text,
            result.into(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn resolve_pending_subresource_fetch_completion(
        &mut self,
        pending: PendingSubresourceFetchState,
        response_status_text: Option<String>,
        skip_fetch_security_validation: bool,
        response_filter: Option<AsyncSubresourceFetchResponseFilter>,
        network_error_text: Option<String>,
        completion_result: AsyncSubresourceFetchResult,
    ) -> Result<AsyncSubresourceFetchBodyActivity> {
        let (supplied_parkable_image, result) = match completion_result {
            AsyncSubresourceFetchResult::Response(response) => (None, Ok(response)),
            AsyncSubresourceFetchResult::Image { response, encoded } => {
                (Some(encoded), Ok(response))
            }
            AsyncSubresourceFetchResult::Failure(error) => (None, Err(error)),
        };
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
                response_status_text,
                skip_fetch_security_validation,
                network_error_text,
                result,
            );
        }
        let physical = result.as_ref().ok().cloned();
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
                    let supplied_snapshot = supplied_parkable_image
                        .as_ref()
                        .map(|image| {
                            image.snapshot().map_err(|error| {
                                format!("failed to read image response bytes: {error}")
                            })
                        })
                        .transpose()?;
                    let response_body = supplied_snapshot
                        .as_ref()
                        .map_or_else(|| response.body_bytes(), AsRef::as_ref);
                    let validation = crate::network_host::validate_fetch_response_security_policy_with_body_classified(
                        &pending.request_origin,
                        &response.head(),
                        response_body,
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

            match result {
                Ok(mut response) => {
                    let response_status = response.status;
                    let parkable_image = (!opaque_response_blocked
                        && pending.info.resource_type == SubresourceResourceType::Image)
                        .then(|| {
                            supplied_parkable_image.clone().unwrap_or_else(|| {
                                let runner = pending.load.task_runner();
                                pending
                                    .load
                                    .request_client()
                                    .parkable_image_manager(&runner)
                                    .from_frozen_bytes(response.take_body_bytes())
                            })
                        });
                    let record_started = moli_trace::cdp_runtime_trace_enabled().then(Instant::now);
                    if opaque_response_blocked {
                        publish_buffered_subresource_result(
                            &context_host,
                            pending.response_stream(),
                            physical.as_ref(),
                            &Err(crate::network_host::ABORTED_ERROR_TEXT.to_owned()),
                            None,
                        );
                    } else {
                        pending.network().body_completed_with(
                            ResourceResponseHead {
                                head: response.head(),
                                status_text: response_status_text.clone(),
                                network_request_headers: response.network_request_headers().map(<[_]>::to_vec),
                            },
                            parkable_image.as_ref().map_or_else(|| SubresourceResponseBody::from_navigation_response(&response), |image| SubresourceResponseBody::from_parkable_image(image.clone())),
                            subresource_turn_observer(&context_host),
                        );
                    }
                    trace_async_subresource_stage(
                        "async_subresource_network_recorded",
                        trace_fields,
                        record_started,
                    );
                    let mut observable_response = response;
                    if matches!(
                        pending.info.resource_type,
                        SubresourceResourceType::Fetch | SubresourceResourceType::Xhr
                    ) && !response_filter.is_some_and(|filter| filter.is_readable()) {
                        observable_response.headers =
                            crate::network_host::filter_cors_exposed_response_headers(
                                &pending.request_origin,
                                &observable_response.head(),
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
                                    &pending.request_origin,
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
                            crate::context_bootstrap::record_resource_performance_entry(
                                scope,
                                crate::context_bootstrap::ResourcePerformanceEntry::from_network_response(
                                    pending.info.url.as_str(),
                                    "xmlhttprequest",
                                    None,
                                    &observable_response,
                                ),
                            );
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
                        } => {
                            let encoded = parkable_image
                                .as_ref()
                                .expect("an accepted image response must be parkable");
                            apply_image_subresource_terminal(
                                scope,
                                &context_host,
                                image_handle,
                                sequence,
                                pending.info.internal_id,
                                &pending.info.url,
                                ImageSubresourceTerminal::Response {
                                    response: &observable_response,
                                    encoded,
                                },
                            )
                        }
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
                                let descriptor = crate::network_host::image_response_descriptor_from_parkable(
                                    &observable_response,
                                    parkable_image
                                        .as_ref()
                                        .expect("a CSS image response must be parkable"),
                                );
                                let _ = context_host
                                    .borrow_mut()
                                    .complete_stylesheet_css_image_response(
                                        identity,
                                        descriptor,
                                        parkable_image
                                            .as_ref()
                                            .expect("a CSS image response must be parkable")
                                            .clone(),
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
                        | PendingSubresourceContinuation::WebSocket(_) => {}
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
                    publish_buffered_subresource_result(
                        &context_host,
                        pending.response_stream(),
                        physical.as_ref(),
                        &Err(error_text.clone()),
                        Some(network_error_text),
                    );
                    let continuation_started = moli_trace::cdp_runtime_trace_enabled().then(Instant::now);
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
                        | PendingSubresourceContinuation::WebSocket(_) => {}
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
        let response_stream = state.pending.response_stream().clone();
        let completion_tx = self._context_host.borrow().resource_completion_sender();
        let preflight_observer = state.pending.preflight_observer(completion_tx.clone());
        {
            let mut host = self._context_host.borrow_mut();
            host.begin_active_subresource_request();
            host.record_running_subresource_fetch(state);
        }
        let preflight_headers = request.request_headers.to_byte_strings();
        crate::network_host::spawn_async_subresource_fetch(
            task_runner,
            completion_tx,
            request_client,
            request,
            cancel_handle,
            preflight_headers,
            internal_id,
            response_stream,
            preflight_observer,
            request_url,
        );
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
        let internal_id = running.pending.info.internal_id;
        if let Ok(response) = &result {
            let resource = running.pending.response_stream().clone();
            resource.record_request_headers(response.network_request_headers().map(<[_]>::to_vec));
            if resource.intercepts_response(&response.head()) {
                let mut receiver = crate::network_host::ResourceFetchReceiver::new(
                    running.pending.load.task_runner(),
                    self._context_host.borrow().resource_completion_sender(),
                    internal_id,
                    running.request_url.clone(),
                    resource.clone(),
                );
                receiver.response_status_text = response_status_text.clone();
                receiver.skip_fetch_security_validation = skip_fetch_security_validation;
                receiver.response_filter = response_filter;
                receiver.network_error_text = network_error_text;
                let body = ResourceResponseBody::completed(
                    resource,
                    ResourceBodyResponse::from(response.clone()),
                    response_status_text,
                );
                return self
                    .pause_running_subresource_response(running, receiver.pause(body, false));
            }
        }
        let activity = self.resolve_pending_subresource_fetch_body(
            running.pending,
            response_status_text,
            skip_fetch_security_validation,
            response_filter,
            network_error_text,
            result,
        )?;
        self._context_host
            .borrow_mut()
            .record_pending_subresource_continue_event(
                PendingSubresourceContinueEvent::Completed { internal_id },
            );
        Ok(activity)
    }

    fn pause_running_subresource_response(
        &mut self,
        running: RunningSubresourceFetchState,
        response: PausedResourceResponse,
    ) -> Result<AsyncSubresourceFetchBodyActivity> {
        let RunningSubresourceFetchState {
            pending,
            request_url,
            request_method,
            request_headers,
            request_body,
        } = running;
        if !std::sync::Arc::ptr_eq(pending.response_stream(), &response.body.resource) {
            return Ok(AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered);
        }
        if pending.load.is_detached_keepalive() {
            self._context_host
                .borrow_mut()
                .record_running_response(pending);
            drop(response);
            return Ok(AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered);
        }
        let head = response.body.head();
        let internal_id = pending.info.internal_id;
        let resource_type = pending.info.resource_type;
        let network_request_headers = pending.response_stream().record_request_headers(None);
        let intercept_response = pending.response_stream().intercept_response();
        if pending.response_stream().handle_auth_requests()
            && matches!(head.status, 401 | 407)
            && let Some(challenge) =
                crate::network_host::extract_subresource_auth_challenge(&head.headers)
        {
            let mut challenged = pending_subresource_request(
                &pending,
                &request_url,
                &request_method,
                &request_headers,
            )?;
            for redirect in &head.redirect_chain {
                challenged.apply_redirect_status(redirect.status);
            }
            let info = PendingSubresourceAuthInfo {
                internal_id,
                url: head.final_url.clone(),
                method: challenged.method,
                request_headers: challenged.request_headers,
                request_body: challenged.body.as_ref().and(request_body.clone()),
                resource_type,
                request_cookie_report: head.request_cookie_report,
                network_request_headers,
                challenge,
                intercept_response,
            };
            let mut host = self._context_host.borrow_mut();
            host.record_pending_subresource_auth(PendingSubresourceAuthState {
                pending,
                request_url,
                request_method,
                request_headers,
                request_body,
                response,
            });
            host.record_pending_subresource_continue_event(
                PendingSubresourceContinueEvent::AuthRequired(info),
            );
        } else {
            let info = PendingSubresourceResponseInfo {
                internal_id,
                url: request_url,
                final_url: head.final_url,
                method: request_method,
                request_headers,
                request_body,
                resource_type,
                request_cookie_report: head.request_cookie_report,
                network_request_headers,
                response_status: head.status,
                response_headers: head.headers,
                response_body: response.body.body_source(),
                from_cache: head.from_cache,
            };
            let mut host = self._context_host.borrow_mut();
            host.record_pending_subresource_response(PendingSubresourceResponseState {
                pending,
                response,
            });
            host.record_pending_subresource_continue_event(
                PendingSubresourceContinueEvent::ResponsePaused(info),
            );
        }
        Ok(AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered)
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
        if completion.result.is_err() {
            let network = self._context_host.borrow().subresource_network(internal_id);
            if let Some(network) = network {
                completion.publish_with(&network, subresource_turn_observer(&self._context_host));
            }
        }
        let result = match completion.result {
            Ok(response) => {
                if let Some(encoded) = response.body.parkable_image().cloned() {
                    AsyncSubresourceFetchResult::Image {
                        response: crate::protocol_types::NavigationResponse::from_head_and_body(
                            response.head,
                            String::new(),
                            Vec::new(),
                        )
                        .with_network_request_headers(completion.network_request_headers),
                        encoded,
                    }
                } else {
                    response
                        .into_navigation_response()
                        .map(|response| {
                            response
                                .with_network_request_headers(completion.network_request_headers)
                        })
                        .into()
                }
            }
            Err(error) => AsyncSubresourceFetchResult::Failure(error.to_string()),
        };
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
                result.into_result(),
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
        let activity = self.resolve_pending_subresource_fetch_completion(
            pending,
            completion.response_status_text,
            completion.skip_fetch_security_validation,
            completion.response_filter,
            completion.network_error_text,
            result,
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
            #[cfg(test)]
            AsyncSubresourceFetchEvent::Completion(completion) => {
                self.complete_async_subresource_fetch_body(*completion)
            }
            AsyncSubresourceFetchEvent::TransportCompletion(completion) => {
                let network = self
                    ._context_host
                    .borrow()
                    .subresource_network(completion.internal_id());
                match network.and_then(|network| completion.claim(&network)) {
                    Some(completion) => self.complete_async_subresource_fetch_body(completion),
                    None => Ok(AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered),
                }
            }
            AsyncSubresourceFetchEvent::ResponsePaused {
                internal_id,
                response,
            } => {
                let running = {
                    let mut host = self._context_host.borrow_mut();
                    let running = host.take_running_subresource_fetch(internal_id);
                    if running.is_some() {
                        host.finish_active_subresource_request();
                    }
                    running.or_else(|| {
                        host.take_pending_subresource_fetch(internal_id)
                            .map(|pending| RunningSubresourceFetchState {
                                request_url: pending.info.url.clone(),
                                request_method: pending.info.method.clone(),
                                request_headers: pending.info.request_headers.clone(),
                                request_body: pending.info.request_body.clone(),
                                pending,
                            })
                    })
                };
                match running {
                    Some(running) => self.pause_running_subresource_response(running, *response),
                    None => Ok(AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered),
                }
            }
            AsyncSubresourceFetchEvent::NativeNetwork(observation) => {
                self._context_host
                    .borrow_mut()
                    .record_native_resource_observation(observation);
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
            AsyncSubresourceFetchEvent::TransportStreamingFinished {
                body_source_id,
                completion,
            } => {
                let network = self
                    ._context_host
                    .borrow()
                    .subresource_network(completion.internal_id());
                if let Some((network, completion)) = network.and_then(|network| {
                    completion
                        .claim(&network)
                        .map(|completion| (network, completion))
                }) {
                    completion
                        .publish_with(&network, subresource_turn_observer(&self._context_host));
                    let (result, body) = match completion.result {
                        Ok(response) => (Ok(()), response.body),
                        Err(error) => {
                            let message = error.to_string();
                            let body = match error {
                                ResourceResponseFailure::PartialBody { body, .. } => body,
                                ResourceResponseFailure::Request(_)
                                | ResourceResponseFailure::Network { .. } => {
                                    SubresourceResponseBody::from_bytes(Vec::new())
                                }
                            };
                            (Err(message), body)
                        }
                    };
                    self.finish_streaming_async_subresource_fetch_body(
                        completion.internal_id,
                        body_source_id,
                        result,
                        body,
                    )
                } else {
                    Ok(AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered)
                }
            }
            #[cfg(test)]
            AsyncSubresourceFetchEvent::StreamingFinished(finished) => {
                let body = self
                    ._context_host
                    .borrow_mut()
                    .take_test_streaming_body(finished.internal_id);
                self.finish_streaming_async_subresource_fetch_body(
                    finished.internal_id,
                    finished.body_source_id,
                    finished.result,
                    body,
                )
            }
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

    fn start_detached_window_fetch_stream(
        &mut self,
        pending: PendingSubresourceFetchState,
        started: crate::types::AsyncSubresourceStreamingStarted,
    ) -> Result<()> {
        let detached_window_fetch = pending.continuation.is_detached_window_fetch();
        debug_assert!(
            detached_window_fetch,
            "only a detached Fetch keeps a VM-managed stream without a JS consumer"
        );

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
                    if started.skip_fetch_security_validation {
                        return None;
                    }
                    crate::network_host::validate_fetch_response_security_policy(
                        &pending.request_origin,
                        &started.head,
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
            pending.network().failed_with(
                &pending.response_stream().failure(error_text),
                subresource_turn_observer(&self._context_host),
            );
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
        self._context_host
            .borrow_mut()
            .record_streaming_subresource_fetch(StreamingSubresourceFetchState {
                response_filter: started.response_filter,
                pending,
                body_source_id: started.body_source_id,
                head: started.head,
                event_source_parser: None,
                xhr_response: None,
            });
        tracing::debug!(
            internal_id = started.internal_id,
            ?detached_identity,
            "continued network-only subresource streaming without a V8 body source"
        );
        Ok(())
    }

    /// Inject the physical response and its subsequent VM delivery.
    #[cfg(test)]
    pub(super) fn start_streaming_async_subresource_fetch(
        &mut self,
        started: crate::types::AsyncSubresourceStreamingStarted,
    ) -> Result<()> {
        if let Some(stream) = self
            ._context_host
            .borrow()
            .subresource_response_stream(started.internal_id)
        {
            stream.response_started(ResourceResponseHead {
                status_text: None,
                head: started.head.clone(),
                network_request_headers: None,
            });
        }
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
        let pending = {
            let mut host = self._context_host.borrow_mut();
            host.take_pending_subresource_fetch(started.internal_id)
                .or_else(|| {
                    let running = host.take_running_subresource_fetch(started.internal_id)?;
                    host.finish_active_subresource_request();
                    Some(running.pending)
                })
        };
        let Some(pending) = pending else {
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
        if pending.continuation.is_detached_window_fetch() {
            return self
                .start_detached_window_fetch_stream(pending, started)
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
                if started.skip_fetch_security_validation {
                    return None;
                }
                matches!(
                    pending.info.resource_type,
                    SubresourceResourceType::EventSource
                        | SubresourceResourceType::Fetch
                        | SubresourceResourceType::Xhr
                )
                .then(|| {
                    crate::network_host::validate_fetch_response_security_policy(
                        &pending.request_origin,
                        &started.head,
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
                pending.network().failed_with(
                    &pending.response_stream().failure(network_error_text),
                    subresource_turn_observer(&self._context_host),
                );
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
                    | PendingSubresourceContinuation::WebSocket(_) => {}
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
                pending.network().failed_with(
                    &pending.response_stream().failure(error_text),
                    subresource_turn_observer(&self._context_host),
                );
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
            ) && !started.response_filter.is_some_and(|filter| filter.is_readable()) {
                observable_head.headers = crate::network_host::filter_cors_exposed_response_headers(
                    &pending.request_origin,
                    &observable_head,
                    pending.credentials_mode,
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
                        response_filter: started.response_filter,
                        pending,
                        body_source_id: started.body_source_id,
                        head: started.head.clone(),
                        event_source_parser: None,
                        xhr_response: Some(xhr_response)});
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
                    let response_obj = crate::network_host::build_fetch_response_object_from_stream_for_request_mode_with_filter(
                        scope,
                        &pending.request_origin,
                        pending.request_mode,
                        observable_head,
                        started.body_source_id,
                        started.response_filter,
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
                | PendingSubresourceContinuation::WebSocket(_) => {}
            }

            let pending_owner = pending.execution_context.dispatch_scope();
            self._context_host.borrow_mut().record_streaming_subresource_fetch(
                StreamingSubresourceFetchState {
                        response_filter: started.response_filter,
                    pending,
                    body_source_id: started.body_source_id,
                    head: started.head.clone(),
                    event_source_parser,
                    xhr_response: None},
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
        if let Some(stream) = self
            ._context_host
            .borrow()
            .streaming_response_for_test(body_source_id)
        {
            stream.data_received(&bytes);
        }
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
            .note_streaming_subresource_activity(body_source_id);
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
                    // The producer retained these bytes for the native response.
                    // Move this delivery into the Web-visible stream.
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

    /// Standalone ScriptVm test turn for a streaming-finish terminal.
    #[cfg(test)]
    pub(super) fn finish_streaming_async_subresource_fetch(
        &mut self,
        internal_id: u64,
        body_source_id: NetworkBodySourceId,
        result: std::result::Result<(), String>,
    ) -> Result<()> {
        if let Some(stream) = self
            ._context_host
            .borrow()
            .subresource_response_stream(internal_id)
        {
            match &result {
                Ok(()) => {
                    if let Some(response) = stream.finish_response() {
                        response.publish(&stream.network, None);
                    }
                }
                Err(message) => stream.network.failed(&stream.failure(message.clone())),
            }
        }
        let body = self
            ._context_host
            .borrow_mut()
            .take_test_streaming_body(internal_id);
        let activity = self.finish_streaming_async_subresource_fetch_body(
            internal_id,
            body_source_id,
            result,
            body,
        )?;
        self.finish_async_subresource_body_checkpoint_for_test(activity)
    }

    fn finish_streaming_async_subresource_fetch_body(
        &mut self,
        internal_id: u64,
        body_source_id: NetworkBodySourceId,
        result: std::result::Result<(), String>,
        response_body: SubresourceResponseBody,
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
        if streaming.pending.continuation.is_detached_window_fetch() {
            streaming.pending.load.finish();
            return Ok(AsyncSubresourceFetchBodyActivity::NoWindowRealmEntered);
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
                        let finish_body_started =
                            moli_trace::cdp_runtime_trace_enabled().then(Instant::now);
                        let response_body = response_body.clone();
                        let response_body_size = response_body.len();
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
                            match response_body.materialize_bytes() {
                                Ok(bytes) => {
                                    trace_async_subresource_stage(
                                        "async_subresource_streaming_xhr_body_materialized",
                                        trace_fields,
                                        materialize_started,
                                    );
                                    Some(moli_fetch::ResponseBody::materialized_bytes(bytes))
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
                        if let PendingSubresourceContinuation::Xhr(xhr) =
                            streaming.pending.continuation
                            && let Some(response_body) = xhr_delivery_body
                        {
                            let xhr_started =
                                moli_trace::cdp_runtime_trace_enabled().then(Instant::now);
                            crate::context_bootstrap::record_resource_performance_entry(
                                scope,
                                crate::context_bootstrap::ResourcePerformanceEntry::from_streaming_network_response(
                                    streaming.pending.info.url.as_str(),
                                    "xmlhttprequest",
                                    None,
                                    &streaming.head,
                                    response_body_size,
                                ),
                            );
                            let xhr = v8::Local::new(scope, &xhr);
                            let mut observable_head = streaming.head;
                            if !streaming
                                .response_filter
                                .is_some_and(|filter| filter.is_readable())
                            {
                                observable_head.headers =
                                    crate::network_host::filter_cors_exposed_response_headers(
                                        &streaming.pending.request_origin,
                                        &observable_head,
                                        streaming.pending.credentials_mode,
                                    );
                            }
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
                body_activity,
            } => {
                self.apply_pending_child_document_owner_retirements();
                return Ok(
                    CurrentChildDocumentLoadApplication::SupersededDuringApplication {
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

fn publish_buffered_subresource_result(
    context_host: &Rc<RefCell<JsContextHost>>,
    resource: &crate::network::ResourceResponseStream,
    physical: Option<&crate::protocol_types::NavigationResponse>,
    result: &Result<crate::protocol_types::NavigationResponse, String>,
    network_error_text: Option<&str>,
) {
    let network = &resource.network;
    match result {
        Ok(response) => network.body_completed_with(
            ResourceResponseHead {
                head: response.head(),
                status_text: None,
                network_request_headers: response.network_request_headers().map(<[_]>::to_vec),
            },
            SubresourceResponseBody::from_navigation_response(response),
            subresource_turn_observer(context_host),
        ),
        Err(message) => {
            let message = network_error_text.unwrap_or(message).to_owned();
            let failure = match physical {
                Some(response) => ResourceBodyResponse::from(response.clone()).failure(
                    message,
                    response.network_request_headers().map(<[_]>::to_vec),
                ),
                None => resource.failure(message),
            };
            network.failed_with(&failure, subresource_turn_observer(context_host));
        }
    }
}

fn subresource_turn_observer(
    context_host: &Rc<RefCell<JsContextHost>>,
) -> impl FnMut(crate::runtime::RendererNetworkObservation) + '_ {
    |observation| {
        context_host
            .borrow_mut()
            .record_native_resource_observation(observation)
    }
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
        #[cfg(test)]
        AsyncSubresourceFetchEvent::Completion(completion) => AsyncSubresourceTraceFields {
            event_kind: Some("completion"),
            internal_id: Some(completion.internal_id),
            ..AsyncSubresourceTraceFields::default()
        },
        AsyncSubresourceFetchEvent::TransportCompletion(completion) => {
            AsyncSubresourceTraceFields {
                event_kind: Some("keepalive"),
                internal_id: Some(completion.internal_id()),
                ..AsyncSubresourceTraceFields::default()
            }
        }
        AsyncSubresourceFetchEvent::ResponsePaused { internal_id, .. } => {
            AsyncSubresourceTraceFields {
                event_kind: Some("response_paused"),
                internal_id: Some(*internal_id),
                ..Default::default()
            }
        }
        AsyncSubresourceFetchEvent::NativeNetwork(_) => AsyncSubresourceTraceFields {
            event_kind: Some("native_network"),
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
        AsyncSubresourceFetchEvent::TransportStreamingFinished {
            body_source_id,
            completion,
        } => AsyncSubresourceTraceFields {
            event_kind: Some("streaming_finished"),
            internal_id: Some(completion.internal_id()),
            body_source_id: Some(*body_source_id),
            ..Default::default()
        },
        #[cfg(test)]
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
