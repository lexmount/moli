use moli_core::page::RendererMainDocumentCommit;

use crate::conn::{CdpConnection, CommandOwnerScope};
use crate::domains::activity::{
    ProtocolOutputPayloads, ProtocolOutputProjectionContext, ProtocolOutputSink, ProtocolOutputSlot,
};

/// Move-owned main-frame commit facts captured between V8's context reset and
/// creation of the replacement default context.
///
/// The renderer freezes these values at the commit boundary. Projection must
/// never rebuild them from the protocol's current target state: by the time a
/// publication arrives, another navigation may already own the target.
#[derive(Debug)]
pub(in crate::domains) struct MainDocumentCommitPreparedOutput {
    commits: Vec<RendererMainDocumentCommit>,
}

impl MainDocumentCommitPreparedOutput {
    fn new(commit: RendererMainDocumentCommit) -> Self {
        Self {
            commits: vec![commit],
        }
    }

    fn take_commits(&mut self) -> Vec<RendererMainDocumentCommit> {
        std::mem::take(&mut self.commits)
    }

    pub(in crate::domains) fn extend(&mut self, other: Self) {
        self.commits.extend(other.commits);
    }
}

pub(in crate::domains) async fn project_main_document_commit_async(
    conn: &mut CdpConnection,
    context: &mut ProtocolOutputProjectionContext<'_>,
    payloads: Option<&mut ProtocolOutputPayloads>,
) {
    let Some(commits) = payloads
        .and_then(ProtocolOutputPayloads::main_document_commit_mut)
        .map(MainDocumentCommitPreparedOutput::take_commits)
    else {
        return;
    };

    for commit in commits {
        let owner = context.owner();
        let Some((current_frame_id, _, _, _)) =
            conn.target_session_owner_frame_tree_identity_for_owner(owner)
        else {
            continue;
        };
        let Some(current_loader_id) =
            conn.target_session_owner_frame_tree_loader_id_for_owner(owner)
        else {
            continue;
        };
        if current_frame_id != commit.frame_id || current_loader_id != commit.loader_id {
            // Stream routing binds the publication to an exact Page
            // generation. This additional loader check prevents a queued
            // commit fact from being projected after replacement.
            continue;
        }

        let session_ids = conn.page_event_session_ids_for_owner(owner);
        let mut events = Vec::new();
        for session_id in session_ids {
            let event_owner = session_id
                .as_deref()
                .map(CommandOwnerScope::for_session)
                .unwrap_or_else(|| owner.clone());
            let lifecycle_enabled = conn
                .target_page_session_state_for_owner(&event_owner)
                .is_some_and(|state| state.page_lifecycle_events);
            super::emit_navigation_lifecycle_init_background_events(
                &mut events,
                session_id.as_deref(),
                lifecycle_enabled,
                &commit.frame_id,
                &commit.loader_id,
                commit.timestamp,
            );

            let dom_enabled = crate::domains::dom::dom_agent_enabled_for_owner(conn, &event_owner);
            super::emit_navigation_frame_commit_background_events(
                &mut events,
                session_id.as_deref(),
                dom_enabled,
                &commit.frame_id,
                &commit.loader_id,
                &commit.url,
                commit.unreachable_url.as_deref(),
                &commit.security_origin,
                &commit.secure_context_type,
            );
        }
        context.command.protocol_events_mut().extend(events);
    }
}

pub(in crate::domains) const SLOT_MAIN_DOCUMENT_COMMIT: ProtocolOutputSlot =
    ProtocolOutputSlot::MainDocumentCommit;

pub(in crate::domains) fn append_renderer_main_document_commit_to_output_sink(
    commit: RendererMainDocumentCommit,
    sink: &mut (impl ProtocolOutputSink + ?Sized),
) {
    sink.push_produced_slot(SLOT_MAIN_DOCUMENT_COMMIT);
    sink.push_prepared_payload(MainDocumentCommitPreparedOutput::new(commit).into());
}
