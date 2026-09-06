use crate::conn::{CdpConnection, CommandDispatchContext, NavigationDispatchState, NavigationId};
use crate::domains::activity::{
    MainDocumentDownloadNavigationActivity, MainDocumentNavigationActivity,
};
use crate::domains::command_output::CommandOutputBuffer;
use crate::domains::network::{
    MaterializedDownloadDocumentProgress, MaterializedLoadedDocumentProgress,
};
use moli_core::page::{
    RendererDocumentLifecycleEvent, RendererDocumentLifecycleEventKind,
    RendererDocumentLifecycleMilestone, RendererPageCreationArtifacts, RendererRuntimeRealmInfo,
};

pub(super) async fn commit_loaded_navigation_async(
    conn: &mut CdpConnection,
    out: &mut CommandOutputBuffer,
    token: &NavigationId,
    state: NavigationDispatchState,
    prepared: crate::conn::PreparedDocumentNavigation,
    navigation: MaterializedLoadedDocumentProgress,
    inspection_restore: Option<crate::conn::NavigationInspectionRestore>,
    command_context: &mut CommandDispatchContext,
) {
    let MaterializedLoadedDocumentProgress {
        pending_download,
        page_creation_artifacts,
        final_url,
        response_headers,
        response_from_cache,
        main_document_body,
        initial_runtime_realms,
        renderer_output_predecessor,
        main_document_commit: _,
        progress_gate,
        network_error_page,
    } = navigation;
    let is_network_error_page = network_error_page.is_some();
    let (page_creation_artifacts, mut deferred_initial_renderer_document_lifecycle_events) =
        split_renderer_page_creation_lifecycle_at_load_boundary(page_creation_artifacts);
    let mut navigation_activity =
        MainDocumentNavigationActivity::new(state, final_url.clone(), progress_gate, Some(*token));
    if let Some(error_page) = network_error_page.as_ref() {
        navigation_activity =
            navigation_activity.with_network_error_page_result(error_page.error_text().to_owned());
    }
    let Some(()) = commit_and_project_loaded_navigation_async(
        conn,
        out,
        navigation_activity.state(),
        prepared,
        initial_runtime_realms,
        inspection_restore,
        command_context,
    )
    .await
    else {
        return;
    };
    if !is_network_error_page {
        let _ = conn.commit_main_document_resource_for_owner(
            &navigation_activity.state().owner,
            navigation_activity.state().frame_id.clone(),
            navigation_activity.state().loader_id.clone(),
            final_url.clone(),
            response_headers,
            response_from_cache,
            main_document_body,
        );
    }

    let (renderer_document_binding, mut initial_renderer_document_lifecycle_events) = conn
        .bind_renderer_document_lifecycle_for_owner(
            &navigation_activity.state().owner,
            page_creation_artifacts,
            Some(*token),
            navigation_activity.state().frame_id.clone(),
            navigation_activity.state().loader_id.clone(),
        );
    let load_visibility_barrier_armed = renderer_document_binding.is_some()
        && conn.begin_renderer_document_load_visibility_barrier_for_owner(
            &navigation_activity.state().owner,
            &navigation_activity.state().loader_id,
        );
    if load_visibility_barrier_armed {
        // The creation artifact owns only the initial handoff prefix. Once
        // that prefix is taken, every later lifecycle fact is frozen directly
        // into the Page output FIFO, even if it is produced before protocol
        // finishes installing this binding. Never read the Page back here:
        // ordered ingress and the commit cursor preserve that handoff.
        let (_, visible_events) = conn.ingest_renderer_document_lifecycle_events_for_owner(
            &navigation_activity.state().owner,
            std::mem::take(&mut deferred_initial_renderer_document_lifecycle_events),
        );
        initial_renderer_document_lifecycle_events.extend(visible_events);
    }
    navigation_activity.defer_initial_renderer_document_lifecycle_events_until_load_boundary(
        deferred_initial_renderer_document_lifecycle_events,
    );
    // Keep the loaded commit tail boxed: the target/Patchright CDP test thread
    // has historically hit stack limits when this future is inlined.
    Box::pin(async move {
        navigation_activity
            .emit_loaded_navigation_commit_async(
                conn,
                out,
                pending_download,
                renderer_document_binding,
                initial_renderer_document_lifecycle_events,
                renderer_output_predecessor,
            )
            .await;
    })
    .await;
}

