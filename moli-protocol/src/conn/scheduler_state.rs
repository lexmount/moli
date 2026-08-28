use std::collections::VecDeque;

use super::{BackgroundProtocolEvent, NavigationBackgroundEvent};
#[cfg(test)]
use crate::domains::command_output::protocol_message_background_event;
use serde_json::{Value, json};

const RECENT_ACTIVITY_TRACE_LIMIT: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BackgroundNavigationGateKey {
    target_id: Option<String>,
    session_id: Option<String>,
    frame_id: String,
    loader_id: String,
    navigation_request_id: Option<u64>,
}

impl BackgroundNavigationGateKey {
    pub(crate) fn for_navigation(
        token: &super::state::DocumentNavigationToken,
        state: &super::state::NavigationDispatchState,
    ) -> Self {
        // Keep this key limited to fields that are stable from background task
        // spawn through lifecycle completion. `navigate_id` is intentionally
        // excluded because early Page.navigate replies clear it before the
        // completion is sent.
        Self {
            target_id: Some(token.target_id.clone()),
            session_id: state
                .navigate_session_id
                .clone()
                .or_else(|| state.session_id.clone()),
            frame_id: state.frame_id.clone(),
            loader_id: token.loader_id.clone(),
            navigation_request_id: Some(token.request_id.get()),
        }
    }

    /// Reports whether this gate belongs to the renderer/session owner used by
    /// an incoming CDP command.
    pub fn matches_session_owner(&self, session_id: Option<&str>) -> bool {
        self.session_id.as_deref() == session_id
    }

    pub(crate) fn target_id(&self) -> Option<&str> {
        self.target_id.as_deref()
    }

    pub(crate) fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    pub(crate) fn loader_id(&self) -> &str {
        &self.loader_id
    }

    #[cfg(feature = "test-support")]
    pub(crate) fn from_test_parts(
        target_id: Option<String>,
        session_id: Option<String>,
        frame_id: String,
        loader_id: String,
        navigation_request_id: Option<u64>,
    ) -> Self {
        Self {
            target_id,
            session_id,
            frame_id,
            loader_id,
            navigation_request_id,
        }
    }
}

#[derive(Debug)]
pub enum CdpSchedulerEvent {
    DocumentContinuationStarted {
        key: BackgroundNavigationGateKey,
    },
    ProtocolWorkPublished {
        work: crate::domains::activity::ProtocolSchedulerWork,
    },
    PageScreencastStarted {
        registration: crate::domains::page::PageScreencastRegistration,
    },
}

#[derive(Debug)]
pub struct CdpTurnOutcome {
    protocol_events: Vec<BackgroundProtocolEvent>,
    post_renderer_output_events: Vec<BackgroundProtocolEvent>,
    renderer_output_boundary: Option<moli_core::RendererOutputFence>,
    post_response_events: Vec<BackgroundProtocolEvent>,
    scheduler_events: Vec<CdpSchedulerEvent>,
}

/// One renderer-owner turn whose exact publication predecessor must be
/// projected before its protocol result becomes observable.
///
/// This wrapper is intentionally a different type from [`CdpTurnOutcome`]. A
/// scheduler hook cannot pass it to a protocol-only turn consumer and rely on
/// a runtime assertion to catch the lost predecessor.
#[must_use = "renderer owner turns must project or explicitly consume their predecessor"]
#[derive(Debug)]
pub struct CdpRendererOwnerTurnOutcome {
    turn: CdpTurnOutcome,
    renderer_output_predecessor: Option<moli_core::RendererOutputFence>,
}

impl CdpTurnOutcome {
    #[cfg(test)]
    pub(crate) fn new(
        protocol_messages: Vec<serde_json::Value>,
        scheduler_events: Vec<CdpSchedulerEvent>,
    ) -> Self {
        Self::new_with_protocol_events(
            protocol_messages
                .into_iter()
                .map(protocol_message_background_event)
                .collect(),
            scheduler_events,
        )
    }

    pub(crate) fn new_with_protocol_events(
        protocol_events: Vec<BackgroundProtocolEvent>,
        scheduler_events: Vec<CdpSchedulerEvent>,
    ) -> Self {
        Self::new_with_protocol_and_post_response_events(
            protocol_events,
            Vec::new(),
            scheduler_events,
        )
    }

    pub(crate) fn new_with_protocol_and_post_response_events(
        protocol_events: Vec<BackgroundProtocolEvent>,
        post_response_events: Vec<BackgroundProtocolEvent>,
        scheduler_events: Vec<CdpSchedulerEvent>,
    ) -> Self {
        Self {
            protocol_events,
            post_renderer_output_events: Vec::new(),
            renderer_output_boundary: None,
            post_response_events,
            scheduler_events,
        }
    }

