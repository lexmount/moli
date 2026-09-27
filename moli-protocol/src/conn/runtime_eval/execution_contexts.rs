use super::*;

impl CdpConnection {
    pub(crate) async fn runtime_realm_inventory_for_owner_async(
        &mut self,
        owner: &CommandOwnerScope,
    ) -> Result<Vec<RuntimeExecutionContextEvent>, String> {
        let target_id = self
            .target_owner_identity_for_owner(owner)
            .and_then(|(_, target_id)| target_id);
        let page = self.runtime_session_owner_page_mut_for_owner(owner)?;
        let target_id = target_id.as_deref();
        let devtools_target_id = target_id.map(DevToolsTargetId::from);
        let realms = page
            .runtime_realm_inventory_async()
            .await
            .map_err(|error| format!("runtime realm inventory failed: {error}"))?;
        realms
            .into_iter()
            .map(|realm| {
                runtime_realm_info_to_execution_context_event(
                    realm,
                    target_id,
                    devtools_target_id.clone(),
                )
            })
            .collect()
    }

    pub async fn runtime_default_execution_context_id_async(
        &mut self,
    ) -> Result<Option<i64>, String> {
        self.runtime_default_execution_context_id_for_session_owner_async(None)
            .await
    }

    pub async fn runtime_default_execution_context_id_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
    ) -> Result<Option<i64>, String> {
        let page = self.runtime_session_owner_page_mut(session_id)?;
        page.default_execution_context_id_async()
            .await
            .map_err(|error| format!("runtime default execution context lookup failed: {error}"))
    }

    pub(crate) async fn runtime_default_or_initial_execution_context_id_for_owner_async(
        &mut self,
        owner: &CommandOwnerScope,
    ) -> Result<Option<i64>, String> {
        let page = self
            .runtime_session_owner_slot_mut_for_owner(owner)?
            .loaded_page_mut()
            .ok_or_else(|| "NoDocumentLoaded".to_owned())?;
        page.default_or_initial_execution_context_id_async()
            .await
            .map_err(|error| format!("runtime default execution context lookup failed: {error}"))
    }

    pub(crate) async fn runtime_ensure_isolated_world_for_owner_async(
        &mut self,
        owner: &CommandOwnerScope,
        frame_id: Option<&str>,
        world_name: &str,
    ) -> Result<i64, String> {
        let owner_target_id = self
            .target_owner_identity_for_owner(owner)
            .and_then(|(_, target_id)| target_id);
        let page = self.runtime_session_owner_page_mut_for_owner(owner)?;
        let result = if let Some(frame_id) = frame_id
            && owner_target_id.as_deref() != Some(frame_id)
        {
            page.create_isolated_world_for_frame_async(frame_id, world_name, false)
                .await
        } else {
            page.create_isolated_world_async(world_name, false).await
        };
        result.map_err(|error| format!("runtime isolated world creation failed: {error}"))
    }

    pub async fn has_isolated_execution_context_id_async(
        &mut self,
        execution_context_id: i64,
    ) -> Result<bool, String> {
        self.has_isolated_execution_context_id_for_session_owner_async(None, execution_context_id)
            .await
    }

    pub async fn has_isolated_execution_context_id_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
        execution_context_id: i64,
    ) -> Result<bool, String> {
        let page = self.runtime_session_owner_page_mut(session_id)?;
        page.has_isolated_execution_context_id_async(execution_context_id)
            .await
            .map_err(|error| format!("runtime isolated context lookup failed: {error}"))
    }

    pub async fn has_child_default_execution_context_id_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
        execution_context_id: i64,
    ) -> Result<bool, String> {
        let page = self.runtime_session_owner_page_mut(session_id)?;
        page.child_frame_id_for_default_execution_context_id_async(execution_context_id)
            .await
            .map(|frame_id| frame_id.is_some())
            .map_err(|error| format!("runtime child default context lookup failed: {error}"))
    }

    pub async fn child_default_execution_context_id_for_frame_id_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
        frame_id: &str,
    ) -> Result<Option<i64>, String> {
        let owner = CommandOwnerScope::capture(self, session_id);
        self.child_default_execution_context_id_for_frame_id_for_owner_async(&owner, frame_id)
            .await
    }

    pub(crate) async fn child_default_execution_context_id_for_frame_id_for_owner_async(
        &mut self,
        owner: &CommandOwnerScope,
        frame_id: &str,
    ) -> Result<Option<i64>, String> {
        let page = self.runtime_session_owner_page_mut_for_owner(owner)?;
        page.child_default_execution_context_id_for_frame_id_async(frame_id)
            .await
            .map_err(|error| format!("runtime child default context lookup failed: {error}"))
    }

    pub(crate) fn start_child_default_execution_context_lookup_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        execution_context_id: i64,
    ) -> Result<PendingRuntimeChildDefaultContextLookupDispatch, String> {
        let page = self.runtime_session_owner_page_mut_for_owner(owner)?;
        let pending = page
            .start_child_frame_id_for_default_execution_context_id(execution_context_id)
            .map_err(|error| format!("runtime child default context lookup failed: {error}"))?;
        Ok(PendingRuntimeChildDefaultContextLookupDispatch {
            owner: owner.clone(),
            pending,
        })
    }

    pub(crate) fn complete_child_default_execution_context_lookup(
        &mut self,
        completed: CompletedRuntimeChildDefaultContextLookupDispatch,
    ) -> Result<bool, String> {
        let page = self.runtime_session_owner_page_mut_for_owner(&completed.owner)?;
        page.finish_child_frame_id_for_default_execution_context_id(completed.completion)
            .map(|frame_id| frame_id.is_some())
            .map_err(|error| format!("runtime child default context lookup failed: {error}"))
    }

    pub async fn inspector_execution_context_id_for_isolated_context_async(
        &mut self,
        execution_context_id: i64,
    ) -> Result<Option<i64>, String> {
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
    ) -> Result<Option<i64>, String> {
        let page = self.runtime_session_owner_page_mut(session_id)?;
        page.ensure_isolated_worlds_attached_to_inspector_async()
            .await
            .map_err(|error| {
                format!("runtime isolated inspector context attachment failed: {error}")
            })?;
        page.inspector_execution_context_id_for_isolated_context_async(execution_context_id)
            .await
            .map_err(|error| format!("runtime isolated inspector context lookup failed: {error}"))
    }

    pub async fn isolated_execution_context_id_for_inspector_context_async(
        &mut self,
        execution_context_id: i64,
    ) -> Result<Option<i64>, String> {
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
    ) -> Result<Option<i64>, String> {
        let page = self.runtime_session_owner_page_mut(session_id)?;
        page.ensure_isolated_worlds_attached_to_inspector_async()
            .await
            .map_err(|error| {
                format!("runtime isolated compatibility context attachment failed: {error}")
            })?;
        page.isolated_execution_context_id_for_inspector_context_async(execution_context_id)
            .await
            .map_err(|error| {
                format!("runtime isolated compatibility context lookup failed: {error}")
            })
    }

    pub async fn evaluate_runtime_expression_in_execution_context_with_await_async(
        &mut self,
        execution_context_id: i64,
        expression: &str,
        await_promise: bool,
    ) -> Result<Value, String> {
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
    ) -> Result<Value, String> {
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
    ) -> Result<Value, String> {
        let payload = {
            let page = self.runtime_session_owner_page_mut(session_id)?;
            page.evaluate_runtime_expression_in_execution_context_without_navigation_follow_with_await_async(
                execution_context_id,
                expression,
                await_promise,
            )
            .await
            .map_err(|error| format!("runtime evaluation failed: {error}"))?
        };
        self.ingest_runtime_session_owner_output_updates(session_id);
        Ok(payload)
    }

    pub async fn install_runtime_binding_async(
        &mut self,
        name: &str,
        execution_context_name: Option<&str>,
        execution_context_id: Option<i64>,
    ) -> Result<(), String> {
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
    ) -> Result<(), String> {
        let page = self.runtime_session_owner_page_mut(session_id)?;
        page.install_runtime_binding_async(name, execution_context_name, execution_context_id)
            .await
            .map_err(|error| format!("runtime binding install failed: {error}"))
    }

    pub(crate) fn start_install_runtime_binding_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        name: &str,
        execution_context_name: Option<&str>,
        execution_context_id: Option<i64>,
    ) -> Result<PendingRuntimeBindingPageCommandDispatch, String> {
        let page = self.runtime_session_owner_page_mut_for_owner(owner)?;
        let pending = page
            .start_install_runtime_binding(name, execution_context_name, execution_context_id)
            .map_err(|error| format!("runtime binding install failed: {error}"))?;
        Ok(PendingRuntimeBindingPageCommandDispatch {
            owner: owner.clone(),
            operation: "runtime binding install",
            pending,
        })
    }

    pub(crate) fn start_apply_stored_runtime_bindings_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
    ) -> Result<PendingRuntimeBindingPageCommandDispatch, String> {
        let stored_runtime_bindings = self.target_runtime_bindings_for_renderer_owner(owner);
        let session_runtime_bindings =
            self.target_runtime_bindings_for_current_inspector_owner(owner);
        let inspector_session_id =
            self.target_renderer_runtime_inspector_session_id_for_owner(owner);
        let page = self.runtime_session_owner_page_mut_for_owner(owner)?;
        let pending = page
            .start_set_runtime_binding_state(
                inspector_session_id,
                &stored_runtime_bindings,
                &session_runtime_bindings,
            )
            .map_err(|error| format!("runtime binding state update failed: {error}"))?;
        Ok(PendingRuntimeBindingPageCommandDispatch {
            owner: owner.clone(),
            operation: "runtime binding state update",
            pending,
        })
    }

    pub(crate) async fn apply_runtime_binding_state_for_owner_async(
        &mut self,
        owner: &CommandOwnerScope,
    ) -> Result<(), String> {
        let stored_runtime_bindings = self.target_runtime_bindings_for_renderer_owner(owner);
        let session_runtime_bindings =
            self.target_runtime_bindings_for_current_inspector_owner(owner);
        let inspector_session_id =
            self.target_renderer_runtime_inspector_session_id_for_owner(owner);
        let page = self.runtime_session_owner_page_mut_for_owner(owner)?;
        page.set_runtime_binding_state_async(
            inspector_session_id,
            &stored_runtime_bindings,
            &session_runtime_bindings,
        )
        .await
        .map_err(|error| format!("runtime binding state update failed: {error}"))
    }

    pub async fn remove_runtime_binding_async(&mut self, name: &str) -> Result<(), String> {
        self.remove_runtime_binding_for_session_owner_async(None, name)
            .await
    }

    pub async fn remove_runtime_binding_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
        name: &str,
    ) -> Result<(), String> {
        let page = self.runtime_session_owner_page_mut(session_id)?;
        page.remove_runtime_binding_async(name)
            .await
            .map_err(|error| format!("runtime binding removal failed: {error}"))
    }

    pub(crate) fn start_remove_runtime_binding_for_owner(
        &mut self,
        owner: &CommandOwnerScope,
        name: &str,
    ) -> Result<PendingRuntimeBindingPageCommandDispatch, String> {
        let page = self.runtime_session_owner_page_mut_for_owner(owner)?;
        let pending = page
            .start_remove_runtime_binding(name)
            .map_err(|error| format!("runtime binding removal failed: {error}"))?;
        Ok(PendingRuntimeBindingPageCommandDispatch {
            owner: owner.clone(),
            operation: "runtime binding removal",
            pending,
        })
    }

    pub(crate) fn complete_runtime_binding_page_command(
        &mut self,
        completed: CompletedRuntimeBindingPageCommandDispatch,
    ) -> Result<(), String> {
        let page = self.runtime_session_owner_page_mut_for_owner(&completed.owner)?;
        page.finish_unit_runtime_page_command(completed.completion, completed.operation)
            .map_err(|error| format!("{} failed: {error}", completed.operation))
    }

    pub async fn remove_default_runtime_binding_async(&mut self, name: &str) -> Result<(), String> {
        self.remove_default_runtime_binding_for_session_owner_async(None, name)
            .await
    }

    pub async fn remove_default_runtime_binding_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
        name: &str,
    ) -> Result<(), String> {
        let page = self.runtime_session_owner_page_mut(session_id)?;
        page.remove_default_runtime_binding_async(name)
            .await
            .map_err(|error| format!("runtime default binding removal failed: {error}"))
    }

    pub(crate) async fn detach_runtime_inspector_session_for_session_owner_async(
        &mut self,
        session_id: Option<&str>,
    ) -> Result<bool, String> {
        let inspector_session_id =
            self.target_renderer_runtime_inspector_session_id_for_session(session_id);
        let page = self.runtime_session_owner_page_mut(session_id)?;
        page.detach_runtime_inspector_session_async(inspector_session_id.as_deref())
            .await
            .map_err(|error| format!("runtime inspector session detach failed: {error}"))
    }
}
