use super::super::*;
use crate::runtime::page_surface::{
    RendererRuntimeCommandOutput, RendererRuntimeInspectorMessage, RendererRuntimeRemoteObject,
    RendererRuntimeRemoteObjectResolution,
};
use crate::runtime::page_vm::PageVmRuntimeCommandOutputScope;
use moli_page_types::{
    RendererDomDebuggerEventListenerBreakpoint,
    renderer_inspector_protocol_configuration_command_from_message,
};

impl PageVm {
    pub(crate) fn evaluate_expression_for_internal_node_reference(
        &mut self,
        handle: DomHandle,
        await_promise: bool,
        expression: impl FnOnce(u64) -> String,
    ) -> Result<Value> {
        let token = self
            .vm_mut()
            .register_internal_node_reference(handle)
            .ok_or_else(|| anyhow!("live node handle is unavailable for internal JS reference"))?;
        let expression = expression(token);
        let result = self.evaluate_expression_with_await(&expression, await_promise);
        self.vm_mut().discard_internal_node_reference(token);
        result
    }

    pub(crate) fn dispatch_runtime_protocol_message_for_inspector_session(
        &mut self,
        inspector_session_id: Option<&str>,
        raw_json: &str,
    ) -> Result<Vec<RendererRuntimeInspectorMessage>> {
        let messages = self
            .vm_mut()
            .dispatch_inspector_protocol_message_for_session(inspector_session_id, raw_json)?;
        self.capture_runtime_inspector_v8_state(inspector_session_id);
        self.record_runtime_inspector_protocol_configuration_command(
            inspector_session_id,
            raw_json,
            &messages,
        );
        Ok(messages)
    }

    pub(crate) fn dispatch_frontend_runtime_protocol_message_for_inspector_session_with_deferred_response(
        &mut self,
        inspector_session_id: Option<&str>,
        raw_json: &str,
        deferred_response: RendererRuntimeInspectorResponseSender,
    ) -> Result<Vec<RendererRuntimeInspectorMessage>> {
        self.dispatch_runtime_protocol_message_with_command_output(
            inspector_session_id,
            raw_json,
            deferred_response,
        )
    }

    pub(crate) fn dispatch_runtime_protocol_message_for_inspector_session_with_context_resolution(
        &mut self,
        inspector_session_id: Option<&str>,
        action: &str,
        raw_json: &str,
    ) -> Result<Vec<RendererRuntimeInspectorMessage>> {
        let prepared_json =
            self.prepare_runtime_protocol_message_with_context_resolution(action, raw_json)?;
        let messages = self
            .vm_mut()
            .dispatch_inspector_protocol_message_for_session(
                inspector_session_id,
                &prepared_json,
            )?;
        self.capture_runtime_inspector_v8_state(inspector_session_id);
        self.record_runtime_inspector_protocol_configuration_command(
            inspector_session_id,
            &prepared_json,
            &messages,
        );
        Ok(messages)
    }

    pub(crate) fn dispatch_frontend_runtime_protocol_message_for_inspector_session_with_context_resolution_and_deferred_response(
        &mut self,
        inspector_session_id: Option<&str>,
        action: &str,
        raw_json: &str,
        deferred_response: RendererRuntimeInspectorResponseSender,
    ) -> Result<Vec<RendererRuntimeInspectorMessage>> {
        let prepared_json =
            self.prepare_runtime_protocol_message_with_context_resolution(action, raw_json)?;
        self.dispatch_runtime_protocol_message_with_command_output(
            inspector_session_id,
            &prepared_json,
            deferred_response,
        )
    }

    fn dispatch_runtime_protocol_message_with_command_output(
        &mut self,
        inspector_session_id: Option<&str>,
        raw_json: &str,
        mut deferred_response: RendererRuntimeInspectorResponseSender,
    ) -> Result<Vec<RendererRuntimeInspectorMessage>> {
        ensure!(
            self.pending_runtime_command_output.is_none(),
            "renderer runtime command output scopes cannot overlap"
        );
        ensure!(
            !self.runtime_command_settlement.has_session_response(),
            "a previous runtime session response must settle at its owner boundary before another command starts"
        );
        // A V8 Inspector response can settle after this command's physical
        // Page turn. Commit it through the Page owner so the response carries
        // the exact concrete-output tail visible at that owner boundary.
        // Otherwise the protocol scheduler could expose the response before
        // Console/lifecycle/target records produced by the same Page.
        if let Some(owner_wake) = self.runtime_hooks.owner_wake() {
            deferred_response = deferred_response.defer_publication_to_page_owner(owner_wake);
        }
        let protocol_configuration_command = serde_json::from_str::<Value>(raw_json)
            .ok()
            .and_then(|message| {
                renderer_inspector_protocol_configuration_command_from_message(&message)
            })
            .map(|(_, command)| command);
        let scope_id = PageVmRuntimeCommandOutputScopeId(self.next_runtime_command_output_scope_id);
        self.next_runtime_command_output_scope_id = self
            .next_runtime_command_output_scope_id
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("renderer runtime command output scope ID overflow"))?;
        let recorder = RendererRuntimeCommandOutputRecorder::new(
            inspector_session_id.map(str::to_owned),
            deferred_response.call_id(),
        );
        self.pending_runtime_command_output = Some(PageVmRuntimeCommandOutputScope {
            id: scope_id,
            inspector_session_id: inspector_session_id.map(str::to_owned),
            protocol_configuration_command,
            recorder: recorder.clone(),
            lifecycle_document: None,
        });
        let dispatch_result = self
            .vm_mut()
            .dispatch_inspector_protocol_message_for_session_with_deferred_response_and_command_output(
                inspector_session_id,
                raw_json,
                deferred_response,
                recorder.clone(),
            );
        if let Some(state) = self.vm().inspector_v8_session_state(inspector_session_id) {
            recorder.set_v8_state_update(state);
        }
        let replacement_lifecycle_ready = dispatch_result.is_ok()
            && self
                .ready_document_replacement_lifecycle_admission()
                .is_some();
        if replacement_lifecycle_ready {
            return Ok(Vec::new());
        }

