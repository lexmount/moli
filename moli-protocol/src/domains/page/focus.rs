use crate::conn::{BackgroundProtocolEvent, CdpConnection, TargetPageResidenceIdentity};

/// Exact Page-scoped browser activation requested by renderer `Window.focus()`.
///
/// The target id alone is insufficient: a delayed record from a replaced or
/// closed Page must never activate a newer occupant of the same target slot.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PageTargetFocusRequestOwnerAction {
    page_owner: TargetPageResidenceIdentity,
    target_id: String,
}

impl PageTargetFocusRequestOwnerAction {
    pub(crate) fn new(page_owner: TargetPageResidenceIdentity, target_id: String) -> Self {
        Self {
            page_owner,
            target_id,
        }
    }

    fn into_parts(self) -> (TargetPageResidenceIdentity, String) {
        (self.page_owner, self.target_id)
    }
}

pub(crate) async fn complete_page_target_focus_request_owner_action_async(
    conn: &mut CdpConnection,
    action: PageTargetFocusRequestOwnerAction,
) -> Vec<BackgroundProtocolEvent> {
    let (page_owner, target_id) = action.into_parts();
    if !conn.target_page_residence_identity_is_installed(&page_owner) {
        return Vec::new();
    }

    // BrowserContext activation is only an internal owner-selection swap. It
    // must be restored after the target transaction and does not itself
    // represent Page focus.
    let restore_browser_context_id = conn.browser_context.as_ref().map(|bc| bc.id.clone());
    if !conn
        .activate_browser_context_by_id_async(page_owner.browser_context_id())
        .await
    {
        return Vec::new();
    }

    let mut events = Vec::new();
    if conn.target_page_residence_identity_is_installed(&page_owner) {
        match conn
            .promote_background_target_to_active_for_connection_async(&target_id)
            .await
        {
            Ok(Some(activation)) => events.extend(activation.into_protocol_events()),
            Ok(None) => tracing::debug!(%target_id, "dropping focus request for retired target"),
            Err(message) => {
                tracing::warn!(%message, %target_id, "failed to complete renderer Page focus request")
            }
        }
    }

    if let Some(restore_browser_context_id) = restore_browser_context_id
        && restore_browser_context_id != page_owner.browser_context_id()
        && conn.has_browser_context_id(&restore_browser_context_id)
    {
        let _ = conn
            .activate_browser_context_by_id_async(&restore_browser_context_id)
            .await;
    }
    events
}