fn split_renderer_page_creation_lifecycle_at_load_boundary(
    mut artifacts: RendererPageCreationArtifacts,
) -> (
    RendererPageCreationArtifacts,
    Vec<RendererDocumentLifecycleEvent>,
) {
    let Some(load_sequence) = artifacts
        .lifecycle_snapshot
        .load
        .as_ref()
        .map(|stamp| stamp.sequence)
    else {
        return (artifacts, Vec::new());
    };

    let mut deferred = Vec::new();
    let mut before_load = Vec::new();
    for event in std::mem::take(&mut artifacts.initial_lifecycle_events) {
        if event.sequence >= load_sequence {
            deferred.push(event);
        } else {
            before_load.push(event);
        }
    }
    artifacts.initial_lifecycle_events = before_load;
    artifacts.lifecycle_snapshot.load = None;
    if artifacts
        .lifecycle_snapshot
        .terminated
        .as_ref()
        .is_some_and(|stamp| stamp.sequence >= load_sequence)
    {
        artifacts.lifecycle_snapshot.terminated = None;
    }

    if !deferred.iter().any(|event| {
        matches!(
            event.kind,
            RendererDocumentLifecycleEventKind::Milestone(RendererDocumentLifecycleMilestone::Load)
        )
    }) {
        tracing::warn!(
            load_sequence,
            "renderer page creation snapshot contained load without its journal event"
        );
    }

    (artifacts, deferred)
}

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

