use anyhow::{Context as _, Result};
use serde::Deserialize;

use crate::conn::{CdpConnection, CommandOwnerScope, RuntimeInspectorError};

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
) -> Result<()> {
    conn.with_target_devtools_session_state_for_owner_mut(owner, |state| {
        state.upsert_runtime_binding_definition(name, execution_context_name)
    })
    .context(RuntimeInspectorError::BrowserContextNotLoaded)
}

pub(super) fn remove_runtime_binding_definitions_for_owner(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    name: &str,
) -> Result<()> {
    conn.with_target_devtools_session_state_for_owner_mut(owner, |state| {
        state.remove_runtime_binding_definitions(name)
    })
    .context(RuntimeInspectorError::BrowserContextNotLoaded)
}

pub(super) fn clear_runtime_binding_definitions_for_owner(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
) -> Result<()> {
    conn.with_target_devtools_session_state_for_owner_mut(owner, |state| {
        state.clear_runtime_binding_definitions()
    })
    .context(RuntimeInspectorError::BrowserContextNotLoaded)
}
