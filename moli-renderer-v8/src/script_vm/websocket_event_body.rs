//! Exact-target Page WebSocket event body execution.
//!
//! Network ingress and the Page scheduler own the event payload and its
//! bounded/backpressured residence. This component owns only current-realm
//! validation, protocol-visible state updates, and event/Promise settlement.
//! It deliberately does not run a microtask checkpoint, synchronize child
//! browsing contexts, or advance runtime scripts: the unique selected Page
//! dispatcher submits that task-end boundary after this typed effect returns.

use anyhow::Result;

use super::ScriptVm;
use crate::page_task_queue::PageWebSocketBodyEffect;

impl ScriptVm {
    /// Apply one Page WebSocket event body without task-end completion.
    pub(crate) fn apply_current_page_websocket_event_body(
        &mut self,
        event: &moli_websocket::Event,
    ) -> Result<PageWebSocketBodyEffect> {
        let context_host = self._context_host.clone();
        self.with_default_context_scope(|scope, _host_ptr| {
            let socket_id = event.socket_id();
            let target = {
                let mut host = context_host.borrow_mut();
                host.websocket_dispatch_target(scope, socket_id)
            };
            let Some((dispatch_scope, realm_token, context, socket)) = target else {
                return Ok(PageWebSocketBodyEffect::CurrentTargetDisappeared);
            };
            let scope = &mut v8::ContextScope::new(scope, context);
            if crate::native_bridge::current_runtime_observable_context_token(scope)
                != Some(realm_token)
            {
                context_host.borrow_mut().retire_websocket(socket_id);
                tracing::debug!(
                    socket_id,
                    ?realm_token,
                    actual_realm_token = ?crate::native_bridge::current_runtime_observable_context_token(scope),
                    "dropped WebSocket event for stale realm"
                );
                return Ok(PageWebSocketBodyEffect::CurrentTargetDisappeared);
            }

            let previous_owner_context = dispatch_scope.enter(scope);
            match event {
                moli_websocket::Event::SendCompleted {
                    opcode,
                    payload_length,
                    ..
                } => {
                    let opcode = match opcode {
                        moli_websocket::FrameOpcode::Text => {
                            crate::types::WebSocketFrameOpcode::Text
                        }
                        moli_websocket::FrameOpcode::Binary => {
                            crate::types::WebSocketFrameOpcode::Binary
                        }
                    };
                    context_host.borrow_mut().record_websocket_frame(
                        socket_id,
                        crate::types::WebSocketFrameDirection::Sent,
                        opcode,
                        *payload_length,
                    );
                }
                moli_websocket::Event::Open {
                    request_headers,
                    response_status,
                    response_headers,
                    ..
                } => {
                    context_host.borrow_mut().record_websocket_open(
                        socket_id,
                        request_headers.clone(),
                        *response_status,
                        response_headers.clone(),
                    );
                }
                moli_websocket::Event::HandshakeResponse {
                    request_headers,
                    response_status,
                    response_headers,
                    ..
                } => {
                    context_host
                        .borrow_mut()
                        .pause_websocket_handshake_response(
                            socket_id,
                            request_headers.clone(),
                            *response_status,
                            response_headers.clone(),
                        );
                    dispatch_scope.restore(scope, previous_owner_context);
                    return Ok(PageWebSocketBodyEffect::InternalStateApplied);
                }
                moli_websocket::Event::Error { message, .. } => {
                    context_host
                        .borrow_mut()
                        .record_websocket_failure(socket_id, message.clone());
                }
                moli_websocket::Event::Closing { .. } => {
                    context_host
                        .borrow_mut()
                        .record_websocket_closing(socket_id);
                }
                moli_websocket::Event::Close {
                    code,
                    reason,
                    was_clean,
                    ..
                } => {
                    context_host.borrow_mut().record_websocket_close(
                        socket_id,
                        *code,
                        reason.clone(),
                        *was_clean,
                    );
                }
                _ => {}
            }

            let dispatch_result =
                crate::context_bootstrap::dispatch_websocket_event(scope, socket, event);
            if matches!(
                dispatch_result,
                crate::context_bootstrap::WebSocketDispatchResult::Backpressured
            ) {
                dispatch_scope.restore(scope, previous_owner_context);
                return Ok(PageWebSocketBodyEffect::ReadableBackpressured);
            }

            match event {
                moli_websocket::Event::TextMessage { data, .. } => {
                    context_host.borrow_mut().record_websocket_frame(
                        socket_id,
                        crate::types::WebSocketFrameDirection::Received,
                        crate::types::WebSocketFrameOpcode::Text,
                        data.len(),
                    );
                }
                moli_websocket::Event::BinaryMessage { data, .. } => {
                    context_host.borrow_mut().record_websocket_frame(
                        socket_id,
                        crate::types::WebSocketFrameDirection::Received,
                        crate::types::WebSocketFrameOpcode::Binary,
                        data.len(),
                    );
                }
                _ => {}
            }
            if matches!(event, moli_websocket::Event::Close { .. }) {
                context_host.borrow_mut().forget_websocket(socket_id);
            }
            dispatch_scope.restore(scope, previous_owner_context);

            Ok(match dispatch_result {
                crate::context_bootstrap::WebSocketDispatchResult::Dispatched => {
                    PageWebSocketBodyEffect::CallbackVisibleWorkApplied
                }
                crate::context_bootstrap::WebSocketDispatchResult::Noop => {
                    PageWebSocketBodyEffect::InternalStateApplied
                }
                crate::context_bootstrap::WebSocketDispatchResult::Backpressured => {
                    unreachable!("backpressured WebSocket payload returned before settlement")
                }
            })
        })
    }
}
