use anyhow::Result;

use super::*;
use anyhow::Context as _;

pub(crate) struct PreparedRuntimeRealmInventory {
    scope: RuntimePageScope,
    pending: Option<moli_core::page::PendingPageCommand>,
}

impl PreparedRuntimeRealmInventory {
    pub(crate) fn scope(&self) -> RuntimePageScope {
        self.scope.clone()
    }
    pub(crate) fn prepare(conn: &mut CdpConnection, owner: &CommandOwnerScope) -> Result<Self> {
        let scope = RuntimePageScope::capture(conn, owner)?;
        let pending = conn
            .loaded_page_mut_for_protocol_access_for_owner(scope.owner())?
            .start_runtime_realm_inventory()?;
        Ok(Self {
            scope,
            pending: Some(pending),
        })
    }

    pub(crate) async fn run(
        self,
        driver: &mut crate::conn::ProtocolWorkflowDriver<'_>,
    ) -> Result<Vec<RuntimeExecutionContextEvent>> {
        self.run_collecting_renderer_predecessor(driver, &mut None)
            .await
    }

    pub(crate) async fn run_collecting_renderer_predecessor(
        mut self,
        driver: &mut crate::conn::ProtocolWorkflowDriver<'_>,
        predecessor: &mut Option<moli_core::RendererOutputFence>,
    ) -> Result<Vec<RuntimeExecutionContextEvent>> {
        let pending = self
            .pending
            .take()
            .expect("one inventory query owns one renderer wait");
        let completed = pending
            .wait()
            .await
            .context("runtime realm inventory failed")?;
        if let Some(fence) = completed.renderer_output_predecessor() {
            fence.merge_into_same_stream_tail(predecessor);
        }
        driver
            .with_state(move |state| state.finish_runtime_realm_inventory(self, completed))
            .await?
    }

    pub(crate) fn finish(
        self,
        conn: &mut CdpConnection,
        completed: moli_core::page::CompletedPageCommand,
    ) -> Result<Vec<RuntimeExecutionContextEvent>> {
        let realms = self
            .scope
            .page_mut(conn)?
            .finish_runtime_realm_inventory(completed)
            .context("runtime realm inventory failed")?;
        let target_id = self.scope.target_id();
        let devtools_target_id = target_id.map(DevToolsTargetId::from);
        Ok(realms
            .into_iter()
            .map(|realm| {
                runtime_realm_info_to_execution_context_event(
                    realm,
                    target_id,
                    devtools_target_id.clone(),
                )
            })
            .collect())
    }
}

#[cfg(test)]
pub(crate) async fn runtime_realm_inventory_using(
    driver: &mut crate::conn::ProtocolWorkflowDriver<'_>,
    owner: CommandOwnerScope,
) -> Result<Vec<RuntimeExecutionContextEvent>> {
    driver
        .with_state(move |state| state.prepare_runtime_realm_inventory(&owner))
        .await??
        .run(driver)
        .await
}

impl CdpConnection {
    #[cfg(test)]
    pub(crate) async fn runtime_realm_inventory_for_owner_async(
        &mut self,
        owner: &CommandOwnerScope,
    ) -> Result<Vec<RuntimeExecutionContextEvent>> {
        crate::conn::runtime_realm_inventory_using(
            &mut crate::conn::ProtocolWorkflowDriver::inline(self),
            owner.clone(),
        )
        .await
    }

    pub async fn runtime_default_execution_context_id_async(&mut self) -> Result<Option<i64>> {
        self.runtime_default_execution_context_id_for_session_owner_async(None)
            .await
    }

