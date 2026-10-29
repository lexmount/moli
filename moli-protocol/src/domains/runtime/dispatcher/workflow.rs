use super::*;
use crate::conn::{
    CommandWorkflowContext, PreparedRuntimePageQuery, ProtocolWorkflowDriver, ProtocolWorkflowState,
};
use futures_util::TryFutureExt as _;

/// Ordinary Runtime business flow. State access stays finite, and each Native
/// query retains its own Page authority while this local workflow can await.
pub(in crate::domains::runtime) struct RuntimeWorkflow {
    pub(in crate::domains::runtime) driver: ProtocolWorkflowDriver,
    pub(in crate::domains::runtime) context: CommandWorkflowContext,
    command_id: u64,
    pub(in crate::domains::runtime) scope: Option<crate::conn::RuntimePageScope>,
    inventory_scope: Option<crate::conn::RuntimePageScope>,
}

pub(in crate::domains::runtime) fn workflow_error(error: impl std::fmt::Display) -> DevToolsError {
    DevToolsError::new(DevToolsErrorKind::Internal, format!("{error:#}"))
}

impl RuntimeWorkflow {
    pub(in crate::domains::runtime) fn capture_scope(
        conn: &CdpConnection,
        command: &AutomationCommand,
    ) -> Result<Option<crate::conn::RuntimePageScope>, DevToolsError> {
        if matches!(command, AutomationCommand::TerminateExecution(_)) {
            return Ok(None);
        }
        let Some(target) = command.context().target_id.as_ref() else {
            return Ok(None);
        };
        let route = conn
            .target_session_route_for_target_id(target.as_str())
            .or_else(|| conn.target_session_route_for_child_frame_id(target.as_str()));
        match route {
            Some(route @ CdpSessionRoute::PageTarget { .. }) => {
                crate::conn::RuntimePageScope::capture(conn, &CommandOwnerScope::for_route(route))
                    .map(Some)
                    .map_err(workflow_error)
            }
            _ => Ok(None),
        }
    }

    pub(in crate::domains::runtime) fn pin_inventory_scope(&mut self) {
        if self.scope.is_none() {
            self.scope = self.inventory_scope.clone();
        }
    }

