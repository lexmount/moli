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
        driver: &mut crate::conn::ProtocolWorkflowDriver,
    ) -> Result<Vec<RuntimeExecutionContextEvent>> {
        self.run_collecting_renderer_predecessor(driver, &mut None)
            .await
    }

    pub(crate) async fn run_collecting_renderer_predecessor(
        mut self,
        driver: &mut crate::conn::ProtocolWorkflowDriver,
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
        driver.with_state(move |state| state.finish_runtime_realm_inventory(self, completed))?
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
    driver: &mut crate::conn::ProtocolWorkflowDriver,
    owner: CommandOwnerScope,
) -> Result<Vec<RuntimeExecutionContextEvent>> {
    driver
        .with_state(move |state| state.prepare_runtime_realm_inventory(&owner))??
        .run(driver)
        .await
}

impl CdpConnection {
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

#[cfg(test)]
mod completion_tests {
    use super::*;
    use crate::conn::{BrowserContext, NoDocumentLoaded, ProtocolWorkflowDriver};
    use crate::testing::TestContext;

    async fn fixture() -> TestContext {
        let mut context = TestContext::new();
        let mut browser = BrowserContext::new("BID-helper-workflow".into());
        browser.set_active_target_id("TID-helper-workflow");
        browser.attach_active_session("SID-helper-workflow");
        context.with_fixture(|conn| conn.install_browser_context_fixture_for_test(browser));
        context
            .install_navigation_fixture_for_session_owner(
                "about:blank",
                Some("SID-helper-workflow"),
            )
            .await;
        context
    }

    #[tokio::test]
    async fn realm_inventory_rejects_a_completed_query_after_its_binding_is_reused() {
        let context = fixture().await;
        let command_owner = CommandOwnerScope::for_session("SID-helper-workflow");
        let mut inventory = context
            .with_fixture(|conn| PreparedRuntimeRealmInventory::prepare(conn, &command_owner))
            .unwrap();
        let completed = inventory.pending.take().unwrap().wait().await.unwrap();
        let route =
            context.with_fixture(|conn| conn.session_route(Some("SID-helper-workflow")).unwrap());
        context.with_fixture(|conn| {
            conn.register_session_route_for_test("SID-helper-workflow", route)
        });
        let error = context
            .with_fixture(|conn| inventory.finish(conn, completed))
            .unwrap_err();
        assert!(error.downcast_ref::<NoDocumentLoaded>().is_some());
        let mut driver = ProtocolWorkflowDriver::new(context.handle());
        let realms = crate::conn::runtime_realm_inventory_using(&mut driver, command_owner)
            .await
            .unwrap();
        assert!(!realms.is_empty());
    }
}