async fn commit_and_project_loaded_navigation_async(
    conn: &mut CdpConnection,
    out: &mut CommandOutputBuffer,
    state: &NavigationDispatchState,
    prepared: crate::conn::PreparedDocumentNavigation,
    initial_runtime_realms: Vec<RendererRuntimeRealmInfo>,
    inspection_restore: Option<crate::conn::NavigationInspectionRestore>,
    command_context: &mut CommandDispatchContext,
) -> Option<()> {
    let commit = match conn.commit_loaded_navigation(prepared) {
        Ok(commit) => commit,
        Err(error) => {
            super::navigation::push_navigation_commit_error(out, state, error.to_string());
            return None;
        }
    };

    // Everything below observes a committed Browser Document. Inspection failure
    // settles DevTools work; it must never turn this navigation into a rollback.
    let restoration = commit
        .inspection_projection
        .map_err(anyhow::Error::from)
        .and_then(|()| {
            let Some(commit_state) = inspection_restore else {
                return Ok(None);
            };
            let pending = conn
                .runtime_session_owner_slot_for_owner(&state.owner)
                .map_err(anyhow::Error::msg)?
                .current_renderer_inspection_binding()
                .ok_or_else(|| anyhow::anyhow!("committed Document has no inspection binding"))?
                .start_runtime_state_restore(
                    commit_state.renderer_runtime_inspector_session_id.clone(),
                    &commit_state.runtime_inspector_session_restore_snapshots,
                    &commit_state.stored_runtime_bindings,
                    &commit_state.session_runtime_bindings,
                    commit_state.runtime_frontend_enabled,
                );
            Ok(Some(pending))
        });
    // This await retains only the exact inspection capability, not the channel
    // or a Browser residence borrow used to resolve it.
    let restoration = match restoration {
        Ok(Some(pending)) => pending.await.map(Some),
        Ok(None) => Ok(None),
        Err(error) => Err(error),
    };
    let inspection_available = match restoration {
        Ok(restored) => {
            if let Some((snapshot, predecessor)) = restored {
                if let Some((context_id, Some(target_id))) =
                    conn.target_owner_identity_for_owner(&state.owner)
                    && let Some(context) = conn.browser_context_by_id_mut(&context_id)
                {
                    context.observe_renderer_page_state_for_target(&target_id, &snapshot);
                }
                if let Some(predecessor) = predecessor {
                    command_context.set_renderer_output_predecessor(predecessor);
                }
            }
            true
        }
        Err(error) => {
            tracing::warn!(%error, session_id = state.owner.session_id(),
                "inspection projection failed after Browser navigation committed");
            if let Ok(slot) = conn.runtime_session_owner_slot_mut_for_owner(&state.owner) {
                slot.install_pending_renderer_call_replacements(Default::default());
            }
            let mut sessions = conn.page_event_session_ids_for_owner(&state.owner);
            // Event routing can fall back to the navigation's attached session.
            // Failure cleanup must also include the actual sessionless primary.
            let primary = conn.runtime_session_owner_primary_session_id_for_owner(&state.owner);
            if !sessions.contains(&primary) {
                sessions.push(primary);
            }
            fail_navigation_inspection_sessions(
                conn,
                out,
                command_context,
                &state.owner,
                sessions,
                "Inspector rebind failed after navigation",
            );
            false
        }
    };
    commit.previous_document_retirement.close().await;
    if let Some(replaced_page_owner) = commit.replaced_page_owner.as_ref() {
        let worker_retirement_events =
            crate::domains::target::retire_dedicated_worker_targets_for_replaced_page_async(
                conn,
                replaced_page_owner,
            )
            .await;
        out.extend_background_events_after_messages(worker_retirement_events);
    }
    if let Some(continuation) = commit.committed_document_post_response_continuation {
        command_context
            .response_flush()
            .defer_until_response_flush(move || continuation.release());
    }
    if inspection_available
        && conn
            .target_runtime_session_state_for_owner(&state.owner)
            .is_some_and(|state| state.runtime_frontend_enabled)
    {
        let _ = conn
            .set_renderer_runtime_agent_owns_page_console_api_events_for_owner(&state.owner, true);
    }
    // Creation realms are inventory for BiDi listeners, not a second source of
    // live CDP executionContextCreated events. Those follow the renderer fence.
    let preload_channel_execution_context_ids = if inspection_available {
        dedupe_preload_channel_execution_context_ids(
            initial_runtime_realms
                .iter()
                .filter_map(runtime_realm_execution_context_id)
                .collect(),
        )
    } else {
        Vec::new()
    };
    if conn.target_owner_has_bidi_channel_preload_script_for_owner(&state.owner) {
        let mut events = Vec::new();
        for &execution_context_id in &preload_channel_execution_context_ids {
            Box::pin(crate::domains::runtime::start_bidi_preload_channel_listeners_for_execution_context_background_events_async(
                conn, &state.owner, execution_context_id, &mut events,
            )).await;
        }
        out.extend_background_events_after_messages(events);
    }
    Some(())
}

/// Rebind failures are session failures, never Browser crash or close commands.
pub(super) fn fail_navigation_inspection_sessions(
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

fn runtime_realm_execution_context_id(realm: &RendererRuntimeRealmInfo) -> Option<i64> {
    runtime_realm_has_native_unique_id(realm).then_some(realm.context_id)
}

fn runtime_realm_has_native_unique_id(realm: &RendererRuntimeRealmInfo) -> bool {
    realm
        .realm_id
        .as_deref()
        .is_some_and(|realm_id| !realm_id.is_empty())
}

fn dedupe_preload_channel_execution_context_ids(mut ids: Vec<i64>) -> Vec<i64> {
    let mut deduped = Vec::new();
    for id in ids.drain(..) {
        if !deduped.contains(&id) {
            deduped.push(id);
        }
    }
    deduped
}
