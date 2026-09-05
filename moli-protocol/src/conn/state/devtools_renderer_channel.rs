use std::collections::HashSet;
use std::fmt;

use moli_core::page::{
    PendingDevToolsIoCommandDispatch, PendingPageCommand, PendingRuntimeInspectorCommandDispatch,
    RendererAgentAttachmentId, RendererDevToolsAgentToken, RendererInspectorCommandRoute,
    RendererRuntimeInspectorMessageBatch,
};
use moli_renderer_v8::{
    RendererInspectionEndpoint, RendererInspectorCommandEnvelope, RendererInspectorIngressTicket,
    RendererRuntimeInspectorMainCommandRoute, RendererRuntimeInspectorResponseSender,
};

use super::NavigationId;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererAgentAttachment {
    id: RendererAgentAttachmentId,
    agent_token: RendererDevToolsAgentToken,
}

impl RendererAgentAttachment {
    fn new(agent_token: RendererDevToolsAgentToken) -> Self {
        Self {
            id: RendererAgentAttachmentId::allocate(),
            agent_token,
        }
    }

    pub(crate) fn id(self) -> RendererAgentAttachmentId {
        self.id
    }

    pub(crate) fn agent_token(self) -> RendererDevToolsAgentToken {
        self.agent_token
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
    pub(crate) fn detach_session(
        &self,
        inspector_session_id: Option<String>,
    ) -> anyhow::Result<()> {
        self.endpoint.detach_session(inspector_session_id)
    }

    pub(crate) async fn restore_runtime_state(
        &self,
        inspector_session_id: Option<String>,
        session_restore_snapshots: &[moli_core::page::RendererInspectorSessionRestoreSnapshot],
        stored_runtime_bindings: &[moli_core::page::RuntimeBindingRegistration],
        session_runtime_bindings: &[moli_core::page::RuntimeBindingRegistration],
        runtime_enabled: bool,
    ) -> anyhow::Result<(
        std::sync::Arc<moli_renderer_v8::RendererPageState>,
        Option<moli_core::RendererOutputFence>,
    )> {
        let pending = self
            .runtime_inspection(inspector_session_id.clone())
            .start_apply_runtime_protocol_state(
                session_restore_snapshots,
                &[],
                stored_runtime_bindings,
                session_runtime_bindings,
            )?;
        let output = PendingPageCommand::from_inspector_main_route(pending)
            .wait()
            .await?
            .into_unit_page_command_turn()?;
        // Release the first Main handoff before admitting Runtime.enable. Its
        // frozen snapshot/fence remain valid without borrowing the candidate Page.
        let (completion, mut predecessor) = output.into_completion_and_predecessor();
        let (_, mut snapshot, _) = completion.into_parts();
        if runtime_enabled {
            let enabled = async {
                self.start_runtime_enable_events(inspector_session_id)?
                    .wait()
                    .await?
                    .into_runtime_protocol_message_command_turn()
            }
            .await;
            // Runtime enable replay retains the existing best-effort contract;
            // applying the stored configuration above is the required phase.
            if let Ok(output) = enabled {
                let (completion, tail) = output.into_completion_and_predecessor();
                let (_, enabled_snapshot, _) = completion.into_parts();
                snapshot = enabled_snapshot;
                if let Some(tail) = tail {
                    predecessor = Some(match predecessor {
                        Some(head) => head.latest_in_same_stream(tail),
                        None => tail,
                    });
                }
            }
        }
        Ok((snapshot, predecessor))
    }

    pub(crate) fn runtime_inspection(
        &self,
        inspector_session_id: Option<String>,
    ) -> moli_renderer_v8::RendererRuntimeInspection<'_> {
        self.endpoint
            .runtime_inspection(self.attachment.id(), inspector_session_id)
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

#[derive(Debug)]
enum RendererChannelAttachment {
    // Migration only: streaming navigation reserves its output route before
    // renderer bootstrap creates the Page. Commit 20 removes this Protocol
    // transaction participant; it never grants inspection of the old Page.
    AwaitingPage(RendererAgentAttachment),
    Bound(RendererAgentBinding),
}

impl RendererChannelAttachment {
    fn attachment(&self) -> RendererAgentAttachment {
        match self {
            Self::AwaitingPage(attachment) => *attachment,
            Self::Bound(binding) => binding.attachment,
        }
    }
}

#[derive(Debug)]
pub(crate) struct PreparedRendererAgentAttachment {
    navigation: NavigationId,
    renderer: RendererChannelAttachment,
}

impl PreparedRendererAgentAttachment {
    pub(crate) fn navigation(&self) -> &NavigationId {
        &self.navigation
    }

    #[cfg(test)]
    pub(crate) fn attachment(&self) -> RendererAgentAttachment {
        self.renderer.attachment()
    }

    pub(crate) fn id(&self) -> RendererAgentAttachmentId {
        self.renderer.attachment().id()
    }

    pub(crate) fn agent_token(&self) -> RendererDevToolsAgentToken {
        self.renderer.attachment().agent_token()
    }

    pub(crate) fn bind(
        &mut self,
        endpoint: RendererInspectionEndpoint,
    ) -> Result<(), DevToolsRendererChannelError> {
        let attachment = self.renderer.attachment();
        if attachment.agent_token() != endpoint.agent_token() {
            return Err(DevToolsRendererChannelError::CandidatePageAttachmentMismatch);
        }
        self.renderer = RendererChannelAttachment::Bound(RendererAgentBinding {
            attachment,
            endpoint,
        });
        Ok(())
    }

    pub(crate) fn binding(&self) -> Option<&RendererAgentBinding> {
        match &self.renderer {
            RendererChannelAttachment::AwaitingPage(_) => None,
            RendererChannelAttachment::Bound(binding) => Some(binding),
        }
    }
}

#[derive(Debug)]
pub(crate) struct CommittedRendererAgentAttachment {
    navigation: NavigationId,
    current: RendererAgentAttachment,
    previous: Option<RendererChannelAttachment>,
}

impl CommittedRendererAgentAttachment {
    pub(crate) fn navigation(&self) -> &NavigationId {
        &self.navigation
    }

    pub(crate) fn current(&self) -> RendererAgentAttachment {
        self.current
    }

    pub(crate) fn previous(&self) -> Option<RendererAgentAttachment> {
        self.previous
            .as_ref()
            .map(RendererChannelAttachment::attachment)
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
    current: Option<RendererChannelAttachment>,
    inflight_cross_document_navigations: HashSet<NavigationId>,
    suspended_attachment: Option<RendererAgentAttachment>,
    latest_started_navigation: Option<NavigationId>,
    committed_latest_navigation: Option<NavigationId>,
    buffered_output: Vec<BufferedRendererInspectorBatch>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererChannelResume {
    suspended_attachment: Option<RendererAgentAttachment>,
    current_attachment: Option<RendererAgentAttachment>,
}

impl RendererChannelResume {
    #[cfg(test)]
    pub(crate) fn replacement(
        self,
    ) -> Option<(RendererAgentAttachmentId, RendererAgentAttachmentId)> {
        let suspended = self.suspended_attachment?;
        let current = self.current_attachment?;
        (suspended.id() != current.id()).then_some((suspended.id(), current.id()))
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
        endpoint: RendererInspectionEndpoint,
    ) -> Result<Option<RendererAgentAttachment>, DevToolsRendererChannelError> {
        self.ensure_open()?;
        Ok(self
            .current
            .replace(RendererChannelAttachment::Bound(RendererAgentBinding {
                attachment: RendererAgentAttachment::new(endpoint.agent_token()),
                endpoint,
            }))
            .map(|previous| previous.attachment()))
    }

    pub(crate) fn current(&self) -> Option<RendererAgentAttachment> {
        self.current
            .as_ref()
            .map(RendererChannelAttachment::attachment)
    }

    pub(crate) fn current_binding(&self) -> Option<&RendererAgentBinding> {
        match self.current.as_ref()? {
            RendererChannelAttachment::Bound(binding) => Some(binding),
            RendererChannelAttachment::AwaitingPage(_) => None,
        }
    }

    pub(crate) fn bind_current(
        &mut self,
        endpoint: RendererInspectionEndpoint,
    ) -> Result<(), DevToolsRendererChannelError> {
        self.ensure_open()?;
        let attachment = self
            .current()
            .filter(|attachment| attachment.agent_token() == endpoint.agent_token())
            .ok_or(DevToolsRendererChannelError::CandidatePageAttachmentMismatch)?;
        self.current = Some(RendererChannelAttachment::Bound(RendererAgentBinding {
            attachment,
            endpoint,
        }));
        Ok(())
    }

    pub(crate) fn navigation_started(
        &mut self,
        navigation: NavigationId,
    ) -> Result<(), DevToolsRendererChannelError> {
        self.ensure_open()?;
        let was_suspended = self.output_is_suspended();
        if !self.inflight_cross_document_navigations.insert(navigation) {
            return Err(DevToolsRendererChannelError::DuplicateNavigation);
        }
        if !was_suspended {
            self.suspended_attachment = self.current();
        }
        self.latest_started_navigation = Some(navigation);
        self.committed_latest_navigation = None;
        Ok(())
    }

    pub(crate) fn attach_candidate(
        &self,
        navigation: &NavigationId,
        agent_token: RendererDevToolsAgentToken,
    ) -> Result<PreparedRendererAgentAttachment, DevToolsRendererChannelError> {
        self.ensure_open()?;
        if !self
            .inflight_cross_document_navigations
            .contains(navigation)
        {
            return Err(DevToolsRendererChannelError::UnknownNavigation);
        }
        Ok(PreparedRendererAgentAttachment {
            navigation: *navigation,
            renderer: RendererChannelAttachment::AwaitingPage(RendererAgentAttachment::new(
                agent_token,
            )),
        })
    }

    pub(crate) fn commit_candidate(
        &mut self,
        candidate: PreparedRendererAgentAttachment,
    ) -> Result<Option<RendererAgentAttachment>, DevToolsRendererChannelError> {
        self.commit_candidate_transaction(candidate)
            .map(|transaction| transaction.previous())
    }

    pub(crate) fn commit_candidate_transaction(
        &mut self,
        candidate: PreparedRendererAgentAttachment,
    ) -> Result<CommittedRendererAgentAttachment, DevToolsRendererChannelError> {
        self.ensure_open()?;
        if !self
            .inflight_cross_document_navigations
            .contains(candidate.navigation())
        {
            return Err(DevToolsRendererChannelError::UnknownNavigation);
        }
        if self.latest_started_navigation.as_ref() != Some(candidate.navigation()) {
            return Err(DevToolsRendererChannelError::SupersededNavigation);
        }
        if self.committed_latest_navigation.as_ref() == Some(candidate.navigation()) {
            return Err(DevToolsRendererChannelError::NavigationAlreadyCommitted);
        }
        self.inflight_cross_document_navigations
            .retain(|navigation| navigation == candidate.navigation());
        self.committed_latest_navigation = Some(candidate.navigation);
        let current = candidate.renderer.attachment();
        let previous = self.current.replace(candidate.renderer);
        Ok(CommittedRendererAgentAttachment {
            navigation: candidate.navigation,
            current,
            previous,
        })
    }

    pub(crate) fn rollback_committed_candidate(
        &mut self,
        transaction: CommittedRendererAgentAttachment,
    ) -> Result<(), DevToolsRendererChannelError> {
        self.ensure_open()?;
        if self.committed_latest_navigation.as_ref() != Some(transaction.navigation())
            || self.current() != Some(transaction.current())
        {
            return Err(DevToolsRendererChannelError::CommittedCandidateMismatch);
        }
        self.current = transaction.previous;
        self.committed_latest_navigation = None;
        Ok(())
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

    #[cfg(test)]
    pub(crate) fn route_candidate_output(
        &mut self,
        candidate: &PreparedRendererAgentAttachment,
        batches: Vec<RendererRuntimeInspectorMessageBatch>,
    ) -> Result<Vec<RendererRuntimeInspectorMessageBatch>, DevToolsRendererChannelError> {
        self.ensure_open()?;
        if !self
            .inflight_cross_document_navigations
            .contains(candidate.navigation())
        {
            return Err(DevToolsRendererChannelError::UnknownNavigation);
        }
        if self.latest_started_navigation.as_ref() != Some(candidate.navigation()) {
            return Err(DevToolsRendererChannelError::SupersededNavigation);
        }
        if batches
            .iter()
            .any(|batch| batch.agent_token != candidate.attachment().agent_token())
        {
            return Err(DevToolsRendererChannelError::MismatchedAgent);
        }
        self.buffer_output(candidate.attachment().id(), batches);
        Ok(Vec::new())
    }

    pub(crate) fn navigation_finished(
        &mut self,
        navigation: &NavigationId,
    ) -> Result<Option<RendererChannelResume>, DevToolsRendererChannelError> {
        self.ensure_open()?;
        if !self.inflight_cross_document_navigations.remove(navigation)
            || self.output_is_suspended()
        {
            return Ok(None);
        }
        Ok(Some(RendererChannelResume {
            suspended_attachment: self.suspended_attachment.take(),
            current_attachment: self.current(),
        }))
    }

    pub(crate) fn output_is_suspended(&self) -> bool {
        !self.inflight_cross_document_navigations.is_empty()
    }

    pub(crate) fn has_navigation(&self, navigation: &NavigationId) -> bool {
        self.inflight_cross_document_navigations
            .contains(navigation)
    }

    pub(crate) fn inflight_navigation_count(&self) -> usize {
        self.inflight_cross_document_navigations.len()
    }

    pub(crate) fn take_released_output(&mut self) -> Vec<RendererRuntimeInspectorMessageBatch> {
        if self.output_is_suspended() {
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
        self.inflight_cross_document_navigations.clear();
        self.suspended_attachment = None;
        self.latest_started_navigation = None;
        self.committed_latest_navigation = None;
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
        if self.output_is_suspended() {
            let releases_current_prefix = batches
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
    SupersededNavigation,
    NavigationAlreadyCommitted,
    StaleAttachment,
    MismatchedAgent,
    CandidatePageAttachmentMismatch,
    CommittedCandidateMismatch,
}

impl fmt::Display for DevToolsRendererChannelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Closed => "renderer channel is closed",
            Self::DuplicateNavigation => "renderer channel navigation is already in flight",
            Self::UnknownNavigation => "renderer channel navigation is not in flight",
            Self::SupersededNavigation => {
                "renderer channel navigation was superseded by a newer navigation"
            }
            Self::NavigationAlreadyCommitted => {
                "renderer channel navigation candidate was already committed"
            }
            Self::StaleAttachment => "renderer Inspector output belongs to a stale attachment",
            Self::MismatchedAgent => {
                "renderer Inspector output agent does not match its attachment"
            }
            Self::CandidatePageAttachmentMismatch => {
                "navigation candidate attachment does not match its Page"
            }
            Self::CommittedCandidateMismatch => {
                "committed renderer candidate transaction no longer matches the channel"
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
    async fn candidate_restore_uses_its_own_binding_and_moves_it_through_commit_and_rollback() {
        let (browser, mut outgoing) = inspection_page().await;
        let mut candidate_page = browser
            .fetch("data:text/html,<title>candidate</title>")
            .await
            .unwrap();
        let mut channel = DevToolsRendererChannel::default();
        channel
            .attach_current(outgoing.renderer_inspection_endpoint())
            .unwrap();
        let original = channel.current().unwrap();
        let navigation = NavigationId::allocate();
        channel.navigation_started(navigation).unwrap();
        let mut candidate = channel
            .attach_candidate(&navigation, candidate_page.renderer_devtools_agent_token())
            .unwrap();
        assert!(
            candidate.binding().is_none(),
            "a reservation cannot inspect the outgoing renderer"
        );
        assert!(
            candidate
                .bind(outgoing.renderer_inspection_endpoint())
                .is_err()
        );
        candidate
            .bind(candidate_page.renderer_inspection_endpoint())
            .unwrap();
        let candidate_attachment = candidate.attachment();
        let registration = moli_core::page::RuntimeBindingRegistration {
            devtools_session: None,
            name: "candidateBinding".to_owned(),
            execution_context_name: None,
        };
        let (snapshot, predecessor) = candidate
            .binding()
            .unwrap()
            .restore_runtime_state(
                None,
                &[],
                std::slice::from_ref(&registration),
                std::slice::from_ref(&registration),
                true,
            )
            .await
            .unwrap();
        let predecessor = predecessor.expect("restore retains the concrete context output fence");
        assert_eq!(
            predecessor.cursor().stream().renderer_agent(),
            candidate_attachment.agent_token()
        );
        assert!(candidate_page.observe_renderer_page_state(&snapshot));
        assert!(!outgoing.observe_renderer_page_state(&snapshot));
        assert_eq!(channel.current(), Some(original));
        for (page, expected) in [
            (&mut outgoing, "undefined"),
            (&mut candidate_page, "function"),
        ] {
            let result = page
                .evaluate_runtime_expression_async("typeof candidateBinding")
                .await
                .unwrap();
            assert_eq!(
                result["value"],
                json!(expected),
                "restore must not configure the outgoing VM"
            );
        }
        let transaction = channel.commit_candidate_transaction(candidate).unwrap();
        assert_eq!(
            channel.current_binding().unwrap().attachment(),
            candidate_attachment
        );
        channel.rollback_committed_candidate(transaction).unwrap();
        let restored = channel.current_binding().unwrap();
        assert_eq!(restored.attachment(), original);
        restored
            .start_runtime_enable_events(None)
            .unwrap()
            .wait()
            .await
            .unwrap()
            .into_runtime_protocol_message_command_turn()
            .unwrap();
        let result = PendingPageCommand::from_inspector_main_route(
            restored
                .runtime_inspection(None)
                .start_default_execution_context_id()
                .unwrap(),
        )
        .wait()
        .await
        .unwrap()
        .finish_runtime_optional_execution_context_id()
        .unwrap();
        assert!(
            result.is_some(),
            "rollback restores the actual outgoing endpoint, not only metadata"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn initial_attach_and_reattach_allocate_distinct_route_leases() {
        let (_browser, page) = inspection_page().await;
        let agent = page.renderer_devtools_agent_token();
        let mut channel = DevToolsRendererChannel::default();

        assert_eq!(
            channel.attach_current(page.renderer_inspection_endpoint()),
            Ok(None)
        );
        let first = channel.current().expect("first attachment");
        assert_eq!(first.agent_token(), agent);

        let replaced = channel
            .attach_current(page.renderer_inspection_endpoint())
            .expect("reattach")
            .expect("replaced attachment");
        let second = channel.current().expect("second attachment");
        assert_eq!(replaced, first);
        assert_eq!(second.agent_token(), agent);
        assert_ne!(second.id(), first.id());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn failed_candidate_keeps_current_attachment() {
        let (_browser, page) = inspection_page().await;
        let candidate_agent = RendererDevToolsAgentToken::allocate();
        let request = NavigationId::allocate();
        let mut channel = DevToolsRendererChannel::default();
        channel
            .attach_current(page.renderer_inspection_endpoint())
            .expect("initial attach");
        let current = channel.current();

        channel
            .navigation_started(request)
            .expect("navigation start");
        let _candidate = channel
            .attach_candidate(&request, candidate_agent)
            .expect("candidate attach");
        assert!(channel.output_is_suspended());
        assert!(
            channel
                .navigation_finished(&request)
                .expect("navigation finish")
                .is_some(),
            "a failed load finishes without committing its candidate"
        );

        assert_eq!(channel.current(), current);
        assert!(!channel.output_is_suspended());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn overlapping_navigation_rejects_superseded_candidate() {
        let (_browser, page) = inspection_page().await;
        let initial_agent = page.renderer_devtools_agent_token();
        let candidate_a_agent = RendererDevToolsAgentToken::allocate();
        let candidate_b_agent = RendererDevToolsAgentToken::allocate();
        let request_a = NavigationId::allocate();
        let request_b = NavigationId::allocate();
        let mut channel = DevToolsRendererChannel::default();
        channel
            .attach_current(page.renderer_inspection_endpoint())
            .expect("initial attach");
        channel.navigation_started(request_a).expect("navigation A");
        let candidate_a = channel
            .attach_candidate(&request_a, candidate_a_agent)
            .expect("candidate A");
        channel.navigation_started(request_b).expect("navigation B");
        let candidate_b = channel
            .attach_candidate(&request_b, candidate_b_agent)
            .expect("candidate B");

        assert_eq!(
            channel.commit_candidate(candidate_a),
            Err(DevToolsRendererChannelError::SupersededNavigation)
        );
        let previous = channel
            .commit_candidate(candidate_b)
            .expect("commit latest candidate")
            .expect("initial attachment");
        assert_eq!(previous.agent_token(), initial_agent);
        assert_eq!(
            channel.current().map(RendererAgentAttachment::agent_token),
            Some(candidate_b_agent)
        );
        assert_eq!(
            channel.inflight_navigation_count(),
            1,
            "committing the latest candidate retires older superseded navigation transactions"
        );
        assert!(
            channel
                .navigation_finished(&request_b)
                .expect("committed navigation finish")
                .is_some()
        );
        assert_eq!(channel.navigation_finished(&request_a), Ok(None));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn committed_candidate_transaction_rolls_back_to_exact_previous_attachment() {
        let (_browser, page) = inspection_page().await;
        let (_candidate_browser, candidate_page) = inspection_page().await;
        let candidate_agent = candidate_page.renderer_devtools_agent_token();
        let request = NavigationId::allocate();
        let mut channel = DevToolsRendererChannel::default();
        channel
            .attach_current(page.renderer_inspection_endpoint())
            .expect("initial attach");
        let initial = channel.current().expect("initial attachment");
        channel
            .navigation_started(request)
            .expect("navigation start");
        let candidate = channel
            .attach_candidate(&request, candidate_agent)
            .expect("candidate attach");

        let transaction = channel
            .commit_candidate_transaction(candidate)
            .expect("candidate commit");
        assert_eq!(transaction.previous(), Some(initial));
        assert_eq!(channel.current(), Some(transaction.current()));
        assert!(
            channel.current_binding().is_none(),
            "reservation cannot inspect the outgoing Page"
        );
        assert_eq!(
            channel.bind_current(page.renderer_inspection_endpoint()),
            Err(DevToolsRendererChannelError::CandidatePageAttachmentMismatch)
        );
        assert!(channel.current_binding().is_none());
        channel
            .bind_current(candidate_page.renderer_inspection_endpoint())
            .unwrap();
        assert_eq!(
            channel.current_binding().unwrap().endpoint.agent_token(),
            candidate_agent
        );

        channel
            .rollback_committed_candidate(transaction)
            .expect("matching transaction should roll back");
        assert_eq!(channel.current(), Some(initial));
        assert_eq!(
            channel.current_binding().unwrap().endpoint.agent_token(),
            initial.agent_token()
        );
        assert_eq!(
            channel.committed_latest_navigation, None,
            "a rolled-back candidate is no longer committed"
        );
        assert!(
            channel.output_is_suspended(),
            "rollback keeps the navigation in flight until the protocol emits its terminal result"
        );
        assert!(
            channel
                .navigation_finished(&request)
                .expect("rolled-back navigation finish")
                .is_some()
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn binding_does_not_keep_retired_page_admission_open() {
        let (_browser, page) = inspection_page().await;
        let mut channel = DevToolsRendererChannel::default();
        channel
            .attach_current(page.renderer_inspection_endpoint())
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
            .attach_current(page.renderer_inspection_endpoint())
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
    fn output_remains_suspended_until_all_overlapping_navigations_finish() {
        let request_a = NavigationId::allocate();
        let request_b = NavigationId::allocate();
        let mut channel = DevToolsRendererChannel::default();

        channel.navigation_started(request_a).expect("navigation A");
        channel.navigation_started(request_b).expect("navigation B");
        assert_eq!(channel.inflight_navigation_count(), 2);
        assert!(channel.output_is_suspended());

        assert_eq!(channel.navigation_finished(&request_b), Ok(None));
        assert!(channel.output_is_suspended());
        assert!(
            channel
                .navigation_finished(&request_a)
                .expect("final overlapping navigation")
                .is_some()
        );
        assert!(!channel.output_is_suspended());
    }

    #[test]
    fn navigation_transition_rejects_duplicate_unknown_and_second_commit() {
        let request = NavigationId::allocate();
        let unknown = NavigationId::allocate();
        let mut channel = DevToolsRendererChannel::default();
        channel
            .navigation_started(request)
            .expect("navigation start");
        assert_eq!(
            channel.navigation_started(request),
            Err(DevToolsRendererChannelError::DuplicateNavigation)
        );
        assert!(matches!(
            channel.attach_candidate(&unknown, RendererDevToolsAgentToken::allocate()),
            Err(DevToolsRendererChannelError::UnknownNavigation)
        ));
        assert_eq!(channel.navigation_finished(&unknown), Ok(None));

        let first = channel
            .attach_candidate(&request, RendererDevToolsAgentToken::allocate())
            .expect("first candidate");
        let second = channel
            .attach_candidate(&request, RendererDevToolsAgentToken::allocate())
            .expect("second candidate");
        channel.commit_candidate(first).expect("first commit");
        assert_eq!(
            channel.commit_candidate(second),
            Err(DevToolsRendererChannelError::NavigationAlreadyCommitted)
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn closed_channel_cannot_attach_or_restart() {
        let request = NavigationId::allocate();
        let (_browser, page) = inspection_page().await;
        let agent = page.renderer_devtools_agent_token();
        let mut channel = DevToolsRendererChannel::default();
        channel
            .attach_current(page.renderer_inspection_endpoint())
            .expect("initial attach");
        channel
            .navigation_started(request)
            .expect("navigation start");
        let candidate = channel
            .attach_candidate(&request, RendererDevToolsAgentToken::allocate())
            .expect("candidate");

        let detached = channel
            .close(RendererAgentDetachReason::TargetClosed)
            .expect("current attachment");
        assert_eq!(detached.agent_token(), agent);
        assert!(channel.is_closed());
        assert_eq!(channel.inflight_navigation_count(), 0);
        assert_eq!(
            channel.attach_current(page.renderer_inspection_endpoint()),
            Err(DevToolsRendererChannelError::Closed)
        );
        assert_eq!(
            channel.navigation_started(NavigationId::allocate()),
            Err(DevToolsRendererChannelError::Closed)
        );
        assert_eq!(
            channel.commit_candidate(candidate),
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
            .attach_current(page.renderer_inspection_endpoint())
            .expect("initial attach");

        let detached = channel
            .close(RendererAgentDetachReason::TargetCrashed)
            .expect("crashed renderer attachment");
        assert_eq!(detached.agent_token(), agent);
        assert!(channel.is_closed());
        assert!(channel.reopen_after_target_crash());
        assert!(!channel.is_closed());
        assert!(!channel.reopen_after_target_crash());
        assert!(channel.navigation_started(NavigationId::allocate()).is_ok());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn successful_cutover_releases_only_current_attachment_output() {
        let (_browser, page) = inspection_page().await;
        let old_agent = page.renderer_devtools_agent_token();
        let new_agent = RendererDevToolsAgentToken::allocate();
        let request = NavigationId::allocate();
        let mut channel = DevToolsRendererChannel::default();
        channel
            .attach_current(page.renderer_inspection_endpoint())
            .expect("old attach");
        let old_attachment = channel.current().expect("old attachment");
        channel
            .navigation_started(request)
            .expect("navigation start");
        let candidate = channel
            .attach_candidate(&request, new_agent)
            .expect("candidate");

        assert!(
            channel
                .route_current_output(old_attachment.id(), vec![batch(old_agent, "old")])
                .expect("route old output")
                .is_empty()
        );
        assert!(
            channel
                .route_candidate_output(&candidate, vec![batch(new_agent, "new")])
                .expect("route candidate output")
                .is_empty()
        );
        channel
            .commit_candidate(candidate)
            .expect("candidate commit");
        let resume = channel
            .navigation_finished(&request)
            .expect("navigation finish")
            .expect("channel resume");
        assert_eq!(
            resume.replacement(),
            Some((old_attachment.id(), channel.current().unwrap().id()))
        );

        let released = channel.take_released_output();
        assert_eq!(released.len(), 1);
        assert_eq!(batch_marker(&released[0]), Some("new"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn failed_navigation_releases_buffered_current_output() {
        let (_browser, page) = inspection_page().await;
        let agent = page.renderer_devtools_agent_token();
        let request = NavigationId::allocate();
        let mut channel = DevToolsRendererChannel::default();
        channel
            .attach_current(page.renderer_inspection_endpoint())
            .expect("current attach");
        let attachment = channel.current().expect("current attachment");
        channel
            .navigation_started(request)
            .expect("navigation start");
        assert!(
            channel
                .route_current_output(attachment.id(), vec![batch(agent, "retained")])
                .expect("route output")
                .is_empty()
        );

        let resume = channel
            .navigation_finished(&request)
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
            .attach_current(page.renderer_inspection_endpoint())
            .expect("current attach");
        let attachment = channel.current().expect("current attachment");
        channel
            .navigation_started(request)
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
        assert!(channel.output_is_suspended());
        assert!(channel.take_released_output().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn stale_attachment_and_mismatched_agent_are_rejected() {
        let (_browser, page) = inspection_page().await;
        let agent = page.renderer_devtools_agent_token();
        let other_agent = RendererDevToolsAgentToken::allocate();
        let mut channel = DevToolsRendererChannel::default();
        channel
            .attach_current(page.renderer_inspection_endpoint())
            .expect("first attach");
        let stale = channel.current().expect("first attachment");
        channel
            .attach_current(page.renderer_inspection_endpoint())
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
