use std::collections::HashSet;
use std::fmt;

use moli_core::browser::BrowserSequence;
use moli_core::page::{
    PendingDevToolsIoCommandDispatch, PendingPageCommand, PendingRuntimeInspectorCommandDispatch,
    RendererAgentAttachmentId, RendererDevToolsAgentToken, RendererInspectorCommandRoute,
    RendererRuntimeInspectorMessageBatch,
};
use moli_renderer_v8::{
    RendererInspectionEndpoint, RendererInspectorCommandEnvelope, RendererInspectorIngressTicket,
    RendererRuntimeInspectorMainCommandRoute, RendererRuntimeInspectorResponseSender,
};

use super::{DocumentId, NavigationId};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererAgentAttachment {
    id: RendererAgentAttachmentId,
    agent_token: RendererDevToolsAgentToken,
    document: DocumentId,
    browser_sequence: BrowserSequence,
}

impl RendererAgentAttachment {
    fn new(
        document: DocumentId,
        browser_sequence: BrowserSequence,
        agent_token: RendererDevToolsAgentToken,
    ) -> Self {
        Self {
            id: RendererAgentAttachmentId::allocate(),
            agent_token,
            document,
            browser_sequence,
        }
    }

    pub(crate) fn id(self) -> RendererAgentAttachmentId {
        self.id
    }

    pub(crate) fn agent_token(self) -> RendererDevToolsAgentToken {
        self.agent_token
    }

    pub(crate) fn document(self) -> DocumentId {
        self.document
    }

    pub(crate) fn browser_sequence(self) -> BrowserSequence {
        self.browser_sequence
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DocumentProjectionFenceKey {
    document: DocumentId,
    browser_sequence: BrowserSequence,
    renderer_attachment: RendererAgentAttachmentId,
}

/// Move-only authority to expose output from one rebound renderer Document.
#[derive(Debug, Eq, PartialEq)]
#[must_use = "a Document projection fence must be published before its channel is retired"]
pub(crate) struct DocumentProjectionFence {
    key: DocumentProjectionFenceKey,
}

impl DocumentProjectionFence {
    fn new(attachment: RendererAgentAttachment) -> Self {
        Self {
            key: DocumentProjectionFenceKey {
                document: attachment.document(),
                browser_sequence: attachment.browser_sequence(),
                renderer_attachment: attachment.id(),
            },
        }
    }

    pub(crate) fn document(&self) -> DocumentId {
        self.key.document
    }

    pub(crate) fn browser_sequence(&self) -> BrowserSequence {
        self.key.browser_sequence
    }

    pub(crate) fn renderer_attachment(&self) -> RendererAgentAttachmentId {
        self.key.renderer_attachment
    }
}

/// The DevTools binding owns inspection ingress, never the Browser Page.
pub(crate) struct RendererAgentBinding {
    attachment: RendererAgentAttachment,
    endpoint: RendererInspectionEndpoint,
}

impl fmt::Debug for RendererAgentBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RendererAgentBinding")
            .field("attachment", &self.attachment)
            .finish_non_exhaustive()
    }
}

impl RendererAgentBinding {
    pub(crate) async fn detach_session(
        &self,
        inspector_session_id: Option<String>,
        fetch_subresource_interception: Option<(
            bool,
            Option<moli_core::page::SubresourceResourceType>,
        )>,
    ) -> anyhow::Result<()> {
        self.endpoint
            .detach_session(inspector_session_id, fetch_subresource_interception)
            .await
    }

