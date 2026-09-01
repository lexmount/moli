use crate::conn::{BackgroundProtocolEvent, CommandDispatchContext, CommandOwnerScope};
use crate::domains::network::{
    MainDocumentProgressBackgroundEventBarrier, MainDocumentProgressGate,
};

/// Exact protocol projection context for one already-frozen output batch.
///
/// The context selects only how a frozen fact is projected for a session or
/// command. It cannot inspect renderer state, select Page work, retry a
/// producer, or change the output families owned by the batch.
pub(in crate::domains) struct ProtocolOutputProjectionContext<'a> {
    owner: &'a CommandOwnerScope,
    pub(in crate::domains) command: &'a mut CommandDispatchContext,
    pub(in crate::domains) subresource_frame_id: Option<&'a str>,
    pub(in crate::domains) subresource_timestamp: Option<f64>,
    pub(in crate::domains) subresource_network_request_id: Option<&'a str>,
}

impl<'a> ProtocolOutputProjectionContext<'a> {
    pub(in crate::domains) fn new(
        owner: &'a CommandOwnerScope,
        command: &'a mut CommandDispatchContext,
    ) -> Self {
        Self {
            owner,
            command,
            subresource_frame_id: None,
            subresource_timestamp: None,
            subresource_network_request_id: None,
        }
    }

    pub(in crate::domains) fn owner(&self) -> &CommandOwnerScope {
        self.owner
    }
}

/// Projection guard for main-document background events captured before the
/// response body becomes externally visible.
pub(super) struct MainDocumentBodyCompleteProjection<'a> {
    progress_gate: &'a mut MainDocumentProgressGate,
}

impl<'a> MainDocumentBodyCompleteProjection<'a> {
    pub(super) fn new(progress_gate: &'a mut MainDocumentProgressGate) -> Self {
        Self { progress_gate }
    }

    pub(super) fn project_background_events(self, out: &mut Vec<BackgroundProtocolEvent>) {
        MainDocumentProgressBackgroundEventBarrier::drain_until_body_finished_visible(
            out,
            self.progress_gate,
        );
    }
}

#[cfg(test)]
mod tests {
    use crate::conn::{CommandDispatchContext, CommandOwnerScope};

    use super::ProtocolOutputProjectionContext;

    #[test]
    fn plans_use_closed_projection_families_with_exact_context() {
        let owner = CommandOwnerScope::for_session("session-1");
        let mut command = CommandDispatchContext::default();
        let context = ProtocolOutputProjectionContext {
            owner: &owner,
            command: &mut command,
            subresource_frame_id: Some("frame-1"),
            subresource_timestamp: Some(12.5),
            subresource_network_request_id: Some("REQ-1"),
        };

        assert_eq!(context.owner().session_id(), Some("session-1"));
        assert_eq!(context.subresource_frame_id, Some("frame-1"));
        assert_eq!(context.subresource_network_request_id, Some("REQ-1"));
        assert_eq!(context.subresource_timestamp, Some(12.5));
    }
}
