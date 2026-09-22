use crate::conn::{CdpConnection, CdpSessionRoute, CommandOwnerScope, TargetEmulationStateUpdate};

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
    mut update: impl for<'a> FnMut(TargetEmulationStateUpdate<'a>),
) -> Result<(), String> {
    let owners = if super::emulation_command_is_context_wide(conn, session_id) {
        conn.browser_context
            .as_ref()
            .map(|context| {
                context
                    .page_targets
                    .iter()
                    .map(|target| {
                        CommandOwnerScope::for_route(CdpSessionRoute::PageTarget {
                            browser_context_id: context.id.clone(),
                            target_id: target.target_id().to_owned(),
                            session_key: moli_page_types::DevToolsSessionKey::Primary,
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    } else {
        vec![CommandOwnerScope::capture(conn, session_id)]
    };
    if owners.is_empty() {
        return Err("BrowserContextNotLoaded".to_owned());
    }
    for owner in owners {
        if !conn.update_emulation_state_for_owner(&owner, |state| {
            if let Some(state) = state {
                update(state);
            }
        }) {
            return Err("BrowserContextNotLoaded".to_owned());
        }
    }
    Ok(())
}
