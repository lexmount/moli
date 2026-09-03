use crate::conn::{CdpConnection, TargetEmulationStateUpdate};

pub(super) fn update_page_emulation_state(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
    f: impl FnOnce(TargetEmulationStateUpdate<'_>),
) -> Result<(), String> {
    if conn.update_emulation_state_for_session_owner(session_id, |state| {
        if let Some(state) = state {
            f(state);
        }
    }) {
        return Ok(());
    }
    Err("BrowserContextNotLoaded".to_owned())
}
