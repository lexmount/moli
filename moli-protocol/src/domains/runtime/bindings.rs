use serde::Deserialize;

use crate::conn::{CdpConnection, CommandOwnerScope};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AddBindingParams {
    pub(super) name: String,
    #[serde(default)]
    pub(super) execution_context_name: Option<String>,
    #[serde(default)]
    pub(super) execution_context_id: Option<i64>,
}

pub(super) fn persist_runtime_binding_definition_for_owner(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    name: String,
    execution_context_name: Option<String>,
) -> Result<(), String> {
    conn.with_target_devtools_session_state_for_route_mut(
        owner.session_id(),
        owner.session_owner_route(),
        |state| state.upsert_runtime_binding_definition(name, execution_context_name),
    )
    .ok_or_else(|| "BrowserContextNotLoaded".to_owned())
}

pub(super) fn remove_runtime_binding_definitions_for_owner(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    name: &str,
) -> Result<(), String> {
    conn.with_target_devtools_session_state_for_route_mut(
        owner.session_id(),
        owner.session_owner_route(),
        |state| state.remove_runtime_binding_definitions(name),
    )
    .ok_or_else(|| "BrowserContextNotLoaded".to_owned())
}

pub(super) fn clear_runtime_binding_definitions_for_owner(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
) -> Result<(), String> {
    conn.with_target_devtools_session_state_for_route_mut(
        owner.session_id(),
        owner.session_owner_route(),
        |state| state.clear_runtime_binding_definitions(),
    )
    .ok_or_else(|| "BrowserContextNotLoaded".to_owned())
}