    pub(crate) fn runtime_inspection(
        &self,
        inspector_session_id: Option<String>,
    ) -> moli_renderer_v8::RendererRuntimeInspection<'_> {
        self.endpoint
            .runtime_inspection(self.attachment.id(), inspector_session_id)
    }

    pub(crate) fn page_inspection(
        &self,
        inspector_session_id: Option<String>,
    ) -> moli_renderer_v8::RendererPageInspection<'_> {
        self.endpoint
            .page_inspection(self.attachment.id(), inspector_session_id)
    }

    pub(crate) fn dom_debugger_inspection(
        &self,
        inspector_session_id: Option<String>,
    ) -> moli_renderer_v8::RendererDomDebuggerInspection<'_> {
        self.endpoint
            .dom_debugger_inspection(self.attachment.id(), inspector_session_id)
    }

    pub(crate) fn css_inspection(
        &self,
        inspector_session_id: Option<String>,
    ) -> moli_renderer_v8::RendererCssInspection<'_> {
        self.endpoint
            .css_inspection(self.attachment.id(), inspector_session_id)
    }

    pub(crate) fn accessibility_inspection(
        &self,
        inspector_session_id: Option<String>,
    ) -> moli_renderer_v8::RendererAccessibilityInspection<'_> {
        self.endpoint
            .accessibility_inspection(self.attachment.id(), inspector_session_id)
    }

    pub(crate) fn dom_inspection(
        &self,
        inspector_session_id: Option<String>,
    ) -> moli_renderer_v8::RendererDomInspection<'_> {
        self.endpoint
            .dom_inspection(self.attachment.id(), inspector_session_id)
    }

    pub(crate) fn routes_output_stream(
        &self,
        stream: moli_core::RendererOutputStreamIdentity,
    ) -> bool {
        self.endpoint.routes_output_stream(stream)
    }

    pub(crate) fn start_performance_get_metrics(
        &self,
        inspector_session_id: Option<String>,
        result: serde_json::Value,
        response: Option<RendererRuntimeInspectorResponseSender>,
    ) -> anyhow::Result<PendingDevToolsIoCommandDispatch> {
        self.endpoint
            .enqueue_performance_get_metrics(
                RendererInspectorIngressTicket::new(
                    Some(self.attachment.id()),
                    inspector_session_id,
                    RendererInspectorCommandRoute::Io,
                ),
                result,
                response,
            )
            .map(PendingDevToolsIoCommandDispatch::from_route)
    }

    pub(crate) fn start_set_script_execution_disabled(
        &self,
        inspector_session_id: Option<String>,
        disabled: bool,
        response: Option<RendererRuntimeInspectorResponseSender>,
    ) -> anyhow::Result<PendingDevToolsIoCommandDispatch> {
        self.endpoint
            .enqueue_set_script_execution_disabled(
                RendererInspectorIngressTicket::new(
                    Some(self.attachment.id()),
                    inspector_session_id,
                    RendererInspectorCommandRoute::Io,
                ),
                disabled,
                response,
            )
            .map(PendingDevToolsIoCommandDispatch::from_route)
    }

    pub(crate) fn attachment(&self) -> RendererAgentAttachment {
        self.attachment
    }

    pub(crate) fn start_runtime_enable_events(
        &self,
        inspector_session_id: Option<String>,
    ) -> anyhow::Result<PendingPageCommand> {
        self.endpoint
            .enqueue_main_command(
                RendererInspectorCommandEnvelope::new_main_runtime_enable_events(
                    RendererInspectorIngressTicket::new(
                        Some(self.attachment.id()),
                        inspector_session_id,
                        RendererInspectorCommandRoute::MainThread,
                    ),
                ),
            )
            .map(PendingPageCommand::from_inspector_main_route)
    }

    pub(crate) fn start_main_protocol_on_page_owner(
        &self,
        inspector_session_id: Option<String>,
        context_resolution_action: Option<String>,
        raw_json: String,
        response: Option<RendererRuntimeInspectorResponseSender>,
    ) -> anyhow::Result<RendererRuntimeInspectorMainCommandRoute> {
        self.endpoint.enqueue_main_command(
            RendererInspectorCommandEnvelope::new_main_protocol_on_page_owner(
                RendererInspectorIngressTicket::new(
                    Some(self.attachment.id()),
                    inspector_session_id,
                    RendererInspectorCommandRoute::MainThread,
                ),
                context_resolution_action,
                raw_json,
                response,
            ),
        )
    }

    pub(crate) fn start_protocol_message(
        &self,
        inspector_session_id: Option<String>,
        lane: RendererInspectorCommandRoute,
        context_resolution_action: Option<String>,
        raw_json: String,
        response: RendererRuntimeInspectorResponseSender,
    ) -> anyhow::Result<PendingRuntimeInspectorCommandDispatch> {
        match lane {
            RendererInspectorCommandRoute::MainThread => self
                .endpoint
                .enqueue_main_command(RendererInspectorCommandEnvelope::new_main_protocol(
                    RendererInspectorIngressTicket::new(
                        Some(self.attachment.id()),
                        inspector_session_id,
                        lane,
                    ),
                    context_resolution_action,
                    raw_json,
                    response,
                ))
                .map(PendingRuntimeInspectorCommandDispatch::from_main_route),
            RendererInspectorCommandRoute::Io => {
                anyhow::ensure!(
                    context_resolution_action.is_none(),
                    "an IO Inspector command cannot require Page owner context resolution"
                );
                self.start_io_protocol_message(inspector_session_id, raw_json, Some(response))
            }
        }
    }

    pub(crate) fn start_io_protocol_message(
        &self,
        inspector_session_id: Option<String>,
        raw_json: String,
        response: Option<RendererRuntimeInspectorResponseSender>,
    ) -> anyhow::Result<PendingRuntimeInspectorCommandDispatch> {
        self.endpoint
            .enqueue_io_command(RendererInspectorCommandEnvelope::new_io(
                RendererInspectorIngressTicket::new(
                    Some(self.attachment.id()),
                    inspector_session_id,
                    RendererInspectorCommandRoute::Io,
                ),
                raw_json,
                response,
            ))
            .map(PendingRuntimeInspectorCommandDispatch::from_io_route)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RendererAgentDetachReason {
    ExplicitDetach,
    TargetClosed,
    TargetCrashed,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum DevToolsRendererChannelLifecycle {
    #[default]
    Open,
    Closed(RendererAgentDetachReason),
}

#[derive(Debug, Default)]
pub(crate) struct DevToolsRendererChannel {
    lifecycle: DevToolsRendererChannelLifecycle,
    current: Option<RendererAgentBinding>,
    pending_document_navigations: HashSet<NavigationId>,
    pending_document_projection: Option<PendingDocumentProjection>,
    held_attachment: Option<RendererAgentAttachment>,
    buffered_output: Vec<BufferedRendererInspectorBatch>,
}

#[derive(Debug)]
struct PendingDocumentProjection {
    navigation: NavigationId,
    fence: DocumentProjectionFenceKey,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererOutputHoldRelease {
    held_attachment: Option<RendererAgentAttachment>,
    current_attachment: Option<RendererAgentAttachment>,
}

impl RendererOutputHoldRelease {
    #[cfg(test)]
    pub(crate) fn replacement(
        self,
    ) -> Option<(RendererAgentAttachmentId, RendererAgentAttachmentId)> {
        let held = self.held_attachment?;
        let current = self.current_attachment?;
        (held.id() != current.id()).then_some((held.id(), current.id()))
    }
}

#[derive(Debug)]
struct BufferedRendererInspectorBatch {
    attachment_id: RendererAgentAttachmentId,
    batch: RendererRuntimeInspectorMessageBatch,
}

impl DevToolsRendererChannel {
    pub(crate) fn attach_current(
        &mut self,
        document: DocumentId,
        browser_sequence: BrowserSequence,
        endpoint: RendererInspectionEndpoint,
    ) -> Result<Option<RendererAgentAttachment>, DevToolsRendererChannelError> {
        self.ensure_open()?;
        Ok(self
            .current
            .replace(RendererAgentBinding {
                attachment: RendererAgentAttachment::new(
                    document,
                    browser_sequence,
                    endpoint.agent_token(),
                ),
                endpoint,
            })
            .map(|previous| previous.attachment()))
    }

    pub(crate) fn current(&self) -> Option<RendererAgentAttachment> {
        self.current.as_ref().map(RendererAgentBinding::attachment)
    }

    pub(crate) fn current_binding(&self) -> Option<&RendererAgentBinding> {
        self.current.as_ref()
    }

    pub(crate) fn begin_document_projection(
        &mut self,
        navigation: NavigationId,
    ) -> Result<(), DevToolsRendererChannelError> {
        self.ensure_open()?;
        let was_pending = self.document_projection_is_pending();
        if !self.pending_document_navigations.insert(navigation) {
            return Err(DevToolsRendererChannelError::DuplicateNavigation);
        }
        if !was_pending {
            self.held_attachment = self.current();
        }
        Ok(())
    }

    /// Consume a Browser commit; the channel has no candidate or commit authority.
    pub(crate) fn document_committed(
        &mut self,
        navigation: NavigationId,
        document: DocumentId,
        browser_sequence: BrowserSequence,
        endpoint: RendererInspectionEndpoint,
    ) -> Result<
        (Option<RendererAgentAttachment>, DocumentProjectionFence),
        DevToolsRendererChannelError,
    > {
        self.ensure_open()?;
        if self.pending_document_projection.is_some() {
            return Err(DevToolsRendererChannelError::ProjectionPending);
        }
        if !self.pending_document_navigations.contains(&navigation) {
            return Err(DevToolsRendererChannelError::UnknownNavigation);
        }
        let previous = self.attach_current(document, browser_sequence, endpoint)?;
        self.pending_document_navigations
            .retain(|pending| *pending == navigation);
        let fence = DocumentProjectionFence::new(
            self.current()
                .expect("a successful renderer rebind must install its attachment"),
        );
        self.pending_document_projection = Some(PendingDocumentProjection {
            navigation,
            fence: fence.key,
        });
        Ok((previous, fence))
    }

    pub(crate) fn route_current_output(
        &mut self,
        attachment_id: RendererAgentAttachmentId,
        batches: Vec<RendererRuntimeInspectorMessageBatch>,
    ) -> Result<Vec<RendererRuntimeInspectorMessageBatch>, DevToolsRendererChannelError> {
        self.ensure_open()?;
        let Some(current) = self.current() else {
            return Ok(Vec::new());
        };
        if current.id() != attachment_id {
            return Err(DevToolsRendererChannelError::StaleAttachment);
        }
        self.route_validated_output(attachment_id, batches)
    }

    pub(crate) fn finish_navigation_without_document_projection(
        &mut self,
        navigation: &NavigationId,
    ) -> Result<Option<RendererOutputHoldRelease>, DevToolsRendererChannelError> {
        self.ensure_open()?;
        if self
            .pending_document_projection
            .as_ref()
            .is_some_and(|pending| &pending.navigation == navigation)
        {
            return Err(DevToolsRendererChannelError::ProjectionPending);
        }
        if !self.pending_document_navigations.remove(navigation)
            || self.document_projection_is_pending()
        {
            return Ok(None);
        }
        Ok(Some(RendererOutputHoldRelease {
            held_attachment: self.held_attachment.take(),
            current_attachment: self.current(),
        }))
    }

    pub(crate) fn publish_document_projection(
        &mut self,
        fence: DocumentProjectionFence,
    ) -> Result<Option<RendererOutputHoldRelease>, DevToolsRendererChannelError> {
        self.ensure_open()?;
        let Some(pending) = self.pending_document_projection.as_ref() else {
            return Err(DevToolsRendererChannelError::StaleProjectionFence);
        };
        if pending.fence != fence.key
            || self.current().is_none_or(|current| {
                current.document() != fence.document()
                    || current.browser_sequence() != fence.browser_sequence()
                    || current.id() != fence.renderer_attachment()
            })
        {
            return Err(DevToolsRendererChannelError::StaleProjectionFence);
        }
        let pending = self
            .pending_document_projection
            .take()
            .expect("validated pending Document projection");
        assert!(
            self.pending_document_navigations
                .remove(&pending.navigation),
            "a pending Document projection must retain its navigation hold"
        );
        if self.document_projection_is_pending() {
            return Ok(None);
        }
        Ok(Some(RendererOutputHoldRelease {
            held_attachment: self.held_attachment.take(),
            current_attachment: self.current(),
        }))
    }

    pub(crate) fn document_projection_is_pending(&self) -> bool {
        !self.pending_document_navigations.is_empty()
    }

    pub(crate) fn has_navigation(&self, navigation: &NavigationId) -> bool {
        self.pending_document_navigations.contains(navigation)
    }

    pub(crate) fn pending_navigation_count(&self) -> usize {
        self.pending_document_navigations.len()
    }

    pub(crate) fn take_released_output(&mut self) -> Vec<RendererRuntimeInspectorMessageBatch> {
        if self.document_projection_is_pending() {
            return Vec::new();
        }
        let Some(current) = self.current() else {
            self.buffered_output.clear();
            return Vec::new();
        };
        let released = self.take_buffered_current_output(current);
        self.buffered_output.clear();
        released
    }

    pub(crate) fn detach_current(
        &mut self,
        _reason: RendererAgentDetachReason,
    ) -> Result<Option<RendererAgentAttachment>, DevToolsRendererChannelError> {
        self.ensure_open()?;
        self.pending_document_navigations.clear();
        self.pending_document_projection = None;
        self.held_attachment = None;
        self.buffered_output.clear();
        Ok(self.current.take().map(|current| current.attachment()))
    }

    pub(crate) fn close(
        &mut self,
        reason: RendererAgentDetachReason,
    ) -> Option<RendererAgentAttachment> {
        if matches!(self.lifecycle, DevToolsRendererChannelLifecycle::Closed(_)) {
            return None;
        }
        self.lifecycle = DevToolsRendererChannelLifecycle::Closed(reason);
        self.pending_document_navigations.clear();
        self.pending_document_projection = None;
        self.held_attachment = None;
        self.buffered_output.clear();
        self.current.take().map(|current| current.attachment())
    }

    pub(crate) fn is_closed(&self) -> bool {
        matches!(self.lifecycle, DevToolsRendererChannelLifecycle::Closed(_))
    }

    pub(crate) fn reopen_after_target_crash(&mut self) -> bool {
        if !matches!(
            self.lifecycle,
            DevToolsRendererChannelLifecycle::Closed(RendererAgentDetachReason::TargetCrashed)
        ) {
            return false;
        }
        *self = Self::default();
        true
    }

    fn ensure_open(&self) -> Result<(), DevToolsRendererChannelError> {
        if self.is_closed() {
            return Err(DevToolsRendererChannelError::Closed);
        }
        Ok(())
    }

    fn route_validated_output(
        &mut self,
        attachment_id: RendererAgentAttachmentId,
        batches: Vec<RendererRuntimeInspectorMessageBatch>,
    ) -> Result<Vec<RendererRuntimeInspectorMessageBatch>, DevToolsRendererChannelError> {
        let Some(current) = self.current() else {
            return Ok(Vec::new());
        };
        if batches
            .iter()
            .any(|batch| batch.agent_token != current.agent_token())
        {
            return Err(DevToolsRendererChannelError::MismatchedAgent);
        }
        if self.document_projection_is_pending() {
            let releases_current_prefix = self.pending_document_projection.is_none()
                && batches
                    .iter()
                    .any(RendererRuntimeInspectorMessageBatch::has_renderer_protocol_response);
            self.buffer_output(attachment_id, batches);
            if releases_current_prefix {
                // Main ingress remains suspended, but Chromium's existing
                // renderer session pipe can still return IO responses until
                // endpoint replacement. Release the whole current-attachment
                // prefix so the response cannot overtake notifications that
                // preceded it in the same renderer journal.
                return Ok(self.take_buffered_current_output(current));
            }
            return Ok(Vec::new());
        }
        Ok(batches)
    }

    fn take_buffered_current_output(
        &mut self,
        current: RendererAgentAttachment,
    ) -> Vec<RendererRuntimeInspectorMessageBatch> {
        let mut released = Vec::new();
        let mut retained = Vec::new();
        for buffered in self.buffered_output.drain(..) {
            if buffered.attachment_id == current.id()
                && buffered.batch.agent_token == current.agent_token()
            {
                released.push(buffered.batch);
            } else {
                retained.push(buffered);
            }
        }
        self.buffered_output = retained;
        released
    }

    fn buffer_output(
        &mut self,
        attachment_id: RendererAgentAttachmentId,
        batches: Vec<RendererRuntimeInspectorMessageBatch>,
    ) {
        self.buffered_output
            .extend(
                batches
                    .into_iter()
                    .map(|batch| BufferedRendererInspectorBatch {
                        attachment_id,
                        batch,
                    }),
            );
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DevToolsRendererChannelError {
    Closed,
    DuplicateNavigation,
    UnknownNavigation,
    ProjectionPending,
    StaleProjectionFence,
    StaleAttachment,
    MismatchedAgent,
}

impl fmt::Display for DevToolsRendererChannelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Closed => "renderer channel is closed",
            Self::DuplicateNavigation => "renderer channel navigation is already in flight",
            Self::UnknownNavigation => "renderer channel navigation is not in flight",
            Self::ProjectionPending => "renderer Document projection is still pending",
            Self::StaleProjectionFence => "renderer Document projection fence is stale",
            Self::StaleAttachment => "renderer Inspector output belongs to a stale attachment",
            Self::MismatchedAgent => {
                "renderer Inspector output agent does not match its attachment"
            }
        })
    }
}

impl std::error::Error for DevToolsRendererChannelError {}

#[cfg(test)]
mod tests {
    use super::*;
    use moli_core::page::{DevToolsSessionKey, RendererRuntimeInspectorMessage};
    use serde_json::json;

    async fn inspection_page() -> (moli_core::runtime::Browser, moli_core::page::Page) {
        let browser = moli_core::runtime::Browser::new(Default::default()).unwrap();
        let page = browser
            .fetch("data:text/html,<title>binding</title>")
            .await
            .unwrap();
        (browser, page)
    }

    fn batch(
        agent_token: RendererDevToolsAgentToken,
        marker: &str,
    ) -> RendererRuntimeInspectorMessageBatch {
        RendererRuntimeInspectorMessageBatch::new(
            agent_token,
            DevToolsSessionKey::Primary,
            vec![RendererRuntimeInspectorMessage::protocol(json!({
                "method": "Runtime.consoleAPICalled",
                "params": { "marker": marker },
            }))],
        )
    }

    fn batch_marker(batch: &RendererRuntimeInspectorMessageBatch) -> Option<&str> {
        let RendererRuntimeInspectorMessage::Protocol(message) = batch.messages.first()? else {
            return None;
        };
        message
            .get("params")
            .and_then(|params| params.get("marker"))
            .and_then(serde_json::Value::as_str)
    }

    fn response_batch(
        agent_token: RendererDevToolsAgentToken,
        call_id: i32,
    ) -> RendererRuntimeInspectorMessageBatch {
        RendererRuntimeInspectorMessageBatch::new(
            agent_token,
            DevToolsSessionKey::Primary,
            vec![RendererRuntimeInspectorMessage::protocol(json!({
                "id": call_id,
                "result": {},
            }))],
        )
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn committed_binding_runtime_configuration_owns_ingress_without_page_or_channel_borrow() {
        let (browser, mut outgoing) = inspection_page().await;
        let mut committed_page = browser
            .fetch("data:text/html,<title>committed</title>")
            .await
            .unwrap();
        let mut channel = DevToolsRendererChannel::default();
        channel
            .attach_current(
                DocumentId::allocate(),
                BrowserSequence::allocate(),
                outgoing.renderer_inspection_endpoint(),
            )
            .unwrap();
        let original = channel.current().unwrap();
        let navigation = NavigationId::allocate();
        channel.begin_document_projection(navigation).unwrap();
        let (previous, _fence) = channel
            .document_committed(
                navigation,
                DocumentId::allocate(),
                BrowserSequence::allocate(),
                committed_page.renderer_inspection_endpoint(),
            )
            .unwrap();
        assert_eq!(previous, Some(original));
        let attachment = channel.current().unwrap();
        let registration = moli_core::page::RuntimeBindingRegistration {
            devtools_session: None,
            name: "committedBinding".into(),
            execution_context_name: None,
        };
        let pending = channel
            .current_binding()
            .unwrap()
            .runtime_inspection(None)
            .start_apply_runtime_protocol_state(
                &[],
                &[],
                std::slice::from_ref(&registration),
                std::slice::from_ref(&registration),
            )
            .unwrap();
        let enable = channel
            .current_binding()
            .unwrap()
            .start_runtime_enable_events(None)
            .unwrap();
        // Admitted commands own an exact endpoint, not a borrow of channel or Page.
        // Dropping a binding does not retire its Browser Page; admitted inspection
        // work still follows the renderer/session lifetime, not a channel borrow.
        drop(channel);
        let restored = PendingPageCommand::from_inspector_main_route(pending)
            .wait()
            .await
            .unwrap()
            .into_unit_page_command_turn()
            .unwrap();
        let (completion, predecessor) = restored.into_completion_and_predecessor();
        let (_, snapshot, _) = completion.into_parts();
        assert_eq!(
            predecessor.unwrap().cursor().stream().renderer_agent(),
            attachment.agent_token()
        );
        assert!(committed_page.observe_renderer_page_state(&snapshot));
        assert!(!outgoing.observe_renderer_page_state(&snapshot));
        let enabled = enable
            .wait()
            .await
            .unwrap()
            .into_runtime_protocol_message_command_turn()
            .unwrap();
        let (completion, predecessor) = enabled.into_completion_and_predecessor();
        let (_, snapshot, _) = completion.into_parts();
        assert_eq!(
            predecessor.unwrap().cursor().stream().renderer_agent(),
            attachment.agent_token()
        );
        assert!(committed_page.observe_renderer_page_state(&snapshot));
        assert!(!outgoing.observe_renderer_page_state(&snapshot));
        for (page, expected) in [
            (&mut outgoing, "undefined"),
            (&mut committed_page, "function"),
        ] {
            assert_eq!(
                page.evaluate_runtime_expression_async("typeof committedBinding")
                    .await
                    .unwrap()["value"],
                json!(expected),
                "runtime commands must only configure the committed endpoint"
            );
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn initial_attach_and_reattach_allocate_distinct_route_leases() {
        let (_browser, page) = inspection_page().await;
        let agent = page.renderer_devtools_agent_token();
        let mut channel = DevToolsRendererChannel::default();

        assert_eq!(
            channel.attach_current(
                DocumentId::allocate(),
                BrowserSequence::allocate(),
                page.renderer_inspection_endpoint()
            ),
            Ok(None)
        );
        let first = channel.current().expect("first attachment");
        assert_eq!(first.agent_token(), agent);

        let replaced = channel
            .attach_current(
                DocumentId::allocate(),
                BrowserSequence::allocate(),
                page.renderer_inspection_endpoint(),
            )
            .expect("reattach")
            .expect("replaced attachment");
        let second = channel.current().expect("second attachment");
        assert_eq!(replaced, first);
        assert_eq!(second.agent_token(), agent);
        assert_ne!(second.id(), first.id());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn failed_navigation_keeps_current_attachment() {
        let (_browser, page) = inspection_page().await;
        let request = NavigationId::allocate();
        let mut channel = DevToolsRendererChannel::default();
        channel
            .attach_current(
                DocumentId::allocate(),
                BrowserSequence::allocate(),
                page.renderer_inspection_endpoint(),
            )
            .expect("initial attach");
        let current = channel.current();

        channel
            .begin_document_projection(request)
            .expect("navigation start");
        assert!(channel.document_projection_is_pending());
        assert!(
            channel
                .finish_navigation_without_document_projection(&request)
                .expect("navigation finish")
                .is_some(),
            "a failed load finishes without committing its candidate"
        );

        assert_eq!(channel.current(), current);
        assert!(!channel.document_projection_is_pending());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn browser_commit_retires_superseded_projection_holds() {
        let (_browser, page) = inspection_page().await;
        let (_next_browser, next_page) = inspection_page().await;
        let first = NavigationId::allocate();
        let committed = NavigationId::allocate();
        let mut channel = DevToolsRendererChannel::default();
        channel
            .attach_current(
                DocumentId::allocate(),
                BrowserSequence::allocate(),
                page.renderer_inspection_endpoint(),
            )
            .unwrap();
        let original = channel.current().unwrap();
        channel.begin_document_projection(first).unwrap();
        channel.begin_document_projection(committed).unwrap();
        // Browser has already selected the winning Document. This is projection,
        // not a second validation/commit state machine.
        let (previous, fence) = channel
            .document_committed(
                committed,
                DocumentId::allocate(),
                BrowserSequence::allocate(),
                next_page.renderer_inspection_endpoint(),
            )
            .unwrap();
        assert_eq!(previous, Some(original));
        assert_eq!(
            channel.current().unwrap().agent_token(),
            next_page.renderer_devtools_agent_token()
        );
        assert_eq!(channel.pending_navigation_count(), 1);
        assert!(
            channel
                .publish_document_projection(fence)
                .unwrap()
                .is_some()
        );
        assert_eq!(
            channel.finish_navigation_without_document_projection(&first),
            Ok(None)
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn binding_does_not_keep_retired_page_admission_open() {
        let (_browser, page) = inspection_page().await;
        let mut channel = DevToolsRendererChannel::default();
        channel
            .attach_current(
                DocumentId::allocate(),
                BrowserSequence::allocate(),
                page.renderer_inspection_endpoint(),
            )
            .unwrap();
        drop(page);
        let binding = channel.current_binding().unwrap();
        for lane in [
            RendererInspectorCommandRoute::MainThread,
            RendererInspectorCommandRoute::Io,
        ] {
            let (tx, _rx) = tokio::sync::oneshot::channel();
            let error = binding
                .start_protocol_message(
                    None,
                    lane,
                    None,
                    json!({"id": 1, "method": "Debugger.pause"}).to_string(),
                    RendererRuntimeInspectorResponseSender::new(1, tx),
                )
                .err()
                .expect("retired Page must seal both lanes even while its binding survives");
            assert!(error.to_string().contains("Inspector Page is retired"));
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn detach_and_drop_binding_leave_browser_page_alive() {
        let (_browser, mut page) = inspection_page().await;
        let mut channel = DevToolsRendererChannel::default();
        channel
            .attach_current(
                DocumentId::allocate(),
                BrowserSequence::allocate(),
                page.renderer_inspection_endpoint(),
            )
            .unwrap();
        channel
            .detach_current(RendererAgentDetachReason::ExplicitDetach)
            .unwrap();
        assert!(channel.current_binding().is_none());
        drop(channel);
        assert_eq!(
            page.evaluate_runtime_expression_async("40 + 2")
                .await
                .unwrap(),
            json!({"type": "number", "value": 42, "description": "42"})
        );
    }

    #[test]
    fn output_remains_held_until_all_overlapping_navigations_finish() {
        let request_a = NavigationId::allocate();
        let request_b = NavigationId::allocate();
        let mut channel = DevToolsRendererChannel::default();

        channel
            .begin_document_projection(request_a)
            .expect("navigation A");
        channel
            .begin_document_projection(request_b)
            .expect("navigation B");
        assert_eq!(channel.pending_navigation_count(), 2);
        assert!(channel.document_projection_is_pending());

        assert_eq!(
            channel.finish_navigation_without_document_projection(&request_b),
            Ok(None)
        );
        assert!(channel.document_projection_is_pending());
        assert!(
            channel
                .finish_navigation_without_document_projection(&request_a)
                .expect("final overlapping navigation")
                .is_some()
        );
        assert!(!channel.document_projection_is_pending());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn projection_hold_rejects_duplicate_and_ignores_unknown_completion() {
        let request = NavigationId::allocate();
        let unknown = NavigationId::allocate();
        let mut channel = DevToolsRendererChannel::default();
        channel.begin_document_projection(request).unwrap();
        assert_eq!(
            channel.begin_document_projection(request),
            Err(DevToolsRendererChannelError::DuplicateNavigation)
        );
        assert_eq!(
            channel.finish_navigation_without_document_projection(&unknown),
            Ok(None)
        );
        assert!(channel.document_projection_is_pending());
        assert!(
            channel
                .finish_navigation_without_document_projection(&request)
                .unwrap()
                .is_some()
        );
        assert!(!channel.document_projection_is_pending());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn closed_channel_cannot_attach_or_restart() {
        let request = NavigationId::allocate();
        let (_browser, page) = inspection_page().await;
        let agent = page.renderer_devtools_agent_token();
        let mut channel = DevToolsRendererChannel::default();
        channel
            .attach_current(
                DocumentId::allocate(),
                BrowserSequence::allocate(),
                page.renderer_inspection_endpoint(),
            )
            .expect("initial attach");
        channel
            .begin_document_projection(request)
            .expect("navigation start");

        let detached = channel
            .close(RendererAgentDetachReason::TargetClosed)
            .expect("current attachment");
        assert_eq!(detached.agent_token(), agent);
        assert!(channel.is_closed());
        assert_eq!(channel.pending_navigation_count(), 0);
        assert_eq!(
            channel.attach_current(
                DocumentId::allocate(),
                BrowserSequence::allocate(),
                page.renderer_inspection_endpoint()
            ),
            Err(DevToolsRendererChannelError::Closed)
        );
        assert_eq!(
            channel.begin_document_projection(NavigationId::allocate()),
            Err(DevToolsRendererChannelError::Closed)
        );
        assert_eq!(
            channel.document_committed(
                request,
                DocumentId::allocate(),
                BrowserSequence::allocate(),
                page.renderer_inspection_endpoint(),
            ),
            Err(DevToolsRendererChannelError::Closed)
        );
        assert_eq!(channel.close(RendererAgentDetachReason::TargetClosed), None);
        assert!(!channel.reopen_after_target_crash());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn crashed_channel_reopens_for_target_recovery_navigation() {
        let (_browser, page) = inspection_page().await;
        let agent = page.renderer_devtools_agent_token();
        let mut channel = DevToolsRendererChannel::default();
        channel
            .attach_current(
                DocumentId::allocate(),
                BrowserSequence::allocate(),
                page.renderer_inspection_endpoint(),
            )
            .expect("initial attach");

        let detached = channel
            .close(RendererAgentDetachReason::TargetCrashed)
            .expect("crashed renderer attachment");
        assert_eq!(detached.agent_token(), agent);
        assert!(channel.is_closed());
        assert!(channel.reopen_after_target_crash());
        assert!(!channel.is_closed());
        assert!(!channel.reopen_after_target_crash());
        assert!(
            channel
                .begin_document_projection(NavigationId::allocate())
                .is_ok()
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn successful_cutover_releases_only_current_attachment_output() {
        let (_browser, page) = inspection_page().await;
        let old_agent = page.renderer_devtools_agent_token();
        let (_candidate_browser, candidate_page) = inspection_page().await;
        let new_agent = candidate_page.renderer_devtools_agent_token();
        let request = NavigationId::allocate();
        let mut channel = DevToolsRendererChannel::default();
        channel
            .attach_current(
                DocumentId::allocate(),
                BrowserSequence::allocate(),
                page.renderer_inspection_endpoint(),
            )
            .expect("old attach");
        let old_attachment = channel.current().expect("old attachment");
        channel
            .begin_document_projection(request)
            .expect("navigation start");

        assert!(
            channel
                .route_current_output(old_attachment.id(), vec![batch(old_agent, "old")])
                .expect("route old output")
                .is_empty()
        );
        let committed_document = DocumentId::allocate();
        let committed_sequence = BrowserSequence::allocate();
        let (_, fence) = channel
            .document_committed(
                request,
                committed_document,
                committed_sequence,
                candidate_page.renderer_inspection_endpoint(),
            )
            .unwrap();
        let current = channel.current().unwrap();
        assert_eq!(fence.document(), committed_document);
        assert_eq!(fence.browser_sequence(), committed_sequence);
        assert_eq!(fence.renderer_attachment(), current.id());
        assert!(
            channel
                .route_current_output(current.id(), vec![batch(new_agent, "new")])
                .unwrap()
                .is_empty(),
            "committing Browser state does not release new-generation output before projection finishes"
        );
        assert_eq!(
            channel.finish_navigation_without_document_projection(&request),
            Err(DevToolsRendererChannelError::ProjectionPending),
            "a generic navigation terminal must not bypass the committed projection fence"
        );
        assert!(
            channel
                .route_current_output(current.id(), vec![response_batch(new_agent, 18)])
                .expect("route new-generation response")
                .is_empty(),
            "even an IO response from the rebound attachment must wait for frame projection"
        );
        let stale_fence = DocumentProjectionFence {
            key: DocumentProjectionFenceKey {
                document: fence.document(),
                browser_sequence: BrowserSequence::allocate(),
                renderer_attachment: fence.renderer_attachment(),
            },
        };
        assert_eq!(
            channel.publish_document_projection(stale_fence),
            Err(DevToolsRendererChannelError::StaleProjectionFence),
            "a mismatched Browser occurrence must not consume the exact projection fence"
        );
        let resume = channel
            .publish_document_projection(fence)
            .expect("publish projection fence")
            .expect("channel resume");
        assert_eq!(
            resume.replacement(),
            Some((old_attachment.id(), channel.current().unwrap().id()))
        );

        let released = channel.take_released_output();
        assert_eq!(released.len(), 2);
        assert_eq!(batch_marker(&released[0]), Some("new"));
        assert!(released[1].has_renderer_protocol_response());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn failed_navigation_releases_buffered_current_output() {
        let (_browser, page) = inspection_page().await;
        let agent = page.renderer_devtools_agent_token();
        let request = NavigationId::allocate();
        let mut channel = DevToolsRendererChannel::default();
        channel
            .attach_current(
                DocumentId::allocate(),
                BrowserSequence::allocate(),
                page.renderer_inspection_endpoint(),
            )
            .expect("current attach");
        let attachment = channel.current().expect("current attachment");
        channel
            .begin_document_projection(request)
            .expect("navigation start");
        assert!(
            channel
                .route_current_output(attachment.id(), vec![batch(agent, "retained")])
                .expect("route output")
                .is_empty()
        );

        let resume = channel
            .finish_navigation_without_document_projection(&request)
            .expect("navigation finish")
            .expect("channel resume");
        assert_eq!(resume.replacement(), None);
        let released = channel.take_released_output();
        assert_eq!(released.len(), 1);
        assert_eq!(batch_marker(&released[0]), Some("retained"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn current_session_response_releases_its_buffered_prefix_during_navigation() {
        let (_browser, page) = inspection_page().await;
        let agent = page.renderer_devtools_agent_token();
        let request = NavigationId::allocate();
        let mut channel = DevToolsRendererChannel::default();
        channel
            .attach_current(
                DocumentId::allocate(),
                BrowserSequence::allocate(),
                page.renderer_inspection_endpoint(),
            )
            .expect("current attach");
        let attachment = channel.current().expect("current attachment");
        channel
            .begin_document_projection(request)
            .expect("navigation start");

        assert!(
            channel
                .route_current_output(attachment.id(), vec![batch(agent, "before-response")])
                .expect("route notification prefix")
                .is_empty()
        );
        let released = channel
            .route_current_output(attachment.id(), vec![response_batch(agent, 17)])
            .expect("route session response");

        assert_eq!(released.len(), 2);
        assert_eq!(batch_marker(&released[0]), Some("before-response"));
        assert!(released[1].has_renderer_protocol_response());
        assert!(channel.document_projection_is_pending());
        assert!(channel.take_released_output().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn stale_attachment_and_mismatched_agent_are_rejected() {
        let (_browser, page) = inspection_page().await;
        let agent = page.renderer_devtools_agent_token();
        let other_agent = RendererDevToolsAgentToken::allocate();
        let mut channel = DevToolsRendererChannel::default();
        channel
            .attach_current(
                DocumentId::allocate(),
                BrowserSequence::allocate(),
                page.renderer_inspection_endpoint(),
            )
            .expect("first attach");
        let stale = channel.current().expect("first attachment");
        channel
            .attach_current(
                DocumentId::allocate(),
                BrowserSequence::allocate(),
                page.renderer_inspection_endpoint(),
            )
            .expect("reattach");
        let current = channel.current().expect("current attachment");

        assert_eq!(
            channel.route_current_output(stale.id(), vec![batch(agent, "stale")]),
            Err(DevToolsRendererChannelError::StaleAttachment)
        );
        assert_eq!(
            channel.route_current_output(current.id(), vec![batch(other_agent, "wrong-agent")]),
            Err(DevToolsRendererChannelError::MismatchedAgent)
        );
    }
}
