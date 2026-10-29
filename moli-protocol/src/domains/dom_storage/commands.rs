use anyhow::Result;

use moli_core::{
    network::WebStorageAreaKind,
    page::{ChildFrameTreeSnapshot, Page},
};
use serde_json::json;
use url::Url;

use crate::{
    conn::{
        BrowserContextPageStorageHandles, CdpConnection, Cmd, CommandOwnerScope,
        CommandWorkflowContext, PreparedCommandWorkflow, PreparedRuntimePageQuery,
        ProtocolWorkflowDriver,
    },
    domains::{actions::DomStorageAction, command_output::CommandOutputPlan},
};

use super::params::{DomStorageId, RemoveItemParams, SetItemParams, StorageIdParams};

const FRAME_NOT_FOUND: &str = "Frame not found for the given storage id";

pub(crate) enum DomStorageCommandTaskStep {
    Workflow(PreparedCommandWorkflow),
    Complete(CommandOutputPlan),
}

pub(crate) struct PreparedDomStorageOperation {
    owner: CommandOwnerScope,
    storage_id: DomStorageId,
    operation: DomStorageOperation,
}

#[derive(Debug)]
enum DomStorageOperation {
    Clear,
    GetItems,
    RemoveItem { key: String },
    SetItem { key: String, value: String },
}

pub(crate) fn try_start_dom_storage_command_dispatch(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
) -> DomStorageCommandTaskStep {
    match cmd.parse_action::<DomStorageAction>() {
        Some(DomStorageAction::Enable) => {
            DomStorageCommandTaskStep::Complete(enable_command_output_plan(conn, cmd.session_id))
        }
        Some(DomStorageAction::Disable) => {
            DomStorageCommandTaskStep::Complete(disable_command_output_plan(conn, cmd.session_id))
        }
        Some(DomStorageAction::Clear) => {
            let Some((storage_id, operation)) =
                parse_storage_id_operation(cmd, DomStorageOperation::Clear)
            else {
                return invalid_params_step();
            };
            start_storage_operation(conn, cmd, storage_id, operation)
        }
        Some(DomStorageAction::GetDomStorageItems) => {
            let Some((storage_id, operation)) =
                parse_storage_id_operation(cmd, DomStorageOperation::GetItems)
            else {
                return invalid_params_step();
            };
            start_storage_operation(conn, cmd, storage_id, operation)
        }
        Some(DomStorageAction::RemoveDomStorageItem) => {
            let params: RemoveItemParams = match cmd.get_params() {
                Ok(Some(params)) => params,
                _ => return invalid_params_step(),
            };
            start_storage_operation(
                conn,
                cmd,
                params.storage_id,
                DomStorageOperation::RemoveItem { key: params.key },
            )
        }
        Some(DomStorageAction::SetDomStorageItem) => {
            let params: SetItemParams = match cmd.get_params() {
                Ok(Some(params)) => params,
                _ => return invalid_params_step(),
            };
            start_storage_operation(
                conn,
                cmd,
                params.storage_id,
                DomStorageOperation::SetItem {
                    key: params.key,
                    value: params.value,
                },
            )
        }
        None => {
            DomStorageCommandTaskStep::Complete(CommandOutputPlan::error(-32601, "UnknownMethod"))
        }
    }
}

fn parse_storage_id_operation(
    cmd: &Cmd<'_>,
    operation: DomStorageOperation,
) -> Option<(DomStorageId, DomStorageOperation)> {
    let params: StorageIdParams = cmd.get_params().ok()??;
    Some((params.storage_id, operation))
}

fn invalid_params_step() -> DomStorageCommandTaskStep {
    DomStorageCommandTaskStep::Complete(CommandOutputPlan::error(-32602, "InvalidParams"))
}

fn enable_command_output_plan(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
) -> CommandOutputPlan {
    if conn
        .target_devtools_session_state_for_session(session_id)
        .is_some_and(|state| state.dom_storage_session_state.is_enabled())
    {
        return CommandOutputPlan::success();
    }

    let handles = match storage_handles_for_session_owner(conn, session_id) {
        Ok(handles) => handles,
        Err(plan) => return plan,
    };
    let subscription = handles
        .web_storage_store
        .lock()
        .subscribe_mutations(WebStorageAreaKind::Local);
    handles
        .session_storage_store
        .lock()
        .add_mutation_subscription(WebStorageAreaKind::Session, &subscription);

    let enabled = conn.with_target_devtools_session_state_for_session_mut(session_id, |state| {
        state.dom_storage_session_state.enable(subscription);
    });
    if enabled.is_none() {
        return CommandOutputPlan::error(-32000, "DOMStorage is not available for this target");
    }
    CommandOutputPlan::success()
}