    /// Inserts one independent renderer publication between two already
    /// ordered protocol segments.
    ///
    /// Unlike `renderer_output_predecessor`, this cursor does not claim that
    /// the renderer output was caused by the command or belongs before its
    /// response. It preserves the source-time position of an independently
    /// transported renderer event, such as a main-Document commit.
    pub(crate) fn with_renderer_output_boundary(
        mut self,
        boundary: Option<moli_core::RendererOutputFence>,
        post_renderer_output_events: Vec<BackgroundProtocolEvent>,
    ) -> Self {
        assert!(
            self.renderer_output_boundary.is_none(),
            "one protocol turn cannot contain multiple renderer insertion boundaries"
        );
        let has_boundary = boundary.is_some();
        self.renderer_output_boundary = boundary;
        if has_boundary {
            self.post_renderer_output_events = post_renderer_output_events;
        } else {
            assert!(
                post_renderer_output_events.is_empty(),
                "post-renderer output requires an exact renderer boundary"
            );
        }
        self
    }

    /// Binds this protocol turn to the last concrete renderer record that must
    /// be admitted before the turn's response or completion event is exposed.
    ///
    /// One command/owner turn belongs to one renderer stream. Multiple
    /// contributions in that stream collapse to its latest cursor; combining
    /// unrelated streams is an ownership error rather than a frontier.
    pub(crate) fn with_renderer_output_predecessor(
        self,
        predecessor: Option<moli_core::RendererOutputFence>,
    ) -> CdpRendererOwnerTurnOutcome {
        CdpRendererOwnerTurnOutcome {
            turn: self,
            renderer_output_predecessor: predecessor,
        }
    }

    #[cfg(test)]
    pub fn into_parts(self) -> (Vec<serde_json::Value>, Vec<CdpSchedulerEvent>) {
        let mut protocol_events = self.protocol_events;
        assert!(
            self.renderer_output_boundary.is_none(),
            "tests must route an exact renderer boundary instead of flattening it"
        );
        protocol_events.extend(self.post_renderer_output_events);
        protocol_events.extend(self.post_response_events);
        (
            protocol_events
                .into_iter()
                .map(BackgroundProtocolEvent::into_protocol_message)
                .collect(),
            self.scheduler_events,
        )
    }

    pub fn into_protocol_event_parts(
        self,
    ) -> (Vec<BackgroundProtocolEvent>, Vec<CdpSchedulerEvent>) {
        let mut protocol_events = self.protocol_events;
        assert!(
            self.renderer_output_boundary.is_none(),
            "non-command protocol output cannot flatten an exact renderer boundary"
        );
        protocol_events.extend(self.post_renderer_output_events);
        protocol_events.extend(self.post_response_events);
        (protocol_events, self.scheduler_events)
    }

    pub fn into_command_turn_parts(
        self,
    ) -> (
        Vec<BackgroundProtocolEvent>,
        Vec<BackgroundProtocolEvent>,
        Option<moli_core::RendererOutputFence>,
        Vec<BackgroundProtocolEvent>,
        Vec<CdpSchedulerEvent>,
    ) {
        (
            self.protocol_events,
            self.post_renderer_output_events,
            self.renderer_output_boundary,
            self.post_response_events,
            self.scheduler_events,
        )
    }
}

impl CdpRendererOwnerTurnOutcome {
    pub fn renderer_output_predecessor(&self) -> Option<&moli_core::RendererOutputFence> {
        self.renderer_output_predecessor.as_ref()
    }

    #[cfg(test)]
    pub fn into_parts(self) -> (Vec<serde_json::Value>, Vec<CdpSchedulerEvent>) {
        assert!(
            self.renderer_output_predecessor.is_none(),
            "tests must project an exact renderer predecessor instead of flattening it"
        );
        self.turn.into_parts()
    }

    pub fn into_protocol_event_parts(
        self,
    ) -> (
        Vec<BackgroundProtocolEvent>,
        Vec<CdpSchedulerEvent>,
        Option<moli_core::RendererOutputFence>,
    ) {
        let (protocol_events, scheduler_events) = self.turn.into_protocol_event_parts();
        (
            protocol_events,
            scheduler_events,
            self.renderer_output_predecessor,
        )
    }

