use url::Url;

use crate::conn::{
    CdpConnection, CommandDispatchContext, CommittedRendererAgentAttachment,
    DocumentNavigationToken, LoadedNavigationRendererAttachmentCommit, NavigationDispatchState,
};
use crate::domains::activity::{
    MainDocumentDownloadNavigationActivity, MainDocumentNavigationActivity,
};
use crate::domains::command_output::CommandOutputBuffer;
use crate::domains::network::{
    MaterializedDownloadDocumentProgress, MaterializedLoadedDocumentProgress,
};
use moli_core::page::{
    Page, RendererDocumentLifecycleEvent, RendererDocumentLifecycleEventKind,
    RendererDocumentLifecycleMilestone, RendererPageCreationArtifacts, RendererRuntimeRealmInfo,
};

#[derive(Default)]
struct LoadedPageCommitOutcome {
    preload_channel_execution_context_ids: Vec<i64>,
}

pub(super) async fn commit_loaded_navigation_async(
    conn: &mut CdpConnection,
    out: &mut CommandOutputBuffer,
    token: Option<&DocumentNavigationToken>,
    state: NavigationDispatchState,
    navigation: MaterializedLoadedDocumentProgress,
    committed_renderer_attachment: Option<CommittedRendererAgentAttachment>,
    command_context: &mut CommandDispatchContext,
) {
    let MaterializedLoadedDocumentProgress {
        page,
        pending_download,
        page_creation_artifacts,
        final_url,
        response_headers,
        response_from_cache,
        main_document_body,
        initial_runtime_realms,
        renderer_output_predecessor,
        main_document_commit,
        progress_gate,
        navigation_engine,
        network_error_page,
    } = navigation;
    let target_url = network_error_page
        .as_ref()
        .map(|error_page| error_page.unreachable_url().clone())
        .unwrap_or_else(|| final_url.clone());
    let Some(main_document_commit) = main_document_commit else {
        let error = "loaded navigation is missing its frozen main Document commit identity";
        if state.navigate_id.is_some() {
            out.push_error_after_messages(-32000, error);
        } else {
            tracing::warn!(
                session_id = state.owner.session_id(),
                loader_id = state.loader_id,
                "{error} after early Page.navigate result"
            );
        }
        return;
    };
    let is_network_error_page = network_error_page.is_some();
    let (page_creation_artifacts, mut deferred_initial_renderer_document_lifecycle_events) =
        split_renderer_page_creation_lifecycle_at_load_boundary(page_creation_artifacts);
    let mut navigation_activity = MainDocumentNavigationActivity::new(
        state,
        final_url.clone(),
        progress_gate,
        token.cloned(),
    );
    if let Some(error_page) = network_error_page.as_ref() {
        navigation_activity =
            navigation_activity.with_network_error_page_result(error_page.error_text().to_owned());
    }
    let Some(commit) = restore_and_commit_loaded_navigation_page_async(
        conn,
        out,
        token,
        navigation_activity.state(),
        page,
        &final_url,
        &target_url,
        &main_document_commit,
        initial_runtime_realms,
        committed_renderer_attachment,
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

    let LoadedPageCommitOutcome {
        preload_channel_execution_context_ids: _,
    } = commit;
    let (renderer_document_binding, mut initial_renderer_document_lifecycle_events) = conn
        .bind_renderer_document_lifecycle_for_owner(
            &navigation_activity.state().owner,
            page_creation_artifacts,
            token.cloned(),
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
    if let Some(engine) = navigation_engine {
        conn.adopt_loaded_navigation_engine_for_owner(&navigation_activity.state().owner, engine);
    }

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

async fn restore_and_commit_loaded_navigation_page_async(
    conn: &mut CdpConnection,
    out: &mut CommandOutputBuffer,
    token: Option<&DocumentNavigationToken>,
    state: &NavigationDispatchState,
    page: Page,
    final_url: &Url,
    target_url: &Url,
    main_document_commit: &moli_core::page::RendererMainDocumentCommit,
    initial_runtime_realms: Vec<RendererRuntimeRealmInfo>,
    committed_renderer_attachment: Option<CommittedRendererAgentAttachment>,
    command_context: &mut CommandDispatchContext,
) -> Option<LoadedPageCommitOutcome> {
    let timing_enabled = moli_trace::cdp_nav_timing_enabled();
    let timing_started = timing_enabled.then(std::time::Instant::now);
    if timing_enabled {
        tracing::info!(
            target: "moli_cdp_nav_timing",
            url = %final_url,
            stage = "restore_commit_start",
        );
    }
    let mut outcome = LoadedPageCommitOutcome::default();
    let prepared_configuration_committed = committed_renderer_attachment.is_some();
    let mut page = page;
    let page_agent_token = page.renderer_devtools_agent_token();
    if let Some(transaction) = committed_renderer_attachment.as_ref() {
        if token != Some(transaction.navigation())
            || transaction.current().agent_token() != page_agent_token
            || conn.current_renderer_agent_attachment_id_for_owner(&state.owner)
                != Some(transaction.current().id())
        {
            tracing::warn!(
                session_id = state.owner.session_id(),
                "prepared navigation Page does not match its committed renderer attachment"
            );
            return None;
        }
        page.bind_renderer_agent_attachment(transaction.current().id());
    }
    let renderer_agent_candidate = match (token, committed_renderer_attachment.as_ref()) {
        (Some(token), None) => {
            match conn.prepare_renderer_agent_candidate_for_owner(&state.owner, token, &mut page) {
                Ok(candidate) => Some(candidate),
                Err(error) => {
                    tracing::debug!(
                        %error,
                        session_id = state.owner.session_id(),
                        loader_id = token.loader_id,
                        "dropping superseded renderer navigation candidate before commit"
                    );
                    return None;
                }
            }
        }
        _ => None,
    };
    let Some(commit_state) = conn.prepare_loaded_navigation_commit_for_owner(&state.owner) else {
        return Some(outcome);
    };
    let permission_overrides = conn
        .effective_permission_overrides_for_browser_context_id(&commit_state.browser_context_id);

    let restore_started = timing_enabled.then(std::time::Instant::now);
    let runtime_output_predecessor = if prepared_configuration_committed {
        Ok(None)
    } else {
        page.restore_runtime_protocol_state_async(
            commit_state.renderer_runtime_inspector_session_id.clone(),
            &commit_state.runtime_inspector_session_restore_snapshots,
            &[],
            &commit_state.stored_runtime_bindings,
            &commit_state.session_runtime_bindings,
            commit_state.runtime_frontend_enabled,
        )
        .await
    };
    match runtime_output_predecessor {
        Ok(runtime_output_predecessor) => {
            if let Some(predecessor) = runtime_output_predecessor {
                command_context.set_renderer_output_predecessor(predecessor);
            }
            let preload_channel_execution_context_ids = initial_runtime_realms
                .iter()
                .filter_map(runtime_realm_execution_context_id)
                .collect::<Vec<_>>();
            let preload_channel_execution_context_ids =
                dedupe_preload_channel_execution_context_ids(preload_channel_execution_context_ids);
            // `initial_runtime_realms` is current-state inventory. It is valid
            // for resolving BiDi preload listener context IDs, but it is not a
            // second source of live CDP lifecycle events. Context-created
            // notifications travel exclusively through the concrete renderer
            // output stream produced while applying Runtime configuration.
            outcome.preload_channel_execution_context_ids = preload_channel_execution_context_ids;
        }
        Err(error) => {
            if state.navigate_id.is_some() {
                out.push_error_after_messages(
                    -32000,
                    format!("failed to restore page runtime protocol state: {error}"),
                );
            } else {
                // No navigate_id means an early Page.navigate result already
                // shipped via response-head fast-ack. The error is invisible
                // to the client; log it so post-ack commit failures don't
                // disappear silently and leave the client waiting on
                // lifecycle events that will never arrive.
                tracing::warn!(
                    %error,
                    session_id = state.owner.session_id(),
                    "navigation commit failed after early Page.navigate result: runtime protocol state restore"
                );
            }
            return None;
        }
    }
    if let Some(started) = restore_started {
        tracing::info!(
            target: "moli_cdp_nav_timing",
            url = %final_url,
            stage = "restore_commit_runtime_restored",
            phase_ms = started.elapsed().as_millis(),
            elapsed_ms = timing_started
                .as_ref()
                .map(std::time::Instant::elapsed)
                .map(|elapsed| elapsed.as_millis())
                .unwrap_or_default(),
        );
    }
    let (fetch_subresource_enabled, fetch_subresource_resource_type) =
        commit_state.fetch_subresource_config;
    let fetch_restore_started = timing_enabled.then(std::time::Instant::now);
    if !prepared_configuration_committed
        && (fetch_subresource_enabled || fetch_subresource_resource_type.is_some())
        && let Err(error) = page
            .set_fetch_subresource_interception_async(
                fetch_subresource_enabled,
                fetch_subresource_resource_type,
            )
            .await
    {
        if state.navigate_id.is_some() {
            out.push_error_after_messages(
                -32000,
                format!("failed to restore page fetch interception state: {error}"),
            );
        } else {
            tracing::warn!(
                %error,
                session_id = state.owner.session_id(),
                "navigation commit failed after early Page.navigate result: fetch interception state restore"
            );
        }
        return None;
    }
    if let Some(started) = fetch_restore_started {
        tracing::info!(
            target: "moli_cdp_nav_timing",
            url = %final_url,
            stage = "restore_commit_fetch_restored",
            phase_ms = started.elapsed().as_millis(),
            elapsed_ms = timing_started
                .as_ref()
                .map(std::time::Instant::elapsed)
                .map(|elapsed| elapsed.as_millis())
                .unwrap_or_default(),
        );
    }
    let permission_started = timing_enabled.then(std::time::Instant::now);
    if !prepared_configuration_committed
        && !permission_overrides.is_empty()
        && let Err(error) = page
            .set_permission_overrides_async(&permission_overrides)
            .await
    {
        if state.navigate_id.is_some() {
            out.push_error_after_messages(
                -32000,
                format!("failed to apply page permission overrides: {error}"),
            );
        } else {
            tracing::warn!(
                %error,
                session_id = state.owner.session_id(),
                "navigation commit failed after early Page.navigate result: permission overrides apply"
            );
        }
        return None;
    }
    if let Some(started) = permission_started {
        tracing::info!(
            target: "moli_cdp_nav_timing",
            url = %final_url,
            stage = "restore_commit_permissions_restored",
            phase_ms = started.elapsed().as_millis(),
            elapsed_ms = timing_started
                .as_ref()
                .map(std::time::Instant::elapsed)
                .map(|elapsed| elapsed.as_millis())
                .unwrap_or_default(),
        );
    }
    let page_commit_started = timing_enabled.then(std::time::Instant::now);
    let page_commit = match conn
        .commit_loaded_navigation_page_for_owner_async(
            &state.owner,
            page,
            match committed_renderer_attachment {
                Some(transaction) => {
                    LoadedNavigationRendererAttachmentCommit::AlreadyCommitted(transaction)
                }
                None => LoadedNavigationRendererAttachmentCommit::Prepare(renderer_agent_candidate),
            },
            target_url,
        )
        .await
    {
        Some(Ok(commit)) => commit,
        Some(Err(error)) => {
            if state.navigate_id.is_some() {
                out.push_error_after_messages(
                    -32000,
                    format!("failed to collect navigation Inspector output: {error}"),
                );
            } else {
                tracing::warn!(
                    %error,
                    session_id = state.owner.session_id(),
                    "navigation commit failed after early Page.navigate result: Inspector output collection"
                );
            }
            return None;
        }
        None => return None,
    };
    if let Some(replaced_page_owner) = page_commit.replaced_page_owner.as_ref() {
        let worker_retirement_events =
            crate::domains::target::retire_dedicated_worker_targets_for_replaced_page_async(
                conn,
                replaced_page_owner,
            )
            .await;
        out.extend_background_events_after_messages(worker_retirement_events);
    }
    if let Some(continuation) = page_commit.committed_document_post_response_continuation {
        command_context
            .response_flush()
            .defer_until_response_flush(move || continuation.release());
    }
    let _ = conn.commit_loaded_navigation_target_identity_for_owner(
        &state.owner,
        main_document_commit,
        target_url,
    );
    if commit_state.runtime_frontend_enabled {
        let _ = conn
            .set_renderer_runtime_agent_owns_page_console_api_events_for_owner(&state.owner, true);
    }
    if let Some(token) = token {
        conn.commit_document_navigation_for_owner_if_matches(&state.owner, token);
    }
    if let Some(started) = page_commit_started {
        tracing::info!(
            target: "moli_cdp_nav_timing",
            url = %final_url,
            stage = "restore_commit_page_installed",
            phase_ms = started.elapsed().as_millis(),
            elapsed_ms = timing_started
                .as_ref()
                .map(std::time::Instant::elapsed)
                .map(|elapsed| elapsed.as_millis())
                .unwrap_or_default(),
        );
    }
    let preload_channel_execution_context_ids = if conn
        .target_owner_has_bidi_channel_preload_script_for_route(
            state.owner.session_id(),
            state.owner.session_owner_route(),
        ) {
        dedupe_preload_channel_execution_context_ids(std::mem::take(
            &mut outcome.preload_channel_execution_context_ids,
        ))
    } else {
        Vec::new()
    };
    let mut preload_channel_listener_events = Vec::new();
    for execution_context_id in preload_channel_execution_context_ids {
        Box::pin(
            crate::domains::runtime::start_bidi_preload_channel_listeners_for_execution_context_background_events_async(
                conn,
                &state.owner,
                execution_context_id,
                &mut preload_channel_listener_events,
            ),
        )
        .await;
    }
    out.extend_background_events_after_messages(preload_channel_listener_events);
    if let Some(started) = timing_started {
        tracing::info!(
            target: "moli_cdp_nav_timing",
            url = %final_url,
            stage = "restore_commit_done",
            elapsed_ms = started.elapsed().as_millis(),
        );
    }
    Some(outcome)
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
