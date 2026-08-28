use super::model::{ChildWindowRealmInit, ChildWindowRealmSnapshot};
use crate::native_bridge::JsContextHost;
use anyhow::{Context, Result, ensure};

pub(super) fn capture_child_window_realm_snapshot(
    host: &JsContextHost,
    init: ChildWindowRealmInit,
) -> Result<ChildWindowRealmSnapshot> {
    ensure!(
        host.frame_document_projection_is_current(
            init.handle,
            init.projection.browsing_context_id(),
            init.projection.owner(),
        ),
        "refused to initialize a stale child Window realm for handle {}",
        init.handle.index()
    );

    let entry = host
        .child_browsing_contexts
        .get(&init.handle)
        .context("missing child browsing-context state")?;
    let snapshot = ChildWindowRealmSnapshot {
        handle: init.handle,
        browsing_context_id: init.projection.browsing_context_id(),
        owner: init.projection.owner(),
        current_url: host
            .child_browsing_context_current_url(init.handle)
            .context("missing child browsing-context URL")?,
        origin: host
            .child_browsing_context_window_origin(init.handle)
            .context("missing child browsing-context origin")?,
        window_name: entry.window_name().to_owned(),
        navigation_seed: entry.committed_navigation_entry_seed(),
        navigation_type: entry.performance_navigation_type().to_owned(),
        performance_time_origin: entry.performance_time_origin_millis(),
        policy: entry.document_policy_container_snapshot(),
    };

    validate_child_window_realm_snapshot(host, &snapshot)?;
    Ok(snapshot)
}

pub(super) fn validate_child_window_realm_snapshot(
    host: &JsContextHost,
    snapshot: &ChildWindowRealmSnapshot,
) -> Result<()> {
    ensure!(
        host.frame_document_projection_is_current(
            snapshot.handle,
            snapshot.browsing_context_id,
            snapshot.owner,
        ),
        "child Window document owner changed while realm state was captured"
    );
    ensure!(
        host.child_browsing_context_policy_container_snapshot(snapshot.handle)
            .as_ref()
            == Some(&snapshot.policy),
        "child Window policy changed while realm state was captured"
    );
    Ok(())
}