fn disable_command_output_plan(
    conn: &mut CdpConnection,
    session_id: Option<&str>,
) -> CommandOutputPlan {
    let disabled = conn.with_target_devtools_session_state_for_session_mut(session_id, |state| {
        state.dom_storage_session_state.disable();
    });
    if disabled.is_none() {
        return CommandOutputPlan::error(-32000, "DOMStorage is not available for this target");
    }
    CommandOutputPlan::success()
}

fn start_storage_operation(
    conn: &mut CdpConnection,
    cmd: &Cmd<'_>,
    storage_id: DomStorageId,
    operation: DomStorageOperation,
) -> DomStorageCommandTaskStep {
    let owner_scope = CommandOwnerScope::capture(conn, cmd.session_id);
    if let Err(plan) = storage_handles_for_owner(conn, &owner_scope) {
        return DomStorageCommandTaskStep::Complete(plan);
    }

    if let Some(storage_key) = conn
        .runtime_session_owner_initial_empty_document_storage_key(cmd.session_id)
        .map(|storage_key| storage_key.serialized_storage_key())
    {
        return DomStorageCommandTaskStep::Complete(complete_storage_operation(
            conn,
            &owner_scope,
            storage_id,
            operation,
            &storage_key,
        ));
    }

    if conn
        .loaded_page_mut_for_protocol_access_for_owner(&owner_scope)
        .is_ok()
    {
        let query = PreparedRuntimePageQuery::prepare(
            conn,
            &owner_scope,
            "DOMStorage storage key query failed",
            |page| page.start_document_storage_key_snapshot(),
            Page::finish_document_storage_key_snapshot,
        );
        let query = match query {
            Ok(query) => query,
            Err(error) => return DomStorageCommandTaskStep::Complete(storage_query_error(error)),
        };
        let operation = PreparedDomStorageOperation {
            owner: owner_scope,
            storage_id,
            operation,
        };
        let access = conn.protocol_state_access();
        return DomStorageCommandTaskStep::Workflow(
            PreparedCommandWorkflow::new(move |mut context| async move {
                let result = operation
                    .run(
                        query,
                        &mut ProtocolWorkflowDriver::new(access),
                        &mut context,
                    )
                    .await;
                match result {
                    Ok(plan) => context.complete(plan),
                    Err(error) => context.fail(storage_query_error(error)),
                }
            })
            .cancel_on_waiter_drop(),
        );
    }

    let Some(target_url) = conn.runtime_session_owner_target_url(cmd.session_id) else {
        return DomStorageCommandTaskStep::Complete(frame_not_found_plan());
    };
    let storage_key = match top_frame_storage_key_for_url(&target_url) {
        Ok(storage_key) => storage_key,
        Err(plan) => return DomStorageCommandTaskStep::Complete(plan),
    };
    DomStorageCommandTaskStep::Complete(complete_storage_operation(
        conn,
        &owner_scope,
        storage_id,
        operation,
        &storage_key,
    ))
}

impl PreparedDomStorageOperation {
    async fn run(
        self,
        query: PreparedRuntimePageQuery<String>,
        driver: &mut ProtocolWorkflowDriver,
        context: &mut CommandWorkflowContext,
    ) -> Result<CommandOutputPlan> {
        let mut storage_key = query
            .run(
                driver,
                context.context_mut().renderer_output_predecessor_mut(),
            )
            .await?;
        if !storage_id_matches_key(&self.storage_id, &storage_key) {
            let query =
                driver.with_state(|state| state.prepare_dom_storage_child_query(&self))??;
            let frames = query
                .run(
                    driver,
                    context.context_mut().renderer_output_predecessor_mut(),
                )
                .await?;
            let Some(child_storage_key) =
                matching_child_frame_storage_key(&self.storage_id, &frames)
            else {
                return Ok(frame_not_found_plan());
            };
            storage_key = child_storage_key;
        }
        context.with_state(driver, move |state, _| {
            state.finish_dom_storage_operation(self, storage_key)
        })
    }

    pub(crate) fn prepare_child_query(
        &self,
        conn: &mut CdpConnection,
    ) -> Result<PreparedRuntimePageQuery<Vec<ChildFrameTreeSnapshot>>> {
        PreparedRuntimePageQuery::prepare(
            conn,
            &self.owner,
            "DOMStorage child frame query failed",
            |page| page.start_child_frame_tree_snapshot(),
            Page::finish_child_frame_tree_snapshot,
        )
    }

    pub(crate) fn finish(self, conn: &CdpConnection, storage_key: &str) -> CommandOutputPlan {
        complete_storage_operation(
            conn,
            &self.owner,
            self.storage_id,
            self.operation,
            storage_key,
        )
    }
}

