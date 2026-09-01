use std::collections::HashMap;

use crate::conn::{CdpConnection, CommandOwnerScope};

#[cfg(test)]
use super::TargetRuntimeObservableSourceOutput;
use super::items::ObservableRuntimeEmissionCursor;
#[cfg(test)]
use super::{TargetRuntimeObservableSourceSummary, TargetRuntimeObservableState};

pub(in crate::domains) fn advance_runtime_observable_cursors_to_current_for_session_owner(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
) {
    let owner = CommandOwnerScope::capture(conn, session_id);
    advance_runtime_observable_cursors_to_current_for_owner(conn, &owner);
}

pub(in crate::domains) fn advance_runtime_observable_cursors_to_current_for_owner(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
) {
    if let Some((context_console_counts, exception_entries)) =
        runtime_observable_cursor_end_from_owner_queue_for_owner(conn, owner)
    {
        let _ = conn.with_target_owner_state_for_route_mut(
            owner.session_id(),
            owner.session_owner_route(),
            |owner_state| {
                owner_state.runtime_observable_state.advance_to_current(
                    context_console_counts,
                    0,
                    exception_entries,
                );
            },
        );
        return;
    }
    let Some((owner_queue_console_entries, exception_entries)) =
        runtime_observable_cursor_end_from_owner_observable_queue_for_owner(conn, owner)
    else {
        return;
    };
    let _ = conn.with_target_owner_state_for_route_mut(
        owner.session_id(),
        owner.session_owner_route(),
        |owner_state| {
            owner_state.runtime_observable_state.advance_to_current(
                HashMap::new(),
                owner_queue_console_entries,
                exception_entries,
            );
        },
    );
}

fn runtime_observable_cursor_end_from_owner_queue_for_owner(
    conn: &CdpConnection,
    owner: &CommandOwnerScope,
) -> Option<(HashMap<i64, usize>, usize)> {
    let runtime_slot = conn
        .runtime_session_owner_slot_for_route(owner.session_id(), owner.session_owner_route())
        .ok()?;
    let source = runtime_slot.observable_output_latest_source_tail()?;
    let url = conn.runtime_session_owner_target_url_for_route(
        owner.session_id(),
        owner.session_owner_route(),
    )?;
    (source.url() == url && runtime_slot.page_attachment_id() == Some(source.page_attachment_id()))
        .then_some(source)?
        .cursor_end()
}

fn runtime_observable_cursor_end_from_owner_observable_queue_for_owner(
    conn: &CdpConnection,
    owner: &CommandOwnerScope,
) -> Option<(usize, usize)> {
    conn.runtime_session_owner_slot_for_route(owner.session_id(), owner.session_owner_route())
        .ok()?
        .observable_output_cursor_end()
}

#[cfg(test)]
fn retain_unemitted_runtime_observable_prepared_source(
    conn: &CdpConnection,
    source: TargetRuntimeObservableSourceOutput,
) -> Option<TargetRuntimeObservableSourceOutput> {
    retain_unemitted_runtime_observable_prepared_source_for_session_owner(conn, None, source)
}

#[cfg(test)]
pub(in crate::domains) fn retain_unemitted_runtime_observable_prepared_source_for_session_owner(
    conn: &CdpConnection,
    session_id: Option<&str>,
    source: TargetRuntimeObservableSourceOutput,
) -> Option<TargetRuntimeObservableSourceOutput> {
    let summary = source.summary();
    runtime_observable_owner_has_unemitted_source(conn, session_id, &summary).then_some(source)
}

pub(super) fn mark_runtime_observable_emission_cursor_for_session_owner(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
    cursor: ObservableRuntimeEmissionCursor,
) {
    let (context_console_counts, exception_entries) = cursor.into_parts();
    mark_runtime_observable_activity_emitted_for_session_owner(
        conn,
        session_id,
        context_console_counts,
        exception_entries,
    );
}

fn mark_runtime_observable_activity_emitted_for_session_owner(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
    context_console_counts: HashMap<i64, usize>,
    exception_entries: usize,
) {
    let owner_session_id = runtime_observable_owner_session_id(conn, session_id);
    let _ = conn.with_target_owner_state_for_session_mut(owner_session_id, |owner_state| {
        owner_state
            .runtime_observable_state
            .mark_emitted_console_counts(context_console_counts);
        owner_state
            .runtime_observable_state
            .mark_emitted_exception_entries(exception_entries);
    });
}

