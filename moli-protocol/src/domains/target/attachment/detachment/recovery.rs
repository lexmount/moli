use super::*;
use crate::domains::target::session_disposal::{
    FailedSessionPageRetirement, SessionDisposalIdentity,
};

/// Retained by the original command context, including while its supervised
/// task is cancelled or unwinds. It owns only exact cleanup authority.
pub(crate) struct TargetDetachmentFailureCleanup {
    sessions: Vec<SessionFailureCleanup>,
}

struct SessionFailureCleanup {
    identity: SessionDisposalIdentity,
    commit: SessionDetachmentCommit,
}

pub(crate) struct TargetDetachmentRecovery {
    sessions: Vec<(SessionFailureCleanup, FailedSessionPageRetirement)>,
    pages: Vec<moli_core::page::Page>,
    background_events: Vec<BackgroundProtocolEvent>,
    response: CommandOutputPlan,
}

impl TargetDetachmentFailureCleanup {
    pub(super) fn new(sessions: &[SessionDetachment]) -> Self {
        Self {
            sessions: sessions
                .iter()
                .filter_map(|session| {
                    if !session.disposal.owns_cleanup() {
                        return None;
                    }
                    Some(SessionFailureCleanup {
                        identity: session.identity.clone(),
                        commit: session.commit.clone(),
                    })
                })
                .collect(),
        }
    }

    pub(in crate::domains::target) fn prepare(
        self,
        conn: &mut CdpConnection,
        context: &mut CommandDispatchContext,
        response: CommandOutputPlan,
    ) -> TargetDetachmentRecovery {
        let mut sessions = Vec::new();
        let mut pages = Vec::new();
        let mut background_events = Vec::new();
        for session in self.sessions {
            if !session.identity.is_current(conn) {
                continue;
            }
            crate::domains::runtime::fail_pending_session_calls(
                conn,
                &mut background_events,
                context.protocol_events_mut(),
                session.identity.plan().session_id(),
            );
            let mut retired = session.identity.prepare_failed_page_retirement(conn);
            // Exact Page retirement owns Inspector destruction at close ACK.
            // Do not detach its primary Inspector through a live target route
            // while that target is already free to install a successor.
            let _ = session.identity.detach_inspector(conn);
            if let Some(page) = retired.take_page() {
                pages.push(page);
            }
            sessions.push((session, retired));
        }
        TargetDetachmentRecovery {
            sessions,
            pages,
            background_events,
            response,
        }
    }
}

impl TargetDetachmentRecovery {
    pub(in crate::domains::target) async fn wait(mut self) -> Self {
        for page in std::mem::take(&mut self.pages) {
            // Failure recovery belongs to the same command supervision entry.
            // Observe close before removing a binding, just as normal disposal.
            let _ = page.close_async().await;
        }
        self
    }

    pub(in crate::domains::target) fn finish(self, conn: &mut CdpConnection) -> CommandOutputPlan {
        let mut output = CommandOutputPlan::default();
        for event in self.background_events {
            output.push_background_event(event);
        }
        for (session, retired) in self.sessions {
            if !session.identity.is_current(conn) {
                continue;
            }
            for event in session
                .identity
                .finish_failed_page_retirement(conn, retired)
            {
                output.push_background_event(event);
            }
            let (events, error) =
                commit_session_detachment(conn, &session.identity, session.commit);
            for event in events {
                output.push_background_event(event);
            }
            if let Some(error) = error {
                tracing::warn!(session_id = session.identity.plan().session_id(), %error,
                    "failed to remove retired session domain state");
            }
        }
        output.extend(self.response);
        output
    }
}
