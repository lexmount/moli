use moli_core::page::RendererMainDocumentCommit;

use crate::conn::{CdpConnection, RendererPageResidenceIdentity};
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
    commits: Vec<(RendererPageResidenceIdentity, RendererMainDocumentCommit)>,
}

impl MainDocumentCommitPreparedOutput {
    fn new(renderer: RendererPageResidenceIdentity, commit: RendererMainDocumentCommit) -> Self {
        Self {
            commits: vec![(renderer, commit)],
        }
    }

    fn take_commits(&mut self) -> Vec<(RendererPageResidenceIdentity, RendererMainDocumentCommit)> {
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

    for (renderer, commit) in commits {
        let owner = context.owner();
        let RendererMainDocumentCommit::Frame {
            frame_id,
            loader_id,
            url,
            unreachable_url,
            security_origin,
            secure_context_type,
            timestamp,
        } = commit
        else {
            let events = conn.project_native_renderer_document_frame_commit(renderer);
            context.command.protocol_events_mut().extend(events);
            continue;
        };
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
        if current_frame_id != frame_id || current_loader_id != loader_id {
            // Stream routing binds the publication to an exact Page
            // generation. This additional loader check prevents a queued
            // commit fact from being projected after replacement.
            continue;
        }

        let session_ids = conn.page_event_session_ids_for_owner(owner);
        let mut events = Vec::new();
        for session_id in session_ids {
            let event_owner = owner.for_target_event_session(conn, session_id.as_deref());
            let lifecycle_enabled = conn
                .target_page_session_state_for_owner(&event_owner)
                .is_some_and(|state| state.page_lifecycle_events);
            super::emit_navigation_lifecycle_init_background_events(
                &mut events,
                session_id.as_deref(),
                lifecycle_enabled,
                &frame_id,
                &loader_id,
                timestamp,
            );

            let dom_enabled = crate::domains::dom::dom_agent_enabled_for_owner(conn, &event_owner);
            super::emit_navigation_frame_commit_background_events(
                &mut events,
                session_id.as_deref(),
                dom_enabled,
                &frame_id,
                &loader_id,
                &url,
                unreachable_url.as_deref(),
                &security_origin,
                &secure_context_type,
            );
        }
        context.command.protocol_events_mut().extend(events);
    }
}

pub(in crate::domains) const SLOT_MAIN_DOCUMENT_COMMIT: ProtocolOutputSlot =
    ProtocolOutputSlot::MainDocumentCommit;

pub(in crate::domains) fn append_renderer_main_document_commit_to_output_sink(
    renderer: RendererPageResidenceIdentity,
    commit: RendererMainDocumentCommit,
    sink: &mut (impl ProtocolOutputSink + ?Sized),
) {
    sink.push_produced_slot(SLOT_MAIN_DOCUMENT_COMMIT);
    sink.push_prepared_payload(MainDocumentCommitPreparedOutput::new(renderer, commit).into());
}
