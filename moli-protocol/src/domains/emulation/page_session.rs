use crate::conn::{CdpConnection, EmulationPolicyChange};

pub(super) fn update_page_emulation_state(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
    change: EmulationPolicyChange,
) -> Result<(), String> {
    if conn.apply_emulation_override_for_session_owner(session_id, change) {
        return Ok(());
    }
    Err("BrowserContextNotLoaded".to_owned())
}