#[cfg(test)]
fn runtime_observable_owner_runtime_frontend_enabled(
    conn: &CdpConnection,
    owner_session_id: Option<&str>,
) -> bool {
    conn.target_runtime_session_state_for_session(owner_session_id)
        .is_some_and(|state| state.runtime_frontend_enabled)
}

#[cfg(test)]
fn runtime_observable_owner_state<'a>(
    conn: &'a CdpConnection,
    owner_session_id: Option<&str>,
    default_state: &'a TargetRuntimeObservableState,
) -> &'a TargetRuntimeObservableState {
    conn.target_owner_state_for_session(owner_session_id)
        .map(|owner_state| &owner_state.runtime_observable_state)
        .unwrap_or(default_state)
}

#[cfg(test)]
fn runtime_observable_owner_has_unemitted_source(
    conn: &CdpConnection,
    session_id: Option<&str>,
    summary: &TargetRuntimeObservableSourceSummary,
) -> bool {
    let owner_session_id = runtime_observable_owner_session_id(conn, session_id);
    if !runtime_observable_owner_runtime_frontend_enabled(conn, owner_session_id) {
        return false;
    }
    let default_state = TargetRuntimeObservableState::default();
    runtime_observable_owner_state(conn, owner_session_id, &default_state)
        .has_unemitted_source(summary)
}

