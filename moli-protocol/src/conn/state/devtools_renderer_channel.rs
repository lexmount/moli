use std::collections::HashSet;
use std::fmt;

use moli_core::page::{
    RendererAgentAttachmentId, RendererDevToolsAgentToken, RendererRuntimeInspectorMessageBatch,
};

use super::DocumentNavigationToken;

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

#[derive(Debug)]
pub(crate) struct PreparedRendererAgentAttachment {
    navigation: DocumentNavigationToken,
    attachment: RendererAgentAttachment,
}

impl PreparedRendererAgentAttachment {
    pub(crate) fn navigation(&self) -> &DocumentNavigationToken {
        &self.navigation
    }

    #[cfg(test)]
    pub(crate) fn attachment(&self) -> RendererAgentAttachment {
        self.attachment
    }

    pub(crate) fn id(&self) -> RendererAgentAttachmentId {
        self.attachment.id()
    }
}

#[derive(Debug)]
pub(crate) struct CommittedRendererAgentAttachment {
    navigation: DocumentNavigationToken,
    current: RendererAgentAttachment,
    previous: Option<RendererAgentAttachment>,
}

impl CommittedRendererAgentAttachment {
    pub(crate) fn navigation(&self) -> &DocumentNavigationToken {
        &self.navigation
    }

    pub(crate) fn current(&self) -> RendererAgentAttachment {
        self.current
    }

    pub(crate) fn previous(&self) -> Option<RendererAgentAttachment> {
        self.previous
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
    current: Option<RendererAgentAttachment>,
    inflight_cross_document_navigations: HashSet<DocumentNavigationToken>,
    suspended_attachment: Option<RendererAgentAttachment>,
    latest_started_navigation: Option<DocumentNavigationToken>,
    committed_latest_navigation: Option<DocumentNavigationToken>,
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
        agent_token: RendererDevToolsAgentToken,
    ) -> Result<Option<RendererAgentAttachment>, DevToolsRendererChannelError> {
        self.ensure_open()?;
        Ok(self
            .current
            .replace(RendererAgentAttachment::new(agent_token)))
    }

    pub(crate) fn current(&self) -> Option<RendererAgentAttachment> {
        self.current
    }

    pub(crate) fn navigation_started(
        &mut self,
        navigation: DocumentNavigationToken,
    ) -> Result<(), DevToolsRendererChannelError> {
        self.ensure_open()?;
        let was_suspended = self.output_is_suspended();
        if !self
            .inflight_cross_document_navigations
            .insert(navigation.clone())
        {
            return Err(DevToolsRendererChannelError::DuplicateNavigation);
        }
        if !was_suspended {
            self.suspended_attachment = self.current;
        }
        self.latest_started_navigation = Some(navigation);
        self.committed_latest_navigation = None;
        Ok(())
    }

