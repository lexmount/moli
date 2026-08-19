use crate::domains::activity::{
    ProtocolOutputPayloads, ProtocolOutputProjectionContext, ProtocolOutputSink, ProtocolOutputSlot,
};
use crate::{
    conn::{BackgroundProtocolEvent, CdpConnection},
    domains::command_output::protocol_message_background_event_for_target,
    domains::runtime_context_events::{
        RuntimeContextProtocolEvent, apply_runtime_context_protocol_event_side_effects_typed,
        emit_runtime_context_protocol_background_event_typed,
        qualify_runtime_context_protocol_event_for_session_owner_typed,
    },
};
use moli_core::page::{
    RendererRuntimeInspectorMessage, RendererRuntimeInspectorMessageBatch,
    RendererRuntimeInspectorMessageResponseOrder,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RuntimeOutputProjectionStep {
    RuntimeBindingCalls,
    RuntimeInspectorMessages,
    RuntimeInspectorPostResponseMessages,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct RuntimePreparedOutputs {
    binding_call_batches: Vec<RuntimeBindingCallBatch>,
    inspector_message_batches: Vec<RuntimeInspectorMessageBatch>,
    post_response_inspector_message_batches: Vec<RuntimeInspectorMessageBatch>,
}

/// A binding invocation is a historical protocol observation, but its route is
/// the exact Page attachment that existed when the calls were taken.
///
/// Realm retirement does not erase a call that already happened. Page
/// replacement or session detach does invalidate the route, so the attachment
/// remains on the captured value until projection instead of being flattened
/// into a `sessionId` string early.
#[derive(Clone, Debug, PartialEq)]
struct RuntimeBindingCallBatch {
    attachment: crate::conn::TargetPageProtocolAttachmentIdentity,
    calls: Vec<crate::conn::RuntimeBindingCallEvent>,
}

#[derive(Clone, Debug, PartialEq)]
struct RuntimeInspectorMessageBatch {
    /// Exact Page and protocol attachment that owned these messages when the
    /// renderer snapshot was captured.
    ///
    /// `RuntimeInspectorMessageBatch` may contain command responses as well as
    /// notifications. A projection-time session fallback would therefore be
    /// able to deliver an old response to a replacement Page. The attachment
    /// is captured before the output can cross an async or scheduler boundary
    /// and is revalidated immediately before delivery.
    attachment: crate::conn::TargetPageProtocolAttachmentIdentity,
    messages: Vec<RuntimeInspectorMessage>,
    /// Contexts created by this exact Inspector batch.
    ///
    /// BiDi preload-channel listeners are an owner action caused by the
    /// concrete `Runtime.executionContextCreated` fact. Keeping the ids beside
    /// that fact prevents child-frame activity from rescanning the live realm
    /// inventory and becoming a second lifecycle producer.
    created_execution_context_ids: Vec<i64>,
}

#[derive(Clone, Debug, PartialEq)]
enum RuntimeInspectorMessage {
    Context(RuntimeContextProtocolEvent),
    Background(BackgroundProtocolEvent),
}

impl RuntimeInspectorMessage {
    fn from_renderer_message(
        message: RendererRuntimeInspectorMessage,
        target_id: Option<&str>,
    ) -> Self {
        match message {
            RendererRuntimeInspectorMessage::RuntimeContext(event) => {
                Self::Context(RuntimeContextProtocolEvent::from_restore_event(event))
            }
            RendererRuntimeInspectorMessage::Protocol(message) => Self::Background(
                protocol_message_background_event_for_target(message.into_value(), target_id),
            ),
        }
    }

    fn created_execution_context_id(&self) -> Option<i64> {
        match self {
            Self::Context(RuntimeContextProtocolEvent::Created(event)) => event.context_id,
            Self::Context(RuntimeContextProtocolEvent::Destroyed(_))
            | Self::Context(RuntimeContextProtocolEvent::Cleared(_))
            | Self::Background(_) => None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct RuntimePreparedOutputSlot {
    outputs: RuntimePreparedOutputs,
}

pub(in crate::domains) const SLOT_RUNTIME_BINDING_CALLS: ProtocolOutputSlot =
    ProtocolOutputSlot::RuntimeBindingCalls;
pub(in crate::domains) const SLOT_RUNTIME_INSPECTOR_MESSAGES: ProtocolOutputSlot =
    ProtocolOutputSlot::RuntimeInspectorMessages;
pub(in crate::domains) const SLOT_RUNTIME_INSPECTOR_POST_RESPONSE_MESSAGES: ProtocolOutputSlot =
    ProtocolOutputSlot::RuntimeInspectorPostResponseMessages;

impl RuntimeOutputProjectionStep {
    async fn project_async(
        self,
        conn: &mut CdpConnection,
        context: &mut ProtocolOutputProjectionContext<'_>,
        prepared_outputs: Option<&mut ProtocolOutputPayloads>,
    ) {
        match self {
            RuntimeOutputProjectionStep::RuntimeBindingCalls => {
                if let Some(batches) = prepared_outputs
                    .and_then(ProtocolOutputPayloads::runtime_mut)
                    .and_then(RuntimePreparedOutputSlot::take_binding_call_batches)
                {
                    for batch in batches {
                        if !conn
                            .target_page_protocol_attachment_identity_is_current(&batch.attachment)
                        {
                            continue;
                        }
                        let session_id = batch.attachment.session_id().map(str::to_owned);
                        context
                            .command
                            .protocol_events_mut()
                            .extend(batch.calls.into_iter().map(|call| {
                                call.into_background_protocol_event(session_id.as_deref())
                            }));
                    }
                }
            }
            RuntimeOutputProjectionStep::RuntimeInspectorMessages
            | RuntimeOutputProjectionStep::RuntimeInspectorPostResponseMessages => {
                if let Some(batches) = prepared_outputs
                    .and_then(ProtocolOutputPayloads::runtime_mut)
                    .and_then(|outputs| match self {
                        RuntimeOutputProjectionStep::RuntimeInspectorMessages => {
                            outputs.take_inspector_messages()
                        }
                        RuntimeOutputProjectionStep::RuntimeInspectorPostResponseMessages => {
                            outputs.take_post_response_inspector_messages()
                        }
                        RuntimeOutputProjectionStep::RuntimeBindingCalls => unreachable!(),
                    })
                {
                    for batch in batches {
                        if !conn
                            .target_page_protocol_attachment_identity_is_current(&batch.attachment)
                        {
                            continue;
                        }
                        let session_id = batch.attachment.session_id().map(str::to_owned);
                        push_runtime_inspector_messages_for_session(
                            conn,
                            context.command.protocol_events_mut(),
                            batch.messages,
                            session_id.as_deref(),
                        );
                        for execution_context_id in batch.created_execution_context_ids {
                            Box::pin(
                                super::start_bidi_preload_channel_listeners_for_execution_context_background_events_async(
                                    conn,
                                    session_id.as_deref(),
                                    execution_context_id,
                                    context.command.protocol_events_mut(),
                                ),
                            )
                            .await;
                        }
                    }
                }
            }
        }
    }
}

pub(in crate::domains) async fn project_runtime_binding_calls_async(
    conn: &mut CdpConnection,
    context: &mut ProtocolOutputProjectionContext<'_>,
    prepared_outputs: Option<&mut ProtocolOutputPayloads>,
) {
    RuntimeOutputProjectionStep::RuntimeBindingCalls
        .project_async(conn, context, prepared_outputs)
        .await;
}

pub(in crate::domains) async fn project_runtime_inspector_messages_async(
    conn: &mut CdpConnection,
    context: &mut ProtocolOutputProjectionContext<'_>,
    prepared_outputs: Option<&mut ProtocolOutputPayloads>,
) {
    RuntimeOutputProjectionStep::RuntimeInspectorMessages
        .project_async(conn, context, prepared_outputs)
        .await;
}

pub(in crate::domains) async fn project_runtime_inspector_post_response_messages_async(
    conn: &mut CdpConnection,
    context: &mut ProtocolOutputProjectionContext<'_>,
    prepared_outputs: Option<&mut ProtocolOutputPayloads>,
) {
    RuntimeOutputProjectionStep::RuntimeInspectorPostResponseMessages
        .project_async(conn, context, prepared_outputs)
        .await;
}

impl RuntimePreparedOutputs {
    pub(crate) fn from_renderer_runtime_binding_call(
        conn: &CdpConnection,
        session_id: Option<&str>,
        call: moli_core::page::PendingRuntimeBindingCall,
    ) -> Self {
        let Some(attachment) =
            conn.target_page_protocol_attachment_identity_for_session(session_id)
        else {
            return Self::default();
        };
        Self {
            binding_call_batches: vec![RuntimeBindingCallBatch {
                attachment,
                calls: vec![crate::conn::RuntimeBindingCallEvent::from_renderer_call(
                    call,
                )],
            }],
            inspector_message_batches: Vec::new(),
            post_response_inspector_message_batches: Vec::new(),
        }
    }

    pub(crate) fn from_renderer_runtime_inspector_message_batches(
        conn: &mut CdpConnection,
        source_session_id: Option<&str>,
        batches: &[RendererRuntimeInspectorMessageBatch],
    ) -> Self {
        let mut outputs = Self::default();
        for batch in batches.iter().filter(|batch| !batch.messages.is_empty()) {
            let Some(attachment) = conn
                .target_page_protocol_attachment_identity_for_renderer_inspector_route(
                    source_session_id,
                    batch.session.wire_session_id(),
                )
            else {
                continue;
            };
            let mut renderer_messages = batch.messages.clone();
            if let Some(renderer_agent_attachment_id) = batch.renderer_agent_attachment_id() {
                conn.restore_frontend_command_ids_in_devtools_session_output(
                    attachment.session_id(),
                    renderer_agent_attachment_id,
                    &mut renderer_messages,
                );
            }
            if renderer_messages.is_empty() {
                continue;
            }
            let messages = renderer_messages
                .into_iter()
                .map(|message| {
                    RuntimeInspectorMessage::from_renderer_message(
                        message,
                        attachment.page_owner().target_id(),
                    )
                })
                .collect::<Vec<_>>();
            let mut created_execution_context_ids = Vec::new();
            for execution_context_id in messages
                .iter()
                .filter_map(RuntimeInspectorMessage::created_execution_context_id)
            {
                if !created_execution_context_ids.contains(&execution_context_id) {
                    created_execution_context_ids.push(execution_context_id);
                }
            }
            let prepared = RuntimeInspectorMessageBatch {
                attachment,
                messages,
                created_execution_context_ids,
            };
            match batch.command_response_order() {
                RendererRuntimeInspectorMessageResponseOrder::BeforeCommandResponse => {
                    outputs.inspector_message_batches.push(prepared);
                }
                RendererRuntimeInspectorMessageResponseOrder::AfterCommandResponse => outputs
                    .post_response_inspector_message_batches
                    .push(prepared),
            }
        }
        outputs
    }

    pub(crate) fn extend(&mut self, other: Self) {
        self.binding_call_batches.extend(other.binding_call_batches);
        self.inspector_message_batches
            .extend(other.inspector_message_batches);
        self.post_response_inspector_message_batches
            .extend(other.post_response_inspector_message_batches);
    }

    pub(in crate::domains) fn is_empty(&self) -> bool {
        self.binding_call_batches.is_empty()
            && self.inspector_message_batches.is_empty()
            && self.post_response_inspector_message_batches.is_empty()
    }

    pub(in crate::domains) fn append_to_output_sink(
        self,
        sink: &mut (impl ProtocolOutputSink + ?Sized),
    ) {
        if !self.binding_call_batches.is_empty() {
            sink.push_produced_slot(SLOT_RUNTIME_BINDING_CALLS);
        }
        if !self.inspector_message_batches.is_empty() {
            sink.push_produced_slot(SLOT_RUNTIME_INSPECTOR_MESSAGES);
        }
        if !self.post_response_inspector_message_batches.is_empty() {
            sink.push_produced_slot(SLOT_RUNTIME_INSPECTOR_POST_RESPONSE_MESSAGES);
        }
        if !self.is_empty() {
            sink.push_prepared_payload(RuntimePreparedOutputSlot::from_outputs(self).into());
        }
    }

    #[cfg(test)]
    pub(crate) fn from_runtime_binding_calls_for_test(
        attachment: crate::conn::TargetPageProtocolAttachmentIdentity,
        calls: Vec<crate::conn::RuntimeBindingCallEvent>,
    ) -> Self {
        Self {
            binding_call_batches: vec![RuntimeBindingCallBatch { attachment, calls }],
            inspector_message_batches: Vec::new(),
            post_response_inspector_message_batches: Vec::new(),
        }
    }
}

impl RuntimePreparedOutputSlot {
    pub(crate) fn from_outputs(outputs: RuntimePreparedOutputs) -> Self {
        Self { outputs }
    }

    pub(crate) fn extend(&mut self, other: Self) {
        self.outputs.extend(other.outputs);
    }

    fn take_binding_call_batches(&mut self) -> Option<Vec<RuntimeBindingCallBatch>> {
        (!self.outputs.binding_call_batches.is_empty())
            .then(|| std::mem::take(&mut self.outputs.binding_call_batches))
    }

    fn take_inspector_messages(&mut self) -> Option<Vec<RuntimeInspectorMessageBatch>> {
        (!self.outputs.inspector_message_batches.is_empty())
            .then(|| std::mem::take(&mut self.outputs.inspector_message_batches))
    }

    fn take_post_response_inspector_messages(
        &mut self,
    ) -> Option<Vec<RuntimeInspectorMessageBatch>> {
        (!self
            .outputs
            .post_response_inspector_message_batches
            .is_empty())
        .then(|| std::mem::take(&mut self.outputs.post_response_inspector_message_batches))
    }
}

fn push_runtime_inspector_messages_for_session(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    messages: Vec<RuntimeInspectorMessage>,
    session_id: Option<&str>,
) {
    for message in messages {
        match message {
            RuntimeInspectorMessage::Context(mut event) => {
                qualify_runtime_context_protocol_event_for_session_owner_typed(
                    conn, &mut event, session_id,
                );
                apply_runtime_context_protocol_event_side_effects_typed(conn, &event, session_id);
                emit_runtime_context_protocol_background_event_typed(out, event, session_id);
            }
            RuntimeInspectorMessage::Background(mut event) => {
                event.ensure_protocol_session_id(session_id);
                out.push(event);
            }
        }
    }
}

pub(in crate::domains) fn push_routed_renderer_runtime_inspector_message_batch_background_events(
    conn: &mut CdpConnection,
    out: &mut Vec<BackgroundProtocolEvent>,
    batches: Vec<RendererRuntimeInspectorMessageBatch>,
    session_id: Option<&str>,
) {
    let prepared = RuntimePreparedOutputs::from_renderer_runtime_inspector_message_batches(
        conn, session_id, &batches,
    );
    for batch in prepared
        .inspector_message_batches
        .into_iter()
        .chain(prepared.post_response_inspector_message_batches)
    {
        if !conn.target_page_protocol_attachment_identity_is_current(&batch.attachment) {
            continue;
        }
        let effective_session_id = batch.attachment.session_id().map(str::to_owned);
        push_runtime_inspector_messages_for_session(
            conn,
            out,
            batch.messages,
            effective_session_id.as_deref(),
        );
    }
}

#[cfg(test)]
mod tests {
    use moli_core::page::{
        DevToolsSessionKey, RendererDevToolsAgentToken, RendererRuntimeInspectorMessage,
        RendererRuntimeInspectorMessageBatch,
    };
    use serde_json::Value;
    use serde_json::json;

    use crate::conn::{BrowserContext, CdpConnection, CommandDispatchContext};
    use crate::devtools_runtime::AutomationEvent;
    use crate::domains::activity::{ProtocolOutputPayloads, ProtocolOutputProjectionContext};
    use crate::testing::TestContext;

    use super::{RuntimePreparedOutputSlot, RuntimePreparedOutputs};

    fn renderer_messages(messages: Vec<Value>) -> Vec<RendererRuntimeInspectorMessage> {
        messages
            .into_iter()
            .map(RendererRuntimeInspectorMessage::from_v8_inspector_message)
            .collect()
    }

    #[test]
    fn runtime_binding_event_preserves_renderer_realm_identity() {
        let source = moli_core::page::RuntimeBindingCallSourceIdentity::new(17, 29);
        let event = crate::conn::RuntimeBindingCallEvent::from_renderer_call(
            moli_core::page::PendingRuntimeBindingCall {
                source,
                name: "realmBound".to_owned(),
                payload: "payload".to_owned(),
                execution_context_id: 77,
            },
        );

        assert_eq!(
            event.source(),
            source,
            "protocol preparation must retain the renderer realm generation instead of reconstructing it from current context state"
        );
    }

    async fn load_document(ctx: &mut TestContext, html: &str) {
        let mut bc = BrowserContext::new("BID-1".into());
        bc.set_target_url("data:text/html,runtime-backlog-test".to_owned());
        bc.set_active_target_id("TID-1".to_owned());
        bc.attach_active_session("SID-1".to_owned());
        let page = ctx
            .conn
            .load_page_via_runtime_async(&format!("data:text/html,{html}"))
            .await
            .expect("test page should load");
        let _ = bc
            .active_target
            .runtime_slot
            .replace_loaded_page(Some(page));
        ctx.conn.browser_context = Some(bc);
    }

    fn runtime_binding_attachment_fixture() -> (
        CdpConnection,
        crate::conn::TargetPageProtocolAttachmentIdentity,
    ) {
        let mut conn = CdpConnection::default();
        let mut browser_context = BrowserContext::new("BID-1".to_owned());
        browser_context.set_active_target_id("TID-1");
        browser_context.attach_active_session("SID-1");
        browser_context
            .active_target
            .runtime_slot
            .set_page_attachment_id_for_test(1);
        conn.browser_context = Some(browser_context);
        let attachment = conn
            .target_page_protocol_attachment_identity_for_session(Some("SID-1"))
            .expect("exact Runtime binding attachment");
        (conn, attachment)
    }

    fn runtime_binding_outputs(
        attachment: crate::conn::TargetPageProtocolAttachmentIdentity,
        payload: &str,
    ) -> RuntimePreparedOutputs {
        RuntimePreparedOutputs::from_runtime_binding_calls_for_test(
            attachment,
            vec![crate::conn::RuntimeBindingCallEvent::new_for_test(
                17,
                29,
                "preparedBinding",
                payload,
                77,
            )],
        )
    }

    fn prepared_runtime_binding_calls(
        attachment: crate::conn::TargetPageProtocolAttachmentIdentity,
    ) -> ProtocolOutputPayloads {
        ProtocolOutputPayloads::from_slot(RuntimePreparedOutputSlot::from_outputs(
            runtime_binding_outputs(attachment, "prepared-payload"),
        ))
    }

    fn renderer_inspector_outputs(
        conn: &mut CdpConnection,
        source_session_id: Option<&str>,
        renderer_inspector_session_id: Option<&str>,
        messages: Vec<Value>,
    ) -> RuntimePreparedOutputs {
        let session = renderer_inspector_session_id
            .map_or(DevToolsSessionKey::Primary, |session_id| {
                DevToolsSessionKey::Attached(session_id.to_owned())
            });
        let batches = vec![RendererRuntimeInspectorMessageBatch::new(
            RendererDevToolsAgentToken::allocate(),
            session,
            renderer_messages(messages),
        )];
        RuntimePreparedOutputs::from_renderer_runtime_inspector_message_batches(
            conn,
            source_session_id,
            &batches,
        )
    }

    #[test]
    fn renderer_inspector_batches_keep_their_command_response_side() {
        let (mut conn, _) = runtime_binding_attachment_fixture();
        let agent_token = RendererDevToolsAgentToken::allocate();
        let session = DevToolsSessionKey::Primary;
        let batches = vec![
            RendererRuntimeInspectorMessageBatch::new(
                agent_token,
                session.clone(),
                renderer_messages(vec![json!({
                    "method": "Debugger.scriptParsed",
                    "params": {"scriptId": "1"},
                })]),
            ),
            RendererRuntimeInspectorMessageBatch::new_after_command_response(
                agent_token,
                session,
                renderer_messages(vec![
                    json!({"method": "Debugger.resumed", "params": {}}),
                    json!({"method": "Debugger.paused", "params": {"callFrames": []}}),
                ]),
            ),
        ];

        let outputs = RuntimePreparedOutputs::from_renderer_runtime_inspector_message_batches(
            &mut conn,
            Some("SID-1"),
            &batches,
        );

        assert_eq!(outputs.inspector_message_batches.len(), 1);
        assert_eq!(outputs.post_response_inspector_message_batches.len(), 1);
        assert_eq!(
            outputs.post_response_inspector_message_batches[0]
                .messages
                .len(),
            2
        );
    }

    async fn drain_runtime_inspector_outputs(
        conn: &mut CdpConnection,
        outputs: RuntimePreparedOutputs,
        drain_session_id: Option<&str>,
    ) -> Vec<crate::conn::BackgroundProtocolEvent> {
        let mut prepared =
            ProtocolOutputPayloads::from_slot(RuntimePreparedOutputSlot::from_outputs(outputs));
        let mut command_context = CommandDispatchContext::default();
        {
            let mut context = ProtocolOutputProjectionContext {
                session_id: drain_session_id,
                command: &mut command_context,
                subresource_frame_id: None,
                subresource_document_url: None,
                subresource_timestamp: None,
                subresource_network_request_id: None,
            };
            super::project_runtime_inspector_messages_async(
                conn,
                &mut context,
                Some(&mut prepared),
            )
            .await;
        }
        command_context.take_protocol_events()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn runtime_backlog_batch_ignores_prequeued_inspector_registry() {
        let mut ctx = TestContext::new();
        load_document(
            &mut ctx,
            "<!doctype html><script>console.warn('runtime backlog')</script>",
        )
        .await;

        let bc = ctx
            .conn
            .browser_context
            .as_mut()
            .expect("browser context should be loaded");
        bc.devtools_session_state
            .console_output_session_state
            .console_enabled = true;
        bc.devtools_session_state.page_session_state.log_enabled = true;
        bc.devtools_session_state
            .runtime_session_state
            .runtime_frontend_enabled = true;
        ctx.conn
            .register_pending_inspector_await(900_199, Some("SID-1"));

        assert!(
            ctx.conn
                .has_pending_inspector_awaits_for_session_owner(Some("SID-1")),
            "the pending registry remains diagnostic state, not a prequeued activity output"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn runtime_binding_drain_consumes_prepared_events_without_page_readback() {
        let (mut conn, attachment) = runtime_binding_attachment_fixture();
        let mut command_context = CommandDispatchContext::default();
        let mut context = ProtocolOutputProjectionContext {
            session_id: Some("SID-drain-current"),
            command: &mut command_context,
            subresource_frame_id: None,
            subresource_document_url: None,
            subresource_timestamp: None,
            subresource_network_request_id: None,
        };
        let mut prepared = prepared_runtime_binding_calls(attachment);

        super::project_runtime_binding_calls_async(&mut conn, &mut context, Some(&mut prepared))
            .await;
        let out = context
            .command
            .take_protocol_events()
            .into_iter()
            .map(crate::conn::BackgroundProtocolEvent::into_protocol_message)
            .collect::<Vec<_>>();

        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["method"], json!("Runtime.bindingCalled"));
        assert_eq!(out[0]["params"]["payload"], json!("prepared-payload"));
        assert_eq!(
            out[0]["sessionId"],
            json!("SID-1"),
            "captured binding output must retain its capture-time attachment instead of following the contextual projection session"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn runtime_binding_drain_discards_a_replaced_page_attachment() {
        let (mut conn, attachment) = runtime_binding_attachment_fixture();
        let mut prepared = prepared_runtime_binding_calls(attachment);
        conn.runtime_session_owner_slot_mut(Some("SID-1"))
            .expect("Runtime binding target")
            .replace_page_attachment_id_for_test();
        let mut command_context = CommandDispatchContext::default();
        let mut context = ProtocolOutputProjectionContext {
            session_id: Some("SID-1"),
            command: &mut command_context,
            subresource_frame_id: None,
            subresource_document_url: None,
            subresource_timestamp: None,
            subresource_network_request_id: None,
        };

        super::project_runtime_binding_calls_async(&mut conn, &mut context, Some(&mut prepared))
            .await;

        assert!(
            context.command.take_protocol_events().is_empty(),
            "a binding call captured from the old Page must not leak through a replacement attachment"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn runtime_binding_drain_authorizes_each_prepared_attachment_independently() {
        let (mut conn, retired_attachment) = runtime_binding_attachment_fixture();
        let mut outputs = runtime_binding_outputs(retired_attachment, "retired-page");
        conn.runtime_session_owner_slot_mut(Some("SID-1"))
            .expect("Runtime binding target")
            .replace_page_attachment_id_for_test();
        let current_attachment = conn
            .target_page_protocol_attachment_identity_for_session(Some("SID-1"))
            .expect("replacement Runtime binding attachment");
        outputs.extend(runtime_binding_outputs(
            current_attachment,
            "replacement-page",
        ));
        let mut prepared =
            ProtocolOutputPayloads::from_slot(RuntimePreparedOutputSlot::from_outputs(outputs));
        let mut command_context = CommandDispatchContext::default();
        let mut context = ProtocolOutputProjectionContext {
            session_id: Some("SID-unrelated-drain"),
            command: &mut command_context,
            subresource_frame_id: None,
            subresource_document_url: None,
            subresource_timestamp: None,
            subresource_network_request_id: None,
        };

        super::project_runtime_binding_calls_async(&mut conn, &mut context, Some(&mut prepared))
            .await;
        let events = context
            .command
            .take_protocol_events()
            .into_iter()
            .map(crate::conn::BackgroundProtocolEvent::into_protocol_message)
            .collect::<Vec<_>>();

        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["params"]["payload"], json!("replacement-page"));
        assert_eq!(events[0]["sessionId"], json!("SID-1"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn runtime_binding_drain_discards_a_detached_protocol_session() {
        let (mut conn, attachment) = runtime_binding_attachment_fixture();
        let mut prepared = prepared_runtime_binding_calls(attachment);
        assert_eq!(
            conn.browser_context
                .as_mut()
                .expect("browser context")
                .detach_active_session()
                .as_deref(),
            Some("SID-1"),
        );
        let mut command_context = CommandDispatchContext::default();
        let mut context = ProtocolOutputProjectionContext {
            session_id: None,
            command: &mut command_context,
            subresource_frame_id: None,
            subresource_document_url: None,
            subresource_timestamp: None,
            subresource_network_request_id: None,
        };

        super::project_runtime_binding_calls_async(&mut conn, &mut context, Some(&mut prepared))
            .await;

        assert!(
            context.command.take_protocol_events().is_empty(),
            "detached Runtime sessions must not leave a prepared binding event route"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn runtime_inspector_activity_uses_capture_time_attachment() {
        let (mut conn, _attachment) = runtime_binding_attachment_fixture();
        let outputs = renderer_inspector_outputs(
            &mut conn,
            Some("SID-1"),
            None,
            vec![json!({
                "method": "Runtime.consoleAPICalled",
                "params": {
                    "type": "log",
                    "args": [{ "type": "string", "value": "runtime sidecar" }],
                    "executionContextId": 7
                }
            })],
        );
        let events =
            drain_runtime_inspector_outputs(&mut conn, outputs, Some("SID-unrelated-drain")).await;

        assert_eq!(events.len(), 1);
        let (message, automation_event) = events.into_iter().next().unwrap().into_parts();
        assert_eq!(message["method"], json!("Runtime.consoleAPICalled"));
        assert_eq!(message["sessionId"], json!("SID-1"));
        assert_eq!(message["params"]["executionContextId"], json!(7));
        let Some(AutomationEvent::RuntimeConsoleApiCalled(event)) = automation_event else {
            panic!("expected RuntimeConsoleApiCalled sidecar");
        };
        assert_eq!(event.text, "runtime sidecar");
        assert_eq!(
            event.target_id.as_ref().map(|target_id| target_id.as_str()),
            Some("TID-1"),
            "the automation sidecar must retain the capture-time Page target"
        );
        assert_eq!(event.execution_context_id, Some(7));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn runtime_inspector_activity_discards_replaced_page_output() {
        let (mut conn, _attachment) = runtime_binding_attachment_fixture();
        let outputs = renderer_inspector_outputs(
            &mut conn,
            Some("SID-1"),
            None,
            vec![json!({
                "id": 91,
                "result": { "value": "old-page-response" }
            })],
        );
        conn.runtime_session_owner_slot_mut(Some("SID-1"))
            .expect("Runtime inspector target")
            .replace_page_attachment_id_for_test();
        let events = drain_runtime_inspector_outputs(&mut conn, outputs, Some("SID-1")).await;

        assert!(
            events.is_empty(),
            "an inspector response captured from an old Page attachment must not reach its replacement"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn runtime_inspector_activity_discards_detached_attachment() {
        let (mut conn, _attachment) = runtime_binding_attachment_fixture();
        let outputs = renderer_inspector_outputs(
            &mut conn,
            Some("SID-1"),
            None,
            vec![json!({
                "method": "Runtime.consoleAPICalled",
                "params": {}
            })],
        );
        assert_eq!(
            conn.browser_context
                .as_mut()
                .expect("browser context")
                .detach_active_session()
                .as_deref(),
            Some("SID-1"),
        );
        let events = drain_runtime_inspector_outputs(&mut conn, outputs, None).await;

        assert!(
            events.is_empty(),
            "detaching the capture-time session must retire its prepared inspector output route"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn runtime_inspector_activity_preserves_auxiliary_renderer_route() {
        let (mut conn, _attachment) = runtime_binding_attachment_fixture();
        assert!(
            conn.browser_context
                .as_mut()
                .expect("browser context")
                .assign_auxiliary_session_to_target("TID-1", "SID-aux".to_owned())
        );
        let outputs = renderer_inspector_outputs(
            &mut conn,
            Some("SID-1"),
            Some("SID-aux"),
            vec![json!({
                "id": 92,
                "result": { "value": "auxiliary-response" }
            })],
        );
        let events = drain_runtime_inspector_outputs(&mut conn, outputs, Some("SID-1"))
            .await
            .into_iter()
            .map(crate::conn::BackgroundProtocolEvent::into_protocol_message)
            .collect::<Vec<_>>();

        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["id"], json!(92));
        assert_eq!(
            events[0]["sessionId"],
            json!("SID-aux"),
            "an auxiliary renderer inspector batch must retain its own attachment instead of following the source or drain session"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn runtime_inspector_activity_authorizes_each_page_batch_independently() {
        let (mut conn, _attachment) = runtime_binding_attachment_fixture();
        let mut outputs = renderer_inspector_outputs(
            &mut conn,
            Some("SID-1"),
            None,
            vec![json!({
                "id": 93,
                "result": { "value": "old-page" }
            })],
        );
        conn.runtime_session_owner_slot_mut(Some("SID-1"))
            .expect("Runtime inspector target")
            .replace_page_attachment_id_for_test();
        outputs.extend(renderer_inspector_outputs(
            &mut conn,
            Some("SID-1"),
            None,
            vec![json!({
                "id": 94,
                "result": { "value": "replacement-page" }
            })],
        ));
        let events =
            drain_runtime_inspector_outputs(&mut conn, outputs, Some("SID-unrelated-drain"))
                .await
                .into_iter()
                .map(crate::conn::BackgroundProtocolEvent::into_protocol_message)
                .collect::<Vec<_>>();

        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["id"], json!(94));
        assert_eq!(
            events[0]["result"]["value"],
            json!("replacement-page"),
            "one captured slot must validate each capture-time Page attachment instead of authorizing the whole slot from projection context"
        );
        assert_eq!(events[0]["sessionId"], json!("SID-1"));
    }
}
