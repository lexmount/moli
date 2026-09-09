#[cfg(test)]
use crate::conn::NavigationDispatchState;
use crate::conn::{CdpConnection, CommandDispatchContext};
#[cfg(test)]
use crate::domains::activity::{
    MainDocumentDownloadNavigationActivity, MainDocumentNavigationActivity,
};
use crate::domains::command_output::CommandOutputBuffer;
#[cfg(test)]
use crate::domains::network::MaterializedDownloadDocumentProgress;

#[cfg(test)]
pub(super) async fn commit_download_navigation_async(
    conn: &mut CdpConnection,
    out: &mut CommandOutputBuffer,
    state: NavigationDispatchState,
    navigation: MaterializedDownloadDocumentProgress,
    command_context: &mut CommandDispatchContext,
) {
    let MaterializedDownloadDocumentProgress {
        final_url,
        progress_gate,
        body_artifact,
    } = navigation;
    let navigation_activity =
        MainDocumentNavigationActivity::new(state, final_url, progress_gate, None);
    let download_activity =
        MainDocumentDownloadNavigationActivity::new(navigation_activity, body_artifact);

    // Keep this boxed for the same reason as the loaded commit tail: the
    // navigation completion future is otherwise large on small test stacks.
    Box::pin(async move {
        download_activity
            .emit_commit_into_buffer_async(conn, out, command_context)
            .await;
    })
    .await;
}

pub(crate) async fn release_document_projection_output_async(
    conn: &mut CdpConnection,
    out: &mut CommandOutputBuffer,
    command_context: &mut CommandDispatchContext,
    owner: &crate::conn::CommandOwnerScope,
    release: crate::conn::DocumentProjectionOutputRelease,
) {
    let primary_session_id = conn
        .runtime_session_owner_primary_session_id_for_owner(owner)
        .or_else(|| owner.session_id().map(str::to_owned));
    let mut events = Vec::new();
    crate::domains::runtime::push_routed_renderer_runtime_inspector_message_batch_background_events(
        conn,
        &mut events,
        release.released_output,
        primary_session_id.as_deref(),
    );
    out.extend_background_events_after_messages(events);
    if let Some(replacements) = release.renderer_call_replacements {
        let (attachment, terminations, replays, failed_sessions) = replacements.into_parts();
        fail_navigation_inspection_sessions(
            conn,
            out,
            command_context,
            owner,
            failed_sessions,
            "Inspector call identity exhausted during navigation replay",
        );
        out.extend_background_events_after_messages(
            conn.terminate_prepared_renderer_calls_after_navigation(
                terminations,
                "Inspected target navigated or closed",
            ),
        );
        match conn
            .replay_prepared_renderer_calls_after_navigation_async(replays, attachment)
            .await
        {
            Ok(events) => out.extend_background_events_after_messages(events),
            Err(error) => tracing::warn!(%error, session_id = owner.session_id(),
                "failed to replay renderer Inspector commands after navigation"),
        }
    }
}

/// Rebind failures are session failures, never Browser crash or close commands.
pub(crate) fn fail_navigation_inspection_sessions(
    conn: &mut CdpConnection,
    out: &mut CommandOutputBuffer,
    command_context: &mut CommandDispatchContext,
    owner: &crate::conn::CommandOwnerScope,
    sessions: Vec<Option<String>>,
    reason: &'static str,
) {
    let mut events = Vec::new();
    for session_id in sessions {
        let inspector_owner = if let Some(session_id) = session_id {
            crate::conn::CommandOwnerScope::for_session(&session_id)
        } else {
            // None identifies this Target's primary, not the navigation caller
            // and not whichever Target happens to be selected now.
            let Some((browser_context_id, Some(target_id))) =
                conn.target_owner_identity_for_owner(owner)
            else {
                continue;
            };
            crate::conn::CommandOwnerScope::for_route(crate::conn::CdpSessionRoute::PageTarget {
                browser_context_id,
                target_id,
                session_key: moli_page_types::DevToolsSessionKey::Primary,
            })
        };
        conn.fail_pending_inspector_awaits_for_owner_background_events_into(
            &mut events,
            command_context.protocol_events_mut(),
            &inspector_owner,
            reason,
        );
    }
    out.extend_background_events_after_messages(events);
}