    pub(crate) fn attach_candidate(
        &self,
        navigation: &DocumentNavigationToken,
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
            navigation: navigation.clone(),
            attachment: RendererAgentAttachment::new(agent_token),
        })
    }

    pub(crate) fn commit_candidate(
        &mut self,
        candidate: PreparedRendererAgentAttachment,
    ) -> Result<Option<RendererAgentAttachment>, DevToolsRendererChannelError> {
        self.commit_candidate_transaction(candidate)
            .map(|transaction| transaction.previous)
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
        self.committed_latest_navigation = Some(candidate.navigation.clone());
        let current = candidate.attachment;
        let previous = self.current.replace(current);
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
            || self.current != Some(transaction.current())
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
        let Some(current) = self.current else {
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
        navigation: &DocumentNavigationToken,
    ) -> Result<Option<RendererChannelResume>, DevToolsRendererChannelError> {
        self.ensure_open()?;
        if !self.inflight_cross_document_navigations.remove(navigation)
            || self.output_is_suspended()
        {
            return Ok(None);
        }
        Ok(Some(RendererChannelResume {
            suspended_attachment: self.suspended_attachment.take(),
            current_attachment: self.current,
        }))
    }

    pub(crate) fn output_is_suspended(&self) -> bool {
        !self.inflight_cross_document_navigations.is_empty()
    }

    pub(crate) fn inflight_navigation_count(&self) -> usize {
        self.inflight_cross_document_navigations.len()
    }

    pub(crate) fn take_released_output(&mut self) -> Vec<RendererRuntimeInspectorMessageBatch> {
        if self.output_is_suspended() {
            return Vec::new();
        }
        let Some(current) = self.current else {
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
        Ok(self.current.take())
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
        self.current.take()
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
        let Some(current) = self.current else {
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

    fn navigation(label: u64) -> DocumentNavigationToken {
        DocumentNavigationToken {
            target_id: "TID-channel".to_owned(),
            loader_id: format!("LID-{label}"),
            request_id: crate::conn::state::NavigationRequestId::allocate(),
        }
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

    #[test]
    fn initial_attach_and_reattach_allocate_distinct_route_leases() {
        let agent = RendererDevToolsAgentToken::allocate();
        let mut channel = DevToolsRendererChannel::default();

        assert_eq!(channel.attach_current(agent), Ok(None));
        let first = channel.current().expect("first attachment");
        assert_eq!(first.agent_token(), agent);

        let replaced = channel
            .attach_current(agent)
            .expect("reattach")
            .expect("replaced attachment");
        let second = channel.current().expect("second attachment");
        assert_eq!(replaced, first);
        assert_eq!(second.agent_token(), agent);
        assert_ne!(second.id(), first.id());
    }

    #[test]
    fn failed_candidate_keeps_current_attachment() {
        let current_agent = RendererDevToolsAgentToken::allocate();
        let candidate_agent = RendererDevToolsAgentToken::allocate();
        let request = navigation(1);
        let mut channel = DevToolsRendererChannel::default();
        channel
            .attach_current(current_agent)
            .expect("initial attach");
        let current = channel.current();

        channel
            .navigation_started(request.clone())
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

    #[test]
    fn overlapping_navigation_rejects_superseded_candidate() {
        let initial_agent = RendererDevToolsAgentToken::allocate();
        let candidate_a_agent = RendererDevToolsAgentToken::allocate();
        let candidate_b_agent = RendererDevToolsAgentToken::allocate();
        let request_a = navigation(1);
        let request_b = navigation(2);
        let mut channel = DevToolsRendererChannel::default();
        channel
            .attach_current(initial_agent)
            .expect("initial attach");
        channel
            .navigation_started(request_a.clone())
            .expect("navigation A");
        let candidate_a = channel
            .attach_candidate(&request_a, candidate_a_agent)
            .expect("candidate A");
        channel
            .navigation_started(request_b.clone())
            .expect("navigation B");
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

    #[test]
    fn committed_candidate_transaction_rolls_back_to_exact_previous_attachment() {
        let initial_agent = RendererDevToolsAgentToken::allocate();
        let candidate_agent = RendererDevToolsAgentToken::allocate();
        let request = navigation(1);
        let mut channel = DevToolsRendererChannel::default();
        channel
            .attach_current(initial_agent)
            .expect("initial attach");
        let initial = channel.current().expect("initial attachment");
        channel
            .navigation_started(request.clone())
            .expect("navigation start");
        let candidate = channel
            .attach_candidate(&request, candidate_agent)
            .expect("candidate attach");

        let transaction = channel
            .commit_candidate_transaction(candidate)
            .expect("candidate commit");
        assert_eq!(transaction.previous(), Some(initial));
        assert_eq!(channel.current(), Some(transaction.current()));

        channel
            .rollback_committed_candidate(transaction)
            .expect("matching transaction should roll back");
        assert_eq!(channel.current(), Some(initial));
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

    #[test]
    fn output_remains_suspended_until_all_overlapping_navigations_finish() {
        let request_a = navigation(1);
        let request_b = navigation(2);
        let mut channel = DevToolsRendererChannel::default();

        channel
            .navigation_started(request_a.clone())
            .expect("navigation A");
        channel
            .navigation_started(request_b.clone())
            .expect("navigation B");
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
        let request = navigation(1);
        let unknown = navigation(2);
        let mut channel = DevToolsRendererChannel::default();
        channel
            .navigation_started(request.clone())
            .expect("navigation start");
        assert_eq!(
            channel.navigation_started(request.clone()),
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

    #[test]
    fn closed_channel_cannot_attach_or_restart() {
        let request = navigation(1);
        let agent = RendererDevToolsAgentToken::allocate();
        let mut channel = DevToolsRendererChannel::default();
        channel.attach_current(agent).expect("initial attach");
        channel
            .navigation_started(request.clone())
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
            channel.attach_current(agent),
            Err(DevToolsRendererChannelError::Closed)
        );
        assert_eq!(
            channel.navigation_started(navigation(2)),
            Err(DevToolsRendererChannelError::Closed)
        );
        assert_eq!(
            channel.commit_candidate(candidate),
            Err(DevToolsRendererChannelError::Closed)
        );
        assert_eq!(channel.close(RendererAgentDetachReason::TargetClosed), None);
        assert!(!channel.reopen_after_target_crash());
    }

    #[test]
    fn crashed_channel_reopens_for_target_recovery_navigation() {
        let agent = RendererDevToolsAgentToken::allocate();
        let mut channel = DevToolsRendererChannel::default();
        channel.attach_current(agent).expect("initial attach");

        let detached = channel
            .close(RendererAgentDetachReason::TargetCrashed)
            .expect("crashed renderer attachment");
        assert_eq!(detached.agent_token(), agent);
        assert!(channel.is_closed());
        assert!(channel.reopen_after_target_crash());
        assert!(!channel.is_closed());
        assert!(!channel.reopen_after_target_crash());
        assert!(channel.navigation_started(navigation(1)).is_ok());
    }

    #[test]
    fn successful_cutover_releases_only_current_attachment_output() {
        let old_agent = RendererDevToolsAgentToken::allocate();
        let new_agent = RendererDevToolsAgentToken::allocate();
        let request = navigation(1);
        let mut channel = DevToolsRendererChannel::default();
        channel.attach_current(old_agent).expect("old attach");
        let old_attachment = channel.current().expect("old attachment");
        channel
            .navigation_started(request.clone())
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

    #[test]
    fn failed_navigation_releases_buffered_current_output() {
        let agent = RendererDevToolsAgentToken::allocate();
        let request = navigation(1);
        let mut channel = DevToolsRendererChannel::default();
        channel.attach_current(agent).expect("current attach");
        let attachment = channel.current().expect("current attachment");
        channel
            .navigation_started(request.clone())
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

    #[test]
    fn current_session_response_releases_its_buffered_prefix_during_navigation() {
        let agent = RendererDevToolsAgentToken::allocate();
        let request = navigation(1);
        let mut channel = DevToolsRendererChannel::default();
        channel.attach_current(agent).expect("current attach");
        let attachment = channel.current().expect("current attachment");
        channel
            .navigation_started(request.clone())
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

    #[test]
    fn stale_attachment_and_mismatched_agent_are_rejected() {
        let agent = RendererDevToolsAgentToken::allocate();
        let other_agent = RendererDevToolsAgentToken::allocate();
        let mut channel = DevToolsRendererChannel::default();
        channel.attach_current(agent).expect("first attach");
        let stale = channel.current().expect("first attachment");
        channel.attach_current(agent).expect("reattach");
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
