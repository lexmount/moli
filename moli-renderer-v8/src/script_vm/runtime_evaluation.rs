use super::*;

impl ScriptVm {
    pub(crate) fn evaluate_expression_payload_with_await(
        &mut self,
        expression: &str,
        await_promise: bool,
        user_gesture: bool,
    ) -> Result<Value> {
        let outcome = self.begin_runtime_evaluate(
            None,
            expression,
            await_promise,
            user_gesture,
            None,
            RuntimeEvaluateCodeGenerationPolicy::from_cdp(None),
            RuntimeEvaluateResultMode::RemoteObject,
        )?;
        self.require_completed_runtime_evaluate(outcome)
    }

    pub(crate) fn evaluate_expression_payload_in_context_with_await(
        &mut self,
        execution_context_id: Option<i64>,
        expression: &str,
        await_promise: bool,
        user_gesture: bool,
        file_prompt_handler: Option<&str>,
    ) -> Result<Value> {
        let outcome = self.begin_runtime_evaluate(
            execution_context_id,
            expression,
            await_promise,
            user_gesture,
            file_prompt_handler,
            RuntimeEvaluateCodeGenerationPolicy::from_cdp(None),
            RuntimeEvaluateResultMode::RemoteObject,
        )?;
        self.require_completed_runtime_evaluate(outcome)
    }

    pub(crate) fn evaluate_expression_by_value_payload_in_context_with_await(
        &mut self,
        execution_context_id: Option<i64>,
        expression: &str,
        await_promise: bool,
        user_gesture: bool,
        file_prompt_handler: Option<&str>,
    ) -> Result<Value> {
        let outcome = self.begin_runtime_evaluate(
            execution_context_id,
            expression,
            await_promise,
            user_gesture,
            file_prompt_handler,
            RuntimeEvaluateCodeGenerationPolicy::from_cdp(None),
            RuntimeEvaluateResultMode::ByValue,
        )?;
        self.require_completed_runtime_evaluate(outcome)
    }

    pub(crate) fn performance_metric_snapshot(
        &mut self,
    ) -> Result<RendererPerformanceMetricSnapshot> {
        let snapshot_json = self
            .eval(PERFORMANCE_METRICS_SNAPSHOT_EXPRESSION)
            .context("failed to evaluate performance metric snapshot")?;
        serde_json::from_str(&snapshot_json).context("failed to decode performance metric snapshot")
    }