fn runtime_observable_owner_session_id<'a>(
    conn: &CdpConnection,
    session_id: Option<&'a str>,
) -> Option<&'a str> {
    match session_id {
        Some(session_id) if conn.session_route(Some(session_id)).is_some() => Some(session_id),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use moli_core::page::{
        RendererPageDiagnosticsSnapshot, RendererRuntimeObservableSourceSummary,
        RuntimeConsoleMessageSnapshot,
    };

    use crate::conn::{BrowserContext, TargetPageAttachmentId};

    use super::{
        advance_runtime_observable_cursors_to_current_for_session_owner,
        mark_runtime_observable_activity_emitted_for_session_owner,
        retain_unemitted_runtime_observable_prepared_source,
        retain_unemitted_runtime_observable_prepared_source_for_session_owner,
    };
    use crate::domains::observable_output::{
        TargetRuntimeObservableQueueState, TargetRuntimeObservableSourceOutput,
    };

    fn page_attachment_id(raw: u64) -> TargetPageAttachmentId {
        TargetPageAttachmentId::from_raw_for_test(raw)
    }

    fn prepared_source() -> TargetRuntimeObservableSourceOutput {
        let source_snapshot = RendererPageDiagnosticsSnapshot::from_runtime_observable_source(
            RendererRuntimeObservableSourceSummary::from_source_messages(
                Some(7),
                vec![RuntimeConsoleMessageSnapshot {
                    execution_context_id: 7,
                    message: "runtime source console".to_owned(),
                    args: Vec::new(),
                    stack: None,
                }],
                vec!["runtime source lifecycle".to_owned()],
            ),
        );
        let mut queue = TargetRuntimeObservableQueueState::default();
        queue.sync_source_from_renderer_snapshot(
            "http://example.test/runtime-source".to_owned(),
            page_attachment_id(3),
            &source_snapshot,
        );
        queue
            .source_snapshot()
            .latest_source_tail()
            .expect("test source snapshot should produce a source output")
    }

    #[test]
    fn prepared_source_presence_requires_runtime_frontend_enabled_and_unemitted_summary() {
        let mut conn = crate::conn::CdpConnection::default();
        assert!(
            retain_unemitted_runtime_observable_prepared_source(&conn, prepared_source()).is_none(),
            "source presence should require a browser context"
        );

        conn.browser_context = Some(BrowserContext::new_with_page_for_test("BID-1", "TID-1"));
        assert!(
            retain_unemitted_runtime_observable_prepared_source(&conn, prepared_source()).is_none(),
            "source presence should require Runtime.enable"
        );

        {
            let bc = conn
                .browser_context
                .as_mut()
                .expect("browser context should exist");
            bc.active_page_state_mut().devtools_sessions
                [moli_page_types::DevToolsSessionKey::Primary]
                .runtime_session_state
                .runtime_frontend_enabled = true;
        }
        assert!(
            retain_unemitted_runtime_observable_prepared_source(&conn, prepared_source()).is_some(),
            "unemitted source summary should remain visible after Runtime.enable"
        );

        {
            let bc = conn
                .browser_context
                .as_mut()
                .expect("browser context should exist");
            bc.active_page_state_mut()
                .owner_state
                .runtime_observable_state
                .mark_emitted_console_counts(HashMap::from([(7, 1)]));
            bc.active_page_state_mut()
                .owner_state
                .runtime_observable_state
                .mark_emitted_exception_entries(1);
        }
        assert!(
            retain_unemitted_runtime_observable_prepared_source(&conn, prepared_source()).is_none(),
            "fully consumed RuntimeObservable source summary should not produce a drain item"
        );
    }

    #[tokio::test]
    async fn runtime_observable_cursor_advance_uses_stored_source_queue_without_page_readback() {
        let mut conn = crate::conn::CdpConnection::default();
        conn.browser_context = Some(BrowserContext::new_with_page_for_test("BID-1", "TID-1"));
        let source_snapshot = RendererPageDiagnosticsSnapshot::from_runtime_observable_source(
            RendererRuntimeObservableSourceSummary::from_source_messages(
                Some(7),
                vec![RuntimeConsoleMessageSnapshot {
                    execution_context_id: 7,
                    message: "runtime source console".to_owned(),
                    args: Vec::new(),
                    stack: None,
                }],
                vec!["runtime source lifecycle".to_owned()],
            ),
        );
        {
            let bc = conn
                .browser_context
                .as_mut()
                .expect("browser context should exist");
            bc.set_target_url("http://example.test/runtime-source".to_owned());
            bc.active_page_state_mut()
                .runtime_slot
                .set_page_attachment_id_for_test(3);
            bc.active_page_state_mut()
                .runtime_slot
                .sync_observable_output_source_from_renderer_snapshot(
                    "http://example.test/runtime-source".to_owned(),
                    &source_snapshot,
                );
        }

        advance_runtime_observable_cursors_to_current_for_session_owner(&mut conn, None);

        let state = &conn
            .browser_context
            .as_ref()
            .expect("browser context should exist")
            .active_page_state()
            .owner_state
            .runtime_observable_state;
        assert_eq!(
            state.emitted_console_entries(),
            1,
            "cursor advance should use stored queue source counts even when no live page can be read back"
        );
        assert_eq!(
            state.emitted_exception_entries(),
            1,
            "cursor advance should use stored queue lifecycle count without live-page readback"
        );
    }

    #[test]
    fn runtime_observable_auxiliary_session_uses_own_browser_context() {
        let mut conn = crate::conn::CdpConnection::default();
        conn.browser_context = Some(BrowserContext::new("BID-active".to_owned()));

        let mut inactive = BrowserContext::new("BID-inactive".to_owned());
        inactive.set_active_target_id("TID-inactive".to_owned());
        inactive
            .active_page_state_mut()
            .devtools_sessions
            .ensure_attached("SID-aux")
            .runtime_session_state
            .runtime_frontend_enabled = true;
        assert!(inactive.assign_auxiliary_session_to_target("TID-inactive", "SID-aux".to_owned()));
        conn.inactive_browser_contexts.push(inactive);

        assert!(
            retain_unemitted_runtime_observable_prepared_source_for_session_owner(
                &conn,
                Some("SID-aux"),
                prepared_source(),
            )
            .is_some(),
            "auxiliary runtime observable presence should read the inactive owner context"
        );

        mark_runtime_observable_activity_emitted_for_session_owner(
            &mut conn,
            Some("SID-aux"),
            HashMap::from([(7, 1)]),
            1,
        );
        assert!(
            retain_unemitted_runtime_observable_prepared_source_for_session_owner(
                &conn,
                Some("SID-aux"),
                prepared_source(),
            )
            .is_none(),
            "auxiliary runtime observable mark should write the inactive owner context"
        );
    }
}