    pub async fn runtime_default_execution_context_id_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
    ) -> Result<Option<i64>> {
        let page = self.loaded_page_mut_for_protocol_access(session_id)?;
        page.default_execution_context_id_async()
            .await
            .context("runtime default execution context lookup failed")
    }

    pub async fn has_isolated_execution_context_id_async(
        &mut self,
        execution_context_id: i64,
    ) -> Result<bool> {
        self.has_isolated_execution_context_id_for_session_owner_async(None, execution_context_id)
            .await
    }

    pub async fn has_isolated_execution_context_id_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
        execution_context_id: i64,
    ) -> Result<bool> {
        let page = self.loaded_page_mut_for_protocol_access(session_id)?;
        page.has_isolated_execution_context_id_async(execution_context_id)
            .await
            .context("runtime isolated context lookup failed")
    }

    pub async fn has_child_default_execution_context_id_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
        execution_context_id: i64,
    ) -> Result<bool> {
        let page = self.loaded_page_mut_for_protocol_access(session_id)?;
        page.child_frame_id_for_default_execution_context_id_async(execution_context_id)
            .await
            .map(|frame_id| frame_id.is_some())
            .context("runtime child default context lookup failed")
    }

    pub async fn child_default_execution_context_id_for_frame_id_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
        frame_id: &str,
    ) -> Result<Option<i64>> {
        let owner = CommandOwnerScope::capture(self, session_id);
        self.child_default_execution_context_id_for_frame_id_for_owner_async(&owner, frame_id)
            .await
    }

    pub(crate) async fn child_default_execution_context_id_for_frame_id_for_owner_async(
        &mut self,
        owner: &CommandOwnerScope,
        frame_id: &str,
    ) -> Result<Option<i64>> {
        let owner = owner.clone();
        let frame_id = frame_id.to_owned();
        crate::conn::runtime_page_query_using(
            &mut crate::conn::ProtocolWorkflowDriver::inline(self),
            move |state| state.prepare_runtime_child_context_query(&owner, frame_id),
            &mut None,
        )
        .await
    }

    pub(crate) fn start_child_default_execution_context_lookup_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        execution_context_id: i64,
    ) -> Result<PendingRuntimeChildDefaultContextLookupDispatch> {
        let page = self.loaded_page_mut_for_protocol_access_for_owner(owner)?;
        let pending = page
            .start_child_frame_id_for_default_execution_context_id(execution_context_id)
            .context("runtime child default context lookup failed")?;
        Ok(PendingRuntimeChildDefaultContextLookupDispatch {
            owner: owner.clone(),
            pending,
        })
    }

    pub(crate) fn complete_child_default_execution_context_lookup(
        &mut self,
        completed: CompletedRuntimeChildDefaultContextLookupDispatch,
    ) -> Result<bool> {
        let page = self.loaded_page_mut_for_protocol_access_for_owner(&completed.owner)?;
        page.finish_child_frame_id_for_default_execution_context_id(completed.completion)
            .map(|frame_id| frame_id.is_some())
            .context("runtime child default context lookup failed")
    }

    pub async fn inspector_execution_context_id_for_isolated_context_async(
        &mut self,
        execution_context_id: i64,
    ) -> Result<Option<i64>> {
        self.inspector_execution_context_id_for_isolated_context_for_session_owner_async(
            None,
            execution_context_id,
        )
        .await
    }

    pub async fn inspector_execution_context_id_for_isolated_context_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
        execution_context_id: i64,
    ) -> Result<Option<i64>> {
        let page = self.loaded_page_mut_for_protocol_access(session_id)?;
        page.ensure_isolated_worlds_attached_to_inspector_async()
            .await
            .context("runtime isolated inspector context attachment failed")?;
        page.inspector_execution_context_id_for_isolated_context_async(execution_context_id)
            .await
            .context("runtime isolated inspector context lookup failed")
    }

    pub async fn isolated_execution_context_id_for_inspector_context_async(
        &mut self,
        execution_context_id: i64,
    ) -> Result<Option<i64>> {
        self.isolated_execution_context_id_for_inspector_context_for_session_owner_async(
            None,
            execution_context_id,
        )
        .await
    }

    pub async fn isolated_execution_context_id_for_inspector_context_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
        execution_context_id: i64,
    ) -> Result<Option<i64>> {
        let page = self.loaded_page_mut_for_protocol_access(session_id)?;
        page.ensure_isolated_worlds_attached_to_inspector_async()
            .await
            .context("runtime isolated compatibility context attachment failed")?;
        page.isolated_execution_context_id_for_inspector_context_async(execution_context_id)
            .await
            .context("runtime isolated compatibility context lookup failed")
    }

    pub async fn evaluate_runtime_expression_in_execution_context_with_await_async(
        &mut self,
        execution_context_id: i64,
        expression: &str,
        await_promise: bool,
    ) -> Result<Value> {
        self.evaluate_runtime_expression_in_execution_context_for_session_owner_async(
            None,
            execution_context_id,
            expression,
            await_promise,
        )
        .await
    }

    pub async fn evaluate_runtime_expression_in_execution_context_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
        execution_context_id: i64,
        expression: &str,
        await_promise: bool,
    ) -> Result<Value> {
        self.evaluate_runtime_expression_in_execution_context_for_session_owner_once_async(
            session_id,
            execution_context_id,
            expression,
            await_promise,
        )
        .await
    }

    pub(super) async fn evaluate_runtime_expression_in_execution_context_for_session_owner_once_async(
        &mut self,
        session_id: Option<&str>,
        execution_context_id: i64,
        expression: &str,
        await_promise: bool,
    ) -> Result<Value> {
        let payload = {
            let page = self.loaded_page_mut_for_protocol_access(session_id)?;
            page.evaluate_runtime_expression_in_execution_context_without_navigation_follow_with_await_async(
                execution_context_id,
                expression,
                await_promise,
            )
            .await
            .context("runtime evaluation failed")?
        };
        self.ingest_runtime_session_owner_output_updates(session_id);
        Ok(payload)
    }

    pub async fn install_runtime_binding_async(
        &mut self,
        name: &str,
        execution_context_name: Option<&str>,
        execution_context_id: Option<i64>,
    ) -> Result<()> {
        self.install_runtime_binding_for_session_owner_async(
            None,
            name,
            execution_context_name,
            execution_context_id,
        )
        .await
    }

    pub async fn install_runtime_binding_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
        name: &str,
        execution_context_name: Option<&str>,
        execution_context_id: Option<i64>,
    ) -> Result<()> {
        let page = self.loaded_page_mut_for_protocol_access(session_id)?;
        page.install_runtime_binding_async(name, execution_context_name, execution_context_id)
            .await
            .context("runtime binding install failed")
    }

    pub(crate) fn start_install_runtime_binding_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        name: &str,
        execution_context_name: Option<&str>,
        execution_context_id: Option<i64>,
    ) -> Result<PendingRuntimeBindingPageCommandDispatch> {
        let page = self.loaded_page_mut_for_protocol_access_for_owner(owner)?;
        let pending = page
            .start_install_runtime_binding(name, execution_context_name, execution_context_id)
            .context("runtime binding install failed")?;
        Ok(PendingRuntimeBindingPageCommandDispatch {
            owner: owner.clone(),
            operation: "runtime binding install",
            pending,
        })
    }

    pub(crate) fn start_apply_stored_runtime_bindings_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
    ) -> Result<PendingRuntimeBindingPageCommandDispatch> {
        let stored_runtime_bindings = self.target_runtime_bindings_for_renderer_owner(owner);
        let session_runtime_bindings =
            self.target_runtime_bindings_for_current_inspector_owner(owner);
        let inspector_session_id =
            self.target_renderer_runtime_inspector_session_id_for_owner(owner);
        let page = self.loaded_page_mut_for_protocol_access_for_owner(owner)?;
        let pending = page
            .start_set_runtime_binding_state(
                inspector_session_id,
                &stored_runtime_bindings,
                &session_runtime_bindings,
            )
            .context("runtime binding state update failed")?;
        Ok(PendingRuntimeBindingPageCommandDispatch {
            owner: owner.clone(),
            operation: "runtime binding state update",
            pending,
        })
    }

    #[cfg(test)]
    pub(crate) async fn apply_runtime_binding_state_for_owner_async(
        &mut self,
        owner: &CommandOwnerScope,
    ) -> Result<()> {
        let stored_runtime_bindings = self.target_runtime_bindings_for_renderer_owner(owner);
        let session_runtime_bindings =
            self.target_runtime_bindings_for_current_inspector_owner(owner);
        let inspector_session_id =
            self.target_renderer_runtime_inspector_session_id_for_owner(owner);
        let page = self.loaded_page_mut_for_protocol_access_for_owner(owner)?;
        page.set_runtime_binding_state_async(
            inspector_session_id,
            &stored_runtime_bindings,
            &session_runtime_bindings,
        )
        .await
        .context("runtime binding state update failed")
    }

    pub async fn remove_runtime_binding_async(&mut self, name: &str) -> Result<()> {
        self.remove_runtime_binding_for_session_owner_async(None, name)
            .await
    }

    pub async fn remove_runtime_binding_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
        name: &str,
    ) -> Result<()> {
        let page = self.loaded_page_mut_for_protocol_access(session_id)?;
        page.remove_runtime_binding_async(name)
            .await
            .context("runtime binding removal failed")
    }

    pub(crate) fn start_remove_runtime_binding_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        name: &str,
    ) -> Result<PendingRuntimeBindingPageCommandDispatch> {
        let page = self.loaded_page_mut_for_protocol_access_for_owner(owner)?;
        let pending = page
            .start_remove_runtime_binding(name)
            .context("runtime binding removal failed")?;
        Ok(PendingRuntimeBindingPageCommandDispatch {
            owner: owner.clone(),
            operation: "runtime binding removal",
            pending,
        })
    }

    pub(crate) fn complete_runtime_binding_page_command(
        &mut self,
        completed: CompletedRuntimeBindingPageCommandDispatch,
    ) -> Result<()> {
        let page = self.loaded_page_mut_for_protocol_access_for_owner(&completed.owner)?;
        page.finish_unit_runtime_page_command(completed.completion, completed.operation)
            .with_context(|| format!("{} failed", completed.operation))
    }

    pub async fn remove_default_runtime_binding_async(&mut self, name: &str) -> Result<()> {
        self.remove_default_runtime_binding_for_session_owner_async(None, name)
            .await
    }

    pub async fn remove_default_runtime_binding_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
        name: &str,
    ) -> Result<()> {
        let page = self.loaded_page_mut_for_protocol_access(session_id)?;
        page.remove_default_runtime_binding_async(name)
            .await
            .context("runtime default binding removal failed")
    }

    pub(crate) fn detach_runtime_inspector_session_for_session_owner(
        &mut self,
        session_id: Option<&str>,
    ) -> Result<bool> {
        let inspector_session_id =
            self.target_renderer_runtime_inspector_session_id_for_session(session_id);
        let page = self.loaded_page_mut_for_protocol_access(session_id)?;
        page.detach_runtime_inspector_session(inspector_session_id.as_deref())
            .map_err(|error| error.context("runtime inspector session detach failed"))
    }
}