    pub(crate) fn performance_metric_snapshot_without_script(
        &self,
        lifecycle: crate::runtime::RendererDocumentLifecycleSnapshot,
        resource_count: usize,
    ) -> RendererPerformanceMetricSnapshot {
        let started_micros = lifecycle.started.timestamp_micros;
        let timestamp_ms = |timestamp_micros: u64| timestamp_micros as f64 / 1_000.0;
        let dom = moli_dom_memory_counters(self.document_runtime.dom_host().dom());
        RendererPerformanceMetricSnapshot {
            time_origin_ms: Some(timestamp_ms(started_micros)),
            now_ms: Some(
                moli_time::monotonic_timestamp_micros().saturating_sub(started_micros) as f64
                    / 1_000.0,
            ),
            navigation_start_ms: Some(timestamp_ms(started_micros)),
            dom_content_loaded_ms: lifecycle
                .dom_content_loaded
                .map(|stamp| timestamp_ms(stamp.timestamp_micros)),
            load_event_ms: lifecycle
                .load
                .map(|stamp| timestamp_ms(stamp.timestamp_micros)),
            document_count: Some(1.0),
            frame_count: Some((1 + dom.iframe_element_count) as f64),
            node_count: Some(dom.node_count as f64),
            resource_count: Some(resource_count as f64),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn begin_runtime_evaluate(
        &mut self,
        execution_context_id: Option<i64>,
        expression: &str,
        await_promise: bool,
        user_gesture: bool,
        file_prompt_handler: Option<&str>,
        code_generation_policy: RuntimeEvaluateCodeGenerationPolicy,
        result_mode: RuntimeEvaluateResultMode,
    ) -> Result<RuntimeEvaluateOutcome> {
        self.validate_runtime_evaluate_context(execution_context_id)?;
        let call_id = self.next_internal_runtime_evaluate_call_id()?;
        let mut params = serde_json::Map::new();
        params.insert("expression".to_owned(), json!(expression));
        params.insert("awaitPromise".to_owned(), json!(await_promise));
        params.insert(
            "returnByValue".to_owned(),
            json!(result_mode.returns_by_value()),
        );
        params.insert(
            "allowUnsafeEvalBlockedByCSP".to_owned(),
            json!(code_generation_policy.allows_unsafe_eval_blocked_by_csp()),
        );
        if let Some(execution_context_id) = execution_context_id {
            params.insert("contextId".to_owned(), json!(execution_context_id));
        }
        if user_gesture {
            params.insert("userGesture".to_owned(), json!(true));
        }
        if let Some(file_prompt_handler) = file_prompt_handler {
            params.insert(
                WEBDRIVER_BIDI_FILE_PROMPT_HANDLER_PARAM.to_owned(),
                json!(file_prompt_handler),
            );
        }
        let raw_request = serde_json::to_string(&json!({
            "id": call_id,
            "method": "Runtime.evaluate",
            "params": params,
        }))
        .context("failed to encode internal Runtime.evaluate request")?;
        let (response_tx, mut response_rx) = tokio::sync::oneshot::channel();
        self.dispatch_internal_runtime_evaluate_protocol_message(
            &raw_request,
            RendererRuntimeInspectorResponseSender::new(call_id, response_tx),
        )?;
        match response_rx.try_recv() {
            Ok(completion) => self
                .runtime_evaluate_payload_from_completion(completion)
                .map(RuntimeEvaluateOutcome::Complete),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) if await_promise => {
                self.pending_internal_runtime_evaluates
                    .insert(call_id, response_rx);
                Ok(RuntimeEvaluateOutcome::Pending(
                    PendingRuntimeEvaluateCall { call_id },
                ))
            }
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                self.page_inspector
                    .cancel_internal_runtime_evaluate_response(call_id);
                Err(anyhow!(
                    "internal Runtime.evaluate `{call_id}` produced no synchronous response"
                ))
            }
            Err(tokio::sync::oneshot::error::TryRecvError::Closed) => Err(anyhow!(
                "internal Runtime.evaluate `{call_id}` response channel closed"
            )),
        }
    }