    pub fn into_renderer_owner_turn_parts(
        self,
    ) -> (
        Vec<BackgroundProtocolEvent>,
        Vec<BackgroundProtocolEvent>,
        Option<moli_core::RendererOutputFence>,
        Vec<BackgroundProtocolEvent>,
        Vec<CdpSchedulerEvent>,
        Option<moli_core::RendererOutputFence>,
    ) {
        let (
            protocol_events,
            post_renderer_output_events,
            renderer_output_boundary,
            post_response_events,
            scheduler_events,
        ) = self.turn.into_command_turn_parts();
        (
            protocol_events,
            post_renderer_output_events,
            renderer_output_boundary,
            post_response_events,
            scheduler_events,
            self.renderer_output_predecessor,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devtools_runtime::AutomationEvent;

    #[test]
    fn turn_outcome_raw_protocol_messages_regain_typed_sidecars() {
        let outcome = CdpTurnOutcome::new(
            vec![json!({
                "method": "Page.fileChooserOpened",
                "params": {
                    "frameId": "FRAME",
                    "backendNodeId": 42,
                    "mode": "selectSingle"
                },
                "sessionId": "SID"
            })],
            Vec::new(),
        );

        let (events, scheduler_events) = outcome.into_protocol_event_parts();
        assert!(scheduler_events.is_empty());
        let [(message, automation_event)] = events
            .into_iter()
            .map(BackgroundProtocolEvent::into_parts)
            .collect::<Vec<_>>()
            .try_into()
            .expect("expected one protocol event");

        assert_eq!(message["method"], json!("Page.fileChooserOpened"));
        assert_eq!(message["sessionId"], json!("SID"));
        assert!(matches!(
            automation_event,
            Some(AutomationEvent::PageFileChooserOpened(event))
                if event.frame_id.as_str() == "FRAME"
                    && event.backend_node_id == 42
                    && event.mode == "selectSingle"
        ));
    }
}

#[derive(Default)]
pub(super) struct CdpConnectionSchedulerState {
    pending_navigation_background_events: Vec<NavigationBackgroundEvent>,
    next_deferred_main_document_load_observation_id: u64,
    next_protocol_work_publish_sequence: u64,
    pub(super) renderer_output_ingress: crate::domains::activity::OrderedRendererOutputIngress,
    scheduler_events: Vec<CdpSchedulerEvent>,
    recent_activity_traces: VecDeque<Value>,
    next_activity_trace_id: u64,
}

impl CdpConnectionSchedulerState {
    pub(super) fn allocate_protocol_work_publish_sequence(
        &mut self,
    ) -> crate::domains::activity::ProtocolWorkPublishSequence {
        self.next_protocol_work_publish_sequence = self
            .next_protocol_work_publish_sequence
            .checked_add(1)
            .expect("protocol work publish sequence exhausted");
        crate::domains::activity::ProtocolWorkPublishSequence::new(
            self.next_protocol_work_publish_sequence,
        )
    }

    pub(super) fn allocate_deferred_main_document_load_observation_id(
        &mut self,
    ) -> super::DeferredMainDocumentLoadObservationId {
        self.next_deferred_main_document_load_observation_id = self
            .next_deferred_main_document_load_observation_id
            .checked_add(1)
            .expect("deferred main-document load observation identity exhausted");
        super::DeferredMainDocumentLoadObservationId(
            self.next_deferred_main_document_load_observation_id,
        )
    }

    pub(crate) fn push_scheduler_event(&mut self, event: CdpSchedulerEvent) {
        self.scheduler_events.push(event);
    }

    pub(super) fn extend_scheduler_events(&mut self, events: Vec<CdpSchedulerEvent>) {
        self.scheduler_events.extend(events);
    }

    pub(super) fn take_scheduler_events(&mut self) -> Vec<CdpSchedulerEvent> {
        std::mem::take(&mut self.scheduler_events)
    }

    pub(super) fn push_activity_trace(&mut self, mut event: Value) {
        if !moli_trace::cdp_nav_timing_enabled() && !moli_trace::cdp_runtime_trace_enabled() {
            return;
        }
        self.next_activity_trace_id = self.next_activity_trace_id.wrapping_add(1);
        if let Some(object) = event.as_object_mut() {
            object.insert("id".to_owned(), json!(self.next_activity_trace_id));
        }
        self.recent_activity_traces.push_back(event);
        while self.recent_activity_traces.len() > RECENT_ACTIVITY_TRACE_LIMIT {
            self.recent_activity_traces.pop_front();
        }
    }

    pub(super) fn push_navigation_background_event(&mut self, event: NavigationBackgroundEvent) {
        self.pending_navigation_background_events.push(event);
    }

    pub(super) fn take_navigation_background_events(&mut self) -> Vec<NavigationBackgroundEvent> {
        std::mem::take(&mut self.pending_navigation_background_events)
    }

    pub(super) fn moli_memory_diagnostics(&self) -> Value {
        json!({
            "pendingNavigationBackgroundEventCount": self.pending_navigation_background_events.len(),
            "pendingSchedulerEventCount": self.scheduler_events.len(),
            "recentActivityTraceCount": self.recent_activity_traces.len(),
            "recentActivityTraces": self
                .recent_activity_traces
                .iter()
                .cloned()
                .collect::<Vec<_>>(),
        })
    }
}