        let had_response = recorder.has_response();
        let command_error = dispatch_result.as_ref().err().map(ToString::to_string);
        self.finish_pending_runtime_command_output(command_error, dispatch_result.is_err());

        if had_response {
            return Ok(Vec::new());
        }
        dispatch_result?;
        Ok(Vec::new())
    }

    fn capture_runtime_inspector_v8_state(&mut self, inspector_session_id: Option<&str>) {
        if let Some(state) = self.vm().inspector_v8_session_state(inspector_session_id) {
            self.runtime_command_settlement.set_v8_state_update(state);
        }
    }

    pub(in crate::runtime) fn abandon_pending_runtime_command_lifecycle(
        &mut self,
        expected_scope_id: PageVmRuntimeCommandOutputScopeId,
    ) -> bool {
        if !self
            .pending_runtime_command_output
            .as_ref()
            .is_some_and(|scope| scope.id == expected_scope_id)
        {
            return false;
        }
        let Some(scope) = self.pending_runtime_command_output.take() else {
            return false;
        };
        self.vm()
            .end_runtime_inspector_command_output(scope.inspector_session_id.as_deref());
        self.vm().cancel_runtime_inspector_response_for_session(
            scope.inspector_session_id.as_deref(),
            scope.recorder.call_id(),
        );
        true
    }

    pub(in crate::runtime) fn finish_pending_runtime_command_output(
        &mut self,
        error_message: Option<String>,
        cancel_unparked_response: bool,
    ) -> bool {
        let Some(scope) = self.pending_runtime_command_output.take() else {
            return false;
        };
        let had_response = scope.recorder.has_response();
        self.vm()
            .end_runtime_inspector_command_output(scope.inspector_session_id.as_deref());

        if scope.recorder.response_succeeded()
            && let Some(command) = scope.protocol_configuration_command
        {
            self.apply_successful_runtime_inspector_protocol_configuration_command(
                scope.inspector_session_id.as_deref(),
                command,
            );
        }
        if cancel_unparked_response && !had_response {
            self.vm().cancel_runtime_inspector_response_for_session(
                scope.inspector_session_id.as_deref(),
                scope.recorder.call_id(),
            );
        }
        let settlement = match error_message {
            Some(message) if had_response => {
                Some(scope.recorder.finish_with_error_for_page(message))
            }
            Some(_) => {
                let _ = scope.recorder.finish();
                None
            }
            None => Some(scope.recorder.finish_for_page()),
        };
        if let Some(settlement) = settlement {
            self.append_runtime_command_output_settlement(settlement);
        }
        had_response
    }

    fn append_runtime_command_output_settlement(
        &mut self,
        settlement: crate::runtime::RendererRuntimeCommandOutputSettlement,
    ) {
        self.runtime_command_settlement.append(settlement);
    }

    fn record_runtime_inspector_protocol_configuration_command(
        &mut self,
        inspector_session_id: Option<&str>,
        raw_json: &str,
        messages: &[RendererRuntimeInspectorMessage],
    ) {
        let Ok(message) = serde_json::from_str::<Value>(raw_json) else {
            return;
        };
        let Some(call_id) = message.get("id").and_then(Value::as_u64) else {
            return;
        };
        if renderer_inspector_response_succeeded(messages, call_id)
            && let Some((_call_id, command)) =
                renderer_inspector_protocol_configuration_command_from_message(&message)
        {
            self.apply_successful_runtime_inspector_protocol_configuration_command(
                inspector_session_id,
                command,
            );
        }
    }

    fn apply_successful_runtime_inspector_protocol_configuration_command(
        &mut self,
        inspector_session_id: Option<&str>,
        command: RendererInspectorProtocolConfigurationCommand,
    ) {
        let session_key = DevToolsSessionKey::from_wire_session_id(
            inspector_session_id.filter(|session_id| !session_id.is_empty()),
        );
        let remove = {
            let configuration = self
                .devtools
                .inspector_protocol_configurations
                .entry(session_key.clone())
                .or_default();
            configuration.apply_successful_command(command);
            !configuration.requires_restore()
        };
        if remove {
            self.devtools
                .inspector_protocol_configurations
                .remove(&session_key);
        }
    }

    fn runtime_inspector_frontend_restore_enabled(
        &self,
        inspector_session_id: Option<&str>,
    ) -> bool {
        let session_key = DevToolsSessionKey::from_wire_session_id(
            inspector_session_id.filter(|session_id| !session_id.is_empty()),
        );
        self.devtools
            .inspector_protocol_configurations
            .get(&session_key)
            .is_some_and(|configuration| configuration.runtime_frontend_enabled)
    }

    pub(crate) fn configure_dom_debugger_event_listener_breakpoint(
        &mut self,
        inspector_session_id: Option<&str>,
        breakpoint: RendererDomDebuggerEventListenerBreakpoint,
        enabled: bool,
    ) {
        self.vm_mut()
            .configure_dom_debugger_event_listener_breakpoint(
                inspector_session_id,
                breakpoint.clone(),
                enabled,
            );
        let session_key = DevToolsSessionKey::from_wire_session_id(
            inspector_session_id.filter(|session_id| !session_id.is_empty()),
        );
        let remove = {
            let configuration = self
                .devtools
                .inspector_protocol_configurations
                .entry(session_key.clone())
                .or_default();
            if enabled {
                configuration.set_dom_debugger_event_listener_breakpoint(breakpoint);
            } else {
                configuration.remove_dom_debugger_event_listener_breakpoint(&breakpoint);
            }
            !configuration.requires_restore()
        };
        if remove {
            self.devtools
                .inspector_protocol_configurations
                .remove(&session_key);
        }
    }

    pub(crate) fn configure_dom_debugger_xhr_breakpoint(
        &mut self,
        inspector_session_id: Option<&str>,
        breakpoint: RendererDomDebuggerXhrBreakpoint,
        enabled: bool,
    ) {
        self.vm_mut().configure_dom_debugger_xhr_breakpoint(
            inspector_session_id,
            breakpoint.clone(),
            enabled,
        );
        let session_key = DevToolsSessionKey::from_wire_session_id(
            inspector_session_id.filter(|session_id| !session_id.is_empty()),
        );
        let remove = {
            let configuration = self
                .devtools
                .inspector_protocol_configurations
                .entry(session_key.clone())
                .or_default();
            if enabled {
                configuration.set_dom_debugger_xhr_breakpoint(breakpoint);
            } else {
                configuration.remove_dom_debugger_xhr_breakpoint(&breakpoint);
            }
            !configuration.requires_restore()
        };
        if remove {
            self.devtools
                .inspector_protocol_configurations
                .remove(&session_key);
        }
    }

    fn prepare_runtime_protocol_message_with_context_resolution(
        &mut self,
        action: &str,
        raw_json: &str,
    ) -> Result<String> {
        let context_param_name = match action {
            "evaluate" => Some("contextId"),
            "callFunctionOn" | "addBinding" => Some("executionContextId"),
            _ => None,
        };
        let Some(context_param_name) = context_param_name else {
            return Ok(raw_json.to_owned());
        };

        let mut message: Value = serde_json::from_str(raw_json)?;
        let params = message
            .as_object_mut()
            .and_then(|message| {
                message
                    .entry("params")
                    .or_insert_with(|| json!({}))
                    .as_object_mut()
            })
            .ok_or_else(|| anyhow!("runtime protocol params must be an object"))?;

        if let Some(context_id) = params.get(context_param_name).and_then(Value::as_i64) {
            self.rewrite_isolated_runtime_context_param(params, context_param_name, context_id)?;
        } else if action == "callFunctionOn"
            && !params.contains_key("objectId")
            && !params.contains_key("uniqueContextId")
            && let Some(context_id) = self.default_execution_context_id()
        {
            params.insert(context_param_name.to_owned(), json!(context_id));
        }

        Ok(serde_json::to_string(&message)?)
    }

    fn rewrite_isolated_runtime_context_param(
        &mut self,
        params: &mut serde_json::Map<String, Value>,
        context_param_name: &str,
        context_id: i64,
    ) -> Result<()> {
        if !self.has_isolated_execution_context_id(context_id) {
            return Ok(());
        }

        self.ensure_isolated_worlds_attached_to_inspector()?;
        if let Some(inspector_context_id) =
            self.inspector_execution_context_id_for_isolated_context(context_id)
        {
            params.insert(context_param_name.to_owned(), json!(inspector_context_id));
        }
        Ok(())
    }

    pub(crate) fn inspector_execution_context_id_for_isolated_context(
        &self,
        execution_context_id: i64,
    ) -> Option<i64> {
        self.vm()
            .inspector_execution_context_id_for_isolated_context(execution_context_id)
    }

    pub(crate) fn isolated_execution_context_id_for_inspector_context(
        &self,
        execution_context_id: i64,
    ) -> Option<i64> {
        self.vm()
            .isolated_execution_context_id_for_inspector_context(execution_context_id)
    }

    pub(crate) fn ensure_isolated_worlds_attached_to_inspector(&mut self) -> Result<()> {
        self.vm_mut().ensure_isolated_worlds_attached_to_inspector()
    }

    pub(crate) fn apply_runtime_protocol_state(
        &mut self,
        inspector_session_id: Option<&str>,
        session_restore_snapshots: &[crate::runtime::RendererInspectorSessionRestoreSnapshot],
        isolated_worlds: &[crate::protocol_types::RuntimeIsolatedWorldDefinition],
        stored_runtime_bindings: &[crate::protocol_types::RuntimeBindingRegistration],
        session_runtime_bindings: &[crate::protocol_types::RuntimeBindingRegistration],
    ) -> Result<()> {
        self.vm()
            .reattach_v8_inspector_sessions(session_restore_snapshots);
        self.set_runtime_isolated_world_definitions(isolated_worlds);
        self.set_stored_runtime_bindings(stored_runtime_bindings);
        self.set_inspector_session_runtime_bindings(inspector_session_id, session_runtime_bindings);
        for world in isolated_worlds {
            self.create_isolated_world(&world.name, world.grant_universal_access)?;
        }
        for binding in session_runtime_bindings {
            self.add_runtime_binding(
                inspector_session_id,
                &binding.name,
                binding.execution_context_name.as_deref(),
                None,
            )?;
        }
        for binding in stored_runtime_bindings {
            if binding.devtools_session.is_some() || session_runtime_bindings.contains(binding) {
                continue;
            }
            self.install_runtime_binding(
                &binding.name,
                binding.execution_context_name.as_deref(),
                None,
            )?;
        }
        Ok(())
    }

    pub(crate) fn create_isolated_world_for_frame(
        &mut self,
        frame_id: &str,
        name: &str,
        grant_universal_access: bool,
    ) -> Result<i64> {
        self.vm_mut()
            .create_isolated_world_for_frame(frame_id, name, grant_universal_access)
    }

    fn set_runtime_isolated_world_definitions(
        &mut self,
        isolated_worlds: &[crate::protocol_types::RuntimeIsolatedWorldDefinition],
    ) {
        self.runtime_isolated_worlds.clear();
        for world in isolated_worlds {
            self.remember_runtime_isolated_world_definition(
                &world.name,
                world.grant_universal_access,
            );
        }
    }

    fn remember_runtime_isolated_world_definition(
        &mut self,
        name: &str,
        grant_universal_access: bool,
    ) {
        if let Some(existing) = self
            .runtime_isolated_worlds
            .iter_mut()
            .find(|world| world.name == name)
        {
            existing.grant_universal_access |= grant_universal_access;
            return;
        }
        self.runtime_isolated_worlds
            .push(crate::protocol_types::RuntimeIsolatedWorldDefinition {
                name: name.to_owned(),
                grant_universal_access,
            });
    }

    pub(crate) fn create_isolated_world_runtime_activity(
        &mut self,
        inspector_session_id: Option<&str>,
        frame_id: Option<&str>,
        name: &str,
        grant_universal_access: bool,
    ) -> Result<i64> {
        let devtools_session = DevToolsSessionKey::from_wire_session_id(
            inspector_session_id.filter(|session_id| !session_id.is_empty()),
        );
        let had_world =
            self.vm()
                .has_isolated_world_for_owner(Some(&devtools_session), frame_id, name);
        let execution_context_id = match frame_id {
            Some(frame_id) => self.vm_mut().ensure_isolated_world_for_frame_owner(
                Some(&devtools_session),
                frame_id,
                name,
                grant_universal_access,
            )?,
            None => self.vm_mut().ensure_isolated_world_for_owner(
                Some(&devtools_session),
                name,
                grant_universal_access,
            )?,
        };
        for binding in self.inspector_session_runtime_bindings_for_world(inspector_session_id, name)
        {
            self.install_runtime_binding(&binding.name, Some(name), Some(execution_context_id))?;
        }
        if !had_world {
            let scripts = self
                .document_start_scripts
                .iter()
                .filter(|script| {
                    script.world_name.as_deref() == Some(name)
                        && script.devtools_session.as_ref() == Some(&devtools_session)
                })
                .cloned()
                .collect::<Vec<_>>();
            for script in scripts {
                if let Err(error) = self
                    .run_document_start_script_in_execution_context(execution_context_id, &script)
                {
                    tracing::debug!(
                        %error,
                        execution_context_id,
                        world_name = name,
                        "isolated world document-start script failed after context creation"
                    );
                }
            }
        }
        Ok(execution_context_id)
    }

    pub(crate) fn runtime_enable_events(
        &mut self,
        inspector_session_id: Option<&str>,
    ) -> Result<RendererRuntimeCommandOutput> {
        const INTERNAL_RUNTIME_ENABLE_ID: u64 = 900_003;

        // The renderer-owner command boundary has already waited for pending
        // exact-Document child-realm tasks. Runtime.enable can therefore use
        // V8's authoritative reportAllContexts() result directly. Do not add
        // a realm-inventory replay here: live context creation has exactly one
        // producer, while late enable is this command-local V8 report.
        let already_enabled = self.runtime_inspector_frontend_restore_enabled(inspector_session_id);
        let mut messages = if already_enabled {
            Vec::new()
        } else {
            let request = json!({
                "id": INTERNAL_RUNTIME_ENABLE_ID,
                "method": "Runtime.enable",
            });
            let raw_request = serde_json::to_string(&request)?;
            let messages = self
                .vm_mut()
                .dispatch_internal_inspector_protocol_message_for_session(
                    inspector_session_id,
                    &raw_request,
                    i32::try_from(INTERNAL_RUNTIME_ENABLE_ID)
                        .expect("bounded internal Runtime.enable call id"),
                )?;
            self.record_runtime_inspector_protocol_configuration_command(
                inspector_session_id,
                &raw_request,
                &messages,
            );
            messages
        };
        self.vm_mut()
            .ensure_isolated_worlds_attached_to_inspector()?;
        messages.extend(
            self.vm_mut()
                .take_runtime_inspector_messages(inspector_session_id),
        );
        let messages = messages
            .into_iter()
            .filter(RendererRuntimeInspectorMessage::has_v8_inspector_method)
            .collect();
        let mut output = RendererRuntimeCommandOutput::from_messages(messages);
        if let Some(state) = self.vm().inspector_v8_session_state(inspector_session_id) {
            output.set_v8_state_update(state);
        }
        Ok(output)
    }

    pub(crate) fn runtime_disable_events(
        &mut self,
        inspector_session_id: Option<&str>,
    ) -> Result<RendererRuntimeCommandOutput> {
        const INTERNAL_RUNTIME_DISABLE_ID: u64 = 900_005;
        let was_enabled = self.runtime_inspector_frontend_restore_enabled(inspector_session_id);
        let raw_request = serde_json::to_string(&json!({
            "id": INTERNAL_RUNTIME_DISABLE_ID,
            "method": "Runtime.disable",
        }))?;
        let mut messages = self
            .vm_mut()
            .dispatch_internal_inspector_protocol_message_for_session(
                inspector_session_id,
                &raw_request,
                i32::try_from(INTERNAL_RUNTIME_DISABLE_ID)
                    .expect("bounded internal Runtime.disable call id"),
            )?;
        anyhow::ensure!(
            renderer_inspector_response_succeeded(&messages, INTERNAL_RUNTIME_DISABLE_ID),
            "Runtime.disable did not complete successfully"
        );
        self.record_runtime_inspector_protocol_configuration_command(
            inspector_session_id,
            &raw_request,
            &messages,
        );

        // V8 keeps existing bindings active in this Document after disable.
        // Clear only their restore metadata in this renderer turn; a later
        // Browser completion must not restore them for a future Document.
        let session_key = DevToolsSessionKey::from_wire_session_id(inspector_session_id);
        let session_bindings = if was_enabled {
            self.runtime_bindings
                .retain(|binding| binding.devtools_session.as_ref() != Some(&session_key));
            Vec::new()
        } else {
            self.vm()
                .inspector_session_runtime_bindings(inspector_session_id)
        };
        let stored_bindings = self.runtime_bindings.clone();
        self.set_runtime_binding_state(inspector_session_id, &stored_bindings, &session_bindings);
        messages.extend(
            self.vm_mut()
                .take_runtime_inspector_messages(inspector_session_id),
        );
        let mut output = RendererRuntimeCommandOutput::from_messages(
            messages
                .into_iter()
                .filter(RendererRuntimeInspectorMessage::has_v8_inspector_method)
                .collect(),
        );
        if let Some(state) = self.vm().inspector_v8_session_state(inspector_session_id) {
            output.set_v8_state_update(state);
        }
        Ok(output)
    }

    pub(crate) fn runtime_binding_events(
        &mut self,
        inspector_session_id: Option<&str>,
        binding: crate::protocol_types::RuntimeBindingRegistration,
        remove: bool,
    ) -> Result<RendererRuntimeCommandOutput> {
        const INTERNAL_RUNTIME_BINDING_ID: u64 = 900_006;
        let was_enabled = self.runtime_inspector_frontend_restore_enabled(inspector_session_id);
        let mut params = json!({"name": binding.name});
        if let Some(name) = &binding.execution_context_name {
            params["executionContextName"] = json!(name);
        }
        let raw_request = serde_json::to_string(&json!({
            "id": INTERNAL_RUNTIME_BINDING_ID,
            "method": if remove { "Runtime.removeBinding" } else { "Runtime.addBinding" },
            "params": params,
        }))?;
        let mut messages = self
            .vm_mut()
            .dispatch_internal_inspector_protocol_message_for_session(
                inspector_session_id,
                &raw_request,
                i32::try_from(INTERNAL_RUNTIME_BINDING_ID)
                    .expect("bounded internal Runtime binding call id"),
            )?;
        let succeeded =
            renderer_inspector_response_succeeded(&messages, INTERNAL_RUNTIME_BINDING_ID);
        anyhow::ensure!(
            runtime_inspector_response_message(&messages, INTERNAL_RUNTIME_BINDING_ID).is_some(),
            "Runtime binding command did not complete"
        );
        if succeeded {
            self.record_runtime_inspector_protocol_configuration_command(
                inspector_session_id,
                &raw_request,
                &messages,
            );
            if remove && !was_enabled {
                self.remove_runtime_binding(&binding.name)?;
            }
            let session_key = DevToolsSessionKey::from_wire_session_id(inspector_session_id);
            let mut session_bindings = self
                .vm()
                .inspector_session_runtime_bindings(inspector_session_id);
            if remove {
                session_bindings.retain(|existing| existing.name != binding.name);
                self.runtime_bindings.retain(|existing| {
                    existing.devtools_session.as_ref() != Some(&session_key)
                        || existing.name != binding.name
                });
            } else {
                if !session_bindings.contains(&binding) {
                    session_bindings.push(binding.clone());
                }
                let stored_binding = binding.with_devtools_session(session_key);
                if !self.runtime_bindings.contains(&stored_binding) {
                    self.runtime_bindings.push(stored_binding);
                }
            }
            // Commit only this mutation against the renderer's current state.
            // No later adapter can reapply an older Browser binding snapshot.
            let stored_bindings = self.runtime_bindings.clone();
            self.set_runtime_binding_state(
                inspector_session_id,
                &stored_bindings,
                &session_bindings,
            );
        }
        messages.extend(
            self.vm_mut()
                .take_runtime_inspector_messages(inspector_session_id),
        );
        let mut output = RendererRuntimeCommandOutput::from_messages(
            messages
                .into_iter()
                .filter(|message| {
                    message.has_v8_inspector_method()
                        || matches!(message, RendererRuntimeInspectorMessage::Protocol(message)
                        if message.payload().has_field("error"))
                })
                .collect(),
        );
        if let Some(state) = self.vm().inspector_v8_session_state(inspector_session_id) {
            output.set_v8_state_update(state);
        }
        Ok(output)
    }

    pub(crate) fn commit_runtime_binding_removal(
        &mut self,
        inspector_session_id: Option<&str>,
        name: &str,
        output: Result<RendererRuntimeCommandOutput, String>,
    ) -> Result<RendererRuntimeCommandOutput> {
        let output = output.map_err(anyhow::Error::msg)?;
        if !output.messages().iter().any(|message| {
            matches!(message, RendererRuntimeInspectorMessage::Protocol(message)
                if message.payload().has_field("error"))
        }) {
            let session_key = DevToolsSessionKey::from_wire_session_id(inspector_session_id);
            let mut session_bindings = self
                .vm()
                .inspector_session_runtime_bindings(inspector_session_id);
            session_bindings.retain(|binding| binding.name != name);
            self.runtime_bindings.retain(|binding| {
                binding.devtools_session.as_ref() != Some(&session_key) || binding.name != name
            });
            // The existing Inspector session executed V8 removal above the
            // suspended Page stack. This stage must not enter a new V8 scope
            // (including ensure_runtime_inspector_session).
            let stored_bindings = self.runtime_bindings.clone();
            self.set_stored_runtime_bindings(&stored_bindings);
            self.set_inspector_session_runtime_bindings(inspector_session_id, &session_bindings);
        }
        Ok(output)
    }

    pub(crate) fn add_runtime_binding(
        &mut self,
        inspector_session_id: Option<&str>,
        name: &str,
        execution_context_name: Option<&str>,
        execution_context_id: Option<i64>,
    ) -> Result<()> {
        const INTERNAL_RUNTIME_ADD_BINDING_ID: u64 = 900_004;

        let inspector_execution_context_id = if let Some(execution_context_id) =
            execution_context_id
        {
            if self.has_isolated_execution_context_id(execution_context_id) {
                match self.inspector_execution_context_id_for_isolated_context(execution_context_id)
                {
                    Some(inspector_context_id) => Some(inspector_context_id),
                    None => {
                        self.install_runtime_binding(
                            name,
                            execution_context_name,
                            Some(execution_context_id),
                        )?;
                        return Ok(());
                    }
                }
            } else {
                Some(execution_context_id)
            }
        } else {
            None
        };

        let mut params = json!({ "name": name });
        if let Some(execution_context_name) = execution_context_name {
            params["executionContextName"] = json!(execution_context_name);
        }
        if let Some(inspector_execution_context_id) = inspector_execution_context_id {
            params["executionContextId"] = json!(inspector_execution_context_id);
        }
        let request = json!({
            "id": INTERNAL_RUNTIME_ADD_BINDING_ID,
            "method": "Runtime.addBinding",
            "params": params,
        });
        let raw_request = serde_json::to_string(&request)?;
        let messages = self
            .vm_mut()
            .dispatch_inspector_protocol_message_for_session(inspector_session_id, &raw_request)?;
        if let Some(response) =
            runtime_inspector_response_message(&messages, INTERNAL_RUNTIME_ADD_BINDING_ID)
            && let Some(error) = response.get("error")
        {
            return Err(anyhow!("failed to add runtime binding: {error}"));
        }
        self.install_runtime_binding(name, execution_context_name, execution_context_id)?;
        Ok(())
    }

    pub(crate) fn resolve_runtime_object_for_live_handle(
        &mut self,
        inspector_session_id: Option<&str>,
        handle: crate::dom::NodeId,
        execution_context_id: Option<i64>,
        object_group: Option<&str>,
    ) -> Result<Option<RendererRuntimeRemoteObject>> {
        const INTERNAL_RUNTIME_NODE_RESOLVE_ID: u64 = 900_002;
        let context_id = if let Some(context_id) =
            self.runtime_evaluate_context_id_for_resolution(execution_context_id)?
        {
            Some(context_id)
        } else if execution_context_id.is_some() {
            return Ok(None);
        } else {
            None
        };
        let token = self
            .vm_mut()
            .register_internal_node_reference(handle)
            .ok_or_else(|| anyhow!("live node handle is unavailable for runtime object resolve"))?;
        let result = (|| {
            let mut params = json!({
                "expression": format!(
                    "__moliHostResolveInternalNodeReference({token})"
                ),
                "objectGroup": object_group.unwrap_or(""),
            });
            if let Some(context_id) = context_id {
                params["contextId"] = json!(context_id);
            }
            let request = json!({
                "id": INTERNAL_RUNTIME_NODE_RESOLVE_ID,
                "method": "Runtime.evaluate",
                "params": params,
            });
            let raw_request = serde_json::to_string(&request)?;
            let messages = self
                .vm_mut()
                .dispatch_inspector_protocol_message_for_session(
                    inspector_session_id,
                    &raw_request,
                )?;
            Ok(Self::runtime_remote_object_from_inspector_messages(
                &messages,
                INTERNAL_RUNTIME_NODE_RESOLVE_ID,
            ))
        })();
        self.vm_mut().discard_internal_node_reference(token);
        result
    }

    pub(crate) fn resolve_runtime_object_for_backend_node_id(
        &mut self,
        inspector_session_id: Option<&str>,
        backend_node_id: u32,
        execution_context_id: Option<i64>,
        object_group: Option<&str>,
    ) -> Result<RendererRuntimeRemoteObjectResolution> {
        let Some(handle) = self.live_handle_for_backend_node_id(backend_node_id) else {
            return Ok(RendererRuntimeRemoteObjectResolution::MissingNode);
        };
        let remote_object = self.resolve_runtime_object_for_live_handle(
            inspector_session_id,
            handle,
            execution_context_id,
            object_group,
        )?;
        match remote_object {
            Some(remote_object) => Ok(RendererRuntimeRemoteObjectResolution::Found(remote_object)),
            None if execution_context_id.is_some() => {
                Ok(RendererRuntimeRemoteObjectResolution::MissingContext)
            }
            None => Ok(RendererRuntimeRemoteObjectResolution::MissingNode),
        }
    }

    fn runtime_evaluate_context_id_for_resolution(
        &self,
        execution_context_id: Option<i64>,
    ) -> Result<Option<i64>> {
        match execution_context_id {
            Some(context_id) => {
                if let Some(inspector_context_id) =
                    self.inspector_execution_context_id_for_isolated_context(context_id)
                {
                    Ok(Some(inspector_context_id))
                } else if self.has_isolated_execution_context_id(context_id) {
                    Ok(None)
                } else {
                    Ok(Some(context_id))
                }
            }
            None => Ok(None),
        }
    }

    fn runtime_remote_object_from_inspector_messages(
        messages: &[RendererRuntimeInspectorMessage],
        response_id: u64,
    ) -> Option<RendererRuntimeRemoteObject> {
        let response = runtime_inspector_response_message(messages, response_id)?;
        if response.get("error").is_some() || response["result"]["exceptionDetails"].is_object() {
            return None;
        }
        RendererRuntimeRemoteObject::from_protocol_value(response["result"]["result"].clone())
    }

    pub(crate) fn run_document_start_script_now_with_runtime_state(
        &mut self,
        inspector_session_id: Option<&str>,
        script: &DocumentStartScript,
    ) -> Result<Option<(i64, bool)>> {
        if self.script_execution_disabled() {
            return Ok(None);
        }

        let mut created_world_before_script = false;
        if let Some(world_name) = script.world_name.as_deref() {
            let session_key = DevToolsSessionKey::from_wire_session_id(
                inspector_session_id.filter(|session_id| !session_id.is_empty()),
            );
            let runtime_bindings = if script.devtools_session.as_ref() == Some(&session_key) {
                self.inspector_session_runtime_bindings_for_world(inspector_session_id, world_name)
            } else {
                self.runtime_bindings
                    .iter()
                    .filter(|binding| {
                        binding.devtools_session.as_ref() == script.devtools_session.as_ref()
                            && binding.execution_context_name.as_deref() == Some(world_name)
                    })
                    .cloned()
                    .collect()
            };
            if !runtime_bindings.is_empty() {
                let had_world = self.vm().has_isolated_world_for_owner(
                    script.devtools_session.as_ref(),
                    None,
                    world_name,
                );
                let execution_context_id = self.vm_mut().ensure_isolated_world_for_owner(
                    script.devtools_session.as_ref(),
                    world_name,
                    false,
                )?;
                created_world_before_script = !had_world;
                for binding in runtime_bindings {
                    self.install_runtime_binding(
                        &binding.name,
                        Some(world_name),
                        Some(execution_context_id),
                    )?;
                }
            }
        }

        Ok(self.vm_mut().run_document_start_script_now(script)?.map(
            |(execution_context_id, created)| {
                (execution_context_id, created || created_world_before_script)
            },
        ))
    }

    pub(crate) fn add_document_start_script_runtime_activity(
        &mut self,
        inspector_session_id: Option<&str>,
        script: &DocumentStartScript,
        run_immediately: bool,
    ) -> Result<Option<(i64, bool)>> {
        self.document_start_scripts.push(script.clone());
        let scripts = self.document_start_scripts.clone();
        self.vm_mut().set_stored_document_start_scripts(&scripts);
        if !run_immediately {
            return Ok(None);
        }
        self.run_document_start_script_now_with_runtime_state(inspector_session_id, script)
    }

    pub(crate) fn run_document_start_script_in_execution_context(
        &mut self,
        execution_context_id: i64,
        script: &DocumentStartScript,
    ) -> Result<()> {
        if self.script_execution_disabled() {
            return Ok(());
        }
        self.vm_mut()
            .run_document_start_script_in_execution_context(execution_context_id, script)
    }

    pub(crate) fn remove_document_start_script_by_registry_key(&mut self, registry_key: &str) {
        self.document_start_scripts
            .retain(|script| script.registry_key.as_deref() != Some(registry_key));
        let scripts = self.document_start_scripts.clone();
        self.vm_mut().set_stored_document_start_scripts(&scripts);
    }

    pub(crate) fn set_stored_runtime_bindings(
        &mut self,
        bindings: &[crate::protocol_types::RuntimeBindingRegistration],
    ) {
        self.runtime_bindings = bindings.to_vec();
        self.vm_mut().set_stored_runtime_bindings(bindings);
    }

    pub(crate) fn set_inspector_session_runtime_bindings(
        &mut self,
        inspector_session_id: Option<&str>,
        bindings: &[crate::protocol_types::RuntimeBindingRegistration],
    ) {
        self.vm_mut()
            .set_inspector_session_runtime_bindings(inspector_session_id, bindings);
        let session_key = DevToolsSessionKey::from_wire_session_id(
            inspector_session_id.filter(|session_id| !session_id.is_empty()),
        );
        let remove = {
            let configuration = self
                .devtools
                .inspector_protocol_configurations
                .entry(session_key.clone())
                .or_default();
            configuration.runtime_bindings = bindings.to_vec();
            !configuration.requires_restore()
        };
        if remove {
            self.devtools
                .inspector_protocol_configurations
                .remove(&session_key);
        }
    }

    pub(crate) fn inspector_session_runtime_bindings_for_world(
        &self,
        inspector_session_id: Option<&str>,
        world_name: &str,
    ) -> Vec<crate::protocol_types::RuntimeBindingRegistration> {
        self.vm()
            .inspector_session_runtime_bindings(inspector_session_id)
            .into_iter()
            .filter(|binding| binding.execution_context_name.as_deref() == Some(world_name))
            .collect()
    }

    pub(crate) fn set_runtime_binding_state(
        &mut self,
        inspector_session_id: Option<&str>,
        stored_runtime_bindings: &[crate::protocol_types::RuntimeBindingRegistration],
        session_runtime_bindings: &[crate::protocol_types::RuntimeBindingRegistration],
    ) {
        self.set_stored_runtime_bindings(stored_runtime_bindings);
        self.set_inspector_session_runtime_bindings(inspector_session_id, session_runtime_bindings);
    }

    pub(crate) fn detach_runtime_inspector_session(
        &mut self,
        inspector_session_id: Option<&str>,
    ) -> bool {
        let session_key = DevToolsSessionKey::from_wire_session_id(
            inspector_session_id.filter(|session_id| !session_id.is_empty()),
        );
        self.runtime_bindings
            .retain(|binding| binding.devtools_session.as_ref() != Some(&session_key));
        let runtime_bindings = self.runtime_bindings.clone();
        self.vm_mut().set_stored_runtime_bindings(&runtime_bindings);
        let detached = self
            .vm_mut()
            .detach_runtime_inspector_session(inspector_session_id);
        self.devtools
            .inspector_protocol_configurations
            .remove(&session_key);
        detached
    }

    pub(crate) fn install_runtime_binding(
        &mut self,
        name: &str,
        execution_context_name: Option<&str>,
        execution_context_id: Option<i64>,
    ) -> Result<()> {
        self.vm_mut()
            .install_runtime_binding(name, execution_context_name, execution_context_id)
    }

    pub(crate) fn remove_runtime_binding(&mut self, name: &str) -> Result<()> {
        self.vm_mut().remove_runtime_binding(name)
    }

    pub(crate) fn remove_default_runtime_binding(&mut self, name: &str) -> Result<()> {
        self.vm_mut().remove_default_runtime_binding(name)
    }
}

fn renderer_inspector_response_succeeded(
    messages: &[RendererRuntimeInspectorMessage],
    call_id: u64,
) -> bool {
    messages.iter().any(|message| {
        let RendererRuntimeInspectorMessage::Protocol(message) = message else {
            return false;
        };
        message.payload().id_u64() == Some(call_id) && !message.payload().has_field("error")
    })
}

fn runtime_inspector_response_message(
    messages: &[RendererRuntimeInspectorMessage],
    response_id: u64,
) -> Option<&Value> {
    messages.iter().find_map(|message| {
        let RendererRuntimeInspectorMessage::Protocol(message) = message else {
            return None;
        };
        (message.payload().id_u64() == Some(response_id)).then(|| message.value())
    })
}