    pub(crate) fn poll_pending_runtime_evaluate(
        &mut self,
        pending: PendingRuntimeEvaluateCall,
    ) -> Result<RuntimeEvaluateOutcome> {
        let Some(response_rx) = self
            .pending_internal_runtime_evaluates
            .get_mut(&pending.call_id)
        else {
            return Err(anyhow!(
                "internal Runtime.evaluate `{}` is no longer pending",
                pending.call_id
            ));
        };
        match response_rx.try_recv() {
            Ok(completion) => {
                self.pending_internal_runtime_evaluates
                    .remove(&pending.call_id);
                self.runtime_evaluate_payload_from_completion(completion)
                    .map(RuntimeEvaluateOutcome::Complete)
            }
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                Ok(RuntimeEvaluateOutcome::Pending(pending))
            }
            Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                self.pending_internal_runtime_evaluates
                    .remove(&pending.call_id);
                Err(anyhow!(
                    "internal Runtime.evaluate `{}` response channel closed",
                    pending.call_id
                ))
            }
        }
    }

    pub(crate) fn cancel_pending_runtime_evaluate(&mut self, pending: PendingRuntimeEvaluateCall) {
        if self
            .pending_internal_runtime_evaluates
            .remove(&pending.call_id)
            .is_some()
        {
            self.page_inspector
                .cancel_internal_runtime_evaluate_response(pending.call_id);
        }
    }

    pub(super) fn validate_runtime_evaluate_context(
        &self,
        execution_context_id: Option<i64>,
    ) -> Result<()> {
        let Some(execution_context_id) = execution_context_id else {
            return Ok(());
        };
        if self.runtime_observable_default_execution_context_id() == Some(execution_context_id)
            || self
                .page_isolated_world_contexts
                .has_execution_context_id(execution_context_id)
            || self
                .child_frame_realm_store
                .contains_key(&execution_context_id)
        {
            return Ok(());
        }
        Err(anyhow!(
            "unknown execution context `{execution_context_id}`"
        ))
    }

    pub(super) fn next_internal_runtime_evaluate_call_id(&mut self) -> Result<i32> {
        let call_id = self.next_internal_runtime_evaluate_call_id;
        self.next_internal_runtime_evaluate_call_id =
            call_id.checked_add(1).filter(|next| *next > 0).unwrap_or(1);
        if self
            .pending_internal_runtime_evaluates
            .contains_key(&call_id)
        {
            return Err(anyhow!("internal Runtime.evaluate call id space exhausted"));
        }
        Ok(call_id)
    }

    pub(super) fn require_completed_runtime_evaluate(
        &mut self,
        outcome: RuntimeEvaluateOutcome,
    ) -> Result<Value> {
        match outcome {
            RuntimeEvaluateOutcome::Complete(payload) => Ok(payload),
            RuntimeEvaluateOutcome::Pending(pending) => {
                self.cancel_pending_runtime_evaluate(pending);
                Err(anyhow!(
                    "internal Runtime.evaluate promise remained pending outside an owner continuation"
                ))
            }
        }
    }

    pub(super) fn runtime_evaluate_payload_from_completion(
        &self,
        completion: RendererRuntimeInspectorAsyncCompletion,
    ) -> Result<Value> {
        let call_id = completion.call_id;
        let response = completion
            .output
            .into_protocol_response(call_id)
            .ok_or_else(|| anyhow!("internal Runtime.evaluate `{call_id}` returned no response"))?;
        if let Some(error) = response.get("error") {
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown inspector error");
            return Err(anyhow!(
                "internal Runtime.evaluate `{call_id}` failed: {message}"
            ));
        }
        let result = response
            .get("result")
            .and_then(Value::as_object)
            .ok_or_else(|| {
                anyhow!("internal Runtime.evaluate `{call_id}` returned an invalid result")
            })?;
        if let Some(exception_details) = result.get("exceptionDetails") {
            let description = exception_details
                .pointer("/exception/description")
                .and_then(Value::as_str)
                .or_else(|| exception_details.get("text").and_then(Value::as_str))
                .unwrap_or("Uncaught");
            return Ok(json!({ "exception": description }));
        }
        result.get("result").cloned().ok_or_else(|| {
            anyhow!("internal Runtime.evaluate `{call_id}` returned no remote object")
        })
    }

    pub(crate) fn exec_in_execution_context(
        &mut self,
        execution_context_id: i64,
        source: &str,
    ) -> Result<()> {
        if self
            .child_frame_realm_store
            .contains_key(&execution_context_id)
        {
            return self.exec_child_frame_source_script_job_for_execution_context_id(
                execution_context_id,
                FrameScriptJobKind::Eval,
                source,
            );
        }
        self.exec_in_isolated_context(execution_context_id, source)
    }

    pub(super) fn runtime_binding_document_owner(
        &mut self,
        execution_context_id: Option<i64>,
    ) -> Result<FrameDocumentTaskOwner> {
        if execution_context_id.is_none()
            || execution_context_id == self.runtime_observable_default_execution_context_id()
        {
            return self
                .current_main_document_task_owner()
                .ok_or_else(|| anyhow!("Runtime binding has no current main document owner"));
        }

        let execution_context_id = execution_context_id.expect("checked execution context id");
        if let Some(world) = self
            .page_isolated_world_contexts
            .context(execution_context_id)
        {
            let owner = world.document_owner;
            if !self
                ._context_host
                .borrow()
                .document_task_owner_is_current(owner)
            {
                return Err(anyhow!(
                    "isolated execution context `{execution_context_id}` belongs to a retired document"
                ));
            }
            return Ok(owner);
        }

        self.prune_stale_child_default_execution_contexts();
        let child_handle = self
            .child_frame_realm_store
            .get(&execution_context_id)
            .map(|world| world.child_handle)
            .ok_or_else(|| anyhow!("unknown execution context `{execution_context_id}`"))?;
        self.child_frame_owner_realm_id_for_execution_context_id(execution_context_id)?;
        self._context_host
            .borrow()
            .current_child_document_task_owner(child_handle)
            .ok_or_else(|| {
                anyhow!(
                    "child execution context `{execution_context_id}` has no current document owner"
                )
            })
    }

    pub(super) fn install_runtime_binding_in_default_context(&mut self, name: &str) -> Result<()> {
        let owner = self.runtime_binding_document_owner(None)?;
        let context_ptr: *const v8::Global<v8::Context> =
            &self.page_default_runtime.context as *const _;
        let execution_context_id = self.default_or_initial_execution_context_id().unwrap_or(0);
        self.install_runtime_binding_in_context(context_ptr, execution_context_id, owner, name)
    }

    pub(super) fn install_runtime_binding_in_execution_context(
        &mut self,
        execution_context_id: i64,
        name: &str,
    ) -> Result<()> {
        if execution_context_id == 0 {
            return self.install_runtime_binding_in_default_context(name);
        }
        if self
            .child_frame_realm_store
            .contains_key(&execution_context_id)
        {
            return self
                .install_runtime_binding_in_child_default_context(execution_context_id, name);
        }
        self.install_runtime_binding_in_isolated_context(execution_context_id, name)
    }

    pub(super) fn install_runtime_binding_in_isolated_context(
        &mut self,
        execution_context_id: i64,
        name: &str,
    ) -> Result<()> {
        let owner = self.runtime_binding_document_owner(Some(execution_context_id))?;
        let context_ptr: *const v8::Global<v8::Context> = self
            .page_isolated_world_contexts
            .context(execution_context_id)
            .map(|world| &world.context as *const _)
            .ok_or_else(|| {
                anyhow!("unknown isolated execution context `{execution_context_id}`")
            })?;
        self.install_runtime_binding_in_context(context_ptr, execution_context_id, owner, name)
    }

    pub(super) fn install_runtime_binding_in_child_default_context(
        &mut self,
        execution_context_id: i64,
        name: &str,
    ) -> Result<()> {
        let owner = self.runtime_binding_document_owner(Some(execution_context_id))?;
        let context_ptr =
            self.child_frame_realm_context_ptr_for_execution_context_id(execution_context_id)?;
        self.install_runtime_binding_in_context(context_ptr, execution_context_id, owner, name)
    }

    pub(super) fn install_runtime_binding_in_context(
        &mut self,
        context_ptr: *const v8::Global<v8::Context>,
        execution_context_id: i64,
        owner: FrameDocumentTaskOwner,
        name: &str,
    ) -> Result<()> {
        self.with_context_scope_by_ptr(context_ptr, |scope, runtime_ptr| {
            let context_token = crate::native_bridge::current_runtime_observable_context_token(
                scope,
            )
            .ok_or_else(|| anyhow!("Runtime binding context has no runtime context token"))?;
            let execution_context = crate::native_bridge::RuntimeBindingExecutionContext::new(
                owner.local_window_id,
                context_token,
            );
            // SAFETY: `with_context_scope_by_ptr` keeps the context host alive and
            // invokes this closure synchronously without retaining `runtime_ptr`.
            if !unsafe { &mut *runtime_ptr }
                .register_runtime_binding_execution_context(execution_context, owner)
            {
                return Err(anyhow!(
                    "Runtime binding context belongs to a retired document owner"
                ));
            }
            let global = scope.get_current_context().global(scope);
            let key = v8_string(scope, name)
                .ok_or_else(|| anyhow!("failed to allocate runtime binding key `{name}`"))?;
            let data = build_runtime_binding_data(
                scope,
                runtime_ptr.cast::<std::ffi::c_void>(),
                key,
                execution_context_id,
                execution_context,
            )
            .map_err(|error| anyhow!("failed to declare runtime binding data: {error}"))?;
            let binding = v8::Function::builder(runtime_binding_callback)
                .data(data.into())
                .build(scope)
                .ok_or_else(|| anyhow!("failed to create runtime binding `{name}`"))?;
            global
                .define_own_property(
                    scope,
                    key.into(),
                    binding.into(),
                    v8::PropertyAttribute::DONT_ENUM,
                )
                .unwrap_or(false)
                .then_some(())
                .ok_or_else(|| anyhow!("failed to install runtime binding `{name}`"))?;
            Ok(())
        })
    }

    pub(super) fn remove_runtime_binding_from_default_context(&mut self, name: &str) -> Result<()> {
        let context_ptr: *const v8::Global<v8::Context> =
            &self.page_default_runtime.context as *const _;
        self.with_context_scope_by_ptr(context_ptr, |scope, _| {
            let global = scope.get_current_context().global(scope);
            let key = v8_string(scope, name)
                .ok_or_else(|| anyhow!("failed to allocate runtime binding key `{name}`"))?;
            let _ = global.delete(scope, key.into());
            Ok(())
        })
    }

    pub(super) fn remove_runtime_binding_from_isolated_context(
        &mut self,
        execution_context_id: i64,
        name: &str,
    ) -> Result<()> {
        let context_ptr: *const v8::Global<v8::Context> = self
            .page_isolated_world_contexts
            .context(execution_context_id)
            .map(|world| &world.context as *const _)
            .ok_or_else(|| {
                anyhow!("unknown isolated execution context `{execution_context_id}`")
            })?;
        self.with_context_scope_by_ptr(context_ptr, |scope, _| {
            let global = scope.get_current_context().global(scope);
            let key = v8_string(scope, name)
                .ok_or_else(|| anyhow!("failed to allocate runtime binding key `{name}`"))?;
            let _ = global.delete(scope, key.into());
            Ok(())
        })
    }

    pub(super) fn remove_runtime_binding_from_child_default_contexts(
        &mut self,
        name: &str,
    ) -> Result<()> {
        let context_ids = self
            .child_frame_realm_store
            .execution_context_ids()
            .collect::<Vec<_>>();
        for execution_context_id in context_ids {
            self.with_child_frame_realm_context_scope(execution_context_id, |scope, _| {
                let global = scope.get_current_context().global(scope);
                let key = v8_string(scope, name)
                    .ok_or_else(|| anyhow!("failed to allocate runtime binding key `{name}`"))?;
                let _ = global.delete(scope, key.into());
                Ok(())
            })?;
        }
        Ok(())
    }

    pub(super) fn content_security_policy_script_element_request<'a>(
        &self,
        script: &'a PreparedScript,
    ) -> ContentSecurityPolicyScriptElementRequest<'a> {
        let parser_inserted_by_handle =
            script.host_script_handle.as_deref().is_some_and(|handle| {
                matches!(
                    self.document_runtime.script_handle_source(handle),
                    ScriptHandleSource::ParserOwned | ScriptHandleSource::DocumentWriteOwned
                )
            });
        let parser_inserted_by_node = self
            .document_runtime
            .dom_host()
            .node(script.node_id)
            .is_some_and(|node| node.flags().parser_created());
        ContentSecurityPolicyScriptElementRequest {
            nonce: script.fetch_metadata.nonce.as_deref(),
            integrity: script.fetch_metadata.integrity.as_deref(),
            parser_inserted: parser_inserted_by_handle || parser_inserted_by_node,
        }
    }
}
