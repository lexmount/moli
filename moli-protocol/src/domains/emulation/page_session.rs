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

pub(super) fn update_style_environment_state(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
    update: impl FnOnce(TargetEmulationStateUpdate<'_>),
) -> Result<(), String> {
    update_page_emulation_state(conn, session_id, update)
}

pub(super) fn set_context_emulated_media(
    conn: &mut CdpConnection,
    media: crate::conn::EmulatedMediaOverrides,
) -> Result<(), String> {
    let context = conn
        .browser_context
        .as_mut()
        .ok_or_else(|| "BrowserContextNotLoaded".to_owned())?;
    context.set_default_emulated_media(media);
    Ok(())
}

pub(super) fn set_context_preferred_text_scale(
    conn: &mut CdpConnection,
    scale: Option<f32>,
) -> Result<(), String> {
    let context = conn
        .browser_context
        .as_mut()
        .ok_or_else(|| "BrowserContextNotLoaded".to_owned())?;
    context.set_default_preferred_text_scale(scale);
    Ok(())
}