fn storage_query_error(error: anyhow::Error) -> CommandOutputPlan {
    if error.is::<crate::conn::NoDocumentLoaded>() {
        frame_not_found_plan()
    } else {
        CommandOutputPlan::error(-32000, format!("{error:#}"))
    }
}

fn complete_storage_operation(
    conn: &CdpConnection,
    owner: &CommandOwnerScope,
    storage_id: DomStorageId,
    operation: DomStorageOperation,
    storage_key: &str,
) -> CommandOutputPlan {
    if !storage_id_matches_key(&storage_id, storage_key) {
        return frame_not_found_plan();
    }
    let handles = match storage_handles_for_owner(conn, owner) {
        Ok(handles) => handles,
        Err(plan) => return plan,
    };
    let store = if storage_id.is_local_storage {
        handles.web_storage_store
    } else {
        handles.session_storage_store
    };
    let mut store = store.lock();

    match operation {
        DomStorageOperation::Clear => mutation_output_plan(store.try_clear(storage_key)),
        DomStorageOperation::GetItems => {
            let entries = store
                .sorted_keys(storage_key)
                .into_iter()
                .filter_map(|key| {
                    let value = store.get_item(storage_key, &key)?;
                    Some(vec![key, value])
                })
                .collect::<Vec<_>>();
            CommandOutputPlan::result(json!({ "entries": entries }))
        }
        DomStorageOperation::RemoveItem { key } => {
            mutation_output_plan(store.try_remove_item(storage_key, &key))
        }
        DomStorageOperation::SetItem { key, value } => {
            mutation_output_plan(store.try_set_item(storage_key, &key, &value))
        }
    }
}

fn mutation_output_plan<E>(result: Result<bool, E>) -> CommandOutputPlan
where
    E: std::fmt::Display,
{
    match result {
        Ok(_) => CommandOutputPlan::success(),
        Err(error) => CommandOutputPlan::error(-32000, error.to_string()),
    }
}

fn storage_handles_for_session_owner(
    conn: &CdpConnection,
    session_id: Option<&str>,
) -> Result<BrowserContextPageStorageHandles, CommandOutputPlan> {
    let owner = CommandOwnerScope::capture(conn, session_id);
    storage_handles_for_owner(conn, &owner)
}

fn storage_handles_for_owner(
    conn: &CdpConnection,
    owner: &CommandOwnerScope,
) -> Result<BrowserContextPageStorageHandles, CommandOutputPlan> {
    let Some((browser_context_id, Some(target_id))) = conn.target_owner_identity_for_owner(owner)
    else {
        return Err(CommandOutputPlan::error(
            -32000,
            "DOMStorage is not available for this target",
        ));
    };
    conn.browser_context_by_id(&browser_context_id)
        .and_then(|context| context.page_storage_handles_for_target(&target_id))
        .ok_or_else(|| {
            CommandOutputPlan::error(-32000, "DOMStorage is not available for this target")
        })
}

fn matching_child_frame_storage_key(
    storage_id: &DomStorageId,
    frames: &[ChildFrameTreeSnapshot],
) -> Option<String> {
    for frame in frames {
        if storage_id_matches_key(storage_id, &frame.storage_key) {
            return Some(frame.storage_key.clone());
        }
        if let Some(storage_key) = matching_child_frame_storage_key(storage_id, &frame.child_frames)
        {
            return Some(storage_key);
        }
    }
    None
}

fn storage_id_matches_key(storage_id: &DomStorageId, storage_key: &str) -> bool {
    let Some(parsed) = moli_storage_key::deserialize_serialized_storage_key(storage_key) else {
        return false;
    };
    if moli_storage_key::serialized_storage_key_has_opaque_origin(storage_key) {
        return false;
    }
    if let Some(expected_storage_key) = storage_id
        .storage_key
        .as_deref()
        .filter(|storage_key| !storage_key.is_empty())
    {
        return expected_storage_key == storage_key;
    }
    storage_id
        .security_origin
        .as_deref()
        .is_some_and(|security_origin| security_origin == parsed.origin())
}

fn top_frame_storage_key_for_url(target_url: &str) -> Result<String, CommandOutputPlan> {
    let url =
        Url::parse(target_url).map_err(|_| CommandOutputPlan::error(-32602, "InvalidParams"))?;
    Ok(moli_storage_key::MoliStorageKey::first_party_from_url(
        &url,
        moli_storage_key::url_needs_opaque_nonce(&url)
            .then_some(moli_storage_key::OpaqueOriginNonce::new(0)),
    )
    .serialized_storage_key())
}

fn frame_not_found_plan() -> CommandOutputPlan {
    CommandOutputPlan::error(-32000, FRAME_NOT_FOUND)
}