    fn state_raw<T: 'static>(
        &mut self,
        operation: impl for<'state> FnOnce(ProtocolWorkflowState<'state>) -> T,
    ) -> Result<T> {
        let scope = self.scope.as_ref();
        self.context.with_state(&mut self.driver, |state, _| {
            state.with_runtime_scope(scope, operation)
        })?
    }

    pub(in crate::domains::runtime) async fn dom(
        &mut self,
        mut command: AutomationCommand,
    ) -> Result<AutomationResult, DevToolsError> {
        let context = command.context();
        let target_id = context.target_id.clone();
        let owner = self
            .state(|state| state.runtime_dom_query_owner(context))?
            .ok_or_else(|| {
                DevToolsError::new(DevToolsErrorKind::NoSuchTarget, MissingRuntimeTarget)
            })?;
        if let AutomationCommand::ResolveNode(resolve) = &mut command
            && resolve.execution_context_id.is_none()
            && let Some(target_id) = target_id
        {
            let id = target_id.as_str().to_owned();
            if self
                .state(move |state| state.runtime_target_route(&id, false))?
                .is_none()
            {
                let lookup_owner = owner.clone();
                let frame_id = target_id.into_string();
                resolve.execution_context_id = Some(
                    self.query(move |state| {
                        state.prepare_runtime_child_context_query(&lookup_owner, frame_id)
                    })
                    .await?
                    .ok_or_else(|| {
                        DevToolsError::new(DevToolsErrorKind::Internal, MissingFrameRuntimeContext)
                    })?,
                );
            }
        }
        let pending =
            self.state(move |state| state.prepare_runtime_dom_query(owner, command))??;
        pending
            .run(
                &mut self.driver,
                self.context.context_mut().renderer_output_predecessor_mut(),
            )
            .await
    }
    pub(in crate::domains::runtime) async fn execute(
        &mut self,
        mut command: AutomationCommand,
    ) -> Result<AutomationResult, DevToolsError> {
        match command {
            AutomationCommand::GetRealms(command) => {
                return execute_devtools_get_realms_command_using(self, command).await;
            }
            AutomationCommand::ReleaseObjects(command) => {
                return execute_devtools_release_objects_command_using(self, command).await;
            }
            AutomationCommand::LocateNodes(command) => {
                return Box::pin(execute_devtools_locate_nodes_command_using(self, command)).await;
            }
            _ => {}
        }
        let result_kind = devtools_runtime_command_result_kind(&command);
        let ownership = devtools_runtime_result_ownership(&command);
        let options = devtools_runtime_serialization_options(&command);
        let target = devtools_runtime_target_using(self, &command).await?;
        let realm = if result_kind == DevToolsRuntimeCommandResultKind::Script {
            devtools_realm_id_for_runtime_target_using(self, &target).await
        } else {
            None
        };
        let owner = CommandOwnerScope::for_route(target.route.clone());
        if let AutomationCommand::CallFunction(call) = &mut command
            && call.context.protocol == FrontendProtocol::WebDriverBidi
        {
            remap_bidi_node_shared_references_for_target_using(self, &target, call, realm.as_ref())
                .await?;
        }
        self.state(|state| {
            state.validate_runtime_automation_handles(&owner, &command, realm.as_ref())
        })??;
        let response = self.native(target.clone(), command).await?;
        if result_kind == DevToolsRuntimeCommandResultKind::Empty {
            return devtools_empty_result_from_response(response);
        }
        let mut result = devtools_script_result_from_response(response, ownership, realm.clone())?;
        // An accepted public-script result still belongs to its original
        // command after navigation. Do not materialize or register its handles
        // against a replacement Document.
        if let Some(scope) = self.scope.clone() {
            let current = self
                .context
                .with_state(&mut self.driver, move |state, _| {
                    state.runtime_scope_accepts_work(&scope)
                })
                .map_err(workflow_error)?;
            if !current {
                return Ok(result);
            }
        }
        self.register_result(&owner, &result, None)?;
        materialize_devtools_script_dom_collection_remote_value_using(
            self,
            &mut result,
            options.as_ref(),
            &target,
            realm.as_ref(),
        )
        .await;
        materialize_devtools_script_deep_serialized_root_value_using(
            self,
            &mut result,
            options.as_ref(),
            &target,
        )
        .await;
        materialize_devtools_script_node_remote_value_using(
            self,
            &mut result,
            options.as_ref(),
            &target,
            realm.as_ref(),
        )
        .await;
        materialize_devtools_script_deep_serialized_node_remote_values_using(
            self,
            &mut result,
            options.as_ref(),
            &target,
            realm.as_ref(),
        )
        .await;
        materialize_devtools_script_window_remote_value(&mut result, &target);
        self.register_result(&owner, &result, realm.as_ref())?;
        Ok(result)
    }

    pub(in crate::domains::runtime) async fn native(
        &mut self,
        target: DevToolsRuntimeTarget,
        command: AutomationCommand,
    ) -> Result<Value, DevToolsError> {
        let owner = CommandOwnerScope::for_route(target.route.clone());
        let mut listeners = Vec::new();
        let (command, params) = match command {
            AutomationCommand::EvaluateScript(command) => {
                let params = devtools_evaluate_script_params(&command, target.execution_context_id);
                (AutomationCommand::EvaluateScript(command), params)
            }
            AutomationCommand::CallFunction(mut command) => {
                if command.context.protocol == FrontendProtocol::WebDriverBidi {
                    listeners = Box::pin(materialize_bidi_channel_argument_proxies_using(
                        self,
                        &target,
                        &mut command,
                    ))
                    .await?;
                }
                let params = devtools_call_function_params_using(
                    self,
                    &owner,
                    &command,
                    target.execution_context_id,
                )
                .await
                .map_err(workflow_error)?;
                (AutomationCommand::CallFunction(command), params)
            }
            command @ AutomationCommand::TerminateExecution(_) => (command, json!({})),
            _ => {
                return Err(DevToolsError::new(
                    DevToolsErrorKind::Unsupported,
                    crate::conn::RuntimeInspectorError::UnknownMethod,
                ));
            }
        };
        let id = self.command_id;
        let scope = self.scope.clone();
        let (prepared, identity) = self
            .context
            .with_state(&mut self.driver, move |state, context| {
                state.with_runtime_scope(scope.as_ref(), move |state| {
                    state.prepare_runtime_script(owner, command, params, id, listeners, context)
                })
            })
            .map_err(workflow_error)?
            .map_err(workflow_error)?;
        let recovery = prepared.recovery_scope();
        let recovery_owner = prepared.output_owner();
        let host = prepared.runtime_host_scope();
        let completed = match prepared.run_using(&mut self.driver).await {
            Ok(completed) => completed,
            Err(error) => {
                let response = CommandOutputPlan::error(-32000, error);
                let prepared = self
                    .context
                    .with_state(&mut self.driver, move |state, context| {
                        state.recover_runtime_script(
                            recovery,
                            recovery_owner,
                            identity,
                            host,
                            response,
                            context,
                        )
                    })
                    .map_err(workflow_error)?;
                prepared
                    .run_using(&mut self.driver)
                    .await
                    .map_err(workflow_error)?
            }
        };
        self.context
            .with_state(&mut self.driver, move |state, context| {
                state.finish_protocol_subcommand(completed, id, context)
            })
            .map_err(workflow_error)?
            .ok_or_else(|| {
                DevToolsError::new(DevToolsErrorKind::Internal, MissingRuntimeCommandResult)
            })
    }

    pub(in crate::domains::runtime) async fn query_raw<T: 'static>(
        &mut self,
        operation: impl for<'state> FnOnce(
            ProtocolWorkflowState<'state>,
        ) -> Result<PreparedRuntimePageQuery<T>>,
    ) -> Result<T> {
        let prepared = self.state_raw(operation)??;
        prepared
            .run(
                &mut self.driver,
                self.context.context_mut().renderer_output_predecessor_mut(),
            )
            .await
    }

    pub(in crate::domains::runtime) fn register_result(
        &mut self,
        owner: &CommandOwnerScope,
        result: &AutomationResult,
        realm: Option<&DevToolsRealmId>,
    ) -> Result<(), DevToolsError> {
        self.state(|state| state.register_runtime_script_result(owner, result, realm))
    }

    pub(in crate::domains::runtime) fn node_snapshot(
        &mut self,
        owner: &CommandOwnerScope,
        object_id: &str,
        depth: i32,
        pierce: bool,
    ) -> impl Future<Output = Result<Option<moli_core::page::DocumentNodeObjectSnapshot>, DevToolsError>>
    {
        self.query(move |state| {
            state.prepare_runtime_object_node_query(owner, object_id.to_owned(), depth, pierce)
        })
    }

    pub(in crate::domains::runtime) fn backend_node_snapshot(
        &mut self,
        owner: &CommandOwnerScope,
        id: u32,
        depth: i32,
        pierce: bool,
    ) -> impl Future<Output = Result<Option<moli_core::page::DocumentNodeObjectSnapshot>, DevToolsError>>
    {
        self.query(move |state| state.prepare_runtime_backend_node_query(owner, id, depth, pierce))
    }

    pub(in crate::domains::runtime) fn register_node_binding(
        &mut self,
        owner: &CommandOwnerScope,
        shared_id: &str,
        backend_node_id: u32,
    ) -> impl Future<Output = Result<(), DevToolsError>> {
        self.query(move |state| {
            state.prepare_runtime_bidi_binding_registration(
                owner,
                shared_id.to_owned(),
                backend_node_id,
            )
        })
    }

    pub(in crate::domains::runtime) fn register_node_reference(
        &mut self,
        owner: &CommandOwnerScope,
        object_id: &str,
        realm: Option<&DevToolsRealmId>,
        shared_id: Option<&str>,
    ) {
        if let Err(error) = self.state(|state| {
            state.register_runtime_node_reference(
                owner,
                object_id.to_owned(),
                realm.cloned(),
                shared_id.map(str::to_owned),
            )
        }) {
            tracing::debug!(?error, "failed to register Runtime node reference");
        }
    }

    pub(in crate::domains::runtime) fn new(
        driver: ProtocolWorkflowDriver,
        context: CommandWorkflowContext,
        command_id: u64,
    ) -> Self {
        Self {
            driver,
            context,
            command_id,
            scope: None,
            inventory_scope: None,
        }
    }

    pub(in crate::domains::runtime) fn state<T: 'static>(
        &mut self,
        operation: impl for<'state> FnOnce(ProtocolWorkflowState<'state>) -> T,
    ) -> Result<T, DevToolsError> {
        self.state_raw(operation).map_err(workflow_error)
    }

    pub(in crate::domains::runtime) fn query<T: 'static>(
        &mut self,
        operation: impl for<'state> FnOnce(
            ProtocolWorkflowState<'state>,
        ) -> Result<PreparedRuntimePageQuery<T>>,
    ) -> impl Future<Output = Result<T, DevToolsError>> {
        self.query_raw(operation).map_err(workflow_error)
    }

    pub(in crate::domains::runtime) async fn helper(
        &mut self,
        owner: CommandOwnerScope,
        method: impl Into<String>,
        params: Value,
    ) -> Result<BackgroundCommandResponsePayload, DevToolsError> {
        let method = method.into();
        let release = matches!(
            method.as_str(),
            "Runtime.releaseObject" | "Runtime.releaseObjectGroup"
        );
        let pending = self
            .state(move |state| state.prepare_runtime_helper(owner, &method, params))?
            .map_err(workflow_error)?;
        if release {
            // These release Inspector storage without producing observations.
            return pending
                .run(&mut self.driver)
                .await
                .response
                .map_err(workflow_error);
        }
        pending
            .run_collecting_renderer_predecessor(
                &mut self.driver,
                self.context.context_mut().renderer_output_predecessor_mut(),
            )
            .await
            .map_err(workflow_error)
    }

    pub(in crate::domains::runtime) fn realms(
        &mut self,
        route: CdpSessionRoute,
    ) -> impl Future<Output = Result<Vec<RuntimeExecutionContextEvent>, DevToolsError>> {
        self.realms_inner(route, false)
    }

    pub(in crate::domains::runtime) fn realms_for_search(
        &mut self,
        route: CdpSessionRoute,
    ) -> impl Future<Output = Result<Vec<RuntimeExecutionContextEvent>, DevToolsError>> {
        self.realms_inner(route, true)
    }

    async fn realms_inner(
        &mut self,
        route: CdpSessionRoute,
        allow_missing_document: bool,
    ) -> Result<Vec<RuntimeExecutionContextEvent>, DevToolsError> {
        self.inventory_scope = None;
        let facts_route = route.clone();
        if let Some(realms) =
            self.state(move |state| state.runtime_worker_realms(&facts_route))??
        {
            return Ok(realms);
        }
        let owner = CommandOwnerScope::for_route(route);
        let pending = self.state(move |state| state.prepare_runtime_realm_inventory(&owner))?;
        let result = match pending {
            // Inventory only reads realm metadata. In particular, an all-page
            // search must not join cursors from unrelated renderer streams.
            Ok(pending) => {
                self.inventory_scope = Some(pending.scope());
                pending.run(&mut self.driver).await
            }
            Err(error) => Err(error),
        };
        match result {
            Ok(realms) => Ok(realms),
            Err(error) if allow_missing_document && error.is::<crate::conn::NoDocumentLoaded>() => {
                Ok(Vec::new())
            }
            Err(error) => Err(workflow_error(error)),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("MissingDevToolsCommandResult")]
struct MissingRuntimeCommandResult;

#[derive(Debug, thiserror::Error)]
#[error("NoSuchTarget")]
struct MissingRuntimeTarget;

#[derive(Debug, thiserror::Error)]
#[error("NoExecutionContextForFrame")]
struct MissingFrameRuntimeContext;
